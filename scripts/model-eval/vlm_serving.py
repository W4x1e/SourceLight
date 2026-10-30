"""How fast each way of serving the vision model tags and describes photos.

Sends the app's own requests: the prompts and JSON schema are read from
src-tauri/src/tagger/mod.rs, and images are shrunk as `prepare_image` does
(longest edge, JPEG q85). Server timings come from each server's reply.

Modes:
  ollama  Ollama's /api/chat, as the old Ollama tagger sent it
  llama   llama-server's /v1/chat/completions, as tagger/llama.rs sends it

  python vlm_serving.py bench llama http://127.0.0.1:41440 tag 640 2 24
  python vlm_serving.py serve <llama-server> <model.gguf> <mmproj.gguf> --slots 2 \
      --tasks tag:640 describe:1280 both:1280 --n 24 --label "upstream vulkan"

`serve` starts llama-server with sidecar::llama_server_args's flags, runs the
tasks and stops it. Photos: the app's previews (`_p.jpg`), sampled evenly;
`--labelled` uses the 71 library photos whose genre is clear-cut (qwen_tag.py)
and scores genre accuracy on them. Results are appended to out/vlm_serving.jsonl.
"""
import argparse, base64, io, json, os, re, sqlite3, statistics, subprocess, sys, time, urllib.request
from concurrent.futures import ThreadPoolExecutor
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, "..", "..", "src-tauri", "src")
OUT = os.path.join(HERE, "out", "vlm_serving.jsonl")

if sys.platform == "darwin":
    APP = os.path.expanduser("~/Library/Application Support/app.sourcelight")
else:
    APP = os.path.join(os.environ.get("LOCALAPPDATA", ""), "app.sourcelight")


# --- the app's prompts, read from the Rust source ---------------------------

def rust_str(lit):
    """Evaluates the escapes in a Rust string literal's body."""
    out, i = [], 0
    while i < len(lit):
        c = lit[i]
        if c == "\\":
            n = lit[i + 1]
            if n == "\n":  # line continuation: skip the newline and the next line's indent
                i += 2
                while i < len(lit) and lit[i] in " \t":
                    i += 1
                continue
            out.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\"}[n])
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def load_prompts():
    src = open(os.path.join(SRC, "tagger", "mod.rs"), encoding="utf-8").read()
    macro = lambda name: rust_str(re.search(
        r"macro_rules! " + name + r' \{\s*\(\) => \{\s*"(.*?)"\s*\};', src, re.S).group(1))

    def concat(const):
        body = re.search(r"const " + const + r": &str = (concat!\((.*?)\);|(\w+)!\(\);)", src, re.S)
        if body.group(3):
            return macro(body.group(3))
        parts = re.findall(r'(\w+)!\(\)|"((?:[^"\\]|\\.)*)"', body.group(2), re.S)
        return "".join(macro(m) if m else rust_str(s) for m, s in parts)

    genres = re.findall(r'"([a-z-]+)"', re.search(
        r"pub const GENRES: &\[&str\] = &\[(.*?)\];",
        open(os.path.join(SRC, "models.rs"), encoding="utf-8").read(), re.S).group(1))
    return concat("PROMPT"), concat("CAPTION_PROMPT"), concat("TAG_AND_CAPTION_PROMPT"), genres


TAG, CAPTION, BOTH, GENRES = load_prompts()
PROMPTS = {"tag": TAG, "describe": CAPTION, "both": BOTH}


def schema(with_description=False):
    """tagger::schema / schema_with_description, in the same property order."""
    props = {
        "main_subject": {"type": "string"},
        "subjects": {"type": "array", "items": {"type": "string"}},
        "conditions": {"type": "array", "items": {"type": "string"}},
        "genre": {"type": "string", "enum": GENRES},
        "confidence": {"type": "number"},
    }
    if with_description:
        props["description"] = {"type": "string"}
    return {"type": "object", "properties": props, "required": list(props), "additionalProperties": False}


# The whitespace llama-server's own grammar from a schema allows: none, a space,
# or a newline and an indent.
PRETTY_SPACE = r' | " " | "\n"{1,2} [ \t]{0,20}'


