//! The non-destructive edit stack.
//!
//! An [`EditStack`] is the complete description of how a photo should look. It
//! is small — a few hundred bytes of JSON — which shapes two decisions:
//! history is stored as whole snapshots rather than a command pattern, and the
//! stack lives in the `photos.develop_settings` column rather than a side table.
//!
//! **Every field carries `#[serde(default)]` and the struct carries a
//! `version`.** Stacks written today have to keep loading after the develop
//! module has grown twenty more parameters, so a missing field must mean "the
//! default" rather than a parse failure. That is the entire forward-compatibility
//! mechanism; do not remove it to tidy up the generated TypeScript.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::develop::{
    curve, detail, effects, erase, geometry, grading, lens, lut, mask, mixer, spot,
};

/// Bumped only for changes serde defaults cannot absorb — a renamed field, or
/// one whose meaning changes. Adding a parameter does not need a bump.
///
/// - **2**: exposure 0 is where the camera's own JPEG put the frame, not a
///   fixed placement (`gpu::pipeline::Baseline`). A version-1 stack renders
///   as it always did.
/// - **3**: warmth and tint say what light the photo was under, and it is
///   adapted from that light in the working space (`develop::white_balance`),
///   the same on every camera, rather than by scaling the camera's channels.
///   An earlier stack's balance renders as it always did.
/// - **4**: the colour mixer and colour-range masks read hue, chroma and
///   lightness in OkLCh (`develop::oklab`) rather than HSV on the working
///   space. An earlier stack's mixer and ranges render as they always did.
pub const CURRENT_VERSION: u32 = 4;

