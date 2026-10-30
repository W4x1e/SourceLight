//! The AI model catalogue and downloader.
//!
//! The AI features — Select Subject (BiRefNet), Click-to-Select (SAM 2), search
//! and tagging (SigLIP 2) and People (AuraFace) — run local ONNX models that are
//! **not bundled** in the installer: they are large, and kept outside it they
//! survive app updates instead of arriving again with each one. So the app
//! fetches them into `<app data>/models`. The one exception is [`BUNDLED`]: the
//! face detector is 400 KB and culling needs it, so it ships in the installer.
//! This module is the catalogue of what can be fetched, where from, and a
//! streaming downloader that reports progress.
//!
//! **Every source here is a permissively-licensed public repository or
//! release** (checked
//! against the same MIT/Apache/no-copyleft bar as the code — see the licence
//! field on each entry, which a test holds to [`PERMISSIVE`]), so pointing users
//! at them carries no obligation the rest of the project does not already
//! accept. There is no auth: Hugging Face `resolve` URLs are public and
//! redirect to a CDN, which `reqwest` follows, as do GitHub release
//! downloads. A model no public source offers in the form the app needs is
//! converted and published as a release of W4x1e/models, whose
//! README says where each file came from and what was changed.
//!
//! **Every file is pinned.** A Hugging Face URL names a commit, never `main`,
//! and every file carries the SHA-256 it had when its licence was checked. A
//! repository owner re-uploading different weights — or relicensing them —
//! therefore cannot change what the app installs: the download fails its
//! integrity check instead. Moving to a newer revision is a deliberate edit
//! here, with the licence re-read at the same time.
//!
//! It has **no `ort`/AI dependency** — downloading is just HTTP and files — so
//! the catalogue and downloader compile and work in a plain build too; the
//! models simply sit unused until an `--features ai` build is run.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// One file that makes up a model (a model may be several — SAM 2 is an encoder
/// plus a decoder; a vision model is its weights plus its image projector).
/// Public so `vlm.rs` can list the vision models' files in the same form and
/// have them fetched by the same downloader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// Public download URL, pinned to a commit or a release tag.
    pub url: &'static str,
    /// Destination path relative to the models directory. The loaders expect
    /// exactly these names/subfolders, so they are not free to change.
    pub rel_path: &'static str,
    /// Lower-case hex SHA-256 of the file at that pin. A download that does not
    /// hash to this is discarded before it reaches the models directory.
    pub sha256: &'static str,
}

/// The licences a downloadable model may carry. The app fetches models on the
/// user's behalf, and anyone may use and redistribute it for any purpose, so
/// anything research-only, non-commercial, or conditional on the user
/// registering with the model's publisher is out — whatever its quality: it
/// would attach conditions the app's own licence does not. A test holds every
/// catalogue entry to this list; `vlm.rs` holds the vision models to it too.
pub const PERMISSIVE: &[&str] = &["MIT", "Apache-2.0"];

/// Whether a catalogue licence string is made only of [`PERMISSIVE`] licences.
/// A bundle of several models lists each, separated by " / ".
pub fn is_permissive(license: &str) -> bool {
    license.split(" / ").all(|l| PERMISSIVE.contains(&l.trim()))
}

/// Where a PC's models run, which decides which of two precisions of a model
/// it downloads ([`Variant`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunsOn {
    /// A Windows PC's graphics, through DirectML.
    DirectMl,
    /// A Windows PC with no graphics DirectML can use: the CPU alone.
    Cpu,
}

/// Where this machine's models run, or `None` where no model has a variant
/// to choose (a Mac, which keeps every model's own files).
pub fn runs_on() -> Option<RunsOn> {
    crate::hardware::directml_gpu().map(|gpu| if gpu { RunsOn::DirectMl } else { RunsOn::Cpu })
}

/// One of a model's files in another precision, downloaded in place of the
/// one it [`replaces`](Self::replaces) on a PC whose models run [`on`](Self::on).
///
/// Half precision is what a GPU runs best and what a PC's CPU runs worst:
/// ONNX Runtime's x86 CPU kernels are 32-bit, so a 16-bit model spends its
/// time converting. Measured (docs/BENCHMARKS.md, "Slower machines"): SigLIP
/// 2's vision tower took 101 ms a photo in fp16 on the CPU and 53 ms in fp32;
/// BiRefNet took 1.5 s in fp32 on an RTX 5070 Ti and 88 ms in fp16, and in
/// fp32 ran out of memory on integrated graphics, where fp16 runs. Each
/// variant is the same model from the same pinned revision of the same
/// repository, under the same licence.
pub struct Variant {
    pub on: RunsOn,
    /// The `rel_path` of the file in the model's own list it stands in for.
    pub replaces: &'static str,
    pub file: ModelFile,
    /// The whole model's rough download with this file in it, in MB.
    pub approx_mb: u32,
}

/// A downloadable model, as one user-facing unit.
struct ModelSpec {
    /// Stable id used by the download/delete commands.
    id: &'static str,
    /// What the user sees in the list.
    name: &'static str,
    /// The feature it powers, one line.
    purpose: &'static str,
    /// SPDX-ish licence of the weights, shown so the choice is informed.
    license: &'static str,
    /// Rough total download size in MB, for the list before a download starts.
    /// The real byte total comes from the server's `Content-Length` at run time.
    approx_mb: u32,
    files: &'static [ModelFile],
    /// Files fetched instead of some of `files` where the model runs on
    /// another kind of hardware ([`Variant`]).
    variants: &'static [Variant],
}

