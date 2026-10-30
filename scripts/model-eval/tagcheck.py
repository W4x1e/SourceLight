"""SigLIP 2 spike: how each model ranks the built-in tagger's own vocabulary.

Threshold-free on purpose. The shipped selection (softmax at CLIP's logit scale
of 100) is tuned for CLIP; SigLIP scores are sigmoid-calibrated and need their
own. So this asks what does not depend on tuning: for photos whose scene has an
unambiguous vocabulary label, where does that label rank among the 1,600? And
for clear-cut genres, is the top genre right?
"""
import os, sys, json, re
import numpy as np
from queries import SCENE
from embed import Clip, Siglip, l2

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SUBJECTS = open(os.path.join(REPO, "src-tauri", "resources", "tagger", "subjects.txt"), encoding="utf-8").read()
TEMPLATES = ["a photo of a {}.", "a photo of the {}.", "a close-up photo of {}.", "a photograph showing {}."]
GENRE_PROMPTS = {
    "landscape": "a landscape photograph of scenery", "wildlife": "a wildlife photograph of an animal",
    "portrait": "a portrait photograph of a person", "street": "a candid street photograph",
    "macro": "an extreme close-up macro photograph", "architecture": "an architectural photograph of a building",
}

labels = []
for line in SUBJECTS.splitlines():
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    name = line.split(">")[0].strip()
    if name.startswith("["):
        continue  # groups are never scored
    labels.append(name)

# Scene -> labels any of which counts as right.
KEY = {
    "viaduct": {"viaduct"}, "gull": {"seagull"}, "bee": {"bee"}, "sunflower": {"sunflower"},
    "castle": {"castle"}, "boats": {"boat", "harbour", "fishing boat"}, "stained": {"stained glass"},
    "red_stairs": {"staircase"}, "ice_cream": {"ice cream"}, "hotel": {"hotel", "bicycle"},
    "tomatoes": {"tomato", "vegetable garden"}, "clifftop_bay": {"beach", "bay", "cliff"},
    "kynance": {"cliff", "coast", "bay"}, "clocktower": {"clock tower"}, "cafe": {"coffee"},
    "hydrangea": {"hydrangea"}, "hedge_path": {"hedge"}, "church_ext": {"church"}, "chapel": {"church"},
    "willow_duck": {"duck"}, "allotment": {"vegetable garden", "garden"},
}
GENRE = {
    "macro": {"hydrangea", "bee", "sunflower"}, "wildlife": {"gull", "aerial_bird"},
    "landscape": {"clifftop_bay", "kynance", "town_view", "sea_cliff", "cove", "willow"},
    "portrait": {"door_portrait", "headphones"}, "architecture": {"castle", "church_ext", "chapel", "clocktower"},
}
GENRE_OF = {s: g for g, ss in GENRE.items() for s in ss}


def run(model, emb_path):
    d = np.load(emb_path)
    ids, img = d["ids"], d["img"]
    prompts = [t.format(l) for l in labels for t in TEMPLATES]
    T = np.concatenate([model.texts(prompts[i:i + 64]) for i in range(0, len(prompts), 64)])
    T = l2(l2(T).reshape(len(labels), len(TEMPLATES), -1).mean(1))
    G = l2(model.texts(list(GENRE_PROMPTS.values())))
    sims = img @ T.T
    gsims = img @ G.T
    ranks, hit1, hit5, top = [], 0, 0, {}
    for i, pid in enumerate(ids):
        sc = SCENE[int(pid)]
        order = np.argsort(-sims[i])
        top[int(pid)] = [labels[j] for j in order[:5]]
        if sc in KEY:
            best = min(int(np.where(order == labels.index(k))[0][0]) for k in KEY[sc]) + 1
            ranks.append(best); hit1 += best == 1; hit5 += best <= 5
    g_ok = [list(GENRE_PROMPTS)[int(np.argmax(gsims[i]))] == GENRE_OF[SCENE[int(p)]]
            for i, p in enumerate(ids) if SCENE[int(p)] in GENRE_OF]
    n = len(ranks)
    return dict(photos=n, hit1=round(hit1 / n, 3), hit5=round(hit5 / n, 3),
                mrr=round(float(np.mean([1 / r for r in ranks])), 3), median_rank=int(np.median(ranks)),
                genre_photos=len(g_ok), genre_acc=round(float(np.mean(g_ok)), 3)), top


if __name__ == "__main__":
    out = {}
    for name, M, path in [("clip-b32", Clip, "out/emb_clip-b32_dml.npz"),
                          ("siglip2-fp16", lambda: Siglip("fp16"), "out/emb_siglip2-b16-224-fp16_dml.npz")]:
        s, top = run(M(), path)
        out[name] = dict(summary=s, top5=top)
        print(name, s, flush=True)
    json.dump(out, open("out/tags.json", "w"), indent=1)