/// The version whose meanings are these: whether exposure is measured from
/// the camera's brightness, whether the balance is by temperature, and
/// whether hue is read in OkLCh. Each meaning came with a version and
/// needs the ones before it; a mix no version has keeps the earlier
/// meanings exact and loses the later ones.
fn version_for(camera_matched: bool, by_temperature: bool, perceptual_hue: bool) -> u32 {
    match (camera_matched, by_temperature, perceptual_hue) {
        (true, true, true) => 4,
        (true, true, false) => 3,
        (true, false, _) => 2,
        (false, _, _) => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct EditStack {
    pub version: u32,

    /// Exposure in stops. Applied as a linear multiply of `2^exposure`, so +1
    /// is exactly one stop brighter.
    pub exposure: f32,

    /// White-balance adjustment **relative to the camera's as-shot setting**,
    /// -1..=1. Positive warms (more red, less blue).
    ///
    /// From version 3 it moves the light the photo is adapted from along the
    /// Planckian locus (`develop::white_balance`): 175 mireds a unit at first,
    /// up to the top of the temperature scale at +1 and about 2800 K from
    /// daylight at -1. Relative, not degrees Kelvin, since the as-shot light
    /// is its zero. Before, it scaled the camera's red and blue channels
    /// (`wb_scale`), which moved each camera differently.
    pub warmth: f32,
    /// Green/magenta balance, relative to as-shot, -1..=1. Positive pushes
    /// towards magenta: from version 3, the light moves 0.013 Δuv a unit
    /// towards green, which balancing takes back out.
    pub tint: f32,

    // Tone controls, all -1..=1 with 0 neutral. See `tone.rs` for the curve
    // these drive; the shader never sees them directly, only the baked LUT.
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,

    /// Global colour intensity, both -1..=1 with 0 neutral. Applied in the
    /// working space after the mixer and grade — `saturation` scales every
    /// pixel's distance from grey equally, while `vibrance` weights the boost
    /// toward the pixels that are least saturated already, so it lifts muted
    /// colour first and leaves the vivid parts alone. Both add no version bump:
    /// a stack written before they existed loads with them at 0, a no-op.
    pub vibrance: f32,
    pub saturation: f32,

    // Colour mixer: one entry per hue band, ordered as `mixer::BAND_HUES`.
    // Flat arrays rather than a nested struct so the generated TypeScript stays
    // a plain `number[]` the sliders can index.
    pub mixer_hue: [f32; mixer::BANDS],
    pub mixer_saturation: [f32; mixer::BANDS],
    pub mixer_luminance: [f32; mixer::BANDS],

    /// Point-curve control points, in display-referred 0..1.
    ///
    /// Empty means the identity, which is why this can be added without a
    /// version bump: a stack written before the curve existed deserialises to
    /// an empty vector and renders exactly as it did before.
    pub curve: Vec<curve::CurvePoint>,

    /// Per-channel point curves (red, green, blue), each display-referred 0..1.
    /// Empty means identity — so, like `curve`, they add no version bump and a
    /// pre-existing stack loads unchanged.
    #[serde(default)]
    pub curve_red: Vec<curve::CurvePoint>,
    #[serde(default)]
    pub curve_green: Vec<curve::CurvePoint>,
    #[serde(default)]
    pub curve_blue: Vec<curve::CurvePoint>,

    /// Shadow/midtone/highlight colour wheels. Neutral at rest, and neutral
    /// means a byte-level no-op, so this also needs no version bump.
    pub grading: grading::ColorGrading,

    /// Crop, straighten, rotation and mirroring. A pure coordinate transform in
    /// the develop pass, so adjusting it never re-runs the demosaic.
    pub geometry: geometry::Geometry,

    /// Local contrast at two scales. Neutral at rest.
    pub detail: detail::Detail,

    /// Distortion, vignetting and lateral chromatic aberration. Manual, since
    /// LibRaw exposes no correction figures — see `develop::lens`.
    pub lens: lens::LensCorrection,

    /// Local adjustments — masks that confine an edit to part of the frame.
    /// Empty means none, so this needs no version bump: a stack written before
    /// masks existed deserialises to an empty vector and renders identically.
    pub masks: Vec<mask::Mask>,

    /// Spot / heal / clone patches. Same story as `masks`: absent means an empty
    /// vector, so no version bump.
    #[serde(default)]
    pub spots: Vec<spot::Spot>,

    /// Objects taken out with magic erase, in the order they were. Each names
    /// its fills, which are files of their own (`develop::erase`), and where
    /// they go on the sensor, so a later crop or turn leaves them in place.
    /// They belong to this photo alone: no copy, paste or preset takes them.
    #[serde(default)]
    pub erases: Vec<erase::Erase>,

    /// The mask whose coverage a preview tints, while masking. A view
    /// setting carried on the stack it renders, never part of the edit: not
    /// saved, not sent, not in history.
    #[serde(skip)]
    #[specta(skip)]
    pub show_mask: Option<u32>,

    /// Finishing effects: post-crop vignette and film grain. Neutral at rest.
    #[serde(default)]
    pub effects: effects::Effects,

    /// Creative "look" LUT applied as the final stage. Empty name = none.
    #[serde(default)]
    pub lut: lut::LutRef,

    /// The built-in preset on this edit and its amount, so the amount can be
    /// changed later. A record only: the preset's values are in the fields
    /// above. Empty id = none.
    #[serde(default)]
    pub preset: super::presets::PresetRef,
}

impl Default for EditStack {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            exposure: 0.0,
            warmth: 0.0,
            tint: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            mixer_hue: [0.0; mixer::BANDS],
            mixer_saturation: [0.0; mixer::BANDS],
            mixer_luminance: [0.0; mixer::BANDS],
            curve: Vec::new(),
            curve_red: Vec::new(),
            curve_green: Vec::new(),
            curve_blue: Vec::new(),
            grading: grading::ColorGrading::default(),
            geometry: geometry::Geometry::default(),
            detail: detail::Detail::default(),
            lens: lens::LensCorrection::default(),
            masks: Vec::new(),
            spots: Vec::new(),
            erases: Vec::new(),
            show_mask: None,
            effects: effects::Effects::default(),
            lut: lut::LutRef::default(),
            preset: super::presets::PresetRef::default(),
        }
    }
}

impl EditStack {
    /// True when the photo is untouched, so the UI can distinguish "no edits"
    /// from "edited back to zero" without a separate flag.
    ///
    /// Compares against a default rather than listing fields, so a new
    /// parameter cannot be forgotten here and silently make edited photos look
    /// untouched. `version` is excluded because a stack written by an older
    /// build is still unedited.
    pub fn is_default(&self) -> bool {
        Self {
            version: self.version,
            ..Self::default()
        } == *self
    }

    /// The colour-mixer bands, for `mixer::ColorMixer::build_lut`.
    pub fn mixer(&self) -> mixer::ColorMixer {
        mixer::ColorMixer {
            hue: self.mixer_hue,
            saturation: self.mixer_saturation,
            luminance: self.mixer_luminance,
        }
    }