/// The catalogue. Licences re-read at source and every file pinned on
/// 2026-09-24:
///
/// - BiRefNet_lite — MIT (`onnx-community/BiRefNet_lite-ONNX`, from
///   `ZhengPeng7/BiRefNet_lite`).
/// - SAM 2 Hiera-Tiny — Apache-2.0 (`SharpAI/sam2-hiera-tiny-onnx`, from
///   `facebook/sam2-hiera-tiny`).
/// - SigLIP 2 base-patch16-224 — Apache-2.0 (`google/siglip2-base-patch16-224`,
///   via the `onnx-community` fp16 export). It replaced CLIP ViT-B/32 (MIT):
///   better search in English, and it understands other languages.
/// - YuNet — MIT (`opencv/face_detection_yunet`; its own LICENSE file).
/// - MediaPipe face mesh (FaceMesh V2) and blendshapes (Blendshape V2) —
///   Apache-2.0 (Google's model cards), as ONNX conversions published as
///   release assets by `MediaMolder/MediaMolder` from Google's own
///   `face_landmarker.task`. They replaced OCEC (MIT), which read smiles and
///   small faces as closed eyes.
/// - AuraFace v1 — Apache-2.0 (`fal/AuraFace-v1`), trained, its card says, on
///   commercially licensed data. It replaced FaceX nano (Apache-2.0 weights,
///   but trained on MS1M-RefineV2), which could not tell people apart in
///   real photos: see `docs/AI-MODEL-SPIKES.md`.
const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "subject",
        name: "Select Subject",
        purpose: "One-click subject / background cut-out",
        license: "MIT",
        approx_mb: 224,
        files: &[ModelFile {
            url: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/de15b22ba131738a16dff04aab8bdf8dc32e3ac1/onnx/model.onnx",
            rel_path: "birefnet.onnx",
            sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
        }],
        // The same export in fp16, for DirectML: 18 times faster on an RTX
        // 5070 Ti, a twentieth of the memory, and it runs on integrated
        // graphics, where fp32 ran out of memory. Its masks match fp32's
        // (docs/BENCHMARKS.md, "Slower machines"). The CPU, and a Mac, keep
        // fp32, which a CPU runs 2.6 times faster.
        variants: &[Variant {
            on: RunsOn::DirectMl,
            replaces: "birefnet.onnx",
            file: ModelFile {
                url: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/de15b22ba131738a16dff04aab8bdf8dc32e3ac1/onnx/model_fp16.onnx",
                rel_path: "birefnet_fp16.onnx",
                sha256: "d39b897ceb16ae654c1731f3dba0cf9b368d9cae74b5a57459b455cc8bfec402",
            },
            approx_mb: 115,
        }],
    },
    ModelSpec {
        id: "click",
        name: "Click to Select",
        purpose: "Interactive point-and-click masking (SAM 2)",
        license: "Apache-2.0",
        approx_mb: 148,
        files: &[
            ModelFile {
                url: "https://huggingface.co/SharpAI/sam2-hiera-tiny-onnx/resolve/2b467a10bfd7d49ba5fe6a00fbee4351f0b1a739/encoder.onnx",
                rel_path: "sam2/encoder.onnx",
                sha256: "df265cb552475e1b3a6cb57c939e57c95ed849bfc2f985c06efab85d8bca6db9",
            },
            ModelFile {
                url: "https://huggingface.co/SharpAI/sam2-hiera-tiny-onnx/resolve/2b467a10bfd7d49ba5fe6a00fbee4351f0b1a739/decoder.onnx",
                rel_path: "sam2/decoder.onnx",
                sha256: "63198f1f1e273d8f2f4a9d1baf926e53a01d78dc50e0674640e1513dc00d9927",
            },
        ],
        variants: &[],
    },
    // NAFNet-SIDD-width64 (Chen et al. 2022; megvii-research/NAFNet: MIT,
    // with BasicSR's Apache-2.0) for AI noise reduction. Qualcomm AI
    // Hub's ONNX export, converted to fp16 and published as a release of
    // W4x1e/models: half the download of the 32-bit model, and
    // 1.35–1.9x faster on the graphics hardware nearly every PC has (slower on
    // a CPU alone, the rare fallback). It replaced SCUNet, three to six times
    // slower everywhere. See docs/AI-MODEL-SPIKES.md.
    ModelSpec {
        id: "denoise",
        name: "AI Noise Reduction",
        purpose: "Clears colour speckle and blotchy high-ISO noise in Develop, keeping edges (NAFNet)",
        license: "MIT / Apache-2.0",
        approx_mb: 234,
        files: &[ModelFile {
            url: "https://github.com/W4x1e/models/releases/download/nafnet-sidd-width64-fp16-v1/nafnet_denoise_fp16.onnx",
            rel_path: "denoise/nafnet_denoise_fp16.onnx",
            sha256: "af6782e882e3ddb3d72ea3bf41b055ea58063851a183e8fafc3e05cd9e9652c9",
        }],
        variants: &[],
    },
    // Big LaMa (Suvorov et al. 2021; advimman/lama, Apache-2.0), Carve's
    // ONNX export at a fixed 512 px (`Carve/LaMa-ONNX`, Apache-2.0), for
    // magic erase. Chosen over MI-GAN (MIT, 28 MB), which smudged the gap a
    // person leaves and invented objects in a large one: see
    // docs/AI-MODEL-SPIKES.md.
    ModelSpec {
        id: "erase",
        name: "Magic Erase",
        purpose: "Takes people and objects out of a photo in Develop, filling in what was behind them (LaMa)",
        license: "Apache-2.0",
        approx_mb: 208,
        files: &[ModelFile {
            url: "https://huggingface.co/Carve/LaMa-ONNX/resolve/c3c0c9e468934d62e79c329e35d82dd09ff8c444/lama_fp32.onnx",
            rel_path: "erase/lama_fp32.onnx",
            sha256: "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6",
        }],
        variants: &[],
    },
    ModelSpec {
        id: "tagger",
        name: "Search & Auto-Tagging",
        purpose: "Search by what photos show, in your own language, and tag them (SigLIP 2)",
        license: "Apache-2.0",
        approx_mb: 785,
        files: &[
            // SigLIP 2 (Google, Apache-2.0), the onnx-community fp16 export.
            // fp16, not the int8 `_quantized` files: those score below CLIP.
            // It needs full graph optimisation on DirectML and basic on the CPU
            // (see `build_session`), or the vision tower is wrong or won't load.
            ModelFile {
                url: "https://huggingface.co/onnx-community/siglip2-base-patch16-224-ONNX/resolve/ba1f3b0843f24bc5417d38e19c37b287d719b2f4/onnx/vision_model_fp16.onnx",
                rel_path: "siglip2/vision_model.onnx",
                sha256: "a1959f7bd3993a607e48839f6d01e25b876fe76afda301b028b78eef68aabd95",
            },
            ModelFile {
                url: "https://huggingface.co/onnx-community/siglip2-base-patch16-224-ONNX/resolve/ba1f3b0843f24bc5417d38e19c37b287d719b2f4/onnx/text_model_fp16.onnx",
                rel_path: "siglip2/text_model.onnx",
                sha256: "711da56ada0a4aa11c7dd3320df741081a3cae4f0ae1b5e5c6d5b294738d0eb0",
            },
            ModelFile {
                url: "https://huggingface.co/onnx-community/siglip2-base-patch16-224-ONNX/resolve/ba1f3b0843f24bc5417d38e19c37b287d719b2f4/tokenizer.json",
                rel_path: "siglip2/tokenizer.json",
                sha256: "cb9140fae3ac5122c972d37adf83e1248471a38147ad76f8215c8872c6fd8322",
            },
        ],
        // On a PC with no GPU the vision tower runs in fp32: twice as fast on
        // the CPU as fp16 (53 ms a photo against 101) and 100 MB smaller in
        // memory, for 186 MB more to download. fp32 is the reference fp16 is
        // checked against. The text tower stays fp16 everywhere: it runs once
        // a search, and in fp32 it is 1.1 GB.
        variants: &[Variant {
            on: RunsOn::Cpu,
            replaces: "siglip2/vision_model.onnx",
            file: ModelFile {
                url: "https://huggingface.co/onnx-community/siglip2-base-patch16-224-ONNX/resolve/ba1f3b0843f24bc5417d38e19c37b287d719b2f4/onnx/vision_model.onnx",
                rel_path: "siglip2/vision_model_fp32.onnx",
                sha256: "c0573e3f4140c3a7c4e9cc5912bd6b26a033b46a6a8e8af26cbea262b163bcad",
            },
            approx_mb: 971,
        }],
    },
    ModelSpec {
        id: "faces",
        name: "Face tools",
        purpose: "Find faces, and flag closed eyes and soft faces in a cull",
        license: "MIT / Apache-2.0",
        approx_mb: 7,
        files: &[
            // YuNet — face detection + 5 landmarks (OpenCV, MIT).
            ModelFile {
                url: "https://huggingface.co/opencv/face_detection_yunet/resolve/3cc26e7f1014a5ee5d74a42acee58bafc9d0a310/face_detection_yunet_2023mar.onnx",
                rel_path: "faces/yunet.onnx",
                sha256: "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
            },
            // MediaPipe's face mesh (FaceMesh V2, Google, Apache-2.0): 478
            // landmarks from a 256×256 face crop. ONNX conversion of
            // `face_landmarker.task` float16/1 by MediaMolder, checked
            // bit-for-bit against PINTO0309's independent conversion. A release
            // asset can be replaced under the same URL, which is exactly what
            // the pinned digest guards against.
            ModelFile {
                url: "https://github.com/MediaMolder/MediaMolder/releases/download/face-expression-model-v1/face_landmarks_detector.onnx",
                rel_path: "faces/mesh.onnx",
                sha256: "3e824fb93ac4cc39a0cf3b448008311988f6b8dc3e8ecd8665a1daa1303ff635",
            },
            // Its blendshapes (Blendshape V2, Apache-2.0): 52 expression
            // scores from 146 of those landmarks, `eyeBlink` apart from
            // `eyeSquint`, which is what lets a smile read as open eyes.
            ModelFile {
                url: "https://github.com/MediaMolder/MediaMolder/releases/download/face-expression-model-v1/face_blendshapes.onnx",
                rel_path: "faces/blendshapes.onnx",
                sha256: "f6bd15b960a55057dbbf05bb2f32d757c67e651b15c9c13d4d5d744ca03dd3aa",
            },
        ],
        variants: &[],
    },
    // Its own entry: culling needs only the two small models above,
    // and should not have to fetch 261 MB to find a closed eye.
    ModelSpec {
        id: "people",
        name: "People",
        purpose: "Recognise the same person across photos, to group them in People",
        license: "Apache-2.0",
        approx_mb: 261,
        files: &[
            // AuraFace v1 (fal) — ArcFace ResNet100 face embedder for people
            // grouping. **Apache-2.0**, and trained on commercially licensed
            // data, unlike InsightFace's own ArcFace weights (non-commercial).
            // 112×112 aligned RGB in, 512-d embedding out.
            ModelFile {
                url: "https://huggingface.co/fal/AuraFace-v1/resolve/af6d057c9b0ec4071d4c49c80e3539258798b609/glintr100.onnx",
                rel_path: "faces/auraface.onnx",
                sha256: "a7933ea5330113b01c9b60351d8f4c33003f145d8470ac5f0e52ee2effe25c60",
            },
        ],
        variants: &[],
    },
];

