//! Diagnostic: where does middle grey actually sit in normalised sensor data?
//!
//! `MIDDLE_GREY_IN` anchors the whole tone curve. It is expressed in *our*
//! normalised units, where 1.0 is sensor saturation — which is **not** the same
//! as the 0.18 of "18% grey". 18% is reflectance relative to a diffuse white
//! card, and a camera places diffuse white well below saturation to keep
//! headroom for speculars. Conflating the two renders every photo dark.
//!
//! Rather than argue about the right number, this measures it: for each raw,
//! solve for the anchor that would make the neutral curve reproduce the
//! camera's own rendering of the same file.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --test middle_grey -- --nocapture
//! ```

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::tone::ToneParams;
use sourcelight_lib::raw::decoder::{CfaPattern, RawDecoder};
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};
use std::path::PathBuf;

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

fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[test]
fn measure_middle_grey_against_camera_renderings() {
    let files = raws(40);
    if files.is_empty() {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    }

    let decoder = LibRawDecoder::new();
    let tone = ToneParams::default();
    let mut implied = Vec::new();

    for path in &files {
        let Ok(raw) = decoder.decode(path) else {
            continue;
        };

        // The **median** green photosite, not the mean.
        //
        // This matters. A tone curve is monotonic, so `median(f(X)) ==
        // f(median(X))` exactly — solving against medians is an identity.
        // Means have no such property: `E[f(X)] != f(E[X])` for a curved f, so
        // matching them conflates the anchor with the curve's shape and biases
        // the answer by a noticeable fraction of a stop.
        // No white-balance factor: the mosaic pass normalises the multipliers
        // against green, so green is exactly 1.0 and this is already the value
        // the shader sees.
        let a = raw.active_area;
        let black = raw.black[1] as f32;
        let range = (raw.white[1] as f32 - black).max(1.0);
        let mut samples: Vec<f32> = Vec::new();
        let period = raw.cfa_pattern.period();
        for ty in (0..a.height.saturating_sub(period)).step_by(period as usize * 8) {
            for tx in (0..a.width.saturating_sub(period)).step_by(period as usize * 8) {
                for dy in 0..period {
                    for dx in 0..period {
                        if raw.cfa_pattern.color_at(tx + dx, ty + dy) != 1 {
                            continue;
                        }
                        if let Some(v) = raw.at(a.x + tx + dx, a.y + ty + dy) {
                            samples.push(((v as f32 - black) / range).max(0.0));
                        }
                    }
                }
            }
        }
        if samples.is_empty() {
            continue;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let scene_mean = samples[samples.len() / 2];

        // What the camera itself made of the same scene, in linear light.
        let Ok(bytes) = decoder.embedded_preview(path) else {
            continue;
        };
        let Ok(preview) = image::load_from_memory(&bytes) else {
            continue;
        };
        // Green rather than luma, to match the scene proxy channel-for-channel;
        // and again the median rather than the mean.
        let preview = preview.to_rgb8();
        let mut greens: Vec<f64> = preview
            .pixels()
            .map(|p| srgb_to_linear(p.0[1] as f64 / 255.0))
            .collect();
        greens.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let display_mean = greens[greens.len() / 2];

        // Solve for the anchor that makes our neutral curve land on the
        // camera's own rendering. Bisection over a generous range; the curve is
        // monotonic in the anchor, so this is well behaved.
        let (mut lo, mut hi) = (0.005f32, 0.60f32);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            let rendered = tone.map_stops((scene_mean / mid).log2()) as f64;
            // A larger anchor means the scene sits lower relative to it, so the
            // render gets darker.
            if rendered > display_mean {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let anchor = 0.5 * (lo + hi);
        let calibrated = CALIBRATED_ON.iter().any(|m| raw.model.contains(m));
        // A DNG says where it wants to sit (`BaselineExposure`), and a
        // monochrome sensor's every site is unfiltered — see the guard below.
        let monochrome = raw.cfa_pattern == CfaPattern::Bayer([1; 4]);
        implied.push((anchor, calibrated, raw.baseline_exposure, monochrome));

        eprintln!(
            "{:<22} scene green mean {scene_mean:.4}  camera {display_mean:.4} linear  \
             -> implied anchor {anchor:.4}  (baseline exposure {:+.2})",
            path.file_name().unwrap().to_string_lossy(),
            raw.baseline_exposure
        );
    }

    assert!(!implied.is_empty(), "no files could be measured");
    let current = sourcelight_lib::develop::tone::MIDDLE_GREY_IN;
    let median = |mut v: Vec<f32>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let all: Vec<f32> = implied.iter().map(|(a, ..)| *a).collect();
    eprintln!(
        "\n{} files: implied anchor median {:.4}, range {:.4}..{:.4}\n\
         current MIDDLE_GREY_IN = {current}",
        all.len(),
        median(all.clone()),
        all.iter().cloned().fold(f32::MAX, f32::min),
        all.iter().cloned().fold(f32::MIN, f32::max),
    );

    // Where each body puts middle grey is its own metering, and bodies
    // differ: across the six fixture cameras, one frame each, the anchor
    // spans 0.053 to 0.132 — well over a stop — with the white levels right.
    // One constant cannot match all of them, so a mixed set measures the mix,
    // not the calibration. (It used to pass on the fixtures only because the
    // Canon was normalised by its format's ceiling instead of where its sensor
    // saturates, which happened to land its frame beside the constant.)
    //
    // So two guards. Every frame within a stop: a black or white level gone
    // wrong moves a body by far more, and this still catches it on the
    // fixture set. Photos no longer open at this constant — each is matched
    // to its own camera's JPEG (the test below) — so this is a check on the
    // levels, not on how bright anything opens.
    //
    // Measured from where the file asks to sit: a DNG's `BaselineExposure`
    // moves the constant by its stops. Without it a DJI drone is 1.2 stops
    // out, a Leica M10 1.1; with it, the phones, drones and DNG bodies in the
    // fixtures are all within 0.95.
    //
    // Monochrome sensors are set aside. With no colour filter over any site
    // the sensor sits higher for the same metering — the Pentax K-3 III
    // Monochrome's PEF is 1.16 stops out, and the same camera's DNG asks for
    // half a stop less to say so — and a PEF has no `BaselineExposure` to
    // carry the rest.
    for (anchor, _, baseline, monochrome) in &implied {
        if *monochrome {
            continue;
        }
        let expected = current * (-baseline).exp2();
        let off = (expected / anchor).log2().abs();
        assert!(
            off < 1.0,
            "a frame's implied anchor is {anchor:.4}, {off:.2} stops from {expected:.4} — \
             more than any body's metering explains; check its black and white levels"
        );
    }

    // And the constant itself, against the body it was measured on. Tight,
    // because the median estimator is precise there: nine of twelve a6400
    // frames agree within 0.12 stops. An earlier, looser bound of half a stop
    // let a visibly-dark render pass, which is the failure this catches.
    let calibrated: Vec<f32> = implied
        .iter()
        .filter(|(_, c, ..)| *c)
        .map(|(a, ..)| *a)
        .collect();
    if calibrated.len() < MIN_CALIBRATION_FRAMES {
        eprintln!(
            "only {} frame(s) from the calibration body — need {MIN_CALIBRATION_FRAMES} to \
             judge the constant. Point SOURCELIGHT_TEST_RAW at an a6400 shoot to exercise it.",
            calibrated.len()
        );
        return;
    }
    let median = median(calibrated);
    let error_stops = (current / median).log2().abs();
    assert!(
        error_stops < 0.15,
        "MIDDLE_GREY_IN is {current} but the measured anchor is {median:.4} — \
         {error_stops:.2} stops out, so photos will open visibly {} than the camera's own JPEG",
        if current > median {
            "darker"
        } else {
            "brighter"
        }
    );
}

/// The body `MIDDLE_GREY_IN` was measured on.
const CALIBRATED_ON: &[&str] = &["ILCE-6400"];
/// Frames from it needed before its median means anything.
const MIN_CALIBRATION_FRAMES: usize = 5;

/// The median green of a render, in linear light.
fn rendered_median(rgba: &[u8]) -> f32 {
    let lut = sourcelight_lib::raster::srgb_lut8();
    let mut greens: Vec<f32> = rgba.chunks_exact(4).map(|p| lut[p[1] as usize]).collect();
    let mid = greens.len() / 2;
    *greens.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1
}

/// A photo opens where its camera put it. `MIDDLE_GREY_IN` is one body's
/// metering, and the others meter over a stop either side of it (above), so
/// each frame is matched to its own camera's JPEG instead — and this checks
/// the render, not the arithmetic: whatever the tone curve, look profile and
/// colour make of the frame, its median lands on the camera's.
///
/// An edit saved before this (version 1) keeps the brightness it was made
/// against, so it is rendered too, to show what matching moved.
#[test]
fn a_default_render_opens_at_the_cameras_brightness() {
    use sourcelight_lib::develop::EditStack;
    use sourcelight_lib::gpu::pipeline::Pipeline;

    let files = raws(40);
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
    let old = EditStack {
        version: 1,
        ..EditStack::default()
    };

    eprintln!(
        "{:<24} {:>8} {:>8} {:>8} {:>9}",
        "file", "camera", "ours", "v1", "off (EV)"
    );
    let mut worst: Option<(String, f32)> = None;
    for path in &files {
        let Ok(raw) = decoder.decode(path) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(camera) = raw.camera_median else {
            eprintln!("{name:<24} no camera JPEG to match; opens at its baseline exposure");
            continue;
        };
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let (w, h) = session.proxy_size(256);
        let render = |stack: &EditStack| {
            rendered_median(
                &pipeline
                    .render_to_pixels(gpu, &session, stack, w, h)
                    .unwrap(),
            )
        };
        let (ours, v1) = (render(&EditStack::default()), render(&old));
        let off = (ours / camera).log2();
        eprintln!("{name:<24} {camera:>8.4} {ours:>8.4} {v1:>8.4} {off:>+9.2}");
        if worst.as_ref().is_none_or(|w| off.abs() > w.1.abs()) {
            worst = Some((name, off));
        }
    }

    // The pipeline measures a small render of its own and corrects, so every
    // frame lands within the 8-bit steps of its median (~0.03 stops here).
    // The estimate before that correction, from the tone curve alone, was
    // within 0.2 stops on most bodies and 0.6 off on the Panasonic, whose
    // colour moves its median; a bound that loose would not notice the
    // correction going.
    if let Some((name, off)) = worst {
        assert!(
            off.abs() < 0.1,
            "{name} opens {off:+.2} stops from its camera's own JPEG"
        );
    }
}
