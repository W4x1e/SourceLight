//! Automatic tone — the "Auto" button.
//!
//! Reads statistics off the rendered image and picks exposure / whites / blacks
//! / highlights / shadows to produce a balanced starting point, the way
//! an editor's Auto button or a classic "auto levels" does. It is deliberately **not**
//! AI: it's a handful of transparent heuristics over a luminance histogram, so
//! every decision is a unit test rather than a model's guess, it runs instantly,
//! and it needs nothing downloaded.
//!
//! The split that keeps it testable: this module is *pure* — pixels in,
//! statistics out, statistics to slider values — with no GPU or session state.
//! The one impure step (rendering the image to get pixels) lives in
//! `session.rs`, which already renders a proxy for the histogram.
//!
//! It only ever writes the five tone fields, never colour, geometry, masks or
//! curves, so pressing Auto refines exposure without disturbing the rest of an
//! edit. White balance is deliberately out of scope for now — gray-world auto-WB
//! neutralises intentionally warm light (a sunset) as readily as a real cast, so
//! it belongs behind its own, separate control.

use super::stack::EditStack;

/// Statistics of a rendered image that auto-tone reasons about — all on the
/// display-encoded output (0..1), which is what the user actually sees and wants
/// balanced. Percentiles rather than min/max, so a single stuck pixel can't drag
/// the black or white point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageStats {
    /// Perceptual-luminance median (the 50th percentile), 0..1 — the midtone
    /// anchor exposure targets.
    pub median: f32,
    /// A low percentile of luminance (the "black point"), 0..1.
    pub low: f32,
    /// A high percentile of luminance (the "white point"), 0..1.
    pub high: f32,
    /// Fraction of pixels crushed to black (0..1).
    pub shadow_clip: f32,
    /// Fraction of pixels blown to white (0..1).
    pub highlight_clip: f32,
}

/// The percentiles used for the black/white points. Not 0/100 — a few stuck or
/// specular pixels shouldn't define the endpoints, so we trim the extremes.
const LOW_PCT: f32 = 0.005; // 0.5%
const HIGH_PCT: f32 = 0.995; // 99.5%

/// Rec.709 luminance weights on the display-encoded RGB. Approximate (true luma
/// wants linear light) but this is a perceptual histogram for a heuristic, and
/// matching what the eye weights is exactly the point.
fn luma(r: u8, g: u8, b: u8) -> f32 {
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

/// Builds [`ImageStats`] from an RGBA8 buffer (the proxy render). Empty or
/// malformed input yields a neutral, do-nothing set of stats so the caller never
/// has to special-case it.
pub fn stats_from_pixels(rgba: &[u8]) -> ImageStats {
    let mut hist = [0u32; 256];
    let mut total = 0u32;
    let (mut shadow, mut high) = (0u32, 0u32);
    for px in rgba.chunks_exact(4) {
        let y = luma(px[0], px[1], px[2]);
        let bin = ((y * 255.0).round() as usize).min(255);
        hist[bin] += 1;
        total += 1;
        if px[0].max(px[1]).max(px[2]) >= 252 {
            high += 1;
        }
        if px[0].min(px[1]).min(px[2]) <= 3 {
            shadow += 1;
        }
    }
    if total == 0 {
        return ImageStats {
            median: 0.5,
            low: 0.0,
            high: 1.0,
            shadow_clip: 0.0,
            highlight_clip: 0.0,
        };
    }

    let percentile = |p: f32| -> f32 {
        let target = (p * total as f32) as u32;
        let mut cum = 0u32;
        for (bin, &count) in hist.iter().enumerate() {
            cum += count;
            if cum >= target {
                return bin as f32 / 255.0;
            }
        }
        1.0
    };

    ImageStats {
        median: percentile(0.5),
        low: percentile(LOW_PCT),
        high: percentile(HIGH_PCT),
        shadow_clip: shadow as f32 / total as f32,
        highlight_clip: high as f32 / total as f32,
    }
}

/// The five tone values auto-tone produces. A partial edit — applied *onto* the
/// existing stack, replacing only these fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoTone {
    pub exposure: f32,
    pub whites: f32,
    pub blacks: f32,
    pub highlights: f32,
    pub shadows: f32,
}

