//! Built-in local tagger: zero-shot image tagging with a CLIP-family model
//! (SigLIP 2; CLIP ViT-B/32 before),
//! run **in-process via ONNX Runtime** — no server, no API key, nothing to
//! install beyond one model file. The zero-setup option for people who don't
//! want Ollama or a cloud key.
//!
//! How it differs from the VLM providers: a chatty vision-language model writes
//! free-form answers; CLIP instead *scores an image against a fixed vocabulary*.
//! So we hand it the genre list plus a large subject vocabulary and keep the
//! best matches. That is exactly the shape the library's facets need (a closed
//! genre set, a handful of subject nouns), and it is fast and fully local. It
//! cannot caption or reason like a VLM — that is what the Ollama provider is for.
//!
//! The subject vocabulary lives in `resources/tagger/subjects.txt`: ~1,600
//! photo-oriented labels arranged in a hierarchy (golden retriever → dog → pet →
//! animal). CLIP can only ever answer with a word it was offered, so a short
//! list meant anything outside it got the nearest *wrong* word; the hierarchy
//! lets a confident specific match also tag its general parents, and lets a
//! photo the model is unsure about settle on the parent instead of guessing.
//!
//! Two halves, mirroring `develop/ai.rs`:
//! - **Pure logic** (vocabulary, prompt templates, cosine/softmax/selection,
//!   CLIP image preprocessing, the label-embedding cache format) — always
//!   compiled and unit-tested; no model needed.
//! - **The ONNX session + tokenizer** (`BuiltinTagger`, behind the `ai` feature)
//!   — loads the CLIP vision + text encoders and a tokenizer, embeds the label
//!   vocabulary once (then caches it on disk), and matches each image against it.

use std::sync::OnceLock;

use crate::models::{TagResult, GENRES};

/// The folder under the models directory the image/text model lives in. The
/// provider's model id stays `"clip"` in settings; only the files moved.
pub const MODEL_DIR: &str = "siglip2";

/// The subject vocabulary file, compiled in so there is no runtime path to get
/// wrong and the tests read exactly what ships.
pub const SUBJECTS_TXT: &str = include_str!("../../resources/tagger/subjects.txt");

/// One entry of the subject vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub name: String,
    /// Index of the parent label, if any.
    pub parent: Option<usize>,
    /// A *group* organises the hierarchy but is never scored or emitted as a
    /// tag — see the header of `subjects.txt`.
    pub group: bool,
}

/// Parses the `subjects.txt` format: one label per line, optionally
/// `label > parent`, `[name]` for a group, `#` comments and blank lines
/// ignored. Fails on duplicates, unknown parents and cycles, so a typo in the
/// file is a test failure rather than a silently flat hierarchy.
pub fn parse_labels(text: &str) -> Result<Vec<Label>, String> {
    let mut raw: Vec<(String, Option<String>, bool)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, parent) = match line.split_once('>') {
            Some((a, b)) => (a.trim(), Some(b.trim().to_string())),
            None => (line, None),
        };
        let (name, group) = match name.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            Some(inner) => (inner.trim(), true),
            None => (name, false),
        };
        if name.is_empty() || parent.as_deref() == Some("") {
            return Err(format!("line {}: malformed entry '{line}'", n + 1));
        }
        raw.push((name.to_string(), parent, group));
    }

    let mut index = std::collections::HashMap::new();
    for (i, (name, _, _)) in raw.iter().enumerate() {
        if index.insert(name.clone(), i).is_some() {
            return Err(format!("duplicate label '{name}'"));
        }
    }
    let labels: Vec<Label> = raw
        .iter()
        .map(|(name, parent, group)| {
            let parent = match parent {
                Some(p) => Some(
                    *index
                        .get(p)
                        .ok_or_else(|| format!("'{name}' has unknown parent '{p}'"))?,
                ),
                None => None,
            };
            Ok(Label {
                name: name.clone(),
                parent,
                group: *group,
            })
        })
        .collect::<Result<_, String>>()?;

    // A cycle would make `ancestors` loop forever.
    for (i, l) in labels.iter().enumerate() {
        let mut cur = l.parent;
        let mut steps = 0;
        while let Some(p) = cur {
            steps += 1;
            if p == i || steps > labels.len() {
                return Err(format!("'{}' is its own ancestor", l.name));
            }
            cur = labels[p].parent;
        }
    }
    Ok(labels)
}

/// The shipped subject vocabulary.
pub fn subjects() -> &'static [Label] {
    static LABELS: OnceLock<Vec<Label>> = OnceLock::new();
    LABELS.get_or_init(|| parse_labels(SUBJECTS_TXT).expect("subjects.txt is validated by tests"))
}

/// Every taggable (non-group) subject name, in file order.
pub fn subject_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        subjects()
            .iter()
            .filter(|l| !l.group)
            .map(|l| l.name.as_str())
            .collect()
    })
}

/// Candidate lighting/weather conditions. Kept short — only the ones a model can
/// read reliably from a single frame.
pub const CONDITIONS: &[&str] = &[
    "golden hour",
    "overcast",
    "backlit",
    "night",
    "sunny",
    "foggy",
    "rainy",
    "indoor lighting",
    "high contrast",
    "soft light",
];

/// A natural-language prompt for a genre — better zero-shot than the bare label.
pub fn genre_prompt(genre: &str) -> String {
    let phrase = match genre {
        "landscape" => "a landscape photograph of scenery",
        "wildlife" => "a wildlife photograph of an animal",
        "portrait" => "a portrait photograph of a person",
        "street" => "a candid street photograph",
        "macro" => "an extreme close-up macro photograph",
        "astro" => "an astrophotography photo of the night sky",
        "architecture" => "an architectural photograph of a building",
        "sports" => "a sports action photograph",
        "event" => "a photograph of a gathering or celebration",
        "still-life" => "a still life photograph of objects",
        "abstract" => "an abstract photograph of pattern and texture",
        other => return format!("a {other} photograph"),
    };
    phrase.to_string()
}