/// One catalogue entry plus whether it is already on disk — the shape the UI
/// lists.
#[derive(serde::Serialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AiModelStatus {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub license: String,
    pub approx_mb: u32,
    pub installed: bool,
}

/// Download progress for one model, emitted on the `ai-download-progress` event.
#[derive(serde::Serialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AiDownloadProgress {
    pub id: String,
    pub received: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

/// Rejects a relative path that could escape the models directory. The paths are
/// all our own constants today, but this is the one check that must hold if the
/// catalogue is ever edited carelessly, so it is enforced rather than trusted.
fn is_safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.starts_with('\\')
        && !rel.contains(':')
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn find(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// Whether a finished stream is the whole file. A server that reports no
/// `Content-Length` (`content_length == 0`) can't be checked, so it is accepted;
/// otherwise the byte count must match exactly, or a silently-truncated stream
/// would be renamed into place and read as installed.
fn is_complete(received: u64, content_length: u64) -> bool {
    content_length == 0 || received == content_length
}

/// The files `spec` is made of on a PC whose models run `on`: its own, with
/// the [`Variant`] for `on`, if it has one, in place of the file it replaces.
fn files_for(spec: &'static ModelSpec, on: Option<RunsOn>) -> Vec<ModelFile> {
    spec.files
        .iter()
        .map(|f| {
            spec.variants
                .iter()
                .find(|v| Some(v.on) == on && v.replaces == f.rel_path)
                .map_or(*f, |v| v.file)
        })
        .collect()
}

/// `spec`'s rough download in MB on a PC whose models run `on`.
fn approx_mb_for(spec: &ModelSpec, on: Option<RunsOn>) -> u32 {
    spec.variants
        .iter()
        .find(|v| Some(v.on) == on)
        .map_or(spec.approx_mb, |v| v.approx_mb)
}

/// A model counts as installed only when *every* file exists and is non-empty —
/// a half-finished or zero-byte download must not read as ready. The files
/// are the ones for where this machine's models run: a GPU PC holding only
/// BiRefNet's fp32 file fetches its fp16 one, which it runs 18 times faster.
fn installed(spec: &'static ModelSpec, models_dir: &Path) -> bool {
    installed_on(spec, models_dir, runs_on())
}

fn installed_on(spec: &'static ModelSpec, models_dir: &Path, on: Option<RunsOn>) -> bool {
    files_installed(&files_for(spec, on), models_dir)
}

/// Whether every one of `files` is in `models_dir` and non-empty.
pub fn files_installed(files: &[ModelFile], models_dir: &Path) -> bool {
    files.iter().all(|f| {
        std::fs::metadata(models_dir.join(f.rel_path))
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false)
    })
}

