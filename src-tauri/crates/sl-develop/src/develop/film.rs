//! Film-response primitives: the building blocks a convincing look is made of.
//!
//! The built-in looks used to be composed from four toys — a red/blue trade, a
//! smoothstep, a saturation scale and a linear lift — which is why they read as
//! *tints* rather than as film. What actually separates the two is well
//! documented and none of it was here:
//!
//! - a **characteristic curve** with a real toe and shoulder, so shadows and
//!   highlights lose contrast gradually instead of running into the ends of the
//!   range and clipping;
//! - **per-channel** versions of that curve, which is where a stock's colour
//!   signature lives — Portra's warmth is not a tint applied afterwards, it is
//!   red, green and blue responding to light differently;
//! - **hue-dependent** colour bias, so a look can cool greens without touching
//!   skin, rather than moving every hue by the same amount.
//!
//! Everything here is an original implementation of those published
//! *characteristics*. No LUT data, curve table or name is copied from any
//! commercial product — everything in the app must be ours to release under
//! its own licence, and the built-in pack has always been our own work.
//!
//! # Contract
//!
//! Every function takes and returns values in `[0, 1]` in the display-encoded
//! space the LUT stage runs in (see [`crate::develop::lut`]), is **monotonic**
//! in its input where it is a tone curve, and clamps — so any composition of
//! these stays in range and no look can emit an out-of-range or non-finite
//! texel. The tests hold all three.

pub fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

/// Rec.709 luma — the grey a colour desaturates toward.
pub fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Scales the distance from grey. `s > 1` saturates, `s < 1` mutes, `0` is mono.
pub fn saturation(c: [f32; 3], s: f32) -> [f32; 3] {
    let l = luma(c);
    [
        clamp01(l + (c[0] - l) * s),
        clamp01(l + (c[1] - l) * s),
        clamp01(l + (c[2] - l) * s),
    ]
}

/// The **toe** of a characteristic curve: slope falls away toward black, so
/// deep shadows compress into a soft foot instead of hitting 0 and stopping.
///
/// `k` is how much toe there is; 0 is a straight line. Fixes both endpoints
/// (0→0, 1→1) and is strictly increasing, which is what makes it safe to
/// compose with everything else here.
pub fn soft_toe(x: f32, k: f32) -> f32 {
    let x = clamp01(x);
    let k = k.max(0.0);
    if k < 1e-6 {
        return x;
    }
    // x²/(x+k) has the falling slope we want near 0; the (1+k) factor restores
    // f(1) = 1 so the white point does not move.
    clamp01(x * x / (x + k) * (1.0 + k))
}

/// The **shoulder**: slope falls away toward white, so a bright window or a sky
/// rolls off rather than clipping flat. The toe, mirrored.
pub fn soft_shoulder(x: f32, k: f32) -> f32 {
    1.0 - soft_toe(1.0 - clamp01(x), k)
}

/// Mid-grey, in the display-encoded space these primitives work in. The pivot
/// [`HdCurve`]'s gamma turns about.
const PIVOT: f32 = 0.5;

/// Changes the slope about `PIVOT` while holding black, mid-grey and white
/// where they are.
///
/// A plain `x^gamma` is the wrong primitive for this: raising the exponent
/// makes the whole image *darker*, not more contrasty, so a "print" built on it
/// pulls the highlights down instead of pushing them up. Gamma on a
/// characteristic curve means the slope of its straight-line portion, and this
/// is that — steeper above 1, flatter below, with the mid-tone staying put.
fn gamma_about(x: f32, gamma: f32) -> f32 {
    let x = clamp01(x);
    let g = gamma.max(0.05);
    if x <= PIVOT {
        PIVOT * (x / PIVOT).powf(g)
    } else {
        1.0 - (1.0 - PIVOT) * ((1.0 - x) / (1.0 - PIVOT)).powf(g)
    }
}

/// One channel's characteristic curve.
///
/// `gamma` is the slope through the mid-tones — the straight-line portion of a
/// real H&D plot — while `toe` and `shoulder` set how gently each end departs
/// from it. Three numbers per channel is enough to describe the difference
/// between a portrait negative and a slide film.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdCurve {
    pub toe: f32,
    pub shoulder: f32,
    pub gamma: f32,
}

impl HdCurve {
    pub const fn new(toe: f32, shoulder: f32, gamma: f32) -> Self {
        Self {
            toe,
            shoulder,
            gamma,
        }
    }

    /// A straight line — the identity response.
    pub const fn linear() -> Self {
        Self::new(0.0, 0.0, 1.0)
    }

    pub fn apply(self, x: f32) -> f32 {
        // Slope first (it sets the mid-tone contrast), then the two ends soften
        // what that produced. Composing monotone functions keeps the whole
        // curve monotone, which is the property that stops a look inverting
        // tones somewhere in the middle of its range.
        let x = gamma_about(x, self.gamma);
        soft_shoulder(soft_toe(x, self.toe), self.shoulder)
    }
}

