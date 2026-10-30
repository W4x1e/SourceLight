"""Scores embeddings against the hand-labelled ground truth in queries.py.

Retrieval: mAP, recall@10 and precision@5 over the query set, ranking every
photo by cosine as `search::rank` does. Similar: is a photo's nearest
neighbour from the same scene? Duplicates: how well cosine separates the
labelled near-duplicate pairs from pairs of different scenes (ROC AUC), and
the best F1 any single threshold achieves.
"""
import sys, json, itertools
import numpy as np
from queries import QUERIES, TRANSLATED, SCENE, NEAR_DUPES


def ap(ranked, rel):
    hits, s = 0, 0.0
    for i, pid in enumerate(ranked, 1):
        if pid in rel:
            hits += 1
            s += hits / i
    return s / len(rel)


def retrieval(ids, ranked_lists, sets):
    m = dict(mAP=[], R10=[], P5=[])
    per = {}
    for (q, rel), ranked in zip(sets, ranked_lists):
        m["mAP"].append(ap(ranked, rel))
        m["R10"].append(len(set(ranked[:10]) & rel) / len(rel))
        m["P5"].append(len(set(ranked[:5]) & rel) / min(5, len(rel)))
        per[q] = round(m["mAP"][-1], 3)
    return {k: round(float(np.mean(v)), 3) for k, v in m.items()}, per


def rank_by(img, ids, qvec):
    return [int(ids[i]) for i in np.argsort(-(img @ qvec), kind="stable")]


def auc(pos, neg):
    pos, neg = np.asarray(pos), np.asarray(neg)
    order = np.argsort(np.concatenate([pos, neg]))
    ranks = np.empty(len(order)); ranks[order] = np.arange(1, len(order) + 1)
    return float((ranks[:len(pos)].sum() - len(pos) * (len(pos) + 1) / 2) / (len(pos) * len(neg)))


def image_side(ids, img):
    idx = {int(p): i for i, p in enumerate(ids)}
    sims = img @ img.T
    np.fill_diagonal(sims, -9)
    nn = np.argmax(sims, axis=1)
    nn_same = float(np.mean([SCENE[int(ids[i])] == SCENE[int(ids[j])] for i, j in enumerate(nn)]))
    pos = [sims[idx[a], idx[b]] for a, b in NEAR_DUPES]
    neg = [sims[idx[a], idx[b]] for a, b in itertools.combinations(sorted(idx), 2) if SCENE[a] != SCENE[b]]
    dupe_set = {tuple(sorted(p)) for p in NEAR_DUPES}
    # Best-threshold F1 for "these two are duplicates" over ALL pairs (same-scene
    # non-duplicates count as negatives too: a burst is a dupe, a reframe isn't).
    allp = [(sims[idx[a], idx[b]], (a, b) in dupe_set) for a, b in itertools.combinations(sorted(idx), 2)]
    best = (0, 0)
    for t in np.linspace(0.5, 0.99, 50):
        tp = sum(1 for s, d in allp if s >= t and d)
        fp = sum(1 for s, d in allp if s >= t and not d)
        fn = len(dupe_set) - tp
        f1 = 2 * tp / (2 * tp + fp + fn) if tp else 0
        best = max(best, (f1, t))
    return dict(similar_nn_same_scene=round(nn_same, 3), dupe_auc=round(auc(pos, neg), 4),
                dupe_best_f1=round(best[0], 3), dupe_best_threshold=round(float(best[1]), 3),
                dupe_pos_mean=round(float(np.mean(pos)), 3), diff_scene_p99=round(float(np.percentile(neg, 99)), 3))


def evaluate(path):
    d = np.load(path)
    ids, img, txt = d["ids"], d["img"], d["txt"]
    n = len(QUERIES)
    en, per_en = retrieval(ids, [rank_by(img, ids, txt[i]) for i in range(n)], QUERIES)
    xl, per_xl = retrieval(ids, [rank_by(img, ids, txt[n + i]) for i in range(len(TRANSLATED))], TRANSLATED)
    return dict(english=en, translated=xl, **image_side(ids, img)), per_en, per_xl


if __name__ == "__main__":
    out = {}
    for path in sys.argv[1:]:
        name = path.split("emb_")[-1].removesuffix(".npz")
        s, per_en, per_xl = evaluate(path)
        out[name] = dict(summary=s, per_query=per_en, per_translated=per_xl)
        print(f"{name:32s} EN {s['english']}  XL {s['translated']}  NN {s['similar_nn_same_scene']}  "
              f"dupeAUC {s['dupe_auc']} F1 {s['dupe_best_f1']}@{s['dupe_best_threshold']}")
    json.dump(out, open("out/retrieval.json", "w"), indent=1)
