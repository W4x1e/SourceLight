"""Step 0: dumps the library's photos to photos.json (the list every other
script reads) and draws numbered contact sheets of them into sheets/, which is
how the ground truth in queries.py was labelled."""
import sqlite3, os, json
from PIL import Image, ImageDraw, ImageFont
D = os.path.join(os.environ['LOCALAPPDATA'], 'app.sourcelight')
c = sqlite3.connect(os.path.join(D, 'library.db'))
rows = c.execute("select id, preview_path, thumb_path, filename, capture_date from photos where variant=0 order by capture_date, id").fetchall()
json.dump([dict(id=r[0], preview=r[1], thumb=r[2], filename=r[3], date=r[4]) for r in rows], open('photos.json','w', encoding='utf-8'), indent=0)
os.makedirs('sheets', exist_ok=True)
CELL, COLS, ROWS = 300, 5, 4
font = ImageFont.truetype('arialbd.ttf', 22)
per = COLS*ROWS
for s in range(0, len(rows), per):
    sheet = Image.new('RGB', (COLS*CELL, ROWS*CELL), (20,20,20))
    for k, r in enumerate(rows[s:s+per]):
        p = r[1] if r[1] and os.path.exists(r[1]) else r[2]
        try:
            im = Image.open(p).convert('RGB'); im.thumbnail((CELL-6, CELL-6))
        except Exception as e:
            im = Image.new('RGB', (CELL-6, CELL-6), (90,0,0))
        x, y = (k % COLS)*CELL, (k // COLS)*CELL
        sheet.paste(im, (x+3+(CELL-6-im.width)//2, y+3+(CELL-6-im.height)//2))
        d = ImageDraw.Draw(sheet); d.rectangle([x+4,y+4,x+62,y+32], fill=(0,0,0)); d.text((x+8,y+6), str(r[0]), fill=(255,255,0), font=font)
    sheet.save(f'sheets/sheet_{s//per:02d}.jpg', quality=85)
print(len(rows), 'photos;', (len(rows)+per-1)//per, 'sheets')
print(rows[0][1])
