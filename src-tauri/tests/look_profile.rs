//! Measures what a per-camera look profile actually has to do.
//!
//! `colour_match` reports the *aggregate* gap against the camera JPEG — one
//! saturation ratio, three channel ratios. That is enough to know a profile is
//! needed and nowhere near enough to author one: a profile is per hue, and a
//! frame average hides which hues are flat and which are already right.
//!
//! This bins every pixel by hue into the mixer's eight bands and compares our
//! render to the camera's, band by band, so the profile's constants are
//! measured rather than guessed — the same discipline `middle_grey` uses for
//! the exposure anchor. It asserts nothing; it prints a table to read.
//!
//! **Grouped by camera**, because a profile is per body and a folder can
//! mix them. A camera with [`MIN_FRAMES_FOR_A_PROFILE`] or more frames gets a
//! ready-to-paste `LookProfile::from_ratios([...])` line; fewer, and it says why
//! not. Every camera without a measured profile is also pooled, which is what
//! the generic fallback (`GENERIC_RATIO` in `develop/look.rs`) is fitted to.
//!
//! Every camera now renders with *some* profile — its own, or the generic
//! lift — so the table measures the residual gap. The candidates undo the
//! profile that was applied, so they are the raw ratios `from_ratios` expects
//! and can be pasted as they are. To author a profile for a new body:
//! point this at 5+ frames of *different scenes* from it, paste the candidate
//! into `develop/look.rs`, and add the model to `for_camera`.
//!
//! For a camera with a measured profile, `sat r` reads near 1.0 on the
//! well-populated bands. Re-run this to recalibrate `develop/look.rs` whenever
//! the tone stage or demosaic changes enough to move upstream saturation.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --release --test look_profile -- --nocapture
//! ```
//!
//! Distributional, not per-pixel: the embedded preview is not EXIF-rotated
//! while our render is, so a portrait frame does not correspond pixel for pixel
//! — but how saturated the blues are does not depend on where they sit.

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::mixer::{BANDS, BAND_HUES, BAND_NAMES};
use sourcelight_lib::develop::{EditStack, LookProfile};
use sourcelight_lib::gpu::pipeline::Pipeline;
use sourcelight_lib::raw::decoder::RawDecoder;
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};
use std::collections::BTreeMap;
use std::path::PathBuf;

const COMPARE_EDGE: u32 = 256;

/// Frames of *different scenes* a camera needs before its ratios are a profile
/// rather than a description of one photograph. A per-hue average over a single
/// frame measures what that frame contains: one frame of a forest "needs" a
/// huge green lift. The same threshold as `colour_match`'s saturation floor.
const MIN_FRAMES_FOR_A_PROFILE: usize = 5;

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