/// The whole catalogue with installed status, for the settings list.
pub fn statuses(models_dir: &Path) -> Vec<AiModelStatus> {
    MODELS
        .iter()
        .map(|m| AiModelStatus {
            id: m.id.to_string(),
            name: m.name.to_string(),
            purpose: m.purpose.to_string(),
            license: m.license.to_string(),
            approx_mb: approx_mb_for(m, runs_on()),
            installed: installed(m, models_dir),
        })
        .collect()
}

/// Files earlier builds downloaded that nothing reads any more: CLIP, replaced
/// by SigLIP 2, FaceX, replaced by AuraFace, SCUNet, replaced by
/// NAFNet, and NAFNet's 32-bit files, replaced by its fp16 one.
const RETIRED: &[&str] = &[
    "clip/vision_model.onnx",
    "clip/text_model.onnx",
    "clip/tokenizer.json",
    "clip/labels.bin",
    "faces/facex.onnx",
    "faces/ocec.onnx",
    "denoise/scunet_color_real_psnr.onnx",
    "denoise/scunet_color_real_psnr.onnx.data",
    "denoise/nafnet_denoise.onnx",
    "denoise/nafnet_denoise.data",
];

/// Models the AI installer carries, so they never need a download: the face
/// detector and the eye models, 7 MB between them. Asking before a download
/// that size was a dialog for nothing, and it stood between a cull and its
/// closed-eye check. The installer puts their files under `models/` in its
/// resources, at the same relative paths as the catalogue.
pub const BUNDLED: &[&str] = &["faces"];

/// Copies bundled model `id` into `models_dir` from the first of `sources` that
/// holds every file with its pinned digest. Returns whether the model is
/// installed afterwards. A copy that does not match its digest is skipped, so
/// a damaged install falls back to the download, never to a wrong model.
pub fn install_bundled(sources: &[PathBuf], models_dir: &Path, id: &str) -> bool {
    let Some(spec) = find(id) else {
        return false;
    };
    if installed(spec, models_dir) {
        return true;
    }
    for root in sources {
        let files: Option<Vec<(Vec<u8>, &str)>> = spec
            .files
            .iter()
            .map(|f| {
                let bytes = std::fs::read(root.join(f.rel_path)).ok()?;
                (crate::hashing::hex(&Sha256::digest(&bytes)) == f.sha256)
                    .then_some((bytes, f.rel_path))
            })
            .collect();
        let Some(files) = files else { continue };
        let copied = files.iter().all(|(bytes, rel)| {
            let dest = models_dir.join(rel);
            let part = part_path(&dest);
            dest.parent()
                .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
                && std::fs::write(&part, bytes).is_ok()
                && std::fs::rename(&part, &dest).is_ok()
        });
        if copied {
            return true;
        }
    }
    false
}

/// Deletes [`RETIRED`] model files, and the `clip/` folder once it is empty.
/// They are the app's own downloads, not the user's files, and were only ever
/// fetched to be used by a build that no longer exists. Best effort: a file
/// that cannot be removed now is tried again next launch.
pub fn remove_retired(models_dir: &Path) {
    for rel in RETIRED {
        let _ = std::fs::remove_file(models_dir.join(rel));
    }
    let _ = std::fs::remove_dir(models_dir.join("clip"));
}

/// The file to load for `rel`, one of a model's own files: this machine's
/// precision of it where that is on disk ([`runs_on`]), else whichever is. A
/// PC that has only the other precision still runs, as before, while the
/// download of its own is pending.
pub fn best_file(models_dir: &Path, rel: &str) -> Option<PathBuf> {
    best_file_on(models_dir, rel, runs_on())
}

fn best_file_on(models_dir: &Path, rel: &str, on: Option<RunsOn>) -> Option<PathBuf> {
    let variants: Vec<&Variant> = MODELS
        .iter()
        .flat_map(|m| m.variants)
        .filter(|v| v.replaces == rel)
        .collect();
    let mine = variants.iter().filter(|v| Some(v.on) == on);
    let others = variants.iter().filter(|v| Some(v.on) != on);
    mine.map(|v| v.file.rel_path)
        .chain([rel])
        .chain(others.map(|v| v.file.rel_path))
        .map(|r| models_dir.join(r))
        .find(|p| p.is_file())
}

/// Deletes a model's files, every variant's too (and any now-empty
/// subfolder). Missing files are not an error — the end state is what matters.
pub fn delete(id: &str, models_dir: &Path) -> Result<()> {
    let spec = find(id).with_context(|| format!("unknown model '{id}'"))?;
    let variants: Vec<ModelFile> = spec.variants.iter().map(|v| v.file).collect();
    delete_files(&variants, models_dir)?;
    delete_files(spec.files, models_dir)
}

/// Deletes the files of a model's other precision once this machine's own
/// set is complete: BiRefNet's fp32 file on a PC that now has the fp16 one
/// its GPU runs. They are the app's own downloads, kept by no one. Run at
/// startup, when nothing has a model open; best effort.
pub fn remove_unused_variants(models_dir: &Path) {
    remove_unused_variants_on(models_dir, runs_on());
}

