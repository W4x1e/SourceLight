"""Grouping policy on the real library: which faces vote, the cluster
thresholds, and an attach stage for faces that may join a person but not
found or link one. Scored over ALL detected faces with an identity (a face
left unassigned counts as its own group), with M1-vs-M2 pairs excluded
because the labeller could not tell whether they are one man."""
import json, itertools, sys
import numpy as np
from faces_eval import rows, cluster, LABEL

E = {k: np.load(f"faces/emb_{k}.npy") for k in ["facex-nano", "sface_flip", "auraface_flip"]}
UNSURE = {frozenset(("M1", "M2"))}


def yaw(r):
    le, re_, n = (np.array(r["landmarks"][i]) for i in (0, 1, 2))
    return float(abs(n[0] - (le[0] + re_[0]) / 2) / max(np.linalg.norm(re_ - le), 1e-6))


def edge(r):
    return min(r["box"][2:])


def assign(Em, vote, attachable, t, attach):
    labels = {}
    if vote:
        lab = cluster(Em, vote, t)
        for i, l in zip(vote, lab):
            labels[i] = l
        groups = {}
        for i in vote:
            groups.setdefault(labels[i], []).append(i)
        cents = {g: Em[m].mean(0) / np.linalg.norm(Em[m].mean(0)) for g, m in groups.items()}
        for i in attachable:
            if cents:
                g, s = max(((g, float(Em[i] @ c)) for g, c in cents.items()), key=lambda x: x[1])
                if s >= attach:
                    labels[i] = g
    return labels


def evaluate(labels):
    ids = [i for i in range(len(rows)) if not LABEL[i].startswith("s") or True]
    tp = fp = fn = 0
    for a, b in itertools.combinations(ids, 2):
        if frozenset((LABEL[a], LABEL[b])) in UNSURE:
            continue
        same = LABEL[a] == LABEL[b]
        pred = a in labels and b in labels and labels[a] == labels[b]
        tp += same and pred; fp += (not same) and pred; fn += same and not pred
    p = tp / (tp + fp) if tp + fp else 1.0
    r = tp / (tp + fn) if tp + fn else 1.0
    frag = {}
    for i in labels:
        if not LABEL[i].startswith("s"):
            frag.setdefault(LABEL[i], set()).add(labels[i])
    return round(p, 3), round(r, 3), round(2 * p * r / max(p + r, 1e-9), 3), \
        {k: len(v) for k, v in sorted(frag.items())}, sum(1 for i in range(len(rows)) if i in labels)


if __name__ == "__main__":
    results = []
    for name, Em in E.items():
        for vote_score, vote_yaw in [(0.60, 99), (0.75, 0.6), (0.75, 0.45), (0.80, 0.6)]:
            base = [i for i, r in enumerate(rows) if edge(r) >= 32]
            vote = [i for i in base if rows[i]["score"] >= vote_score and yaw(rows[i]) <= vote_yaw]
            attachable = [i for i in base if i not in vote]
            for core in np.arange(0.30, 0.80, 0.05):
                for merge in np.arange(0.20, core + 0.001, 0.05):
                    for attach in (9, 0.45, 0.40, 0.35):
                        labels = assign(Em, vote, attachable, (round(core, 2), round(merge, 2), 0.0), attach)
                        p, r, f, frag, n = evaluate(labels)
                        results.append((f, p, r, name, vote_score, vote_yaw, round(core, 2), round(merge, 2), attach, frag, n))
    for name in E:
        for vs, vy in [(0.60, 99), (0.75, 0.6), (0.75, 0.45), (0.80, 0.6)]:
            best = max((x for x in results if x[3] == name and x[4] == vs and x[5] == vy), key=lambda x: (x[0], x[1]))
            print(f"{name:14s} vote score>={vs} yaw<={vy:<4} | F1 {best[0]} P {best[1]} R {best[2]} | core {best[6]} merge {best[7]} attach {best[8]} | split {best[9]} | {best[10]} faces grouped")
    # Robustness: F1 of AuraFace policy across nearby thresholds.
    print("\nAuraFace, vote score>=0.75 yaw<=0.6, attach 0.40 — F1 by (core, merge):")
    for core in (0.40, 0.45, 0.50, 0.55, 0.60):
        print(" ", core, [next(x[0] for x in results if x[3] == "auraface_flip" and x[4] == 0.75 and x[5] == 0.6
                               and x[6] == core and x[7] == m and x[8] == 0.40) for m in (0.25, 0.30, 0.35, 0.40)])
