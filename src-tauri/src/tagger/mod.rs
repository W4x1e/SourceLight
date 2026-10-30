//! AI tagging across the local providers: the built-in SigLIP 2 tagger, and
//! the vision model on the app's own `llama-server`.
//!
//! Everything provider-specific lives in the submodules; everything that must
//! stay identical across providers lives here. In particular the **schema field
//! order** is load-bearing: grammar-constrained decoders emit properties in
//! schema order, so `main_subject` must be answered before `genre`. Asking for
//! the genre first made the model label an ice-cream tub in a garden
//! "landscape"; committing to the subject first fixed it. Any new provider must
//! preserve that ordering.

pub mod builtin;
mod llama;

use std::borrow::Cow;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::models::{TagResult, GENRES};

/// The tagging seam.
///
/// Deliberately **not** an `async fn` in a trait: `impl Future` returned from a
/// trait method is not dyn-compatible, and choosing a provider at runtime needs
/// `Box<dyn Tagger>`. `async_trait` boxes the future for us.
#[async_trait]
pub trait Tagger: Send + Sync {
    async fn tag(&self, image: &[u8]) -> Result<TagResult>;

    /// Tags the image and also returns the CLIP search embedding it computed
    /// on the way, if this provider computes one — so a tagging pass fills the
    /// search index for free. Default: just [`Tagger::tag`], no embedding.
    async fn tag_and_embed(&self, image: &[u8]) -> Result<(TagResult, Option<Vec<f32>>)> {
        Ok((self.tag(image).await?, None))
    }

    /// Tags from a stored search embedding without touching the image at all.
    /// `None` means "can't" — this provider doesn't work from embeddings (the
    /// default), or the vector came from a different model — and the caller
    /// falls back to [`Tagger::tag_and_embed`].
    fn tag_from_embedding(&self, _embedding: &[f32]) -> Option<TagResult> {
        None
    }

    /// How many requests a batch may keep in flight at once. Default 1: the
    /// built-in tagger runs its model under a mutex, so overlapping calls would
    /// only queue. A server that runs several requests together (the vision
    /// model's `llama-server`) overrides it.
    fn max_concurrency(&self) -> usize {
        1
    }

    /// Cheap connectivity/auth probe backing the "Test connection" button.
    /// Must fail loudly on a bad key rather than degrading silently.
    async fn check(&self) -> Result<()>;

    /// A free-form caption/description of the image for full-text + OCR search.
    /// Default is "unsupported" — only a VLM provider can do this, so the
    /// built-in CLIP tagger and any non-vision model fall through to a clear
    /// error rather than a silent empty caption.
    async fn caption(&self, _image: &[u8]) -> Result<String> {
        Err(anyhow!(
            "this tagger can't write descriptions — turn on Detailed descriptions in Settings → AI"
        ))
    }

    /// Tags the image and describes it, in one request where the provider can.
    ///
    /// A vision model spends most of a request reading the image — about a
    /// thousand tokens of it — so tags and a description asked for together
    /// cost little more than either alone: 10.3 s a photo against 16.1 s for
    /// the two requests, with Qwen3-VL 4B on an M4. The vision model overrides
    /// this; the default makes the two calls in turn.
    async fn tag_and_caption(&self, image: &[u8]) -> Result<(TagResult, String)> {
        let tags = self.tag(image).await?;
        let caption = self.caption(image).await?;
        Ok((tags, caption))
    }

    /// Loads the model into memory *before* the first photo is sent.
    ///
    /// Without this the cold load happens inside the first `tag`/`caption`
    /// request, which is why tagging looked frozen for a minute at `0/N` on a
    /// 7B vision model: the batch had started, but nothing could report
    /// progress until the weights were resident. Callers warm first and say so
    /// in the UI, so the wait is visible and attributed.
    ///
    /// Default no-op. The built-in tagger loads each encoder the first time a
    /// photo needs it — often never, when it tags from stored embeddings. The
    /// vision model's server is started, and waited for, by `commands/ai.rs`.
    async fn warm(&self) -> Result<()> {
        Ok(())
    }

