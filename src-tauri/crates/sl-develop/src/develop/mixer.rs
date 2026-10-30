//! The colour mixer: per-hue-band hue, saturation and luminance.
//!
//! Eight bands, as raw editors have long used, because they map onto
//! the colours people actually name — "the blues are too cyan", "drop the
//! oranges" — rather than onto anything about the sensor.
//!
//! Like the tone curve, the band maths is evaluated **on the CPU into a LUT**,
//! here indexed by hue. The shader converts to HSV, looks up three numbers and
//! applies them. That keeps the interpolation between bands testable, and it
//! means adding a ninth band would not touch the shader at all.

use serde::{Deserialize, Serialize};

pub const BANDS: usize = 8;

/// Band centres in degrees. Unevenly spaced on purpose — they follow how hue
/// names divide the wheel, not an even split. Red through yellow is crowded
/// because skin, wood and foliage all live there and want independent control.
///
/// These are HSV hues on the working space, which is where an edit saved before
/// version 4 reads its mixer, and where the camera's look profile
/// (`develop::look`) was measured and still applies. A newer edit's mixer reads
/// OkLCh hue instead, around [`perceptual_centres`].
pub const BAND_HUES: [f32; BANDS] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0];

/// The colours the bands are named for, as sRGB — the swatches the panel draws
/// them with: fully saturated at the HSL hues of [`BAND_HUES`].
const NAMED: [[f32; 3]; BANDS] = [
    [1.0, 0.0, 0.0],
    [1.0, 0.5, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 1.0, 1.0],
    [0.0, 0.0, 1.0],
    [0.5, 0.0, 1.0],
    [1.0, 0.0, 1.0],
];

/// Band centres as OkLCh hues: where each band's named colour actually sits
/// (`develop::oklab`). Red is at 29 degrees, yellow at 110, blue at 264 — so a
/// band is the colour its name says, wherever that colour falls.
pub fn perceptual_centres() -> [f32; BANDS] {
    NAMED.map(crate::develop::oklab::srgb_hue)
}

pub const BAND_NAMES: [&str; BANDS] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];

/// Entries in the hue lookup table. 256 over 360 degrees is finer than any
/// visible banding, and the shader interpolates between entries anyway.
pub const LUT_SIZE: usize = 256;

/// Degrees of hue rotation at full deflection. Beyond about this the result
/// stops reading as the same colour adjusted and starts looking like a
/// substitution.
const MAX_HUE_SHIFT: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorMixer {
    pub hue: [f32; BANDS],
    pub saturation: [f32; BANDS],
    pub luminance: [f32; BANDS],
}

impl Default for ColorMixer {
    fn default() -> Self {
        Self {
            hue: [0.0; BANDS],
            saturation: [0.0; BANDS],
            luminance: [0.0; BANDS],
        }
    }
}

/// Which two bands a hue falls between, and how far.
///
/// Returns `(low, high, t)` with weights `1 - t` and `t`. A partition of unity,
/// so a uniform adjustment across all bands is exactly a global adjustment —
/// without that, moving every slider equally would produce hue-dependent
/// ripples.
fn span(hue_deg: f32, centres: &[f32; BANDS]) -> (usize, usize, f32) {
    let h = hue_deg.rem_euclid(360.0);

    for i in 0..BANDS {
        let lo = centres[i];
        // The last band wraps around to the first, 360 degrees on.
        let hi = if i + 1 < BANDS {
            centres[i + 1]
        } else {
            centres[0] + 360.0
        };
        if h >= lo && h < hi {
            return (i, (i + 1) % BANDS, (h - lo) / (hi - lo));
        }
    }
    // Below the first centre: between the wrapping last band and the first.
    let lo = centres[BANDS - 1] - 360.0;
    let hi = centres[0];
    (BANDS - 1, 0, (h - lo) / (hi - lo))
}

/// Interpolates a per-band vector at a hue, as a partition of unity.
///
/// The blend the mixer and the look profile share: adjacent bands reach zero at
/// each other's centres, so a uniform vector is exactly a global adjustment and
/// nothing steps at a band edge. Exposed so [`crate::develop::look`] bakes into
/// the same LUT with the same interpolation rather than a second, subtly
/// different one.
pub(crate) fn blend_band(v: &[f32; BANDS], hue_deg: f32) -> f32 {
    blend_band_around(v, hue_deg, &BAND_HUES)
}

/// [`blend_band`] around given band centres.
fn blend_band_around(v: &[f32; BANDS], hue_deg: f32, centres: &[f32; BANDS]) -> f32 {
    let (lo, hi, t) = span(hue_deg, centres);
    v[lo] * (1.0 - t) + v[hi] * t
}

impl ColorMixer {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    /// Adjustments at a hue: `(hue shift in degrees, saturation, luminance)`.
    pub fn sample(&self, hue_deg: f32) -> [f32; 3] {
        self.sample_around(hue_deg, &BAND_HUES)
    }

    fn sample_around(&self, hue_deg: f32, centres: &[f32; BANDS]) -> [f32; 3] {
        [
            blend_band_around(&self.hue, hue_deg, centres) * MAX_HUE_SHIFT,
            blend_band_around(&self.saturation, hue_deg, centres),
            blend_band_around(&self.luminance, hue_deg, centres),
        ]
    }

    /// Bakes the bands into a hue-indexed table for the shader.
    pub fn build_lut(&self) -> Vec<[f32; 4]> {
        Self::lut_of(|hue| self.sample(hue))
    }

