"""OCR spike, step 1: numbered contact sheets of the sample, 12 to a
sheet, for labelling which photos show text before any model runs. Then look
at each candidate at full size and write ocr_truth.py."""
import json, os
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "ocr-data")
meta = json.load(open(os.path.join(DATA, "photos.json")))
os.makedirs(os.path.join(DATA, "sheets"), exist_ok=True)
TW, TH, COLS, ROWS = 400, 300, 4, 3
font = ImageFont.truetype("arialbd.ttf", 22)
for s in range(0, len(meta), COLS * ROWS):
    sheet = Image.new("RGB", (TW * COLS, TH * ROWS), "white")
    d = ImageDraw.Draw(sheet)
    for i, m in enumerate(meta[s : s + COLS * ROWS]):
        im = Image.open(os.path.join(DATA, m["file"])).convert("RGB")
        im.thumbnail((TW - 6, TH - 6))
        x, y = (i % COLS) * TW, (i // COLS) * TH
        sheet.paste(im, (x + (TW - im.width) // 2, y + (TH - im.height) // 2))
        d.rectangle([x + 4, y + 4, x + 70, y + 32], fill="black")
        d.text((x + 8, y + 6), str(m["id"]), fill="yellow", font=font)
    sheet.save(os.path.join(DATA, "sheets", f"sheet{s // (COLS * ROWS):02d}.jpg"), quality=88)
print(len(os.listdir(os.path.join(DATA, "sheets"))), "sheets")