/// Applies a separate curve per channel.
///
/// This is the primitive the old pack was missing entirely. A stock's colour is
/// not a tint laid over a neutral tone response — it *is* the three channels
/// responding differently, which is why film's colour shifts with exposure and
/// a flat tint's does not.
pub fn channel_response(c: [f32; 3], curves: [HdCurve; 3]) -> [f32; 3] {
    [
        curves[0].apply(c[0]),
        curves[1].apply(c[1]),
        curves[2].apply(c[2]),
    ]
}

/// Hue in degrees (0 = red, 120 = green, 240 = blue) and saturation, HSV-style.
/// Hue is 0 for a neutral colour, where it is meaningless.
fn hue_sat(c: [f32; 3]) -> (f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let chroma = max - min;
    if chroma < 1e-6 || max < 1e-6 {
        return (0.0, 0.0);
    }
    let h = if max == c[0] {
        60.0 * (((c[1] - c[2]) / chroma) % 6.0)
    } else if max == c[1] {
        60.0 * ((c[2] - c[0]) / chroma + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / chroma + 4.0)
    };
    ((h + 360.0) % 360.0, chroma / max)
}

/// Shortest angular distance between two hues, in degrees (0…180).
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

/// A hue-dependent adjustment: within `width_deg` of `center_deg`, scale
/// saturation and rotate hue, falling off smoothly to nothing at the edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueBand {
    pub center_deg: f32,
    pub width_deg: f32,
    /// Multiplier on this band's saturation. 1.0 leaves it alone.
    pub saturation: f32,
    /// Degrees to rotate hues in this band. Positive goes red → yellow → green.
    pub shift_deg: f32,
}

/// Applies hue-dependent colour bias — the thing that lets a look cool its
/// greens without draining the skin tones in the same frame.
///
/// Bands are applied in order and their effects accumulate, so overlapping
/// bands compose rather than the last one winning.
pub fn hue_bias(c: [f32; 3], bands: &[HueBand]) -> [f32; 3] {
    let (hue, sat) = hue_sat(c);
    if sat < 1e-4 {
        // A neutral has no hue to bias, and inventing one would tint greys.
        return [clamp01(c[0]), clamp01(c[1]), clamp01(c[2])];
    }
    let mut out = c;
    for band in bands {
        let d = hue_distance(hue, band.center_deg);
        let half = band.width_deg.max(1.0) / 2.0;
        if d >= half {
            continue;
        }
        // Raised cosine: full effect at the centre, zero and flat at the edge,
        // so a band boundary never shows as a hard edge in a gradient.
        let t = 0.5 * (1.0 + (std::f32::consts::PI * d / half).cos());
        if (band.saturation - 1.0).abs() > 1e-6 {
            out = mix3(out, saturation(out, band.saturation), t);
        }
        if band.shift_deg.abs() > 1e-6 {
            out = mix3(out, rotate_hue(out, band.shift_deg), t);
        }
    }
    [clamp01(out[0]), clamp01(out[1]), clamp01(out[2])]
}

/// Rotates a colour's hue by `deg`, preserving its luma so the rotation does
/// not also change how bright the colour reads.
pub fn rotate_hue(c: [f32; 3], deg: f32) -> [f32; 3] {
    let before = luma(c);
    let (h, _) = hue_sat(c);
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let chroma = max - min;
    if chroma < 1e-6 {
        return c;
    }
    let h2 = (h + deg).rem_euclid(360.0);
    let sector = h2 / 60.0;
    let x = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    // Re-seat the rotated colour at the original luma rather than the original
    // minimum, so a hue shift is not also a brightness shift.
    let rotated = [r, g, b];
    let m = before - luma(rotated);
    [
        clamp01(rotated[0] + m),
        clamp01(rotated[1] + m),
        clamp01(rotated[2] + m),
    ]
}

/// Remaps `[0, 1]` onto `[lift, gain]` — a lifted black point and a pulled-in
/// white point, the basis of a matte or faded look.
pub fn lift_gain(x: f32, lift: f32, gain: f32) -> f32 {
    clamp01(lift + clamp01(x) * (gain - lift))
}

/// Warms (`amt > 0`) or cools (`amt < 0`) by trading red against blue.
pub fn warm(c: [f32; 3], amt: f32) -> [f32; 3] {
    [
        clamp01(c[0] * (1.0 + 0.15 * amt)),
        c[1],
        clamp01(c[2] * (1.0 - 0.15 * amt)),
    ]
}

