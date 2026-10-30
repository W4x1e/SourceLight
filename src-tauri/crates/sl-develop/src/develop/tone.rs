//! The tone stage: scene-referred light to display-referred image.
//!
//! Sensor data is linear and open-ended — a highlight can be twenty times
//! middle grey — while a display tops out at 1.0. Mapping one to the other by
//! clipping is what makes an untoned raw look flat and then blow out. Every
//! camera applies a curve for this reason, and matching that expectation is
//! most of what separates a photograph from a technically-correct rendering.
//!
//! **The curve is evaluated here, on the CPU, into a lookup table.** The shader
//! only samples it. That is deliberate: tone response is the part of the
//! pipeline most likely to be argued over and tuned, and this way every claim
//! about it is a unit test rather than a screenshot. Adding a control later
//! also costs nothing in the shader.

use serde::{Deserialize, Serialize};

/// Entries in the lookup table. 1024 over 16 stops is ~64 samples per stop,
/// far finer than 8-bit output can show, and the shader interpolates between
/// them anyway.
pub const LUT_SIZE: usize = 1024;

/// The table is indexed by stops relative to middle grey rather than by linear
/// value, so precision follows perception: dense in the shadows, sparse in the
/// highlights. A linear index would spend most of its entries on bright values
/// nobody can distinguish.
pub const LUT_MIN_STOPS: f32 = -12.0;
pub const LUT_MAX_STOPS: f32 = 6.0;

/// Where middle grey sits in **normalised sensor units**, where 1.0 is sensor
/// saturation.
///
/// Not 0.18, and the difference is a full stop. "18% grey" is reflectance
/// relative to a *diffuse white* card, and a camera deliberately places diffuse
/// white well below saturation to keep headroom for speculars — so middle grey
/// lands near a tenth of full scale, not a fifth. Anchoring at 0.18 renders
/// every photograph about a stop dark.
///
/// Measured, not assumed: `tests/middle_grey.rs` solves for the anchor that
/// makes this curve reproduce the camera's own rendering of the same file.
/// Across 12 a6400 frames the implied value clusters at a median of 0.0744,
/// with nine of the twelve inside 0.12 stops of each other.
///
/// That measurement compares **medians**, not means. A tone curve is monotonic,
/// so `median(f(X)) == f(median(X))` exactly, while `E[f(X)] != f(E[X])` for
/// anything curved. An earlier mean-based estimate landed at 0.0966 and left
/// every photo about 0.4 stops dark — the estimator was the bug, not the idea.
///
/// Camera-dependent in practice: bodies meter differently, and across the six
/// fixture cameras the implied anchor runs from 0.053 to 0.132, one frame
/// each. A DNG's `BaselineExposure` is applied on top of this
/// (`RawImage::baseline_exposure`); for other raws a per-camera value is a
/// refinement worth making once there are enough frames per body to fit it.
/// The anchor is relative to where the sensor *really* saturates
/// (`raw::levels`), which is what makes the bodies comparable at all.
pub const MIDDLE_GREY_IN: f32 = 0.0744;
/// Where middle grey should land on the display. Holding these equal is what
/// makes a correctly-exposed photo come out correctly exposed.
pub const MIDDLE_GREY_OUT: f32 = 0.18;

/// Slope through middle grey at neutral settings.
///
/// Above 1 on purpose. A slope of exactly 1 reproduces scene contrast
/// faithfully and looks limp, because a display shows a fraction of the
/// dynamic range the eye saw. Every film stock and camera profile does the same.
const BASE_SLOPE: f32 = 1.55;

