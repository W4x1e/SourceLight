//! Exercises preview extraction and EXIF parsing against real ARW files.
//!
//! Raw files are far too large to commit, so this test is opt-in: point
//! `SOURCELIGHT_TEST_ARW` at a folder of .ARW files to run it.
//!
//! ```sh
//! SOURCELIGHT_TEST_ARW=/path/to/raws cargo test --test arw_pipeline -- --nocapture
//! ```

use sourcelight_lib::test_support::{self, Needs};
use sourcelight_lib::{exif_read, raw, scan};
use std::path::PathBuf;

/// Only `.ARW`.
///
/// The test is named for Sony, documents "a folder of .ARW files", and asserts
/// things only true of a TIFF-container raw. Pointed at a mixed folder it
/// picked up a Canon CR3 and failed on it — a real gap, but not this test's,
/// and the panic named the wrong culprit. Filtering keeps that finding where it
/// belongs instead of turning this into a general metadata test by accident.
fn is_arw(p: &std::path::Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("arw"))
}

/// Its own variable first, then the shared raw-fixture folder — the manifest
/// carries a Sony a6400 frame, so `check.sh --raw` and CI need not set both.
fn sample_dir() -> Option<PathBuf> {
    test_support::fixture_path("SOURCELIGHT_TEST_ARW").or_else(test_support::raw_dir)
}

#[test]
fn extracts_preview_and_metadata_from_real_arws() {
    let Some(dir) = sample_dir() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_ARW (or SOURCELIGHT_TEST_RAW) to a folder of .ARW files",
        );
        return;
    };

    let files: Vec<_> = scan::find_photo_files(&dir)
        .into_iter()
        .filter(|p| is_arw(p))
        .collect();
    if files.is_empty() {
        test_support::skip(Needs::Raw, &format!("no .ARW files in {}", dir.display()));
        return;
    }

    // A handful is enough to prove the format is handled.
    for path in files.iter().take(5) {
        let data = std::fs::read(path).expect("read arw");

        let preview = raw::extract_preview(&data)
            .unwrap_or_else(|e| panic!("no preview in {}: {e}", path.display()));
        assert!(
            preview.len() > 10_000,
            "preview from {} suspiciously small ({} bytes)",
            path.display(),
            preview.len()
        );
        assert_eq!(&preview[..2], &[0xFF, 0xD8], "preview is not a JPEG");

        let meta = exif_read::read(&data);
        assert!(
            meta.capture_date.is_some(),
            "no capture date in {}",
            path.display()
        );
        // Normalised to a sortable form: "YYYY-MM-DD HH:MM:SS".
        let date = meta.capture_date.unwrap();
        assert_eq!(date.len(), 19, "unexpected date format: {date}");
        assert_eq!(date.as_bytes()[4], b'-');
        assert!((1..=8).contains(&meta.orientation), "bad orientation");

        // The preview must actually decode as an image.
        let img = image::load_from_memory(preview).expect("preview decodes");
        assert!(img.width() > 0 && img.height() > 0);

        eprintln!(
            "{}: {}x{} preview, {} bytes, {}",
            path.file_name().unwrap().to_string_lossy(),
            img.width(),
            img.height(),
            preview.len(),
            date
        );
    }
}
