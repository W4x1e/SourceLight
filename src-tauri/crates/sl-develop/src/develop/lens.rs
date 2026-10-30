//! Lens corrections: distortion, vignetting and lateral chromatic aberration.
//!
//! **Manual, because the parameters are not available.** The plan allowed three
//! routes: read the correction figures a camera embeds in its maker notes, use
//! `lensfun`'s database, or expose sliders. The first is out — LibRaw's
//! `libraw_lensinfo_t` carries lens *identification* only (make, model, focal
//! length, aperture, serial) and the generated bindings contain no distortion,
//! vignetting or aberration fields at all, so getting at them would mean
//! writing a maker-note parser per manufacturer. `lensfun` is LGPL-3, another
//! shared library to vendor and clear.
//!
//! Sliders work on any lens on any body, need no dependency, and — this is the
//! part that matters — use the *same* correction maths a profile would drive.
//! Adding a database later becomes a matter of filling these numbers in rather
//! than building the corrections again.
//!
//! All three are radial functions of distance from the frame centre, which is
//! why they belong together: one squared radius drives all of them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct LensCorrection {
    /// -1..=1. Positive pulls the corners in, correcting barrel distortion;
    /// negative pushes them out, correcting pincushion.
    pub distortion: f32,
    /// -1..=1. Positive brightens the corners, which is what corrects
    /// vignetting; negative darkens them, which is a look rather than a fix.
    pub vignetting: f32,
    /// -1..=1. Lateral chromatic aberration: red and blue are magnified
    /// slightly differently, so a correction scales them apart again.
    pub chromatic: f32,
}

impl LensCorrection {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    /// The four numbers the shader needs.
    ///
    /// Scaled here rather than in WGSL so the strengths are chosen — and
    /// justified — in one testable place.
    pub fn to_shader(&self) -> [f32; 4] {
        [
            // A quarter of the normalised radius at full deflection. Enough to
            // straighten a wide-angle zoom's barrel, and past the point where
            // the corners fold over themselves.
            self.distortion.clamp(-1.0, 1.0) * 0.25,
            // Doubling the corners at full strength: a fast lens wide open can
            // lose two stops there, which is exactly a factor of four in the
            // other direction, so this covers the common case without letting
            // one slider blow the corners out.
            self.vignetting.clamp(-1.0, 1.0) * 1.0,
            // Lateral aberration is a magnification difference of a fraction of
            // a percent even on a poor lens; anything larger is smearing the
            // image rather than aligning it.
            self.chromatic.clamp(-1.0, 1.0) * 0.004,
            0.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Neutral has to mean *exactly* nothing, since every unedited photo takes
    /// this path through three extra radial computations.
    #[test]
    fn a_neutral_correction_asks_for_nothing() {
        let p = LensCorrection::default().to_shader();
        assert_eq!(&p[..3], &[0.0, 0.0, 0.0], "neutral should be all zeros");
        assert!(LensCorrection::default().is_neutral());
    }

    /// Signs follow what the control claims: positive distortion corrects
    /// barrel, positive vignetting brightens corners. Getting either backwards
    /// is invisible until someone drags the slider the wrong way and it helps.
    #[test]
    fn the_controls_point_the_way_they_are_labelled() {
        let barrel = LensCorrection {
            distortion: 1.0,
            ..Default::default()
        }
        .to_shader();
        assert!(barrel[0] > 0.0);

        let bright = LensCorrection {
            vignetting: 1.0,
            ..Default::default()
        }
        .to_shader();
        assert!(bright[1] > 0.0);
    }

    /// Bounded, and bounded tightly for aberration.
    ///
    /// Distortion past about a third folds the corners of the frame back
    /// through themselves, and an aberration "correction" of more than a
    /// percent is displacing colour further than any lens misplaced it.
    #[test]
    fn the_strengths_stay_inside_sane_limits() {
        for d in [-1.0f32, -0.5, 0.5, 1.0, 5.0, -5.0] {
            let p = LensCorrection {
                distortion: d,
                vignetting: d,
                chromatic: d,
            }
            .to_shader();
            assert!(p[0].abs() <= 0.25 + 1e-6, "distortion {} too strong", p[0]);
            assert!(p[1].abs() <= 1.0 + 1e-6, "vignetting {} too strong", p[1]);
            assert!(p[2].abs() <= 0.005, "aberration {} too strong", p[2]);
        }
    }
}
