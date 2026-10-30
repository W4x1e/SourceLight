"""Apple's on-device model on the app's requests: the driver.

Prepares the photos exactly as vlm_serving.py does (its `prepare`, its
sampling, its prompts), hands them to the Swift CLI (main.swift) one task at a
time, and appends a summary per task to out/vlm_serving.jsonl in the same
shape as vlm_serving.py's, with mode "afm", so the results merge.

  swiftc -O -parse-as-library afm/main.swift -o afm/afm
  python afm/run_afm.py --tasks tag:640 tag:1280 describe:1280 --n 24 --label "apple fm"
"""
import argparse, base64, json, os, statistics, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import vlm_serving as v  # noqa: E402


def run(binary, task, edge, photos, warmup, label, tmp):
    images = []
    for i, p in enumerate(photos):
        path = os.path.join(tmp, f"{task}-{edge}-{i}.jpg")
        with open(path, "wb") as f:
            f.write(base64.b64decode(v.prepare(p, edge)))
        images.append(path)
    job = os.path.join(tmp, f"{task}-{edge}.json")
    with open(job, "w") as f:
        json.dump({"task": task, "prompt": v.PROMPTS[task], "images": images, "genres": v.GENRES}, f)
    out = subprocess.run([binary, job], capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(f"afm failed ({out.returncode}): {out.stderr}")
    # Swift's JSONEncoder leaves out nil fields.
    lines = [{"error": None, "prompt_n": None, "cached_n": None, "gen_n": None, **json.loads(l)}
             for l in out.stdout.splitlines() if l.startswith("{")]
    results = lines[warmup:]

    for r in results:
        if r["error"]:
            r["valid"] = False
        elif task == "describe":
            r["valid"] = bool(r["text"].strip())
        else:
            r["valid"] = v.parse_tags(r["text"]) is not None
    ok = [r for r in results if not r["error"]]
    mean = lambda k: statistics.mean(r[k] for r in ok) if ok else 0
    s = {"label": label, "mode": "afm", "task": task, "edge": edge, "conc": 1, "grammar": "guided",
         "n": len(results), "s_per_photo": statistics.mean(r["latency"] for r in results),
         "latency": statistics.mean(r["latency"] for r in results),
         "prompt_tokens": mean("prompt_n"), "cached_tokens": mean("cached_n"), "gen_tokens": mean("gen_n"),
         "valid": sum(r["valid"] for r in results), "errors": [r["error"] for r in results if r["error"]],
         "genre_acc": None}
    print(f"{label:30s} {task:8s} {edge:>4d}px  {s['s_per_photo']:.2f} s/photo  prompt {s['prompt_tokens']:.0f} tok "
          f"gen {s['gen_tokens']:.0f} tok  valid {s['valid']}/{len(results)}  errors {len(s['errors'])}", flush=True)
    for e in sorted(set(s["errors"])):
        print("   error:", e[:300], flush=True)
    os.makedirs(os.path.dirname(v.OUT), exist_ok=True)
    with open(v.OUT, "a", encoding="utf-8") as f:
        f.write(json.dumps({**s, "photos": photos[warmup:], "replies": [r["text"] for r in results]}) + "\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", default=os.path.join(HERE, "afm"))
    ap.add_argument("--tasks", nargs="+", default=["tag:640", "tag:1280", "describe:1280"])
    ap.add_argument("--n", type=int, default=24)
    ap.add_argument("--warmup", type=int, default=2)
    ap.add_argument("--photos", help="a folder of *_p.jpg previews (default: the app's)")
    ap.add_argument("--label", default="apple fm")
    opts = ap.parse_args()
    photos = [p for p, _ in v.sampled_photos(opts.n + opts.warmup, opts.photos)]
    with tempfile.TemporaryDirectory() as tmp:
        for t in opts.tasks:
            task, edge = t.split(":")
            run(opts.binary, task, int(edge), photos, opts.warmup, opts.label, tmp)


if __name__ == "__main__":
    main()