// Targets and gains. All deliberately gentle — Auto should give a sane starting
// point a user then refines, never a finished, aggressive look.
const MEDIAN_TARGET: f32 = 0.40; // a pleasant midtone on the display curve
const WHITE_TARGET: f32 = 0.95; // where the brightest tones should sit
/// The most the white point may be pushed *by exposure* — kept below
/// `WHITE_TARGET` so a positive lift always leaves a margin before clipping.
/// This is the guard against Auto blowing the highlights on a bright scene.
const WHITE_CEILING: f32 = 0.92;
const BLACK_TARGET: f32 = 0.02; // where the darkest tones should sit
const EXPOSURE_LIMIT: f32 = 1.5; // stops
const TONE_LIMIT: f32 = 0.6; // the -1..1 sliders, capped well short of the ends
const WHITE_GAIN: f32 = 0.6;
const BLACK_GAIN: f32 = 1.2;
const HL_GAIN: f32 = 4.0; // clip fractions are small, so this needs weight
const SH_GAIN: f32 = 4.0;
const CLIP_TOLERANCE: f32 = 0.005; // ignore up to 0.5% clipping — some is normal

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

/// Picks tone adjustments from image statistics.
///
/// - **exposure** lifts/lowers the midtone toward [`MEDIAN_TARGET`] — but a
///   *positive* lift is capped to the headroom above the white point
///   ([`WHITE_CEILING`]), so a dark-foreground / bright-sky scene is not brightened
///   into clipped highlights. A darkening is uncapped (it only helps highlights).
/// - **whites** fill the headroom left *after* the exposure lift, gently;
///   **blacks** deepen when the darkest tone is lifted (a hazy, low-contrast
///   image). Together with exposure this is "auto levels" without blowing the top.
/// - **highlights** pull down only when highlights are actually clipping;
///   **shadows** lift only when shadows are actually crushed — recovery, gated on
///   real clipping so a well-exposed frame is left alone.
pub fn auto_tone(s: &ImageStats) -> AutoTone {
    // Exposure toward the midtone target. Floor the median so a near-black frame
    // gives a bounded lift, not infinity.
    let median = s.median.max(0.02);
    let want_ev = (MEDIAN_TARGET / median).log2();
    // A positive lift may not push the white point past WHITE_CEILING — the
    // highlight guard. A negative (darkening) move is not bounded by this.
    let headroom_ev = (WHITE_CEILING / s.high.max(1e-3)).log2();
    let exposure = if want_ev > 0.0 {
        clamp(want_ev.min(headroom_ev.max(0.0)), 0.0, EXPOSURE_LIMIT)
    } else {
        clamp(want_ev, -EXPOSURE_LIMIT, 0.0)
    };

    // Whites: fill only the headroom left after the exposure lift. `2^exposure`
    // over-estimates the display brightening, which biases whites conservatively
    // (toward the safe side of clipping). Only ever raises.
    let post_high = (s.high * 2f32.powf(exposure)).min(1.0);
    let whites = clamp(WHITE_GAIN * (WHITE_TARGET - post_high), 0.0, TONE_LIMIT);
    // Blacks: only ever deepen, and only when the black point is lifted.
    let blacks = clamp(-BLACK_GAIN * (s.low - BLACK_TARGET), -TONE_LIMIT, 0.0);

    // Recovery, gated on real clipping beyond the tolerance.
    let highlights = clamp(
        -HL_GAIN * (s.highlight_clip - CLIP_TOLERANCE).max(0.0),
        -TONE_LIMIT,
        0.0,
    );
    let shadows = clamp(
        SH_GAIN * (s.shadow_clip - CLIP_TOLERANCE).max(0.0),
        0.0,
        TONE_LIMIT,
    );

    AutoTone {
        exposure,
        whites,
        blacks,
        highlights,
        shadows,
    }
}