/// Split-toning: pushes shadows toward `shadow` and highlights toward `high`,
/// weighted by luma, with `amt` overall strength. Tint colours are given around
/// a neutral `0.5`, so `(tint - 0.5)` is the push direction on each channel.
pub fn split_tone(c: [f32; 3], shadow: [f32; 3], high: [f32; 3], amt: f32) -> [f32; 3] {
    let l = luma(c);
    let sw = (1.0 - l) * amt;
    let hw = l * amt;
    [
        clamp01(c[0] + (shadow[0] - 0.5) * sw + (high[0] - 0.5) * hw),
        clamp01(c[1] + (shadow[1] - 0.5) * sw + (high[1] - 0.5) * hw),
        clamp01(c[2] + (shadow[2] - 0.5) * sw + (high[2] - 0.5) * hw),
    ]
}

/// The **print stage**: a second characteristic curve applied on top of the
/// first, as a negative gains its contrast from being printed onto paper.
///
/// This is what gives cinema looks their depth. Doing it as a second curve
/// rather than simply steepening the first is not a formality — compounding two
/// gentle S curves keeps both ends soft while the middle gets steep, which a
/// single hard curve cannot do without clipping.
pub fn print_stage(c: [f32; 3], strength: f32) -> [f32; 3] {
    let s = strength.clamp(0.0, 1.0);
    if s < 1e-6 {
        return c;
    }
    let curve = HdCurve::new(0.06 * s, 0.10 * s, 1.0 + 0.35 * s);
    channel_response(c, [curve; 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample grid every property below is checked on.
    fn steps() -> Vec<f32> {
        (0..=64).map(|i| i as f32 / 64.0).collect()
    }

    #[test]
    fn the_toe_fixes_its_endpoints_and_only_ever_rises() {
        for &k in &[0.0, 0.05, 0.3, 1.0] {
            assert!((soft_toe(0.0, k) - 0.0).abs() < 1e-5, "k={k}");
            assert!((soft_toe(1.0, k) - 1.0).abs() < 1e-5, "k={k}");
            let mut prev = -1.0;
            for x in steps() {
                let y = soft_toe(x, k);
                assert!(y >= prev - 1e-6, "toe must not fall: k={k} x={x}");
                assert!((0.0..=1.0).contains(&y) && y.is_finite());
                prev = y;
            }
        }
    }

    #[test]
    fn the_toe_darkens_and_the_shoulder_lightens_the_middle() {
        // The point of each: a toe pulls the low mid down (less shadow
        // contrast), a shoulder pushes the high mid up.
        assert!(soft_toe(0.25, 0.3) < 0.25);
        assert!(soft_shoulder(0.75, 0.3) > 0.75);
    }

    #[test]
    fn the_shoulder_fixes_its_endpoints_and_only_ever_rises() {
        for &k in &[0.0, 0.05, 0.3, 1.0] {
            assert!((soft_shoulder(0.0, k) - 0.0).abs() < 1e-5);
            assert!((soft_shoulder(1.0, k) - 1.0).abs() < 1e-5);
            let mut prev = -1.0;
            for x in steps() {
                let y = soft_shoulder(x, k);
                assert!(y >= prev - 1e-6, "shoulder must not fall: k={k} x={x}");
                prev = y;
            }
        }
    }

    #[test]
    fn a_characteristic_curve_is_monotonic_in_range_and_finite() {
        let curves = [
            HdCurve::linear(),
            HdCurve::new(0.10, 0.14, 0.92),
            HdCurve::new(0.02, 0.30, 1.25),
            HdCurve::new(0.40, 0.40, 0.60),
        ];
        for c in curves {
            let mut prev = -1.0;
            for x in steps() {
                let y = c.apply(x);
                assert!(y.is_finite(), "{c:?} at {x}");
                assert!((0.0..=1.0).contains(&y), "{c:?} at {x} gave {y}");
                assert!(y >= prev - 1e-6, "{c:?} must not fall at {x}");
                prev = y;
            }
        }
    }

    #[test]
    fn gamma_raises_contrast_without_darkening_the_image() {
        // The bug this primitive exists to avoid: `x^gamma` steepens *and*
        // darkens, so a print stage built on it pulled the highlights down.
        assert!(
            (gamma_about(0.5, 2.0) - 0.5).abs() < 1e-5,
            "mid-grey must hold"
        );
        assert!(gamma_about(0.25, 2.0) < 0.25, "shadows deepen");
        assert!(gamma_about(0.75, 2.0) > 0.75, "highlights lift");
        // And the reverse below 1.
        assert!(gamma_about(0.25, 0.5) > 0.25);
        assert!(gamma_about(0.75, 0.5) < 0.75);
        // Endpoints never move.
        assert!((gamma_about(0.0, 2.0)).abs() < 1e-6);
        assert!((gamma_about(1.0, 2.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_linear_curve_is_the_identity() {
        for x in steps() {
            assert!((HdCurve::linear().apply(x) - x).abs() < 1e-5);
        }
    }

    #[test]
    fn hue_is_read_correctly_at_the_primaries() {
        assert!((hue_sat([1.0, 0.0, 0.0]).0 - 0.0).abs() < 1e-3);
        assert!((hue_sat([0.0, 1.0, 0.0]).0 - 120.0).abs() < 1e-3);
        assert!((hue_sat([0.0, 0.0, 1.0]).0 - 240.0).abs() < 1e-3);
        // A neutral has no hue, and reports no saturation rather than a
        // meaningless angle.
        assert_eq!(hue_sat([0.5, 0.5, 0.5]), (0.0, 0.0));
    }

    #[test]
    fn hue_distance_wraps_the_short_way() {
        assert!((hue_distance(350.0, 10.0) - 20.0).abs() < 1e-3);
        assert!((hue_distance(10.0, 350.0) - 20.0).abs() < 1e-3);
        assert!((hue_distance(0.0, 180.0) - 180.0).abs() < 1e-3);
    }

    #[test]
    fn a_hue_band_moves_its_own_hue_and_leaves_the_others_alone() {
        // Cool the greens hard; red must not move.
        let bands = [HueBand {
            center_deg: 120.0,
            width_deg: 80.0,
            saturation: 0.2,
            shift_deg: 0.0,
        }];
        let green = [0.2, 0.8, 0.2];
        let red = [0.8, 0.2, 0.2];
        assert!(hue_bias(green, &bands) != green, "the band must act");
        let red_out = hue_bias(red, &bands);
        for i in 0..3 {
            assert!(
                (red_out[i] - red[i]).abs() < 1e-5,
                "red is outside the band and must be untouched"
            );
        }
    }

    #[test]
    fn hue_bias_never_tints_a_neutral() {
        // The failure this guards: a look that quietly puts a colour cast in
        // every grey in the frame.
        for g in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let bands = [HueBand {
                center_deg: 30.0,
                width_deg: 360.0,
                saturation: 2.0,
                shift_deg: 40.0,
            }];
            let out = hue_bias([g, g, g], &bands);
            assert!((out[0] - out[1]).abs() < 1e-5 && (out[1] - out[2]).abs() < 1e-5);
        }
    }

    #[test]
    fn rotating_a_hue_by_a_full_turn_returns_it() {
        let c = [0.8, 0.35, 0.2];
        let out = rotate_hue(c, 360.0);
        for i in 0..3 {
            assert!((out[i] - c[i]).abs() < 2e-2, "{out:?} vs {c:?}");
        }
    }

    #[test]
    fn rotating_a_hue_keeps_its_brightness() {
        let c = [0.8, 0.35, 0.2];
        for deg in [30.0, 90.0, 180.0, 300.0] {
            let out = rotate_hue(c, deg);
            assert!(
                (luma(out) - luma(c)).abs() < 2e-2,
                "{deg}° changed brightness: {out:?}"
            );
        }
    }

    #[test]
    fn the_print_stage_adds_contrast_without_leaving_the_range() {
        let dark = print_stage([0.25, 0.25, 0.25], 1.0);
        let light = print_stage([0.75, 0.75, 0.75], 1.0);
        assert!(dark[0] < 0.25, "shadows go down");
        assert!(light[0] > 0.75, "highlights go up");
        for x in steps() {
            let out = print_stage([x, x, x], 1.0);
            assert!(out.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn every_primitive_stays_in_range_over_the_whole_cube() {
        // The invariant the LUT bake depends on: no composition of these can
        // emit an out-of-range or non-finite texel.
        let bands = [
            HueBand {
                center_deg: 30.0,
                width_deg: 90.0,
                saturation: 1.4,
                shift_deg: -8.0,
            },
            HueBand {
                center_deg: 200.0,
                width_deg: 120.0,
                saturation: 0.7,
                shift_deg: 12.0,
            },
        ];
        let curves = [
            HdCurve::new(0.08, 0.16, 0.95),
            HdCurve::new(0.06, 0.14, 1.00),
            HdCurve::new(0.04, 0.20, 1.06),
        ];
        for r in 0..9 {
            for g in 0..9 {
                for b in 0..9 {
                    let c = [r as f32 / 8.0, g as f32 / 8.0, b as f32 / 8.0];
                    let c = channel_response(c, curves);
                    let c = hue_bias(c, &bands);
                    let c = print_stage(c, 0.6);
                    let c = split_tone(c, [0.46, 0.49, 0.55], [0.55, 0.50, 0.45], 0.2);
                    let c = saturation(warm(c, 0.4), 1.1);
                    assert!(
                        c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                        "escaped the cube: {c:?}"
                    );
                }
            }
        }
    }
}
