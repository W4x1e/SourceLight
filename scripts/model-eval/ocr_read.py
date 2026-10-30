"""OCR spike, step 2: PP-OCR over the sample: DB text detection (PP-OCRv4 mobile) and CTC
recognition (PP-OCRv3 English), with PaddleOCR's own pre- and post-processing.

    python ocr_read.py <det max side> [threads]

Writes ocr-data/out/ocr_<side>.json: per photo, every line read with its box and
confidence, plus timings split by stage. The text-angle classifier is not used:
text upside down is not read (and neither would it be in the app without it).
"""
import json, math, os, sys, time
import cv2
import numpy as np
import onnxruntime as ort

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "ocr-data")
SIDE = int(sys.argv[1]) if len(sys.argv) > 1 else 960
THREADS = int(sys.argv[2]) if len(sys.argv) > 2 else 0

opts = ort.SessionOptions()
if THREADS:
    opts.intra_op_num_threads = THREADS
det = ort.InferenceSession(os.path.join(HERE, "models", "ocr", "det.onnx"), opts, providers=["CPUExecutionProvider"])
rec = ort.InferenceSession(os.path.join(HERE, "models", "ocr", "rec_en.onnx"), opts, providers=["CPUExecutionProvider"])
CHARS = ["<blank>"] + rec.get_modelmeta().custom_metadata_map["character"].splitlines() + [" "]
assert len(CHARS) == rec.get_outputs()[0].shape[2], len(CHARS)

MEAN = np.array([0.485, 0.456, 0.406], np.float32)
STD = np.array([0.229, 0.224, 0.225], np.float32)
THRESH, BOX_THRESH, UNCLIP, MIN_SIZE = 0.3, 0.6, 1.5, 3
REC_H, REC_W, REC_BATCH = 48, 320, 6


def detect(bgr):
    h, w = bgr.shape[:2]
    r = min(1.0, SIDE / max(h, w))
    rh, rw = max(32, round(h * r / 32) * 32), max(32, round(w * r / 32) * 32)
    x = cv2.resize(bgr, (rw, rh)).astype(np.float32) / 255.0
    x = ((x - MEAN) / STD).transpose(2, 0, 1)[None]
    t = time.perf_counter()
    pred = det.run(None, {"x": x})[0][0, 0]
    t_model = time.perf_counter() - t
    boxes = []
    bitmap = (pred > THRESH).astype(np.uint8)
    contours, _ = cv2.findContours(bitmap, cv2.RETR_LIST, cv2.CHAIN_APPROX_SIMPLE)
    for c in contours[:1000]:
        rect = cv2.minAreaRect(c)
        if min(rect[1]) < MIN_SIZE:
            continue
        pts = cv2.boxPoints(rect)
        # Box score: mean probability inside the box ("fast" mode).
        x0, y0 = np.clip(np.floor(pts.min(0)).astype(int), 0, [rw - 1, rh - 1])
        x1, y1 = np.clip(np.ceil(pts.max(0)).astype(int), 0, [rw - 1, rh - 1])
        mask = np.zeros((y1 - y0 + 1, x1 - x0 + 1), np.uint8)
        cv2.fillPoly(mask, [(pts - [x0, y0]).astype(np.int32)], 1)
        score = cv2.mean(pred[y0 : y1 + 1, x0 : x1 + 1], mask)[0]
        if score < BOX_THRESH:
            continue
        # Unclip: grow the box by area * ratio / perimeter on every side.
        (cx, cy), (bw, bh), ang = rect
        d = bw * bh * UNCLIP / (2 * (bw + bh))
        grown = ((cx, cy), (bw + 2 * d, bh + 2 * d), ang)
        if min(grown[1]) < MIN_SIZE + 2:
            continue
        pts = cv2.boxPoints(grown) * [w / rw, h / rh]
        boxes.append((order(pts), float(score)))
    boxes.sort(key=lambda b: (round(b[0][0][1] / 10), b[0][0][0]))
    return boxes, t_model, pred.shape


def order(pts):
    """Top-left, top-right, bottom-right, bottom-left."""
    pts = pts[np.argsort(pts[:, 0])]
    left, right = pts[:2], pts[2:]
    tl, bl = left[np.argsort(left[:, 1])]
    tr, br = right[np.argsort(right[:, 1])]
    return np.array([tl, tr, br, bl], np.float32)