def one_line_grammar(with_description=False, cap=None, space=' " "?'):
    """The GBNF llama-server builds from `schema`, except that whitespace is at
    most one space: no newlines or indents. The reply is the same JSON, pretty-
    printed less, so fewer tokens. (Allowing *no* whitespace breaks the model.)
    `cap` bounds each list, so a model stuck repeating one word ("rock",
    "rock", …) ends the list instead of running out of tokens mid-JSON."""
    genre = " | ".join(f'"\\"{g}\\""' for g in GENRES)
    desc = ' "," space description-kv' if with_description else ""
    more = f"{{0,{cap - 1}}}" if cap else "*"
    rules = [
        r'char ::= [^"\\\x7F\x00-\x1F] | [\\] (["\\bfnrt] | "u" [0-9a-fA-F]{4})',
        'string ::= "\\"" char* "\\""',
        f'space ::={space}',
        f'list ::= "[" space (string ("," space string){more})? space "]"',
        'number ::= "-"? ([0] | [1-9] [0-9]{0,15}) ("." [0-9]{1,16})? ([eE] [-+]? [0-9]{1,16})?',
        f"genre ::= ({genre})",
        'main-subject-kv ::= "\\"main_subject\\"" space ":" space string',
        'subjects-kv ::= "\\"subjects\\"" space ":" space list',
        'conditions-kv ::= "\\"conditions\\"" space ":" space list',
        'genre-kv ::= "\\"genre\\"" space ":" space genre',
        'confidence-kv ::= "\\"confidence\\"" space ":" space number',
        'description-kv ::= "\\"description\\"" space ":" space string',
        'root ::= "{" space main-subject-kv "," space subjects-kv "," space conditions-kv "," space '
        f'genre-kv "," space confidence-kv{desc} space "}}"',
    ]
    return "\n".join(rules) + "\n"


# --- photos --------------------------------------------------------------------

# Clear-cut genres of the labelled library scenes (qwen_tag.py), by library id.
GENRE_BY_SCENE = {"macro": {"hydrangea", "bee", "sunflower"}, "wildlife": {"gull", "aerial_bird"},
                  "landscape": {"clifftop_bay", "kynance", "town_view", "sea_cliff", "cove", "willow"},
                  "portrait": {"door_portrait", "headphones"},
                  "architecture": {"castle", "church_ext", "chapel", "clocktower"},
                  "still-life": {"ice_cream", "cafe"}}


def labelled_photos():
    """(path, genre) for the 71 photos of the test library whose genre is clear-cut."""
    sys.path.insert(0, HERE)
    from queries import SCENE
    genre_of = {s: g for g, ss in GENRE_BY_SCENE.items() for s in ss}
    con = sqlite3.connect(f"file:{os.path.join(APP, 'library.db')}?mode=ro", uri=True)
    # The labels name photos by id in one catalogue; in any other, those ids
    # are other photos, and the scores would be nonsense.
    first = con.execute("select filename from photos where id = 1").fetchone()
    if not first or first[0] != "DSC00007.ARW":
        raise SystemExit("--labelled needs the library queries.py was labelled from; sample with --n instead")
    rows = con.execute("select id, preview_path from photos where variant = 0 order by id").fetchall()
    return [(p, genre_of[SCENE[i]]) for i, p in rows
            if SCENE.get(i) in genre_of and p and os.path.exists(p)]


def sampled_photos(n, folder=None):
    folder = folder or os.path.join(APP, "thumbs")
    files = sorted(f for f in os.listdir(folder) if f.endswith("_p.jpg"))
    step = max(1, len(files) // n)
    return [(os.path.join(folder, f), None) for f in files[::step][:n]]


def prepare(path, edge):
    """tagger::ollama::prepare_image: fit the longest edge, JPEG q85; smaller ones untouched."""
    raw = open(path, "rb").read()
    im = Image.open(io.BytesIO(raw))
    if max(im.size) <= edge:
        return base64.b64encode(raw).decode()
    im = im.convert("RGB")
    im.thumbnail((edge, edge), Image.BILINEAR)
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=85)
    return base64.b64encode(buf.getvalue()).decode()


# --- requests ------------------------------------------------------------------

def post(url, body, timeout=900):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def ollama_request(host, model, task, b64, opts):
    body = {"model": model, "messages": [{"role": "user", "content": PROMPTS[task], "images": [b64]}],
            "stream": False, "keep_alive": "30m"}
    if task == "tag":
        body.update(format=schema(), options={"temperature": 0.0, "num_ctx": 4096, "num_predict": 256})
    elif task == "describe":
        body.update(options={"temperature": 0.2, "num_ctx": 4096})
    else:
        body.update(format=schema(True), options={"temperature": 0.0, "num_ctx": 4096, "num_predict": 512})
    r = post(f"{host}/api/chat", body)
    return {"text": r["message"]["content"], "prompt_n": r.get("prompt_eval_count", 0),
            "prompt_ms": r.get("prompt_eval_duration", 0) / 1e6,
            "gen_n": r.get("eval_count", 0), "gen_ms": r.get("eval_duration", 0) / 1e6}