impl AutoTone {
    /// Writes these values onto a stack, leaving every other field untouched.
    pub fn apply_to(&self, stack: &mut EditStack) {
        stack.exposure = self.exposure;
        stack.whites = self.whites;
        stack.blacks = self.blacks;
        stack.highlights = self.highlights;
        stack.shadows = self.shadows;
    }

    /// The stack whose tone fields are neutral — what we render to *measure* the
    /// scene, so Auto is idempotent (pressing it twice gives the same result)
    /// rather than compounding on its own previous output.
    pub fn neutral_measurement(stack: &EditStack) -> EditStack {
        let mut m = stack.clone();
        m.exposure = 0.0;
        m.whites = 0.0;
        m.blacks = 0.0;
        m.highlights = 0.0;
        m.shadows = 0.0;
        m
    }
}

// ---------------------------------------------------------------------------
// Auto white balance
// ---------------------------------------------------------------------------

/// Average colour of the image's near-neutral mid-tones — the basis for a
/// gray-world white-balance estimate. Averaging *only* low-saturation midtones is
/// what stops a big red car or a blue sky from hijacking the estimate, and it's
/// what keeps auto-WB from wrecking an intentionally warm scene: if there's
/// anything neutral in the frame it anchors the correction to that, not to the
/// overall warm average.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WbStats {
    pub r_mean: f32,
    pub g_mean: f32,
    pub b_mean: f32,
    /// Fraction of the image that counted as a near-neutral midtone. Below
    /// [`MIN_NEUTRAL_FRAC`] there isn't enough neutral reference to trust, so
    /// auto-WB does nothing rather than guess.
    pub neutral_fraction: f32,
}

/// Only mid-tones inform WB: near-black and near-white pixels carry little
/// reliable chroma, and clipping skews it.
const WB_LUMA_LOW: f32 = 0.12;
const WB_LUMA_HIGH: f32 = 0.90;
/// HSV-saturation ceiling for a pixel to count as "near-neutral". A genuine grey
/// sits near 0; anything vivid is excluded so real colours don't set the balance.
/// Set loose enough (0.30) that a strongly cast "neutral" — a white wall under
/// tungsten reads quite orange — still qualifies, which is exactly the case
/// auto-WB most needs to see, while a saturated object stays excluded.
const NEUTRAL_SAT_MAX: f32 = 0.30;
/// Need at least this fraction of the frame to be near-neutral before correcting.
const MIN_NEUTRAL_FRAC: f32 = 0.005;

/// HSV saturation `(max-min)/max` of a display pixel, 0 for a perfect grey.
fn saturation(r: u8, g: u8, b: u8) -> f32 {
    let max = r.max(g).max(b) as f32;
    if max <= 0.0 {
        return 0.0;
    }
    let min = r.min(g).min(b) as f32;
    (max - min) / max
}

/// Averages the near-neutral mid-tones of an RGBA8 buffer for a gray-world
/// estimate. When too little of the frame is neutral, `neutral_fraction` falls
/// below the threshold and [`auto_wb`] declines to correct.
pub fn wb_stats_from_pixels(rgba: &[u8]) -> WbStats {
    let (mut sr, mut sg, mut sb) = (0f64, 0f64, 0f64);
    let mut neutral = 0u32;
    let mut total = 0u32;
    for px in rgba.chunks_exact(4) {
        total += 1;
        let y = luma(px[0], px[1], px[2]);
        if !(WB_LUMA_LOW..=WB_LUMA_HIGH).contains(&y) {
            continue;
        }
        if saturation(px[0], px[1], px[2]) > NEUTRAL_SAT_MAX {
            continue;
        }
        sr += px[0] as f64;
        sg += px[1] as f64;
        sb += px[2] as f64;
        neutral += 1;
    }
    if neutral == 0 || total == 0 {
        return WbStats {
            r_mean: 0.5,
            g_mean: 0.5,
            b_mean: 0.5,
            neutral_fraction: 0.0,
        };
    }
    let n = neutral as f64;
    WbStats {
        r_mean: (sr / n / 255.0) as f32,
        g_mean: (sg / n / 255.0) as f32,
        b_mean: (sb / n / 255.0) as f32,
        neutral_fraction: neutral as f32 / total as f32,
    }
}

