//! Downloading and installing the vision model's runtime: llama.cpp's
//! `llama-server`, the "local AI runtime" of Settings → AI.
//!
//! `sidecar.rs` runs the server, `tagger::llama` talks to it, and `vlm.rs`
//! fetches the model it runs. This module obtains the server itself, in the
//! same shape as the model downloader: an official, pinned release archive,
//! fetched on request into `<app data>/llama-runtime`, so the base install stays
//! lean and only users who turn on detailed descriptions pay for it.
//!
//! **Why llama.cpp's own release, not Ollama's.** The app used to
//! download Ollama (1.46 GB on Windows, mostly CUDA 12 and 13 libraries) and
//! talk to its API. Measured on the same photos, that API was the slow part:
//! it made every Qwen3-VL image cost at least 1,024 tokens, ran Qwen3-VL one
//! photo at a time, skipped integrated GPUs and used half the CPU's cores.
//! The official Vulkan build runs every Windows GPU — NVIDIA, AMD, Intel — as
//! fast as Ollama's CUDA did, in a 33 MB archive; the macOS build uses Metal.
//! See `docs/AI-MODEL-SPIKES.md`.
//!
//! **Licence.** llama.cpp is MIT; its GitHub release assets are the project's
//! own builds, so redistribution and use are unencumbered.
//!
//! **Pinned, not "latest".** One build is pinned in [`PINNED`] with
//! each platform's asset and its SHA-256, the download URL is built directly
//! (no API call), and the digest is always checked. llama.cpp releases several
//! times a day; moving to a newer one is a deliberate edit and a re-test, like
//! any other dependency.
//!
//! Like `ai_models.rs`, this module is plain HTTP + files + archive extraction —
//! **no `ort`/AI dependency** — so it compiles and works in a base build.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The llama.cpp build every runtime is installed from: the one the llama-server
/// spike measured on Windows (Vulkan, CPU) and the M4 (Metal).
pub const LLAMA_BUILD: &str = "b11234";

/// One platform's runtime archive in the pinned release.
struct PinnedAsset {
    /// `std::env::consts::OS`.
    os: &'static str,
    /// `std::env::consts::ARCH`.
    arch: &'static str,
    /// The asset's file name in the release.
    name: &'static str,
    /// Its size in bytes, so disk-space checks and progress have a real figure
    /// before the stream starts.
    size: u64,
    /// Lower-case hex SHA-256, from the release's own asset digest at the
    /// time of pinning (2026-09-29).
    sha256: &'static str,
}

/// The runtime for each supported platform, at [`LLAMA_BUILD`]. Windows on x64
/// takes the Vulkan build, which carries the CPU backends too, so a PC with no
/// usable GPU falls back within the same files; Windows on ARM has only a CPU
/// build. The Mac is Apple silicon only, as the app is.
const PINNED: &[PinnedAsset] = &[
    PinnedAsset {
        os: "windows",
        arch: "x86_64",
        name: "llama-b11234-bin-win-vulkan-x64.zip",
        size: 33_058_657,
        sha256: "a8a2a0f18d778a9efe0dd75bfce0efa29d4110bf4569c724b2f92846733f8a54",
    },
    PinnedAsset {
        os: "windows",
        arch: "aarch64",
        name: "llama-b11234-bin-win-cpu-arm64.zip",
        size: 12_042_683,
        sha256: "e0c3f6f0ad3ab2edf9e5047dbb60cac70e8024ba3c50293fe3e0a35b445b903e",
    },
    PinnedAsset {
        os: "macos",
        arch: "aarch64",
        name: "llama-b11234-bin-macos-arm64.tar.gz",
        size: 11_757_756,
        sha256: "ee87c0ef14d224a408f8fa409c355f6734d2b34a899afe25a25975b275cc5094",
    },
];

/// Which archive a release asset is, decided from its file name. This — not a
/// platform `#[cfg]` — is what selects the extractor, so the choice stays
/// data-driven and unit-testable, and a future asset-naming change is a one-line
/// edit here rather than scattered conditionals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    /// `.zip` — how llama.cpp ships its Windows builds.
    Zip,
    /// `.tgz` / `.tar.gz` — how it ships macOS and Linux.
    TarGz,
}