/// How far the curve can travel above middle grey before flattening, in stops.
///
/// **Solved for, not chosen.** The sigmoid below only *approaches* its
/// asymptote, so placing that asymptote near display white does not make the
/// curve reach white — it makes it crawl. At 2.6 the brightest thing a sensor
/// can record, its own saturation point, rendered at 0.625 linear: 207/255.
/// Nothing in any photograph could come out white, so every blown sky was grey
/// and the whole image read as hazy.
///
/// The constraint that fixes it is that sensor saturation must map to display
/// white. Saturation sits `log2(1 / MIDDLE_GREY_IN)` = 3.75 stops above middle
/// grey; white sits `log2(1 / MIDDLE_GREY_OUT)` = 2.47 display stops above it.
/// Requiring `BASE_SLOPE * 3.75 / (1 + BASE_SLOPE * 3.75 / SHOULDER) = 2.47`
/// gives 4.31. `sensor_saturation_renders_as_display_white` pins that
/// relationship, so recalibrating `MIDDLE_GREY_IN` fails the test rather than
/// quietly dimming every highlight again.
///
/// Values above saturation still clamp, which is what should happen — they are
/// blown. The roll-off is gentler than before but still smooth, and the
/// clamping now happens at white instead of two thirds of the way there.
const SHOULDER: f32 = 4.31;
/// The same below middle grey.
///
/// Much longer than the shoulder, and the reason matters: the sigmoid
/// asymptotes at `MIDDLE_GREY_OUT * 2^-TOE`, which is the darkest value the
/// curve can ever produce. At 5.5 that floor is ~13/255 — visibly milky blacks,
/// which reads as haze across every shadow in the frame. A longer toe
/// compresses less and lets deep shadows actually reach black, while still
/// rolling off smoothly near the midtones.
const TOE: f32 = 16.0;

/// How much the tone curve preserves colour ratios rather than toning each
/// channel independently. 0 is fully per-channel, 1 fully ratio-preserving; the
/// shader blends the two by this value (`develop.wgsl::apply_tone`).
///
/// **The tuning knob for tone-stage saturation.** The ratio-preserving path now
/// uses a **max-RGB norm** (not the earlier luminance norm), which is
/// self-bounding: every channel ends at or below the toned max, so nothing is
/// scaled past 1.0 for the final clamp to flatten. That was the bug that kept
/// this at 0 — a luminance norm pushed saturated channels above 1.0 and the
/// clamp ate the difference, so the blend stopped responding. Colours that leave
/// the *display* gamut are now handled by `gpu::color::gamut_map` (a
/// hue-preserving desaturation), not a per-channel clip.
///
/// Toning per channel costs roughly 8% saturation against the camera JPEG (see
/// `tests/colour_match.rs`); preserving the ratios recovers most of that while
/// the max-RGB norm keeps the film-like highlight roll-off. `0.8` leans toward
/// hue fidelity with some per-channel bleach retained — tune against real frames
/// (`tests/colour_match.rs`, `tests/look_profile.rs`), and later expose it as a
/// per-photo control.
pub const CHROMA_PRESERVE: f32 = 0.8;

/// How many stops either side of middle grey the regional controls reach.
const REGION_STOPS: f32 = 4.0;

/// Stops of shift at full highlight/shadow deflection.
///
/// Bounded by monotonicity, not by taste. A smoothstep over `REGION_STOPS` has
/// a peak derivative of `1.5 / REGION_STOPS`, so the regional term can steal at
/// most `REGION_GAIN * 1.5 / REGION_STOPS` from the slope. At the flattest
/// contrast that budget is 0.775, and 1.5 keeps the worst case comfortably
/// positive — a curve that doubles back produces inverted tones and banding.
const REGION_GAIN: f32 = 1.5;

/// The user-facing tone controls, all nominally -1..=1 with 0 as neutral.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToneParams {
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
}

