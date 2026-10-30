"""Calibrates face embedders on LFW (2,200 labelled pairs, 1,100 same), run
through the shipped detector + alignment by `probe_people dump`.

For each image the most central detected face is taken (LFW centres the
subject). Reports verification AUC, and the cosine at fixed false-match rates
between different people, which is what a grouping threshold has to respect.
"""
import json, os, sys
import numpy as np
from PIL import Image
from common import session, APP

HERE = os.path.dirname(os.path.abspath(__file__))
D = os.path.join(HERE, "lfw")
pairs = json.load(open(os.path.join(D, "pairs.json")))
faces = [json.loads(l) for l in open(os.path.join(D, "faces", "faces.jsonl"))]

best = {}
for f in faces:
    w, h = f["img"]
    cx, cy = f["box"][0] + f["box"][2] / 2, f["box"][1] + f["box"][3] / 2
    d = (cx - w / 2) ** 2 + (cy - h / 2) ** 2
    if f["photo"] not in best or d < best[f["photo"]][0]:
        best[f["photo"]] = (d, f)
face = {p: v[1] for p, v in best.items()}
usable = [p for p in pairs if p["a"] in face and p["b"] in face]
print(f"{len(face)} of 4400 images have a face; {len(usable)} of {len(pairs)} pairs usable")


def l2(x):
    return x / np.linalg.norm(x, axis=-1, keepdims=True)


MODELS = {
    "facex-nano": (os.path.join(APP, "models", "faces", "facex.onnx"), lambda c: (c - 127.5) / 128.0),
    "sface": (os.path.join(HERE, "models", "faces", "sface.onnx"), lambda c: c),
    "auraface": (os.path.join(HERE, "models", "faces", "auraface.onnx"), lambda c: (c - 127.5) / 127.5),
}
ids = sorted(face)
crops = {p: np.asarray(Image.open(os.path.join(D, "faces", "aligned", f"{p}_{face[p]['n']}.png")).convert("RGB"),
                       np.float32) for p in ids}
report = {}
for name, (path, norm) in MODELS.items():
    s = session(path, "cpu")
    inp = s.get_inputs()[0].name
    batch1 = s.get_inputs()[0].shape[0] == 1
    E = {}
    for i in range(0, len(ids), 1 if batch1 else 64):
        chunk = ids[i:i + (1 if batch1 else 64)]
        x = np.stack([norm(crops[p]).transpose(2, 0, 1) for p in chunk]).astype(np.float32)
        e = l2(s.run(None, {inp: x})[0]) + l2(s.run(None, {inp: x[:, :, :, ::-1].copy()})[0])
        for p, v in zip(chunk, l2(e)):
            E[p] = v
    sims = np.array([float(E[p["a"]] @ E[p["b"]]) for p in usable])
    same = np.array([p["same"] for p in usable], bool)
    pos, neg = sims[same], sims[~same]
    auc = float(np.mean(pos[:, None] > neg[None, :]))
    # Thresholds that let through 1% / 0.1% of different-person pairs, and the
    # share of same-person pairs they still accept.
    t1, t01 = float(np.quantile(neg, 0.99)), float(np.quantile(neg, 0.999))
    acc = max(((sims >= t) == same).mean() for t in np.linspace(-0.2, 1, 241))
    report[name] = dict(auc=round(auc, 4), best_accuracy=round(float(acc), 4),
                        same_median=round(float(np.median(pos)), 3), diff_median=round(float(np.median(neg)), 3),
                        diff_max=round(float(neg.max()), 3),
                        thr_far1=round(t1, 3), tar_at_far1=round(float((pos >= t1).mean()), 3),
                        thr_far01=round(t01, 3), tar_at_far01=round(float((pos >= t01).mean()), 3))
    print(name, report[name], flush=True)
json.dump(report, open(os.path.join(HERE, "out", "faces_lfw.json"), "w"), indent=1)
