//! Per-camera look profiles — the "Camera Standard" equivalent.
//!
//! A raw rendered neutrally is correct but flat: toning each channel in a space
//! as wide as Rec.2020 costs saturation, and camera makers deliberately punch
//! up certain hues (foliage most of all) in their own JPEGs. Measured against
//! the a6400's JPEG across 40 frames, our render is **18% less saturated
//! overall**, and the gap is strongly hue-dependent — greens sit 35% flat while
//! aqua is already slightly over. `tests/look_profile.rs` is that measurement.
//!
//! A profile closes most of that gap. Three deliberate limits:
//!
//! - **It is the camera's, not the user's.** Resolved from the raw's model at
//!   `prepare` and composed into the mixer LUT in the pipeline — never in
//!   `EditStack`. So it does not enter history, travel with a preset, or undo,
//!   and a photo's *edits* stay separable from its *baseline*. This mirrors the
//!   colour matrix and orientation, which are also intrinsic to the file.
//! - **Saturation only.** The per-band value column in the measurement is
//!   dominated by scene content, not colour science, so a luminance twist would
//!   be fitting noise; and doing it in HSV saturation leaves `value` untouched,
//!   which is why the exposure anchor and `middle_grey` are unaffected. Hue
//!   twist is left at identity until there is a colour target to measure it
//!   from rather than a hue histogram.
//! - **Damped, not matched.** Reproducing the JPEG is explicitly not the goal —
//!   a developer that did would have no reason to exist. The profile applies a
//!   fraction of the measured gap (see `DAMPING`), enough to give the render
//!   life without becoming a camera clone.
//!
//! A camera without a measured profile gets [`GENERIC`]: a mild, uniform lift
//! rather than nothing. Measured against five unprofiled bodies' own
//! JPEGs, every one but Olympus rendered flatter than the camera did, and a
//! Canon or Panasonic owner's first comparison is exactly that one. Uniform
//! across hues on purpose — five frames is enough to say "flat overall", and
//! nowhere near enough to say *which* hues, which is what a measured profile
//! for that body is for. Set it to [`LookProfile::IDENTITY`] to go back to
//! neutral for unknown cameras.

use crate::develop::mixer::{blend_band, ColorMixer, BANDS, LUT_SIZE};

/// How much of the measured gap a profile closes. Below 1.0 on purpose: 1.0
/// reproduces the camera JPEG, which is not what a raw developer is for.
const DAMPING: f32 = 0.65;

/// A camera's baseline colour rendering, as per-band saturation multipliers.
///
/// Stored as *multiplier minus one*, the same convention as `ColorMixer`'s
/// saturation, so the two compose by multiplying `(1 + a)(1 + b)` and a profile
/// is literally "a mixer the camera ships with underneath the user's".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookProfile {
    pub saturation: [f32; BANDS],
}

impl LookProfile {
    /// No adjustment: the render stands as the neutral pipeline produced it.
    pub const IDENTITY: LookProfile = LookProfile {
        saturation: [0.0; BANDS],
    };

    /// Builds a profile from raw per-band saturation *ratios* (camera ÷ ours),
    /// applying [`DAMPING`]. Keeping the constructor in these units means the
    /// committed numbers read directly off the measurement table.
    const fn from_ratios(ratios: [f32; BANDS]) -> LookProfile {
        // `const` so the tables below are compile-time constants; a plain
        // `.map` closure is not const, hence the manual loop.
        let mut saturation = [0.0f32; BANDS];
        let mut i = 0;
        while i < BANDS {
            saturation[i] = (ratios[i] - 1.0) * DAMPING;
            i += 1;
        }
        LookProfile { saturation }
    }

    /// The profile for a camera, by make and model. Identity when unrecognised.
    ///
    /// Matched loosely on the model substring rather than an exact string,
    /// because bodies report themselves inconsistently — "ILCE-6400" here, but
    /// the same sensor appears under other names — and a near-miss should still
    /// land the right profile rather than silently fall back to neutral.
    pub fn for_camera(make: &str, model: &str) -> LookProfile {
        Self::measured_for(make, model).unwrap_or(GENERIC)
    }

    /// A profile measured for this specific body, if there is one — as opposed
    /// to the [`GENERIC`] fallback. The distinction matters to anything judging
    /// how close a render gets to the camera (`colour_match`, and the
    /// `look_profile` harness), since only a measured profile is fitted to it.
    pub fn measured_for(make: &str, model: &str) -> Option<LookProfile> {
        let m = model.to_ascii_uppercase();
        let _ = make;
        if m.contains("ILCE-6400") || m.contains("A6400") {
            Some(A6400)
        } else {
            None
        }
    }

    pub fn is_identity(&self) -> bool {
        self.saturation.iter().all(|&s| s == 0.0)
    }