impl Default for ToneParams {
    fn default() -> Self {
        Self {
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl ToneParams {
    /// Maps a scene-linear value to a display-referred one.
    ///
    /// `stops` is `log2(linear / MIDDLE_GREY_IN)`, so 0 is middle grey, +1 is
    /// one stop brighter. Working in stops rather than linear values is what
    /// makes the controls behave evenly across the range.
    pub fn map_stops(&self, stops: f32) -> f32 {
        // Highlights and shadows act only on their own side of middle grey,
        // ramping in smoothly so neither leaves a visible seam at the midpoint.
        let above = smoothstep(0.0, REGION_STOPS, stops);
        let below = smoothstep(0.0, REGION_STOPS, -stops);

        // Exponential rather than linear, so the slope halves at -1 and doubles
        // at +1 and can never reach zero. A linear mapping hits zero slope at
        // -1, which flattens the image to a single grey and — worse — lets the
        // regional terms below invert the curve.
        let slope = BASE_SLOPE * self.contrast.clamp(-1.0, 1.0).exp2();
        let mut y = slope * stops
            + self.highlights.clamp(-1.0, 1.0) * above * REGION_GAIN
            + self.shadows.clamp(-1.0, 1.0) * below * REGION_GAIN;

        // Soft shoulder and toe. A rational sigmoid, so the curve approaches
        // its limits asymptotically and never clips to a flat wall of white.
        let latitude = if y > 0.0 { SHOULDER } else { TOE };
        y /= 1.0 + y.abs() / latitude;

        let linear = MIDDLE_GREY_OUT * y.exp2();

        // Whites and blacks move the endpoints, which is what actually lets a
        // photo reach true black and true white after the soft roll-off above.
        //
        // Signs follow the usual convention, so muscle memory transfers: **positive opens,
        // negative clips**. Raising blacks lifts them off zero; lowering them
        // crushes to true black. Raising whites brings highlights to clipping.
        let black_point = self.blacks.clamp(-1.0, 1.0) * -0.05;
        let white_point = 1.0 + self.whites.clamp(-1.0, 1.0) * -0.4;
        ((linear - black_point) / (white_point - black_point).max(1e-4)).clamp(0.0, 1.0)
    }

    /// Convenience for callers holding a linear value rather than stops.
    pub fn map_linear(&self, linear: f32) -> f32 {
        if linear <= 0.0 {
            return self.map_stops(LUT_MIN_STOPS);
        }
        self.map_stops((linear / MIDDLE_GREY_IN).log2())
    }

    /// The scene value the **neutral** curve renders as `display` (linear,
    /// 0..=1) — the curve run backwards.
    ///
    /// For a photo that arrives already rendered: a JPEG's pixels are
    /// display light, and undoing the neutral curve on the way in turns them
    /// into the scene light the rest of the pipeline expects. At neutral
    /// settings the curve then puts back exactly what the file held, while
    /// exposure, highlights and the rest act on it just as they do on a raw,
    /// roll-off included.
    ///
    /// Closed form, because at neutral settings every regional term is zero
    /// and the sigmoid inverts: `y = u / (1 + |u| / L)` gives
    /// `u = y / (1 - |y| / L)`, defined while `|y| < L`. Black lies below the
    /// toe's reach and returns 0.
    pub fn neutral_inverse(display: f32) -> f32 {
        if display <= 0.0 {
            return 0.0;
        }
        let y = (display.min(1.0) / MIDDLE_GREY_OUT).log2();
        let latitude = if y > 0.0 { SHOULDER } else { TOE };
        let room = 1.0 - y.abs() / latitude;
        if room <= 1e-6 {
            return if y > 0.0 { 1.0 } else { 0.0 };
        }
        let u = y / room;
        MIDDLE_GREY_IN * (u / BASE_SLOPE).exp2()
    }

    /// Stops of exposure that make the neutral curve render a frame whose
    /// median sensor value is `scene` (normalised, 1.0 = saturation) at the
    /// median its camera's own JPEG shows, `camera` (linear display light).
    ///
    /// Bodies meter differently and their JPEGs place the result
    /// differently: against the one constant `MIDDLE_GREY_IN`, an Olympus
    /// opened 0.8 stops brighter than its own JPEG and a Pentax 1.2, and DNG
    /// `BaselineExposure` only partly says so (a Pixel's points the wrong
    /// way). Matching each frame to the picture the photographer saw on the
    /// camera covers every body, and its exposure-compensation and dynamic-
    /// range modes too. Medians, not means: the curve is monotonic, so the
    /// median of the render is the curve of the median.
    ///
    /// The curve is not the whole render — colour moves a frame's median
    /// too — so this is the first estimate, which `Pipeline::open_at_camera`
    /// measures on a render and corrects.
    ///
    /// `None` when either median says nothing — a black frame, a blown one —
    /// and clamped to two stops either way, past which the preview is more
    /// likely a different picture (a creative mode, a stitched panorama)
    /// than a different placement.
    pub fn exposure_matching(scene: f32, camera: f32) -> Option<f32> {
        if !(scene > 1e-5 && (0.002..0.95).contains(&camera)) {
            return None;
        }
        let wanted = Self::neutral_inverse(camera);
        (wanted > 0.0).then(|| (wanted / scene).log2().clamp(-2.0, 2.0))
    }

    /// Bakes the curve into a table the shader can sample.
    pub fn build_lut(&self) -> Vec<f32> {
        (0..LUT_SIZE)
            .map(|i| {
                let t = i as f32 / (LUT_SIZE - 1) as f32;
                self.map_stops(LUT_MIN_STOPS + t * (LUT_MAX_STOPS - LUT_MIN_STOPS))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The neutral curve and its inverse round-trip every display value a
    /// JPEG can hold, so a rendered photo opens looking as it did.
    #[test]
    fn the_neutral_curve_runs_backwards() {
        let neutral = ToneParams::default();
        for i in 1..=255 {
            let display = crate::raster::srgb_to_linear(i as f32 / 255.0);
            let back = neutral.map_linear(ToneParams::neutral_inverse(display));
            assert!(
                (back - display).abs() < 1e-3 + display * 1e-3,
                "sRGB {i}: {display} came back {back}"
            );
        }
        assert_eq!(ToneParams::neutral_inverse(0.0), 0.0);
        // White is the sensor's saturation point, where a raw's white is.
        assert!((ToneParams::neutral_inverse(1.0) - 1.0).abs() < 0.02);
    }

    fn neutral() -> ToneParams {
        ToneParams::default()
    }

    /// The anchor the whole curve hangs off: a correctly exposed mid grey has
    /// to come out as mid grey, or every photo is over- or under-exposed.
    #[test]
    fn middle_grey_maps_to_middle_grey() {
        let out = neutral().map_linear(MIDDLE_GREY_IN);
        assert!(
            (out - MIDDLE_GREY_OUT).abs() < 0.005,
            "middle grey rendered at {out:.4}, expected {MIDDLE_GREY_OUT}"
        );
    }

    /// The other end of the anchor: the brightest value a sensor can record has
    /// to render as white.
    ///
    /// Middle grey alone does not pin a tone curve — it fixes where the curve
    /// passes through, not how far it travels. With too short a shoulder the
    /// curve asymptotes below 1.0 and saturation renders at 207/255, so blown
    /// skies come out grey and every photo looks hazy. Nothing caught that,
    /// because every other test here checks the curve's shape rather than its
    /// range.
    #[test]
    fn sensor_saturation_renders_as_display_white() {
        // 1.0 in normalised sensor units is saturation, by definition.
        let out = neutral().map_linear(1.0);
        assert!(
            out > 0.99,
            "a saturated photosite rendered at {out:.3} ({:.0}/255) instead of white — \
             the shoulder asymptotes below 1.0 and no highlight can reach it",
            out * 255.0
        );
    }

    /// Middle grey must survive the shoulder change above. Held separately from
    /// `middle_grey_maps_to_middle_grey` because the two anchors constrain
    /// different constants and it should be obvious which one moved.
    #[test]
    fn lengthening_the_shoulder_leaves_the_midtones_alone() {
        let t = neutral();
        // A stop either side of grey, where the shoulder has least excuse to
        // interfere.
        for (linear, expected) in [
            (MIDDLE_GREY_IN, 0.18),
            (MIDDLE_GREY_IN * 0.5, 0.18 * 0.5f32.powf(BASE_SLOPE)),
        ] {
            let out = t.map_linear(linear);
            assert!(
                (out - expected).abs() < 0.03,
                "{linear:.4} rendered at {out:.4}, expected about {expected:.4}"
            );
        }
    }

    /// A tone curve that is not monotonic produces posterised bands and
    /// inverted tones. This must hold for every combination of controls, not
    /// just the neutral one.
    #[test]
    fn the_curve_is_monotonic_for_every_setting() {
        let extremes = [-1.0f32, -0.5, 0.0, 0.5, 1.0];
        for &contrast in &extremes {
            for &highlights in &extremes {
                for &shadows in &extremes {
                    for &whites in &extremes {
                        for &blacks in &extremes {
                            let p = ToneParams {
                                contrast,
                                highlights,
                                shadows,
                                whites,
                                blacks,
                            };
                            let lut = p.build_lut();
                            for (i, pair) in lut.windows(2).enumerate() {
                                assert!(
                                    pair[1] >= pair[0] - 1e-6,
                                    "curve decreases at entry {i} ({:.5} -> {:.5}) for {p:?}",
                                    pair[0],
                                    pair[1]
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn output_stays_within_the_display_range() {
        for p in [
            neutral(),
            ToneParams {
                contrast: 1.0,
                highlights: 1.0,
                shadows: 1.0,
                whites: 1.0,
                blacks: 1.0,
            },
            ToneParams {
                contrast: -1.0,
                highlights: -1.0,
                shadows: -1.0,
                whites: -1.0,
                blacks: -1.0,
            },
        ] {
            for v in p.build_lut() {
                assert!((0.0..=1.0).contains(&v), "{v} outside 0..=1 for {p:?}");
            }
        }
    }

    /// Without this the render is flat: scene contrast reproduced literally
    /// looks far duller than the scene did, because the display shows a
    /// fraction of the range the eye saw.
    #[test]
    fn the_neutral_curve_expands_contrast_around_middle_grey() {
        let p = neutral();
        let below = p.map_linear(MIDDLE_GREY_IN / 2.0);
        let above = p.map_linear(MIDDLE_GREY_IN * 2.0);

        // A slope of exactly 1 would put these at half and double middle grey.
        assert!(
            below < MIDDLE_GREY_OUT / 2.0,
            "one stop down rendered at {below:.4}, which is no darker than linear"
        );
        assert!(
            above > MIDDLE_GREY_OUT * 1.6,
            "one stop up rendered at {above:.4}, which is no brighter than linear"
        );
    }

    #[test]
    fn contrast_steepens_the_curve_without_moving_middle_grey() {
        let flat = ToneParams {
            contrast: -0.5,
            ..neutral()
        };
        let punchy = ToneParams {
            contrast: 0.5,
            ..neutral()
        };

        // The pivot must not move, or contrast doubles as an exposure control.
        assert!(
            (flat.map_linear(MIDDLE_GREY_IN) - punchy.map_linear(MIDDLE_GREY_IN)).abs() < 0.005
        );

        assert!(punchy.map_linear(MIDDLE_GREY_IN * 4.0) > flat.map_linear(MIDDLE_GREY_IN * 4.0));
        assert!(punchy.map_linear(MIDDLE_GREY_IN / 4.0) < flat.map_linear(MIDDLE_GREY_IN / 4.0));
    }

    /// Highlights must not quietly darken the shadows, or the controls fight
    /// each other and neither is predictable.
    #[test]
    fn highlights_act_above_middle_grey_only() {
        let p = ToneParams {
            highlights: -1.0,
            ..neutral()
        };
        let n = neutral();

        assert!(
            p.map_linear(MIDDLE_GREY_IN * 8.0) < n.map_linear(MIDDLE_GREY_IN * 8.0) - 0.01,
            "pulling highlights down did not darken the highlights"
        );
        assert!(
            (p.map_linear(MIDDLE_GREY_IN / 8.0) - n.map_linear(MIDDLE_GREY_IN / 8.0)).abs() < 0.002,
            "pulling highlights down also moved the shadows"
        );
    }

    #[test]
    fn shadows_act_below_middle_grey_only() {
        let p = ToneParams {
            shadows: 1.0,
            ..neutral()
        };
        let n = neutral();

        assert!(
            p.map_linear(MIDDLE_GREY_IN / 8.0) > n.map_linear(MIDDLE_GREY_IN / 8.0) + 0.005,
            "lifting shadows did not brighten the shadows"
        );
        assert!(
            (p.map_linear(MIDDLE_GREY_IN * 8.0) - n.map_linear(MIDDLE_GREY_IN * 8.0)).abs() < 0.002,
            "lifting shadows also moved the highlights"
        );
    }

    /// Whites brings *recoverable* highlights up to clipping early.
    ///
    /// This test used to claim the neutral curve should never clip, and checked
    /// it four stops above middle grey — which is past what the sensor can even
    /// record. Asserting that an unrecordable value must not reach white is
    /// what let the shoulder asymptote below 1.0 unnoticed, so the probe now
    /// sits three stops up, inside the sensor's range, where rolling off is
    /// genuinely the right behaviour.
    #[test]
    fn whites_brings_highlights_to_clipping() {
        let bright = MIDDLE_GREY_IN * 8.0;
        assert!(
            neutral().map_linear(bright) < 1.0,
            "a highlight inside the sensor's range should roll off rather than clip"
        );
        let clipped = ToneParams {
            whites: 1.0,
            ..neutral()
        };
        assert_eq!(
            clipped.map_linear(bright),
            1.0,
            "raising whites should drive highlights to full white"
        );
    }

    /// The usual sign convention: negative blacks clip, positive lift. Worth
    /// pinning, because getting it backwards is invisible until someone with
    /// the muscle memory drags the slider the wrong way.
    #[test]
    fn negative_blacks_crush_and_positive_blacks_lift() {
        let dark = MIDDLE_GREY_IN / 64.0;
        let baseline = neutral().map_linear(dark);
        assert!(baseline > 0.0);

        let crushed = ToneParams {
            blacks: -1.0,
            ..neutral()
        };
        assert_eq!(crushed.map_linear(dark), 0.0, "negative blacks should clip");

        let lifted = ToneParams {
            blacks: 1.0,
            ..neutral()
        };
        assert!(
            lifted.map_linear(dark) > baseline,
            "positive blacks should lift shadows off zero"
        );
    }

    /// The bug this guards: contrast mapped linearly reaches zero slope at -1,
    /// which flattens the image and lets the regional terms invert the curve.
    #[test]
    fn the_flattest_contrast_still_has_positive_slope() {
        let p = ToneParams {
            contrast: -1.0,
            ..neutral()
        };
        assert!(
            p.map_linear(MIDDLE_GREY_IN * 4.0) > p.map_linear(MIDDLE_GREY_IN) + 0.01,
            "minimum contrast collapsed the curve to flat"
        );
    }

    #[test]
    fn the_lut_spans_the_configured_range() {
        let lut = neutral().build_lut();
        assert_eq!(lut.len(), LUT_SIZE);
        assert_eq!(lut[0], neutral().map_stops(LUT_MIN_STOPS));
        assert_eq!(lut[LUT_SIZE - 1], neutral().map_stops(LUT_MAX_STOPS));
    }

    /// Zero and negative light can appear after white balance and the colour
    /// matrix; they must resolve to black rather than NaN.
    #[test]
    fn non_positive_light_is_handled() {
        assert!(neutral().map_linear(0.0).is_finite());
        assert!(neutral().map_linear(-0.1).is_finite());
        assert!(neutral().map_linear(0.0) < 0.01);
    }

    /// Deep shadows must reach something an 8-bit display renders as black.
    ///
    /// The toe asymptotes rather than terminating, so the floor is set by
    /// `TOE`. Too short a toe leaves it well above zero and every shadow in
    /// every photo picks up a grey haze — which is exactly what a first
    /// attempt at 5.5 stops did.
    /// The real sRGB transfer function, not a 2.2 power. They differ sharply
    /// near black — sRGB is linear below 0.0031 — which is precisely the region
    /// this test is about.
    fn to_srgb_byte(linear: f32) -> f32 {
        let encoded = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round()
    }

    #[test]
    fn deep_shadows_reach_black() {
        let floor = neutral().map_stops(LUT_MIN_STOPS);
        let as_byte = to_srgb_byte(floor);
        assert!(
            as_byte <= 4.0,
            "darkest tone renders at {as_byte}/255 ({floor:.6} linear) — blacks will look milky"
        );
    }
}