impl Archive {
    /// The archive kind implied by a file name, or `None` for an unrecognised
    /// extension (so we never try to extract something we can't).
    fn from_name(name: &str) -> Option<Archive> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".zip") {
            Some(Archive::Zip)
        } else if lower.ends_with(".tgz") || lower.ends_with(".tar.gz") {
            Some(Archive::TarGz)
        } else {
            None
        }
    }
}

/// A resolved, downloadable runtime: where it is, how big, what kind of
/// archive, and the digest it must have — everything `install` needs.
#[derive(Debug, Clone)]
pub struct ResolvedPack {
    pub version: String,
    pub asset_name: String,
    pub url: String,
    /// The asset's byte size, so the UI has a real denominator before the
    /// stream starts.
    pub size: u64,
    pub archive: Archive,
    /// The pinned SHA-256 the archive must hash to before it is extracted.
    pub sha256: String,
}

/// The OS/arch this build targets, in `std::env::consts` spelling.
fn current_target() -> (&'static str, &'static str) {
    (std::env::consts::OS, std::env::consts::ARCH)
}

/// Whether the runtime is installed: unpacked whole, by this build.
///
/// A runtime counted as installed if any `llama-server` was there, so one
/// extracted half-way (a disk filling, an antivirus taking a DLL) was never
/// fixed, and a later release that moves [`LLAMA_BUILD`] — for a fix in the
/// server, or a flag only the new one knows — kept running the old one for
/// ever. Now an install is unpacked aside and moved into place only once
/// whole, with a [`MANIFEST`] naming its build and each file's SHA-256; one
/// without it, or from another build, is missing, and is downloaded again.
/// Finding the binary is still `sidecar::llama_server_binary`'s job, so "what
/// counts as installed" and "what we run" cannot drift apart. Every file the
/// manifest names must still be there, so one taken since the install (an
/// antivirus quarantining a DLL) makes the runtime missing and downloaded
/// again, rather than a server that fails to start every time.
pub fn installed(runtime_dir: &Path) -> bool {
    crate::sidecar::llama_server_binary(runtime_dir).is_some()
        && read_manifest(runtime_dir).is_some_and(|m| {
            m.build == LLAMA_BUILD && m.files.keys().all(|rel| runtime_dir.join(rel).is_file())
        })
}

/// The record an install leaves of itself, beside the runtime.
pub const MANIFEST: &str = "northlight-runtime.json";

/// Where an install is unpacked before it is moved into place, beside the
/// runtime folder.
pub const STAGING: &str = "llama-runtime.staging";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Manifest {
    build: String,
    /// Each file's path under the runtime folder, with `/`, and its SHA-256.
    files: std::collections::BTreeMap<String, String>,
}

fn read_manifest(runtime_dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(runtime_dir.join(MANIFEST)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes the manifest for what `dir` holds: [`LLAMA_BUILD`], and every
/// file's SHA-256.
fn write_manifest(dir: &Path) -> Result<()> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if path.is_file() {
                let bytes =
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, crate::hashing::hex(&Sha256::digest(&bytes)));
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeMap::new();
    walk(dir, dir, &mut files)?;
    let manifest = Manifest {
        build: LLAMA_BUILD.to_string(),
        files,
    };
    std::fs::write(dir.join(MANIFEST), serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("writing the manifest in {}", dir.display()))
}

/// Moves a whole, unpacked install from `staging` to `runtime_dir`,
/// replacing whatever was there.
fn put_in_place(staging: &Path, runtime_dir: &Path) -> Result<()> {
    write_manifest(staging)?;
    if runtime_dir.exists() {
        std::fs::remove_dir_all(runtime_dir)
            .with_context(|| format!("removing the old runtime at {}", runtime_dir.display()))?;
    }
    std::fs::rename(staging, runtime_dir)
        .with_context(|| format!("moving the runtime into {}", runtime_dir.display()))
}

/// The pinned runtime for `os`/`arch`, or an error naming the platform, so the
/// UI can say why rather than offering a dead button. Pure: no network.
fn resolve_for(os: &str, arch: &str) -> Result<ResolvedPack> {
    let pin = PINNED
        .iter()
        .find(|p| p.os == os && p.arch == arch)
        .with_context(|| format!("no local AI runtime is available for {os}/{arch}"))?;
    let archive = Archive::from_name(pin.name)
        .with_context(|| format!("unrecognised archive type for '{}'", pin.name))?;
    Ok(ResolvedPack {
        version: LLAMA_BUILD.to_string(),
        asset_name: pin.name.to_string(),
        url: format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_BUILD}/{}",
            pin.name
        ),
        size: pin.size,
        archive,
        sha256: pin.sha256.to_string(),
    })
}

