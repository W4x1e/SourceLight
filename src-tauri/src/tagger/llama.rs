//! The vision model on the app's own `llama-server`, over its
//! OpenAI-compatible API.
//!
//! This replaced a managed Ollama. Ollama ran these same models on
//! the same `llama-server`, but its API fixed the two things that decide how
//! long a photo takes:
//!
//! - **What an image costs.** Ollama started Qwen3-VL with
//!   `--image-min-tokens 1024`, so even a small tag image was read as ~1,000
//!   tokens, and reading the image is most of a request. Here the image's own
//!   size decides it (see `sidecar::llama_server_args`): tags are asked of a
//!   640 px image, ~260 tokens, and descriptions of a 1280 px one, because
//!   reading text in the frame needs the detail — at 256–384 tokens the model
//!   *invented* sign text rather than missing it.
//! - **Parallel requests,** which Ollama refused for the Qwen3-VL family.
//!
//! Measured with Qwen3-VL 4B: on an RTX 5070 Ti, tags in 0.23–0.30 s a photo
//! where Ollama took 0.71 s; on a 16 GB M4, 2.5 s where it took 8.2 s.
//!
//! The instructions come before the image in each request, so a slot reads
//! them once and reuses them for every photo after (~460 tokens).
//!
//! The server itself — starting it on the model's files, stopping it when
//! idle — is managed in `commands/ai.rs`; this is only the client.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use super::{
    caption_prompt, error_for_status, parse_json, parse_tagged_description, parse_translations,
    schema, schema_with_description, tag_and_caption_prompt, translation_prompt,
    translation_schema, translation_tokens, Tagger, PROMPT,
};
use crate::models::TagResult;

/// Longest edge of the image sent for tags: ~260 image tokens. Genre, subject
/// and objects need no more — they agreed with full-size tags on 13 of 16
/// photos, and the rest were photos two genres fit equally well.
const TAG_EDGE: u32 = 640;

/// Longest edge for descriptions, which also transcribe visible text, so they
/// keep more detail than tagging does: ~1,080 image tokens.
const CAPTION_EDGE: u32 = 1280;

/// Ceilings on a runaway reply; tags are ~50 tokens, tags and a description
/// ~150.
const TAG_MAX_TOKENS: u32 = 256;
const DESCRIBE_MAX_TOKENS: u32 = 512;

/// The repetition penalty on descriptions: Ollama's own default, which the
/// app had without asking for it until the server was run directly.
/// Without it Qwen3-VL 2B looped on 9 of 194 descriptions, the same sentences
/// over and over to the token limit; with it, on none of the nine.
const REPEAT_PENALTY: f64 = 1.1;
const REPEAT_LAST_N: u32 = 64;

/// The most items a list in the tags may hold: the prompt asks for 3–8
/// subjects. The grammar ends a list there, so a model stuck repeating one
/// word ("rock", "rock", …) closes it instead of running out of tokens with
/// the JSON unfinished — which the 2B models did, on about 1 photo in 70. The
/// cap cost nothing measurable: the 2B's tags were the same with and without
/// it (genre 0.89, 5.9 subjects a photo).
const MAX_LIST_ITEMS: usize = 8;

pub struct LlamaServerTagger {
    host: String,
    parallel: usize,
    /// The language descriptions are written in; `None` is English.
    language: Option<&'static str>,
    client: reqwest::Client,
}

impl LlamaServerTagger {
    pub fn new(parallel: usize) -> Self {
        Self {
            host: crate::sidecar::link().url(""),
            parallel: crate::sidecar::llama_server_slots(parallel),
            language: None,
            // A request can wait behind the other slots' photos. Never
            // through a proxy.
            client: crate::sidecar::loopback_client(Duration::from_secs(300)),
        }
    }

    /// Writes descriptions in `language` (see `tagger::language_name`).
    pub fn with_language(mut self, language: Option<&'static str>) -> Self {
        self.language = language;
        self
    }

