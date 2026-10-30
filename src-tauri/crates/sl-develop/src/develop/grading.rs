//! Colour grading: tinting shadows, midtones and highlights independently.
//!
//! The three-wheel panel every colour grading tool has. Where the colour mixer
//! reaches for a *hue* — "make the blues deeper" — this reaches for a *tonal
//! range*: warm the highlights and cool the shadows, and the picture reads as
//! film regardless of what was in the frame.
//!
//! **Baked into a luminance-indexed table on the CPU**, exactly like the tone
//! curve and the mixer. Every pixel's grade depends only on how bright it is,
//! so the entire calculation — the range weights, the hue-to-RGB conversion,
//! the blend across ranges — collapses to one lookup per pixel. The shader
//! reads a multiplier and multiplies. All the reasoning lives here, where it is
//! testable.

use serde::{Deserialize, Serialize};

/// Entries in the table, indexed by display luminance 0..1.
///
/// Far smaller than the tone table's 1024, and it can be: this is a slow,
/// smooth function of luminance with no fine structure to resolve, and the
/// sampler interpolates between entries.
pub const LUT_SIZE: usize = 256;

/// Rec.2020 luminance weights, matching the working space the grade is applied
/// in. Using sRGB's weights here would make the range split disagree with the
/// pixels it is splitting.
pub const LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];

/// Where each range sits on the luminance axis at neutral balance.
///
/// Not 0/0.5/1: a range centred on absolute black or white would spend most of
/// its weight outside the image. These sit far enough apart to be independent
/// and far enough inside to be reachable.
const CENTRES: [f32; 3] = [0.15, 0.5, 0.85];

/// Width of each range's influence, before `blending` widens or narrows it.
///
/// Tied to the spacing between centres rather than chosen freely — at this
/// width the three weights overlap enough to cross over smoothly and not so
/// much that moving one wheel visibly moves the others.
const BASE_WIDTH: f32 = 0.26;

/// One wheel: a direction on the colour circle, how far to push, and a
/// brightness trim for that range.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct GradeRange {
    /// Degrees, 0..360. Meaningless when `saturation` is zero.
    pub hue: f32,
    /// 0..=1, how strongly the hue is applied.
    pub saturation: f32,
    /// -1..=1 brightness trim for this tonal range.
    pub luminance: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorGrading {
    pub shadows: GradeRange,
    pub midtones: GradeRange,
    pub highlights: GradeRange,
    /// -1..=1. Widens the ranges so they blend into each other, or narrows
    /// them so each stays in its own band.
    pub blending: f32,
    /// -1..=1. Slides the whole split brighter or darker, which is how a photo
    /// with an unusual tonal distribution gets its shadows graded as shadows.
    pub balance: f32,
}