def crop(bgr, pts):
    w = int(max(np.linalg.norm(pts[0] - pts[1]), np.linalg.norm(pts[2] - pts[3])))
    h = int(max(np.linalg.norm(pts[0] - pts[3]), np.linalg.norm(pts[1] - pts[2])))
    dst = np.array([[0, 0], [w, 0], [w, h], [0, h]], np.float32)
    m = cv2.getPerspectiveTransform(pts, dst)
    img = cv2.warpPerspective(bgr, m, (w, h), borderMode=cv2.BORDER_REPLICATE, flags=cv2.INTER_CUBIC)
    if h / max(w, 1) >= 1.5:  # vertical text: read it turned on its side
        return np.ascontiguousarray(np.rot90(img)), True
    return img, False


def read_lines(crops):
    """Recognise each crop. Without the angle classifier, a vertical line is
    read both ways round and a weak horizontal one upside down as well; the
    more confident reading wins."""
    first, t1 = recognise([c for c, _ in crops])
    again = [i for i, (c, vertical) in enumerate(crops) if vertical or first[i][1] < 0.6]
    if not again:
        return first, t1, 0
    second, t2 = recognise([np.ascontiguousarray(np.rot90(crops[i][0], 2)) for i in again])
    for i, alt in zip(again, second):
        if alt[1] > first[i][1]:
            first[i] = alt
    return first, t1 + t2, len(again)


def recognise(crops):
    out = [None] * len(crops)
    t_model = 0.0
    order_ = np.argsort([c.shape[1] / c.shape[0] for c in crops])
    for s in range(0, len(crops), REC_BATCH):
        idx = order_[s : s + REC_BATCH]
        ratio = max(REC_W / REC_H, max(crops[i].shape[1] / crops[i].shape[0] for i in idx))
        width = int(REC_H * ratio)
        batch = np.zeros((len(idx), 3, REC_H, width), np.float32)
        for j, i in enumerate(idx):
            c = crops[i]
            rw = min(width, int(math.ceil(REC_H * c.shape[1] / c.shape[0])))
            x = cv2.resize(c, (rw, REC_H)).astype(np.float32) / 255.0
            batch[j, :, :, :rw] = ((x - 0.5) / 0.5).transpose(2, 0, 1)
        SHAPES.append(list(batch.shape))
        t = time.perf_counter()
        probs = rec.run(None, {"x": batch})[0]
        t_model += time.perf_counter() - t
        for j, i in enumerate(idx):
            best, conf = probs[j].argmax(1), probs[j].max(1)
            keep = [k for k in range(len(best)) if best[k] != 0 and (k == 0 or best[k] != best[k - 1])]
            text = "".join(CHARS[best[k]] for k in keep)
            out[i] = (text, float(conf[keep].mean()) if keep else 0.0)
    return out, t_model


meta = json.load(open(os.path.join(DATA, "photos.json")))
if os.environ.get("ONLY"):
    meta = [m for m in meta if str(m["id"]) in os.environ["ONLY"].split(",")]
results = {}
tot = {"read": 0.0, "det_model": 0.0, "det_all": 0.0, "rec_model": 0.0, "rec_all": 0.0, "lines": 0}
t_all = time.perf_counter()
for m in meta:
    SHAPES = []
    t0 = time.perf_counter()
    bgr = cv2.imread(os.path.join(DATA, m["file"]))
    t1 = time.perf_counter()
    boxes, t_det, shape = detect(bgr)
    t2 = time.perf_counter()
    crops = [crop(bgr, b) for b, _ in boxes]
    lines, t_rec, n_again = read_lines(crops) if crops else ([], 0.0, 0)
    t3 = time.perf_counter()
    tot["reread"] = tot.get("reread", 0) + n_again
    tot["read"] += t1 - t0
    tot["det_model"] += t_det
    tot["det_all"] += t2 - t1
    tot["rec_model"] += t_rec
    tot["rec_all"] += t3 - t2
    tot["lines"] += len(lines)
    results[m["id"]] = {
        "det_input": list(shape),
        "rec_batches": SHAPES,
        "ms": round((t3 - t1) * 1000, 1),
        "lines": [
            {"text": t, "conf": round(c, 3), "det": round(s, 3), "box": b.round(1).tolist()}
            for (b, s), (t, c) in zip(boxes, lines)
        ],
    }
n = len(meta)
summary = {k: (round(v / n * 1000, 1) if k not in ("lines", "reread") else v) for k, v in tot.items()}
summary["photos"] = n
summary["wall_s"] = round(time.perf_counter() - t_all, 1)
summary["side"] = SIDE
summary["threads"] = THREADS or "default"
os.makedirs(os.path.join(DATA, "out"), exist_ok=True)
tag = f"{SIDE}" + (f"_t{THREADS}" if THREADS else "")
json.dump({"summary": summary, "photos": results}, open(os.path.join(DATA, "out", f"ocr_{tag}.json"), "w"), indent=1)
print(json.dumps(summary))
