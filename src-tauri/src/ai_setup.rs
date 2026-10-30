//! Automatic AI provisioning — the ladder that turns "I want to tag these
//! photos" into a loaded model, without making the user an administrator of it.
//!
//! Before this, reaching the first tagged photo meant: choose a provider from a
//! dropdown, optionally type a server URL, press Start server, pick a model
//! from a fit-graded table, press Download, then go back and press Tag. Six
//! decisions inside a Settings modal — and not one of them has an answer the
//! app doesn't already know. It can see whether the runtime and the model are
//! downloaded, how much VRAM the machine has, and which model that suits.
//!
//! So the ladder runs on demand instead, from whatever the user actually
//! clicked. Every rung checks before it acts, so calling it in front of every AI
//! action costs nothing once things are in place. A rung that needs a download
//! stops and reports [`Readiness::needs`], and [`Readiness::ask_first`] says
//! whether the caller should ask or just carry on: see [`ask_first`] for the
//! rule. A yes to the vision model is stored, so that is asked once *ever*.
//!
//! The rungs themselves are I/O — spawning a server, streaming weights — and
//! live in `commands/ai.rs` with the other AI commands. What lives here is the vocabulary
//! and the decisions, which are pure and tested.

use crate::tagger::Provider;

/// Settings key recording that the user has agreed, once, to the app
/// downloading the vision model and its runtime. The on-device models do not
/// need it (see [`ask_first`]). A key stored by an older build, when it
/// covered those too, means the same thing here.
pub const CONSENT_KEY: &str = "ai_setup_consented";

/// What the user is trying to do.
///
/// Features need different models, and one a provider genuinely cannot serve
/// should say so up front rather than fail on photo 1 of 5000 — the built-in
/// CLIP tagger has no `caption`, so asking it for captions is a dead end
/// however much gets downloaded first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AiPurpose {
    /// Genre and subject tags. Either provider can do this.
    Tagging,
    /// Free-prose descriptions for text search. Needs a vision-language model.
    Captions,
    /// CLIP embeddings for "looks like" search.
    Search,
    /// Face detection and eye state, for culling.
    Faces,
    /// Recognising people: detection plus identity embeddings, for People.
    People,
    /// Everything automatic analysis uses — search and tags, face detection
    /// and People — so a new library's first import is one download, not one
    /// per feature met afterwards.
    Analysis,
    /// One-click subject cut-out in Develop.
    SelectSubject,
    /// Point-and-click interactive masking in Develop.
    ClickSelect,
    /// AI noise reduction in Develop.
    Denoise,
    /// Magic erase in Develop: an object taken out and the gap filled.
    Erase,
}

impl AiPurpose {
    /// Whether this purpose needs a vision-language model behind a server,
    /// rather than one of the small on-device ONNX models.
    pub fn needs_vlm(self) -> bool {
        matches!(self, Self::Tagging | Self::Captions)
    }

    /// The on-device catalogue ids this purpose needs; empty when it needs a
    /// provider instead. Tagging maps to one only under the built-in provider,
    /// and that case is resolved by the caller, which knows the provider.
    pub fn onnx_models(self) -> &'static [&'static str] {
        match self {
            Self::Search => &["tagger"],
            // Culling asks only for the small detection models; People adds
            // the 261 MB recogniser.
            Self::Faces => &["faces"],
            Self::People => &["faces", "people"],
            // Not the recogniser: recognising who is in photos waits for a
            // yes in People. Detection, for culling, does not.
            Self::Analysis => &["tagger", "faces"],
            // Two separate models, and deliberately two separate purposes:
            // asking someone to download 224 MB of BiRefNet because they
            // clicked the SAM 2 tool would be a bait-and-switch.
            Self::SelectSubject => &["subject"],
            Self::ClickSelect => &["click"],
            Self::Denoise => &["denoise"],
            // Selecting what to erase by clicking asks for SAM 2 itself, as
            // the masking tool does; brushing needs no model.
            Self::Erase => &["erase"],
            _ => &[],
        }
    }

    /// How the purpose reads in a sentence.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tagging => "tagging",
            Self::Captions => "captions",
            Self::Search => "photo search",
            Self::Faces => "face detection",
            Self::People => "recognising people",
            Self::Analysis => "photo search and people",
            Self::SelectSubject => "subject selection",
            Self::ClickSelect => "click-to-select",
            Self::Denoise => "AI noise reduction",
            Self::Erase => "magic erase",
        }
    }
}

/// The provider a fresh install uses the first time an AI feature runs.
///
/// Always the built-in tagger. It used to be the vision model whenever
/// Ollama was installed or the GPU had 6 GB, which made the first press of Tag
/// fetch a ~1.4 GB runtime plus a multi-gigabyte vision model — on top of the
/// model that search needs regardless. The search model tags well on its
/// own, so the first AI download is one model that serves search,
/// tags, Similar and duplicates. The vision model is now something the user
/// asks for — "Detailed descriptions" in Settings → AI — never a default.
pub const DEFAULT_PROVIDER: Provider = Provider::Builtin;

/// Whether a download should wait for a yes rather than just start.
///
/// The vision model always asks, the first time: it is gigabytes, and the user
/// turned on a feature, not a download. The on-device models ask only on a
/// connection that may cost money. Everywhere else a dialog before a few
/// hundred megabytes the user needs for what they just clicked was friction
/// with one sensible answer, so they start, with progress and Cancel in
/// Activity. `metered` is [`crate::hardware::metered`]; unknown counts as
/// metered.
pub fn ask_first(on_device: bool, metered: Option<bool>) -> bool {
    !on_device || metered != Some(false)
}

