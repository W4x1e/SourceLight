//! Fuji X-Trans: the non-Bayer mosaic, and the black level that comes with it.
//!
//! Every other camera in the suite is Bayer and reports its black level in
//! `color.black`. Fuji reports zero there and puts the real figure — 1022 — in
//! LibRaw's black-level *pattern grid*, which `dcraw_process()` would normally
//! fold in for you. This pipeline never calls that, so the grid has to be read
//! here, and nothing in the suite could notice it was not.
//!
//! ```sh
//! SOURCELIGHT_RAF=/path/to/sample.raf cargo test --test xtrans -- --nocapture
//! ```

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::EditStack;
use sourcelight_lib::gpu::pipeline::Pipeline;
use sourcelight_lib::raw::decoder::{CfaPattern, RawDecoder};
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};

/// A Fuji file: `SOURCELIGHT_RAF` if set, otherwise the first `.raf` in the
/// shared raw-fixture folder. The fallback is what makes these run at all under
/// `check.sh --raw` and CI — nothing there sets the Fuji-specific variable, and
/// the manifest has carried an X-T3 frame all along.
fn sample() -> Option<std::path::PathBuf> {
    if let Some(path) = test_support::fixture_path("SOURCELIGHT_RAF") {
        return Some(path);
    }
    let dir = test_support::raw_dir()?;
    test_support::files_in(&dir, |e| e == "raf")
        .into_iter()
        .next()
}

#[test]
fn a_fuji_raf_decodes_as_x_trans_with_its_black_level() {
    let Some(path) = sample() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_RAF to a Fuji .raf file, or SOURCELIGHT_TEST_RAW to a folder with one",
        );
        return;
    };
    let raw = LibRawDecoder::new()
        .decode(&path)
        .expect("RAF should decode");
    eprintln!("{} {} — {}x{}", raw.make, raw.model, raw.width, raw.height);

    let CfaPattern::XTrans(tile) = &raw.cfa_pattern else {
        panic!(
            "a Fuji file should report an X-Trans mosaic, got {:?}",
            raw.cfa_pattern
        );
    };

    // The layout's defining property: green outnumbers red and blue two to
    // one, which is why X-Trans resolves luminance well and why a Bayer
    // demosaic cannot be pointed at it.
    let greens = tile.iter().filter(|&&c| c == 1 || c == 3).count();
    let reds = tile.iter().filter(|&&c| c == 0).count();
    let blues = tile.iter().filter(|&&c| c == 2).count();
    assert_eq!(
        (greens, reds, blues),
        (20, 8, 8),
        "unexpected tile {tile:?}"
    );

    // **The regression this file exists for.** Fuji leaves `color.black` at
    // zero and states 1022 in the pattern grid instead. Reading only the
    // scalar leaves every photosite 1022 too high, which lifts the blacks to a
    // foggy grey and — because white balance then scales an offset that should
    // not be there — throws a heavy magenta cast over the whole frame.
    assert!(
        raw.black.iter().all(|&b| b > 500),
        "black level came back as {:?}; the pattern grid was not read",
        raw.black
    );
    assert!(
        raw.black.iter().zip(raw.white).all(|(&b, w)| b < w / 2),
        "black {:?} is implausibly close to white {:?}",
        raw.black,
        raw.white
    );
}

#[test]
fn an_x_trans_frame_renders_without_a_cast() {
    let Some(path) = sample() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_RAF to a Fuji .raf file, or SOURCELIGHT_TEST_RAW to a folder with one",
        );
        return;
    };
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };

    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");

    let (w, h) = session.proxy_size(600);
    let px = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("render");

    let mut sums = [0f64; 3];
    let mut darkest = 255.0f64;
    for p in px.chunks_exact(4) {
        for (s, c) in sums.iter_mut().zip(&p[..3]) {
            *s += *c as f64;
        }
        darkest = darkest.min((p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0);
    }
    let n = (w * h) as f64;
    let mean = sums.map(|s| s / n);
    eprintln!("mean {mean:?}, darkest pixel {darkest:.1}");

    // Compared against the camera's own preview would be better, but the point
    // here is coarser: an unsubtracted black level throws the channels apart
    // and floors the darkest pixel far above zero. Both were unmistakable —
    // mean [184, 139, 185] against [110, 113, 94] once fixed.
    let spread = mean.iter().cloned().fold(f64::MIN, f64::max)
        - mean.iter().cloned().fold(f64::MAX, f64::min);
    assert!(
        spread < 30.0,
        "channels are {spread:.1} apart ({mean:?}) — that is a cast across the whole frame"
    );
    assert!(
        darkest < 60.0,
        "the darkest pixel is {darkest:.1}; the blacks are lifted, which is what an \
         unsubtracted black level looks like"
    );
}
