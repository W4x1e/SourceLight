"""The vision model on MLX: the app's own requests, one photo at a time.

Same prompts, image sizes and photos as vlm_serving.py (imported from it), run
in-process through mlx_vlm's Python API rather than a server, so the numbers
compare with `vlm_serving.py serve ... --slots 1`. By default there is no
grammar: tags are JSON by the prompt alone, and the valid-JSON rate says how
often that holds. `--schema` constrains tags to the app's JSON schema
(vlm_serving.schema) through mlx_vlm's llguidance logits processor — the
counterpart of llama-server's json_schema.

  python vlm_mlx.py <model dir> --tasks tag:640 describe:1280 both:1280 --n 24 --label "mlx 4bit"

Needs Python 3.10+ with `mlx-vlm` (MIT). Results are appended to
out/vlm_serving.jsonl in the same shape as vlm_serving.py's, with mode "mlx".
"""
import argparse, base64, json, os, statistics, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import vlm_serving as v  # noqa: E402

from mlx_vlm import generate, load  # noqa: E402
from mlx_vlm.prompt_utils import apply_chat_template  # noqa: E402
from mlx_vlm.structured import build_json_schema_logits_processor  # noqa: E402

# tagger/llama.rs: tags at temperature 0 and 256 tokens, descriptions at 0.2 and
# 512, tags with a description at 0 and 512.
SETTINGS = {"tag": (0.0, 256), "describe": (0.2, 512), "both": (0.0, 512)}


def run(model, processor, config, task, edge, photos, warmup, label, tmp, constrained=False):
    temperature, max_tokens = SETTINGS[task]
    prompt = apply_chat_template(processor, config, v.PROMPTS[task], num_images=1)
    files = []
    for i, p in enumerate(photos):
        path = os.path.join(tmp, f"{task}-{edge}-{i}.jpg")
        with open(path, "wb") as f:
            f.write(base64.b64decode(v.prepare(p, edge)))
        files.append(path)

    tokenizer = processor.tokenizer if hasattr(processor, "tokenizer") else processor
    guided = constrained and task != "describe"

    def one(path):
        start = time.perf_counter()
        extra = {}
        if guided:
            # A processor holds its parse state, so a fresh one per request.
            extra["logits_processors"] = [
                build_json_schema_logits_processor(tokenizer, v.schema(task == "both"))]
        r = generate(model, processor, prompt, image=[path], temperature=temperature,
                     max_tokens=max_tokens, verbose=False, **extra)
        return {"latency": time.perf_counter() - start, "text": r.text,
                "prompt_n": r.prompt_tokens, "gen_n": r.generation_tokens,
                "prompt_tps": r.prompt_tps, "gen_tps": r.generation_tps, "peak_gb": r.peak_memory,
                "finish": r.finish_reason}

    for path in files[:warmup]:
        one(path)
    results = [one(path) for path in files[warmup:]]

    for r in results:
        tags = v.parse_tags(r["text"]) if task != "describe" else None
        r["valid"] = bool(r["text"].strip()) if task == "describe" else (
            tags is not None and (task == "tag" or bool(str(tags.get("description", "")).strip())))
    mean = lambda k: statistics.mean(r[k] for r in results)
    s = {"label": label, "mode": "mlx", "task": task, "edge": edge, "conc": 1,
         "grammar": "schema" if guided else "none",
         "n": len(results), "s_per_photo": mean("latency"), "latency": mean("latency"),
         "prompt_tokens": mean("prompt_n"), "cached_tokens": 0, "gen_tokens": mean("gen_n"),
         "prompt_tps": mean("prompt_tps"), "gen_tps": mean("gen_tps"),
         "prompt_ms": statistics.mean(1000 * r["prompt_n"] / r["prompt_tps"] for r in results if r["prompt_tps"]),
         "gen_ms": statistics.mean(1000 * r["gen_n"] / r["gen_tps"] for r in results if r["gen_tps"]),
         "peak_gb": max(r["peak_gb"] for r in results), "valid": sum(r["valid"] for r in results),
         "genre_acc": None}
    print(f"{label:30s} {task:8s} {edge:>4d}px  {s['s_per_photo']:.2f} s/photo  prompt {s['prompt_tokens']:.0f} tok "
          f"@ {s['prompt_tps']:.0f} tok/s  gen {s['gen_tokens']:.0f} tok @ {s['gen_tps']:.1f} tok/s  "
          f"peak {s['peak_gb']:.2f} GB  valid {s['valid']}/{len(results)}", flush=True)
    os.makedirs(os.path.dirname(v.OUT), exist_ok=True)
    with open(v.OUT, "a", encoding="utf-8") as f:
        f.write(json.dumps({**s, "photos": photos[warmup:],
                            "replies": [r["text"] for r in results]}) + "\n")
    return s


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("model", help="an MLX model folder, e.g. mlx-community/Qwen3-VL-4B-Instruct-4bit")
    ap.add_argument("--tasks", nargs="+", default=["tag:640", "describe:1280", "both:1280"])
    ap.add_argument("--n", type=int, default=24)
    ap.add_argument("--warmup", type=int, default=2)
    ap.add_argument("--photos", help="a folder of *_p.jpg previews (default: the app's)")
    ap.add_argument("--label", default="mlx")
    ap.add_argument("--schema", action="store_true", help="constrain tags to the app's JSON schema")
    opts = ap.parse_args()

    started = time.perf_counter()
    model, processor = load(opts.model)
    config = model.config
    print(f"{opts.label}: loaded in {time.perf_counter() - started:.1f}s", flush=True)
    photos = [p for p, _ in v.sampled_photos(opts.n + opts.warmup, opts.photos)]
    with tempfile.TemporaryDirectory() as tmp:
        for t in opts.tasks:
            task, edge = t.split(":")
            run(model, processor, config, task, int(edge), photos, opts.warmup, opts.label, tmp, opts.schema)


if __name__ == "__main__":
    main()
