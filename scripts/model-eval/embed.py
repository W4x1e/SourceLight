"""SigLIP 2 spike: image + query embeddings from CLIP ViT-B/32 (as shipped) and SigLIP 2
base-patch16-224, over the whole library, with timings. Writes out/emb_*.npz."""
import os, sys, time, json
import numpy as np
from PIL import Image
from tokenizers import Tokenizer
from common import APP, HERE, PHOTOS, image, session, VramPeak, dump

provider = sys.argv[1] if len(sys.argv) > 1 else "dml"


def l2(x):
    return x / np.linalg.norm(x, axis=-1, keepdims=True)


# ---------------------------------------------------------------- CLIP (as the app does it)
CLIP_MEAN = np.array([0.481455, 0.457827, 0.408211], np.float32)
CLIP_STD = np.array([0.26863, 0.261303, 0.275777], np.float32)


def clip_pre(im):
    # tagger/builtin.rs clip_preprocess: short side to 224 (triangle), centre crop.
    s = 224 / min(im.size)
    nw, nh = max(224, round(im.width * s)), max(224, round(im.height * s))
    im = im.resize((nw, nh), Image.BILINEAR)
    x0, y0 = (nw - 224) // 2, (nh - 224) // 2
    a = np.asarray(im.crop((x0, y0, x0 + 224, y0 + 224)), np.float32) / 255.0
    return ((a - CLIP_MEAN) / CLIP_STD).transpose(2, 0, 1)


class Clip:
    name = "clip-b32"

    def __init__(self):
        d = os.path.join(APP, "models", "clip")
        self.v = session(os.path.join(d, "vision_model.onnx"), provider)
        self.t = session(os.path.join(d, "text_model.onnx"), provider)
        self.tok = Tokenizer.from_file(os.path.join(d, "tokenizer.json"))
        self.files = [os.path.join(d, f) for f in ("vision_model.onnx", "text_model.onnx", "tokenizer.json")]

    def images(self, batch):
        x = np.stack([clip_pre(im) for im in batch])
        return self.v.run(["image_embeds"], {"pixel_values": x})[0]

    def texts(self, qs):
        rows = []
        for q in qs:
            ids = self.tok.encode(q).ids[:77]
            rows.append(ids + [0] * (77 - len(ids)))
        return self.t.run(["text_embeds"], {"input_ids": np.array(rows, np.int64)})[0]


# ---------------------------------------------------------------- SigLIP 2
class Siglip:
    def __init__(self, variant="fp16"):
        self.name = f"siglip2-b16-224-{variant}"
        d = os.path.join(HERE, "models", "siglip2")
        sfx = "" if variant == "fp32" else "_" + variant
        self.v = session(os.path.join(d, f"vision_model{sfx}.onnx"), provider, basic=(provider == "cpu"))
        self.t = session(os.path.join(d, f"text_model{sfx}.onnx"), provider, basic=(provider == "cpu"))
        self.tok = Tokenizer.from_file(os.path.join(d, "tokenizer.json"))
        self.pad = self.tok.token_to_id("<pad>")
        self.files = [os.path.join(d, f) for f in (f"vision_model{sfx}.onnx", f"text_model{sfx}.onnx", "tokenizer.json")]

    def images(self, batch):
        # SiglipImageProcessor: plain resize to 224x224 (bilinear), (x - .5) / .5.
        x = np.stack([
            ((np.asarray(im.resize((224, 224), Image.BILINEAR), np.float32) / 255.0 - 0.5) / 0.5).transpose(2, 0, 1)
            for im in batch])
        return self.v.run(["pooler_output"], {"pixel_values": x})[0]

    def texts(self, qs):
        # Lower-cased, EOS appended, padded to 64 with <pad>: how SigLIP was trained.
        rows = []
        for q in qs:
            ids = self.tok.encode(q.lower()).ids[:64]
            rows.append(ids + [self.pad] * (64 - len(ids)))
        return self.t.run(["pooler_output"], {"input_ids": np.array(rows, np.int64)})[0]


if __name__ == "__main__":
    from queries import QUERIES, TRANSLATED
    texts = [q for q, _ in QUERIES] + [q for q, _ in TRANSLATED]
    report = {}
    which = sys.argv[2:] or ["clip", "fp16"]
    for w in which:
        t0 = time.perf_counter()
        m = Clip() if w == "clip" else Siglip(w)
        load = time.perf_counter() - t0
        size = sum(os.path.getsize(f) for f in m.files)
        ims = [image(p) for p in PHOTOS]
        m.images(ims[:1])  # warm-up
        with VramPeak() as vp:
            t0 = time.perf_counter()
            one = [m.images([im])[0] for im in ims[:40]]
            per_image_b1 = (time.perf_counter() - t0) / 40
            t0 = time.perf_counter()
            embs = np.concatenate([m.images(ims[i:i + 16]) for i in range(0, len(ims), 16)])
            per_image_b16 = (time.perf_counter() - t0) / len(ims)
            m.texts(texts[:1])
            t0 = time.perf_counter()
            tq = np.concatenate([m.texts([q]) for q in texts])
            per_query = (time.perf_counter() - t0) / len(texts)
        # Batch-1 and batch-16 must agree, or the export is batch-unsafe.
        drift = float(np.max(1 - np.sum(l2(np.array(one)) * l2(embs[:40]), axis=1)))
        np.savez(os.path.join(HERE, "out", f"emb_{m.name}_{provider}.npz"),
                 ids=np.array([p["id"] for p in PHOTOS]), img=l2(embs), txt=l2(tq), texts=np.array(texts))
        report[m.name] = dict(provider=provider, load_s=round(load, 2), files_mb=round(size / 1e6),
                              ms_per_image_b1=round(per_image_b1 * 1000, 1),
                              ms_per_image_b16=round(per_image_b16 * 1000, 1),
                              ms_per_query=round(per_query * 1000, 1), vram_delta_mb=vp.delta,
                              dim=int(embs.shape[1]), batch_drift=drift)
        print(m.name, report[m.name], flush=True)
        del m
    import glob
    path = os.path.join(HERE, "out", f"embed_timing_{provider}.json")
    old = json.load(open(path)) if os.path.exists(path) else {}
    old.update(report)
    dump(f"embed_timing_{provider}.json", old)
