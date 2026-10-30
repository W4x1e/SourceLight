"""The VLM tagging path (the old Ollama tagger) per model: the app's PROMPT and
JSON schema, lifted from the Rust source; 896px edge, temperature 0,
num_predict 256. Scores valid-JSON rate and genre accuracy on the photos
whose genre is clear-cut (tagcheck.GENRE), and times it."""
import json, os, re, sys, time
from common import PHOTOS, image
from queries import SCENE
from qwen import b64, post
import qwen

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "src-tauri", "src")
src = open(os.path.join(REPO, "tagger", "mod.rs"), encoding="utf-8").read()
PROMPT = re.search(r'pub\(crate\) const PROMPT: &str = "\\\n(.*?)";', src, re.S).group(1).replace('\\"', '"')
GENRES = re.findall(r'"([a-z-]+)"', re.search(r"pub const GENRES: &\[&str\] = &\[(.*?)\];",
                                                open(os.path.join(REPO, "models.rs"), encoding="utf-8").read(), re.S).group(1))
SCHEMA = {"type": "object", "properties": {
    "main_subject": {"type": "string"}, "subjects": {"type": "array", "items": {"type": "string"}},
    "conditions": {"type": "array", "items": {"type": "string"}}, "genre": {"type": "string", "enum": GENRES},
    "confidence": {"type": "number"}}, "required": ["main_subject", "subjects", "conditions", "genre", "confidence"],
    "additionalProperties": False}
GENRE = {"macro": {"hydrangea", "bee", "sunflower"}, "wildlife": {"gull", "aerial_bird"},
         "landscape": {"clifftop_bay", "kynance", "town_view", "sea_cliff", "cove", "willow"},
         "portrait": {"door_portrait", "headphones"}, "architecture": {"castle", "church_ext", "chapel", "clocktower"},
         "still-life": {"ice_cream", "cafe"}}
GENRE_OF = {s: g for g, ss in GENRE.items() for s in ss}

if __name__ == "__main__":
    model, host = sys.argv[1], sys.argv[2]
    qwen.HOST = host
    photos = [p for p in PHOTOS if SCENE[p["id"]] in GENRE_OF]
    post("/api/generate", {"model": model, "prompt": "", "keep_alive": "30m"})
    ok = right = 0; total = 0.0; rows = []
    for p in photos:
        im = image(p).copy(); im.thumbnail((896, 896))
        import io, base64
        buf = io.BytesIO(); im.save(buf, "JPEG", quality=90)
        t = time.perf_counter()
        r = post("/api/chat", {"model": model, "stream": False, "keep_alive": "30m", "format": SCHEMA,
                               "messages": [{"role": "user", "content": PROMPT,
                                             "images": [base64.b64encode(buf.getvalue()).decode()]}],
                               "options": {"temperature": 0.0, "num_ctx": 4096, "num_predict": 256}})
        total += time.perf_counter() - t
        try:
            j = json.loads(r["message"]["content"]); ok += 1
            right += j["genre"] == GENRE_OF[SCENE[p["id"]]]
            rows.append(dict(id=p["id"], **j))
        except Exception as e:
            rows.append(dict(id=p["id"], error=str(e), raw=r["message"]["content"][:200]))
    post("/api/generate", {"model": model, "keep_alive": 0})
    stats = dict(model=model, photos=len(photos), valid_json=ok, genre_acc=round(right / len(photos), 3),
                 ms_per_photo=round(total / len(photos) * 1000))
    print(stats)
    json.dump(dict(stats=stats, rows=rows), open(f"out/tag_{model.replace(':', '_')}.json", "w"), indent=1)
