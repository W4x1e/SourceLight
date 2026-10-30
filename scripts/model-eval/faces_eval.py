"""Face embedders on the shipped pipeline's own aligned crops, scored against
faces_truth.py. Pairwise separation (threshold-free), then clustering with the
real people::cluster via the probe binary."""
import json, os, subprocess, sys, itertools
import numpy as np
from PIL import Image
from common import session, APP
from faces_truth import LABEL

HERE = os.path.dirname(os.path.abspath(__file__))
PROBE = os.path.join(HERE, "..", "..", "src-tauri", "target", "release", "probe_people.exe")
rows = [json.loads(l) for l in open(os.path.join(HERE, "faces", "faces.jsonl"))]
crops = [np.asarray(Image.open(os.path.join(HERE, "faces", "aligned", f"{r['photo']}_{r['n']}.png")).convert("RGB"),
                    np.float32) for r in rows]


def l2(x):
    return x / np.linalg.norm(x, axis=-1, keepdims=True)


def run(path, norm, flip):
    s = session(path, "cpu")
    name = s.get_inputs()[0].name
    out = []
    for c in crops:
        x = norm(c).transpose(2, 0, 1)[None].astype(np.float32)
        e = s.run(None, {name: x})[0][0]
        if flip:
            e = l2(e) + l2(s.run(None, {name: x[:, :, :, ::-1].copy()})[0][0])
        out.append(e)
    return l2(np.array(out))


MODELS = {
    "facex-nano": (os.path.join(APP, "models", "faces", "facex.onnx"), lambda c: (c - 127.5) / 128.0),
    "sface": (os.path.join(HERE, "models", "faces", "sface.onnx"), lambda c: c),
    "auraface": (os.path.join(HERE, "models", "faces", "auraface.onnx"), lambda c: (c - 127.5) / 127.5),
}


def pairs(E, idx):
    same, diff = [], []
    for a, b in itertools.combinations(idx, 2):
        s = float(E[a] @ E[b])
        (same if LABEL[a] == LABEL[b] else diff).append(s)
    return np.array(same), np.array(diff)


def auc(pos, neg):
    return float(np.mean([(p > n) + 0.5 * (p == n) for p in pos for n in neg]))


def cluster(E, idx, t=None):
    path = os.path.join(HERE, "faces", "_emb.json")
    json.dump({"embeddings": E[idx].tolist()}, open(path, "w"))
    args = [PROBE, "cluster", path] + ([str(v) for v in t] if t else [])
    return json.loads(subprocess.check_output(args, text=True))


def score(labels, idx):
    """Pairwise precision/recall of the clustering, and how many groups each
    real identity (with 2+ groupable faces) is split into."""
    tp = fp = fn = 0
    for (i, a), (j, b) in itertools.combinations(list(enumerate(idx)), 2):
        same_true, same_pred = LABEL[a] == LABEL[b], labels[i] == labels[j]
        tp += same_true and same_pred; fp += (not same_true) and same_pred; fn += same_true and not same_pred
    p = tp / (tp + fp) if tp + fp else 1.0
    r = tp / (tp + fn) if tp + fn else 1.0
    split = {}
    for i, a in enumerate(idx):
        split.setdefault(LABEL[a], set()).add(labels[i])
    frag = {k: len(v) for k, v in split.items() if sum(LABEL[a] == k for a in idx) > 1}
    return dict(precision=round(p, 3), recall=round(r, 3), groups_per_person=frag,
                people_found=len(set(labels)))


if __name__ == "__main__":
    groupable = [i for i, r in enumerate(rows) if r["groupable"]]
    print(f"{len(rows)} faces, {len(groupable)} groupable; identities with 2+ groupable faces:",
          {k: sum(LABEL[i] == k for i in groupable) for k in sorted(set(LABEL[i] for i in groupable))
           if sum(LABEL[i] == k for i in groupable) > 1})
    rust = l2(np.array([r["facex"] for r in rows]))
    report = {}
    for name, (path, norm) in MODELS.items():
        for flip in (False, True):
            E = run(path, norm, flip)
            if name == "facex-nano" and not flip:
                print("  python FaceX vs the app's FaceX: min cos", float(np.min(np.sum(E * rust, 1))).__round__(4))
            same, diff = pairs(E, groupable)
            key = name + (" +flip" if flip else "")
            report[key] = dict(same_median=round(float(np.median(same)), 3), same_min=round(float(same.min()), 3),
                               diff_max=round(float(diff.max()), 3), diff_p99=round(float(np.percentile(diff, 99)), 3),
                               auc=round(auc(same, diff), 4))
            if name == "facex-nano":
                report[key]["cluster_default"] = score(cluster(E, groupable), groupable)
            np.save(os.path.join(HERE, "faces", f"emb_{key.replace(' +', '_')}.npy"), E)
            print(key, report[key], flush=True)
    json.dump(report, open(os.path.join(HERE, "out", "faces.json"), "w"), indent=1)