/// Phrasings each subject is embedded under; their embeddings are averaged.
///
/// Prompt ensembling is the standard CLIP zero-shot trick, and it also papers
/// over grammar: "a photo of a snow" reads oddly, but averaged with "a
/// photograph showing snow" the label still lands where it should.
pub const SUBJECT_TEMPLATES: &[&str] = &[
    "a photo of a {}.",
    "a photo of the {}.",
    "a close-up photo of {}.",
    "a photograph showing {}.",
];

pub fn subject_prompts(noun: &str) -> Vec<String> {
    SUBJECT_TEMPLATES
        .iter()
        .map(|t| t.replace("{}", noun))
        .collect()
}
pub fn condition_prompt(cond: &str) -> String {
    format!("a photo taken in {cond}")
}

/// Every label the model scores, in a fixed order — genres, then subjects, then
/// conditions — each with the prompts whose averaged embedding represents it.
/// The three ranges below index into this list (and into the similarity vector
/// built from it).
pub fn label_prompts() -> Vec<Vec<String>> {
    GENRES
        .iter()
        .map(|g| vec![genre_prompt(g)])
        .chain(subjects().iter().map(|s| subject_prompts(&s.name)))
        .chain(CONDITIONS.iter().map(|c| vec![condition_prompt(c)]))
        .collect()
}

/// Where each vocabulary sits within [`label_prompts`].
pub fn genre_range() -> std::ops::Range<usize> {
    0..GENRES.len()
}
pub fn subject_range() -> std::ops::Range<usize> {
    GENRES.len()..GENRES.len() + subjects().len()
}
pub fn condition_range() -> std::ops::Range<usize> {
    let start = GENRES.len() + subjects().len();
    start..start + CONDITIONS.len()
}

/// L2-normalises a vector in place (so cosine similarity is a plain dot product).
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
    for x in v.iter_mut() {
        *x /= norm;
    }
}

/// Dot product — cosine similarity for already-normalised vectors.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    crate::vector::dot(a, b)
}

/// The normalised mean of several normalised embeddings — one label's vector
/// from its ensemble of prompts.
pub fn mean_embedding(embs: &[Vec<f32>]) -> Vec<f32> {
    let dim = embs.first().map_or(0, |e| e.len());
    let mut out = vec![0f32; dim];
    for e in embs {
        for (o, x) in out.iter_mut().zip(e) {
            *o += x;
        }
    }
    l2_normalize(&mut out);
    out
}

/// Softmax with a temperature (a logit scale ~100 makes the winner clear).
/// `-inf` entries get exactly zero probability, which is how groups are left out.
pub fn softmax(scores: &[f32], temperature: f32) -> Vec<f32> {
    if scores.is_empty() {
        return Vec::new();
    }
    let max = scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        return vec![0.0; scores.len()];
    }
    let exps: Vec<f32> = scores
        .iter()
        .map(|s| ((s - max) * temperature).exp())
        .collect();
    let sum = exps.iter().sum::<f32>().max(1e-8);
    exps.iter().map(|e| e / sum).collect()
}

/// Cosine similarities are multiplied by this before the softmax, for genre and
/// conditions. At 100 SigLIP 2's genre confidence spreads like CLIP's did
/// (median 0.86 against 0.89 on a real library).
const LOGIT_SCALE: f32 = 100.0;
/// The same for subjects, measured separately: at 160 SigLIP 2 tags a
/// photo's clear subject on 66% of labelled photos (CLIP at 100: 58%) at the
/// same ~3.7 tags per photo. Higher saturates without tagging more.
const SUBJECT_LOGIT_SCALE: f32 = 160.0;
/// A subject is claimed once it (with its descendants) holds this much of the
/// probability mass. Tuned by eye with `tests/tag_probe.rs` on a real library:
/// 0.08 dropped the people from scenes a place dominated, 0.04 let in stray
/// objects ("battery" on a castle).
const SUBJECT_MIN_PROB: f32 = 0.06;
/// Probability below this is noise and is not pooled into parents. Without the
/// floor, a broad label with hundreds of descendants ("animal") collects a
/// sliver from each and gets claimed on photos with no animal in them.
const POOL_FLOOR: f32 = 0.01;
/// A parent gives way to its specific children only when they explain at
/// least this share of its pooled mass — otherwise the model was not sure
/// *which* dog, and "dog" is the honest answer.
const SPECIFIC_SHARE: f32 = 0.5;
/// At most this many subjects are claimed directly; their parents are added on
/// top, up to [`MAX_TAGS`] in all.
const MAX_SUBJECTS: usize = 4;
const MAX_TAGS: usize = 8;

