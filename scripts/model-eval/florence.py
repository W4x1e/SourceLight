"""Florence-2-base-ft captions and OCR, in-process on ONNX Runtime.

Four exported graphs (onnx-community/Florence-2-base-ft @ e88a44e):
vision_encoder -> image tokens; embed_tokens -> prompt tokens; encoder over
both; decoder_model_merged (with KV cache) generates. Greedy decoding with
no_repeat_ngram_size=3, as generation_config asks (it also asks num_beams=3;
greedy is what an in-app loop would start with, and the cost of beams is
noted separately).

Usage: florence.py <dml|cpu> <limit> -> out/florence_<provider>.json
"""
import os, sys, time, json
import numpy as np
from PIL import Image
from tokenizers import Tokenizer
from common import HERE, PHOTOS, image, session, VramPeak, dump

provider = sys.argv[1] if len(sys.argv) > 1 else "dml"
limit = int(sys.argv[2]) if len(sys.argv) > 2 else len(PHOTOS)
size = sys.argv[3] if len(sys.argv) > 3 else "base"
D = os.path.join(HERE, "models", f"florence-{size}")
PROMPTS = {
    "<MORE_DETAILED_CAPTION>": "Describe with a paragraph what is shown in the image.",
    "<DETAILED_CAPTION>": "Describe in detail what is shown in the image.",
    "<OCR>": "What is the text in the image?",
}
MEAN = np.array([0.485, 0.456, 0.406], np.float32)
STD = np.array([0.229, 0.224, 0.225], np.float32)
EOS, START, HD = 2, 2, 64
LAYERS, HEADS = {"base": (6, 12), "large": (12, 16)}[size]


class Florence:
    def __init__(self):
        # "hybrid": the one-shot graphs on the GPU, the per-token decoder on the
        # CPU — DirectML corrupts the merged decoder's cached branch (the If
        # node's use_cache_branch=True path), while step 0 is exact.
        big = "dml" if provider in ("dml", "hybrid") else "cpu"
        small = "cpu" if provider in ("cpu", "hybrid") else "dml"
        self.vision = session(os.path.join(D, "vision_encoder_fp16.onnx"), big, big == "cpu")
        self.encoder = session(os.path.join(D, "encoder_model_fp16.onnx"), big, big == "cpu")
        self.embed = session(os.path.join(D, "embed_tokens_fp16.onnx"), small, small == "cpu")
        self.decoder = session(os.path.join(D, "decoder_model_merged.onnx"), small, small == "cpu")
        self.tok = Tokenizer.from_file(os.path.join(D, "tokenizer.json"))
        self.files = [os.path.join(D, f) for f in ("vision_encoder_fp16.onnx", "embed_tokens_fp16.onnx",
                                                    "encoder_model_fp16.onnx", "decoder_model_merged.onnx",
                                                    "tokenizer.json")]
        self._img_cache = None

    def image_features(self, im):
        a = np.asarray(im.resize((768, 768), Image.BICUBIC), np.float32) / 255.0
        x = ((a - MEAN) / STD).transpose(2, 0, 1)[None]
        return self.vision.run(None, {"pixel_values": x})[0]

    def generate(self, feats, task, max_new=256):
        ids = np.array([self.tok.encode(PROMPTS[task]).ids], np.int64)
        text = self.embed.run(None, {"input_ids": ids})[0]
        embeds = np.concatenate([feats, text], axis=1).astype(np.float32)
        mask = np.ones(embeds.shape[:2], np.int64)
        enc = self.encoder.run(None, {"inputs_embeds": embeds, "attention_mask": mask})[0]

        empty = np.zeros((1, HEADS, 0, HD), np.float32)
        past = {f"past_key_values.{i}.{k}.{v}": empty
                for i in range(LAYERS) for k in ("decoder", "encoder") for v in ("key", "value")}
        out_names = [o.name for o in self.decoder.get_outputs()]
        tokens = [START]
        for step in range(max_new):
            inp = self.embed.run(None, {"input_ids": np.array([[tokens[-1]]], np.int64)})[0].astype(np.float32)
            res = self.decoder.run(None, {"inputs_embeds": inp, "encoder_hidden_states": enc,
                                          "encoder_attention_mask": mask,
                                          "use_cache_branch": np.array([step > 0]), **past})
            r = dict(zip(out_names, res))
            logits = r["logits"][0, -1].copy()
            # no_repeat_ngram_size = 3
            if len(tokens) >= 3:
                prefix = tuple(tokens[-2:])
                for j in range(len(tokens) - 2):
                    if tuple(tokens[j:j + 2]) == prefix:
                        logits[tokens[j + 2]] = -np.inf
            # generation_config: forced_bos_token_id = 0 — BART decoders start
            # </s> <s> ..., and the first generated token is forced to <s>.
            nxt = 0 if step == 0 else int(np.argmax(logits))
            for i in range(LAYERS):
                past[f"past_key_values.{i}.decoder.key"] = r[f"present.{i}.decoder.key"]
                past[f"past_key_values.{i}.decoder.value"] = r[f"present.{i}.decoder.value"]
                if step == 0:  # the cross-attention cache is computed once
                    past[f"past_key_values.{i}.encoder.key"] = r[f"present.{i}.encoder.key"]
                    past[f"past_key_values.{i}.encoder.value"] = r[f"present.{i}.encoder.value"]
            tokens.append(nxt)
            if nxt == EOS:
                break
        return self.tok.decode(tokens, skip_special_tokens=True).strip(), len(tokens) - 1


if __name__ == "__main__":
    from common import vram_used_mb
    idle = vram_used_mb()
    t0 = time.perf_counter()
    f = Florence()
    loaded = vram_used_mb() - idle
    load = time.perf_counter() - t0
    ims = [image(p) for p in PHOTOS[:limit]]
    f.generate(f.image_features(ims[0]), "<DETAILED_CAPTION>")  # warm-up
    rows, t_vis, t_cap, t_ocr, n_tok = [], 0.0, 0.0, 0.0, 0
    with VramPeak() as vp:
        for p, im in zip(PHOTOS[:limit], ims):
            a = time.perf_counter(); feats = f.image_features(im); b = time.perf_counter()
            cap, n = f.generate(feats, "<MORE_DETAILED_CAPTION>"); c = time.perf_counter()
            ocr, _ = f.generate(feats, "<OCR>", 64); d = time.perf_counter()
            t_vis += b - a; t_cap += c - b; t_ocr += d - c; n_tok += n
            rows.append(dict(id=p["id"], caption=cap, ocr=ocr))
            print(p["id"], f"{(d - a) * 1000:.0f}ms", cap[:90], "| OCR:", ocr[:40], flush=True)
    k = len(rows)
    stats = dict(model=size, provider=provider, photos=k, load_s=round(load, 2),
                 files_mb=round(sum(os.path.getsize(x) for x in f.files) / 1e6),
                 ms_vision=round(t_vis / k * 1000), ms_caption=round(t_cap / k * 1000),
                 ms_ocr=round(t_ocr / k * 1000), ms_total=round((t_vis + t_cap + t_ocr) / k * 1000),
                 caption_tokens_mean=round(n_tok / k, 1), vram_loaded_mb=loaded,
                 vram_peak_mb=vp.peak - idle)
    print(stats)
    dump(f"florence_{provider}{'' if size == 'base' else '_' + size}.json", dict(stats=stats, rows=rows))