/// The pinned runtime for this machine.
pub fn resolve() -> Result<ResolvedPack> {
    let (os, arch) = current_target();
    resolve_for(os, arch)
}

/// Rough download size of this machine's runtime, in MB — what the consent
/// dialog quotes. Falls back to the Windows x64 figure, the largest, so the
/// quote is never optimistic.
pub fn download_mb() -> u32 {
    let bytes = resolve().map(|p| p.size).unwrap_or(33_058_657);
    (bytes as f64 / 1_048_576.0).ceil() as u32
}

/// Roughly what the runtime takes on disk once unpacked, in MB, for the
/// Storage list: 91 MB for the Windows Vulkan build, less elsewhere.
pub const INSTALLED_MB: u32 = 91;

/// Progress stages, so the UI bar can label the two phases distinctly: the
/// download, then the extraction.
pub const STAGE_DOWNLOAD: &str = "downloading";
pub const STAGE_EXTRACT: &str = "extracting";

/// Downloads and installs the runtime into `runtime_dir`, reporting progress as
/// `on_progress(stage, received, total)`. The archive streams to a `.part` temp
/// then extracts into place; on success the archive is removed and
/// `llama-server` is present for `sidecar::llama_server_binary` to find.
///
/// Verifies the binary exists after extraction — a truncated or wrong-shaped
/// archive must not read as installed.
pub async fn install(
    runtime_dir: &Path,
    cancel: &crate::jobs::CancelToken,
    mut on_progress: impl FnMut(&str, u64, u64),
) -> Result<String> {
    let pack = resolve()?;
    // The archive is downloaded and then extracted beside it, so at the peak it
    // is on disk twice over and a little more — checked before either.
    crate::hardware::ensure_space(
        runtime_dir,
        pack.size.saturating_mul(3),
        "the local AI runtime",
    )?;
    // Unpacked aside and moved into place only once whole.
    let staging = runtime_dir.with_file_name(STAGING);
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging)
        .await
        .with_context(|| format!("creating {}", staging.display()))?;
    let result = download_and_unpack(&pack, &staging, cancel, &mut on_progress).await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    result?;
    let (from, to) = (staging.clone(), runtime_dir.to_path_buf());
    tokio::task::spawn_blocking(move || put_in_place(&from, &to))
        .await
        .context("install task panicked")??;
    if !installed(runtime_dir) {
        bail!("the runtime archive extracted but no 'llama-server' was found in it");
    }
    on_progress(STAGE_EXTRACT, 1, 1);
    Ok(pack.version)
}