def llama_request(host, task, b64, opts):
    body = {"messages": [{"role": "user", "content": [
        {"type": "text", "text": PROMPTS[task]},
        {"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{b64}"}}]}]}
    if task == "describe":
        body.update(temperature=0.2, max_tokens=512)
    else:
        body.update(temperature=0.0, max_tokens=256 if task == "tag" else 512)
        if opts.grammar == "oneline":
            body["grammar"] = one_line_grammar(task == "both")
        elif opts.grammar == "capped":
            body["grammar"] = one_line_grammar(task == "both", cap=8)
        elif opts.grammar == "capped-pretty":
            # The cap alone, with the whitespace llama-server's own grammar allows.
            body["grammar"] = one_line_grammar(task == "both", cap=8, space=PRETTY_SPACE)
        else:
            body["response_format"] = {"type": "json_schema", "json_schema": {"schema": schema(task == "both")}}
    if opts.no_think:
        body["chat_template_kwargs"] = {"enable_thinking": False}
    if opts.sampling:
        body.update(json.loads(opts.sampling))
    r = post(f"{host}/v1/chat/completions", body)
    t = r.get("timings", {})
    return {"text": r["choices"][0]["message"].get("content") or "", "prompt_n": t.get("prompt_n", 0),
            "cache_n": t.get("cache_n", 0), "prompt_ms": t.get("prompt_ms", 0.0),
            "gen_n": t.get("predicted_n", 0), "gen_ms": t.get("predicted_ms", 0.0)}


def parse_tags(text):
    s = text.strip()
    if s.startswith("```"):
        s = s.strip("`")
        s = s[4:] if s.startswith("json") else s
    try:
        v = json.loads(s)
    except ValueError:
        return None
    return v if isinstance(v, dict) and v.get("genre") in GENRES and isinstance(v.get("subjects"), list) else None


def bench(opts, photos=None):
    if opts.list:
        photos = [(p.strip(), None) for p in open(opts.list, encoding="utf-8") if p.strip()]
    photos = photos or (labelled_photos() if opts.labelled else sampled_photos(opts.n + opts.warmup, opts.photos))
    if opts.labelled or opts.list:
        photos = photos[: opts.warmup] + photos  # warm on two of them, then time all
    images = [(prepare(p, opts.edge), g) for p, g in photos]

    def one(item):
        b64, _ = item
        start = time.perf_counter()
        if opts.mode == "ollama":
            r = ollama_request(opts.host, opts.model, opts.task, b64, opts)
        else:
            r = llama_request(opts.host, opts.task, b64, opts)
        r["latency"] = time.perf_counter() - start
        return r

    # The first request after a load can pay one-off costs (shader compiles,
    # kernel JIT); what the user waits through, so it is recorded, not timed in.
    warm = [round(one(item)["latency"], 2) for item in images[: opts.warmup]]
    timed = images[opts.warmup:]
    start = time.perf_counter()
    with ThreadPoolExecutor(max_workers=opts.conc) as pool:
        results = list(pool.map(one, timed))
    wall = time.perf_counter() - start

    for r, (_, genre) in zip(results, timed):
        tags = parse_tags(r["text"]) if opts.task != "describe" else None
        r["valid"] = bool(r["text"].strip()) if opts.task == "describe" else (
            tags is not None and (opts.task == "tag" or bool(str(tags.get("description", "")).strip())))
        r["genre_ok"] = None if (genre is None or tags is None) else tags["genre"] == genre

    mean = lambda k: statistics.mean(r.get(k, 0) for r in results)
    graded = [r["genre_ok"] for r in results if r["genre_ok"] is not None]
    s = {"label": opts.label, "mode": opts.mode, "model": opts.model, "task": opts.task, "edge": opts.edge,
         "conc": opts.conc, "grammar": opts.grammar, "n": len(results), "s_per_photo": wall / len(results),
         "latency": mean("latency"), "prompt_tokens": mean("prompt_n"), "cached_tokens": mean("cache_n"),
         "prompt_ms": mean("prompt_ms"), "gen_tokens": mean("gen_n"), "gen_ms": mean("gen_ms"),
         "valid": sum(r["valid"] for r in results), "warmup_s": warm,
         "genre_acc": (sum(graded) / len(graded)) if graded else None}
    acc = f"  genre {s['genre_acc']:.2f} ({sum(graded)}/{len(results)})" if graded else ""
    print(f"{opts.label:34s} {opts.task:8s} {opts.edge:>4d}px x{opts.conc}  {s['s_per_photo']:.2f} s/photo  "
          f"latency {s['latency']:.2f}s  prompt {s['prompt_tokens']:.0f} tok ({s['cached_tokens']:.0f} cached) "
          f"{s['prompt_ms']:.0f} ms  gen {s['gen_tokens']:.0f} tok {s['gen_ms']:.0f} ms  "
          f"valid {s['valid']}/{len(results)}{acc}  warm-up {warm}", flush=True)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "a", encoding="utf-8") as f:
        f.write(json.dumps({**s, "photos": [p for p, _ in photos[opts.warmup:]],
                            "replies": [r["text"] for r in results]}) + "\n")
    return s