/// Chooses subject labels from their cosine similarities (indexed as
/// [`subjects`]), most confident first, each followed later by its parents.
///
/// How it reads the scores:
/// 1. Softmax across the whole vocabulary. Raw similarities sit in a
///    narrow band whatever the image, so only their *relative* size carries
///    information — a fixed threshold on them admits everything or nothing.
/// 2. Pool each label's probability into its ancestors, so "dog" is as sure as
///    all the dog breeds together.
/// 3. Claim labels whose pooled probability clears the bar, preferring the most
///    specific: a parent is dropped when its claimed children carry most of it.
/// 4. Add the parents of what was claimed, so a golden retriever is also
///    findable under "dog".
pub fn select_subjects(sims: &[f32], labels: &[Label]) -> Vec<String> {
    if sims.is_empty() || labels.len() != sims.len() {
        return Vec::new();
    }
    let masked: Vec<f32> = sims
        .iter()
        .zip(labels)
        .map(|(s, l)| if l.group { f32::NEG_INFINITY } else { *s })
        .collect();
    let p = softmax(&masked, SUBJECT_LOGIT_SCALE);

    let mut pooled = p.clone();
    for (i, &pi) in p.iter().enumerate() {
        if pi < POOL_FLOOR {
            continue;
        }
        let mut cur = labels[i].parent;
        while let Some(a) = cur {
            pooled[a] += pi;
            cur = labels[a].parent;
        }
    }

    let is_ancestor = |a: usize, mut d: usize| {
        while let Some(p) = labels[d].parent {
            if p == a {
                return true;
            }
            d = p;
        }
        false
    };

    let candidates: Vec<usize> = (0..labels.len())
        .filter(|&i| !labels[i].group && pooled[i] >= SUBJECT_MIN_PROB)
        .collect();
    // A parent gives way to a single dominant child (golden retriever over
    // dog)...
    let kept: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&a| {
            let best_child = candidates
                .iter()
                .filter(|&&d| is_ancestor(a, d))
                .map(|&d| pooled[d])
                .fold(0.0, f32::max);
            best_child < SPECIFIC_SHARE * pooled[a]
        })
        .collect();
    // ...and when it stands, the children that failed to dominate it are
    // uncertain refinements of it, not extra findings: three breeds at 17%
    // each under a 100% "dog" is a dog of unknown breed.
    let mut claimed: Vec<usize> = kept
        .iter()
        .copied()
        .filter(|&d| {
            !kept
                .iter()
                .any(|&a| is_ancestor(a, d) && pooled[d] < SPECIFIC_SHARE * pooled[a])
        })
        .collect();

    // Nothing stands out: still name the single best guess rather than
    // leaving the photo with no subject at all.
    if claimed.is_empty() {
        if let Some((best, _)) = p
            .iter()
            .enumerate()
            .filter(|(i, _)| !labels[*i].group)
            .max_by(|a, b| a.1.total_cmp(b.1))
        {
            claimed.push(best);
        }
    }
    claimed.sort_by(|&a, &b| pooled[b].total_cmp(&pooled[a]));
    claimed.truncate(MAX_SUBJECTS);

    let mut out: Vec<usize> = claimed.clone();
    for &c in &claimed {
        let mut cur = labels[c].parent;
        while let Some(a) = cur {
            if !labels[a].group && !out.contains(&a) {
                out.push(a);
            }
            cur = labels[a].parent;
        }
    }
    out.truncate(MAX_TAGS);
    out.into_iter().map(|i| labels[i].name.clone()).collect()
}

/// Turns per-label cosine similarities into a [`TagResult`].
///
/// - **genre**: the single best of the genre group; `confidence` is its softmax
///   probability so an ambiguous frame reads as low-confidence.
/// - **subjects**: see [`select_subjects`]; `main_subject` is the top one.
/// - **conditions**: only those the model is fairly sure of.
///
/// `sims` are cosine similarities indexed exactly as [`label_prompts`].
pub fn assemble(sims: &[f32]) -> TagResult {
    let g = &sims[genre_range()];
    let (gi, _) = g
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap_or((0, &0.0));
    let genre = GENRES.get(gi).copied().unwrap_or("").to_string();
    let confidence = softmax(g, LOGIT_SCALE).get(gi).copied().unwrap_or(0.0) as f64;

    let subjects = select_subjects(&sims[subject_range()], subjects());
    let main_subject = subjects.first().cloned().unwrap_or_default();

    // Conditions: at most two, and only when one clearly wins its group. Most
    // photographs have one readable lighting condition or none at all, so this
    // is deliberately conservative — a wrong condition is worse than a missing
    // one, because it pollutes a facet people filter on.
    let c = &sims[condition_range()];
    let probs = softmax(c, LOGIT_SCALE);
    let mut cranked: Vec<(usize, f32)> = probs.iter().cloned().enumerate().collect();
    cranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let conditions: Vec<String> = cranked
        .iter()
        .take(2)
        .filter(|(_, p)| *p > 0.25)
        .map(|(i, _)| CONDITIONS[*i].to_string())
        .collect();

    TagResult {
        main_subject,
        genre,
        subjects,
        conditions,
        confidence,
    }
}

// ------------------------------------------------------------------------
// Label-embedding cache
// ------------------------------------------------------------------------

const CACHE_MAGIC: &[u8; 8] = b"NLLABEL1";

/// A fingerprint of everything the label embeddings depend on: every prompt
/// string, and the text encoder's identity (size + modified time — cheap, and
/// any re-download changes one of them). FNV-1a rather than `DefaultHasher`,
/// whose output is not promised to be stable between Rust releases.
pub fn cache_key(prompts: &[Vec<String>], model_identity: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(model_identity.as_bytes());
    for group in prompts {
        eat(b"\x1e");
        for p in group {
            eat(p.as_bytes());
            eat(b"\x1f");
        }
    }
    h
}

/// Serialises the label matrix: magic, key, count, dimension, then row-major
/// little-endian `f32`s.
pub fn encode_label_cache(key: u64, rows: &[Vec<f32>]) -> Vec<u8> {
    let dim = rows.first().map_or(0, |r| r.len());
    let mut out = Vec::with_capacity(24 + rows.len() * dim * 4);
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&key.to_le_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    out.extend_from_slice(&(dim as u32).to_le_bytes());
    for r in rows {
        for x in r {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    out
}

/// Reads a cache written by [`encode_label_cache`], or `None` when it is for a
/// different key, a different number of labels, or damaged in any way — in
/// which case the caller simply re-embeds.
pub fn decode_label_cache(bytes: &[u8], key: u64, count: usize) -> Option<Vec<Vec<f32>>> {
    let header = bytes.get(..24)?;
    if &header[..8] != CACHE_MAGIC {
        return None;
    }
    let got_key = u64::from_le_bytes(header[8..16].try_into().ok()?);
    let n = u32::from_le_bytes(header[16..20].try_into().ok()?) as usize;
    let dim = u32::from_le_bytes(header[20..24].try_into().ok()?) as usize;
    if got_key != key || n != count || dim == 0 || bytes.len() != 24 + n * dim * 4 {
        return None;
    }
    let floats: Vec<f32> = bytes[24..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Some(floats.chunks_exact(dim).map(|c| c.to_vec()).collect())
}

/// SigLIP image preprocessing: a plain resize to `size`×`size` (no crop, so
/// nothing at the edges of a wide frame is lost), normalised to [-1, 1] as
/// planar NCHW `f32`. Pure and tested; the model run is the only part that
/// needs the feature.
pub fn siglip_preprocess(rgb: &[u8], w: u32, h: u32, size: u32) -> Vec<f32> {
    let size = size.max(1);
    let img = image::RgbImage::from_raw(w.max(1), h.max(1), rgb.to_vec())
        .unwrap_or_else(|| image::RgbImage::new(size, size));
    // The image crate's Triangle filter, to the bit, without its cost: on a
    // GPU it was most of each photo's time (`raster::resize_triangle`).
    let resized = crate::raster::resize_triangle(&img, size, size);
    let plane = (size * size) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, p) in resized.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (p.0[c] as f32 / 255.0 - 0.5) / 0.5;
        }
    }
    out
}

