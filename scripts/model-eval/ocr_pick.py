"""OCR spike, step 0: pick the 200 photos and copy their previews.

    python ocr_pick.py <the app's data folder>

Every photo whose keywords say it shows a sign (or a poster, menu, shopfront...),
plus a seeded random draw from the rest, so the set has enough text to measure
recall and enough without to measure false positives. The catalogue is copied
into ocr-data/ first and only the copy is opened; previews are copied, never
written.
"""
import json, os, random, re, shutil, sqlite3, sys
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "ocr-data")
APP = sys.argv[1]
os.makedirs(os.path.join(DATA, "cat"), exist_ok=True)
os.makedirs(os.path.join(DATA, "photos"), exist_ok=True)
for f in ("library.db", "library.db-wal", "library.db-shm"):
    if os.path.exists(os.path.join(APP, f)):
        shutil.copyfile(os.path.join(APP, f), os.path.join(DATA, "cat", f))

c = sqlite3.connect(os.path.join(DATA, "cat", "library.db"))
rows = c.execute(
    "SELECT id, preview_path FROM photos WHERE variant_of IS NULL AND NOT missing ORDER BY id"
).fetchall()
thumbs = os.path.join(APP, "thumbs")
have = {pid: os.path.join(thumbs, os.path.basename(p)) for pid, p in rows if p}
have = {k: v for k, v in have.items() if os.path.exists(v)}
print("photos with a preview:", len(have))

texty = re.compile(r"sign|poster|menu|plaque|banner|shop|store|label|logo|graffiti|book|newspaper|board", re.I)
tagged = {
    pid
    for pid, name in c.execute("SELECT pt.photo_id, t.name FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id")
    if texty.search(name) and not re.search(r"paddle|surf|skate|cutting", name, re.I)
}
lead = sorted(pid for pid in tagged if pid in have)
rest = sorted(pid for pid in have if pid not in tagged)
random.Random(242).shuffle(rest)
chosen = lead + rest[: 200 - len(lead)]
print("tagged as showing text:", len(lead), "random:", 200 - len(lead))

meta = []
for pid in chosen:
    dst = os.path.join(DATA, "photos", f"{pid}.jpg")
    if not os.path.exists(dst):
        shutil.copyfile(have[pid], dst)
    w, h = Image.open(dst).size
    meta.append({"id": pid, "file": f"photos/{pid}.jpg", "w": w, "h": h, "tagged": pid in tagged})
json.dump(meta, open(os.path.join(DATA, "photos.json"), "w"), indent=1)
