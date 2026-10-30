"""Eyes open or shut: the app's reading, scored against labelled faces.

The same pipeline and rule as `src-tauri/src/faces.rs` (MediaPipe face mesh,
three looks, blendshapes, `eye_open_from`), run from Python over faces in the
app's catalogue that were labelled by eye on 2026-09-28, plus an optional
folder of photos with eyes plainly shut (the owner's library has no real
blinks, so those came from Wikimedia Commons and are not kept here).

    python eyes_eval.py                    # the labelled catalogue faces
    python eyes_eval.py --closed DIR       # and every face in DIR's photos

Face ids are the owner's catalogue's; a regroup keeps them, a re-detect does
not. Reads the catalogue read-only. Needs opencv-python for --closed.
"""
import argparse
import math
import os
import sqlite3
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image

# Not `common.APP`: common.py wants the harness's photos.json, which this
# script doesn't use.
APP = os.path.join(os.environ["LOCALAPPDATA"], "app.sourcelight")

REPO = Path(__file__).resolve().parents[2]
MODELS = REPO / "src-tauri" / "vendor" / "models" / "faces"

# Labelled by eye from contact sheets, 2026-09-28.
OPEN = [567, 609, 562, 604, 422, 563, 605, 423, 554, 596, 394, 362, 361, 421, 393, 553, 595, 420, 403, 548, 590, 425,
        383, 411, 364, 402, 568, 610, 380, 247, 410, 359, 413, 379, 424, 341, 344, 346, 331, 338, 340, 348, 262, 336,
        324, 333, 267, 265, 272, 471]
SUNGLASSES = [250, 306, 285, 549, 591, 287, 286, 251, 252, 555, 597, 307, 552, 594, 445, 546, 441, 442, 440, 253, 260,
              255, 257, 464, 274, 303, 488, 374, 276, 304, 278, 446, 506, 539]
LOOKING_DOWN = [572, 614, 571, 613, 415, 545, 587, 543, 585, 366]
NOT_A_READABLE_FACE = [566, 608, 551, 593, 376, 375, 308, 428, 460, 479]  # profiles, a rock, an arm, a silhouette

SUBSET = [0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 17, 21, 33, 37, 39, 40, 46, 52, 53, 54, 55, 58, 61, 63, 65, 66, 67, 70, 78,
          80, 81, 82, 84, 87, 88, 91, 93, 95, 103, 105, 107, 109, 127, 132, 133, 136, 144, 145, 146, 148, 149, 150, 152,
          153, 154, 155, 157, 158, 159, 160, 161, 162, 163, 168, 172, 173, 176, 178, 181, 185, 191, 195, 197, 234, 246,
          249, 251, 263, 267, 269, 270, 276, 282, 283, 284, 285, 288, 291, 293, 295, 296, 297, 300, 308, 310, 311, 312,
          314, 317, 318, 321, 323, 324, 332, 334, 336, 338, 356, 361, 362, 365, 373, 374, 375, 377, 378, 379, 380, 381,
          382, 384, 385, 386, 387, 388, 389, 390, 397, 398, 400, 402, 405, 409, 415, 454, 466, 468, 469, 470, 471, 472,
          473, 474, 475, 476, 477]

mesh = ort.InferenceSession(str(MODELS / "mesh.onnx"), providers=["CPUExecutionProvider"])
blend = ort.InferenceSession(str(MODELS / "blendshapes.onnx"), providers=["CPUExecutionProvider"])


def look(img, cx, cy, side, angle):
    """The mesh on one square look; landmarks in image pixels, and presence."""
    s = side / 256.0
    ca, sa = math.cos(angle), math.sin(angle)
    a, b, d, e = s * ca, -s * sa, s * sa, s * ca
    c, f = cx - 128 * a - 128 * b, cy - 128 * d - 128 * e
    patch = img.transform((256, 256), Image.AFFINE, (a, b, c, d, e, f), resample=Image.BILINEAR)
    x = np.asarray(patch, dtype=np.float32)[None].transpose(0, 3, 1, 2) / 255.0
    out = mesh.run(None, {mesh.get_inputs()[0].name: x})
    p = out[0].reshape(478, 3)
    pts = np.stack([a * p[:, 0] + b * p[:, 1] + c, d * p[:, 0] + e * p[:, 1] + f], 1)
    return pts, 1 / (1 + math.exp(-out[1].item()))


def around_mesh(pts):
    (rx, ry), (lx, ly) = pts[33], pts[263]
    return pts[:, 0].mean(), pts[:, 1].mean(), max(np.ptp(pts[:, 0]), np.ptp(pts[:, 1])) * 1.5, math.atan2(ly - ry, lx - rx)


def eye_open(img, x, y, w, h):
    """`faces::eye_open_from` on the app's three looks. None is can't tell."""
    pts, _ = look(img, x + w / 2, y + h / 2, max(w, h) * 1.5, 0.0)
    pts, _ = look(img, *around_mesh(pts))
    pts, presence = look(img, *around_mesh(pts))
    shapes = blend.run(None, {blend.get_inputs()[0].name: pts[SUBSET].astype(np.float32)[None]})[0]
    blink = min(shapes[9], shapes[10])
    d = lambda i, j: float(np.linalg.norm(pts[i] - pts[j]))
    aspect = min(d(159, 145) / max(d(33, 133), 1e-3), d(386, 374) / max(d(362, 263), 1e-3))
    if presence < 0.9 or w < 64:
        return None
    shut = blink >= 0.6 or (aspect <= 0.10 and blink >= 0.45)
    return min(1 - blink, 0.45) if shut else max(1 - blink, 0.55)


def catalogue(ids):
    db = Path(APP) / "library.db"
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    out = {}
    for fid in ids:
        r = con.execute("SELECT ph.preview_path, f.x, f.y, f.w, f.h FROM faces f JOIN photos ph ON ph.id = f.photo_id"
                        " WHERE f.id = ?", (fid,)).fetchone()
        if r:
            out[fid] = eye_open(Image.open(r[0]).convert("RGB"), *r[1:])
    return out


def report(name, readings, want):
    vals = list(readings.values())
    shut = sum(v is not None and v < 0.5 for v in vals)
    opened = sum(v is not None and v >= 0.5 for v in vals)
    cant = sum(v is None for v in vals)
    print(f"{name:22} {len(vals):3} faces   open {opened:3}   shut {shut:3}   can't tell {cant:3}   ({want})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--closed", help="folder of photos whose faces all have their eyes shut")
    args = ap.parse_args()
    report("open eyes", catalogue(OPEN), "want none shut")
    report("sunglasses", catalogue(SUNGLASSES), "want none shut")
    report("looking down", catalogue(LOOKING_DOWN), "either is fair: the lids are down")
    report("not a readable face", catalogue(NOT_A_READABLE_FACE), "want all can't tell")
    if args.closed:
        import cv2
        got = {}
        for f in sorted(Path(args.closed).glob("*.jpg")):
            bgr = cv2.imread(str(f))
            det = cv2.FaceDetectorYN.create(str(MODELS / "yunet.onnx"), "", (bgr.shape[1], bgr.shape[0]), 0.6)
            _, faces = det.detect(bgr)
            if faces is not None:
                x, y, w, h = map(float, max(faces, key=lambda r: r[2] * r[3])[:4])
                got[f.name] = eye_open(Image.open(f).convert("RGB"), x, y, w, h)
        report("eyes shut (--closed)", got, "want all shut")


if __name__ == "__main__":
    main()