/// Downloads the archive into `staging`, checks it, and unpacks it there.
async fn download_and_unpack(
    pack: &ResolvedPack,
    staging: &Path,
    cancel: &crate::jobs::CancelToken,
    on_progress: &mut impl FnMut(&str, u64, u64),
) -> Result<()> {
    // --- download to a .part temp ---
    let archive_path = staging.join(match pack.archive {
        Archive::Zip => "llama-runtime.zip",
        Archive::TarGz => "llama-runtime.tgz",
    });
    let tmp = {
        let mut s = archive_path.as_os_str().to_owned();
        s.push(".part");
        PathBuf::from(s)
    };

    let client = reqwest::Client::builder()
        .user_agent("SourceLight")
        .build()
        .context("building the download client")?;
    let mut resp = client
        .get(&pack.url)
        .send()
        .await
        .with_context(|| format!("requesting {}", pack.url))?
        .error_for_status()
        .with_context(|| format!("downloading {}", pack.url))?;
    // Prefer the streamed Content-Length; fall back to the pinned size.
    let total = resp.content_length().unwrap_or(pack.size).max(1);
    let mut hasher = Sha256::new();

    {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("creating {}", tmp.display()))?;
        let mut received: u64 = 0;
        let mut last_emit: u64 = 0;
        on_progress(STAGE_DOWNLOAD, 0, total);
        // Cancellable while downloading only: that is most of the wait, and
        // it stops even while nothing is arriving. Once extraction starts it
        // runs to the end, so a cancel can never leave a half-unpacked
        // runtime that looks installed. Cancelled or stalled, `install`
        // removes the staging folder, `.part` and all.
        while let Some(chunk) = crate::ai_models::next_chunk(&mut resp, cancel).await? {
            let chunk = chunk.as_ref();
            file.write_all(chunk).await.context("writing a chunk")?;
            hasher.update(chunk);
            received += chunk.len() as u64;
            // Throttle to ~every 2 MB so the download doesn't flood the UI.
            if received - last_emit >= 2 * 1_048_576 {
                last_emit = received;
                on_progress(STAGE_DOWNLOAD, received, total.max(received));
            }
        }
        file.flush().await.context("flushing")?;
    }

    // Integrity: refuse to extract or run an archive whose bytes don't match the
    // pinned digest. Always checked — there is no "digest unavailable" case.
    let got = crate::hashing::hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(&pack.sha256) {
        let _ = tokio::fs::remove_file(&tmp).await;
        eprintln!(
            "integrity check failed for {}: expected sha256 {}, got {got}",
            pack.asset_name, pack.sha256
        );
        bail!(
            "The download of {} didn't arrive intact — try again",
            pack.asset_name
        );
    }

    tokio::fs::rename(&tmp, &archive_path)
        .await
        .with_context(|| format!("finalising {}", archive_path.display()))?;

    // --- extract (blocking, CPU/IO heavy) off the async runtime ---
    on_progress(STAGE_EXTRACT, 0, 1);
    let dest = staging.to_path_buf();
    let archive_for_extract = archive_path.clone();
    let kind = pack.archive;
    tokio::task::spawn_blocking(move || extract(&archive_for_extract, &dest, kind))
        .await
        .context("extraction task panicked")?
        .context("extracting the runtime archive")?;

    // The archive itself is no longer needed once extracted.
    let _ = tokio::fs::remove_file(&archive_path).await;
    if crate::sidecar::llama_server_binary(staging).is_none() {
        bail!("the runtime archive extracted but no 'llama-server' was found in it");
    }
    Ok(())
}

/// Extracts an archive into `dest`, dispatching on the detected kind. Both paths
/// guard against traversal (a "zip-slip" entry like `../../evil`), so a malformed
/// archive can never write outside `dest`.
fn extract(archive: &Path, dest: &Path, kind: Archive) -> Result<()> {
    match kind {
        Archive::Zip => extract_zip(archive, dest),
        Archive::TarGz => extract_tar_gz(archive, dest),
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("reading the zip index")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("reading a zip entry")?;
        // `enclosed_name` returns None for any path that would escape the root.
        let Some(rel) = entry.enclosed_name() else {
            bail!(
                "zip entry '{}' has an unsafe path",
                entry.name().unwrap_or_default()
            );
        };
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut writer =
            std::fs::File::create(&out).with_context(|| format!("writing {}", out.display()))?;
        std::io::copy(&mut entry, &mut writer)
            .with_context(|| format!("extracting {}", out.display()))?;
        set_unix_mode(&out, entry.unix_mode());
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    // `unpack` refuses to write outside `dest`, giving the same traversal
    // guarantee as the zip path, and it preserves entry permissions (the Unix
    // executable bit on `llama-server`) and the macOS build's dylib symlinks.
    tar.unpack(dest)
        .with_context(|| format!("unpacking into {}", dest.display()))?;
    Ok(())
}

/// Restores an entry's Unix permission bits after a zip extract (the zip format
/// stores them, but `File::create` doesn't apply them). Notably this re-sets the
/// executable bit on `llama-server`. A no-op on Windows, where the mode is
/// meaningless.
#[cfg(unix)]
fn set_unix_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        // The permission bits only: an archive's setuid or setgid bit is never
        // applied (RUSTSEC-2026-0367 is that mistake in zip's own extract).
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777));
    }
}