    /// The tone controls, for `tone::ToneParams::build_lut`.
    pub fn tone(&self) -> crate::develop::ToneParams {
        crate::develop::ToneParams {
            contrast: self.contrast,
            highlights: self.highlights,
            shadows: self.shadows,
            whites: self.whites,
            blacks: self.blacks,
        }
    }

    /// The finished tone table: the parametric curve with the point curve
    /// composed on top.
    ///
    /// Both stages live in one table, so the shader samples once and neither
    /// the develop pass nor the export path needs to know the point curve
    /// exists. The order is fixed here rather than at the call sites — the
    /// parametric curve maps scene light to display, and the point curve
    /// reshapes that display-referred result, which is what makes its 0..1
    /// axes mean what the panel shows.
    pub fn tone_lut(&self) -> Vec<f32> {
        let mut lut = self.tone().build_lut();
        curve::Curve::new(&self.curve).apply_to_lut(&mut lut);
        lut
    }

    /// The per-channel (red/green/blue) curve LUT the shader samples after tone.
    /// Identity when no channel is touched, so it is a no-op then.
    pub fn channel_lut(&self) -> Vec<f32> {
        curve::channel_lut(
            &self.curve_red,
            &self.curve_green,
            &self.curve_blue,
            curve::CHANNEL_LUT_SIZE,
        )
    }

    /// Per-channel multipliers a version-1 or 2 stack applies *on top of* the
    /// camera's as-shot white balance, in camera-RGB channel order. A newer
    /// one moves the light instead (`develop::white_balance`), which needs
    /// the camera's matrix and so is the session's to work out
    /// (`DevelopSession::white_balance`).
    ///
    /// Warmth pushes red and blue in opposite directions, which is what makes
    /// it read as a colour-temperature change; tint moves green against both.
    /// Green is held at 1.0 so overall brightness stays put and exposure
    /// remains the only control that changes it.
    pub fn wb_scale(&self) -> [f32; 3] {
        let warmth = self.warmth.clamp(-1.0, 1.0);
        let tint = self.tint.clamp(-1.0, 1.0);
        [
            (1.0 + warmth * 0.5) * (1.0 + tint * 0.1),
            1.0,
            (1.0 - warmth * 0.5) * (1.0 + tint * 0.1),
        ]
    }

    /// Linear exposure multiplier.
    pub fn exposure_scale(&self) -> f32 {
        self.exposure.exp2()
    }

    /// Whether exposure 0 means the camera's own brightness (version 2 on),
    /// rather than the fixed placement a version-1 stack was edited against.
    pub fn camera_matched(&self) -> bool {
        self.version >= 2
    }

    /// Whether warmth and tint move the light by temperature (version 3 on),
    /// rather than scaling the camera's channels (`wb_scale`) as an earlier
    /// stack's did.
    pub fn by_temperature(&self) -> bool {
        self.version >= 3
    }

    /// Whether the colour mixer and colour-range masks read OkLCh (version 4
    /// on), rather than HSV on the working space as an earlier stack's did.
    pub fn perceptual_hue(&self) -> bool {
        self.version >= 4
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("serialising edit stack")
    }

    /// Parses a stored stack. Unknown fields are ignored and missing ones take
    /// their defaults, so a stack written by any earlier build still loads.
    pub fn from_json(json: &str) -> Result<Self> {
        let stack: Self = serde_json::from_str(json).context("parsing edit stack")?;
        Ok(stack.sanitised())
    }

    /// The stack with its masks put within what can be rendered; see
    /// `mask::sanitise`. Applied where a stack comes in: the catalogue, and
    /// each command that takes one from the page.
    pub fn sanitised(mut self) -> Self {
        mask::sanitise(&mut self.masks);
        self
    }
}

/// A part of an edit that copies, pastes and saves as a preset on its own.
/// Copying the whole stack carried the crop, spot patches and mask
/// bitmaps — things that fit one photo's content — onto every photo it touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    WhiteBalance,
    Light,
    Color,
    Curves,
    Detail,
    Lens,
    Effects,
    Lut,
    Crop,
    Masks,
    Spots,
}

impl Group {
    pub const ALL: [Group; 11] = [
        Group::WhiteBalance,
        Group::Light,
        Group::Color,
        Group::Curves,
        Group::Detail,
        Group::Lens,
        Group::Effects,
        Group::Lut,
        Group::Crop,
        Group::Masks,
        Group::Spots,
    ];