/// The white-balance values auto-WB produces — a warmth/tint offset onto the
/// existing stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoWb {
    pub warmth: f32,
    pub tint: f32,
}

// Gains map the log channel-ratios to slider units. Deliberately gentle — an
// early pass felt too aggressive, so these nudge rather than swing; the
// *direction* is exact, magnitude is easy to retune. Both are damped and
// tightly clamped.
const WB_WARM_GAIN: f32 = 0.7;
const WB_TINT_GAIN: f32 = 0.7;
const WB_LIMIT: f32 = 0.35; // well short of the -1..1 slider ends

/// Picks a warmth/tint correction that neutralises the average cast of the
/// near-neutral midtones (gray-world).
///
/// - **warmth**: our slider is amber-positive / blue-negative, so a frame whose
///   neutrals lean red (`r > b`) is cooled (negative) and a blue one warmed.
/// - **tint**: magenta-positive / green-negative, so neutrals leaning green get
///   magenta added back (positive) and vice-versa.
///
/// Returns no change when there wasn't enough neutral reference in the frame —
/// better to leave WB alone than to swing it on a guess.
pub fn auto_wb(s: &WbStats) -> AutoWb {
    if s.neutral_fraction < MIN_NEUTRAL_FRAC {
        return AutoWb {
            warmth: 0.0,
            tint: 0.0,
        };
    }
    let r = s.r_mean.max(1e-3);
    let g = s.g_mean.max(1e-3);
    let b = s.b_mean.max(1e-3);

    // Red-vs-blue imbalance → warmth (cool a warm cast, so negate).
    let warm_error = (r / b).ln();
    let warmth = clamp(-WB_WARM_GAIN * warm_error, -WB_LIMIT, WB_LIMIT);

    // Green vs the geometric mean of red & blue → tint (magenta counters green).
    let tint_error = (g / (r * b).sqrt()).ln();
    let tint = clamp(WB_TINT_GAIN * tint_error, -WB_LIMIT, WB_LIMIT);

    AutoWb { warmth, tint }
}

/// How far from neutral a colour is, as two log ratios: red against blue
/// (what warmth moves) and green against their geometric mean (what tint
/// moves). Both zero for a grey.
fn cast(rgb: [f32; 3]) -> [f32; 2] {
    let [r, g, b] = rgb.map(|v| v.max(1e-4));
    [(r / b).ln(), (g / (r * b).sqrt()).ln()]
}

