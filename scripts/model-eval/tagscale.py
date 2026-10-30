"""Calibrates the tagger's LOGIT_SCALE for SigLIP 2 with a faithful port of
tagger/builtin.rs select_subjects + the genre/condition picks.

For each scale: tags per photo, and precision = share of photos (with a
clear key label, tagcheck.KEY) whose claimed subjects include the key label
or one of its ancestors/descendants... kept strict: the key label itself or
its descendants. Compared against CLIP at the shipped 100."""
import os, sys
import numpy as np
from queries import SCENE
from embed import Clip, Siglip, l2
import embed
embed.provider = "dml"
from tagcheck import KEY, GENRE_OF, GENRE_PROMPTS, TEMPLATES


REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
raw = []
for line in open(os.path.join(REPO, "src-tauri/resources/tagger/subjects.txt"), encoding="utf-8"):
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    name, parent = (line.split(">", 1) + [None])[:2]
    name = name.strip(); parent = parent.strip() if parent else None
    group = name.startswith("[") and name.endswith("]")
    raw.append((name.strip("[]").strip(), parent, group))
index = {n: i for i, (n, _, _) in enumerate(raw)}
LABELS = [dict(name=n, parent=index.get(p) if p else None, group=g) for n, p, g in raw]
CONDITIONS = ["golden hour", "overcast", "backlit", "night", "sunny", "foggy", "rainy", "indoor lighting",
              "high contrast", "soft light"]


def softmax(x, t):
    x = np.asarray(x, np.float64); m = np.max(x[np.isfinite(x)])
    e = np.where(np.isfinite(x), np.exp((x - m) * t), 0.0)
    return e / max(e.sum(), 1e-8)


def ancestors(i):
    out = []; cur = LABELS[i]["parent"]
    while cur is not None:
        out.append(cur); cur = LABELS[cur]["parent"]
    return out


def select(sims, scale, min_prob=0.06, floor=0.01, share=0.5):
    masked = np.array([-np.inf if l["group"] else s for s, l in zip(sims, LABELS)])
    p = softmax(masked, scale)
    pooled = p.copy()
    for i, pi in enumerate(p):
        if pi < floor:
            continue
        for a in ancestors(i):
            pooled[a] += pi
    is_anc = lambda a, d: a in ancestors(d)
    cand = [i for i in range(len(LABELS)) if not LABELS[i]["group"] and pooled[i] >= min_prob]
    kept = [a for a in cand if max([pooled[d] for d in cand if is_anc(a, d)] or [0]) < share * pooled[a]]
    claimed = [d for d in kept if not any(is_anc(a, d) and pooled[d] < share * pooled[a] for a in kept)]
    if not claimed:
        claimed = [max((i for i in range(len(LABELS)) if not LABELS[i]["group"]), key=lambda i: p[i])]
    claimed.sort(key=lambda i: -pooled[i]); claimed = claimed[:4]
    out = list(claimed)
    for c in claimed:
        for a in ancestors(c):
            if not LABELS[a]["group"] and a not in out:
                out.append(a)
    return [LABELS[i]["name"] for i in out[:8]], [LABELS[i]["name"] for i in claimed]


def run(model, emb_path, scales):
    d = np.load(emb_path); ids, img = d["ids"], d["img"]
    names = [l["name"] for l in LABELS]
    prompts = [t.format(n) for n in names for t in TEMPLATES]
    T = np.concatenate([model.texts(prompts[i:i + 64]) for i in range(0, len(prompts), 64)])
    T = l2(l2(T).reshape(len(names), len(TEMPLATES), -1).mean(1))
    G = l2(model.texts(list(GENRE_PROMPTS.values())))
    C = l2(model.texts([f"a photo taken in {c}" for c in CONDITIONS]))
    S, GS, CS = img @ T.T, img @ G.T, img @ C.T
    for scale in scales:
        hit = n = 0; tags = []; cond = []; gconf = []
        for i, pid in enumerate(ids):
            allt, claimed = select(S[i], scale)
            tags.append(len(allt))
            pc = softmax(CS[i], scale); top = np.argsort(-pc)[:2]
            cond.append(sum(pc[j] > 0.25 for j in top))
            gconf.append(float(softmax(GS[i], scale).max()))
            sc = SCENE[int(pid)]
            if sc in KEY:
                n += 1
                keyset = set(KEY[sc])
                hit += any(t in keyset for t in allt)
        print(f"  scale {scale:4d}: key label tagged on {hit}/{n} ({hit / n:.0%}), tags/photo {np.mean(tags):.2f}, "
              f"conditions/photo {np.mean(cond):.2f}, genre confidence median {np.median(gconf):.2f}", flush=True)


if __name__ == "__main__":
    print("CLIP B/32")
    run(Clip(), "out/emb_clip-b32_dml.npz", [100])
    print("SigLIP 2")
    run(Siglip("fp16"), "out/emb_siglip2-b16-224-fp16_dml.npz", [60, 80, 100, 130, 160, 200, 250])
