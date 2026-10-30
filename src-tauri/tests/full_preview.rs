//! The zoom's full-resolution source, on real raws.
//!
//! The point of the feature is that the embedded JPEG is much bigger than the
//! 1600px cached preview. That is a property of each camera's files, not of
//! our code, so it is checked against the real fixture set rather than assumed.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=tests/fixtures/raws cargo test --test full_preview -- --nocapture
//! ```

#![cfg(feature = "libraw")]

use sourcelight_lib::full_preview;
use sourcelight_lib::gpu::pipeline::Pipeline;
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::raw::PREVIEW_EDGE;
use sourcelight_lib::test_support::{self, Needs};

#[test]
fn every_fixture_offers_more_than_the_cached_preview() {
    // Empty, as CI's AI leg sets it, is a skip; a wrong path still panics.
    let Some(dir) = test_support::raw_dir() else {
        test_support::skip(Needs::Raw, "set SOURCELIGHT_TEST_RAW to the raw fixtures");
        return;
    };
    let files = test_support::files_in(&dir, sourcelight_lib::raw::is_supported_extension);
    assert!(!files.is_empty(), "no raw files in SOURCELIGHT_TEST_RAW");

    let mut smaller = Vec::new();
    for path in &files {
        let loaded = full_preview::load(path, 1).expect("embedded preview");
        match loaded {
            Some(bytes) => {
                let img = image::load_from_memory(&bytes).expect("served JPEG decodes");
                eprintln!(
                    "{}: embedded {}x{}, {} KB",
                    path.file_name().unwrap().to_string_lossy(),
                    img.width(),
                    img.height(),
                    bytes.len() / 1024
                );
                assert!(img.width().max(img.height()) > PREVIEW_EDGE);
            }
            None => smaller.push(path.clone()),
        }
    }
    // A camera that embeds only a small preview is allowed — see `render` —
    // but it should be the exception, not the rule.
    assert!(
        smaller.len() * 2 < files.len(),
        "most fixtures should embed a large preview: {smaller:?}"
    );

    // The rest go through the develop pipeline instead, and must come out at
    // sensor resolution — far beyond the cached preview.
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };
    let pipeline = Pipeline::new(gpu);
    let decoder = LibRawDecoder::new();
    for path in &smaller {
        let started = std::time::Instant::now();
        let bytes = full_preview::render(gpu, &pipeline, &decoder, path).expect("render fallback");
        let img = image::load_from_memory(&bytes).expect("rendered JPEG decodes");
        eprintln!(
            "{}: rendered {}x{}, {} KB in {:?}",
            path.file_name().unwrap().to_string_lossy(),
            img.width(),
            img.height(),
            bytes.len() / 1024,
            started.elapsed()
        );
        assert!(img.width().max(img.height()) > 2 * PREVIEW_EDGE);
    }
}
