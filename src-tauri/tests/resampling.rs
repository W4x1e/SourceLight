//! Does the interactive preview show what the export will produce?
//!
//! The develop pass draws a proxy of roughly 1400px from a 6000px sensor, so
//! each output pixel spans about eighteen photosites. Reading one bilinear tap
//! covers four of them and skips the rest, and the result is not merely
//! "slightly wrong" — the preview *invents* detail. High-contrast edges alias,
//! and single-pixel demosaic fringing survives as isolated coloured dots
//! instead of averaging into its surroundings.
//!
//! Nothing else in the suite can see this. Every other test compares averages
//! or renders at a size where no resampling happens, and undersampling
//! preserves the mean almost exactly while destroying local structure.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --test resampling -- --nocapture
//! ```

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::EditStack;
use sourcelight_lib::gpu::pipeline::Pipeline;
use sourcelight_lib::raw::decoder::RawDecoder;
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};
use std::path::PathBuf;

/// Energy at a two-pixel period, which is where undersampling lands.
///
/// Read as a *ratio* against a properly resampled render, never alone. The
/// direction is what identifies the fault: aliasing adds detail that is not in
/// the photograph and reads high, over-filtering removes real detail and reads
/// low. A plain difference cannot tell those apart, because both simply move
/// pixels away from the reference.
fn high_frequency_energy(img: &image::RgbImage) -> f64 {
    let (w, h) = img.dimensions();
    if w < 3 || h < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            for c in 0..3 {
                let at = |dx: i32, dy: i32| {
                    img.get_pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32)
                        .0[c] as f64
                };
                sum += (4.0 * at(0, 0) - at(-1, 0) - at(1, 0) - at(0, -1) - at(0, 1)).abs();
            }
        }
    }
    sum / ((w - 2) * (h - 2) * 3) as f64
}

/// Up to `limit` raw fixtures, sorted. Empty when none are configured, which the
/// caller reports with `test_support::skip` — a path that is set but missing
/// fails in `raw_dir` instead of quietly returning nothing.
fn raws(limit: usize) -> Vec<PathBuf> {
    let Some(dir) = test_support::raw_dir() else {
        return Vec::new();
    };
    let exts = LibRawDecoder::new().extensions();
    let mut files = test_support::files_in(&dir, |e| exts.contains(&e));
    files.truncate(limit);
    files
}

#[test]
fn the_proxy_neither_aliases_nor_blurs() {
    let files = raws(6);
    if files.is_empty() {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    }
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };

    let decoder = LibRawDecoder::new();
    let pipeline = Pipeline::new(gpu);
    let stack = EditStack::default();
    let mut worst: Option<(String, f64)> = None;

    eprintln!(
        "{:<16} {:>10} {:>12} {:>10}",
        "file", "proxy", "reference", "ratio"
    );

    for path in &files {
        let Ok(raw) = decoder.decode(path) else {
            continue;
        };
        let Ok(session) = pipeline.prepare(gpu, &raw) else {
            continue;
        };

        let to_rgb = |px: Vec<u8>, w: u32, h: u32| {
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, px).unwrap()).to_rgb8()
        };

        // Full resolution then Lanczos3 is what the export path does, so it is
        // both the correct answer and the thing the preview has to agree with
        // for "what you see is what you get" to mean anything.
        let (fw, fh) = (session.width, session.height);
        let Ok(full) = pipeline.render_to_pixels(gpu, &session, &stack, fw, fh) else {
            continue;
        };
        let full = to_rgb(full, fw, fh);

        let (pw, ph) = session.proxy_size(900);
        let Ok(direct) = pipeline.render_to_pixels(gpu, &session, &stack, pw, ph) else {
            continue;
        };
        let direct = to_rgb(direct, pw, ph);
        let reference =
            image::imageops::resize(&full, pw, ph, image::imageops::FilterType::Lanczos3);

        let (a, b) = (
            high_frequency_energy(&direct),
            high_frequency_energy(&reference),
        );
        let ratio = a / b.max(1e-6);
        eprintln!(
            "{:<16} {a:>10.3} {b:>12.3} {:>9.2}x",
            path.file_name().unwrap().to_string_lossy(),
            ratio
        );
        if worst
            .as_ref()
            .is_none_or(|w| (ratio - 1.0).abs() > (w.1 - 1.0).abs())
        {
            worst = Some((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                ratio,
            ));
        }
    }

    let Some((file, ratio)) = worst else {
        panic!("nothing could be compared");
    };
    eprintln!("\nfurthest from the reference: {file} at {ratio:.2}x");

    // A single bilinear tap measured 1.58x on a frame with branches against a
    // blown sky. Box-filtered mips cannot quite reach Lanczos3's sharpness, so
    // the achievable range sits a little under 1.0 — the bound is two-sided
    // because over-blurring is also a real failure, just a gentler one.
    assert!(
        ratio < 1.25,
        "{file}: the proxy has {ratio:.2}x the reference's high-frequency energy — \
         it is undersampling and inventing detail"
    );
    assert!(
        ratio > 0.75,
        "{file}: the proxy has only {ratio:.2}x the reference's high-frequency energy — \
         it is over-filtered and the preview is softer than the export"
    );
}
