//! The vision-model catalogue and how each model fits a machine.
//!
//! Detailed descriptions (and the tags written with them) run a VLM on the
//! app's own `llama-server` (`sidecar`, `tagger::llama`); which one is
//! realistic depends almost entirely on GPU memory. This is the curated list of
//! models we recommend — each one's files pinned like every other model the app
//! downloads, and fetched by the same downloader (`ai_models`) — plus the pure
//! logic that turns a measured VRAM figure into a per-model verdict and a
//! single recommended pick, kept here and unit-tested rather than guessed in
//! the UI.
//!
//! The files are **Qwen's own GGUFs** (`Qwen/Qwen3-VL-*-Instruct-GGUF`): the
//! weights at Q4_K_M, and the image projector at Q8_0, which reads as well as
//! its F16 at half the size (genre 0.90 on the 71 labelled photos either
//! way). They replaced Ollama's registry, whose tags could change under the
//! app and whose files only Ollama's own llama.cpp could read.

use crate::ai_models::ModelFile;

/// One catalogue entry. `&'static str` throughout — this is fixed data, mapped
/// to an owned, TS-exported shape at the command layer.
pub struct VlmModel {
    /// Stable id, stored as the `ai_model` setting.
    pub id: &'static str,
    pub name: &'static str,
    pub params: &'static str,
    /// Download size, GB: the weights and the projector.
    pub size_gb: f32,
    /// VRAM (GB) to run comfortably on the GPU.
    pub min_vram_gb: u32,
    /// OCR / text-in-image quality — "excellent" | "good" | "basic".
    pub ocr: &'static str,
    /// Licence of the weights, as the model's publisher states it. Held to
    /// [`crate::ai_models::PERMISSIVE`] by a test: anyone may use the app for
    /// any purpose, and it picks and downloads these on the user's behalf.
    pub license: &'static str,
    pub note: &'static str,
    /// The weights, then the image projector, each pinned to a commit and a
    /// SHA-256. See [`weights`] and [`projector`].
    pub files: &'static [ModelFile],
}

impl VlmModel {
    /// Where the weights land in the models directory.
    pub fn weights(&self) -> &'static str {
        self.files[0].rel_path
    }

    /// Where the image projector lands.
    pub fn projector(&self) -> &'static str {
        self.files[1].rel_path
    }
}

/// How a model fits the detected hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    /// Comfortably within GPU memory.
    WillRun,
    /// Close to the limit — may spill to system RAM and slow down.
    Tight,
    /// More GPU memory than this machine has.
    TooLarge,
    /// VRAM couldn't be read, so we can't say (never block on this).
    Unknown,
}

/// The recommended models, smallest to largest.
///
/// **Licensing decides what is listed before quality does** (checked at
/// source 2026-09-24 and again 2026-09-29): Qwen3-VL 2B, 4B and 8B are
/// Apache-2.0. Qwen2.5-VL 3B (research licence) and MiniCPM-V (registration)
/// were listed once and are gone for good.
///
/// **Measured, not assumed** (`docs/AI-MODEL-SPIKES.md`): the 4B describes as
/// well as Qwen2.5-VL 7B did in far less memory; the 8B is the best describer
/// (it names landmarks and reads signs); the 2B is quick but wordy, and loops
/// without a repetition penalty. Qwen3.5's small models were measured too
/// and are no better, and slower.
pub fn catalogue() -> &'static [VlmModel] {
    &[
        VlmModel {
            id: "qwen3-vl-2b",
            name: "Qwen3-VL 2B",
            params: "2B",
            size_gb: 1.6,
            min_vram_gb: 4,
            ocr: "good",
            license: "Apache-2.0",
            note: "Small and quick, and still reads text in the frame. For GPUs with 4–6 GB.",
            files: &[
                ModelFile {
                    url: "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/52d6c8ffea26cc873ac5ad116f8631268d7eb503/Qwen3VL-2B-Instruct-Q4_K_M.gguf",
                    rel_path: "vision/qwen3-vl-2b/model.gguf",
                    sha256: "089d75c52f4b7ffc56ba998ffc50aae89fcafc755f9e7208aacca281dca6c2ae",
                },
                ModelFile {
                    url: "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF/resolve/52d6c8ffea26cc873ac5ad116f8631268d7eb503/mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf",
                    rel_path: "vision/qwen3-vl-2b/mmproj.gguf",
                    sha256: "f9a68fabba69c3b81e153367b2c7521030b0fa8bb0de400c9599c8e6725f9c82",
                },
            ],
        },
        VlmModel {
            id: "qwen3-vl-4b",
            name: "Qwen3-VL 4B",
            params: "4B",
            size_gb: 3.0,
            min_vram_gb: 6,
            ocr: "excellent",
            license: "Apache-2.0",
            note: "Strong descriptions and text reading in about 4 GB. The pick for most GPUs.",
            files: &[
                ModelFile {
                    url: "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/1cd86afb9a95c410a6038ab3b40d8b578c892266/Qwen3VL-4B-Instruct-Q4_K_M.gguf",
                    rel_path: "vision/qwen3-vl-4b/model.gguf",
                    sha256: "66358cb18bb6b3b1b6675aa412c7a88ef01d228f481184d13668e5201c730a0a",
                },
                ModelFile {
                    url: "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/1cd86afb9a95c410a6038ab3b40d8b578c892266/mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf",
                    rel_path: "vision/qwen3-vl-4b/mmproj.gguf",
                    sha256: "30ba2c7dd3127a4561b6cba9d13d0f711c91bdb38742e2f56d73c8cb596bd06d",
                },
            ],
        },
        VlmModel {
            id: "qwen3-vl-8b",
            name: "Qwen3-VL 8B",
            params: "8B",
            size_gb: 5.8,
            min_vram_gb: 10,
            ocr: "excellent",
            license: "Apache-2.0",
            note: "The best descriptions: names landmarks and reads signs. For GPUs with 10 GB or more.",
            files: &[
                ModelFile {
                    url: "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct-GGUF/resolve/f982a07559d4a2f6c8744d840bf6fccab30eea96/Qwen3VL-8B-Instruct-Q4_K_M.gguf",
                    rel_path: "vision/qwen3-vl-8b/model.gguf",
                    sha256: "67d1659bfe71b89d50b45a4ad1a9e5b997e5bb16ce5da66a6a6167abd569e9e2",
                },
                ModelFile {
                    url: "https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct-GGUF/resolve/f982a07559d4a2f6c8744d840bf6fccab30eea96/mmproj-Qwen3VL-8B-Instruct-Q8_0.gguf",
                    rel_path: "vision/qwen3-vl-8b/mmproj.gguf",
                    sha256: "c6ba85508d82f42590e6eb77d5340369ab6fecf107a7561d809523d8aa5f3bfd",
                },
            ],
        },
    ]
}