# --- a managed llama-server ------------------------------------------------------

def server_args(model, mmproj, slots, port, device=None, threads=None, jinja=False, extra=()):
    """sidecar::llama_server_args, plus the device and thread count the spike
    varies. `jinja` swaps the fixed ChatML template for the model's own (needed
    to turn Qwen3.5's thinking off)."""
    args = ["--model", model, "--mmproj", mmproj, "--host", "127.0.0.1", "--port", str(port),
            "--no-webui", "--offline", "--ctx-size", str(4096 * slots), "--parallel", str(slots),
            "--cache-type-k", "q8_0", "--cache-type-v", "q8_0", "--flash-attn", "on", "--cache-ram", "0",
            "--image-min-tokens", "64", "--image-max-tokens", "1280"]
    args += ["--jinja"] if jinja else ["--no-jinja", "--chat-template", "chatml"]
    if device:
        args += ["--device", device]
    if threads:
        args += ["--threads", str(threads), "--threads-batch", str(threads)]
    return args + list(extra)


def serve(opts):
    port = 41440
    log = os.path.join(HERE, "out", f"llama-server-{re.sub(r'[^a-z0-9]+', '-', opts.label.lower())}.log")
    os.makedirs(os.path.dirname(log), exist_ok=True)
    args = [opts.binary] + server_args(opts.gguf, opts.mmproj, opts.slots, port, opts.device, opts.threads,
                                       opts.no_think, opts.extra or ())
    started = time.perf_counter()
    flags = 0x08000000 if sys.platform == "win32" else 0  # CREATE_NO_WINDOW
    # Ollama's build loads its GPU backend from the working directory (its
    # lib/ollama/vulkan or cuda_v13 folder), so that can be named with --cwd.
    cwd = opts.cwd or os.path.dirname(opts.binary)
    env = {**os.environ, "PATH": cwd + os.pathsep + os.environ.get("PATH", "")}
    proc = subprocess.Popen(args, stdout=open(log, "w"), stderr=subprocess.STDOUT,
                            cwd=cwd, env=env, creationflags=flags)
    try:
        while True:
            if proc.poll() is not None:
                raise SystemExit(f"llama-server exited; see {log}")
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                    if r.status == 200:
                        break
            except Exception:
                pass
            time.sleep(0.25)
        print(f"{opts.label}: loaded in {time.perf_counter() - started:.1f}s", flush=True)
        for t in opts.tasks:
            task, edge = t.split(":")
            run = argparse.Namespace(**{**vars(opts), "mode": "llama", "host": f"http://127.0.0.1:{port}",
                                        "task": task, "edge": int(edge), "conc": opts.conc or opts.slots})
            bench(run)
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--n", type=int, default=24, help="photos to time (sampled mode)")
    common.add_argument("--warmup", type=int, default=2)
    common.add_argument("--labelled", action="store_true", help="the 71 clear-cut-genre photos, scored")
    common.add_argument("--photos", help="a folder of *_p.jpg previews (default: the app's)")
    common.add_argument("--list", help="a file of image paths, one per line, to run in order")
    common.add_argument("--label", default="")
    common.add_argument("--model", default="qwen3-vl:4b-instruct", help="Ollama tag (ollama mode)")
    common.add_argument("--grammar", choices=["schema", "oneline", "capped", "capped-pretty"], default="schema",
                        help="oneline: JSON on one line; capped: that, with lists of at most 8")
    common.add_argument("--no-think", action="store_true", help="Qwen3.5: enable_thinking=false (needs --jinja)")
    common.add_argument("--sampling", help='more request fields as JSON, e.g. \'{"repeat_penalty": 1.1}\'')

    b = sub.add_parser("bench", parents=[common], help="time one task against a running server")
    b.add_argument("mode", choices=["ollama", "llama"])
    b.add_argument("host")
    b.add_argument("task", choices=list(PROMPTS))
    b.add_argument("edge", type=int)
    b.add_argument("conc", type=int)

    s = sub.add_parser("serve", parents=[common], help="start llama-server, run tasks, stop it")
    s.add_argument("binary")
    s.add_argument("gguf")
    s.add_argument("mmproj")
    s.add_argument("--slots", type=int, default=2)
    s.add_argument("--conc", type=int, default=0, help="requests in flight (default: slots)")
    s.add_argument("--device")
    s.add_argument("--threads", type=int)
    s.add_argument("--cwd", help="working directory for llama-server (Ollama's build: its backend folder)")
    s.add_argument("--tasks", nargs="+", default=["tag:640", "describe:1280", "both:1280"])
    s.add_argument("--extra", nargs=argparse.REMAINDER, help="more llama-server flags")

    opts = ap.parse_args()
    if opts.cmd == "bench":
        bench(opts)
    else:
        serve(opts)


if __name__ == "__main__":
    main()
