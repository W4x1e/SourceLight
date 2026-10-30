"""Search recall with each caption source, the way the app searches.

The caption half is FTS5 over the caption (unicode61 remove_diacritics 2),
queried with fts::build_query's shape (every token quoted, ANDed, the last a
prefix) and ranked by bm25; the semantic half ranks every photo by cosine; the
two are fused by reciprocal rank (k = 60). Scored against queries.py.
"""
import json, re, sqlite3
import numpy as np
from queries import QUERIES, TRANSLATED
from evaluate import retrieval, rank_by

K = 60.0


def build_query(raw):
    toks = [t for t in re.split(r"[^\w]+", raw) if t and t != "_"]
    if not toks:
        return None
    parts = ['"' + t.replace('"', '""') + '"' for t in toks]
    parts[-1] += "*"
    return " ".join(parts)


def fts(rows):
    c = sqlite3.connect(":memory:")
    c.execute("CREATE VIRTUAL TABLE f USING fts5(caption, tokenize='unicode61 remove_diacritics 2')")
    c.executemany("INSERT INTO f(rowid, caption) VALUES (?, ?)", [(r["id"], r["text"]) for r in rows])
    return lambda q: [pid for (pid,) in c.execute(
        "SELECT rowid FROM f WHERE f MATCH ? ORDER BY rank LIMIT 1000", (build_query(q),))]


def fuse(lists):
    score = {}
    for lst in lists:
        for pos, pid in enumerate(lst):
            score[pid] = score.get(pid, 0) + 1 / (K + pos)
    return [p for p, _ in sorted(score.items(), key=lambda x: (-x[1], x[0]))]


if __name__ == "__main__":
    fl = json.load(open("out/florence_hybrid.json", encoding="utf-8"))["rows"]
    qw = json.load(open("out/qwen.json", encoding="utf-8"))["rows"]
    fl_l = json.load(open("out/florence_hybrid_large.json", encoding="utf-8"))["rows"]
    sources = {
        "none": None,
        "qwen2.5vl-7b": [dict(id=r["id"], text=r["caption"]) for r in qw],
        **{f"qwen3-vl-{m}": [dict(id=r["id"], text=r["caption"]) for r in json.load(open(f"out/qwen_qwen3-vl_{m}-instruct.json", encoding="utf-8"))["rows"]]
           for m in ("2b", "4b", "8b")},
        "florence-base (caption)": [dict(id=r["id"], text=r["caption"]) for r in fl],
        "florence-base (caption+ocr)": [dict(id=r["id"], text=r["caption"] + " " + r["ocr"]) for r in fl],
        "florence-large (caption+ocr)": [dict(id=r["id"], text=r["caption"] + " " + r["ocr"]) for r in fl_l],
    }
    sets = QUERIES + TRANSLATED
    out = {}
    for emb_name, path in [("clip-b32", "out/emb_clip-b32_dml.npz"), ("siglip2", "out/emb_siglip2-b16-224-fp16_dml.npz")]:
        d = np.load(path)
        ids, img, txt = d["ids"], d["img"], d["txt"]
        semantic = [rank_by(img, ids, txt[i]) for i in range(len(sets))]
        for src, rows in sources.items():
            search = fts(rows) if rows else (lambda q: [])
            cap = [search(q) for q, _ in sets]
            fused = [fuse([s, c]) for s, c in zip(semantic, cap)]
            # Photos neither half returned are not shown; score as unranked.
            n = len(QUERIES)
            en, _ = retrieval(ids, fused[:n], QUERIES)
            xl, _ = retrieval(ids, fused[n:], TRANSLATED)
            conly, _ = retrieval(ids, cap[:n], QUERIES)
            hit = np.mean([len(c) > 0 for c in cap[:n]])
            out[f"{emb_name} + {src}"] = dict(fused_en=en, fused_translated=xl, caption_only_en=conly,
                                             queries_with_caption_hits=round(float(hit), 3))
            print(f"{emb_name:9s} + {src:28s} fused EN {en}  XL mAP {xl['mAP']}  | caption-only EN mAP {conly['mAP']} (hits on {hit:.0%} of queries)")
    json.dump(out, open("out/captionsearch.json", "w"), indent=1)