#[cfg(not(unix))]
fn set_unix_mode(_path: &Path, _mode: Option<u32>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_kind_is_read_from_the_extension() {
        assert_eq!(
            Archive::from_name("llama-b11234-bin-win-vulkan-x64.zip"),
            Some(Archive::Zip)
        );
        assert_eq!(
            Archive::from_name("llama-b11234-bin-macos-arm64.tar.gz"),
            Some(Archive::TarGz)
        );
        assert_eq!(Archive::from_name("x.tgz"), Some(Archive::TarGz));
        assert_eq!(Archive::from_name("llama-server.exe"), None);
    }

    #[test]
    fn every_pin_is_a_readable_archive_of_the_pinned_build_with_a_digest() {
        for p in PINNED {
            assert!(
                Archive::from_name(p.name).is_some(),
                "{} is not an archive the extractor reads",
                p.name
            );
            assert!(
                p.name.starts_with(&format!("llama-{LLAMA_BUILD}-")),
                "{} is not from {LLAMA_BUILD}",
                p.name
            );
            assert!(
                crate::hashing::is_sha256_hex(p.sha256)
                    && p.sha256.bytes().all(|b| !b.is_ascii_uppercase()),
                "{} has no lower-case SHA-256",
                p.name
            );
            assert!(p.size > 0);
        }
    }

    #[test]
    fn every_shipping_platform_resolves_to_the_pinned_release() {
        let win = resolve_for("windows", "x86_64").unwrap();
        assert_eq!(win.asset_name, "llama-b11234-bin-win-vulkan-x64.zip");
        assert_eq!(
            win.url,
            format!(
                "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_BUILD}/llama-b11234-bin-win-vulkan-x64.zip"
            )
        );
        assert_eq!(win.archive, Archive::Zip);
        assert_eq!(
            resolve_for("windows", "aarch64").unwrap().asset_name,
            "llama-b11234-bin-win-cpu-arm64.zip"
        );
        let mac = resolve_for("macos", "aarch64").unwrap();
        assert_eq!(mac.archive, Archive::TarGz);
        // An unsupported platform is an error, not a wrong asset.
        assert!(resolve_for("macos", "x86_64").is_err());
        assert!(resolve_for("linux", "x86_64").is_err());
    }

    #[test]
    fn the_consent_quote_is_the_real_size() {
        // A few tens of MB, where Ollama's Windows runtime was 1.4 GB.
        let mb = download_mb();
        assert!((10..=40).contains(&mb), "{mb} MB");
    }

    /// Installed means unpacked whole, by this build: a server binary
    /// alone, or one from another build, is a runtime still to download.
    #[test]
    fn installed_means_this_build_unpacked_whole() {
        let scratch = crate::test_support::Scratch::new("llama-pack-installed");
        let root = scratch.join("llama-runtime");
        let staging = scratch.join(STAGING);
        let exe = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };

        assert!(!installed(&root));
        // The macOS archive unpacks into a folder named for the build.
        std::fs::create_dir_all(root.join("llama-b11234")).unwrap();
        std::fs::write(root.join("llama-b11234").join(exe), b"x").unwrap();
        assert!(
            !installed(&root),
            "no manifest: half-unpacked, or from before them"
        );

        std::fs::create_dir_all(staging.join("llama-b11234")).unwrap();
        std::fs::write(staging.join("llama-b11234").join(exe), b"y").unwrap();
        put_in_place(&staging, &root).unwrap();
        assert!(installed(&root));
        assert!(!staging.exists());
        let manifest = read_manifest(&root).unwrap();
        assert_eq!(manifest.files.len(), 1, "{:?}", manifest.files);

        // Another build's install is missing.
        let text = std::fs::read_to_string(root.join(MANIFEST))
            .unwrap()
            .replace(LLAMA_BUILD, "b1");
        std::fs::write(root.join(MANIFEST), text).unwrap();
        assert!(!installed(&root));
    }
}