// ------------------------------------------------------------------------
// ONNX session + tokenizer — only with `--features ai`.
// ------------------------------------------------------------------------
#[cfg(feature = "ai")]
pub use imp::BuiltinTagger;

#[cfg(feature = "ai")]
mod imp {
    use super::*;
    use anyhow::{Context, Result};
    use async_trait::async_trait;
    use std::path::Path;

    /// SigLIP 2 base-224's input resolution.
    const IMAGE_SIZE: u32 = 224;
    /// SigLIP's text length. It was trained with every prompt padded to this,
    /// so shorter padding (or none) shifts the embeddings.
    const CONTEXT: usize = 64;
    /// Prompts per text-encoder run when embedding the vocabulary.
    const TEXT_BATCH: usize = 64;
    /// Where the embedded vocabulary is cached, inside the model folder.
    const CACHE_FILE: &str = "labels.bin";

    /// A loaded SigLIP 2 model. Its label vocabulary is embedded on first use
    /// by tagging, not at load: search needs only the two encoders, and
    /// embedding ~6,500 label prompts first held up the first index by
    /// seconds on a GPU and minutes on a CPU, with nothing on screen.
    ///
    /// Each half is loaded the first time it is needed, not with the model.
    /// Indexing needs only the vision encoder, and tagging from stored
    /// embeddings with cached labels needs neither; the text encoder — 565 MB
    /// of the 751 on disk — and the tokenizer's vocabulary are for a search,
    /// or for embedding the labels afresh. Loading all of it up front put
    /// about 2 GB in memory on an M4 for a session that only indexed.
    pub struct BuiltinTagger {
        vision: Part<std::sync::Mutex<ort::session::Session>>,
        /// Kept once loaded (not dropped after embedding the labels) so a
        /// *search query* can be embedded on demand into the same space as the
        /// images.
        text: Part<std::sync::Mutex<ort::session::Session>>,
        tokenizer: Part<tokenizers::Tokenizer>,
        /// One L2-normalised text embedding per label in `label_prompts()`
        /// order, filled by [`Self::labels`] (from the on-disk cache when it
        /// matches). An error is kept as text so it is reported, not retried
        /// on every photo.
        label_embeddings: std::sync::OnceLock<std::result::Result<Vec<Vec<f32>>, String>>,
        /// Where the model (and the label cache) live.
        dir: std::path::PathBuf,
    }

    /// A part of the model, loaded the first time it is asked for and kept.
    /// A failure is kept too, as text, so a broken file is reported each time
    /// rather than loaded again for every photo.
    type Part<T> = std::sync::OnceLock<std::result::Result<T, String>>;

    const VISION: &str = "vision_model.onnx";
    /// The vision tower in fp32, which a PC without a GPU downloads instead
    /// (`ai_models::Variant`): twice as fast on its CPU.
    const VISION_FP32: &str = "vision_model_fp32.onnx";
    const TEXT: &str = "text_model.onnx";
    const TOKENIZER: &str = "tokenizer.json";

    /// The model loaded from a folder, held weakly: see [`BuiltinTagger::shared`].
    type Loaded = Option<(std::path::PathBuf, std::sync::Weak<BuiltinTagger>)>;
    static LOADED: std::sync::Mutex<Loaded> = std::sync::Mutex::new(None);

    impl BuiltinTagger {
        /// The model in `dir` (`vision_model.onnx`, `text_model.onnx`,
        /// `tokenizer.json`), ready to load each part on first use. The files
        /// are checked now, so a missing one fails here, where the caller can
        /// say so, rather than halfway through a run. The label vocabulary
        /// waits for [`Self::labels`].
        pub fn load(dir: &Path) -> Result<Self> {
            crate::develop::ai::ensure_ort_dylib();
            for file in [TEXT, TOKENIZER] {
                let path = dir.join(file);
                anyhow::ensure!(
                    path.is_file(),
                    "the model is incomplete: no {}",
                    path.display()
                );
            }
            anyhow::ensure!(
                dir.join(VISION).is_file() || dir.join(VISION_FP32).is_file(),
                "the model is incomplete: no {}",
                dir.join(VISION).display()
            );
            Ok(Self {
                vision: Part::new(),
                text: Part::new(),
                tokenizer: Part::new(),
                label_embeddings: std::sync::OnceLock::new(),
                dir: dir.to_path_buf(),
            })
        }

        /// The model in `dir`, shared: the one already loaded if search or
        /// tagging holds it, else a new one.
        ///
        /// Search keeps its model for the session, and tagging built its own
        /// beside it — 630 MB more, measured on an M4 — for the same weights.
        /// Only a weak reference is kept here, so the model still goes when
        /// neither holds it.
        pub fn shared(dir: &Path) -> Result<std::sync::Arc<Self>> {
            let mut loaded = crate::sync::lock_recover(&LOADED);
            if let Some((at, model)) = loaded.as_ref() {
                if let Some(live) = model.upgrade().filter(|_| at == dir) {
                    return Ok(live);
                }
            }
            let model = std::sync::Arc::new(Self::load(dir)?);
            *loaded = Some((dir.to_path_buf(), std::sync::Arc::downgrade(&model)));
            Ok(model)
        }

