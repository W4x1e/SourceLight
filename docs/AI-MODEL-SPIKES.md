# Two model spikes: SigLIP 2 and Florence-2

Measured 2026-09-24 on the real library (194 Sony a6400 photos, mostly UK
travel: gardens, coast, harbours, a castle, people), an RTX 5070 Ti, Windows 11.
Both spikes used one harness, ground truth and set of photos, so the results
can be compared directly.

**The short version.**

- **Switch to SigLIP 2.**
  - It is better at nearly everything search does.
  - It is much better for anyone searching in a language other than English.
  - It runs as fast as CLIP on the GPU, and the download is 177 MB larger.
  - It needs care in two places: graph-optimisation level per execution provider, and re-tuned thresholds.
- **Don't adopt Florence-2.**
  - Once search runs on SigLIP 2, captions add only 0.6–1.6 points of mAP from any captioner. Search no longer needs a VLM.
  - Florence-2's descriptions are noticeably worse than qwen2.5vl's, and they are shown to the user.
  - Its OCR is too noisy to index.
  - The large model is slower than qwen.
  - The better move is to make Ollama *optional*, and not to replace it.

## How it was measured

- **Ground truth before any model output.** The next three bullets were all written from numbered contact sheets of all 194 photos (`sheets.py`) before any model had been run.
- **Queries.** 38 English search queries, each phrased the way someone types into a search box, with the set of photos that clearly show the thing asked for.
- **Translated queries.** 12 of those needs asked in French, German, Spanish or Italian, scored against the same sets.
- **Scenes.** Every photo was assigned to a scene (54 scenes), and 41 pairs were marked as near-duplicates.
- **Search** is scored the way `search::rank` works: every photo is ranked by cosine, and the ranking is scored with mAP, recall@10 and precision@5.
- **Similar:** is a photo's nearest neighbour from the same scene?
- **Duplicates:** how well cosine separates the near-duplicate pairs from pairs in different scenes (ROC AUC).
- **Tagging** is scored without thresholds, because the shipped selection is tuned to CLIP.
  - For the 96 photos whose scene has an unambiguous label in `subjects.txt`: where does that label rank among the 1,623?
  - For 65 photos with a clear-cut genre: is the top genre right?
- **Captions are scored through the app's real search path.**
  - The caption half is FTS5 with the same tokenizer and `fts::build_query` shape.
  - That list is fused with the semantic ranking by RRF (k = 60), as `resolve_search_ids` does.
- **Captions are also read side by side.** They are shown in the lightbox and the info panel, so recall alone isn't enough.
- **CLIP is run exactly as shipped.** It is the installed `models/clip` files with `clip_preprocess` reproduced.
- **qwen2.5vl:7b is run exactly as shipped.** It is the pinned Ollama runtime (v0.32.14) with `CAPTION_PROMPT`, a 1280 px edge, temperature 0.2 and `num_ctx` 4096.

The harness is in `scripts/model-eval/` (see its README). It is ONNX Runtime 1.24.4 (DirectML) from Python, which calls the same runtime the app does from Rust.

**Limits.**
- One library, one camera, one person labelling.
- 194 photos is enough to show large differences, not small ones. Treat differences under ~0.02 mAP as noise.
- Only a handful of photos contain text, so the OCR finding is qualitative.

---

## SigLIP 2 vs CLIP ViT-B/32

`onnx-community/siglip2-base-patch16-224-ONNX` @ `ba1f3b0`. It is an export of `google/siglip2-base-patch16-224`, which is **Apache-2.0**. The export repo declares no licence of its own; the weights are a derivative of the Apache-2.0 upstream.

### Quality

| | CLIP B/32 | SigLIP 2 (fp16) |
|---|---|---|
| Search, English: mAP | 0.809 | **0.908** |
| Search, English: recall@10 | 0.864 | **0.915** |
| Search, English: precision@5 | 0.835 | **0.917** |
| Search, other languages: mAP | 0.587 | **0.923** |
| Similar: nearest neighbour from same scene | 0.918 | 0.923 |
| Duplicates: ROC AUC | 0.9998 | 1.0000 |
| Tagging: the right label in the top 5 | 0.760 | **0.802** |
| Tagging: mean reciprocal rank | 0.469 | **0.520** |
| Genre: top-1 accuracy (65 photos) | 0.815 | 0.800 |

**English queries** (38 in all):
- 17 get better and 3 get worse, by more than 0.05 each.
- The biggest gains: "yellow boat" 0.26 → 0.85, "man with a backpack" 0.23 → 0.60, "coffee cup" 0.33 → 1.00, "aerial view of a town and causeway" 0.65 → 1.00, "woman in a red jacket" 0.47 → 0.72.
- The losses: "bee on a flower" 0.92 → 0.73, "dusk by the water" 0.86 → 0.79, "someone taking a photo with a camera" 0.79 → 0.74.

**Other languages:**
- CLIP B/32 is English-only, and it shows. "vidriera" (stained glass) scores 0.01, "Burg auf einem Hügel" 0.15 and "bateaux dans le port" 0.28.
- SigLIP 2 scores 0.83, 0.89 and 1.00 on the same three.
- For a UK/EU audience this is the headline.

### Cost

| | CLIP B/32 | SigLIP 2 fp16 |
|---|---|---|
| Download | 608 MB | 785 MB (vision 186 MB, text 565 MB, tokenizer 34 MB) |
| GPU (DirectML), per image, batch 16 | 8.9 ms | 8.7 ms |
| GPU, per query | 4.7 ms | 3.8 ms |
| CPU only, per image, batch 16 | 17 ms | 59 ms |
| Embedding | 512-d | 768-d (+50% per row in `photos.embedding`) |

The text tower is most of the size: a 256k-token multilingual vocabulary.

### Things that will bite

1. **Graph-optimisation level must depend on the execution provider.**
   - The fp16 vision graph *fails to load* on the CPU provider at the default level (Level 3). ORT's layer-norm fusion trips on an inserted cast.
   - At Basic it loads, and it is exact on CPU.
   - **On DirectML, Basic gives wrong answers.** Unfused fp16 layer norm loses precision, and embeddings drift to a mean cosine of 0.72 against fp32. Search falls *below* CLIP (mAP 0.79).
   - DirectML at Level 3 is exact (cosine ≥ 0.9999 against fp32).
   - `tagger/builtin.rs::build_session` registers DML → CUDA → CPU with the default level. So it needs Level 3 when DirectML is present, and Level 1 (Basic) on the CPU fallback.
   - Add a regression test for this: embed a fixed image and compare with a stored fp32 vector.