/// The warmth and tint that make one picked colour neutral: the white-balance
/// eyedropper.
///
/// Unlike [`auto_wb`], which nudges the frame's average cast gently, this is
/// exact: the user has said what grey is. `measure(warmth, tint)` returns the
/// picked point's colour with that balance; Newton's method with a
/// finite-difference Jacobian finds where its cast is zero, halving any step
/// that does not reduce it.
///
/// **Measure in camera space** where you can (see `session::neutral_point`).
/// The sliders act there, as gains on the camera's channels, so warmth moves
/// mostly red against blue and tint green against both, and the steps are
/// well posed. In display RGB a camera matrix can mix them until the two
/// sliders look alike. A cast beyond the sliders' reach — a light no lamp
/// gives, since they move the light by temperature
/// (`develop::white_balance`) — gets the nearest they go.
pub fn neutralise(
    mut measure: impl FnMut(f32, f32) -> anyhow::Result<[f32; 3]>,
) -> anyhow::Result<AutoWb> {
    // Probe steps sized to move the picture about 3% each: tint scales red and
    // blue by a tenth per unit where warmth moves them by a half, and a step
    // smaller than one 8-bit level measures as no change at all.
    const STEP_W: f32 = 0.06;
    const STEP_T: f32 = 0.3;
    const DONE: f32 = 0.003;
    let first = measure(0.0, 0.0)?;
    if first.iter().any(|&v| v <= 1e-4) {
        anyhow::bail!("That spot is too dark to read. Pick something grey and well lit.");
    }
    let miss = |e: [f32; 2]| e[0].hypot(e[1]);

    let (mut w, mut t) = (0.0f32, 0.0f32);
    let mut e = cast(first);
    for _ in 0..10 {
        if e[0].abs() < DONE && e[1].abs() < DONE {
            break;
        }
        // Step inward at the ends of the range so the difference is measured.
        let dw = if w + STEP_W > 1.0 { -STEP_W } else { STEP_W };
        let dt = if t + STEP_T > 1.0 { -STEP_T } else { STEP_T };
        let ew = cast(measure(w + dw, t)?);
        let et = cast(measure(w, t + dt)?);
        let j = [
            [(ew[0] - e[0]) / dw, (et[0] - e[0]) / dt],
            [(ew[1] - e[1]) / dw, (et[1] - e[1]) / dt],
        ];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        let scale = j[0][0].abs().max(j[1][1].abs()).max(1e-6);
        let (sw, st) = if det.abs() > 1e-3 * scale * scale {
            (
                -(j[1][1] * e[0] - j[0][1] * e[1]) / det,
                -(-j[1][0] * e[0] + j[0][0] * e[1]) / det,
            )
        } else {
            // Nearly degenerate: each slider on its own axis.
            (
                -e[0] / j[0][0].abs().max(1e-3).copysign(j[0][0]),
                -e[1] / j[1][1].abs().max(1e-3).copysign(j[1][1]),
            )
        };
        // Halve the step until it helps; stop when nothing does.
        let mut k = 1.0;
        let found = loop {
            let (nw, nt) = (clamp(w + k * sw, -1.0, 1.0), clamp(t + k * st, -1.0, 1.0));
            let ne = cast(measure(nw, nt)?);
            if miss(ne) < miss(e) {
                break Some((nw, nt, ne));
            }
            k *= 0.5;
            if k < 0.1 {
                break None;
            }
        };
        let Some((nw, nt, ne)) = found else { break };
        (w, t, e) = (nw, nt, ne);
    }
    Ok(AutoWb { warmth: w, tint: t })
}

impl AutoWb {
    /// Writes warmth/tint onto a stack, leaving every other field untouched.
    pub fn apply_to(&self, stack: &mut EditStack) {
        stack.warmth = self.warmth;
        stack.tint = self.tint;
    }

    /// The stack with white balance neutral — what we render to *measure* the
    /// cast, so pressing Auto WB reads the scene rather than compounding on its
    /// own previous correction.
    pub fn neutral_measurement(stack: &EditStack) -> EditStack {
        let mut m = stack.clone();
        m.warmth = 0.0;
        m.tint = 0.0;
        m
    }
}

#[cfg(test)]
mod tests {
    // Tests build stacks by `default()` then setting a few fields — clearer than
    // a full struct literal for a large struct.
    #![allow(clippy::field_reassign_with_default)]
    use super::*;

    /// An RGBA buffer of one solid grey level, for driving the stats.
    fn solid(level: u8, n: usize) -> Vec<u8> {
        std::iter::repeat_n([level, level, level, 255], n)
            .flatten()
            .collect()
    }

    /// A buffer split between two levels in a given ratio, for percentile tests.
    fn mix(lo: u8, hi: u8, lo_frac: f32, n: usize) -> Vec<u8> {
        let lo_n = (n as f32 * lo_frac) as usize;
        let mut px = solid(lo, lo_n);
        px.extend(solid(hi, n - lo_n));
        px
    }