    /// Names `terms` — tags as stored, in English — in `language` (an English
    /// language name from [`language_name`]), one answer per term and `None`
    /// where the model gave none. For the tags a vision model wrote freely,
    /// which no fixed table can name: the interface shows the answers, and
    /// the tags themselves stay English (see `db::tag_translations`).
    ///
    /// Default "unsupported": only a language model can do this.
    async fn translate_terms(
        &self,
        _terms: &[String],
        _language: &str,
    ) -> Result<Vec<Option<String>>> {
        Err(anyhow!("this tagger can't translate tags"))
    }
}

/// A shared tagger is a tagger — how tagging uses the built-in model search
/// already has loaded (`builtin::BuiltinTagger::shared`), instead of loading a
/// second copy of it.
#[async_trait]
impl<T: Tagger + ?Sized> Tagger for std::sync::Arc<T> {
    async fn tag(&self, image: &[u8]) -> Result<TagResult> {
        (**self).tag(image).await
    }
    async fn tag_and_embed(&self, image: &[u8]) -> Result<(TagResult, Option<Vec<f32>>)> {
        (**self).tag_and_embed(image).await
    }
    fn tag_from_embedding(&self, embedding: &[f32]) -> Option<TagResult> {
        (**self).tag_from_embedding(embedding)
    }
    fn max_concurrency(&self) -> usize {
        (**self).max_concurrency()
    }
    async fn check(&self) -> Result<()> {
        (**self).check().await
    }
    async fn caption(&self, image: &[u8]) -> Result<String> {
        (**self).caption(image).await
    }
    async fn tag_and_caption(&self, image: &[u8]) -> Result<(TagResult, String)> {
        (**self).tag_and_caption(image).await
    }
    async fn warm(&self) -> Result<()> {
        (**self).warm().await
    }
    async fn translate_terms(
        &self,
        terms: &[String],
        language: &str,
    ) -> Result<Vec<Option<String>>> {
        (**self).translate_terms(terms, language).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    None,
    /// In-process SigLIP 2 zero-shot tagging — no server, no key. See `builtin`.
    Builtin,
    /// A vision model (Qwen3-VL) on the app's own `llama-server`: tags, and
    /// the detailed descriptions only it can write. See `llama`.
    Vision,
}

impl Provider {
    /// `ollama` is what the vision model was stored as when it
    /// ran through Ollama; it still means the vision model.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "builtin" | "local" => Self::Builtin,
            "vision" | "ollama" => Self::Vision,
            _ => Self::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Builtin => "builtin",
            Self::Vision => "vision",
        }
    }

    /// Sensible default model per provider, used when settings are empty.
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Builtin => "clip",
            Self::Vision => crate::vlm::recommended(None),
            Self::None => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub provider: Provider,
    pub model: String,
    /// App models directory: `siglip2/` for the built-in tagger, `vision/`
    /// for the vision models.
    pub model_dir: Option<std::path::PathBuf>,
    /// Where `llama_pack` installs `llama-server`. Only the vision model uses it.
    pub runtime_dir: Option<std::path::PathBuf>,
    /// How many photos to send at once, already resolved from the setting (see
    /// [`resolve_parallel`]). Only the vision model uses it.
    pub parallel: usize,
    /// The language a description is written in, as an English name from
    /// [`language_name`]; `None` is English. Tags stay English whatever this is.
    pub caption_language: Option<&'static str>,
}

/// The most requests a batch keeps in flight, however much VRAM there is. Past
/// this, a single GPU gains little more throughput from batching and each
/// request just waits longer.
pub const MAX_PARALLEL: usize = 4;

/// The `ai_parallel` setting as a request count: an explicit 1..=4 is taken as
/// is, and 0 (or unset) means "decide from the GPU".
///
/// Each parallel request is one more copy of the context cache in VRAM on top
/// of the model itself, so the auto choice scales with memory: a 7B vision
/// model already fills most of an 8 GB card. Unknown VRAM stays at 1 — the safe
/// answer when we can't tell.
pub fn resolve_parallel(setting: usize, vram_mb: Option<u32>) -> usize {
    if setting > 0 {
        return setting.min(MAX_PARALLEL);
    }
    match vram_mb {
        Some(mb) if mb >= 16 * 1024 => 4,
        Some(mb) if mb >= 12 * 1024 => 3,
        Some(mb) if mb >= 8 * 1024 => 2,
        _ => 1,
    }
}

