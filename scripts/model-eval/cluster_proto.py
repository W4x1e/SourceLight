"""Python port of people::cluster (to check against the Rust) and the
average-linkage variant. `avg=True` judges a merge by the mean pairwise cosine
between two groups' members — dot(sum_a, sum_b) / (n_a * n_b) — instead of the
cosine of their re-normalised centroids, which inflates for mixed groups."""
import numpy as np


def find(p, i):
    while p[i] != i:
        p[i] = p[p[i]]
        i = p[i]
    return i


def cluster(E, core, merge, floor, avg=False):
    n = len(E)
    if n == 0:
        return []
    S = E @ E.T
    p = list(range(n))
    ii, jj = np.where(np.triu(S >= core, 1))
    for i, j in zip(ii, jj):
        a, b = find(p, i), find(p, j)
        if a != b:
            p[b] = a
    roots = {}
    for i in range(n):
        roots.setdefault(find(p, i), []).append(i)
    groups = sorted(roots.values(), key=lambda g: g[0])
    sums = [E[g].sum(0) for g in groups]

    def sim(a, b):
        if avg:
            return float(sums[a] @ sums[b]) / (len(groups[a]) * len(groups[b]))
        ca, cb = sums[a] / np.linalg.norm(sums[a]), sums[b] / np.linalg.norm(sums[b])
        return float(ca @ cb)

    while True:
        merged = False
        a = 0
        while a < len(groups):
            b = a + 1
            while b < len(groups):
                if sim(a, b) >= merge:
                    groups[a] = sorted(groups[a] + groups.pop(b))
                    sums[a] = sums[a] + sums.pop(b)
                    merged = True
                else:
                    b += 1
            a += 1
        if not merged:
            break
    evicted = []
    for g, grp in enumerate(groups):
        if len(grp) < 3:
            continue
        c = sums[g] / np.linalg.norm(sums[g])
        keep = [m for m in grp if (float(E[m] @ sums[g]) / len(grp) if avg else float(E[m] @ c)) >= floor]
        evicted += [m for m in grp if m not in keep]
        groups[g] = keep
    groups = [g for g in groups if g] + [[m] for m in evicted]
    groups.sort(key=lambda g: g[0])
    out = [0] * n
    for k, g in enumerate(groups):
        for m in g:
            out[m] = k
    return out
