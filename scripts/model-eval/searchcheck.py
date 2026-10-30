"""Scores the app's own search path (probe_search) against queries.py, and
its tags against tagcheck.KEY — the Rust end-to-end twin of evaluate.py.

Usage: searchcheck.py <probe_search.exe> <siglip2 dir> <library.db copy>
Set ORT_DYLIB_PATH to the vendored DirectML onnxruntime.dll to test what ships.
"""
import json, os, subprocess, sys
from queries import QUERIES, TRANSLATED, SCENE
from evaluate import retrieval
from tagcheck import KEY

probe, model_dir, db = sys.argv[1:4]
sets = QUERIES + TRANSLATED
qpath = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out", "queries.json")
json.dump([q for q, _ in sets], open(qpath, "w", encoding="utf-8"), ensure_ascii=False)
out = subprocess.run([probe, model_dir, db, qpath], capture_output=True, text=True, encoding="utf-8")
if out.returncode:
    sys.exit(out.stderr)
print(out.stderr.strip().splitlines()[-1])
lines = [json.loads(l) for l in out.stdout.splitlines() if l.strip()]
ranked = {l["query"]: l["ranked"] for l in lines if "query" in l}
tags = {l["photo"]: l["tags"] for l in lines if "photo" in l}

n = len(QUERIES)
en, _ = retrieval(None, [ranked[q] for q, _ in QUERIES], QUERIES)
xl, _ = retrieval(None, [ranked[q] for q, _ in TRANSLATED], TRANSLATED)
print("search, English    ", en)
print("search, translated ", xl)

hit = total = 0; count = []
for pid, t in tags.items():
    if not t:
        continue
    count.append(len(t["subjects"]))
    sc = SCENE[pid]
    if sc in KEY:
        total += 1
        hit += any(s in KEY[sc] for s in t["subjects"])
print(f"tags: key subject on {hit}/{total} ({hit / total:.0%}), {sum(count) / len(count):.2f} subjects per photo")