/// The catalogue entry for `id`.
pub fn find(id: &str) -> Option<&'static VlmModel> {
    catalogue().iter().find(|m| m.id == id)
}

/// Whether `id` is a model the app will pick or download. Anything else — a
/// tag typed by hand, an Ollama tag from an older version, or one removed from the
/// catalogue since it was stored — is treated as unset, so it is never
/// fetched on the user's behalf.
pub fn is_listed(id: &str) -> bool {
    find(id).is_some()
}

/// Verdict for a model needing `min_vram_gb` on a machine with `vram_mb` VRAM.
/// `None` VRAM is `Unknown`, never `TooLarge` — we don't block on a reading we
/// couldn't take.
pub fn fit(min_vram_gb: u32, vram_mb: Option<u32>) -> Fit {
    let Some(mb) = vram_mb else {
        return Fit::Unknown;
    };
    let gb = mb as f32 / 1024.0;
    let need = min_vram_gb as f32;
    if gb + 0.25 >= need {
        Fit::WillRun
    } else if gb >= need - 1.5 {
        Fit::Tight
    } else {
        Fit::TooLarge
    }
}

/// Rough download size of a catalogue model, in MB — what a consent prompt
/// quotes before a download starts. A model we don't list falls back to the
/// largest entry, so the figure shown is never wildly optimistic.
pub fn size_mb(id: &str) -> u32 {
    let gb = find(id).map(|m| m.size_gb).unwrap_or_else(|| {
        catalogue()
            .iter()
            .map(|m| m.size_gb)
            .fold(0.0_f32, f32::max)
    });
    (gb * 1024.0).round() as u32
}

/// The single model to recommend for a machine, by VRAM. Unknown VRAM gets a
/// safe middle pick rather than the largest, so we never steer someone toward a
/// model that won't load.
pub fn recommended(vram_mb: Option<u32>) -> &'static str {
    recommended_for(vram_mb, crate::hardware::directml_gpu())
}

