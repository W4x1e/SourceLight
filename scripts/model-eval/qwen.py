"""Florence-2 spike baseline: qwen2.5vl:7b captions exactly as the app asks for them —
the pinned Ollama runtime (v0.32.14), CAPTION_PROMPT, the photo's longest
edge at 1280 px, temperature 0.2, num_ctx 4096. -> out/qwen.json"""
import base64, io, json, sys, time, urllib.request
from common import PHOTOS, image, VramPeak, vram_used_mb, dump

HOST = "http://127.0.0.1:41439"
PROMPT = """Write a short, factual description of this photograph for a searchable photo
library. In two or three sentences, cover: the main subject, the setting or
location, notable colours, any people or activity, and the overall mood or
lighting. Use plain, concrete words a person would actually search for.

If any text is clearly legible in the image — signs, labels, documents, screens,
book spines — transcribe it verbatim in the same description.

Reply with the description only: no preamble, no bullet points, no quotation."""


def post(path, body):
    req = urllib.request.Request(HOST + path, json.dumps(body).encode(), {"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=600))


def b64(im):
    im = im.copy(); im.thumbnail((1280, 1280))
    buf = io.BytesIO(); im.save(buf, "JPEG", quality=90)
    return base64.b64encode(buf.getvalue()).decode()


if __name__ == "__main__":
    MODEL = sys.argv[1] if len(sys.argv) > 1 else "qwen2.5vl:7b"
    limit = int(sys.argv[2]) if len(sys.argv) > 2 else len(PHOTOS)
    idle = vram_used_mb()
    t0 = time.perf_counter()
    post("/api/generate", {"model": MODEL, "prompt": "", "keep_alive": "30m"})  # load
    load = time.perf_counter() - t0
    loaded = vram_used_mb()
    rows, total, toks = [], 0.0, 0
    with VramPeak() as vp:
        for p in PHOTOS[:limit]:
            t = time.perf_counter()
            r = post("/api/chat", {"model": MODEL, "stream": False, "keep_alive": "30m",
                                   "messages": [{"role": "user", "content": PROMPT, "images": [b64(image(p))]}],
                                   "options": {"temperature": 0.2, "num_ctx": 4096}})
            dt = time.perf_counter() - t
            total += dt; toks += r.get("eval_count", 0)
            rows.append(dict(id=p["id"], caption=r["message"]["content"].strip()))
            print(p["id"], f"{dt * 1000:.0f}ms", rows[-1]["caption"][:100], flush=True)
    post("/api/generate", {"model": MODEL, "keep_alive": 0})  # unload, so the next model starts clean
    stats = dict(model=MODEL, photos=len(rows), load_s=round(load, 1), ms_per_photo=round(total / len(rows) * 1000),
                 caption_tokens_mean=round(toks / len(rows), 1), vram_loaded_mb=loaded - idle,
                 vram_peak_mb=vp.peak - idle)
    print(stats)
    dump("qwen.json" if MODEL == "qwen2.5vl:7b" else f"qwen_{MODEL.replace(':', '_')}.json", dict(stats=stats, rows=rows))
