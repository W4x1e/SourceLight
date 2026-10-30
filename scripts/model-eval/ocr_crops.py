"""Draw what an OCR run read, for checking by eye.

    python ocr_crops.py ocr-data/out/ocr_1600.json <min conf> <id,id,...> <out.jpg>

Each read line: its crop from the preview (with some margin), and the text read.
"""
import json, os, sys
import numpy as np
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "ocr-data")
run = json.load(open(sys.argv[1]))
conf = float(sys.argv[2])
ids = sys.argv[3].split(",")
font = ImageFont.truetype("arial.ttf", 18)
tiles = []
for pid in ids:
    img = Image.open(os.path.join(DATA, "photos", f"{pid}.jpg")).convert("RGB")
    for l in run["photos"][pid]["lines"]:
        if l["conf"] < conf:
            continue
        b = np.array(l["box"])
        x0, y0 = b.min(0) - 40
        x1, y1 = b.max(0) + 40
        c = img.crop((max(0, x0), max(0, y0), min(img.width, x1), min(img.height, y1)))
        c.thumbnail((520, 180))
        tile = Image.new("RGB", (540, c.height + 30), "white")
        tile.paste(c, (10, 28))
        ImageDraw.Draw(tile).text((10, 4), f"{pid}: {l['text']!r} ({l['conf']:.2f})", fill="black", font=font)
        tiles.append(tile)
sheet = Image.new("RGB", (1080, sum(t.height for t in tiles[::2]) + 20), "white")
y = [0, 0]
for i, t in enumerate(tiles):
    col = i % 2
    sheet.paste(t, (col * 540, y[col]))
    y[col] += t.height
sheet = sheet.crop((0, 0, 1080, max(y)))
sheet.save(sys.argv[4], quality=90)
print(len(tiles), "lines")