fn remove_unused_variants_on(models_dir: &Path, on: Option<RunsOn>) {
    for spec in MODELS.iter().filter(|m| !m.variants.is_empty()) {
        if !installed_on(spec, models_dir, on) {
            continue;
        }
        let wanted = files_for(spec, on);
        let every = spec
            .files
            .iter()
            .chain(spec.variants.iter().map(|v| &v.file));
        for f in every.filter(|f| !wanted.contains(f)) {
            let path = models_dir.join(f.rel_path);
            if path.exists() {
                eprintln!(
                    "removing {}, not the precision this machine runs",
                    f.rel_path
                );
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Deletes `files` from `models_dir`, and any subfolder they leave empty.
pub fn delete_files(files: &[ModelFile], models_dir: &Path) -> Result<()> {
    for f in files {
        let path = models_dir.join(f.rel_path);
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
        // And a download of it that never finished.
        let _ = std::fs::remove_file(part_path(&path));
        // Tidy an emptied subfolder (e.g. sam2/, siglip2/); ignore if not empty.
        if let Some(parent) = path.parent() {
            if parent != models_dir {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
    Ok(())
}

/// Deletes every unfinished download (`*.part`) anywhere under `dir`.
/// Run at startup, when no download is running.
pub fn remove_partial_downloads(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_partial_downloads(&path);
        } else if path.extension().is_some_and(|e| e == "part") {
            eprintln!("removing an unfinished download: {}", path.display());
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// The temp path a download streams into before the atomic rename, so an
/// interrupted download never leaves a truncated file that `installed` would
/// accept.
fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Streams every file of a model into place, calling `on_progress(received,
/// total)` as bytes arrive. `total` is the sum of the servers' reported sizes
/// where known, else the catalogue's estimate, so a bar has a stable
/// denominator across the model's several files.
///
/// Each file lands via a `.part` temp then an atomic rename, and the whole set
/// is fetched in order; a failure leaves the already-finished files in place
/// (they are individually valid) and surfaces the error.
///
/// Stops between chunks once `cancel` is set, removing the partial file and
/// returning [`Cancelled`](crate::jobs::Cancelled). Files already finished are
/// left in place — each is complete and verified on its own.
pub async fn download(
    id: &str,
    models_dir: &Path,
    cancel: &crate::jobs::CancelToken,
    on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    let spec = find(id).with_context(|| format!("unknown model '{id}'"))?;
    let on = runs_on();
    download_files(
        spec.name,
        approx_mb_for(spec, on),
        &files_for(spec, on),
        models_dir,
        cancel,
        on_progress,
    )
    .await
}

/// [`download`] for any list of pinned files: `name` for errors and the
/// disk-space check, `approx_mb` for the bar's denominator until the servers
/// report sizes. The vision models (`vlm.rs`) are fetched through this.
pub async fn download_files(
    name: &str,
    approx_mb: u32,
    files: &[ModelFile],
    models_dir: &Path,
    cancel: &crate::jobs::CancelToken,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    for f in files {
        anyhow::ensure!(
            is_safe_rel(f.rel_path),
            "unsafe model path '{}'",
            f.rel_path
        );
    }

    let grand_total_est = (approx_mb as u64) * 1_048_576;
    // Refused up front, not discovered at 90% of a multi-gigabyte download.
    crate::hardware::ensure_space(models_dir, grand_total_est, name)?;
    let client = reqwest::Client::builder()
        .user_agent("SourceLight")
        .build()
        .context("building the download client")?;

    let mut prior_files: u64 = 0; // bytes finished in earlier files of this model
    for f in files {
        let dest = models_dir.join(f.rel_path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = part_path(&dest);

        prior_files += fetch(
            &client,
            f.url,
            &tmp,
            f.sha256,
            f.rel_path,
            cancel,
            |received, this_total| {
                let denom = grand_total_est.max(prior_files + this_total.max(received));
                on_progress(prior_files + received, denom);
            },
        )
        .await?;
        tokio::fs::rename(&tmp, &dest)
            .await
            .with_context(|| format!("finalising {}", dest.display()))?;
    }

    on_progress(prior_files, prior_files.max(1));
    Ok(())
}

/// Streams `url` into `tmp`, checking it is complete and hashes to `sha256`,
/// calling `progress(received, total)` about every megabyte. Returns the bytes
/// received. `what` names the file in errors. A failed or cancelled fetch
/// leaves no `tmp` behind: a network drop 4 GB into a model used to leave the
/// 4 GB.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    tmp: &Path,
    sha256: &str,
    what: &str,
    cancel: &crate::jobs::CancelToken,
    progress: impl FnMut(u64, u64),
) -> Result<u64> {
    let result = fetch_into(client, url, tmp, sha256, what, cancel, progress).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(tmp).await;
    }
    result
}

/// How long a download may go without a byte before it is given up as
/// stalled. `chunk()` alone waits for ever on a connection that stays up but
/// stops sending (a CDN or proxy stalling mid-file), and TCP keepalive never
/// notices one.
const STALL: std::time::Duration = std::time::Duration::from_secs(60);

/// How often a download waiting for its next bytes looks at its Cancel.
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// The next chunk of `resp`: [`Cancelled`](crate::jobs::Cancelled) as soon as
/// `cancel` is set, even while nothing is arriving, and an error once nothing
/// has for [`STALL`]. Shared with the runtime download (`llama_pack`).
pub(crate) async fn next_chunk(
    resp: &mut reqwest::Response,
    cancel: &crate::jobs::CancelToken,
) -> Result<Option<impl AsRef<[u8]>>> {
    let waiting = tokio::time::Instant::now();
    loop {
        if cancel.is_cancelled() {
            return Err(crate::jobs::Cancelled.into());
        }
        // `chunk()` is cancel-safe: one cut short by the timeout loses nothing.
        match tokio::time::timeout(CANCEL_POLL, resp.chunk()).await {
            Ok(chunk) => return chunk.context("reading a chunk"),
            Err(_) if waiting.elapsed() >= STALL => {
                anyhow::bail!("The download stopped arriving — try again")
            }
            Err(_) => {}
        }
    }
}

/// How many times one download that breaks off part-way picks up where it
/// stopped before it gives up.
const RESUMES: u32 = 5;

/// How long a broken-off download waits before asking for the rest, so a
/// connection that has just dropped has a moment to come back.
const RESUME_PAUSE: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(20)
} else {
    std::time::Duration::from_secs(2)
};

/// The rest of `url` from byte `from`, or `None` if the server will not send
/// exactly that, or the download is cancelled meanwhile. A server that
/// answers a range with the whole file is no use mid-file: the download then
/// fails, as it did before, and starts again from the next attempt.
async fn resume_from(
    client: &reqwest::Client,
    url: &str,
    from: u64,
    cancel: &crate::jobs::CancelToken,
) -> Option<reqwest::Response> {
    let paused = tokio::time::Instant::now();
    while paused.elapsed() < RESUME_PAUSE {
        if cancel.is_cancelled() {
            return None;
        }
        tokio::time::sleep(CANCEL_POLL.min(RESUME_PAUSE)).await;
    }
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={from}-"))
        .send()
        .await
        .ok()?;
    let starts_at = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes "))
        .and_then(|v| v.split('-').next())
        .and_then(|v| v.trim().parse::<u64>().ok());
    (resp.status() == reqwest::StatusCode::PARTIAL_CONTENT && starts_at == Some(from))
        .then_some(resp)
}

/// [`fetch`]'s work, which may leave `tmp` behind on an error.
async fn fetch_into(
    client: &reqwest::Client,
    url: &str,
    tmp: &Path,
    sha256: &str,
    what: &str,
    cancel: &crate::jobs::CancelToken,
    mut progress: impl FnMut(u64, u64),
) -> Result<u64> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading {url}"))?;
    let this_total = resp.content_length().unwrap_or(0);
    let mut hasher = Sha256::new();
    let mut file = tokio::fs::File::create(tmp)
        .await
        .with_context(|| format!("creating {}", tmp.display()))?;

    use tokio::io::AsyncWriteExt;
    let mut received: u64 = 0;
    let mut last_emit: u64 = 0;
    let mut resumes = 0;
    // Cancelled, or broken off for good, the error goes back to `fetch`,
    // which removes `tmp`.
    loop {
        let broke = match next_chunk(&mut resp, cancel).await {
            Ok(Some(chunk)) => {
                let chunk = chunk.as_ref();
                file.write_all(chunk).await.context("writing a chunk")?;
                hasher.update(chunk);
                received += chunk.len() as u64;
                // Throttle events to ~every 1 MB so a large file does not
                // flood the UI with thousands of messages.
                if received - last_emit >= 1_048_576 {
                    last_emit = received;
                    progress(received, this_total);
                }
                continue;
            }
            Ok(None) if is_complete(received, this_total) => break,
            // Closed early without an error: the check below says so,
            // unless the rest can be had.
            Ok(None) => None,
            Err(e) if e.is::<crate::jobs::Cancelled>() => return Err(e),
            Err(e) => Some(e),
        };
        // A connection that breaks off part-way, a dropped Wi-Fi or a stalled
        // CDN, picks up where it stopped: on a slow line the 785 MB search
        // model takes the best part of an hour, and starting it again from
        // nothing at 90% is the difference between finishing and not. The
        // digest is still checked over the whole file below.
        let resumed = if resumes < RESUMES && this_total > 0 {
            resumes += 1;
            eprintln!(
                "download of {what} broke off at {received} of {this_total} bytes ({}); resuming",
                broke
                    .as_ref()
                    .map_or("closed early".to_string(), |e| format!("{e:#}"))
            );
            resume_from(client, url, received, cancel).await
        } else {
            None
        };
        match (resumed, broke) {
            (Some(rest), _) => resp = rest,
            (None, Some(e)) => return Err(e),
            (None, None) => break,
        }
    }
    file.flush().await.context("flushing")?;
    drop(file);

    // Guard against a stream that ended early without erroring — otherwise the
    // truncated `.part` would be renamed into place and pass `installed`.
    if !is_complete(received, this_total) {
        let _ = tokio::fs::remove_file(tmp).await;
        // The numbers are for the log, not the person.
        eprintln!("incomplete download of {what}: got {received} of {this_total} bytes");
        anyhow::bail!("The download of {what} stopped before the end — try again");
    }

    // Integrity: a file whose bytes don't match the pinned digest never
    // reaches the models dir (and so is never loaded into ONNX Runtime).
    // Always checked — there is no "digest unavailable" case any more.
    let got = crate::hashing::hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(sha256) {
        let _ = tokio::fs::remove_file(tmp).await;
        // Two digests in a toast read as a crash: they go to the log.
        eprintln!("integrity check failed for {what}: expected sha256 {sha256}, got {got}");
        anyhow::bail!("The download of {what} didn't arrive intact — try again");
    }
    Ok(received)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> crate::test_support::Scratch {
        crate::test_support::Scratch::new(&format!("ai-models-{name}"))
    }

    #[test]
    fn the_catalogue_is_well_formed() {
        assert!(!MODELS.is_empty());
        let mut ids = std::collections::HashSet::new();
        for m in MODELS {
            assert!(ids.insert(m.id), "duplicate model id '{}'", m.id);
            assert!(!m.files.is_empty(), "{} has no files", m.id);
            // A variant stands in for a file the model has, under a name of
            // its own, so the two precisions can sit side by side.
            for v in m.variants {
                assert!(
                    m.files.iter().any(|f| f.rel_path == v.replaces),
                    "{}: a variant replaces '{}', which is not one of its files",
                    m.id,
                    v.replaces
                );
                assert!(
                    m.files.iter().all(|f| f.rel_path != v.file.rel_path),
                    "{}: a variant overwrites '{}'",
                    m.id,
                    v.file.rel_path
                );
            }
            for f in m.files.iter().chain(m.variants.iter().map(|v| &v.file)) {
                assert!(
                    is_safe_rel(f.rel_path),
                    "{}: unsafe path '{}'",
                    m.id,
                    f.rel_path
                );
                // Public, no-auth, permissively-licensed sources only: a Hugging
                // Face `resolve` URL, or a GitHub release download (both are
                // stable public HTTPS the downloader can follow without a token).
                let hf =
                    f.url.starts_with("https://huggingface.co/") && f.url.contains("/resolve/");
                let gh = f.url.starts_with("https://github.com/")
                    && f.url.contains("/releases/download/");
                assert!(
                    hf || gh,
                    "{} url is not a public HF resolve or GitHub release URL: {}",
                    m.id,
                    f.url
                );
                // Pinned: a Hugging Face URL names a 40-hex commit, never a
                // branch, so the bytes behind it cannot move.
                if hf {
                    let rev = f
                        .url
                        .split("/resolve/")
                        .nth(1)
                        .unwrap()
                        .split('/')
                        .next()
                        .unwrap();
                    assert!(
                        rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()),
                        "{}: {} is not pinned to a commit",
                        m.id,
                        f.url
                    );
                }
                assert!(
                    crate::hashing::is_sha256_hex(f.sha256)
                        && f.sha256.bytes().all(|b| !b.is_ascii_uppercase()),
                    "{}: {} has no lower-case SHA-256",
                    m.id,
                    f.rel_path
                );
            }
        }
    }

    /// Anyone may use the app for any purpose. A model that is research-only,
    /// non-commercial, or usable only after registering with its publisher
    /// must never reach the catalogue, however good it is.
    #[test]
    fn every_model_is_permissively_licensed() {
        for m in MODELS {
            assert!(
                is_permissive(m.license),
                "{} is licensed '{}', which is not in {PERMISSIVE:?}",
                m.id,
                m.license
            );
        }
        assert!(is_permissive("MIT / Apache-2.0"));
        assert!(!is_permissive("qwen-research"));
        assert!(!is_permissive("MIT / CC-BY-NC-4.0"));
    }

    /// A GPU PC fetches BiRefNet in fp16 and a PC without one SigLIP 2's
    /// vision tower in fp32; everything else, and every model on a Mac, is
    /// the catalogue's own files.
    #[test]
    fn each_machine_fetches_the_precision_it_runs_best() {
        let rel = |id: &str, on| -> Vec<&str> {
            files_for(find(id).unwrap(), on)
                .iter()
                .map(|f| f.rel_path)
                .collect()
        };
        assert_eq!(
            rel("subject", Some(RunsOn::DirectMl)),
            ["birefnet_fp16.onnx"]
        );
        assert_eq!(rel("subject", Some(RunsOn::Cpu)), ["birefnet.onnx"]);
        assert_eq!(rel("subject", None), ["birefnet.onnx"]);
        assert_eq!(
            rel("tagger", Some(RunsOn::Cpu))[0],
            "siglip2/vision_model_fp32.onnx"
        );
        assert_eq!(
            rel("tagger", Some(RunsOn::DirectMl))[0],
            "siglip2/vision_model.onnx"
        );
        assert_eq!(rel("tagger", None)[0], "siglip2/vision_model.onnx");
        assert_eq!(rel("click", Some(RunsOn::Cpu)), rel("click", None));
        // The size quoted before a download is the variant's.
        let subject = find("subject").unwrap();
        assert!(approx_mb_for(subject, Some(RunsOn::DirectMl)) < approx_mb_for(subject, None));
    }

    /// A GPU PC holding only BiRefNet's fp32 file has not got the model it
    /// runs; once the fp16 one arrives, the fp32 one goes.
    #[test]
    fn the_other_precision_goes_once_this_machines_is_there() {
        let root = scratch("variants");
        let root = root.path();
        let subject = find("subject").unwrap();
        let gpu = Some(RunsOn::DirectMl);
        std::fs::write(root.join("birefnet.onnx"), b"fp32").unwrap();
        assert!(!installed_on(subject, root, gpu));
        assert!(installed_on(subject, root, Some(RunsOn::Cpu)));
        // Incomplete: nothing is removed.
        remove_unused_variants_on(root, gpu);
        assert!(root.join("birefnet.onnx").exists());

        std::fs::write(root.join("birefnet_fp16.onnx"), b"fp16").unwrap();
        assert!(installed_on(subject, root, gpu));
        remove_unused_variants_on(root, gpu);
        assert!(!root.join("birefnet.onnx").exists());
        assert!(root.join("birefnet_fp16.onnx").exists());

        // Loading takes this machine's precision, else what there is.
        let best = |on| best_file_on(root, "birefnet.onnx", on);
        assert_eq!(best(gpu), Some(root.join("birefnet_fp16.onnx")));
        assert_eq!(
            best(Some(RunsOn::Cpu)),
            Some(root.join("birefnet_fp16.onnx"))
        );
        std::fs::write(root.join("birefnet.onnx"), b"fp32").unwrap();
        assert_eq!(best(Some(RunsOn::Cpu)), Some(root.join("birefnet.onnx")));
        assert_eq!(best(None), Some(root.join("birefnet.onnx")));
        assert_eq!(best(gpu), Some(root.join("birefnet_fp16.onnx")));

        // Deleting the model takes every precision of it.
        std::fs::write(root.join("birefnet.onnx"), b"fp32").unwrap();
        delete("subject", root).unwrap();
        assert!(!root.join("birefnet.onnx").exists());
        assert!(!root.join("birefnet_fp16.onnx").exists());
    }

    #[test]
    fn is_safe_rel_rejects_traversal_and_absolutes() {
        assert!(is_safe_rel("clip/vision_model.onnx"));
        assert!(is_safe_rel("birefnet.onnx"));
        assert!(!is_safe_rel("../evil.onnx"));
        assert!(!is_safe_rel("/etc/passwd"));
        assert!(!is_safe_rel("c:\\windows\\x"));
        assert!(!is_safe_rel("a/../../b"));
        assert!(!is_safe_rel(""));
    }

    #[test]
    fn installed_requires_every_file_present_and_non_empty() {
        let root = scratch("installed");
        let root = root.path();
        let sam2 = find("click").unwrap();

        // Nothing there yet.
        assert!(!installed(sam2, root));

        // One of the two files, and empty — still not installed.
        std::fs::create_dir_all(root.join("sam2")).unwrap();
        std::fs::write(root.join("sam2/encoder.onnx"), b"").unwrap();
        assert!(!installed(sam2, root));

        // Both present and non-empty — installed.
        std::fs::write(root.join("sam2/encoder.onnx"), b"x").unwrap();
        std::fs::write(root.join("sam2/decoder.onnx"), b"y").unwrap();
        assert!(installed(sam2, root));

        // The status list reflects it, and leaves the others uninstalled.
        let all = statuses(root);
        let click = all.iter().find(|s| s.id == "click").unwrap();
        assert!(click.installed);
        assert!(!all.iter().find(|s| s.id == "subject").unwrap().installed);
    }

    #[test]
    fn is_complete_verifies_length_when_known() {
        assert!(is_complete(100, 100)); // exact
        assert!(is_complete(100, 0)); // unknown length: cannot verify, accept
        assert!(!is_complete(99, 100)); // truncated
        assert!(!is_complete(101, 100)); // over-long (shouldn't happen) still rejected
    }

    /// A download that breaks off part-way picks up where it stopped, and
    /// one whose server cannot send the rest fails as it always did.
    #[tokio::test]
    async fn a_broken_off_download_picks_up_where_it_stopped() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let body: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let sha = crate::hashing::hex(&Sha256::digest(&body));
        for honours_ranges in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/model.onnx", listener.local_addr().unwrap());
            let served = body.clone();
            // The first answer stops at 40% and drops the connection; a
            // request for the rest gets it, or the whole file again.
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let mut buf = vec![0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                    let from = request.lines().find_map(|l| {
                        l.strip_prefix("range: bytes=")?
                            .trim()
                            .trim_end_matches('-')
                            .parse::<usize>()
                            .ok()
                    });
                    let len = served.len();
                    let (head, part) = match from {
                        Some(from) if honours_ranges => (
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{len}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                len - 1,
                                len - from
                            ),
                            &served[from..],
                        ),
                        Some(_) => (
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"),
                            &served[..],
                        ),
                        None => (
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"),
                            &served[..len * 2 / 5],
                        ),
                    };
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(part).await;
                    let _ = socket.shutdown().await;
                }
            });

            let root = scratch(if honours_ranges {
                "resume"
            } else {
                "no-resume"
            });
            let tmp = root.join("model.onnx.part");
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let jobs = crate::jobs::Jobs::default();
            let cancel = jobs.start("model:test");
            let got = fetch(
                &client,
                &url,
                &tmp,
                &sha,
                "the test model",
                &cancel,
                |_, _| {},
            )
            .await;
            if honours_ranges {
                assert_eq!(got.unwrap(), body.len() as u64);
                assert_eq!(std::fs::read(&tmp).unwrap(), body);
            } else {
                assert!(
                    got.is_err(),
                    "a whole file sent mid-file must not be appended"
                );
                assert!(!tmp.exists(), "a failed download leaves nothing behind");
            }
        }
    }

    #[test]
    fn part_path_appends_without_losing_the_extension() {
        let p = part_path(Path::new("/models/clip/vision_model.onnx"));
        assert!(p.to_string_lossy().ends_with("vision_model.onnx.part"));
    }

    #[test]
    fn delete_removes_files_and_is_idempotent() {
        let root = scratch("delete");
        let root = root.path();
        std::fs::create_dir_all(root.join("sam2")).unwrap();
        std::fs::write(root.join("sam2/encoder.onnx"), b"x").unwrap();
        std::fs::write(root.join("sam2/decoder.onnx"), b"y").unwrap();
        assert!(installed(find("click").unwrap(), root));

        delete("click", root).unwrap();
        assert!(!installed(find("click").unwrap(), root));
        // The emptied subfolder is tidied.
        assert!(!root.join("sam2").exists());
        // Deleting again is fine.
        delete("click", root).unwrap();
    }

    #[test]
    fn retired_files_are_removed_and_current_ones_kept() {
        let dir = scratch("retired");
        std::fs::create_dir_all(dir.join("clip")).unwrap();
        std::fs::create_dir_all(dir.join("faces")).unwrap();
        for f in [
            "clip/vision_model.onnx",
            "clip/labels.bin",
            "faces/facex.onnx",
            "faces/yunet.onnx",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        remove_retired(&dir);
        assert!(
            !dir.join("clip").exists(),
            "the emptied clip/ folder goes too"
        );
        assert!(!dir.join("faces/facex.onnx").exists());
        assert!(
            dir.join("faces/yunet.onnx").exists(),
            "a current model is untouched"
        );
        // No retired path is also a current download.
        for rel in RETIRED {
            assert!(
                MODELS
                    .iter()
                    .all(|m| m.files.iter().all(|f| f.rel_path != *rel)),
                "{rel}"
            );
        }
    }

    /// The vendored copies are what the installer ships, so they must be the
    /// pinned files: a wrong one would be skipped and the model downloaded.
    #[test]
    fn the_vendored_bundled_models_match_their_pins() {
        let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/models");
        for id in BUNDLED {
            for f in find(id).unwrap().files {
                let bytes = std::fs::read(vendor.join(f.rel_path))
                    .unwrap_or_else(|e| panic!("vendor/models/{}: {e}", f.rel_path));
                assert_eq!(
                    crate::hashing::hex(&Sha256::digest(&bytes)),
                    f.sha256,
                    "{}",
                    f.rel_path
                );
            }
        }
    }

    #[test]
    fn a_bundled_model_is_copied_in_only_when_it_matches() {
        let root = scratch("bundled");
        let (bad, good, models) = (root.join("bad"), root.join("good"), root.join("models"));
        let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/models");
        for f in find("faces").unwrap().files {
            let dest = bad.join(f.rel_path);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(&dest, b"not the model").unwrap();
            let dest = good.join(f.rel_path);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(vendor.join(f.rel_path), &dest).unwrap();
        }

        // A damaged copy is passed over rather than installed.
        assert!(!install_bundled(
            std::slice::from_ref(&bad),
            &models,
            "faces"
        ));
        assert!(!installed(find("faces").unwrap(), &models));
        // The first good source wins, and a second call is a no-op.
        assert!(install_bundled(
            &[bad.clone(), good.clone()],
            &models,
            "faces"
        ));
        assert!(installed(find("faces").unwrap(), &models));
        assert!(install_bundled(&[], &models, "faces"));
        // A model whose files are not in the sources stays a download.
        assert!(!install_bundled(&[good], &models, "people"));
    }
}
