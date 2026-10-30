//! The vendored LibRaw matches what `vendor/libraw/VERSION` says it is.
//!
//! `cargo deny` cannot see a native DLL, so the version record next to it is
//! the only way `scripts/check-libraw.sh` can tell whether we have fallen
//! behind upstream. That only works if the record is true. These tests make it
//! impossible to replace the DLL, the import library or the headers without
//! also updating the record, which is step 3 of docs/LIBRAW.md.
//!
//! No `libraw` feature needed: these read files, they do not load the DLL. The
//! loaded DLL's own version is checked by `raw::libraw::tests`.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn vendor() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/libraw")
}

fn record(key: &str) -> String {
    let text = std::fs::read_to_string(vendor().join("VERSION")).expect("vendor/libraw/VERSION");
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("no {key}= line in vendor/libraw/VERSION"))
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
fn the_binaries_match_the_record() {
    let dll = vendor().join("bin/x86_64-pc-windows-msvc/libraw.dll");
    let lib = vendor().join("lib/x86_64-pc-windows-msvc/libraw.lib");
    assert_eq!(
        sha256(&dll),
        record("dll_sha256"),
        "libraw.dll changed without vendor/libraw/VERSION — see docs/LIBRAW.md"
    );
    assert_eq!(
        sha256(&lib),
        record("lib_sha256"),
        "libraw.lib changed without vendor/libraw/VERSION — see docs/LIBRAW.md"
    );
    // Built here rather than downloaded (scripts/vendor-libraw-macos.sh), so
    // the record is also the only proof of which build is committed.
    let dylib = vendor().join("lib/aarch64-apple-darwin/libraw.dylib");
    assert_eq!(
        sha256(&dylib),
        record("dylib_sha256"),
        "libraw.dylib changed without vendor/libraw/VERSION — see docs/LIBRAW.md"
    );
}

#[test]
fn the_header_matches_the_record() {
    // The bindings are generated from this header, so a DLL rolled forward
    // without its headers would be read through the old struct layouts.
    let header = std::fs::read_to_string(vendor().join("include/libraw/libraw_version.h")).unwrap();
    let define = |name: &str| {
        header
            .lines()
            .find_map(|l| {
                let rest = l.trim().strip_prefix("#define ")?.strip_prefix(name)?;
                // `LIBRAW_MAJOR_VERSION` must not match `LIBRAW_MAJOR_VERSIONX`.
                rest.starts_with(char::is_whitespace)
                    .then(|| rest.trim().to_string())
            })
            .unwrap_or_else(|| panic!("no #define {name} in libraw_version.h"))
    };
    let from_header = format!(
        "{}.{}.{}",
        define("LIBRAW_MAJOR_VERSION"),
        define("LIBRAW_MINOR_VERSION"),
        define("LIBRAW_PATCH_VERSION"),
    );
    // A lettered re-release (0.21.5b) keeps the numeric header of its base.
    let recorded = record("version");
    let numeric = recorded.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    assert_eq!(from_header, numeric, "headers and VERSION disagree");
}

#[test]
fn notice_names_the_recorded_version() {
    // NOTICE is what the LGPL obliges us to keep accurate.
    let notice =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../NOTICE")).unwrap();
    for package in [record("package"), record("macos_package")] {
        assert!(
            notice.contains(&package),
            "NOTICE does not mention {package} — update its LibRaw section"
        );
    }
}
