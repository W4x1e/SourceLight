# Model evaluation harness

The harness behind [`docs/AI-MODEL-SPIKES.md`](../../docs/AI-MODEL-SPIKES.md).
It runs candidate models over the photos in the app's own catalogue and scores
them against hand-labelled ground truth. It uses ONNX Runtime with DirectML
from Python: the same runtime the app loads from Rust.

It is not part of the build and nothing here ships. The photos stay on the
machine, and the models, outputs and contact sheets are git-ignored.

## Setup (Windows)

```bash
cd scripts/model-eval
python -m venv venv && ./venv/Scripts/python -m pip install -r requirements.txt
./fetch-models.sh        # every variant measured, ~5.7 GB; "min" for ~1.2 GB
```

CLIP is read from the app's installed models, so install the built-in tagger in
the app first. `qwen.py` needs an Ollama server with qwen2.5vl:7b pulled; the
app no longer ships one.

## Running

| Step | Script | Writes |
|---|---|---|
| 0 | `sheets.py`: dump the catalogue, draw numbered contact sheets | `photos.json`, `sheets/` |
| 1 | `embed.py dml clip fp16 [fp32 quantized]` (and `cpu`) | `out/emb_*.npz`, timings |
| 2 | `evaluate.py out/emb_*.npz`: search, Similar, duplicates | `out/retrieval.json` |
| 3 | `tagcheck.py`: tag ranking over `subjects.txt`, genre | `out/tags.json` |
| 4 | `florence.py hybrid 194 [base\|large]` | `out/florence_hybrid*.json` |
| 5 | `qwen.py` | `out/qwen.json` |
| 6 | `captionsearch.py`: FTS5 + RRF, as the app searches | `out/captionsearch.json` |
| 7 | `qwen_tag.py <model> <host>`: the VLM tagging path, JSON validity and genre | `out/tag_*.json` |
| 8 | `probe_people dump` (Rust, see below), then `faces_eval.py`: embedders on the shipped crops | `out/faces.json` |
| 9 | `faces_lfw.py`, `faces_lfw_cluster.py`: calibration and clustering at scale on LFW | `out/faces_lfw*.json` |
| 10 | `faces_policy.py`, `cluster_proto.py`: vote gate, attach and thresholds | stdout |
| 11 | `searchcheck.py <probe_search.exe> <siglip2 dir> <library.db copy>`: the app's own Rust search and tagging path, scored with the same queries | stdout |
| 12 | `vlm_serving.py`: how fast each way of serving the vision model tags and describes (see below) | `out/vlm_serving.jsonl` |

The face steps need the probe: `cargo build --release --features ai,dev-tools --bin
probe_people`, then `probe_people dump <faces models dir> photos.json faces`.
`faces_truth.py` labels the faces that dump found in *this* library by
identity, so relabel it for any other. LFW comes from the `logasja/lfw` pairs
parquet on Hugging Face and is used for calibration only; nothing from it
ships.

Set `PYTHONIOENCODING=utf-8` on Windows; captions contain non-ASCII text.

## Reading text in photos

A separate sample and ground truth, in `ocr-data/` (git-ignored). The models
are fetched into `models/ocr/` by `fetch-models.sh`. It needs `opencv-python`
on top of `requirements.txt`.

| Step | Script | Writes |
|---|---|---|
| 0 | `ocr_pick.py <the app's data folder>`: copy the catalogue, pick 200 photos (every one keyworded as showing a sign, the rest at random), copy their previews | `ocr-data/photos.json`, `ocr-data/photos/` |
| 1 | `ocr_sheets.py`: numbered contact sheets; then label `ocr_truth.py` from the sheets and each candidate at full size | `ocr-data/sheets/` |
| 2 | `ocr_read.py <detector max side> [threads]`: PP-OCR with PaddleOCR's own pre- and post-processing, on the CPU | `ocr-data/out/ocr_<side>.json` |
| 3 | `ocr_score.py <run> [min conf] [--substr] [--detail]`: words read, photos findable, false positives | stdout |
| 4 | `ocr_crops.py <run> <min conf> <ids> <out.jpg>`: what was read where, for checking by eye | an image |
| 5 | `ocr_probe.rs` (copy to `src-tauri/tests/`): the same shapes through the app's own session builder, DirectML and CPU | stdout |

## The ground truth is tied to one library

`queries.py` and `ocr_truth.py` refer to photos by **library id** in the
catalogue these numbers came from. For another library, redo step 0 and relabel from the sheets
*before* running any model. The labels must not be shaped by what a model
returned.

## Gotchas this harness already hit

- SigLIP 2 fp16 needs **full** graph optimisation on DirectML (Basic gives
  wrong embeddings) and **Basic** on CPU (full fails to load). `embed.py` and
  `florence.py` pick the level per provider.
- Florence-2's fp16 merged decoder won't load; use the fp32 one. DirectML
  corrupts its cached-decode branch, so `florence.py hybrid` runs the decoder
  on the CPU.
- Florence-2 forces `<s>` as the first generated token. Leave that out and you
  get fluent-looking nonsense.

## The shipped path, end to end

`probe_search` (`cargo build --release --features ai,dev-tools --bin
probe_search`) indexes a *copy* of the catalogue through `BuiltinTagger` and
`db::set_embedding`, and ranks each query with `search::rank`, the semantic
half of search. `searchcheck.py` scores it. Set `ORT_DYLIB_PATH` to
`src-tauri/vendor/onnxruntime/onnxruntime.dll` to test the DirectML runtime
the app ships. On 2026-09-24 it reproduced the Python numbers: mAP 0.909
(English) and 0.923 (translated), with the key subject tagged on 66% of photos.

## Serving the vision model

`vlm_serving.py` times the app's vision-model requests. It sends them to
Ollama's `/api/chat` the way the old Ollama tagger did, or to `llama-server`'s
`/v1/chat/completions` the way `tagger/llama.rs` does. It reads the prompts
and the schema from the Rust source. It needs only Pillow, and runs on Windows
and macOS.

`serve` starts `llama-server` with the flags `sidecar::llama_server_args`
gives it, runs the tasks, then stops it:

```bash
python vlm_serving.py serve <llama-server> <model.gguf> <mmproj.gguf> \
  --slots 2 --device Vulkan0 --tasks tag:640 describe:1280 both:1280 \
  --labelled --label "b11234 vulkan 4B"
```

- **Which photos:** `--labelled` runs the 71 library photos whose genre is
  clear-cut, and scores genre accuracy. Otherwise it samples `--n` of the
  app's previews evenly.
- **`--grammar oneline`** keeps the JSON reply on one line: fewer tokens.
  Allowing no whitespace at all breaks the model.
- **`bench`** times one task against a server that is already running,
  whether Ollama or `llama-server`.