/// One thing that must be fetched before a purpose can run, as the consent
/// dialog lists it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    /// A plain line for the dialog: "the local AI runtime".
    pub label: String,
    /// Rough size, so the total is known before agreeing.
    pub approx_mb: u32,
}

impl Requirement {
    pub fn new(label: impl Into<String>, approx_mb: u32) -> Self {
        Self {
            label: label.into(),
            approx_mb,
        }
    }
}

/// The answer to "can this run now, and if not, what would it take?".
///
/// Deliberately one flat struct rather than an enum: the same value both gates
/// an action and renders the AI status card, so a caller never needs two round
/// trips to find out what it is looking at.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    /// The purpose can run right now.
    pub ready: bool,
    /// Non-empty when a one-time download stands in the way. Call again with
    /// `consented` to fetch it — after asking, if [`Self::ask_first`] says so.
    pub needs: Vec<Requirement>,
    /// Whether to ask before fetching `needs`, from [`ask_first`].
    pub ask_first: bool,
    /// The connection is known to be metered, for the question's wording.
    pub metered: bool,
    /// Set when no amount of downloading would help — a build compiled without
    /// the `ai` feature, or a purpose this provider cannot serve.
    pub blocked: Option<String>,
    /// The provider in force after the ladder ran (it may have chosen one).
    pub provider: String,
    /// The model in force, for the status line.
    pub model: String,
    /// One human-readable sentence about the current state.
    pub detail: String,
}

impl Readiness {
    pub fn ready(provider: &str, model: &str, detail: impl Into<String>) -> Self {
        Self {
            ready: true,
            needs: vec![],
            ask_first: false,
            metered: false,
            blocked: None,
            provider: provider.into(),
            model: model.into(),
            detail: detail.into(),
        }
    }

    /// Downloads stand in the way; `on_device` and `metered` decide whether
    /// the caller asks first (see [`ask_first`]).
    pub fn needs(
        provider: &str,
        model: &str,
        needs: Vec<Requirement>,
        on_device: bool,
        metered: Option<bool>,
    ) -> Self {
        Self {
            ready: false,
            needs,
            ask_first: ask_first(on_device, metered),
            metered: metered == Some(true),
            blocked: None,
            provider: provider.into(),
            model: model.into(),
            detail: String::new(),
        }
    }

    pub fn blocked(provider: &str, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            ready: false,
            needs: vec![],
            ask_first: false,
            metered: false,
            blocked: Some(reason.clone()),
            provider: provider.into(),
            model: String::new(),
            detail: reason,
        }
    }

    /// Total download the outstanding requirements represent.
    pub fn approx_mb(&self) -> u32 {
        self.needs.iter().map(|r| r.approx_mb).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_vlm_purposes_need_a_server() {
        assert!(AiPurpose::Tagging.needs_vlm());
        assert!(AiPurpose::Captions.needs_vlm());
        assert!(!AiPurpose::Search.needs_vlm());
        assert!(!AiPurpose::Faces.needs_vlm());
        assert!(!AiPurpose::People.needs_vlm());
        assert!(!AiPurpose::Analysis.needs_vlm());
        assert!(!AiPurpose::SelectSubject.needs_vlm());
        assert!(!AiPurpose::ClickSelect.needs_vlm());
    }

    #[test]
    fn onnx_purposes_name_a_catalogue_entry() {
        assert_eq!(AiPurpose::Search.onnx_models(), ["tagger"]);
        assert_eq!(AiPurpose::Captions.onnx_models(), [] as [&str; 0]);
        // Culling must not fetch the recogniser; People needs both.
        assert_eq!(AiPurpose::Faces.onnx_models(), ["faces"]);
        assert_eq!(AiPurpose::People.onnx_models(), ["faces", "people"]);
        // A first import sets up everything analysis runs, in one go — but
        // not the recogniser, which People asks for.
        assert_eq!(AiPurpose::Analysis.onnx_models(), ["tagger", "faces"]);
        // The two masking tools are separate purposes on purpose: one is a
        // 224 MB download and the other 180 MB, and clicking one tool must not
        // fetch the other's model.
        assert_eq!(AiPurpose::SelectSubject.onnx_models(), ["subject"]);
        assert_eq!(AiPurpose::ClickSelect.onnx_models(), ["click"]);
        assert_eq!(AiPurpose::Denoise.onnx_models(), ["denoise"]);
        assert_eq!(AiPurpose::Erase.onnx_models(), ["erase"]);
    }

    #[test]
    fn a_fresh_install_tags_with_the_built_in_model() {
        assert_eq!(DEFAULT_PROVIDER, Provider::Builtin);
    }

    #[test]
    fn consent_total_sums_every_outstanding_download() {
        let r = Readiness::needs(
            "vision",
            "qwen3-vl-4b",
            vec![
                Requirement::new("the local AI runtime", 32),
                Requirement::new("the Qwen3-VL 4B vision model", 3072),
            ],
            false,
            Some(false),
        );
        assert_eq!(r.approx_mb(), 3104);
        assert!(!r.ready && r.blocked.is_none());
        assert!(r.ask_first, "the vision model always asks");
    }

    #[test]
    fn only_on_device_models_on_an_unmetered_connection_skip_the_question() {
        assert!(!ask_first(true, Some(false)));
        // Metered, or unknown: ask.
        assert!(ask_first(true, Some(true)));
        assert!(ask_first(true, None));
        // The vision model always asks, whatever the connection.
        assert!(ask_first(false, Some(false)));
        assert!(ask_first(false, None));
        let r = Readiness::needs("local", "tagger", vec![], true, Some(true));
        assert!(r.ask_first && r.metered);
        let r = Readiness::needs("local", "tagger", vec![], true, None);
        assert!(
            r.ask_first && !r.metered,
            "unknown is asked about, not called metered"
        );
    }
}