/// Builds the configured provider.
pub fn build(cfg: &ProviderConfig) -> Result<Box<dyn Tagger>> {
    Ok(match cfg.provider {
        Provider::Builtin => {
            #[cfg(feature = "ai")]
            {
                let dir = cfg
                    .model_dir
                    .clone()
                    .ok_or_else(|| anyhow!("no model directory available"))?
                    .join(builtin::MODEL_DIR);
                Box::new(builtin::BuiltinTagger::shared(&dir)?)
            }
            #[cfg(not(feature = "ai"))]
            {
                return Err(anyhow!(
                    "this build has no built-in tagger — rebuild with `--features ai`"
                ));
            }
        }
        // Always the app's own server: there is no other host. Starting
        // it on the model's files is `commands/ai.rs`'s job, before the first photo.
        Provider::Vision => Box::new(
            llama::LlamaServerTagger::new(cfg.parallel).with_language(cfg.caption_language),
        ),
        Provider::None => return Err(anyhow!("no AI provider configured")),
    })
}

// ---------------------------------------------------------------------------
// Shared prompt + schema
// ---------------------------------------------------------------------------

/// What a description should cover. A macro rather than a `const` so the
/// caption request and the combined one (`concat!` takes only literals) say
/// exactly the same thing.
macro_rules! describe {
    () => {
        "\
Write a short, factual description of this photograph for a searchable photo
library. In two or three sentences, cover: the main subject, the setting or
location, notable colours, any people or activity, and the lighting. Use plain,
concrete words a person would actually search for. Write it the way a person
captions their own photos: do not open with \"This image shows\" or similar, and
leave out judgements such as stunning, beautiful, serene or vibrant.

If any text is clearly legible in the image — signs, labels, documents, screens,
book spines — transcribe it verbatim in the same description."
    };
}

/// How to tag, in schema order. A macro for the same reason as `describe`.
macro_rules! tag_instructions {
    () => {
        "\
You are tagging a photograph for a personal photo library.

First identify main_subject: the single thing the photograph is actually OF - the
element that dominates the frame or is in sharpest focus. Look at what fills the
frame and what the lens is focused on, not the background.

Then list subjects: 3-8 concrete visible nouns, lowercase singular, most prominent
first. Only what is actually visible.

Then conditions: lighting/weather only if clearly identifiable (\"golden hour\",
\"overcast\", \"backlit\", \"night\", \"sunny\"). Empty list if unclear.

Then choose genre - the ONE category that matches main_subject:
- portrait: main_subject is a person
- street: main_subject is candid people or urban life
- wildlife: main_subject is a wild animal, bird or insect
- macro: main_subject is a small thing shot very close (insect, flower detail)
- still-life: main_subject is an inanimate object, food or drink
- architecture: main_subject is a building or structure
- sports: main_subject is physical action or competition
- event: main_subject is a gathering or celebration
- astro: main_subject is the night sky
- abstract: no identifiable main_subject, only pattern or texture
- landscape: main_subject IS the wide scene itself - scenery, garden or vista with
  no person, animal or object dominating

Critical: if main_subject is a person, object, food or animal, the genre is NOT
landscape, even when the setting is outdoors and green.

Finally confidence: genuine certainty in the genre. Use the FULL range - 0.4-0.6
when ambiguous, above 0.9 only when unmistakable."
    };
}

/// Captioning is free-form, not schema-constrained: recall is what matters for
/// search, so the caption should be a dense, plain-prose description plus any
/// legible text (OCR) — more searchable words beat terse elegance. Kept
/// deliberately unstructured so it also reads well in the Lightbox metadata.
pub(crate) const CAPTION_PROMPT: &str = concat!(
    describe!(),
    "\n\nReply with the description only: no preamble, no bullet points, no quotation."
);

/// The instruction order mirrors the schema's property order on purpose.
pub(crate) const PROMPT: &str = tag_instructions!();

/// The field order here is the whole point — see the module docs. A complete
/// `required` list plus `additionalProperties: false` keeps grammar-constrained
/// decoding strict, so one schema serves every provider.
pub(crate) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "main_subject": { "type": "string" },
            "subjects": { "type": "array", "items": { "type": "string" } },
            "conditions": { "type": "array", "items": { "type": "string" } },
            "genre": { "type": "string", "enum": GENRES },
            "confidence": { "type": "number" }
        },
        "required": ["main_subject", "subjects", "conditions", "genre", "confidence"],
        "additionalProperties": false
    })
}