    #[test]
    fn stats_are_neutral_for_empty_input() {
        // No pixels → a defined, mid-grey set of stats rather than a panic or NaN.
        let s = stats_from_pixels(&[]);
        assert_eq!(s.median, 0.5);
        assert_eq!(s.low, 0.0);
        assert_eq!(s.high, 1.0);
        // And auto-tone over them stays within its clamps (no NaN/inf).
        let a = auto_tone(&s);
        assert!(a.exposure.abs() <= EXPOSURE_LIMIT);
        assert!(a.whites >= 0.0 && a.blacks <= 0.0);
    }

    #[test]
    fn a_dark_image_gets_brightened() {
        // Median ~0.16 (level 40) — well under the 0.42 target.
        let s = stats_from_pixels(&solid(40, 1000));
        let a = auto_tone(&s);
        assert!(
            a.exposure > 0.5,
            "dark frame should be lifted, got {}",
            a.exposure
        );
    }

    #[test]
    fn a_bright_image_gets_pulled_down() {
        // Median ~0.83 (level 212) — well over target.
        let s = stats_from_pixels(&solid(212, 1000));
        assert!(auto_tone(&s).exposure < -0.5);
    }

    #[test]
    fn a_balanced_image_is_left_roughly_alone() {
        // Median right at the target level (0.42 * 255 ≈ 107), full-ish range.
        let mut px = solid(107, 800);
        px.extend(solid(5, 100)); // a few darks near black
        px.extend(solid(250, 100)); // a few brights near white
        let s = stats_from_pixels(&px);
        let a = auto_tone(&s);
        assert!(a.exposure.abs() < 0.2, "exposure {}", a.exposure);
        // Endpoints already reach the extremes, so no white/black push.
        assert!(a.whites.abs() < 0.05 && a.blacks.abs() < 0.05);
    }

    #[test]
    fn a_dark_scene_with_bright_highlights_is_not_over_lifted() {
        // The failure the before/afters showed: a backlit / high-DR frame —
        // mostly dark, but a bright sky patch already near white. Targeting the
        // dark median would blow the sky, so the highlight guard must hold the
        // exposure lift right down.
        let mut px = solid(50, 800); // dark foreground, median ~0.20
        px.extend(solid(245, 200)); // bright sky ~0.96, 20% of the frame
        let s = stats_from_pixels(&px);
        let a = auto_tone(&s);
        assert!(
            a.exposure < 0.25,
            "bright highlights must cap the lift, got {}",
            a.exposure
        );
        // And whites must not stack more clipping on top.
        assert!(
            a.whites < 0.05,
            "whites should not push a bright top, got {}",
            a.whites
        );
    }

    #[test]
    fn a_dark_scene_with_no_highlights_is_still_lifted() {
        // The counterpart: genuinely dark and dull, nothing bright to protect —
        // here Auto *should* brighten, so the guard mustn't over-neuter it.
        let s = stats_from_pixels(&solid(45, 1000)); // ~0.18, no bright pixels
        assert!(
            auto_tone(&s).exposure > 0.5,
            "a dull dark frame should still lift"
        );
    }

    #[test]
    fn a_dull_low_contrast_image_spreads_to_the_endpoints() {
        // Everything bunched in the mid-greys: whites should rise, blacks deepen.
        let s = stats_from_pixels(&mix(90, 150, 0.5, 1000));
        let a = auto_tone(&s);
        assert!(a.whites > 0.1, "whites should rise, got {}", a.whites);
        assert!(a.blacks < -0.1, "blacks should deepen, got {}", a.blacks);
    }

    #[test]
    fn clipped_highlights_are_recovered_and_crushed_shadows_lifted() {
        // 20% pure white (clipping) and 20% pure black (crushed).
        let mut px = solid(255, 200);
        px.extend(solid(0, 200));
        px.extend(solid(110, 600));
        let s = stats_from_pixels(&px);
        let a = auto_tone(&s);
        assert!(
            a.highlights < 0.0,
            "highlights should recover, got {}",
            a.highlights
        );
        assert!(a.shadows > 0.0, "shadows should lift, got {}", a.shadows);
    }