    /// What a copy takes unless told otherwise: everything that means the same
    /// on another photo. The crop, spots and masks are drawn on this one's
    /// content, so they stay behind.
    pub fn copied_by_default(self) -> bool {
        !matches!(self, Group::Crop | Group::Masks | Group::Spots)
    }
}

impl EditStack {
    /// `onto`, with the chosen groups taken from `self` and the rest kept.
    ///
    /// What exposure, the balance and hue mean depends on the version
    /// (`camera_matched`, `by_temperature`, `perceptual_hue`), so a group that
    /// moves one of them brings that meaning with it, and the pasted values
    /// look as they did.
    pub fn take_groups(&self, onto: &EditStack, groups: &[Group]) -> EditStack {
        let mut out = onto.clone();
        let meanings = |s: &EditStack| (s.camera_matched(), s.by_temperature(), s.perceptual_hue());
        let (mut camera_matched, mut by_temperature, mut perceptual_hue) = meanings(onto);
        for g in groups {
            match g {
                Group::WhiteBalance => {
                    by_temperature = self.by_temperature();
                    out.warmth = self.warmth;
                    out.tint = self.tint;
                }
                Group::Light => {
                    camera_matched = self.camera_matched();
                    out.exposure = self.exposure;
                    out.contrast = self.contrast;
                    out.highlights = self.highlights;
                    out.shadows = self.shadows;
                    out.whites = self.whites;
                    out.blacks = self.blacks;
                }
                Group::Color => {
                    perceptual_hue = self.perceptual_hue();
                    out.vibrance = self.vibrance;
                    out.saturation = self.saturation;
                    out.mixer_hue = self.mixer_hue;
                    out.mixer_saturation = self.mixer_saturation;
                    out.mixer_luminance = self.mixer_luminance;
                    out.grading = self.grading;
                    // A preset is mostly colour: its record travels with it.
                    out.preset = self.preset.clone();
                }
                Group::Curves => {
                    out.curve = self.curve.clone();
                    out.curve_red = self.curve_red.clone();
                    out.curve_green = self.curve_green.clone();
                    out.curve_blue = self.curve_blue.clone();
                }
                Group::Detail => out.detail = self.detail,
                Group::Lens => out.lens = self.lens,
                Group::Effects => out.effects = self.effects,
                Group::Lut => out.lut = self.lut.clone(),
                Group::Crop => out.geometry = self.geometry,
                Group::Masks => {
                    // Colour-range masks read hue as the version says.
                    perceptual_hue = self.perceptual_hue();
                    out.masks = self.masks.clone();
                }
                Group::Spots => out.spots = self.spots.clone(),
            }
        }
        if (camera_matched, by_temperature, perceptual_hue) != meanings(onto) {
            out.version = version_for(camera_matched, by_temperature, perceptual_hue);
        }
        out
    }
}

/// Undo/redo over whole-stack snapshots.
///
/// Snapshots rather than inverse operations: a stack is small enough that
/// storing complete copies costs nothing, and it removes an entire class of bug
/// where an undo fails to exactly reverse its redo.
#[derive(Debug, Clone)]
pub struct History {
    entries: Vec<(String, EditStack)>,
    /// Index of the currently applied entry. Always valid — the initial state
    /// is entry 0, so the vector is never empty.
    cursor: usize,
}

impl History {
    pub fn new(initial: EditStack) -> Self {
        Self {
            entries: vec![("Open".to_string(), initial)],
            cursor: 0,
        }
    }

    pub fn current(&self) -> &EditStack {
        &self.entries[self.cursor].1
    }