/// Tags and a description in one request: the tag instructions, then the
/// description's, in the order the schema asks for them.
pub(crate) const TAG_AND_CAPTION_PROMPT: &str = concat!(
    tag_instructions!(),
    "\n\nLast, description: ",
    describe!(),
    "\n\nWrite it as plain prose."
);

/// The languages a description can be written in besides English — the
/// interface's (docs/I18N.md). The interface sends a code, and only a name from
/// this list ever reaches a prompt.
pub fn language_name(code: &str) -> Option<&'static str> {
    match code {
        "de" => Some("German"),
        "es" => Some("Spanish"),
        "fr" => Some("French"),
        "ja" => Some("Japanese"),
        _ => None,
    }
}

/// [`CAPTION_PROMPT`], asking for the description in `language` when there is one.
pub(crate) fn caption_prompt(language: Option<&str>) -> Cow<'static, str> {
    match language {
        None => Cow::Borrowed(CAPTION_PROMPT),
        Some(name) => Cow::Owned(format!(
            "{CAPTION_PROMPT}\n\nWrite the description in {name}."
        )),
    }
}

/// [`TAG_AND_CAPTION_PROMPT`], with the description in `language` when there is
/// one — and only the description: tags are the catalogue's keys, filtered on
/// and written into files as keywords, so they stay the English the rest of
/// the prompt asks for (docs/I18N.md).
pub(crate) fn tag_and_caption_prompt(language: Option<&str>) -> Cow<'static, str> {
    match language {
        None => Cow::Borrowed(TAG_AND_CAPTION_PROMPT),
        Some(name) => Cow::Owned(format!(
            "{TAG_AND_CAPTION_PROMPT}\n\nWrite the description in {name}. Keep main_subject, \
             subjects, conditions and genre in English, exactly as asked above."
        )),
    }
}

/// Asks for `terms` in `language`, each paired with the term it answers so a
/// skipped or reordered answer cannot land on the wrong tag.
pub(crate) fn translation_prompt(terms: &[String], language: &str) -> String {
    format!(
        "These are keywords from a personal photo library: short English labels for what \
         a photograph shows — things, animals, places, activities, light. Give each one in \
         {language} as a photo app would show it: the everyday word, singular, no article, \
         no explanation, capitalised only as {language} requires. Answer every keyword, in \
         the same order, repeating the English exactly as given.\n\n{}",
        serde_json::to_string(terms).unwrap_or_default()
    )
}

/// A ceiling on a translation reply's length: each answer is about fifteen
/// tokens of JSON, so this only stops a runaway.
pub(crate) fn translation_tokens(count: usize) -> u32 {
    u32::try_from(count)
        .unwrap_or(u32::MAX)
        .saturating_mul(40)
        .saturating_add(64)
}

/// The reply [`translation_prompt`] asks for.
pub(crate) fn translation_schema(count: usize) -> Value {
    json!({
        "type": "object",
        "properties": {
            "translations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "en": { "type": "string" },
                        "translation": { "type": "string" }
                    },
                    "required": ["en", "translation"],
                    "additionalProperties": false
                },
                "minItems": count,
                "maxItems": count
            }
        },
        "required": ["translations"],
        "additionalProperties": false
    })
}