    #[test]
    fn a_touch_of_clipping_is_tolerated() {
        // Well under the tolerance: no recovery fires.
        let mut px = solid(255, 2); // 0.2% clipped
        px.extend(solid(110, 998));
        let s = stats_from_pixels(&px);
        assert_eq!(auto_tone(&s).highlights, 0.0);
    }

    #[test]
    fn neutral_measurement_zeroes_only_the_tone_fields() {
        let mut stack = EditStack::default();
        stack.exposure = 1.0;
        stack.whites = 0.5;
        stack.contrast = 0.3; // not a tone-auto field — must survive
        stack.warmth = 0.4; // colour — must survive
        let m = AutoTone::neutral_measurement(&stack);
        assert_eq!(m.exposure, 0.0);
        assert_eq!(m.whites, 0.0);
        assert_eq!(m.contrast, 0.3, "contrast is not touched");
        assert_eq!(m.warmth, 0.4, "white balance is not touched");
    }

    #[test]
    fn apply_writes_only_the_five_tone_fields() {
        let mut stack = EditStack::default();
        stack.contrast = 0.25;
        let a = AutoTone {
            exposure: 0.7,
            whites: 0.2,
            blacks: -0.1,
            highlights: -0.3,
            shadows: 0.15,
        };
        a.apply_to(&mut stack);
        assert_eq!(stack.exposure, 0.7);
        assert_eq!(stack.shadows, 0.15);
        assert_eq!(stack.contrast, 0.25, "other fields untouched");
    }

    // --- auto white balance ---

    /// An RGBA buffer of one solid colour.
    fn solid_rgb(r: u8, g: u8, b: u8, n: usize) -> Vec<u8> {
        std::iter::repeat_n([r, g, b, 255], n).flatten().collect()
    }

    #[test]
    fn a_neutral_grey_needs_no_correction() {
        let s = wb_stats_from_pixels(&solid(128, 1000));
        let wb = auto_wb(&s);
        assert!(wb.warmth.abs() < 0.02 && wb.tint.abs() < 0.02, "{wb:?}");
    }

    #[test]
    fn a_warm_cast_is_cooled_and_a_blue_cast_warmed() {
        // Warm near-neutral midtones (r>b, low saturation): should cool (warmth<0).
        let warm = wb_stats_from_pixels(&solid_rgb(150, 130, 118, 1000));
        assert!(auto_wb(&warm).warmth < -0.05, "{:?}", auto_wb(&warm));
        // Blue cast (b>r): should warm (warmth>0).
        let cool = wb_stats_from_pixels(&solid_rgb(118, 130, 150, 1000));
        assert!(auto_wb(&cool).warmth > 0.05);
    }

    #[test]
    fn a_green_cast_gets_magenta_and_a_magenta_cast_gets_green() {
        // Green-leaning neutral (g high): tint should go positive (magenta).
        let green = wb_stats_from_pixels(&solid_rgb(126, 140, 126, 1000));
        assert!(auto_wb(&green).tint > 0.05, "{:?}", auto_wb(&green));
        // Magenta-leaning (g low): tint should go negative (green).
        let magenta = wb_stats_from_pixels(&solid_rgb(140, 120, 140, 1000));
        assert!(auto_wb(&magenta).tint < -0.05);
    }

    #[test]
    fn a_vivid_image_with_no_neutrals_is_left_alone() {
        // A fully saturated red frame: no near-neutral pixels qualify, so auto-WB
        // declines rather than trying to "correct" a deliberate colour.
        let s = wb_stats_from_pixels(&solid_rgb(220, 20, 20, 1000));
        assert!(s.neutral_fraction < MIN_NEUTRAL_FRAC);
        assert_eq!(
            auto_wb(&s),
            AutoWb {
                warmth: 0.0,
                tint: 0.0
            }
        );
    }

    #[test]
    fn a_saturated_object_does_not_hijack_the_estimate() {
        // Mostly neutral grey with a saturated red patch — the red is excluded,
        // so the grey wins and almost no correction is applied.
        let mut px = solid(128, 900);
        px.extend(solid_rgb(220, 20, 20, 100));
        let wb = auto_wb(&wb_stats_from_pixels(&px));
        assert!(wb.warmth.abs() < 0.05 && wb.tint.abs() < 0.05, "{wb:?}");
    }

