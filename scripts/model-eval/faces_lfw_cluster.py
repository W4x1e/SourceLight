"""Clustering at scale: every LFW image in the pairs set (identity from its
file name) grouped by the real people::cluster, per embedder and thresholds.
This is where chaining shows: thousands of faces, hundreds of identities.

Scored with pairwise precision/recall, plus:
- split: mean groups per identity that has 2+ faces (1.0 = never split)
- mixed: share of multi-face groups holding more than one identity
"""
import json, os, re, itertools, sys
import numpy as np
import pyarrow.parquet as pq
from PIL import Image
from common import session, APP
from faces_eval import cluster as _cluster

HERE = os.path.dirname(os.path.abspath(__file__))
D = os.path.join(HERE, "lfw")
t = pq.read_table(os.path.join(D, "pairs_test.parquet")).to_pylist()
ident = {}
for k, r in enumerate(t):
    for s in (0, 1):
        ident[2 * k + s] = re.sub(r"_\d+\.jpg$", "", r[f"img_{s}"]["path"])
# The same image appears in several pairs: keep one copy per file name.
first = {}
for k, r in enumerate(t):
    for s in (0, 1):
        first.setdefault(r[f"img_{s}"]["path"], 2 * k + s)
faces = {}
for l in open(os.path.join(D, "faces", "faces.jsonl")):
    f = json.loads(l)
    w, h = f["img"]
    d = (f["box"][0] + f["box"][2] / 2 - w / 2) ** 2 + (f["box"][1] + f["box"][3] / 2 - h / 2) ** 2
    if f["photo"] in set(first.values()) and (f["photo"] not in faces or d < faces[f["photo"]][0]):
        faces[f["photo"]] = (d, f)
ids = sorted(faces)
names = [ident[p] for p in ids]
multi = {n for n in set(names) if names.count(n) > 1}
print(f"{len(ids)} unique faces, {len(set(names))} identities, {len(multi)} with 2+ faces")


def l2(x):
    return x / np.linalg.norm(x, axis=-1, keepdims=True)


MODELS = {
    "facex-nano": (os.path.join(APP, "models", "faces", "facex.onnx"), lambda c: (c - 127.5) / 128.0),
    "sface": (os.path.join(HERE, "models", "faces", "sface.onnx"), lambda c: c),
    "auraface": (os.path.join(HERE, "models", "faces", "auraface.onnx"), lambda c: (c - 127.5) / 127.5),
}


def embed(name):
    cache = os.path.join(D, f"emb_{name}.npy")
    if os.path.exists(cache):
        return np.load(cache)
    path, norm = MODELS[name]
    s = session(path, "cpu")
    inp = s.get_inputs()[0].name
    bs = 1 if s.get_inputs()[0].shape[0] == 1 else 64
    out = []
    for i in range(0, len(ids), bs):
        x = np.stack([norm(np.asarray(Image.open(os.path.join(D, "faces", "aligned", f"{p}_{faces[p][1]['n']}.png"))
                                      .convert("RGB"), np.float32)).transpose(2, 0, 1) for p in ids[i:i + bs]])
        out.append(l2(l2(s.run(None, {inp: x})[0]) + l2(s.run(None, {inp: x[:, :, :, ::-1].copy()})[0])))
    E = np.concatenate(out)
    np.save(cache, E)
    return E


def score(labels):
    by_group, by_name = {}, {}
    for n, g in zip(names, labels):
        by_group.setdefault(g, []).append(n)
        by_name.setdefault(n, set()).add(g)
    tp = sum(sum(v.count(n) * (v.count(n) - 1) // 2 for n in set(v)) for v in by_group.values())
    pred = sum(len(v) * (len(v) - 1) // 2 for v in by_group.values())
    true = sum(names.count(n) * (names.count(n) - 1) // 2 for n in set(names))
    p, r = tp / max(pred, 1), tp / max(true, 1)
    split = np.mean([len(by_name[n]) for n in multi])
    mixed = np.mean([len(set(v)) > 1 for v in by_group.values() if len(v) > 1] or [0])
    return dict(P=round(p, 3), R=round(r, 3), F1=round(2 * p * r / max(p + r, 1e-9), 3),
                split=round(float(split), 2), mixed=round(float(mixed), 3), groups=len(by_group))


if __name__ == "__main__":
    grid = {"facex-nano": [(0.75, 0.50, 0.38), (0.55, 0.40, 0.2), (0.45, 0.35, 0.2), (0.40, 0.30, 0.2)],
            "sface": [(0.60, 0.45, 0.2), (0.50, 0.40, 0.2), (0.45, 0.35, 0.2), (0.40, 0.30, 0.2)],
            "auraface": [(0.60, 0.40, 0.2), (0.50, 0.35, 0.2), (0.45, 0.30, 0.2), (0.40, 0.30, 0.2),
                         (0.35, 0.30, 0.2), (0.30, 0.30, 0.0), (0.50, 0.30, 0.1), (0.45, 0.25, 0.1)]}
    out = {}
    for name, ts in grid.items():
        E = embed(name)
        for th in ts:
            s = score(_cluster(E, list(range(len(ids))), th) if False else _cluster_all(E, th)) \
                if False else None
            path = os.path.join(D, "_emb.json")
            json.dump({"embeddings": E.tolist()}, open(path, "w"))
            import subprocess
            from faces_eval import PROBE
            labels = json.loads(subprocess.check_output([PROBE, "cluster", path] + [str(v) for v in th], text=True))
            s = score(labels)
            out[f"{name} {th}"] = s
            print(f"{name:10s} core {th[0]:.2f} merge {th[1]:.2f} floor {th[2]:.2f} -> {s}", flush=True)
    json.dump(out, open(os.path.join(HERE, "out", "faces_lfw_cluster.json"), "w"), indent=1)
