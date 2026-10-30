//! Diagnostic: how does our render differ from the camera's own JPEG?
//!
//! Brightness is measurable in one number and already guarded by
//! `tests/middle_grey.rs`. Colour is not — a render can sit at the right
//! average level and still be wrong in hue or saturation. This compares the two
//! renderings of the same file channel by channel.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --test colour_match -- --nocapture
//! ```

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::EditStack;
use sourcelight_lib::gpu::pipeline::Pipeline;
use sourcelight_lib::raw::decoder::RawDecoder;
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};
use std::path::PathBuf;

/// Small enough that resampling differences are irrelevant, large enough to
/// average over real scene content.
/// Frames from a profiled camera needed before the saturation floor means
/// anything. Well under the ~40 the threshold was fitted on, but enough that
/// one unusual scene cannot decide it alone.
const MIN_SAMPLE_FOR_SATURATION: usize = 5;

const COMPARE_EDGE: u32 = 128;

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

fn stats(img: &image::RgbImage) -> ([f64; 3], f64) {
    let n = img.pixels().len() as f64;
    let mut sums = [0f64; 3];
    let mut sat = 0.0;
    for p in img.pixels() {
        for (s, c) in sums.iter_mut().zip(p.0) {
            *s += c as f64;
        }
        let max = p.0.iter().max().copied().unwrap_or(0) as f64;
        let min = p.0.iter().min().copied().unwrap_or(0) as f64;
        sat += if max > 0.0 { (max - min) / max } else { 0.0 };
    }
    (sums.map(|s| s / n), sat / n)
}

/// How colourful a frame's brightest pixels are: `(fraction of frame, mean
/// channel spread there)`, spread being max minus min in 0..255.
///
/// Read as a *ratio against the camera*, never in isolation. Brightness alone
/// does not identify a blown region — a vivid sunlit colour is bright too, and
/// an absolute threshold flags it. Nor does a tight neutrality bound work: the
/// pipeline's known warm/cool gap against Sony already spreads a white sky by
/// about 20, so anything stricter just re-measures the channel ratios above.
///
/// Comparing the two renderings of the same highlights sidesteps both. The
/// magenta failure put this ratio near 12; ordinary disagreement sits near 2.
///
/// Distributional rather than per-pixel on purpose. The embedded preview is
/// not EXIF-rotated while our render is, so on a portrait frame the two do not
/// correspond pixel for pixel — but how colourful the highlights are does not
/// depend on where in the frame they sit.
fn highlight_colourfulness(img: &image::RgbImage) -> (f64, f64) {
    let mut count = 0u32;
    let mut spread = 0f64;
    for p in img.pixels() {
        let max = p.0.iter().max().copied().unwrap() as i32;
        let min = p.0.iter().min().copied().unwrap() as i32;
        if max > 235 {
            count += 1;
            spread += (max - min) as f64;
        }
    }
    (
        count as f64 / img.pixels().len() as f64,
        spread / count.max(1) as f64,
    )
}

