"""OCR spike, step 3: score a run against ocr_truth.py.

    python ocr_score.py ocr-data/out/ocr_1600.json [min conf] [--substr] [--detail]

Words are matched as search would match them: the line's text lower-cased,
accents dropped, split on anything that isn't a letter or digit.
"""
import json, os, re, sys, unicodedata
from ocr_truth import TRUTH

path = sys.argv[1]
MIN_CONF = float(sys.argv[2]) if len(sys.argv) > 2 and not sys.argv[2].startswith("-") else 0.5
DETAIL = "--detail" in sys.argv
SUBSTR = "--substr" in sys.argv
run = json.load(open(path))


def tokens(text):
    t = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode().lower()
    return [w for w in re.split(r"[^a-z0-9]+", t) if w]


def meaningful(w):
    """A token worth indexing: 3+ characters, or 2+ digits."""
    return len(w) >= 3 or (len(w) >= 2 and w.isdigit())


words_total = words_found = 0
text_photos = found_photos = 0
fp_photos, junk_total, kept_total = [], 0, 0
rows = []
for pid, r in run["photos"].items():
    pid = int(pid)
    kept = [l for l in r["lines"] if l["conf"] >= MIN_CONF]
    toks = {w for l in kept for w in tokens(l["text"]) if meaningful(w)}
    kept_total += len(toks)
    t = TRUTH.get(pid)
    if t is None:
        if toks:
            fp_photos.append((pid, sorted(toks)))
        continue
    want = {w for w in t["words"]}
    blob = " ".join(sorted(toks))
    # Substring mode: a trigram index finds a word inside a longer token
    # ("biennale" in "ofbiennalearte2026"); words under 3 characters still
    # need an exact token.
    hit = (lambda w: w in toks or (len(w) >= 3 and w in blob)) if SUBSTR else (lambda w: w in toks)
    ok = {w for w in t["words"] if hit(w)}
    allowed = set(t["words"]) | set(t["maybe"])
    junk = sorted(w for w in toks if w not in allowed)
    junk_total += len(junk)
    bonus = sorted(w for w in toks if w in t["maybe"])
    if want:
        text_photos += 1
        found_photos += bool(ok)
        words_total += len(want)
        words_found += len(ok)
    rows.append((pid, sorted(ok), sorted(want - ok), bonus, junk))

no_text = sum(1 for pid in run["photos"] if int(pid) not in TRUTH)
s = run["summary"]
print(f"{os.path.basename(path)}  {'substring' if SUBSTR else 'exact'}  conf>={MIN_CONF}  det side {s['side']}  threads {s['threads']}")
print(f"  words read:      {words_found}/{words_total} = {words_found / words_total:.0%}")
print(f"  photos findable: {found_photos}/{text_photos} = {found_photos / text_photos:.0%} (by at least one of their words)")
print(f"  false positives: {len(fp_photos)}/{no_text} photos without text had a word read")
print(f"  junk on text photos: {junk_total} tokens outside words+maybe")
print(f"  time per photo: {s['det_all'] + s['rec_all']:.0f} ms (detect {s['det_all']:.0f}, of which model {s['det_model']:.0f}; "
      f"recognise {s['rec_all']:.0f}, of which model {s['rec_model']:.0f}); {s['lines']} lines, {s.get('reread', 0)} re-read")
if DETAIL:
    print("\n  photo: found | missed | bonus | junk")
    for pid, ok, miss, bonus, junk in sorted(rows):
        print(f"  {pid}: {ok} | {miss} | {bonus} | {junk}")
    print("\n  false positives:")
    for pid, toks in fp_photos:
        print(f"  {pid}: {toks}")