    /// One chat request: the instructions, then the image, so the slot's
    /// cached instructions are a prefix of the next request too.
    async fn chat(&self, body: Value) -> Result<String> {
        // The model runs from the send to the reply: the app's own GPU models
        // wait meanwhile (`gpu_gate`).
        let _gpu = crate::gpu_gate::GATE.vision().await;
        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.host))
            .bearer_auth(&crate::sidecar::link().key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(error_for_status("llama-server", resp).await);
        }
        let reply: ChatCompletion = resp.json().await?;
        reply
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow!("llama-server returned no reply"))
    }
}

#[derive(Deserialize)]
struct ChatCompletion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    content: String,
}

/// Shrinks an image so its longest edge is at most `max_edge`, re-encoded as
/// JPEG. Returns the input untouched when it is already small enough or can't
/// be decoded — the model then sees what it always did rather than nothing.
pub fn prepare_image(image: &[u8], max_edge: u32) -> Vec<u8> {
    let Ok(decoded) = image::load_from_memory(image) else {
        return image.to_vec();
    };
    if decoded.width().max(decoded.height()) <= max_edge {
        return image.to_vec();
    }
    let small = decoded
        .resize(max_edge, max_edge, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
    match small.write_with_encoder(encoder) {
        Ok(()) => out,
        Err(_) => image.to_vec(),
    }
}

/// [`prepare_image`] off the async reactor, base64-encoded for the request.
/// Decoding and resizing a 1600px JPEG is tens of milliseconds of CPU, and
/// several of these run at once.
async fn prepared_b64(image: &[u8], max_edge: u32) -> String {
    let owned = image.to_vec();
    let bytes = tokio::task::spawn_blocking(move || prepare_image(&owned, max_edge))
        .await
        .unwrap_or_else(|_| image.to_vec());
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The request body: `prompt` then the image, and `options` (sampling, the
/// grammar) on top. No image for a request about words alone.
fn request(prompt: &str, image_b64: Option<&str>, options: Value) -> Value {
    let mut content = vec![json!({ "type": "text", "text": prompt })];
    if let Some(image_b64) = image_b64 {
        content.push(json!({ "type": "image_url",
            "image_url": { "url": format!("data:image/jpeg;base64,{image_b64}") } }));
    }
    let mut body = json!({ "messages": [{ "role": "user", "content": content }] });
    if let (Some(body), Some(options)) = (body.as_object_mut(), options.as_object()) {
        body.extend(options.clone());
    }
    body
}

/// A reply constrained to `schema` by the server's own schema-to-grammar: for
/// the tag translations, whose nested list [`tag_grammar`] doesn't cover.
fn json_schema(schema: Value) -> Value {
    json!({ "type": "json_schema", "json_schema": { "schema": schema } })
}

/// The grammar the tags are written in: the JSON of `schema` — built from
/// [`super::schema`] or [`super::schema_with_description`], so the schema stays
/// the one place that says which fields there are and in what order — as
/// llama-server would build it from the schema, except that **lists hold at
/// most [`MAX_LIST_ITEMS`]**.
///
/// The whitespace is llama-server's own: the model may pretty-print. Keeping
/// the JSON on one line saved ~10 tokens a reply (tags 0.30 → 0.26 s a photo
/// on an RTX 5070 Ti, nothing on an M4), but Qwen3-VL 2B then listed fewer
/// subjects: two or fewer on 18 of 71 photos instead of 7, and genre 0.83
/// instead of 0.89. Allowing *no* whitespace at all breaks the model,
/// which then writes `":",":",…`.
///
/// Main subject still comes before genre: that order is load-bearing (see the
/// module docs of `tagger`).
fn tag_grammar(schema: &Value) -> String {
    let mut rules = Vec::new();
    let fields: Vec<String> = schema["properties"]
        .as_object()
        .map(|props| {
            props
                .iter()
                .map(|(name, spec)| {
                    let value = match spec["enum"].as_array() {
                        Some(values) => {
                            let choices = values
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|v| format!(r#""\"{v}\"""#))
                                .collect::<Vec<_>>()
                                .join(" | ");
                            rules.push(format!("{name}-value ::= {choices}"));
                            format!("{name}-value")
                        }
                        None => match spec["type"].as_str() {
                            Some("array") => "list".to_string(),
                            Some("number") => "number".to_string(),
                            _ => "string".to_string(),
                        },
                    };
                    format!(r#""\"{name}\"" space ":" space {value}"#)
                })
                .collect()
        })
        .unwrap_or_default();
    let more = MAX_LIST_ITEMS - 1;
    let mut grammar = vec![format!(
        r#"root ::= "{{" space {} space "}}""#,
        fields.join(r#" "," space "#)
    )];
    grammar.extend(rules);
    grammar.extend([
        format!(r#"list ::= "[" space (string ("," space string){{0,{more}}})? space "]""#),
        r#"number ::= "-"? ([0] | [1-9] [0-9]{0,15}) ("." [0-9]{1,16})? ([eE] [-+]? [0-9]{1,16})?"#
            .to_string(),
        r#"string ::= "\"" char* "\"""#.to_string(),
        r#"char ::= [^"\\\x7F\x00-\x1F] | [\\] (["\\bfnrt] | "u" [0-9a-fA-F]{4})"#.to_string(),
        r#"space ::= | " " | "\n"{1,2} [ \t]{0,20}"#.to_string(),
    ]);
    grammar.join("\n") + "\n"
}

/// Sampling for tags: greedy, since this is classification and re-running the
/// batch should not quietly reshuffle genres.
fn tag_options() -> Value {
    json!({
        "temperature": 0.0,
        "max_tokens": TAG_MAX_TOKENS,
        "grammar": tag_grammar(&schema()),
    })
}

/// Sampling for a description: a little warmth reads more naturally, and the
/// repetition penalty stops the small model looping.
fn caption_options() -> Value {
    json!({
        "temperature": 0.2,
        "max_tokens": DESCRIBE_MAX_TOKENS,
        "repeat_penalty": REPEAT_PENALTY,
        "repeat_last_n": REPEAT_LAST_N,
    })
}

/// Tags and a description in one reply: greedy, as tags are, with the
/// description's repetition penalty.
fn tag_and_caption_options() -> Value {
    json!({
        "temperature": 0.0,
        "max_tokens": DESCRIBE_MAX_TOKENS,
        "repeat_penalty": REPEAT_PENALTY,
        "repeat_last_n": REPEAT_LAST_N,
        "grammar": tag_grammar(&schema_with_description()),
    })
}

#[async_trait]
impl Tagger for LlamaServerTagger {
    async fn tag(&self, image: &[u8]) -> Result<TagResult> {
        let image = prepared_b64(image, TAG_EDGE).await;
        let reply = self
            .chat(request(PROMPT, Some(&image), tag_options()))
            .await?;
        parse_json("llama-server", &reply)
    }

    async fn caption(&self, image: &[u8]) -> Result<String> {
        let image = prepared_b64(image, CAPTION_EDGE).await;
        let reply = self
            .chat(request(
                &caption_prompt(self.language),
                Some(&image),
                caption_options(),
            ))
            .await?;
        let caption = reply.trim().to_string();
        if caption.is_empty() {
            return Err(anyhow!("the model returned an empty caption"));
        }
        Ok(caption)
    }

    async fn tag_and_caption(&self, image: &[u8]) -> Result<(TagResult, String)> {
        // Full size: the description has to read text in the frame.
        let image = prepared_b64(image, CAPTION_EDGE).await;
        let reply = self
            .chat(request(
                &tag_and_caption_prompt(self.language),
                Some(&image),
                tag_and_caption_options(),
            ))
            .await?;
        parse_tagged_description("llama-server", &reply)
    }

    async fn translate_terms(
        &self,
        terms: &[String],
        language: &str,
    ) -> Result<Vec<Option<String>>> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let reply = self
            .chat(request(
                &translation_prompt(terms, language),
                None,
                json!({
                    "temperature": 0.0,
                    "max_tokens": translation_tokens(terms.len()),
                    "response_format": json_schema(translation_schema(terms.len()))
                }),
            ))
            .await?;
        parse_translations("llama-server", &reply, terms)
    }

    fn max_concurrency(&self) -> usize {
        self.parallel
    }

    async fn check(&self) -> Result<()> {
        let resp = self
            .client
            .get(format!("{}/health", self.host))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map_err(|_| anyhow!("the vision model is not loaded"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(anyhow!("the vision model is still loading"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([90, 140, 200]));
        let mut out = Vec::new();
        img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new(&mut out))
            .unwrap();
        out
    }

    #[test]
    fn prepare_image_caps_the_long_edge_and_keeps_the_aspect() {
        let small = prepare_image(&jpeg(1600, 1000), 800);
        let d = image::load_from_memory(&small).unwrap();
        assert_eq!((d.width(), d.height()), (800, 500));
    }

    #[test]
    fn prepare_image_leaves_small_or_undecodable_input_alone() {
        let already = jpeg(640, 480);
        assert_eq!(prepare_image(&already, 896), already);
        assert_eq!(
            prepare_image(b"not an image", 896),
            b"not an image".to_vec()
        );
    }

    #[test]
    fn the_instructions_come_before_the_image() {
        let body = request("Tag it.", Some("AAAA"), tag_options());
        let content = &body["messages"][0]["content"];
        assert_eq!(content[0]["text"], "Tag it.");
        assert_eq!(
            content[1]["image_url"]["url"],
            "data:image/jpeg;base64,AAAA"
        );
        assert_eq!(body["temperature"], 0.0);
    }

    /// Descriptions carry Ollama's repetition penalty, which the 2B needs not
    /// to loop; tags are held by their grammar instead.
    #[test]
    fn descriptions_are_penalised_for_repeating() {
        for options in [caption_options(), tag_and_caption_options()] {
            assert_eq!(options["repeat_penalty"], REPEAT_PENALTY);
            assert_eq!(options["repeat_last_n"], REPEAT_LAST_N);
        }
        assert!(tag_options().get("repeat_penalty").is_none());
        assert!(caption_options().get("grammar").is_none());
    }

    /// The grammar keeps the schema's order — main subject before genre — and
    /// every genre, puts the description last, and bounds the lists.
    #[test]
    fn the_grammar_follows_the_schema_and_bounds_the_lists() {
        let g = tag_grammar(&schema());
        let root = g.lines().next().unwrap();
        let order = [
            "main_subject",
            "subjects",
            "conditions",
            "genre",
            "confidence",
        ];
        let at: Vec<usize> = order.iter().map(|k| root.find(k).unwrap()).collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{root}");
        assert!(!root.contains("description"));
        for genre in crate::models::GENRES {
            assert!(g.contains(&format!(r#""\"{genre}\"""#)), "{genre}");
        }
        assert!(g.contains(&format!("{{0,{}}}", MAX_LIST_ITEMS - 1)));

        let with = tag_grammar(&schema_with_description());
        let root = with.lines().next().unwrap();
        assert!(root.find("confidence").unwrap() < root.find("description").unwrap());
    }

    /// The grammar the llama-server spike measured (`scripts/model-eval/
    /// vlm_serving.py`, `--grammar capped-pretty`): the same rules.
    #[test]
    fn the_grammar_is_the_one_measured() {
        let g = tag_grammar(&schema_with_description());
        for rule in [
            r#"list ::= "[" space (string ("," space string){0,7})? space "]""#,
            r#"string ::= "\"" char* "\"""#,
            r#"char ::= [^"\\\x7F\x00-\x1F] | [\\] (["\\bfnrt] | "u" [0-9a-fA-F]{4})"#,
            r#"space ::= | " " | "\n"{1,2} [ \t]{0,20}"#,
            r#"genre-value ::= "\"landscape\"" | "\"wildlife\"" | "\"portrait\"" | "\"street\"" | "\"macro\"" | "\"astro\"" | "\"architecture\"" | "\"sports\"" | "\"event\"" | "\"still-life\"" | "\"abstract\"""#,
            r#"root ::= "{" space "\"main_subject\"" space ":" space string "," space "\"subjects\"" space ":" space list "," space "\"conditions\"" space ":" space list "," space "\"genre\"" space ":" space genre-value "," space "\"confidence\"" space ":" space number "," space "\"description\"" space ":" space string space "}""#,
        ] {
            assert!(g.lines().any(|l| l == rule), "missing {rule}\nin\n{g}");
        }
    }

    #[test]
    fn it_runs_as_many_photos_as_the_server_has_slots() {
        assert_eq!(
            LlamaServerTagger::new(4).max_concurrency(),
            crate::sidecar::llama_server_slots(4)
        );
        assert_eq!(LlamaServerTagger::new(0).max_concurrency(), 1);
    }
}
