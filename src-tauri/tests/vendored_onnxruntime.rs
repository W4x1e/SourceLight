//! The vendored ONNX Runtime is the one `vendor/onnxruntime/VERSION` records.
//!
//! The binaries are fetched per machine, not committed, so a second machine,
//! or a re-fetch, could pick up another build — a mirror's, a CPU-only one, a
//! newer version — and the release would sign it into every user's copy.
//! These compare whatever copy is present with the record; a machine without
//! one has nothing to check. `scripts/verify-installer.sh --ai` checks what
//! the installer actually packs.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn vendor() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/onnxruntime")
}

fn record(key: &str) -> String {
    let text =
        std::fs::read_to_string(vendor().join("VERSION")).expect("vendor/onnxruntime/VERSION");
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("no {key}= line in vendor/onnxruntime/VERSION"))
        .trim()
        .to_string()
}

fn sha256(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn the_binaries_present_match_the_record() {
    for (file, key) in [
        ("onnxruntime.dll", "onnxruntime_dll_sha256"),
        ("DirectML.dll", "directml_dll_sha256"),
        ("libonnxruntime.dylib", "macos_dylib_sha256"),
    ] {
        let path = vendor().join(file);
        if !path.exists() {
            continue;
        }
        let want = record(key);
        assert!(
            !want.is_empty(),
            "{file} is vendored but VERSION has no {key} — record it (see README.md)"
        );
        assert_eq!(
            sha256(&path),
            want,
            "{file} is not the build vendor/onnxruntime/VERSION records — see README.md"
        );
    }
}

#[test]
fn the_record_names_the_version_ort_is_built_for() {
    // `ort = "=2.0.0-rc.10"` targets ONNX Runtime 1.22.0 (README.md).
    assert_eq!(record("version"), "1.22.0");
    // The Mac runs 1.24.4, the newest that `ort` accepts, chosen there for
    // CoreML (README.md): recorded apart so the line above doesn't claim it.
    assert_eq!(record("macos_version"), "1.24.4");
}