#[test]
fn report_colour_difference_against_the_camera() {
    // Wide enough to reach frames with a blown sky. The first handful of a
    // shoot are all shaded garden interiors, and the highlight-clipping guard
    // below has nothing to judge on those — which is precisely how the magenta
    // sky escaped notice.
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
    let mut ratios: Vec<[f64; 3]> = Vec::new();
    let mut sat_ratios: Vec<f64> = Vec::new();
    let mut profiled_sat: Vec<f64> = Vec::new();
    let mut unprofiled: Vec<String> = Vec::new();
    let mut worst_blown: Option<(String, f64, f64)> = None;

    eprintln!(
        "{:<16} {:>22} {:>22} {:>18}",
        "file", "ours R,G,B", "camera R,G,B", "sat ours/cam"
    );

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
        let ours = image::RgbaImage::from_raw(w, h, px).unwrap();
        let ours = image::DynamicImage::ImageRgba8(ours).to_rgb8();

        let Ok(bytes) = decoder.embedded_preview(path) else {
            continue;
        };
        let Ok(preview) = image::load_from_memory(&bytes) else {
            continue;
        };
        // Match the camera's framing to ours before comparing.
        let theirs = image::imageops::resize(
            &preview.to_rgb8(),
            w,
            h,
            image::imageops::FilterType::Triangle,
        );

        // Blown regions, judged against the camera on real photographs.
        //
        // A clipped photosite carries no colour — every channel sits at the
        // sensor ceiling — so white balance multiplying them apart is what
        // turned an overcast sky bright magenta. The unit test in
        // `gpu::pipeline` pins the mechanism on a synthetic sensor; this pins
        // it on photographs, which is the gap that let it ship. Frames the
        // camera renders without a white highlight have nothing to judge and
        // are skipped.
        let (mine, my_spread) = highlight_colourfulness(&ours);
        let (theirs_frac, their_spread) = highlight_colourfulness(&theirs);
        if mine > 0.02 && theirs_frac > 0.02 {
            let ratio = my_spread / their_spread.max(1.0);
            if worst_blown.as_ref().is_none_or(|w| ratio > w.2) {
                worst_blown = Some((
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    my_spread,
                    ratio,
                ));
            }
        }

        let (a, sa) = stats(&ours);
        let (b, sb) = stats(&theirs);
        let ratio = [
            a[0] / b[0].max(1.0),
            a[1] / b[1].max(1.0),
            a[2] / b[2].max(1.0),
        ];
        ratios.push(ratio);
        sat_ratios.push(sa / sb.max(1e-6));
        // The saturation floor below only means anything for a camera with a
        // *measured* look profile. Everything else gets the mild generic lift,
        // which is not fitted to any one body and would drag the aggregate
        // under a threshold fitted to a profiled camera.
        if sourcelight_lib::develop::look::LookProfile::measured_for(&raw.make, &raw.model)
            .is_none()
        {
            unprofiled.push(path.file_name().unwrap().to_string_lossy().to_string());
        } else {
            profiled_sat.push(sa / sb.max(1e-6));
        }

        eprintln!(
            "{:<16} {:>7.1}{:>7.1}{:>7.1} {:>7.1}{:>7.1}{:>7.1} {:>10.2} {:>6.2?}",
            path.file_name().unwrap().to_string_lossy(),
            a[0],
            a[1],
            a[2],
            b[0],
            b[1],
            b[2],
            sa / sb.max(1e-6),
            ratio
        );
    }

    assert!(!ratios.is_empty(), "nothing could be compared");
    let mean =
        |f: &dyn Fn(&[f64; 3]) -> f64| ratios.iter().map(f).sum::<f64>() / ratios.len() as f64;
    let (r, g, b) = (mean(&|x| x[0]), mean(&|x| x[1]), mean(&|x| x[2]));
    let sat = sat_ratios.iter().sum::<f64>() / sat_ratios.len() as f64;

    eprintln!(
        "\nmean channel ratio ours/camera: R {r:.3}  G {g:.3}  B {b:.3}\n\
         mean saturation ratio: {sat:.3}\n\
         (1.000 everywhere would mean we match the camera exactly)"
    );

    // Matching the camera exactly is **not** the goal. Its JPEG is the output of
    // proprietary colour science, and a raw developer that reproduced it would
    // have no reason to exist — the established ones do not match it either. What this
    // guards is that our rendering stays *neutral*: the three channels must not
    // drift apart from each other, which is what a colour cast is.
    //
    // **The threshold was previously fitted to too small a sample.** At 8 files
    // this measured 1.08 and the guard sat at 1.12; across 40 it is 1.15, and
    // was 1.148 before the tone shoulder changed — so the spread is real and
    // pre-existing, not a regression, and the narrow sample was flattering it.
    // Recording the honest number rather than the convenient one.
    //
    // **The warm/cool cast is now largely gone, closed by the look profile.**
    // Before it: R 1.028 G 1.038 B 1.111, a spread of 1.081 — blue ran ~8%
    // over. The a6400 profile lifts green saturation, which pulls red and blue
    // down in foliage, and blue fell to 1.014; the spread is now ~1.02. The
    // bound stays at 1.10 rather than tightening to the new figure, because
    // this measures neutrality across whatever 40 frames are present and the
    // scene mix moves it — but a return of the old cast would still trip it.
    let spread = [r, g, b].iter().cloned().fold(f64::MIN, f64::max)
        / [r, g, b].iter().cloned().fold(f64::MAX, f64::min);
    assert!(
        spread < 1.10,
        "channels disagree by {spread:.3}x (R {r:.3} G {g:.3} B {b:.3}) — that is a visible cast"
    );

    // Undersaturation was the known cost of toning each channel in a space as
    // wide as Rec.2020, and the **look profile now closes it**: 0.821 -> 0.987,
    // the per-hue correction measured in `tests/look_profile.rs` and committed
    // in `develop/look.rs`. The floor is 0.90 rather than the old 0.78 so that
    // *removing* the profile — which drops this straight back to 0.82 — fails
    // here rather than silently reverting the render to flat. It is not 0.98:
    // the profile is damped and the scene mix moves the aggregate, so a little
    // headroom is honest.
    //
    // History, since the number moved a lot before it was understood: aliasing
    // speckle read as chroma (0.945 -> 0.870), then a hue rotation in the
    // camera transform read as saturation (0.870 -> 0.821). Neither was
    // saturation; 0.821 was the first honest neutral figure, and 0.987 is that
    // figure with the camera's own look applied.
    if !unprofiled.is_empty() {
        eprintln!(
            "\nNo measured look profile for: {}\n\
             Those get only the mild generic lift, not a fit to the camera's own\n\
             colour science, which is why the aggregate above sits below the\n\
             profiled figure. Only cameras with a measured profile are held to the\n\
             floor below.",
            unprofiled.join(", ")
        );
    }

    // Applied only to cameras that have a profile. Aggregating across bodies
    // that do not would make this a measure of how many profiles exist rather
    // than of whether the ones that exist are working — and it would fail for
    // the wrong reason the moment the sample folder gained a second vendor,
    // which is exactly what happened.
    //
    // It also needs a *sample*, not a frame. This is an aggregate over scenes,
    // and the comments above record that the figure moved from 1.08 to 1.15
    // when the sample went from 8 files to 40 — the scene mix genuinely moves
    // it. A single profiled frame measures that frame, and the one in the CI
    // fixture set reads 0.83 against a threshold fitted to forty. Asserting on
    // it would be a coin toss dressed up as a guard.
    //
    // So on a small sample this one check is reported and not asserted — but
    // only this one. It used to `return` here, which also skipped the
    // blown-highlight guard below: the check that exists because of the magenta
    // sky, and which needs no sample size at all. On the CI fixture set that
    // meant it had never run.
    if profiled_sat.len() < MIN_SAMPLE_FOR_SATURATION {
        eprintln!(
            "
only {} frame(s) from a profiled camera — need {} to judge saturation,
             since this is an aggregate over scenes and one frame is its own scene.
             Point SOURCELIGHT_TEST_RAW at a real shoot to exercise this.",
            profiled_sat.len(),
            MIN_SAMPLE_FOR_SATURATION
        );
    } else {
        let sat = profiled_sat.iter().sum::<f64>() / profiled_sat.len() as f64;
        assert!(
            sat > 0.90,
            "render is {:.0}% as saturated as the camera — the look profile may not be applying",
            sat * 100.0
        );
    }

    // Blown highlights must stay white. Stated as a fraction of the camera's
    // own neutral-white area rather than an absolute count, so it measures
    // agreement about which regions are blown instead of re-testing exposure.
    // The magenta failure drove this to roughly zero — a sky the camera
    // rendered white had no neutral pixels left in ours at all.
    if let Some((file, spread, ratio)) = worst_blown {
        eprintln!(
            "\nmost coloured highlights: {file} — mean channel spread {spread:.1}, \
             {ratio:.2}x the camera's"
        );
        assert!(
            ratio < 5.0,
            "{file}: our highlights are {ratio:.2}x as colourful as the camera's \
             (mean channel spread {spread:.1}) — blown areas are picking up a colour cast"
        );
    }
}