    /// Bakes this profile *under* a user mixer into one hue-indexed LUT.
    ///
    /// The shader applies a single mixer texture, so the two have to be one
    /// table. Saturation is a multiplier, so the combined entry satisfies
    /// `(1 + s) = (1 + profile)(1 + user)` — composing in multiplier space
    /// rather than adding sliders, which would be off by the cross term
    /// (`(1+a)(1+b) != 1 + a + b`). Hue and luminance come straight from the
    /// user mixer, since the profile touches neither.
    pub fn build_lut_under(&self, mixer: &ColorMixer) -> Vec<[f32; 4]> {
        (0..LUT_SIZE)
            .map(|i| {
                let hue = i as f32 / LUT_SIZE as f32 * 360.0;
                let [h, user_sat, l] = mixer.sample(hue);
                let prof_sat = blend_band(&self.saturation, hue);
                let sat = (1.0 + prof_sat) * (1.0 + user_sat) - 1.0;
                [h, sat, l, 0.0]
            })
            .collect()
    }
}

/// Sony a6400. Measured against 40 of the user's own frames — see
/// `tests/look_profile.rs`, which prints exactly this ratio column. Greens are
/// the largest correction (foliage), aqua the one that pulls the other way.
///
/// Ratios: R 1.058, O 1.161, Y 1.211, G 1.355, Aqua 0.932, B 1.090,
/// P 1.244, M 1.074. Re-run the measurement if the tone stage or demosaic
/// changes materially — a saturation shift upstream moves these.
const A6400: LookProfile =
    LookProfile::from_ratios([1.058, 1.161, 1.211, 1.355, 0.932, 1.090, 1.244, 1.074]);

/// Every body without a measured profile. `tests/look_profile.rs` pooled over
/// the five unprofiled fixture cameras (Canon R6, Fuji X-T3, Nikon Z6, Olympus
/// E-M10 III, Panasonic G9 — one scene each) measures the camera JPEGs at
/// **1.141x** our saturation across all hues; per body it ran from 0.92
/// (Olympus) to 1.34 (Panasonic). Damped like any profile, that is a +9% lift.
const GENERIC_RATIO: f32 = 1.141;
const GENERIC: LookProfile = LookProfile::from_ratios([GENERIC_RATIO; BANDS]);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_under_a_neutral_mixer_is_the_mixers_own_lut() {
        let a = LookProfile::IDENTITY.build_lut_under(&ColorMixer::default());
        let b = ColorMixer::default().build_lut();
        assert_eq!(a, b);
    }

    /// An unknown body gets the generic lift: the same small amount on every
    /// hue, less than any band a measured profile would move, and never down.
    #[test]
    fn an_unknown_camera_gets_a_mild_uniform_lift() {
        let p = LookProfile::for_camera("Nikon", "Z6");
        assert!(LookProfile::measured_for("Nikon", "Z6").is_none());
        assert_eq!(p, GENERIC);
        let first = p.saturation[0];
        assert!(
            p.saturation.iter().all(|&s| s == first),
            "uniform across hues"
        );
        assert!(first > 0.0 && first < 0.15, "mild: {first}");
    }

    #[test]
    fn the_a6400_is_recognised_by_either_name() {
        assert_eq!(LookProfile::measured_for("Sony", "ILCE-6400"), Some(A6400));
        assert_eq!(LookProfile::measured_for("SONY", "a6400"), Some(A6400));
        assert_eq!(LookProfile::for_camera("SONY", "a6400"), A6400);
    }

    /// Damping is the whole point: the profile must land strictly between doing
    /// nothing and reproducing the camera. Greens, the largest gap, are the
    /// clearest place to check it.
    #[test]
    fn a_profile_closes_part_of_the_gap_not_all_of_it() {
        // Through `for_camera`, not the `A6400` const directly, so the check is
        // on the resolution path and not a compile-time-constant assertion.
        // Green measured 1.355; the committed delta is (1.355-1)*0.65.
        let green = LookProfile::for_camera("Sony", "ILCE-6400").saturation[3];
        assert!(green > 0.0, "green should be lifted");
        assert!(
            green < 0.355,
            "a profile that closed the whole gap would be a camera clone"
        );
        assert!((green - 0.355 * DAMPING).abs() < 1e-4);
    }

    /// Aqua ran *over* the camera, so its correction pulls saturation down.
    /// A profile that could only add would get this hue wrong.
    #[test]
    fn an_over_saturated_band_is_pulled_down() {
        let aqua = LookProfile::for_camera("Sony", "ILCE-6400").saturation[4];
        assert!(aqua < 0.0, "aqua should desaturate");
    }

    /// Composition is multiplicative, not additive: a +0.3 profile under a +0.3
    /// user is 1.69, not 1.6. Checked at a band centre where interpolation is
    /// exact so the arithmetic is unambiguous.
    #[test]
    fn profile_and_user_saturation_compose_by_multiplying() {
        let profile = LookProfile {
            saturation: [0.3; BANDS],
        };
        let mixer = ColorMixer {
            saturation: [0.3; BANDS],
            ..Default::default()
        };

        let lut = profile.build_lut_under(&mixer);
        // Band 0 sits at hue 0, which is LUT entry 0 exactly.
        let combined_sat = lut[0][1];
        assert!((combined_sat - (1.3 * 1.3 - 1.0)).abs() < 1e-5);
    }
}