/// The answers in a [`translation_schema`] reply, one per term: matched by the
/// English each answer repeats, so a term the model skipped is `None` rather
/// than its neighbour's word. An answer that is blank, runs long, or is the
/// English again is no answer.
pub(crate) fn parse_translations(
    provider: &str,
    reply: &str,
    terms: &[String],
) -> Result<Vec<Option<String>>> {
    #[derive(Deserialize)]
    struct Pair {
        en: String,
        translation: String,
    }
    #[derive(Deserialize)]
    struct Reply {
        translations: Vec<Pair>,
    }
    let parsed: Reply = serde_json::from_str(reply.trim())
        .map_err(|e| anyhow!("{provider} returned unparseable translations: {e}"))?;
    let found: std::collections::HashMap<String, String> = parsed
        .translations
        .into_iter()
        .map(|p| (p.en.trim().to_lowercase(), p.translation.trim().to_string()))
        .collect();
    Ok(terms
        .iter()
        .map(|term| {
            found
                .get(&term.trim().to_lowercase())
                .filter(|t| {
                    !t.is_empty() && t.chars().count() <= 60 && !t.eq_ignore_ascii_case(term)
                })
                .cloned()
        })
        .collect())
}

/// [`schema`] with a `description` after everything else. Last, so the tags
/// are decided exactly as they are without it — observations before
/// conclusions still holds for them — and the prose comes once they are.
pub(crate) fn schema_with_description() -> Value {
    let mut schema = schema();
    schema["properties"]
        .as_object_mut()
        .expect("the schema's properties are an object")
        .insert("description".into(), json!({ "type": "string" }));
    schema["required"]
        .as_array_mut()
        .expect("the schema's required list is an array")
        .push(json!("description"));
    schema
}

/// Parses a combined reply into tags (finalised as [`parse_json`] does) and
/// the description. A reply with tags but no description is an error, so the
/// photo stays in the queue for its description rather than getting none.
pub(crate) fn parse_tagged_description(provider: &str, raw: &str) -> Result<(TagResult, String)> {
    #[derive(Deserialize)]
    struct Described {
        #[serde(flatten)]
        tags: TagResult,
        #[serde(default)]
        description: String,
    }
    let trimmed = strip_code_fence(raw);
    let reply: Described = serde_json::from_str(trimmed).map_err(|e| {
        anyhow!("{provider} returned unparseable JSON: {e}; body was: {trimmed:.300}")
    })?;
    let description = reply.description.trim().to_string();
    if description.is_empty() {
        return Err(anyhow!("{provider} returned tags but an empty description"));
    }
    Ok((finalise(reply.tags), description))
}

/// Post-processing every provider shares, so the same photo tags identically
/// whichever local provider ran it.
pub(crate) fn finalise(mut result: TagResult) -> TagResult {
    // The schema constrains the enum, but normalise anyway so a stray
    // capitalisation can never fragment the genre facet.
    result.genre = result.genre.trim().to_lowercase();
    if !GENRES.contains(&result.genre.as_str()) {
        result.genre = String::new();
    }

    // Models occasionally name a main_subject that never appears in their own
    // subjects list, so fold it in — it is the most searchable term of the lot.
    let mut subjects = result.subjects;
    subjects.insert(0, result.main_subject.clone());
    result.subjects = normalise_list(subjects);
    result.conditions = normalise_list(result.conditions);
    result
}

/// Parses a model's JSON string into a `TagResult`, with an error that says
/// which provider produced the unparseable output.
pub(crate) fn parse_json(provider: &str, raw: &str) -> Result<TagResult> {
    let trimmed = strip_code_fence(raw);
    serde_json::from_str::<TagResult>(trimmed)
        .map(finalise)
        .map_err(|e| anyhow!("{provider} returned unparseable JSON: {e}; body was: {trimmed:.300}"))
}

/// Some providers wrap JSON in a ```json fence despite a schema being set.
fn strip_code_fence(raw: &str) -> &str {
    let s = raw.trim();
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.trim_start_matches('\n')
        .strip_suffix("```")
        .unwrap_or(rest)
        .trim()
}