2. **Don't use the int8 (`_quantized`) export.**
   - At 412 MB it's tempting, but its embeddings are far off fp32 (mean cosine 0.80).
   - It scores *worse than CLIP*: mAP 0.719 in English and 0.536 in other languages.
   - fp32 (1.5 GB) is exact but pointless on a GPU; fp16 matches it. On a PC with no GPU the fp32 vision tower is what runs, since 2026-10-11: twice as fast as fp16 on the CPU ([BENCHMARKS.md](BENCHMARKS.md#slower-machines-2026-10-11)).
3. **Text needs SigLIP's recipe.** Lower-case the text, let the tokenizer append EOS, and pad to 64 with `<pad>`. SigLIP was trained with max-length padding. CLIP's "pad with 0 to 77" is wrong here.
4. **Image preprocessing is different.** It's a plain resize to 224×224 with no centre crop, normalised with mean and std 0.5.
5. **Most thresholds tuned to CLIP move.**
   - *Near-duplicate threshold* (`DUPLICATE_THRESHOLD = 0.92`). On these labels the best-F1 threshold is 0.96 for SigLIP 2 against 0.95 for CLIP. Re-fit it on a bigger labelled set rather than carrying 0.92 over.
   - *Similar* (`SIMILAR_THRESHOLD = 0.8`) can stay. At 0.8, SigLIP 2 keeps 92% of same-scene pairs against CLIP's 83%, and lets in the same ~5% of pairs from different scenes.
   - *Tagger selection.* It uses softmax at CLIP's logit scale of 100. SigLIP scores are sigmoid-calibrated, so `SUBJECT_MIN_PROB`, `POOL_FLOOR` and the scale need re-tuning. The ranking is already better (table above).
6. **Migration.**
   - Stored 512-d CLIP vectors can't be compared with 768-d SigLIP ones.
   - `search::rank` (search and Similar), near-duplicate grouping and `tag_embedding` all skip vectors of the wrong length already. A half-migrated library degrades safely: photos not yet re-indexed just don't match.
   - Store the embedding model per row, or clear the embeddings on switch.
   - Re-index in the background with the existing auto-analysis step. It costs about 9 ms a photo on a GPU, roughly 8 minutes per 50,000 photos.
   - `labels.bin` is keyed on the model, so it rebuilds by itself.

**Recommendation: switch.** A follow-up change does it, with the migration and re-tuning above.

**Done.**
- **Graph optimisation:** Level 3 with DirectML, and Level 1 on the CPU fallback.
- **Pinned files:** the fp16 files, each with a SHA-256.
- **Tagger:** subjects use a logit scale of 160; genre and conditions stay at 100 (`scripts/model-eval/tagscale.py`).
  - At 160, a photo's clear subject is tagged on 66% of labelled photos, against CLIP's 58%, at ~3.7 tags per photo.
  - Genre confidence has a median of 0.86 at 100; at 160 it would saturate near 1.
- **Duplicate presets:** mapped by matching pair counts. Normal 0.92 → 0.935, Loose 0.86 → 0.855, Strict 0.96 → 0.97, Near-identical 0.985 → 0.99. At 0.935 every flagged pair on the library was same-scene.
- **Old embeddings:** migration 20 clears CLIP's 512-d vectors so photos re-index.
- **Retired files:** CLIP's and FaceX's files are removed at launch.
- **Regression test:** `tests/ai_models.rs::siglip2_matches_the_fp32_reference` checks the app's Rust path against fp32 CPU reference vectors. It passes on the shipped DirectML runtime (ORT 1.22) and on the CPU fallback.
- **End to end:** the shipped Rust path on a copy of the real catalogue (`probe_search`, `scripts/model-eval/searchcheck.py`) scores mAP 0.909 in English and 0.923 in other languages, and tags the key subject on 66% of photos. That's the same as the Python spike, on the vendored DirectML runtime, indexing 194 photos in 5.8 s.

---

## Florence-2 vs qwen2.5vl:7b for descriptions

Florence-2-base-ft @ `e88a44e` and Florence-2-large-ft @ `04ace74`, both from `onnx-community` and both **MIT**. They were run in-process on ONNX Runtime. qwen2.5vl:7b was run through the app's own pinned Ollama runtime.

### Search: captions barely matter any more

mAP on the 38 English queries, semantic ranking fused with each caption source:

| Semantic half | none | Florence-2 base | Florence-2 large | qwen2.5vl:7b |
|---|---|---|---|---|
| CLIP B/32 (today) | 0.809 | 0.835 | 0.854 | 0.855 |
| **SigLIP 2** | **0.908** | 0.914 | 0.924 | 0.920 |

Today, with CLIP, captions add 4–5 points. With SigLIP 2 they add 0.6–1.6 points, which is inside the noise on this set. On its own, caption search finds anything at all for only a third of these queries (29–39%), because `build_query` ANDs every word. Captions don't help non-English queries at all, since they are written in English.

### Descriptions: qwen is clearly better, and the user reads them

Read side by side on 17 photos:

- **qwen2.5vl names the subject.**
  - "An ice cream cup with a spoon… spill" (base: "a red and white container"; large: "a piece of white and red paper").
  - "a castle-like structure… atop a small island" (both Florence models miss it).
  - "a pigeon perches on a TV antenna atop a chimney".
  - "a sign for 'Harbour Inn'".
- **Florence-2 is literal and repetitive.**
  - "The pole is brown. There are two orange cones on the pole."
  - It sometimes hallucinates (base put a lighthouse on photo 60), and so does qwen ("taking a selfie", "WELCOME").
  - Large is tidier than base, but misses the same subjects.
- **Florence-2's OCR isn't usable as-is.**
  - On photos with no text it returns junk (`ـحـ`, `-`, `COURTESY`).
  - It stutters on real text ("HARBOURHARBOULHARBORHARBOUT", "BEACHDO T").
  - Large reads a boat registration ("PZ 28") that qwen doesn't mention.
  - Indexing it would need a confidence filter that Florence doesn't expose.

### Cost

| | Florence-2 base | Florence-2 large | qwen2.5vl:7b (as shipped) |
|---|---|---|---|
| Download | 740 MB | 2.16 GB | 1.46 GB runtime + ~6 GB model |
| Process | in-process ORT | in-process ORT | second process, loopback port |
| Per photo, GPU (caption + OCR) | 0.53 s | 1.45 s | 1.20 s (caption only) |
| Per photo, CPU only | ~1.8 s | not measured | not measured |
| VRAM loaded / peak | 0.28 / 0.9 GB | 1.0 / 2.1 GB | 6.7 / 6.8 GB |

### Things that went wrong with the ONNX exports

- **The fp16 merged decoder is an invalid graph.** Its `If` subgraph returns an outer-scope value directly, and ORT refuses to load it. The fp32 decoder (388 MB base, 975 MB large) has to be used instead.
- **DirectML corrupts the merged decoder's cached branch.**
  - Step 0 (`use_cache_branch = false`) matches CPU exactly.
  - From step 1 on, the self-attention cache comes back as garbage, and so does the output text.
  - The fix is to run the vision tower and encoder on the GPU and the per-token decoder on the CPU. That's where the times above come from.
  - Shipping this would mean owning that split, the generation loop and its KV-cache bookkeeping in Rust.
- `generation_config` forces `<s>` as the first generated token. Leave it out and the output is fluent-looking nonsense.

### Follow-up: which Ollama model, now that Qwen3-VL is listed

The spike above measured qwen2.5vl:7b because it was the recommended model for GPUs with 8 GB or more. The catalogue already listed Qwen3-VL 2B and 4B (Apache-2.0) for smaller GPUs, and Qwen3-VL 8B (Apache-2.0) wasn't listed at all. So all three were run through the same harness: the same pinned runtime, prompts and settings.

| | Qwen3-VL 2B | **Qwen3-VL 4B** | Qwen3-VL 8B | qwen2.5vl:7b |
|---|---|---|---|---|
| Download | 1.9 GB | 3.3 GB | 6.1 GB | 6.0 GB |
| VRAM loaded | 2.9 GB | **4.6 GB** | 7.2 GB | 6.7 GB |
| Describe, per photo | 1.49 s | 1.39 s | 1.81 s | 1.20 s |
| Tag, per photo (JSON schema) | 0.64 s | 0.82 s | 1.29 s | 1.02 s |
| Valid tag JSON | 71/71 | 71/71 | 71/71 | 71/71 |
| Search mAP with SigLIP 2 + captions | 0.929 | 0.925 | 0.928 | 0.920 |
| Search mAP with CLIP + captions | 0.891 | 0.871 | 0.872 | 0.855 |

**Search.** Every Qwen3-VL caption helps search at least as much as qwen2.5vl's. The 2B helps most, because it writes longer captions (111 tokens against 68).

**Genre accuracy** doesn't separate them. It ranged from 0.76 to 0.92 on 71 photos, and almost every disagreement is one my labels settle arbitrarily:
- bees called "wildlife" rather than "macro" (the prompt allows both);
- a man gazing at the view called "portrait" (the prompt says a person as the subject isn't landscape);
- a harbour town called "landscape" rather than "architecture".

**Descriptions**, read side by side:
- **8B is the best.**
  - It names St Michael's Mount.
  - It reads "The North Face" and the "BEACH →" sign.
  - It gets the ice cream right.
  - It sometimes names a town it can't know ("Brixham").
- **4B matches qwen2.5vl 7B**: the castle, the ice cream, the signs.
- **2B makes real mistakes**: "Wendy's", and a spilled cup instead of ice cream.

**Change made:** Qwen3-VL now takes every tier.

| GPU memory | Recommended model |
|---|---|
| 12 GB+ | 8B |
| 6–12 GB | 4B (the default, and the pick for the common 8 GB card) |
| under 6 GB | 2B |

qwen2.5vl:7b stays listed, marked as superseded, so a setup that already uses it keeps working.

**On a Mac** (16 GB M4, measured 2026-09-27) the same models run 6–12× slower — 4B tags in
8.2 s and describes in 7.9 s — and Ollama runs the Qwen3-VL family one request at a time
on every platform. The table and what the runs changed are in
[MACOS.md](MACOS.md#the-local-vision-model).

### Recommendation: don't adopt Florence-2; make descriptions optional

**Florence-2 doesn't buy what the issue hoped for.**
- Its search gain over the SigLIP 2 baseline is within noise.
- Its descriptions are a visible step down from what users see today.
- Large, the only version close on quality, is slower than qwen.
- The exports need workarounds on DirectML.

**What the numbers do support:**
1. Switch search to SigLIP 2 first. With SigLIP 2, **search no longer depends on a VLM**. That was the main reason Ollama was a core dependency.
2. Then make descriptions an **optional add-on**: the existing Ollama runtime and model pack, off by default and offered in Settings → AI as "Detailed descriptions (6 GB download, GPU with 8 GB+)".
   - The base install becomes SigLIP 2 plus faces, with no second process, no port and no multi-GB pull.
   - macOS AI doesn't have to wait for an Ollama story.
3. If searching for text in photos turns out to matter (signs, documents), spike a dedicated OCR model rather than a captioner doing OCR on the side. PP-OCR (Apache-2.0) has ONNX exports. This library had only ~6 photos with legible text, too few to judge.

This makes deleting the Ollama path a product decision rather than a technical one. Keeping it optional costs little once nothing else depends on it.

---

## People grouping: FaceX nano → AuraFace, centroid → average linkage

**Problem.** The same person landed in several groups, and some groups mixed strangers. Measured with the shipped detector and alignment (`probe_people dump`) on the library's 43 detected faces, labelled by identity from crops. On 32 of them the app lets the embedding vote.

### The embedder

| Embedder | Separation on the library's faces (AUC) | LFW (2,200 pairs) AUC | Size | Licence |
|---|---|---|---|---|
| FaceX nano (shipped) | **0.69** | 0.9975 | 0.8 MB | Apache-2.0 (trained on MS1M) |
| SFace (OpenCV Zoo) | 0.90 | 0.9989 | 39 MB | Apache-2.0 |
| **AuraFace v1** | **0.90** | 0.9989 | 261 MB | Apache-2.0, commercially licensed training data |

**LFW is easy, and the library is not.**
- **LFW** is mostly clean frontal portraits. On it, FaceX looks fine (AUC 0.9975), which is how its thresholds were calibrated.
- **This library** has sunglasses, hats, profiles and small faces, and there FaceX is close to chance (0.69). AuraFace and SFace hold at 0.90.
- **With thresholds tuned to each model** on the library, the best F1 over all faces is FaceX 0.32, SFace 0.71, AuraFace 0.85.
- **The pick is AuraFace.** It groups best on the real photos, and it has the cleanest licence provenance of the three.

### The clustering

The merge stage compared re-normalised centroids, and that snowballs (see `people.rs`). On 3,443 LFW faces of 2,132 people:

| Merge criterion (AuraFace, core 0.75) | Precision | Recall | Groups per person |
|---|---|---|---|
| normalised centroid, merge 0.40 | 0.33 | 0.99 | 1.03 |
| **average linkage, merge 0.40** | **0.95** | **0.97** | 1.04 |

Loosening the merge to 0.36 costs 10 points of precision at that scale, so 0.40 is the bar.

**Voting.** Only faces with a detection score ≥ 0.75 and a pose offset ≤ 0.6 found or merge a group. Other groupable faces attach to the group they agree with best, at ≥ 0.40 average, or stay unplaced.

### End to end on the library

This ran through the real `store_faces` and `regroup_people` on a copy of the catalogue, with migrations applied (`probe_people library`):

- **Every group is one person.** The exception is a group of older men who may be the same man; I couldn't tell by eye.
- **The young woman photographed with and without a hat and sunglasses is in two groups.** The one suggestion offered is exactly that pair (average 0.27). The next candidate would have been two different people at 0.19, below the 0.25 floor.
- **Profiles, backs of heads and faces under ~32 px are left unplaced** rather than forced into someone.
- **Before, with FaceX and centroid merging,** the same woman was in 6 groups, and pairwise precision was 0.27.
- **Re-analysis keeps names and "not the same" answers.** Re-running every photo after naming two people put both names back on exactly the same faces.

**Harness.** `scripts/model-eval/`: `faces_eval.py`, `faces_lfw.py`, `faces_lfw_cluster.py`, `faces_policy.py` and `cluster_proto.py`, with the probe in `src-tauri/src/bin/probe_people.rs`.

## Eyes open or shut — OCEC → MediaPipe face mesh + blendshapes

OCEC classified crops around YuNet's eye points. On the owner's real candid
photos it read most open eyes as shut: 9 of 16 plainly open-eyed faces on a
labelled sheet, nearly all of them smiling, and about 80% of all faces. A
smile's narrowed eye reads as a blink to it at any resolution.

MediaPipe's Face Landmarker splits the problem differently. A face mesh
(FaceMesh V2, 478 landmarks) then a blendshape model (52 expression scores
from 146 of those landmarks) keeps `eyeBlink` apart from `eyeSquint` and
`cheekSquint`, which is where a smile goes. Both are Apache-2.0; the ONNX
conversions pinned in `ai_models.rs` are MediaMolder's release of Google's
`face_landmarker.task`, bit-for-bit identical in output to PINTO0309's
independent conversion.

The app's pipeline (`faces.rs`): three looks of the mesh, each centred and
levelled on the last (from the detector's box; a loose first box left one
refinement off the face on a close portrait), then the blendshapes. The
blendshape model normalises its input, so scaling or shifting the landmarks
changes nothing; rolling them does.

Measured with `scripts/model-eval/eyes_eval.py` (2026-09-28):

| Labelled faces | Result | Wanted |
|---|---|---|
| Open eyes, 50 | 43 open, 0 shut, 7 can't tell (under 64 px) | none shut |
| Sunglasses, 34 | 20 open, 0 shut, 14 can't tell | none shut |
| Looking down, 10 | 9 open, 1 shut | either: the lids are down |
| Not a readable face (profile, rock, arm), 10 | 9 can't tell, 1 open | never shut |
| Eyes plainly shut (Commons portraits), 8 | 5 shut, 2 open, 1 can't tell | all shut |

What decided the rule (`eye_open_from`):

- **The blendshapes read blinks low.** Plainly shut eyes scored 0.46–0.77, not
  the 0.9 the model's reputation suggests, and open eyes up to 0.47. A single
  threshold on `eyeBlink` either misses blinks or catches downcast gazes.
- **The lid gap separates them.** The eye aspect from the mesh reads 0.03–0.10
  for shut eyes and 0.13 up for open ones. Looking down sits in between on
  both measures, which is honest.
- **The smaller eye, not the larger.** Sunglasses and turned faces put one
  eye high and the other low; a blink closes both.
- **Readability first.** The mesh's own presence score is under 0.2 for
  profiles and false detections, and a face under 64 px in the 1600 px preview
  drifts toward "shut" whatever it does. Those read "can't tell", which never
  counts as shut.

Shut = both eyes' blink ≥ 0.6, or ≥ 0.45 with the lids nearly touching (aspect
≤ 0.10). Open-eye accuracy on readable faces went from 44% (OCEC) to 100%.

## AI denoise — SCUNet on DirectML

Spiked 2026-09-28 on the owner's noisiest real frame: a Sony a6400 raw at
ISO 6400 (indoors, a stone wall, a wooden door, a striped top), developed at
defaults through the app's own pipeline to a 6024×4024 image, then denoised.

**Model:** SCUNet `color_real_psnr` (Zhang et al. 2022; Apache-2.0), the blind
real-noise checkpoint, as ONNX from `Heliosoph/scunet-onnx` (opset 17, 77 MB
with its external weights). NAFNet was the other candidate. Its ONNX builds
on Hugging Face are either Qualcomm's (behind their own asset hub) or empty,
and its SIDD weights are trained on phone-sensor noise.

**DirectML needs static shapes.** With free `height`/`width` dimensions the
session crashes the process (a segfault, no error) on the first run. Pinning
them with `add_free_dimension_override_by_name` to the tile size fixes it.
The same caution as `directml-batch`: never let this fail silently.

**Speed** (RTX 5070 Ti): 71 ms per 256² tile; 486 tiles with a 32 px blended
overlap = **35 s for 24 MP**. On the CPU a tile takes 2.3 s, so a whole frame
would take about 18 minutes: a GPU-only feature, or a very patient one.

**Quality**, side by side at 1:1 against the app's current noise reduction
(Detail → Noise 0.6, Colour noise 0.6):

- Colour noise and luminance blotches go entirely; the current NR leaves both.
- Edges stay crisp: stripes, hair strands.
- At full strength fine texture goes too: stone and wood grain turn smooth
  and plasticky. A PSNR-trained model does this.
- **Blended back at about 60%** the grain goes and the texture stays. An
  Amount control is not optional.

Tone is untouched (mean level within 1/255, 5th/95th percentiles within 3).

**Recommendation: build it**, as an explicit action ("Denoise…") rather than
a live slider:

- Run on the demosaiced linear image at `prepare` time, encoded to sRGB for
  the model and decoded back, tiled at a fixed 256 px on DirectML/CoreML, as a
  background job with progress (tens of seconds).
- Cache the result beside the linear texture, keyed by the photo's hash. The
  develop pass mixes the two by an Amount slider, default about 60%, so
  adjusting it is instant and the edit stays non-destructive.
- Download on demand like the other large models (77 MB), GPU-only, with the
  speed stated before the first run.

**Built** as the spike recommended, with two changes. The result is kept in
memory for the open photo rather than on disk: it is quick to recompute
relative to everything else a reopened photo needs, and a disk cache of
190 MB half-float frames per photo would grow without bound. So a reopened
photo with an Amount shows undenoised until it is run again, and says so;
exports run it themselves. And the Amount is blended on the CPU and
uploaded, rather than mixed in the develop shader, which would need a second
full-size linear texture bound on every render for a control that changes
rarely. `denoise.rs` holds the model and the pure parts; `DevelopState`
the cache; `export::batch` takes the model as a closure.

**Then made four times faster, at half resolution** (2026-09-28). 38 s on an
RTX 5070 Ti was the *good* case: the GPU is at 99% the whole run (full
clock, 261 W), and the model does 75 GMAC per 256 px tile, much of it
memory-bound attention (reshapes, transposes, shifted windows) at about 5%
of the card's arithmetic peak. Scaled by memory bandwidth that is minutes on
a mid-range card and five to ten on integrated graphics. What did *not*
help, measured:

- tile size (256–512 px) and batching (4 tiles per run): flat at 0.9 Mpx/s;
  768 px and up crash DirectML;
- folding the graph's 56 runtime `Size` nodes into constants: the whole graph
  then stays on DirectML (it had 70 tiny CPU nodes), but no faster;
  onnxsim's full simplification is rejected by DirectML at load;
- fp16 (onnxconverter-common, fp32 in and out): 30% faster, 0.64 of an 8-bit
  level from fp32 at worst on DirectML. Worth having, but it needs a hosted,
  modified model file, so it is not done yet.

What did: the model runs on a 2×2 box-averaged copy (a quarter of the
pixels) and its result, scaled back up, supplies all of the colour and the
luminance coarser than two pixels; the finest luminance band stays the
original's (`denoise::denoise_half_with`). On the ISO 6400 frame at 1:1 this
was cleaner in colour than full resolution at 60% (whose 40% of original
brought back faint colour mottling), with no bleeding at edges, and left a
fine, even grain that the Luminance noise slider takes down: 9.8 s against
38.4 s. Quarter resolution (2.8 s) left coarser, blotchy grain.

The panel now says how long it takes on the machine it runs on: during a run
from the pace so far, and before one from the last run's seconds per
megapixel (`src/lib/denoiseTime.ts`).

**A lighter model: NAFNet, spiked 2026-09-28.** SCUNet is heavy for ordinary
hardware, so the question was whether a convolution-only model, which
DirectML runs far more efficiently than attention, could do the same job for
less. The candidate: NAFNet-SIDD-width64 (Chen et al. 2022; MIT, from
megvii-research/NAFNet), as Qualcomm AI Hub's ONNX export
(`qualcomm/NAFNet-DeNoise` on Hugging Face, MIT; the float ONNX is
`nafnet_denoise-onnx-float.zip` from their v0.63.0 release, 431 MB zipped,
sha256 `ddf14ff6…2fd22e1214`). Static 1×3×256×256, input `image`, output
`denoised_image`, opset 21, 116M parameters, and all of it runs on DirectML.

- **Speed:** 12.5 ms per 256 px tile against SCUNet's 73 ms, 5.8× faster.
  Through the app's own pipeline on 24 MP: 6.8 s at full resolution, **2.0 s
  at half resolution** against 9.9–10.2 s for the shipped SCUNet at half, on
  all six high-ISO frames tried.
- **At full resolution it invents colour.** On the a6400 at ISO 6400 it is
  the smoothest of all, but stone and hair take on bluish-grey, magenta and
  green patches: a watercolour look, SIDD's phone-sensor training showing on
  a camera it was not trained on. Not usable that way.
- **At half resolution (the shipped recombination) it matches SCUNet.** On
  six frames at ISO 3200–6400 (door, stone, skin, carved reliefs, a painted
  shield, deep shadows) the two are hard to tell apart at 1:1: the same fine
  grain, clean edges, no colour bleeding. NAFNet keeps slightly more carved
  detail and leaves slightly more colour speckle (neighbour colour
  difference 1.5–1.9 against SCUNet's 1.4–1.7, from 10–14.5 untouched); one
  faint purple streak in hair that SCUNet did not have.
- **Limits of the evidence:** six frames, one camera (Sony a6400). The
  library has no other high-ISO bodies.

**Switched** (the owner: "whatever is best for the most users; UX is key").
Timed per 256 px tile on the three kinds of hardware this machine has:

| | SCUNet | NAFNet | 24 MP at half resolution |
|---|---|---|---|
| RTX 5070 Ti | 73 ms | 12.5 ms | 9 s → 2 s |
| Integrated Radeon (2 CUs, weaker than most laptops') | 3,475 ms | 855 ms | 7 min → 1.8 min |
| CPU, Ryzen 7800X3D, 8 cores | 1,631 ms | 611 ms | 3.4 min → 1.3 min |

Every user feels the speed every time; the download is paid once, and at
431 MB it is in line with the search model's 785 MB. The first-run Amount
went from 60% to 100%:
the half-resolution method keeps the fine grain itself, and at 60% 40% of
the colour speckle came back.

**Then fp16, hosted by us** (the owner: "we can host the model if it is
better"). Converted with onnxconverter-common (fp32 in and out) from the
Qualcomm export, published as release `nafnet-sidd-width64-fp16-v1` of
W4x1e/models with NAFNet's licence and a note of the change:

| Per 256 px tile | fp32 | fp16 |
|---|---|---|
| RTX 5070 Ti | 13.3 ms | 7.1 ms |
| Integrated Radeon | 841 ms | 625 ms |
| CPU only | 564 ms | 1,620 ms |
| Download | 431 MB zip (464 MB unpacked) | 234 MB, one file |

Its output is within 0.29 of an 8-bit level of fp32 on both GPUs. Better for
nearly everyone: every Windows PC with DirectX 12 graphics runs it on the
GPU, and it halves the download and removes the dependence on Qualcomm's zip
(and the zip support it needed). A PC with no usable GPU at all, the rare
fallback, pays about 3.4 min for a 24 MP photo instead of 1.2.

Two things the table shows, not acted on yet:
- on this weak integrated GPU the CPU is faster than the GPU for both
  models (typical laptop graphics are several times stronger, so not
  generally);
- DirectML is given adapter 0, and on a laptop with integrated and discrete
  graphics that may be the integrated one. Untested here (a desktop).

A smaller NAFNet-SIDD-width32 exists (about a quarter of the parameters) but
only as LiteRT (`litert-community/NAFNet-SIDD-width32-LiteRT`), which would
need converting and measuring.

Not done in the spike: raw-domain denoise (before demosaic), which is what
established raw developers ship and which keeps more detail. No permissive
pretrained raw-domain model was found, so sRGB-domain with a blend is the
practical start.

## Highlights — SigLIP 2 zero-shot "good photo" score, plus variety

Spiked 2026-09-28 on the owner's library (1,250 photos). No new model: the
stored SigLIP 2 image embeddings are scored against six "good photograph"
prompts less six "poor photograph" prompts (`search::GOOD_PROMPTS` /
`BAD_PROMPTS`), with the app's own text encoder.

**Labels are too few to measure it properly.** The library has 10 rated
photos, 11 picks and 9 rejects:

- picks vs rejects: AUC 0.70;
- 4–5 stars (5 photos) vs the unrated rest: AUC 0.25;
- no correlation with sharpness (0.06), so it is not a focus score in disguise.

**Looked at, it ranks credibly.** The top 32 are the postcards: a turquoise
lake with a rowing boat, mountain portraits, a harbour. The bottom 32 are
grey-sky snapshots, a bench, a grimacing selfie, clutter. It leans to scenic
and colourful rather than to craft, which is what "the best of a trip"
wants and not what a portfolio edit wants.

**Variety matters more than the score.** On the busiest day (325 photos) the
best 24 by score alone held 35 near-duplicate pairs (cosine > 0.92): the
same couple pose again and again. With maximal marginal relevance (λ 0.7,
`search::pick_diverse`) it held 9, and read as the day: lakes, the hut, the
flag, the valley, and the people once or twice.

**Go**, as a proposal the user edits rather than a verdict:

- Rejects, blinks and soft frames are dropped first (`highlightCandidates`).
- Rank by the score, pick with MMR.
- Everyone *named* in the set gets a photo (`search::cover_people`); strangers
  don't force their way in.
- The result is a collection.

A trained head over the embeddings, fitted to licensed aesthetic ratings,
could beat the prompts; it needs a dataset licensed for commercial use, which
hasn't been found.

## The vision model without Ollama

Measured 2026-09-28 and 29 on three kinds of hardware: the RTX 5070 Ti, the
7800X3D's 2-CU Radeon, and its CPU. The tool was
`scripts/model-eval/vlm_serving.py`, which sends what the app sends:
- the prompts and JSON schema are read from `tagger/mod.rs`;
- images are shrunk as `prepare_image` does it;
- tags are asked of a 640 px image and descriptions of a 1280 px one, as
  `tagger/llama.rs` does.

**How it was scored.** Speed is seconds per photo over 24–32 previews, after
two warm-up photos. Quality was judged two ways:
- genre accuracy on the 71 labelled photos whose genre is clear-cut (the
  `qwen_tag.py` set);
- descriptions of 194 photos, read side by side.

### Ollama is the slow part

Qwen3-VL 4B through Ollama's API, as Windows runs it today, against the same
model on Ollama's own `llama-server` started by the app, as the Mac runs it
(CUDA):

| Per photo | Ollama's API | Its `llama-server`, 2 at once | 4 at once |
|---|---|---|---|
| Tags | 0.71 s (896 px) | 0.32 s | 0.26 s |
| A description | 0.74 s | 0.50 s | 0.48 s |
| Tags and a description | 1.07 s | 0.73 s | 0.75 s |

On a CPU the gap is wider. With the 2B, tags take 28.3 s through Ollama and
4.8 s direct. That is the case for a laptop with only integrated graphics,
because Ollama drops integrated GPUs and falls back to the CPU.

**Why Ollama is slower:**
- **Image cost.** It starts Qwen3-VL with `--image-min-tokens 1024`, so a
  tag image costs ~1,400 tokens instead of ~260. On the CPU, at equal thread
  counts, that alone makes it 4.4× slower.
- **One request at a time.** It runs Qwen3-VL serially
  (ollama/ollama#12849).
- **Half the CPU.** Its CPU backend uses 4 of 8 cores.
- **No grammar.** Its API takes a JSON schema, not a grammar.

**Other problems found along the way:**
- **It phones home.** The managed Ollama calls ollama.com at start and every
  ~4 h. `OLLAMA_NO_CLOUD=1` stops it.
- **It leaks the model on Windows.** Killing `ollama serve` there leaves its
  model runner holding the model (~3 GB of VRAM) indefinitely.
- **The download is large.** The Windows runtime is 1.46 GB, mostly CUDA 12
  and 13 libraries, yet Vulkan was as fast as CUDA on the RTX.

### The official llama.cpp build and Qwen's own files

The runtime and model:
- **Runtime:** llama.cpp b11234's Windows Vulkan build (MIT). The files the
  app needs, including the CPU backends, are 31 MB to download and 89 MB
  unpacked.
- **Model:** `Qwen/Qwen3-VL-4B-Instruct-GGUF` at `1cd86af` (Apache-2.0).
  That is Q4_K_M (2.50 GB) plus the vision projector, either F16 (0.84 GB)
  or Q8_0 (0.45 GB).

| Per photo, RTX 5070 Ti via Vulkan | Tags | A description | Both |
|---|---|---|---|
| 2 at once | 0.30 s | 0.52 s | 0.69 s |
| 4 at once | 0.23 s | 0.45 s | 0.59 s |
| 2 at once, Q8_0 projector | 0.29 s | 0.50 s | |

- **On the RTX:** as fast as Ollama's build, and a little faster at 4 at
  once. Genre accuracy was 0.90 (64/71), with all 71 replies valid JSON.
- **The Q8_0 projector** scored exactly the same (0.90), at the same speed,
  for half the download. Use it.
- **Load and first request:** a load takes 3–5 s. The first request after
  the very first load paid ~16–20 s of one-off Vulkan compilation, which the
  driver then caches.
- **Device choice:** with no `--device` it took the RTX and left the
  integrated Radeon alone. The app should still name the GPU its other models use,
  and not by reported memory: the Radeon reports 16,442 MiB of shared
  memory, more than the RTX's 15,995.
- **On the CPU:** the same as Ollama's build (2B tags 4.7 s, a description
  17.3 s), and it uses all 8 cores by default.
- **On the integrated Radeon it is slower than Ollama's build,** with
  identical files:
  - 2B tags take 10.7 s against 5.8 s;
  - reading the image takes 6.1 s against 2.0 s, while decoding is the same;
  - turning flash attention off recovers a third of it (4.1 s).
  This 2-CU Radeon loses to the CPU on either build, so the CPU is the answer
  here. A real laptop iGPU (Radeon 780M, Intel Arc) must be measured before
  a build is pinned.

**The JSON grammar.** llama-server's grammar from the schema lets the model
pretty-print, and it does. A tag reply is 62 tokens, 11 of them newlines and
indents.
- **One line.** Allowing at most one space (`one_line_grammar`) keeps the
  JSON on one line: 52 tokens, and tags drop from 0.30 to 0.26 s.
- **No whitespace breaks the model.** It degenerates into `":",":",…`.
- **Capped lists.** Capping each list at 8 (the prompt asks for 3–8) stops a
  small model stuck on one word ("rock", "rock", … until the token limit,
  which leaves invalid JSON). With the cap, every model returned 71 of 71
  valid replies.
- **For the 4B, accuracy holds.** Genre with the capped one-line grammar
  was 0.92 (65/71). Every variant moved within ±4 photos of the others.
- **For the 2B, one line costs tags.** Rerun on the 71 labelled photos
  (`--grammar capped-pretty` is the cap alone):

  | Qwen3-VL 2B, tags | Genre | Subjects a photo | Two or fewer | Tokens |
  |---|---|---|---|---|
  | llama-server's grammar | 0.89 | 5.9 | 7 of 71 | 74 |
  | the cap alone | 0.89 | 5.9 | 7 of 71 | 74 |
  | the cap and one line | 0.83 | 5.1 | 18 of 71 | 54 |

  On the M4 one line saved nothing either (tags 2.33 s against 2.30). So
  the app takes **the cap alone**: it ends runaway lists at no measurable
  cost, where one line trades the small model's tags for 13% on a
  discrete GPU.

**Descriptions need Ollama's repetition penalty.** Ollama applies
`repeat_penalty` 1.1 by default; llama-server applies none, and neither does
`tagger/llama.rs`. Without it, Qwen3-VL 2B looped on 9 of the 194
descriptions: the same sentences over and over, 400+ words to the token
limit. On those nine (descriptions sample at temperature 0.2, so a rerun
differs):

| Sampling | Looped | Mean words | Longest | Per photo |
|---|---|---|---|---|
| none, as the Mac sends today | 9 of 9, then 1 on a rerun | 141 | 469 | 0.46 s |
| `repeat_penalty` 1.1, last 64 tokens | 0 of 9 | 81 | 102 | 0.38 s |
| DRY 0.8 | 3 of 9 | 208 | 459 | 0.70 s |

The 2B is the pick for GPUs under 6 GB, and for 8 GB Macs.

### Qwen3.5's small models

The models were Unsloth's GGUFs of Qwen3.5 2B and 4B (Apache-2.0, released
2026-03-02; Qwen publishes none). They ran on the same build, with thinking
off:

| | Qwen3-VL 4B | Qwen3.5 4B | Qwen3-VL 2B | Qwen3.5 2B |
|---|---|---|---|---|
| Tags, RTX, 2 at once | 0.30 s | 0.52–0.81 s | 0.22 s | 0.33–0.35 s |
| A description | 0.52 s | 0.59 s | 0.54 s | 0.36 s |
| Genre, tags (71) | 0.90 | 0.89 | 0.89 | 0.92 |
| Words per description | 57 | 64 | 115 | 68 |
| Tags on the CPU (8 threads) | 7.2 s | | 4.7 s | 4.9 s |

- **Speed:** Qwen3.5 is slower where it matters. Its tag replies run longer
  (~93 tokens against ~62). Its hybrid attention also lets llama-server reuse
  only ~110 cached prompt tokens between photos, against ~340 for Qwen3-VL.
- **Genre accuracy:** it is no better.
- **Descriptions,** read side by side on 194 photos:
  - The two 4Bs are close.
  - Qwen3.5 4B gives slightly richer detail, but it reads text less well.
    "The Ship Inn" is painted large on a pub's wall: Qwen3-VL 4B read it,
    Qwen3.5 4B wrote "The White Swan", and both 2Bs invented other names.
  - A small, soft street sign ("Port… West Co…", "Beach Information")
    defeated both 4Bs. They guessed the place: Qwen3-VL wrote "Porthcurno",
    Qwen3.5 "Pembrokeshire Coast Path".
  - Qwen3-VL 2B is wordy (115 words against 57) and recites the prompt's
    checklist. Without a repetition penalty it also looped on 9 of 194.
  - Qwen3.5 2B ran past one paragraph on 17 of 194, 4 of them markdown lists.
    It looped on 2, once repeating "The Rock" until the token limit.

**Keep Qwen3-VL.** Revisit Qwen3.5 when llama.cpp caches hybrid models
better. Its 9B, for GPUs with 12 GB or more, was not measured.

### Recommendation

Replace Ollama with the official llama.cpp `llama-server`, run by the app on
every platform. Download Qwen's official GGUFs with the app's own model
downloader:

1. **Runtime:** pin a llama.cpp build and its SHA-256, as `ollama_pack` pins
   Ollama. Ship it in the installer or fetch it on first use.
2. **Model files:** pin Qwen's GGUF and the Q8_0 projector per tier in
   `ai_models.rs` (4B: 2.50 + 0.45 GB).
3. **Server settings:**
   - in `sidecar::llama_server_args`: `--device` from the other models' adapter, and
     2 slots (4 from 12 GB);
   - in `tagger/llama.rs`: the list cap in the grammar, and
     `repeat_penalty` 1.1 on descriptions;
   - on a weak integrated GPU, the CPU, until a laptop iGPU is measured.
4. **Processes:** one process instead of two. On Windows, put it in a job
   object so it dies with the app.
5. **Delete** `ollama_pack.rs`, `tagger/ollama.rs`, the Ollama half of
   `sidecar.rs`, and the Ollama store. There are no users yet, so nothing
   needs migrating.

**The Mac half** is in `docs/MACOS.md`. On the M4 the official build
matches Ollama's within ~2%, MLX is no faster, and Apple's on-device model
is fast but not good enough to replace Qwen3-VL. So the plan holds on both
platforms, with one engine.

**Still to do before shipping:** a real laptop iGPU on the pinned build.

## Reading the text in photos — PP-OCR mobile

Measured 2026-09-29 on 200 photos from the owner's library (1,252 Sony a6400
raws), read from the app's own 1600 px previews. GPU: RTX 5070 Ti. CPU: Ryzen 7
7800X3D (8 cores). The harness is `scripts/model-eval/ocr_*`.

**The short version: go.**

- **Recall.** At the preview's full size, 30 of the 35 photos with legible
  text can be found by at least one of their words (86%), and 63% of all
  labelled words are read.
- **False positives.** Nothing was read on any of the 151 photos without
  text, once lines under 0.9 confidence are dropped.
- **Size.** The two models are 14 MB.
- **Speed.** About 50 ms of model time per photo: detection on DirectML, recognition on the CPU.
- **Three conditions.** It needs all three, or the numbers above don't hold:
  - Run detection at the preview's full size. PaddleOCR's default of 960 px finds half as many photos.
  - Index the text for substring search. The English recogniser often drops the spaces between words.
  - Use a 0.9 confidence cut-off, not PaddleOCR's 0.5.

### How it was measured

- **Sample.** Every photo whose keywords named a sign, poster, shopfront or similar (28), plus 172 drawn at random (seed 242).
- **Ground truth came before any model output.** It was taken from contact sheets, then from every candidate at full size.
  - **Words** are text legible at 1600 px that someone could search for. Recall is measured on these: 86 words on 35 photos.
  - **Maybe** is small, partial, cursive or curved text. Reading it is not a hit, missing it is not a miss, and it is never a false positive.
  - Photos listed as neither have no text.
- **Adjudication after the run.** Three "false positives" turned out to be real text missed when labelling: a jacket's logo, an outboard motor's brand, and a boat's number far across a harbour. They were moved to *maybe*. The truth file says which.
- **Scoring works the way search would match.** Text is lower-cased and accents dropped. Matching is either exact tokens, or substrings of three or more characters (a trigram index).
- **The random part of the sample.** 12 of its 172 photos (7%) carry searchable words, and 23 (13%) carry any text.
- **Models.** Both are Apache-2.0 (PaddleOCR), ONNX conversions from `SWHL/RapidOCR` at revision `1cfba2e9`:
  - `PP-OCRv4/ch_PP-OCRv4_det_infer.onnx`: 4.7 MB, sha256 `d2a7720d…`.
  - `PP-OCRv3/en_PP-OCRv3_rec_infer.onnx`: 9.0 MB, sha256 `ef7abd8b…`.
  - The recogniser carries its 95-character list in its own metadata.
  - The text-angle classifier was not used.
- **Pipeline.** PaddleOCR's own pre- and post-processing:
  - Detection: DB threshold 0.3, box threshold 0.6, unclip 1.5.
  - Recognition: lines 48 px high, in batches of six, with CTC decoding.
  - Vertical lines are read both ways round, and weak lines are also read upside down; the more confident reading wins.

### Results

| Detector input | Words read at ≥ 0.9 (exact / substring) | Photos findable at ≥ 0.9 | Text-free photos with a word read, at ≥ 0.5 / ≥ 0.9 |
|---|---|---|---|
| 960 px (PaddleOCR's default) | 34% / 37% | 18 / 35 | 5 / 0 of 151 |
| 1280 px | 37% / 50% | 21 / 35 | 7 / 0 |
| **1600 px (the full preview)** | **49% / 63%** | **30 / 35** | **7 / 0** |

At 1600 px, raising the cut-off from 0.5 to 0.9 costs one substring hit and removes almost all the junk: 112 stray tokens on text photos fall to 13. What survives at 0.9 is nearly all real text read partly: "rbour", "vistoria", "errex", "collateralevent".

**What it reads.** Signs and shopfronts in normal lettering are read cleanly: "Pay & Display", "BEACH", "HARBOUR INN", "GIOIELLERIA S. MARCO", "KANTOR", "BÖDENALM", "TOFANA" on a cable car, "BURNT STUDIOS" on a T-shirt, and "adidas". The vertical "COLLATERAL EVENT OF BIENNALE ARTE 2026" is read too, once it is tried both ways round.

**What it misses.**

- **Small sign text:**
  - The "Beach Information" board, in both frames.
  - "Bibliothek / Biblioteca", read as "sibtioteci".
  - A shop's "GUESS".
  - The small line over "KANTOR" ("TADEUSZ … EMBALLAGE, CRICOTAGE").
- **Stylised or rotated lettering:**
  - "PREMIATA" on a cushion, read as "premaita".
  - An ornate "Garden Bar" sign, read in one frame of two.
- **Upside-down text:** a cup lying upside down. That needs the angle classifier, a further small model that wasn't tested.
- **Paragraphs:** the small body text of a drone-warning notice comes out as noise at 0.5, and at 0.9 it drops out entirely.

### Cost

Model time per photo at 1600 px (Rust, through `commit_on_gpu_or_cpu`, with the recorded input shapes):

| | Detect | Recognise (all lines) | Both |
|---|---|---|---|
| DirectML | 23 ms (median 13) | 34 ms | 57 ms |
| CPU, 4 threads | 136–170 ms | 24 ms | ~190 ms |

- **Recognition belongs on the CPU.** On DirectML it is *slower* than on the CPU, because every batch has a new width and DirectML re-plans for each new shape. Either run recognition on the CPU, or pad widths to a few fixed sizes.
- **Cap the thread count.** On the CPU, ONNX Runtime's default of 8 threads was 2.5× slower than 4 while the rest of the machine was busy. With the CPU at 100% from other work, even 4 threads slowed from 136 to 450 ms.
- **Around the models:**
  - Decoding the preview takes 7–12 ms.
  - Resizing, contours and crops take 30–45 ms in Python with OpenCV.
  - Loading the sessions takes 0.4–0.6 s on DirectML and 0.14 s on the CPU.
- **For a whole library.** At 50,000 photos this is roughly an hour and a half of background work with a GPU and three hours without. It can resume, like the colour backfill.

### Things that will bite

- **Spaces.** The v3 English recogniser often joins words: "mizzilachat", "ofbiennalearte2026", "pz18". Word-token search misses 12 of the 54 words that substring search finds, so index the text with SQLite's trigram tokenizer, not the caption index's.
- **Accents.** The English character list has no accented letters. "BÖDENALM" reads as "BODENALM", which is fine once search folds accents. For French, German, Italian and Spanish text, measure the Latin recogniser (`latin_PP-OCRv3`) or PP-OCRv5 with this harness before shipping; neither was measured here.
- **Detector size.** At 960 px the detector simply doesn't see text a few pixels high, and that is most sign text in a travel photo.
- **Confidence.** At 0.5, words turn up on 7 photos without text:
  - five are invented outright: "had" on a roof, "wns" on a flower, "17" on a coat of arms;
  - two misread tiny real text: a house-number plate, and a hut's painted name.

  At 0.9 there are none, and no labelled word is lost at 1600 px.

## Magic erase — LaMa over MI-GAN

Spiked 2026-10-02 on an M4 Mac mini (ONNX Runtime 1.24.4) with five Sony a6400 raws (6024×4024): a child on a beach, a flag on its pole against rock, two hikers on a path, a rowing boat on a lake and a snowcat on gravel. Each object was selected with SAM 2 clicks, and the gap was filled in the linear image through `denoise::encode`/`decode`, laid back in and exported at full size through the app's own pipeline.

| | LaMa (Big LaMa, fp32) | MI-GAN (pipeline v2) |
|---|---|---|
| Licence | Apache-2.0 | MIT |
| Download | 208 MB | 28 MB |
| One 512² pass, M4 CPU | 2.2–2.8 s | 0.22–0.30 s |
| Session load, CPU | 1.8 s | 0.04 s |
| CoreML | Fails to compile (error −14); 7 s wasted before the CPU fallback | Fails: "Required param 'pad' is missing", as the face mesh does |
| Pixels outside the hole | Returned unchanged | Shifted 0.2% |
| A person on sand, at 100% | Clean: tyre tracks carried through | A visible smudge |
| A 4,300 px vehicle | A dark smear where its shadow was | Paints a fake truck |

**LaMa**, run on the CPU on a Mac, and since 2026-10-11 on Windows too: DirectML accepts the graph and then fails its first pass at a MatMul in the Fourier units, on an RTX 5070 Ti and integrated Radeon graphics alike, so erase never worked on a Windows PC with a GPU ([BENCHMARKS.md](BENCHMARKS.md#slower-machines-2026-10-11)). The rest of what the spike settled is in `develop/erase.rs`:

- **A window around the object, squeezed into 512 px.** Up to a squeeze of two sensor pixels per model pixel a fill is clean at 100%. Past four it is soft, and the tool says so.
- **Tiles for long objects.** A flag pole squeezed into one window left a smeared strip. Tiles of 1,024 px work, but only if each masks *all* of the object it can see: with only its own share masked, LaMa drew the pole back from the pieces above and below. For a wide, thick object (a boat's hull, a standing person), tiles of 1,536 px or more still squeeze less than one window. In the build, two hikers went at f = 3 rather than 6, and the hull at 3 rather than 12.
- **Matched grain**, measured from the window's untouched pixels and mostly shared between channels. Without it a fill looks pasted on; with each channel's grain drawn on its own, a fill speckled with colour.
- **Reach past the selection.** Growing the mask by about 18 px at 24 MP (three SAM texels) took the soft edges, a shoe, a hand. Two pixels left them.
- **The selection is the weak point, not the inpainter.** Clicks across two people gave half of one. A single object still missed a phone in a hand, a camera on a tripod, reflections and shadows. Erasing one object at a time, with the mask shown before it runs and a brush to add to it, is the remedy.

**Built** in `develop/erase.rs`:

- **The fill is baked into the linear image in sensor space**, through the GPU region writes added for it, so clarity, noise reduction and every other neighbourhood filter treats it like the photo, and a later crop or turn leaves it in place.
- **Fills are 16-bit PNGs named by their SHA-256** in `<app data>/patches`. The edit stack holds only the names and the windows, so history and virtual copies stay small.
- **Only changed windows are laid in again**, over the photo as it was before any erase (or the denoise blend). Undo, redo and the denoise Amount cost milliseconds.