        /// Stops [`Self::shared`] handing out the model now loaded, so the next
        /// asker loads the files as they are now — after the model is deleted
        /// or downloaded again, even while a run still holds the old one.
        pub fn forget_shared() {
            *crate::sync::lock_recover(&LOADED) = None;
        }

        fn session<'a>(
            &self,
            part: &'a Part<std::sync::Mutex<ort::session::Session>>,
            file: &str,
        ) -> Result<std::sync::MutexGuard<'a, ort::session::Session>> {
            // The vision tower indexes the library in the background; the
            // text tower answers a search someone is waiting on.
            let workload = if file != TEXT {
                crate::develop::ai::Workload::Background
            } else {
                crate::develop::ai::Workload::Interactive
            };
            let loaded = part.get_or_init(|| {
                build_session(&self.dir.join(file), workload)
                    .map(std::sync::Mutex::new)
                    .map_err(|e| format!("{e:#}"))
            });
            match loaded {
                Ok(session) => Ok(crate::sync::lock_recover(session)),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }

        /// The vision tower's file: this machine's precision of it
        /// (`ai_models::runs_on`: fp32 where it runs on the CPU, fp16
        /// everywhere else) when it is there, else whichever is.
        fn vision_file(&self) -> &'static str {
            let cpu = crate::ai_models::runs_on() == Some(crate::ai_models::RunsOn::Cpu);
            let order = if cpu {
                [VISION_FP32, VISION]
            } else {
                [VISION, VISION_FP32]
            };
            order
                .into_iter()
                .find(|f| self.dir.join(f).is_file())
                .unwrap_or(VISION)
        }

        fn tokenizer(&self) -> Result<&tokenizers::Tokenizer> {
            self.tokenizer
                .get_or_init(|| {
                    tokenizers::Tokenizer::from_file(self.dir.join(TOKENIZER))
                        .map_err(|e| format!("loading {TOKENIZER}: {e}"))
                })
                .as_ref()
                .map_err(|e| anyhow::anyhow!("{e}"))
        }

        /// The label vocabulary's embeddings: from the on-disk cache when it
        /// matches, otherwise by embedding every prompt (a few seconds on a
        /// GPU) and writing the cache for next time.
        pub fn labels(&self) -> Result<&[Vec<f32>]> {
            let got = self.label_embeddings.get_or_init(|| {
                let labels = label_prompts();
                let identity = std::fs::metadata(self.dir.join(TEXT))
                    .map(|m| {
                        let modified = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_secs());
                        format!("{}:{modified}", m.len())
                    })
                    .unwrap_or_default();
                let key = cache_key(&labels, &identity);
                let cache_path = self.dir.join(CACHE_FILE);
                if let Some(rows) = std::fs::read(&cache_path)
                    .ok()
                    .and_then(|b| decode_label_cache(&b, key, labels.len()))
                {
                    return Ok(rows);
                }
                let tokenizer = self.tokenizer().map_err(|e| format!("{e:#}"))?;
                let mut text = self
                    .session(&self.text, TEXT)
                    .map_err(|e| format!("{e:#}"))?;
                let rows =
                    embed_labels(&mut text, tokenizer, &labels).map_err(|e| format!("{e:#}"))?;
                // A failed write only costs the next launch the same
                // embedding pass; it must not fail tagging.
                if let Err(e) = std::fs::write(&cache_path, encode_label_cache(key, &rows)) {
                    eprintln!("tagger: could not cache label embeddings: {e}");
                }
                Ok(rows)
            });
            got.as_deref()
                .map_err(|e| anyhow::anyhow!("embedding the tag vocabulary: {e}"))
        }

        /// The image embedding, L2-normalised — the vector stored per photo
        /// for semantic search, and the basis of tagging below.
        pub fn embed_image(&self, image: &[u8]) -> Result<Vec<f32>> {
            let decoded = image::load_from_memory(image)
                .context("decoding preview image")?
                .to_rgb8();
            let input = siglip_preprocess(
                decoded.as_raw(),
                decoded.width(),
                decoded.height(),
                IMAGE_SIZE,
            );
            let tensor = ort::value::Value::from_array((
                [1_i64, 3, IMAGE_SIZE as i64, IMAGE_SIZE as i64],
                input,
            ))?;
            let mut emb = {
                let mut vision = self.session(&self.vision, self.vision_file())?;
                let name = vision
                    .inputs
                    .first()
                    .map(|i| i.name.clone())
                    .unwrap_or_default();
                let idx = output_index(&vision, &["image_embeds", "pooler_output"]);
                let outputs = vision.run(ort::inputs![name.as_str() => tensor])?;
                let (_shape, data) = outputs[idx].try_extract_tensor::<f32>()?;
                data.to_vec()
            };
            l2_normalize(&mut emb);
            Ok(emb)
        }

        /// The text embedding of a free-text search query, L2-normalised, in
        /// the same space as [`Self::embed_image`] — so a dot product ranks
        /// photos by how well they match the words.
        pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
            let tokenizer = self.tokenizer()?;
            let mut text = self.session(&self.text, TEXT)?;
            let mut emb = embed_texts(&mut text, tokenizer, &[query])?
                .pop()
                .unwrap_or_default();
            l2_normalize(&mut emb);
            Ok(emb)
        }

        /// Tags from an image embedding already in hand — typically the one
        /// the search index stored for the photo — so no image work is needed.
        /// `None` when the vector is from a different model (wrong length).
        pub fn tag_embedding(&self, emb: &[f32]) -> Option<TagResult> {
            let labels = match self.labels() {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("tagger: {e:#}");
                    return None;
                }
            };
            if labels.first().map(|l| l.len()) != Some(emb.len()) {
                return None;
            }
            let sims: Vec<f32> = labels.iter().map(|label| dot(emb, label)).collect();
            Some(assemble(&sims))
        }
    }

    #[async_trait]
    impl crate::tagger::Tagger for BuiltinTagger {
        async fn tag(&self, image: &[u8]) -> Result<TagResult> {
            Ok(self.tag_and_embed(image).await?.0)
        }

        // Inference is CPU/GPU-bound and synchronous, and `self` is shared, so
        // it runs on the current thread under the session mutex rather than
        // being moved across threads.
        async fn tag_and_embed(&self, image: &[u8]) -> Result<(TagResult, Option<Vec<f32>>)> {
            let emb = self.embed_image(image)?;
            let tags = self
                .tag_embedding(&emb)
                .context("image and label embeddings differ in size")?;
            Ok((tags, Some(emb)))
        }

        fn tag_from_embedding(&self, emb: &[f32]) -> Option<TagResult> {
            self.tag_embedding(emb)
        }

        async fn check(&self) -> Result<()> {
            // Loading succeeded (models + tokenizer parsed). Embedding the
            // vocabulary here puts that wait in the tagging batch's "Loading
            // model..." phase rather than on its first photo.
            self.labels().map(|_| ())
        }
    }

    /// Full graph optimisation with the GPU providers, falling back to basic
    /// optimisation on the CPU alone.
    ///
    /// Both halves matter for the fp16 model (measured in
    /// `docs/AI-MODEL-SPIKES.md`). On DirectML, anything short of full
    /// optimisation leaves fp16 layer norm unfused, and the embeddings drift
    /// badly enough that search falls below CLIP's. On the CPU, full
    /// optimisation's layer-norm fusion trips on the export and the vision
    /// graph refuses to load, while basic runs it exactly. So: try full with
    /// the GPU; if that cannot load, the CPU path at basic.
    ///
    /// **A Mac goes straight to the CPU.** CoreML cannot compile this export
    /// under ONNX Runtime 1.24.4 (`Conv: Required param 'pad' is missing` for
    /// the vision half, a failed output declaration for the text half), and
    /// under 1.22.0, where it did compile, it took none of the work: indexing
    /// 73 photos on an M4 took 28.8 s with it and 28.4 s without, the same CPU
    /// time either way. On 1.24.4 the CPU path takes 21.8 s. Trying CoreML
    /// first would only add a failed compile to every load.
    ///
    /// `workload` sets how its threads share the CPU
    /// (`develop::ai::cpu_threads`).
    fn build_session(
        path: &Path,
        workload: crate::develop::ai::Workload,
    ) -> Result<ort::session::Session> {
        use crate::develop::ai::cpu_threads;
        use ort::execution_providers::CPUExecutionProvider;
        use ort::session::builder::GraphOptimizationLevel;
        let cpu = || -> Result<ort::session::Session> {
            cpu_threads(ort::session::Session::builder()?, workload)?
                .with_optimization_level(GraphOptimizationLevel::Level1)?
                .with_execution_providers([CPUExecutionProvider::default().build()])?
                .commit_from_file(path)
                .with_context(|| format!("loading {}", path.display()))
        };
        if cfg!(target_os = "macos") || crate::develop::ai::cpu_only() {
            return cpu();
        }
        let gpu = cpu_threads(ort::session::Session::builder()?, workload)?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_execution_providers(crate::develop::ai::gpu_providers(true, None))
            .and_then(|b| b.commit_from_file(path));
        match gpu {
            Ok(session) => Ok(session),
            Err(e) => {
                eprintln!(
                    "tagger: no GPU session for {} ({e}); using the CPU",
                    path.display()
                );
                cpu()
            }
        }
    }

    /// Embeds every label's prompts and averages each label's ensemble.
    ///
    /// Prompts go through the encoder in batches — one run per prompt made a
    /// ~6,500-prompt vocabulary take minutes. Some exports fix the batch
    /// dimension at 1, so a failed batch falls back to one prompt at a time.
    fn embed_labels(
        text: &mut ort::session::Session,
        tokenizer: &tokenizers::Tokenizer,
        labels: &[Vec<String>],
    ) -> Result<Vec<Vec<f32>>> {
        let flat: Vec<&str> = labels.iter().flatten().map(String::as_str).collect();
        let mut embs: Vec<Vec<f32>> = Vec::with_capacity(flat.len());
        let mut batch = TEXT_BATCH;
        let mut i = 0;
        while i < flat.len() {
            let chunk = &flat[i..(i + batch).min(flat.len())];
            match embed_texts(text, tokenizer, chunk) {
                Ok(out) => {
                    embs.extend(out);
                    i += chunk.len();
                }
                Err(e) if batch > 1 => {
                    eprintln!(
                        "tagger: batched text encoding failed ({e:#}); retrying one at a time"
                    );
                    batch = 1;
                }
                Err(e) => return Err(e),
            }
        }
        for e in &mut embs {
            l2_normalize(e);
        }

        let mut rows = Vec::with_capacity(labels.len());
        let mut at = 0;
        for group in labels {
            rows.push(mean_embedding(&embs[at..at + group.len()]));
            at += group.len();
        }
        Ok(rows)
    }

    /// Runs a batch of prompts through the text encoder, one (not yet
    /// normalised) embedding per prompt. Prompts are lower-cased and padded to
    /// [`CONTEXT`] with the tokenizer's `<pad>`, which is how SigLIP was trained.
    fn embed_texts(
        text: &mut ort::session::Session,
        tokenizer: &tokenizers::Tokenizer,
        prompts: &[&str],
    ) -> Result<Vec<Vec<f32>>> {
        let n = prompts.len();
        let pad = tokenizer.token_to_id("<pad>").unwrap_or(0) as i64;
        let mut ids: Vec<i64> = Vec::with_capacity(n * CONTEXT);
        let mut mask: Vec<i64> = Vec::with_capacity(n * CONTEXT);
        for prompt in prompts {
            let enc = tokenizer
                .encode(prompt.to_lowercase(), true)
                .map_err(|e| anyhow::anyhow!("tokenising: {e}"))?;
            // Pad / truncate to the context length.
            let mut row: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
            let mut m: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
            row.resize(CONTEXT, pad);
            m.resize(CONTEXT, 0);
            ids.extend(row);
            mask.extend(m);
        }

        let ids_t = ort::value::Value::from_array(([n as i64, CONTEXT as i64], ids))?;
        let mask_t = ort::value::Value::from_array(([n as i64, CONTEXT as i64], mask))?;
        let idx = output_index(text, &["text_embeds", "pooler_output"]);
        // Exports differ: some text encoders take an attention mask, others
        // (the Xenova CLIP ONNX among them) take only the ids and would reject
        // the extra input outright. Resolved before `run`, which borrows the
        // session mutably.
        let wants_mask = text.inputs.iter().any(|i| i.name == "attention_mask");
        let outputs = if wants_mask {
            text.run(ort::inputs![
                "input_ids" => ids_t,
                "attention_mask" => mask_t,
            ])?
        } else {
            text.run(ort::inputs!["input_ids" => ids_t])?
        };
        let (_shape, data) = outputs[idx].try_extract_tensor::<f32>()?;
        if data.is_empty() || data.len() % n != 0 {
            anyhow::bail!(
                "text encoder returned {} values for {n} prompts",
                data.len()
            );
        }
        let dim = data.len() / n;
        Ok(data.chunks_exact(dim).map(|c| c.to_vec()).collect())
    }

    /// Index of the first output named in `preferred` (the pooled, projected
    /// embedding), falling back to the first output. Exports vary: CLIP names
    /// it `image_embeds`/`text_embeds`, SigLIP `pooler_output`, and both put
    /// `last_hidden_state` first, which would be silently wrong.
    fn output_index(session: &ort::session::Session, preferred: &[&str]) -> usize {
        preferred
            .iter()
            .find_map(|want| session.outputs.iter().position(|o| o.name == *want))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(name: &str) -> usize {
        subjects()
            .iter()
            .position(|l| l.name == name)
            .unwrap_or_else(|| panic!("'{name}' is in subjects.txt"))
    }

    #[test]
    fn the_shipped_vocabulary_parses_and_is_well_formed() {
        let labels = subjects();
        assert!(labels.len() > 1000, "got {} labels", labels.len());
        for l in labels {
            // Mirrors `tagger::normalise_list`, so a tag never gets rewritten
            // (and split into two facet entries) on its way into the database.
            let n = &l.name;
            assert_eq!(n, &n.to_lowercase(), "'{n}' must be lowercase");
            assert!(!n.contains('_'), "'{n}' must use spaces");
            assert!(n.len() <= 40, "'{n}' is longer than 40 characters");
            assert_eq!(
                n,
                &n.split_whitespace().collect::<Vec<_>>().join(" "),
                "'{n}' has stray whitespace"
            );
        }
        // A group is only useful as somebody's parent.
        for (i, l) in labels.iter().enumerate().filter(|(_, l)| l.group) {
            assert!(
                labels.iter().any(|c| c.parent == Some(i)),
                "group '{}' has no children",
                l.name
            );
        }
    }

    #[test]
    fn parse_rejects_unknown_parents_duplicates_and_cycles() {
        assert!(parse_labels("dog > animal").is_err(), "unknown parent");
        assert!(parse_labels("dog\ndog").is_err(), "duplicate");
        assert!(parse_labels("a > b\nb > a").is_err(), "cycle");
        let ok = parse_labels("# comment\n\n[animal]\ndog > animal\n").unwrap();
        assert_eq!(ok.len(), 2);
        assert!(ok[0].group && !ok[1].group);
        assert_eq!(ok[1].parent, Some(0));
    }

    #[test]
    fn genres_do_not_reappear_as_subject_tags() {
        // "landscape" the genre and "landscape" the subject would be two facet
        // entries with the same name; the vocabulary keeps such roots as groups.
        for g in GENRES {
            assert!(
                !subject_names().contains(g),
                "'{g}' is a genre and a taggable subject"
            );
        }
    }

    #[test]
    fn label_ranges_cover_all_labels_without_overlap() {
        let all = label_prompts();
        assert_eq!(
            all.len(),
            GENRES.len() + subjects().len() + CONDITIONS.len()
        );
        assert_eq!(genre_range().start, 0);
        assert_eq!(subject_range().start, genre_range().end);
        assert_eq!(condition_range().start, subject_range().end);
        assert_eq!(condition_range().end, all.len());
        // A prompt is more than the bare label, and subjects are ensembled.
        assert!(all[0][0].contains("landscape") && all[0][0].len() > "landscape".len());
        assert_eq!(all[subject_range().start].len(), SUBJECT_TEMPLATES.len());
    }

    #[test]
    fn l2_normalize_and_dot_give_cosine() {
        let mut a = vec![3.0, 4.0];
        l2_normalize(&mut a);
        assert!(
            (dot(&a, &a) - 1.0).abs() < 1e-5,
            "a unit vector dotted with itself is 1"
        );
        let mut b = vec![-4.0, 3.0];
        l2_normalize(&mut b);
        assert!(
            dot(&a, &b).abs() < 1e-5,
            "orthogonal vectors have zero cosine"
        );
    }

    #[test]
    fn mean_embedding_is_normalised() {
        let m = mean_embedding(&[vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert!((dot(&m, &m) - 1.0).abs() < 1e-5);
        assert!((m[0] - m[1]).abs() < 1e-6);
    }

    #[test]
    fn softmax_peaks_on_the_largest_and_sums_to_one() {
        let p = softmax(&[0.1, 0.9, 0.2], 100.0);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        assert!(
            p[1] > 0.99,
            "a strong logit-scale makes the winner dominate"
        );
        let masked = softmax(&[0.5, f32::NEG_INFINITY], 100.0);
        assert_eq!(masked[1], 0.0, "-inf is excluded outright");
    }

    /// A synthetic similarity vector: pick a genre, a couple of subjects and a
    /// condition, and check `assemble` reads them back.
    #[test]
    fn assemble_reads_the_top_matches_per_facet() {
        let mut sims = vec![0.0f32; label_prompts().len()];
        let portrait = GENRES.iter().position(|&g| g == "portrait").unwrap();
        sims[portrait] = 0.9;
        // A clear person, and a bicycle nearly as clear.
        sims[subject_range().start + pos("person")] = 0.5;
        sims[subject_range().start + pos("bicycle")] = 0.49;
        let golden = CONDITIONS.iter().position(|&c| c == "golden hour").unwrap();
        sims[condition_range().start + golden] = 0.6;

        let r = assemble(&sims);
        assert_eq!(r.genre, "portrait");
        assert!(
            r.confidence > 0.9,
            "an unambiguous genre is high-confidence"
        );
        assert_eq!(r.main_subject, "person", "the top subject leads");
        assert!(r.subjects.contains(&"bicycle".to_string()));
        assert!(
            r.subjects.contains(&"vehicle".to_string()),
            "a claimed subject brings its parent"
        );
        assert_eq!(r.conditions, vec!["golden hour".to_string()]);
    }

    /// The failure the real model actually produced: CLIP's similarities live in
    /// a narrow band, so *flat* scores must not be read as "everything matches" —
    /// nor pooled into a broad parent that then claims the photo.
    #[test]
    fn a_flat_score_vector_does_not_tag_everything() {
        let sims = vec![0.25f32; label_prompts().len()];
        let r = assemble(&sims);
        assert!(
            r.conditions.is_empty(),
            "no condition stands out, so none should be claimed: {:?}",
            r.conditions
        );
        assert!(
            r.subjects.len() <= 5,
            "a flat vector is one guess plus its parents, got {:?}",
            r.subjects
        );
        assert!(
            GENRES.contains(&r.genre.as_str()),
            "a genre is still chosen from the closed set"
        );
    }

    #[test]
    fn a_confident_specific_label_brings_its_parents() {
        let mut sims = vec![0.0f32; subjects().len()];
        sims[pos("golden retriever")] = 0.5;
        sims[pos("dog")] = 0.49;
        let s = select_subjects(&sims, subjects());
        assert_eq!(s[0], "golden retriever", "the specific label leads: {s:?}");
        for parent in ["dog", "pet", "animal"] {
            assert!(
                s.contains(&parent.to_string()),
                "missing '{parent}' in {s:?}"
            );
        }
    }

    #[test]
    fn an_unsure_breed_settles_on_the_parent() {
        // Three breeds split the vote and none is sure; together they (and the
        // parent itself) make "dog" the confident answer.
        let mut sims = vec![0.0f32; subjects().len()];
        sims[pos("dog")] = 0.5;
        sims[pos("labrador")] = 0.49;
        sims[pos("golden retriever")] = 0.49;
        sims[pos("beagle")] = 0.49;
        let s = select_subjects(&sims, subjects());
        assert_eq!(s[0], "dog", "{s:?}");
        assert!(
            !s.iter().any(|t| t == "labrador" || t == "beagle"),
            "no breed is claimed when none is sure: {s:?}"
        );
    }

    #[test]
    fn groups_are_never_emitted() {
        let mut sims = vec![0.0f32; subjects().len()];
        sims[pos("mountain")] = 0.5;
        // Even scoring a group highest must not tag it.
        sims[pos("landscape")] = 0.9;
        let s = select_subjects(&sims, subjects());
        assert_eq!(s, vec!["mountain".to_string()]);
    }

    #[test]
    fn label_cache_round_trips_and_rejects_a_stale_key() {
        let rows = vec![vec![0.5f32, -0.25], vec![1.0, 0.0]];
        let bytes = encode_label_cache(42, &rows);
        assert_eq!(decode_label_cache(&bytes, 42, 2), Some(rows));
        assert_eq!(decode_label_cache(&bytes, 43, 2), None, "different key");
        assert_eq!(decode_label_cache(&bytes, 42, 3), None, "different count");
        assert_eq!(decode_label_cache(&bytes[..30], 42, 2), None, "truncated");
    }

    #[test]
    fn cache_key_tracks_prompts_and_model() {
        let p = label_prompts();
        let k = cache_key(&p, "100:1");
        assert_eq!(k, cache_key(&p, "100:1"), "stable");
        assert_ne!(k, cache_key(&p, "100:2"), "a new model file re-embeds");
        let mut q = p.clone();
        q[0][0].push('!');
        assert_ne!(k, cache_key(&q, "100:1"), "an edited prompt re-embeds");
    }

    #[test]
    fn siglip_preprocess_squashes_to_a_square_in_minus_one_to_one() {
        // A 4x2 image becomes 2x2 whole, not a crop of its middle: the left
        // half is black and the right half white, and both survive.
        let mut rgb = vec![0u8; 4 * 2 * 3];
        for y in 0..2 {
            for x in 2..4 {
                for c in 0..3 {
                    rgb[(y * 4 + x) * 3 + c] = 255;
                }
            }
        }
        let t = siglip_preprocess(&rgb, 4, 2, 2);
        assert_eq!(t.len(), 3 * 2 * 2, "planar NCHW");
        assert!(t[0] < -0.5, "left edge kept (black is near -1): {}", t[0]);
        assert!(t[1] > 0.5, "right edge kept (white is near 1): {}", t[1]);
        assert!(t.iter().all(|v| (-1.0..=1.0).contains(v)));
    }
}