    /// The table a version-4 edit's mixer reads, indexed by OkLCh hue around
    /// [`perceptual_centres`]: hue shift in degrees, then chroma and lightness
    /// factors less one. All zeros for a neutral mixer, which the shader takes
    /// as nothing to do.
    pub fn build_perceptual_lut(&self) -> Vec<[f32; 4]> {
        let centres = perceptual_centres();
        Self::lut_of(|hue| self.sample_around(hue, &centres))
    }

    fn lut_of(sample: impl Fn(f32) -> [f32; 3]) -> Vec<[f32; 4]> {
        (0..LUT_SIZE)
            .map(|i| {
                let [h, s, l] = sample(i as f32 / LUT_SIZE as f32 * 360.0);
                [h, s, l, 0.0]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_red(hue: f32, saturation: f32, luminance: f32) -> ColorMixer {
        let mut m = ColorMixer::default();
        m.hue[0] = hue;
        m.saturation[0] = saturation;
        m.luminance[0] = luminance;
        m
    }

    #[test]
    fn a_default_mixer_changes_nothing_at_any_hue() {
        let m = ColorMixer::default();
        for i in 0..360 {
            assert_eq!(m.sample(i as f32), [0.0, 0.0, 0.0], "hue {i}");
        }
        assert!(m.is_neutral());
    }

    #[test]
    fn a_band_takes_full_effect_at_its_own_centre() {
        let m = with_red(0.0, 1.0, 0.0);
        assert!((m.sample(BAND_HUES[0])[1] - 1.0).abs() < 1e-5);
    }

    /// Adjacent bands must reach zero at each other's centres, or every slider
    /// leaks into its neighbours and nothing is independently controllable.
    #[test]
    fn a_band_has_no_effect_at_its_neighbours_centres() {
        let m = with_red(0.0, 1.0, 0.0);
        assert!(m.sample(BAND_HUES[1])[1].abs() < 1e-5, "leaked into orange");
        assert!(
            m.sample(BAND_HUES[BANDS - 1])[1].abs() < 1e-5,
            "leaked into magenta"
        );
    }

    /// The weights sum to one everywhere, so moving every band by the same
    /// amount is exactly a global adjustment — no hue-dependent ripple.
    #[test]
    fn uniform_bands_behave_as_a_global_adjustment() {
        let m = ColorMixer {
            saturation: [0.4; BANDS],
            ..Default::default()
        };
        for i in 0..360 {
            let s = m.sample(i as f32)[1];
            assert!((s - 0.4).abs() < 1e-5, "hue {i} gave {s}, expected 0.4");
        }
    }

    /// Hue is circular: 359 degrees and 1 degree are neighbours, and the
    /// interpolation has to cross that seam without a discontinuity.
    #[test]
    fn adjustments_are_continuous_across_the_wrap() {
        let m = with_red(0.0, 1.0, 0.0);
        let just_below = m.sample(359.9)[1];
        let at_zero = m.sample(0.0)[1];
        assert!(
            (just_below - at_zero).abs() < 0.02,
            "discontinuity at the wrap: {just_below} vs {at_zero}"
        );
    }

    #[test]
    fn out_of_range_hues_wrap_rather_than_clamp() {
        let m = with_red(0.0, 1.0, 0.0);
        assert!((m.sample(360.0)[1] - m.sample(0.0)[1]).abs() < 1e-5);
        assert!((m.sample(-30.0)[1] - m.sample(330.0)[1]).abs() < 1e-5);
    }

    #[test]
    fn hue_shift_is_expressed_in_degrees() {
        let m = with_red(1.0, 0.0, 0.0);
        assert!((m.sample(0.0)[0] - MAX_HUE_SHIFT).abs() < 1e-5);
        let back = with_red(-1.0, 0.0, 0.0);
        assert!((back.sample(0.0)[0] + MAX_HUE_SHIFT).abs() < 1e-5);
    }

    #[test]
    fn the_lut_covers_the_hue_circle() {
        let m = with_red(0.0, 1.0, 0.0);
        let lut = m.build_lut();
        assert_eq!(lut.len(), LUT_SIZE);
        // Entry 0 is hue 0, the red centre, so it carries the full adjustment.
        assert!((lut[0][1] - 1.0).abs() < 1e-5);
        // A quarter of the way round is hue 90, between yellow and green,
        // where red has no say.
        assert!(lut[LUT_SIZE / 4][1].abs() < 1e-5);
    }

    /// Every band must be reachable, or a slider silently does nothing.
    #[test]
    fn every_band_influences_some_hue() {
        for (band, name) in BAND_NAMES.iter().enumerate() {
            let mut m = ColorMixer::default();
            m.saturation[band] = 1.0;
            let peak = m.build_lut().iter().map(|e| e[1]).fold(f32::MIN, f32::max);
            assert!(peak > 0.9, "band {band} ({name}) peaks at only {peak}");
        }
    }

    /// The perceptual centres go round the wheel in the order the names do,
    /// each at its own colour: sRGB red is the red band at full strength.
    #[test]
    fn perceptual_centres_are_the_named_colours() {
        let c = perceptual_centres();
        assert!(c.windows(2).all(|p| p[1] > p[0]), "{c:?}");
        assert!(
            (c[0] - 29.2).abs() < 0.5 && (c[5] - 264.1).abs() < 0.5,
            "{c:?}"
        );
        let m = with_red(0.0, 1.0, 0.0);
        let red = crate::develop::oklab::srgb_hue([1.0, 0.0, 0.0]);
        assert!((m.sample_around(red, &c)[1] - 1.0).abs() < 1e-5);
        assert!(
            m.sample_around(c[1], &c)[1].abs() < 1e-5,
            "leaked into orange"
        );
        // And the table is all zeros for a neutral mixer, the shader's cue.
        assert!(ColorMixer::default()
            .build_perceptual_lut()
            .iter()
            .all(|e| *e == [0.0; 4]));
    }
}