fn normalise_list(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        // Models mix "brick_wall" and "brick wall"; without this they become
        // two separate facet entries for the same thing.
        let s = item.trim().to_lowercase().replace('_', " ");
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if s.is_empty() || s.len() > 40 {
            continue;
        }
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Turns a non-2xx response into an error carrying the server's own message,
/// rather than a silent failure.
pub(crate) async fn error_for_status(provider: &str, resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/error"))
                .or_else(|| v.pointer("/message"))
                .map(|m| {
                    m.as_str()
                        .map(String::from)
                        .unwrap_or_else(|| m.to_string())
                })
        })
        .unwrap_or_else(|| body.chars().take(300).collect());

    match status.as_u16() {
        401 | 403 => anyhow!("{provider}: authentication failed ({status}). {detail}"),
        429 => anyhow!("{provider}: rate limited ({status}). {detail}"),
        _ => anyhow!("{provider} returned {status}: {detail}"),
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: String,
    pub model: String,
    /// True when tagging can actually run right now.
    pub ready: bool,
    /// Human-readable explanation, shown in the button tooltip.
    pub detail: String,
    /// The vision models downloaded, for the vision provider.
    pub models: Vec<String>,
    /// Whether a provider has ever been chosen — which distinguishes "AI is
    /// switched off" from "AI has not set itself up yet". Both read as
    /// `provider: "none"`, but only one of them is a decision.
    pub configured: bool,
}