/// Fully saturated RGB for a hue, as a direction rather than a colour — the
/// caller decides how far to travel towards it.
fn hue_to_rgb(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

impl ColorGrading {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    /// How much each range claims a given luminance. Always sums to 1.
    ///
    /// **The normalisation is the important part.** Gaussian falloffs alone
    /// would leave gaps between the ranges where a grade fades out entirely,
    /// and setting all three wheels the same would *not* equal one global
    /// grade — which is the first thing anyone tries. Dividing by the total
    /// makes the weights a partition of unity, so the three wheels stay
    /// independent while still covering the whole range. The colour mixer's
    /// bands are built on the same principle.
    pub fn weights(&self, luma: f32) -> [f32; 3] {
        let width = BASE_WIDTH * (1.0 + self.blending.clamp(-1.0, 1.0) * 0.6);
        let shift = self.balance.clamp(-1.0, 1.0) * 0.25;

        let mut w = [0.0f32; 3];
        let mut total = 0.0;
        for (i, centre) in CENTRES.iter().enumerate() {
            let d = (luma - (centre + shift)) / width.max(1e-3);
            w[i] = (-d * d).exp();
            total += w[i];
        }
        // Far outside every range — only reachable at extreme balance — the
        // weights underflow and the division would produce NaN, blanking the
        // image. Fall back to the nearest range instead.
        if total < 1e-6 {
            let nearest = if luma < CENTRES[1] + shift { 0 } else { 2 };
            let mut fallback = [0.0; 3];
            fallback[nearest] = 1.0;
            return fallback;
        }
        for v in w.iter_mut() {
            *v /= total;
        }
        w
    }

    /// The per-luminance RGB multiplier, as a table the shader samples.
    ///
    /// A multiplier rather than an offset: it scales with the pixel, so a grade
    /// applied to a dark area stays proportionate instead of washing it out
    /// with a flat colour wash.
    pub fn build_lut(&self) -> Vec<[f32; 4]> {
        let ranges = [self.shadows, self.midtones, self.highlights];
        (0..LUT_SIZE)
            .map(|i| {
                let luma = i as f32 / (LUT_SIZE - 1) as f32;
                let w = self.weights(luma);

                // Convex combination, not a product. The weights sum to 1, so
                // averaging the three tints means three identical wheels give
                // exactly that tint — a product would apply it three times.
                let mut rgb = [0.0f32; 3];
                let mut lum = 0.0;
                for (range, weight) in ranges.iter().zip(w) {
                    let sat = range.saturation.clamp(0.0, 1.0);
                    let hue = hue_to_rgb(range.hue);
                    for (c, h) in rgb.iter_mut().zip(hue) {
                        // Towards the hue by `sat`, from no tint at all.
                        *c += weight * (1.0 + sat * (h - 1.0));
                    }
                    lum += weight * range.luminance.clamp(-1.0, 1.0);
                }

                // Tinting by multiplication darkens, because every hue has at
                // least one channel below 1. Renormalising to hold luminance
                // means the wheels change colour and the luminance trim is the
                // only control that changes brightness.
                let y = rgb[0] * LUMA[0] + rgb[1] * LUMA[1] + rgb[2] * LUMA[2];
                let gain = (1.0 + lum * 0.5) / y.max(1e-4);
                [rgb[0] * gain, rgb[1] * gain, rgb[2] * gain, 1.0]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma_of(c: [f32; 4]) -> f32 {
        c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]
    }

    /// An untouched panel must be a byte-level no-op. Every photo goes through
    /// this path, so a grade that tinted slightly at rest would tint the whole
    /// library.
    #[test]
    fn a_neutral_grade_is_the_identity() {
        for entry in ColorGrading::default().build_lut() {
            for c in &entry[..3] {
                assert!(
                    (c - 1.0).abs() < 1e-5,
                    "neutral grade produced a multiplier of {c}"
                );
            }
        }
    }

    /// The weights have to cover the luminance axis exactly once, at every
    /// setting of the two shaping controls — not just the default.
    #[test]
    fn the_range_weights_are_a_partition_of_unity() {
        for blending in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            for balance in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
                let g = ColorGrading {
                    blending,
                    balance,
                    ..Default::default()
                };
                for i in 0..=100 {
                    let luma = i as f32 / 100.0;
                    let sum: f32 = g.weights(luma).iter().sum();
                    assert!(
                        (sum - 1.0).abs() < 1e-4,
                        "weights at luma {luma} summed to {sum} \
                         (blending {blending}, balance {balance})"
                    );
                }
            }
        }
    }

    /// Setting all three wheels alike must equal one global grade. This is the
    /// property the weight normalisation exists for, and the first thing anyone
    /// checks by dragging all three the same way.
    #[test]
    fn three_identical_wheels_equal_one_global_grade() {
        let range = GradeRange {
            hue: 210.0,
            saturation: 0.6,
            luminance: 0.0,
        };
        let g = ColorGrading {
            shadows: range,
            midtones: range,
            highlights: range,
            ..Default::default()
        };
        let lut = g.build_lut();
        let first = lut[0];
        for entry in &lut {
            for (a, b) in entry[..3].iter().zip(&first[..3]) {
                assert!(
                    (a - b).abs() < 1e-4,
                    "a uniform grade varied with luminance: {first:?} vs {entry:?}"
                );
            }
        }
    }

    /// Each wheel has to act on its own end of the range. Without this the
    /// panel has three controls that all do the same thing.
    #[test]
    fn each_wheel_acts_on_its_own_tonal_range() {
        let warm = GradeRange {
            hue: 30.0,
            saturation: 1.0,
            luminance: 0.0,
        };
        let shadows_only = ColorGrading {
            shadows: warm,
            ..Default::default()
        };
        let lut = shadows_only.build_lut();

        // Red/blue ratio is the signature of a warm tint.
        let ratio = |c: [f32; 4]| c[0] / c[2].max(1e-4);
        let dark = ratio(lut[LUT_SIZE / 10]);
        let bright = ratio(lut[LUT_SIZE - 1]);

        assert!(dark > 1.5, "warming the shadows did not warm them: {dark}");
        assert!(
            bright < dark * 0.5,
            "a shadow-only grade reached the highlights: dark {dark}, bright {bright}"
        );
    }

    /// A hue at full saturation must change colour without changing
    /// brightness, so the wheels and the luminance trim stay separate controls.
    #[test]
    fn tinting_holds_luminance_and_only_the_trim_changes_it() {
        for hue in [0.0f32, 60.0, 120.0, 180.0, 240.0, 300.0] {
            let g = ColorGrading {
                shadows: GradeRange {
                    hue,
                    saturation: 1.0,
                    luminance: 0.0,
                },
                midtones: GradeRange {
                    hue,
                    saturation: 1.0,
                    luminance: 0.0,
                },
                highlights: GradeRange {
                    hue,
                    saturation: 1.0,
                    luminance: 0.0,
                },
                ..Default::default()
            };
            let y = luma_of(g.build_lut()[LUT_SIZE / 2]);
            assert!(
                (y - 1.0).abs() < 1e-3,
                "hue {hue} changed luminance to {y} — tint and brightness are not separable"
            );
        }
    }

    /// The luminance trim must move brightness in the direction it is dragged,
    /// and do so across the whole range when all three agree.
    #[test]
    fn the_luminance_trim_brightens_and_darkens() {
        let trim = |v: f32| {
            let r = GradeRange {
                hue: 0.0,
                saturation: 0.0,
                luminance: v,
            };
            let g = ColorGrading {
                shadows: r,
                midtones: r,
                highlights: r,
                ..Default::default()
            };
            luma_of(g.build_lut()[LUT_SIZE / 2])
        };
        assert!(trim(1.0) > 1.2, "positive trim did not brighten");
        assert!(trim(-1.0) < 0.8, "negative trim did not darken");
        assert!((trim(0.0) - 1.0).abs() < 1e-4, "zero trim was not neutral");
    }

    /// Extreme balance pushes every range centre off one end of the axis. The
    /// weights must still resolve rather than dividing by an underflowed total
    /// and blanking the image with NaN.
    #[test]
    fn extreme_settings_never_produce_nan() {
        for blending in [-1.0f32, 1.0] {
            for balance in [-1.0f32, 1.0] {
                let g = ColorGrading {
                    shadows: GradeRange {
                        hue: 350.0,
                        saturation: 1.0,
                        luminance: 1.0,
                    },
                    highlights: GradeRange {
                        hue: 40.0,
                        saturation: 1.0,
                        luminance: -1.0,
                    },
                    blending,
                    balance,
                    ..Default::default()
                };
                for entry in g.build_lut() {
                    for c in entry {
                        assert!(c.is_finite(), "grade produced {c}");
                    }
                }
            }
        }
    }
}