    /// Records a new state. Anything previously undone is discarded, which is
    /// the behaviour every editor has: editing after undo forks from here.
    pub fn push(&mut self, label: impl Into<String>, stack: EditStack) {
        if self.current() == &stack {
            return; // Nothing changed; do not litter history with no-ops.
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push((label.into(), stack));
        self.cursor = self.entries.len() - 1;
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn undo(&mut self) -> Option<&EditStack> {
        if !self.can_undo() {
            return None;
        }
        self.cursor -= 1;
        Some(self.current())
    }

    pub fn redo(&mut self) -> Option<&EditStack> {
        if !self.can_redo() {
            return None;
        }
        self.cursor += 1;
        Some(self.current())
    }

    /// Moves to entry `index`, as that many undos or redos would. Out of
    /// range is ignored.
    pub fn go_to(&mut self, index: usize) -> Option<&EditStack> {
        if index >= self.entries.len() {
            return None;
        }
        self.cursor = index;
        Some(self.current())
    }

    /// Entry labels oldest-first, and which one is applied — for a history panel.
    pub fn labels(&self) -> (Vec<String>, usize) {
        (
            self.entries.iter().map(|(l, _)| l.clone()).collect(),
            self.cursor,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each group moves exactly its own fields: every field of the
    /// stack is some group's, and taking all of them is taking the stack.
    #[test]
    fn taking_every_group_takes_the_whole_stack_and_none_takes_nothing() {
        let mut from = EditStack {
            exposure: 0.4,
            warmth: 0.2,
            tint: -0.1,
            contrast: 0.3,
            highlights: -0.2,
            shadows: 0.2,
            whites: 0.1,
            blacks: -0.1,
            vibrance: 0.3,
            saturation: -0.2,
            ..Default::default()
        };
        from.mixer_hue[0] = 0.5;
        from.geometry.straighten = 3.0;
        from.curve = vec![curve::CurvePoint { x: 0.5, y: 0.6 }];
        let onto = EditStack::default();
        assert_eq!(from.take_groups(&onto, &Group::ALL), from);
        assert_eq!(from.take_groups(&onto, &[]), onto);
    }

    /// Erases are fills for one photo's own pixels, so no group carries them
    /// to another: the photo pasted onto keeps its own.
    #[test]
    fn erases_stay_with_their_photo() {
        let erase = |name: &str| erase::Erase {
            label: name.into(),
            pieces: vec![erase::Piece {
                fill: "a".repeat(64),
                x: 0,
                y: 0,
                size: 512,
            }],
        };
        let from = EditStack {
            erases: vec![erase("theirs")],
            ..Default::default()
        };
        let onto = EditStack {
            erases: vec![erase("ours")],
            ..Default::default()
        };
        assert_eq!(from.take_groups(&onto, &Group::ALL).erases, onto.erases);
    }

    #[test]
    fn a_group_moves_only_its_own_fields() {
        let mut from = EditStack {
            exposure: 0.4,
            warmth: 0.2,
            vibrance: 0.3,
            ..Default::default()
        };
        from.geometry.straighten = 3.0;
        let mut onto = EditStack {
            exposure: -1.0,
            ..Default::default()
        };
        onto.geometry.straighten = -1.0;
        let got = from.take_groups(&onto, &[Group::WhiteBalance, Group::Color]);
        assert_eq!((got.warmth, got.vibrance), (0.2, 0.3));
        assert_eq!((got.exposure, got.geometry.straighten), (-1.0, -1.0));
        assert!(!Group::Crop.copied_by_default());
        assert!(!Group::Masks.copied_by_default());
        assert!(!Group::Spots.copied_by_default());
        assert!(Group::Light.copied_by_default());
    }

    #[test]
    fn history_jumps_to_any_step() {
        let mut h = History::new(EditStack::default());
        for i in 1..=3 {
            h.push(
                format!("step {i}"),
                EditStack {
                    exposure: i as f32,
                    ..Default::default()
                },
            );
        }
        assert_eq!(h.go_to(1).map(|s| s.exposure), Some(1.0));
        assert!(h.can_redo());
        assert_eq!(h.labels().1, 1);
        assert!(h.go_to(9).is_none(), "out of range is ignored");
        assert_eq!(h.labels().1, 1);
        assert_eq!(h.go_to(0).map(|s| s.exposure), Some(0.0));
    }

    #[test]
    fn a_default_stack_is_recognised_as_unedited() {
        assert!(EditStack::default().is_default());
        let edited = EditStack {
            exposure: 0.5,
            ..Default::default()
        };
        assert!(!edited.is_default());
    }

    #[test]
    fn exposure_is_measured_in_stops() {
        let stack = |ev| EditStack {
            exposure: ev,
            ..Default::default()
        };
        assert!((stack(0.0).exposure_scale() - 1.0).abs() < 1e-6);
        assert!((stack(1.0).exposure_scale() - 2.0).abs() < 1e-6);
        assert!((stack(-1.0).exposure_scale() - 0.5).abs() < 1e-6);
        assert!((stack(2.0).exposure_scale() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn neutral_white_balance_leaves_every_channel_alone() {
        assert_eq!(EditStack::default().wb_scale(), [1.0, 1.0, 1.0]);
    }

    /// Warmth has to move red and blue in *opposite* directions, or it is a
    /// brightness control wearing a colour-temperature label.
    #[test]
    fn warmth_trades_red_against_blue() {
        let warm = EditStack {
            warmth: 1.0,
            ..Default::default()
        }
        .wb_scale();
        assert!(warm[0] > 1.0, "red should rise when warming");
        assert!(warm[2] < 1.0, "blue should fall when warming");
        assert_eq!(warm[1], 1.0, "green is the anchor and must not move");

        let cool = EditStack {
            warmth: -1.0,
            ..Default::default()
        }
        .wb_scale();
        assert!(
            cool[0] < 1.0 && cool[2] > 1.0,
            "cooling should mirror warming"
        );
    }

    #[test]
    fn out_of_range_values_are_clamped_rather_than_extrapolated() {
        let a = EditStack {
            warmth: 5.0,
            ..Default::default()
        }
        .wb_scale();
        let b = EditStack {
            warmth: 1.0,
            ..Default::default()
        }
        .wb_scale();
        assert_eq!(a, b);
    }

    #[test]
    fn a_stack_round_trips_through_json() {
        let stack = EditStack {
            version: CURRENT_VERSION,
            exposure: -0.75,
            warmth: 0.25,
            tint: -0.1,
            contrast: 0.3,
            highlights: -0.4,
            shadows: 0.6,
            whites: 0.1,
            blacks: -0.2,
            vibrance: 0.4,
            saturation: -0.3,
            mixer_hue: [0.0; mixer::BANDS],
            mixer_saturation: [0.5; mixer::BANDS],
            mixer_luminance: [0.0; mixer::BANDS],
            curve: vec![
                curve::CurvePoint::new(0.0, 0.02),
                curve::CurvePoint::new(0.5, 0.55),
                curve::CurvePoint::new(1.0, 1.0),
            ],
            curve_red: vec![
                curve::CurvePoint::new(0.0, 0.05),
                curve::CurvePoint::new(1.0, 0.95),
            ],
            curve_green: Vec::new(),
            curve_blue: vec![
                curve::CurvePoint::new(0.0, 0.0),
                curve::CurvePoint::new(1.0, 0.9),
            ],
            grading: grading::ColorGrading {
                shadows: grading::GradeRange {
                    hue: 210.0,
                    saturation: 0.4,
                    luminance: -0.1,
                },
                balance: 0.2,
                ..Default::default()
            },
            geometry: geometry::Geometry {
                crop: geometry::CropRect {
                    x: 0.1,
                    y: 0.05,
                    width: 0.8,
                    height: 0.7,
                },
                straighten: -1.5,
                quarter_turns: 1,
                flip: true,
            },
            detail: detail::Detail {
                clarity: 0.3,
                texture: -0.2,
                sharpen: 0.4,
                sharpen_radius: 1.5,
                sharpen_masking: 0.6,
                sharpen_detail: 0.7,
                noise: 0.25,
                color_noise: 0.35,
                dehaze: 0.5,
                ai_denoise: 0.6,
            },
            lens: lens::LensCorrection {
                distortion: -0.3,
                vignetting: 0.5,
                chromatic: 0.2,
            },
            masks: vec![mask::Mask {
                components: vec![
                    mask::MaskComponent {
                        shape: mask::MaskShape::Radial {
                            cx: 0.5,
                            cy: 0.4,
                            radius_x: 0.3,
                            radius_y: 0.2,
                            angle: 0.1,
                            feather: 0.5,
                        },
                        inverted: true,
                        mode: mask::MaskMode::Add,
                    },
                    mask::MaskComponent {
                        shape: mask::MaskShape::Brush {
                            strokes: vec![mask::Stroke {
                                x: 0.5,
                                y: 0.5,
                                radius: 0.1,
                                hardness: 0.5,
                                erase: false,
                            }],
                        },
                        inverted: false,
                        mode: mask::MaskMode::Subtract,
                    },
                ],
                adjustments: mask::LocalAdjustments {
                    exposure: 0.5,
                    contrast: -0.2,
                    highlights: -0.4,
                    shadows: 0.3,
                    temperature: 0.2,
                    tint: -0.1,
                    saturation: 0.3,
                    whites: -0.2,
                    blacks: 0.1,
                    clarity: 0.4,
                    texture: -0.3,
                },
                name: Some("Sky".into()),
            }],
            spots: vec![spot::Spot {
                target_x: 0.6,
                target_y: 0.4,
                source_x: 0.3,
                source_y: 0.35,
                radius: 0.05,
                feather: 0.4,
                heal: true,
            }],
            erases: vec![erase::Erase {
                label: "Person".into(),
                pieces: vec![erase::Piece {
                    fill: "0f".repeat(32),
                    x: -40,
                    y: 1200,
                    size: 1024,
                }],
            }],
            show_mask: None,
            effects: effects::Effects {
                vignette: -0.4,
                vignette_midpoint: 0.6,
                grain: 0.3,
                grain_size: 1.5,
                halation: 0.25,
                halation_threshold: 0.6,
                grain_highlight_rolloff: 0.5,
            },
            lut: lut::LutRef {
                name: "teal-orange.cube".into(),
                intensity: 0.8,
            },
            preset: super::super::presets::PresetRef {
                id: "film-warm".into(),
                amount: 0.6,
                tone: [
                    super::super::presets::ToneShare {
                        added: 0.1,
                        left: Some(0.7),
                    },
                    Default::default(),
                    Default::default(),
                    Default::default(),
                ],
            },
        };
        assert_eq!(
            EditStack::from_json(&stack.to_json().unwrap()).unwrap(),
            stack
        );
    }

    /// The forward-compatibility guarantee, stated as a test: a stack written
    /// before a parameter existed must still load, with that parameter at its
    /// default. Without it, adding a slider would invalidate every saved edit.
    #[test]
    fn a_stack_missing_newer_fields_still_loads() {
        let old = r#"{"version":1,"exposure":1.5}"#;
        let stack = EditStack::from_json(old).unwrap();
        assert_eq!(stack.exposure, 1.5);
        assert_eq!(stack.warmth, 0.0);
        assert_eq!(stack.tint, 0.0);
        // The point curve was added without a version bump, on the grounds
        // that an absent curve is the identity. That only holds if it really
        // deserialises to empty rather than to something the renderer would
        // apply.
        assert!(stack.curve.is_empty());
    }

    /// The composition order the whole design rests on: parametric curve
    /// first, point curve reshaping its output.
    ///
    /// Checked through the finished table rather than by inspecting the code
    /// path, because getting the order backwards still produces a plausible
    /// image — just one where the curve's axes no longer mean what the panel
    /// draws.
    #[test]
    fn the_point_curve_reshapes_the_parametric_table() {
        let stack = EditStack {
            curve: vec![
                curve::CurvePoint::new(0.0, 0.0),
                curve::CurvePoint::new(0.5, 0.25),
                curve::CurvePoint::new(1.0, 1.0),
            ],
            ..Default::default()
        };

        let parametric = stack.tone().build_lut();
        let composed = stack.tone_lut();
        assert_eq!(parametric.len(), composed.len());

        // Every entry must equal the curve applied to the parametric value.
        let c = curve::Curve::new(&stack.curve);
        for (a, b) in parametric.iter().zip(&composed) {
            assert!((c.eval(*a) - b).abs() < 1e-6);
        }
        // And the curve must actually have done something, or the assertion
        // above would pass on an identity.
        assert!(parametric.iter().zip(&composed).any(|(a, b)| a != b));
    }

    /// No curve must mean no change at all, byte for byte — this is the path
    /// every unedited photo takes.
    #[test]
    fn an_empty_curve_leaves_the_tone_table_untouched() {
        let stack = EditStack::default();
        assert_eq!(stack.tone_lut(), stack.tone().build_lut());
    }

    /// And the reverse: a stack from a *newer* build must not blow up an older
    /// one. Unknown fields are ignored rather than rejected.
    #[test]
    fn a_stack_with_unknown_fields_still_loads() {
        let future = r#"{"version":9,"exposure":0.5,"clarity":0.8,"dehaze":-0.2}"#;
        let stack = EditStack::from_json(future).unwrap();
        assert_eq!(stack.exposure, 0.5);
        assert_eq!(stack.version, 9);
    }

    /// A stack saved before exposure 0 meant the camera's own brightness
    /// keeps rendering as it was edited; a new one, and every unedited photo,
    /// opens at the camera's brightness.
    #[test]
    fn only_new_stacks_open_at_the_cameras_brightness() {
        let old = EditStack::from_json(r#"{"version":1,"exposure":0.3}"#).unwrap();
        assert!(!old.camera_matched());
        assert!(EditStack::default().camera_matched());
        // Saved and loaded again, an old stack stays old.
        let again = EditStack::from_json(&old.to_json().unwrap()).unwrap();
        assert!(!again.camera_matched());

        // Its exposure pasted onto a new photo means what it meant: measured
        // from where photos opened then. Pasting only colour leaves the new
        // photo at the camera's brightness.
        let new = EditStack::default();
        assert!(!old.take_groups(&new, &[Group::Light]).camera_matched());
        assert!(old.take_groups(&new, &[Group::Color]).camera_matched());
    }

    #[test]
    fn only_new_stacks_read_hue_perceptually() {
        assert!(EditStack::default().perceptual_hue());
        let three = EditStack {
            version: 3,
            ..Default::default()
        };
        assert!(!three.perceptual_hue());
        // An older mixer pasted onto a new photo keeps reading HSV, and the
        // photo keeps its brightness and balance.
        let got = three.take_groups(&EditStack::default(), &[Group::Color]);
        assert_eq!(got.version, 3);
        // A new one pasted onto a version-3 edit brings OkLCh with it.
        let got = EditStack::default().take_groups(&three, &[Group::Masks]);
        assert!(got.perceptual_hue() && got.by_temperature() && got.camera_matched());
        // Onto a version-2 edit it cannot, without changing its balance.
        let two = EditStack {
            version: 2,
            ..Default::default()
        };
        assert_eq!(
            EditStack::default()
                .take_groups(&two, &[Group::Color])
                .version,
            2
        );
    }

    #[test]
    fn only_new_stacks_balance_by_temperature() {
        assert!(EditStack::default().by_temperature());
        for version in [1, 2] {
            let old = EditStack {
                version,
                warmth: 0.3,
                ..Default::default()
            };
            assert!(!old.by_temperature());
            // Its balance pasted onto a new photo is scaled channels still,
            // and the new photo keeps its brightness.
            let got = old.take_groups(&EditStack::default(), &[Group::WhiteBalance]);
            assert_eq!((got.version, got.warmth), (2, 0.3));
        }
        // A new balance pasted onto a version-2 edit brings temperature with it.
        let two = EditStack {
            version: 2,
            ..Default::default()
        };
        let got = EditStack::default().take_groups(&two, &[Group::WhiteBalance]);
        assert!(got.by_temperature() && got.camera_matched());
        // Taking nothing that depends on the version leaves it alone.
        assert_eq!(
            EditStack::default()
                .take_groups(&two, &[Group::Color])
                .version,
            2
        );
    }

    #[test]
    fn history_undoes_and_redoes() {
        let mut h = History::new(EditStack::default());
        assert!(!h.can_undo() && !h.can_redo());

        let bright = EditStack {
            exposure: 1.0,
            ..Default::default()
        };
        h.push("Exposure", bright.clone());
        assert!(h.can_undo());
        assert_eq!(h.current(), &bright);

        assert_eq!(h.undo().unwrap(), &EditStack::default());
        assert!(h.can_redo());
        assert_eq!(h.redo().unwrap(), &bright);
    }

    #[test]
    fn pushing_an_identical_stack_is_not_recorded() {
        let mut h = History::new(EditStack::default());
        h.push("No-op", EditStack::default());
        assert!(
            !h.can_undo(),
            "an unchanged stack should not create history"
        );
    }

    /// Editing after an undo forks: the redo branch is gone, matching what
    /// every editor does.
    #[test]
    fn editing_after_undo_discards_the_redo_branch() {
        let mut h = History::new(EditStack::default());
        h.push(
            "A",
            EditStack {
                exposure: 1.0,
                ..Default::default()
            },
        );
        h.push(
            "B",
            EditStack {
                exposure: 2.0,
                ..Default::default()
            },
        );
        h.undo();
        assert!(h.can_redo());

        h.push(
            "C",
            EditStack {
                exposure: 3.0,
                ..Default::default()
            },
        );
        assert!(!h.can_redo(), "the old redo branch must be discarded");
        assert_eq!(h.current().exposure, 3.0);

        let (labels, cursor) = h.labels();
        assert_eq!(labels, vec!["Open", "A", "C"]);
        assert_eq!(cursor, 2);
    }
}