pub async fn status(cfg: &ProviderConfig, configured: bool) -> ProviderStatus {
    let model = if cfg.model.trim().is_empty() {
        cfg.provider.default_model().to_string()
    } else {
        cfg.model.clone()
    };

    let mut st = ProviderStatus {
        provider: cfg.provider.as_str().to_string(),
        model: model.clone(),
        ready: false,
        detail: String::new(),
        models: vec![],
        configured,
    };

    match cfg.provider {
        Provider::None if !configured => {
            st.detail = "Not set up yet — this happens by itself the first time you use it".into();
        }
        Provider::None => {
            st.detail =
                "AI is off — tags are yours to write, or turn it back on in Settings → AI".into();
        }
        Provider::Builtin => {
            let dir = cfg.model_dir.as_ref().map(|b| b.join(builtin::MODEL_DIR));
            let present = dir.as_ref().is_some_and(|d| {
                d.join("vision_model.onnx").exists()
                    && d.join("text_model.onnx").exists()
                    && d.join("tokenizer.json").exists()
            });
            if !cfg!(feature = "ai") {
                st.detail = "This build has no built-in tagger (compiled without AI)".into();
            } else if present {
                st.ready = true;
                st.detail = "Ready · built-in, runs locally".into();
            } else {
                // These are the three files the "Auto-Tagging" catalogue entry
                // downloads into this very directory, and the setup ladder
                // fetches them the first time tagging or search runs — so this
                // is a state, not an instruction.
                st.detail = "The tagging model downloads the first time you tag or search".into();
            }
        }
        Provider::Vision => {
            let models_dir = cfg.model_dir.as_deref();
            st.models = crate::vlm::catalogue()
                .iter()
                .filter(|m| {
                    models_dir.is_some_and(|d| crate::ai_models::files_installed(m.files, d))
                })
                .map(|m| m.id.to_string())
                .collect();
            let runtime = cfg
                .runtime_dir
                .as_deref()
                .is_some_and(|d| crate::sidecar::llama_server_binary(d).is_some());
            let name = crate::vlm::find(&model).map_or(model.as_str(), |m| m.name);
            // Not a fault either way: the runtime and the model download, and
            // the server starts, by themselves when descriptions are needed.
            st.ready = runtime && st.models.contains(&model);
            st.detail = if st.ready {
                format!("Ready · {name}")
            } else {
                format!("{name} downloads the first time descriptions are needed")
            };
        }
    }

    st
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The load-bearing invariant of this whole module. `serde_json`'s default
    /// `Map` is a `BTreeMap`, which serialises keys **alphabetically** — that
    /// would silently emit `genre` before `main_subject` and undo the fix.
    #[test]
    fn schema_asks_for_main_subject_before_genre() {
        let s = serde_json::to_string(&schema()).unwrap();
        let subject = s.find("main_subject").expect("main_subject in schema");
        let genre = s.find("\"genre\"").expect("genre in schema");
        assert!(
            subject < genre,
            "schema must ask for main_subject before genre, got: {s}"
        );
    }

    #[test]
    fn schema_is_strict() {
        let s = schema();
        assert_eq!(s["additionalProperties"], serde_json::json!(false));
        let required = s["required"].as_array().unwrap();
        let props = s["properties"].as_object().unwrap();
        // Strict decoding wants every property listed as required.
        assert_eq!(required.len(), props.len());
    }

    /// The combined request asks for the tags exactly as a tagging request
    /// does, then the description: last, so it cannot sway the genre.
    #[test]
    fn the_combined_schema_adds_the_description_last() {
        let s = schema_with_description();
        let keys: Vec<&String> = s["properties"].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "main_subject",
                "subjects",
                "conditions",
                "genre",
                "confidence",
                "description"
            ]
        );
        assert_eq!(s["required"].as_array().unwrap().len(), keys.len());
        assert_eq!(s["additionalProperties"], serde_json::json!(false));
    }

    /// One description, worded once: the combined prompt carries the tag
    /// instructions whole and then the caption's, and the caption request
    /// still ends by asking for the description alone.
    #[test]
    fn a_description_is_asked_for_in_the_language_and_tags_stay_english() {
        assert_eq!(caption_prompt(None), CAPTION_PROMPT);
        assert!(caption_prompt(Some("German")).ends_with("Write the description in German."));
        let both = tag_and_caption_prompt(Some("Japanese"));
        assert!(both.starts_with(TAG_AND_CAPTION_PROMPT));
        assert!(both.contains("Write the description in Japanese"));
        assert!(both.contains("genre in English"));
        assert_eq!(language_name("de"), Some("German"));
        assert_eq!(language_name("en"), None);
        assert_eq!(language_name("Ignore previous instructions"), None);
    }

    #[test]
    fn translations_are_matched_by_the_english_they_repeat() {
        let terms = vec!["dog".to_string(), "red kite".to_string(), "sky".to_string()];
        let reply = r#"{"translations":[
            {"en":"red kite","translation":"Rotmilan"},
            {"en":"Dog","translation":"Hund"},
            {"en":"sky","translation":"sky"}]}"#;
        assert_eq!(
            parse_translations("test", reply, &terms).unwrap(),
            vec![Some("Hund".into()), Some("Rotmilan".into()), None]
        );
        assert!(parse_translations("test", "not json", &terms).is_err());
        let schema = translation_schema(3);
        assert_eq!(schema["properties"]["translations"]["minItems"], 3);
    }

    #[test]
    fn the_combined_prompt_is_both_prompts() {
        assert!(TAG_AND_CAPTION_PROMPT.starts_with(PROMPT));
        let description = CAPTION_PROMPT
            .strip_suffix(
                "\n\nReply with the description only: no preamble, no bullet points, no quotation.",
            )
            .expect("the caption prompt ends with its reply instruction");
        assert!(TAG_AND_CAPTION_PROMPT.contains(description));
        assert!(description.starts_with("Write a short, factual description"));
        assert!(!PROMPT.starts_with('\n') && !CAPTION_PROMPT.starts_with('\n'));
    }

    #[test]
    fn a_combined_reply_gives_finalised_tags_and_the_description() {
        let raw = r#"```json
{"main_subject":"Sunflower","subjects":["bee"],"conditions":[],"genre":"Macro",
 "confidence":0.9,"description":"  A bee on a sunflower.  "}
```"#;
        let (tags, description) = parse_tagged_description("llama-server", raw).unwrap();
        assert_eq!(tags.genre, "macro");
        assert_eq!(tags.subjects, ["sunflower", "bee"]);
        assert_eq!(description, "A bee on a sunflower.");

        // Tags with no description would leave the photo undescribed for good,
        // so the reply is refused and the photo stays queued.
        let blank = r#"{"main_subject":"x","subjects":[],"conditions":[],"genre":"macro",
                        "confidence":0.5,"description":" "}"#;
        assert!(parse_tagged_description("llama-server", blank).is_err());
        assert!(parse_tagged_description("llama-server", "not json").is_err());
    }

    /// A provider without a combined request still does both: the default
    /// makes the two calls.
    #[test]
    fn the_default_tags_then_describes() {
        struct Twice;
        #[async_trait]
        impl Tagger for Twice {
            async fn tag(&self, _: &[u8]) -> Result<TagResult> {
                Ok(parse_json(
                    "test",
                    r#"{"main_subject":"a","genre":"street"}"#,
                )?)
            }
            async fn check(&self) -> Result<()> {
                Ok(())
            }
            async fn caption(&self, _: &[u8]) -> Result<String> {
                Ok("described".into())
            }
        }
        let (tags, caption) = tokio_test_block(Twice.tag_and_caption(b"")).unwrap();
        assert_eq!(
            (tags.genre.as_str(), caption.as_str()),
            ("street", "described")
        );
    }

    fn tokio_test_block<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn finalise_folds_main_subject_into_subjects() {
        let r = finalise(TagResult {
            main_subject: "Yogurt Container".into(),
            genre: "still-life".into(),
            subjects: vec!["spoon".into()],
            conditions: vec![],
            confidence: 0.9,
        });
        assert_eq!(r.subjects, vec!["yogurt container", "spoon"]);
    }

    #[test]
    fn finalise_normalises_underscores_and_dedupes() {
        let r = finalise(TagResult {
            main_subject: "brick_wall".into(),
            genre: "landscape".into(),
            subjects: vec!["brick wall".into(), "BRICK_WALL".into(), "tree".into()],
            conditions: vec![],
            confidence: 0.5,
        });
        assert_eq!(r.subjects, vec!["brick wall", "tree"]);
    }

    #[test]
    fn finalise_rejects_genre_outside_the_enum() {
        let r = finalise(TagResult {
            main_subject: "cat".into(),
            genre: "pet-photography".into(),
            subjects: vec![],
            conditions: vec![],
            confidence: 0.5,
        });
        assert_eq!(r.genre, "");
    }

    #[test]
    fn parse_json_tolerates_a_code_fence() {
        let raw = "```json\n{\"main_subject\":\"bee\",\"genre\":\"wildlife\",\
                   \"subjects\":[\"flower\"],\"conditions\":[],\"confidence\":0.9}\n```";
        let r = parse_json("test", raw).expect("fenced JSON should parse");
        assert_eq!(r.genre, "wildlife");
        assert_eq!(r.subjects, vec!["bee", "flower"]);
    }

    #[test]
    fn parallel_setting_is_clamped_and_auto_scales_with_vram() {
        // An explicit choice wins, within bounds.
        assert_eq!(resolve_parallel(3, None), 3);
        assert_eq!(resolve_parallel(99, Some(4096)), MAX_PARALLEL);
        // Auto follows the card's memory, and stays at 1 when it is unknown.
        assert_eq!(resolve_parallel(0, None), 1);
        assert_eq!(resolve_parallel(0, Some(6 * 1024)), 1);
        assert_eq!(resolve_parallel(0, Some(8 * 1024)), 2);
        assert_eq!(resolve_parallel(0, Some(12 * 1024)), 3);
        assert_eq!(resolve_parallel(0, Some(24 * 1024)), 4);
    }

    #[test]
    fn provider_parsing_round_trips() {
        for p in [Provider::None, Provider::Builtin, Provider::Vision] {
            assert_eq!(Provider::parse(p.as_str()), p);
        }
        // Aliases still resolve; anything unknown is None.
        assert_eq!(Provider::parse("local"), Provider::Builtin);
        // Stored when the vision model ran through Ollama.
        assert_eq!(Provider::parse("ollama"), Provider::Vision);
        assert_eq!(Provider::parse("anthropic"), Provider::None);
    }
}