    #[test]
    fn wb_stays_within_the_conservative_limit_and_is_gentle() {
        // A strong-but-still-neutral-qualifying warm cast: cooled, but modestly,
        // and never past the clamp — the "not too harsh" property.
        let s = wb_stats_from_pixels(&solid_rgb(170, 140, 120, 1000));
        assert!(s.neutral_fraction >= MIN_NEUTRAL_FRAC, "should qualify");
        let wb = auto_wb(&s);
        assert!(wb.warmth < 0.0, "warm cast is cooled");
        assert!(
            wb.warmth >= -WB_LIMIT && wb.warmth <= WB_LIMIT,
            "within the clamp"
        );
        assert!(wb.tint >= -WB_LIMIT && wb.tint <= WB_LIMIT);
    }

    #[test]
    fn wb_apply_and_neutral_measurement_touch_only_warmth_and_tint() {
        let mut stack = EditStack::default();
        stack.warmth = 0.3;
        stack.tint = -0.2;
        stack.exposure = 1.0; // tone — must survive
        let m = AutoWb::neutral_measurement(&stack);
        assert_eq!(m.warmth, 0.0);
        assert_eq!(m.tint, 0.0);
        assert_eq!(m.exposure, 1.0, "tone is not touched");

        let mut s2 = EditStack::default();
        s2.exposure = 0.5;
        AutoWb {
            warmth: -0.2,
            tint: 0.1,
        }
        .apply_to(&mut s2);
        assert_eq!(s2.warmth, -0.2);
        assert_eq!(s2.tint, 0.1);
        assert_eq!(s2.exposure, 0.5, "tone untouched");
    }

    /// A model of the pipeline: a colour cast, the stack's balance applied as
    /// multipliers, then a display curve. The eyedropper has to undo the cast
    /// without being told how the curve works.
    fn render_cast(cast: [f32; 3], warmth: f32, tint: f32) -> [f32; 3] {
        let scale = EditStack {
            warmth,
            tint,
            ..Default::default()
        }
        .wb_scale();
        std::array::from_fn(|c| (0.3 * cast[c] * scale[c]).powf(1.0 / 2.2))
    }

    #[test]
    fn the_eyedropper_makes_its_point_grey() {
        // Tint reaches about 10% either way (see `wb_scale`), so the green
        // casts stay inside it.
        for cast in [
            [1.3, 1.0, 0.75],
            [0.8, 1.0, 1.25],
            [1.0, 1.06, 1.0],
            [1.1, 0.95, 0.9],
        ] {
            let wb = neutralise(|w, t| Ok(render_cast(cast, w, t))).unwrap();
            let [r, g, b] = render_cast(cast, wb.warmth, wb.tint);
            assert!(
                (r - g).abs() < 0.004 && (b - g).abs() < 0.004,
                "cast {cast:?}: {wb:?} left ({r}, {g}, {b})"
            );
        }
        // A warm cast is cooled, a green one gets magenta.
        assert!(
            neutralise(|w, t| Ok(render_cast([1.3, 1.0, 0.75], w, t)))
                .unwrap()
                .warmth
                < 0.0
        );
        assert!(
            neutralise(|w, t| Ok(render_cast([1.0, 1.06, 1.0], w, t)))
                .unwrap()
                .tint
                > 0.0
        );
    }

    #[test]
    fn the_eyedropper_refuses_black() {
        assert!(neutralise(|_, _| Ok([0.0, 0.01, 0.02])).is_err());
    }

    #[test]
    fn a_cast_beyond_the_sliders_gets_as_close_as_they_go() {
        let wb = neutralise(|w, t| Ok(render_cast([2.5, 1.0, 0.4], w, t))).unwrap();
        assert!(wb.warmth <= -0.99, "{wb:?}");
    }
}