/// [`recommended`], given whether the PC has graphics DirectML can use
/// (`None` where that is not the question: a Mac).
///
/// A Windows PC with none runs the model on its CPU, where the 2B tags a
/// photo in 4.7 s against the 4B's 7.2 (docs/AI-MODEL-SPIKES.md) and leaves
/// 1.4 GB more of the memory such a PC is often short of. Its graphics
/// memory reads as unknown, which on its own would pick the 4B.
pub fn recommended_for(vram_mb: Option<u32>, directml_gpu: Option<bool>) -> &'static str {
    if directml_gpu == Some(false) {
        return "qwen3-vl-2b";
    }
    // 8B needs ~7 GB loaded; recommending it only from 12 GB leaves room for
    // the app's own GPU work (search, faces, develop) alongside it.
    match vram_mb.map(|mb| mb / 1024) {
        Some(gb) if gb >= 12 => "qwen3-vl-8b",
        Some(gb) if gb >= 6 => "qwen3-vl-4b",
        Some(_) => "qwen3-vl-2b",
        None => "qwen3-vl-4b",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_mb_reads_the_catalogue_and_never_underestimates_a_stranger() {
        assert_eq!(size_mb("qwen3-vl-2b"), (1.6 * 1024.0_f32).round() as u32);
        let largest = catalogue()
            .iter()
            .map(|m| m.size_gb)
            .fold(0.0_f32, f32::max);
        // An unlisted id must quote the worst case, not zero — the figure goes
        // straight into a "this will download N GB" prompt.
        assert_eq!(
            size_mb("someone-elses:model"),
            (largest * 1024.0).round() as u32
        );
    }

    #[test]
    fn every_recommended_id_exists_in_the_catalogue() {
        for vram in [None, Some(4096), Some(6144), Some(8192), Some(16384)] {
            for gpu in [None, Some(true), Some(false)] {
                let id = recommended_for(vram, gpu);
                assert!(is_listed(id), "recommended {id} not in catalogue");
            }
        }
    }

    #[test]
    fn fit_grades_headroom_borderline_and_over_budget() {
        assert_eq!(fit(8, Some(16384)), Fit::WillRun); // 16 GB, needs 8
        assert_eq!(fit(8, Some(8192)), Fit::WillRun); // exactly 8 GB
        assert_eq!(fit(12, Some(11264)), Fit::Tight); // 11 GB, needs 12
        assert_eq!(fit(12, Some(6144)), Fit::TooLarge); // 6 GB, needs 12
        assert_eq!(fit(8, None), Fit::Unknown);
    }

    #[test]
    fn recommendation_climbs_with_vram_and_is_safe_when_unknown() {
        let gpu = Some(true);
        assert_eq!(recommended_for(Some(16384), gpu), "qwen3-vl-8b");
        assert_eq!(recommended_for(Some(12288), gpu), "qwen3-vl-8b");
        assert_eq!(recommended_for(Some(8192), gpu), "qwen3-vl-4b");
        assert_eq!(recommended_for(Some(6144), gpu), "qwen3-vl-4b");
        assert_eq!(recommended_for(Some(4096), gpu), "qwen3-vl-2b");
        assert_eq!(recommended_for(None, gpu), "qwen3-vl-4b");
        // A Mac is graded by its memory alone.
        assert_eq!(recommended_for(Some(11468), None), "qwen3-vl-4b");
    }

    /// A Windows PC with no graphics DirectML can use runs the model on its
    /// CPU: the smallest, whatever its memory reads as.
    #[test]
    fn a_pc_without_usable_graphics_gets_the_smallest_model() {
        for vram in [None, Some(0), Some(16384)] {
            assert_eq!(recommended_for(vram, Some(false)), "qwen3-vl-2b");
        }
    }

    /// The app picks these for the user, and anyone may use it for any
    /// purpose, so a research-only or registration-gated model must never be
    /// listed.
    #[test]
    fn every_listed_model_is_permissively_licensed() {
        for m in catalogue() {
            assert!(
                crate::ai_models::is_permissive(m.license),
                "{} is licensed '{}'",
                m.id,
                m.license
            );
        }
    }

    #[test]
    fn the_non_commercial_models_and_ollama_tags_are_not_listed() {
        for id in [
            "qwen2.5vl:3b",
            "minicpm-v",
            "moondream",
            "qwen3-vl:4b-instruct",
            "qwen2.5vl:7b",
        ] {
            assert!(!is_listed(id), "{id} must not be listed");
        }
    }

    /// Every file is pinned like the rest of the app's models: a Hugging Face
    /// commit (never `main`), a SHA-256, and a path of its own under
    /// `vision/`. The weights come first and the projector second, as
    /// `weights()` and `projector()` read them.
    #[test]
    fn every_file_is_pinned_and_in_its_own_folder() {
        for m in catalogue() {
            assert_eq!(m.files.len(), 2, "{}", m.id);
            assert!(m.weights().ends_with("/model.gguf"), "{}", m.id);
            assert!(m.projector().ends_with("/mmproj.gguf"), "{}", m.id);
            for f in m.files {
                assert!(
                    f.rel_path.starts_with(&format!("vision/{}/", m.id)),
                    "{}",
                    f.rel_path
                );
                assert!(f.url.contains("/resolve/") && !f.url.contains("/resolve/main/"));
                assert!(crate::hashing::is_sha256_hex(f.sha256), "{}", f.url);
                assert!(f.url.starts_with("https://huggingface.co/Qwen/"));
            }
        }
    }
}