/// Hue in degrees, saturation and value in 0..1, from an 8-bit RGB pixel.
fn hsv(p: &[u8; 3]) -> (f32, f32, f32) {
    let r = p[0] as f32 / 255.0;
    let g = p[1] as f32 / 255.0;
    let b = p[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let s = if max > 0.0 { d / max } else { 0.0 };
    let h = if d < 1e-6 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    ((h + 360.0) % 360.0, s, max)
}

/// Which band a hue sits nearest — the band whose centre it is closest to,
/// wrapping at 360. Nearest-centre rather than the interpolating span, because
/// this is measuring per band, not blending between them.
fn band_of(hue: f32) -> usize {
    let mut best = 0;
    let mut best_d = f32::MAX;
    for (i, &c) in BAND_HUES.iter().enumerate() {
        let d = (hue - c).abs().min(360.0 - (hue - c).abs());
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[derive(Default, Clone, Copy)]
struct Bin {
    count: f64,
    sat: f64,
    val: f64,
}

/// Accumulates per-band saturation and value over a whole image. Pixels too
/// dark or too desaturated to have a meaningful hue are dropped — their hue is
/// noise, and a look profile does not touch near-neutrals anyway (the shader
/// skips `hsv.y < 1e-4`).
fn accumulate(img: &image::RgbImage, bins: &mut [Bin; BANDS]) {
    for p in img.pixels() {
        let (h, s, v) = hsv(&p.0);
        if s < 0.15 || v < 0.10 {
            continue;
        }
        let b = &mut bins[band_of(h)];
        b.count += 1.0;
        b.sat += s as f64;
        b.val += v as f64;
    }
}

/// Per-band totals for one group of frames.
#[derive(Default)]
struct Tally {
    frames: usize,
    ours: [Bin; BANDS],
    theirs: [Bin; BANDS],
    /// The profile the render already had (multiplier minus one, per band).
    /// Multiplying it back in turns the residual gap into the raw one.
    applied: [f32; BANDS],
    /// A pool across cameras, so the candidate is the uniform generic ratio
    /// rather than a per-hue profile for one body.
    pooled: bool,
}

impl Tally {
    fn ratios(&self) -> [f64; BANDS] {
        std::array::from_fn(|i| {
            let os = self.ours[i].sat / self.ours[i].count.max(1.0);
            let ts = self.theirs[i].sat / self.theirs[i].count.max(1.0);
            ts / os.max(1e-6)
        })
    }

    fn print(&self, title: &str) {
        let plural = if self.frames == 1 { "" } else { "s" };
        eprintln!("\n== {title} ({} frame{plural})", self.frames);
        eprintln!(
            "{:<10} {:>8} {:>10} {:>10} {:>9} {:>10} {:>10} {:>9}",
            "band", "hue", "ours sat", "cam sat", "sat r", "ours val", "cam val", "val r"
        );
        eprintln!("{}", "-".repeat(80));
        let ratios = self.ratios();
        for i in 0..BANDS {
            let (o, t) = (self.ours[i], self.theirs[i]);
            let ov = o.val / o.count.max(1.0);
            let tv = t.val / t.count.max(1.0);
            eprintln!(
                "{:<10} {:>6.0}\u{b0} {:>10.3} {:>10.3} {:>9.3} {:>10.3} {:>10.3} {:>9.3}",
                BAND_NAMES[i],
                BAND_HUES[i],
                o.sat / o.count.max(1.0),
                t.sat / t.count.max(1.0),
                ratios[i],
                ov,
                tv,
                tv / ov.max(1e-6),
            );
        }
        // Every band pooled, pixel-weighted: the one number a uniform profile
        // (like the generic fallback) is fitted to.
        let total = |bins: &[Bin; BANDS]| {
            let (n, s) = bins
                .iter()
                .fold((0.0, 0.0), |(n, s), b| (n + b.count, s + b.sat));
            s / f64::max(n, 1.0)
        };
        let (os, ts) = (total(&self.ours), total(&self.theirs));
        eprintln!(
            "{:<10} {:>8} {:>10.3} {:>10.3} {:>9.3}",
            "all hues",
            "",
            os,
            ts,
            ts / os.max(1e-6)
        );
        if self.frames >= MIN_FRAMES_FOR_A_PROFILE && self.pooled {
            // Uniform, so any band's applied lift is the whole profile's.
            let raw = ts / os.max(1e-6) * (1.0 + self.applied[0] as f64);
            eprintln!("\ngeneric candidate: GENERIC_RATIO = {raw:.3}");
        } else if self.frames >= MIN_FRAMES_FOR_A_PROFILE {
            let list: Vec<String> = ratios
                .iter()
                .zip(self.applied)
                .map(|(r, a)| format!("{:.3}", r * (1.0 + a as f64)))
                .collect();
            eprintln!(
                "\ncandidate: LookProfile::from_ratios([{}])",
                list.join(", ")
            );
        } else {
            eprintln!(
                "\nno candidate: {} frame{plural}, and a profile needs \
                 {MIN_FRAMES_FOR_A_PROFILE}+ of different scenes",
                self.frames
            );
        }
    }
}

#[test]
fn measure_the_per_hue_gap_against_the_camera() {
    let files = raws(400);
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
    // By camera, because a profile is per body; pooling a Sony with a Canon
    // would author a profile for neither.
    let mut by_camera: BTreeMap<String, (bool, Tally)> = BTreeMap::new();
    // Every camera without a measured profile, pooled.
    let mut unprofiled = Tally {
        pooled: true,
        ..Default::default()
    };

    for path in &files {
        let Ok(raw) = decoder.decode(path) else {
            continue;
        };
        let Ok(session) = pipeline.prepare(gpu, &raw) else {
            continue;
        };
        let (w, h) = session.proxy_size(COMPARE_EDGE);
        let Ok(px) = pipeline.render_to_pixels(gpu, &session, &EditStack::default(), w, h) else {
            continue;
        };
        let Ok(bytes) = decoder.embedded_preview(path) else {
            continue;
        };
        let Ok(preview) = image::load_from_memory(&bytes) else {
            continue;
        };
        let mine = image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, px).unwrap())
            .to_rgb8();
        let cam = image::imageops::resize(
            &preview.to_rgb8(),
            w,
            h,
            image::imageops::FilterType::Triangle,
        );

        let camera = format!("{} {}", raw.make.trim(), raw.model.trim());
        let profiled = LookProfile::measured_for(&raw.make, &raw.model).is_some();
        let applied = LookProfile::for_camera(&raw.make, &raw.model).saturation;
        let (_, tally) = by_camera.entry(camera).or_insert_with(|| {
            (
                profiled,
                Tally {
                    applied,
                    ..Default::default()
                },
            )
        });
        tally.frames += 1;
        accumulate(&mine, &mut tally.ours);
        accumulate(&cam, &mut tally.theirs);
        if !profiled {
            unprofiled.applied = applied;
            unprofiled.frames += 1;
            accumulate(&mine, &mut unprofiled.ours);
            accumulate(&cam, &mut unprofiled.theirs);
        }
    }

    for (camera, (profiled, tally)) in &by_camera {
        let note = if *profiled {
            "measured profile applied"
        } else {
            "generic lift applied"
        };
        tally.print(&format!("{camera}: {note}"));
    }
    if unprofiled.frames > 0 && by_camera.len() > 1 {
        unprofiled.print("every camera without a measured profile, pooled");
    }
    eprintln!(
        "\n`sat r` is the multiplier our saturation needs to reach the camera's, per band.\n\
         A look profile is the committed, damped version of this column."
    );
}
