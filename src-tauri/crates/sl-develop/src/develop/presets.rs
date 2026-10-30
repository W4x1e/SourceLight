//! Built-in **presets**: the creative treatments that ship with the app,
//! grouped by style (film, cinematic, black and white…) and by subject
//! (portrait, landscape, food…).
//!
//! A preset is deliberately *partial*. It owns the creative fields — contrast,
//! the tone curves, presence (clarity, texture, vibrance, saturation), the
//! colour mixer, colour grading and the finishing effects — and applies them
//! over the current edit. Exposure, white balance, crop, masks, spots, lens
//! corrections and any LUT are decisions about *this* photo, so a preset never
//! touches them: expose and crop a photo, then try presets on top. Each preset
//! first resets the fields it owns, so switching presets replaces the
//! treatment rather than stacking one on another.
//!
//! **Tone, by offsets.** Highlights, shadows, whites and blacks are where a
//! photo is rescued — a sky pulled back, a face lifted out of shade — so a
//! preset does not own them. It may *move* them: "shadows +0.15" adds to
//! whatever the photo has. The edit records what each slider was given and
//! where the preset left it ([`ToneShare`]), so taking the preset off, or
//! switching to another, gives back exactly its share. A slider moved since
//! (by hand, by Auto, by a paste) is no longer where the preset left it, and
//! from then on it is the photo's: the preset neither takes its share back
//! nor, while it stays on, adds to it again.
//!
//! **Amount.** A preset applies at an amount, 0 to [`MAX_AMOUNT`]. Every owned
//! field is neutral at zero, so the amount scales each one's distance from
//! neutral: sliders and wheel strengths multiply, curve points move toward or
//! past the diagonal. The edit records which preset is on and at what amount
//! ([`PresetRef`]), so the amount can be changed later, on any photo.
//!
//! **The pack is data**, `presets.json` beside this file, built into the
//! binary. A preset lists only the settings it changes. Keeping the values out
//! of code means they can be tuned by eye without a rebuild:
//! `bin/probe_presets.rs` renders any copy of the file on a set of real raws
//! as contact sheets. This module is pure, so what a preset touches and
//! preserves is unit-tested; the look is for the eye.

use crate::develop::curve::CurvePoint;
use crate::develop::effects::Effects;
use crate::develop::grading::{ColorGrading, GradeRange};
use crate::develop::mixer::BANDS;
use crate::develop::EditStack;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// The most a preset can be turned up: half as strong again as designed.
pub const MAX_AMOUNT: f32 = 1.5;

/// An edit's record of the built-in preset on it: which one, and how strongly.
/// Saved with the photo so the amount can be changed later. An empty `id`
/// means none. It describes the edit, it does not render anything: the
/// preset's values are in the stack's own fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct PresetRef {
    pub id: String,
    /// 0..=[`MAX_AMOUNT`], 1 as designed.
    pub amount: f32,
    /// What the preset did to highlights, shadows, whites and blacks, in that
    /// order (see the module docs).
    pub tone: [ToneShare; 4],
}

impl Default for PresetRef {
    fn default() -> Self {
        PresetRef {
            id: String::new(),
            amount: 1.0,
            tone: [ToneShare::default(); 4],
        }
    }
}

/// A preset's share of one tone slider: what it added, and the value it left.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ToneShare {
    pub added: f32,
    /// Where the preset left the slider; `None` when it does not hold it.
    pub left: Option<f32>,
}

impl ToneShare {
    /// Whether the slider, at `value`, is still where the preset left it.
    fn holds(&self, value: f32) -> bool {
        self.left.is_some_and(|left| (value - left).abs() < 1e-4)
    }
}

/// The tone sliders a preset moves by offsets, in [`PresetRef::tone`] order.
fn tone(s: &EditStack) -> [f32; 4] {
    [s.highlights, s.shadows, s.whites, s.blacks]
}

fn set_tone(s: &mut EditStack, v: [f32; 4]) {
    [s.highlights, s.shadows, s.whites, s.blacks] = v;
}

/// Whether a group sorts presets by the look they give or by what is in the
/// photo. The panels list the two kinds apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum GroupKind {
    Style,
    Subject,
}

/// One built-in preset, as the panels list it.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// One group of presets, as the panels list it.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetGroupInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: GroupKind,
    pub presets: Vec<PresetInfo>,
}

/// What a preset changes, as `presets.json` writes it: only the settings it
/// sets, the rest neutral. Unknown names are an error, so a typo in the file
/// fails its test rather than quietly doing nothing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct Settings {
    contrast: f32,
    /// Offsets added to the photo's own highlights, shadows, whites and
    /// blacks, not values that replace them (see the module docs).
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    vibrance: f32,
    saturation: f32,
    clarity: f32,
    texture: f32,
    /// Curves as `[input, output]` points, display-referred 0..1.
    curve: Vec<[f32; 2]>,
    curve_red: Vec<[f32; 2]>,
    curve_green: Vec<[f32; 2]>,
    curve_blue: Vec<[f32; 2]>,
    /// Per band: red, orange, yellow, green, aqua, blue, purple, magenta.
    mixer_hue: [f32; BANDS],
    mixer_saturation: [f32; BANDS],
    mixer_luminance: [f32; BANDS],
    grading: Grade,
    effects: Fx,
}

/// Colour grading: each range as `[hue in degrees, strength 0..1]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct Grade {
    shadows: Option<[f32; 2]>,
    midtones: Option<[f32; 2]>,
    highlights: Option<[f32; 2]>,
    blending: f32,
    balance: f32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct Fx {
    grain: f32,
    grain_size: Option<f32>,
    halation: f32,
    vignette: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preset {
    id: String,
    name: String,
    description: String,
    settings: Settings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Group {
    id: String,
    name: String,
    description: String,
    kind: GroupKind,
    presets: Vec<Preset>,
}

/// A set of preset groups: the built-in one, or (for tuning) a copy of
/// `presets.json` read from disk.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    groups: Vec<Group>,
}

/// Clears the fields a preset owns back to neutral. Everything not listed
/// here is the photo's own and survives every preset.
fn reset_owned(s: &mut EditStack) {
    s.contrast = 0.0;
    s.vibrance = 0.0;
    s.saturation = 0.0;
    s.detail.clarity = 0.0;
    s.detail.texture = 0.0;
    s.curve = Vec::new();
    s.curve_red = Vec::new();
    s.curve_green = Vec::new();
    s.curve_blue = Vec::new();
    s.mixer_hue = [0.0; BANDS];
    s.mixer_saturation = [0.0; BANDS];
    s.mixer_luminance = [0.0; BANDS];
    s.grading = ColorGrading::default();
    s.effects = Effects::default();
}

/// Writes a preset's settings onto a stack whose owned fields are neutral.
fn set_owned(s: &mut EditStack, p: &Settings) {
    let curve = |c: &[[f32; 2]]| c.iter().map(|&[x, y]| CurvePoint::new(x, y)).collect();
    let range = |r: Option<[f32; 2]>| {
        r.map(|[hue, saturation]| GradeRange {
            hue,
            saturation,
            luminance: 0.0,
        })
        .unwrap_or_default()
    };
    s.contrast = p.contrast;
    s.vibrance = p.vibrance;
    s.saturation = p.saturation;
    s.detail.clarity = p.clarity;
    s.detail.texture = p.texture;
    s.curve = curve(&p.curve);
    s.curve_red = curve(&p.curve_red);
    s.curve_green = curve(&p.curve_green);
    s.curve_blue = curve(&p.curve_blue);
    s.mixer_hue = p.mixer_hue;
    s.mixer_saturation = p.mixer_saturation;
    s.mixer_luminance = p.mixer_luminance;
    s.grading = ColorGrading {
        shadows: range(p.grading.shadows),
        midtones: range(p.grading.midtones),
        highlights: range(p.grading.highlights),
        blending: p.grading.blending,
        balance: p.grading.balance,
    };
    s.effects = Effects {
        grain: p.effects.grain,
        halation: p.effects.halation,
        vignette: p.effects.vignette,
        grain_size: p
            .effects
            .grain_size
            .unwrap_or(Effects::default().grain_size),
        ..Effects::default()
    };
}

/// Scales the owned fields of a freshly set preset by `amount`, each toward
/// or away from its neutral value.
fn scale_owned(s: &mut EditStack, amount: f32) {
    let k = |v: f32| (v * amount).clamp(-1.0, 1.0);
    s.contrast = k(s.contrast);
    s.vibrance = k(s.vibrance);
    s.saturation = k(s.saturation);
    s.detail.clarity = k(s.detail.clarity);
    s.detail.texture = k(s.detail.texture);
    for c in [
        &mut s.curve,
        &mut s.curve_red,
        &mut s.curve_green,
        &mut s.curve_blue,
    ] {
        for p in c.iter_mut() {
            p.y = (p.x + (p.y - p.x) * amount).clamp(0.0, 1.0);
        }
    }
    for band in 0..BANDS {
        s.mixer_hue[band] = k(s.mixer_hue[band]);
        s.mixer_saturation[band] = k(s.mixer_saturation[band]);
        s.mixer_luminance[band] = k(s.mixer_luminance[band]);
    }
    for r in [
        &mut s.grading.shadows,
        &mut s.grading.midtones,
        &mut s.grading.highlights,
    ] {
        r.saturation = (r.saturation * amount).clamp(0.0, 1.0);
        r.luminance = k(r.luminance);
    }
    s.grading.blending = k(s.grading.blending);
    s.grading.balance = k(s.grading.balance);
    s.effects.vignette = k(s.effects.vignette);
    s.effects.grain = (s.effects.grain * amount).clamp(0.0, 1.0);
    s.effects.halation = (s.effects.halation * amount).clamp(0.0, 1.0);
}

impl Pack {
    /// Reads a pack from `presets.json`'s format.
    pub fn parse(json: &str) -> anyhow::Result<Pack> {
        Ok(serde_json::from_str(json)?)
    }

    fn find(&self, id: &str) -> Option<&Preset> {
        self.groups
            .iter()
            .flat_map(|g| g.presets.iter())
            .find(|p| p.id == id)
    }

    /// Every group and its presets, in display order.
    pub fn catalogue(&self) -> Vec<PresetGroupInfo> {
        self.groups
            .iter()
            .map(|g| PresetGroupInfo {
                id: g.id.clone(),
                name: g.name.clone(),
                description: g.description.clone(),
                kind: g.kind,
                presets: g
                    .presets
                    .iter()
                    .map(|p| PresetInfo {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        description: p.description.clone(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Every preset id, in display order.
    pub fn ids(&self) -> Vec<&str> {
        self.groups
            .iter()
            .flat_map(|g| g.presets.iter().map(|p| p.id.as_str()))
            .collect()
    }

    /// A preset's English name, if it exists.
    pub fn name(&self, id: &str) -> Option<&str> {
        self.find(id).map(|p| p.name.as_str())
    }

    /// `base` with preset `id` applied at `amount`: the owned fields replaced,
    /// everything else kept, and the edit recording the preset. `None` for an
    /// unknown id.
    pub fn apply(&self, id: &str, amount: f32, base: &EditStack) -> Option<EditStack> {
        let preset = self.find(id)?;
        let amount = amount.clamp(0.0, MAX_AMOUNT);
        let p = &preset.settings;
        let offsets = [p.highlights, p.shadows, p.whites, p.blacks];

        // Tone first, from the edit as it stands: the previous preset's share
        // comes off each slider it still holds, then this one's goes on.
        let before = &base.preset;
        let same = !before.id.is_empty() && before.id == id;
        let mut values = tone(base);
        let mut shares = [ToneShare::default(); 4];
        for i in 0..4 {
            let share = before.tone[i];
            let from = if share.holds(values[i]) {
                Some(values[i] - share.added)
            } else if same {
                // Moved since this preset set it: the photo's now, and a
                // change of amount must not add to it again.
                None
            } else {
                Some(values[i])
            };
            if let Some(from) = from {
                let to = (from + offsets[i] * amount).clamp(-1.0, 1.0);
                values[i] = to;
                shares[i] = ToneShare {
                    added: to - from,
                    left: Some(to),
                };
            }
        }

        let mut out = base.clone();
        reset_owned(&mut out);
        set_owned(&mut out, p);
        scale_owned(&mut out, amount);
        set_tone(&mut out, values);
        out.preset = PresetRef {
            id: id.to_string(),
            amount,
            tone: shares,
        };
        Some(out)
    }
}

/// The pack built into the app.
fn builtin() -> &'static Pack {
    static PACK: OnceLock<Pack> = OnceLock::new();
    PACK.get_or_init(|| {
        // Parsed by a test, so a broken file never reaches a build.
        Pack::parse(include_str!("presets.json")).expect("presets.json parses")
    })
}

/// Every built-in group and its presets, in display order.
pub fn catalogue() -> Vec<PresetGroupInfo> {
    builtin().catalogue()
}

/// Every built-in preset id, in display order.
pub fn ids() -> Vec<&'static str> {
    builtin().ids()
}

/// A built-in preset's English name, if it exists.
pub fn name(id: &str) -> Option<&'static str> {
    builtin().name(id)
}

/// `base` with built-in preset `id` applied at `amount` (see [`Pack::apply`]).
pub fn apply(id: &str, amount: f32, base: &EditStack) -> Option<EditStack> {
    builtin().apply(id, amount, base)
}

/// `base` with its preset taken off: the owned fields back to neutral, its
/// share taken off each tone slider it still holds, and the record cleared. A
/// stack with no preset comes back unchanged.
pub fn remove(base: &EditStack) -> EditStack {
    let mut out = base.clone();
    if !out.preset.id.is_empty() {
        let mut values = tone(&out);
        for (v, share) in values.iter_mut().zip(out.preset.tone) {
            if share.holds(*v) {
                *v -= share.added;
            }
        }
        set_tone(&mut out, values);
        reset_owned(&mut out);
        out.preset = PresetRef::default();
    }
    out
}

#[cfg(test)]
mod tests {
    // Tests build a base stack by `default()` then set a few fields to check
    // preservation — clearer than a full struct literal for a large struct.
    #![allow(clippy::field_reassign_with_default)]
    use super::*;

    #[test]
    fn every_preset_and_group_is_listed_once_with_words() {
        let mut ids = std::collections::HashSet::new();
        let mut groups = std::collections::HashSet::new();
        for g in catalogue() {
            assert!(groups.insert(g.id.clone()), "duplicate group {}", g.id);
            assert!(!g.name.is_empty() && !g.description.is_empty());
            assert!(g.presets.len() >= 5, "{} is thin", g.id);
            for p in g.presets {
                assert!(ids.insert(p.id.clone()), "duplicate preset id {}", p.id);
                assert!(!p.name.is_empty() && !p.description.is_empty());
            }
        }
        assert_eq!(ids.len(), self::ids().len());
    }

    #[test]
    fn every_preset_changes_the_photo_and_records_itself() {
        let base = EditStack::default();
        for id in ids() {
            let on = apply(id, 1.0, &base).unwrap();
            assert_eq!(on.preset.id, id);
            let mut bare = on.clone();
            bare.preset = PresetRef::default();
            assert_ne!(bare, base, "{id} does nothing");
        }
        assert!(apply("does-not-exist", 1.0, &base).is_none());
    }

    #[test]
    fn a_preset_keeps_what_belongs_to_the_photo() {
        let mut base = EditStack::default();
        base.exposure = 0.8;
        base.warmth = 0.3;
        base.tint = -0.2;
        base.highlights = -0.5;
        base.shadows = 0.4;
        base.detail.dehaze = 0.2;
        base.detail.sharpen = 0.7;
        base.geometry.quarter_turns = 1;
        base.lut.name = "film.cube".into();
        base.contrast = 0.9; // owned: replaced

        let on = apply("mono-neutral", 1.0, &base).unwrap();
        assert_eq!(on.exposure, 0.8);
        assert_eq!(on.warmth, 0.3);
        assert_eq!(on.tint, -0.2);
        assert_eq!(on.highlights, -0.5);
        assert_eq!(on.shadows, 0.4);
        assert_eq!(on.detail.dehaze, 0.2);
        assert_eq!(on.detail.sharpen, 0.7);
        assert_eq!(on.geometry.quarter_turns, 1);
        assert_eq!(on.lut.name, "film.cube");
        assert_eq!(on.mixer_saturation, [-1.0; BANDS]);
        assert_ne!(on.contrast, 0.9);
    }

    #[test]
    fn switching_presets_replaces_rather_than_stacks() {
        let base = EditStack::default();
        let vivid = apply("bold-vivid", 1.0, &base).unwrap();
        let matte = apply("portrait-matte", 1.0, &vivid).unwrap();
        assert_eq!(matte, apply("portrait-matte", 1.0, &base).unwrap());
    }

    #[test]
    fn amount_scales_from_neutral() {
        let base = EditStack::default();
        let full = apply("cinematic-teal-orange", 1.0, &base).unwrap();
        let half = apply("cinematic-teal-orange", 0.5, &base).unwrap();
        assert!((half.contrast - full.contrast / 2.0).abs() < 1e-6);
        assert!(
            (half.grading.shadows.saturation - full.grading.shadows.saturation / 2.0).abs() < 1e-6
        );
        assert_eq!(half.grading.shadows.hue, full.grading.shadows.hue);
        let (f, h) = (full.curve[1], half.curve[1]);
        assert!((h.y - (h.x + (f.y - f.x) / 2.0)).abs() < 1e-6);
        assert_eq!(half.preset.amount, 0.5);

        // At zero, every owned field is neutral: curves on the diagonal, no
        // strength on any wheel (a wheel's hue means nothing without one).
        for id in ids() {
            let s = apply(id, 0.0, &base).unwrap();
            assert_eq!(s.contrast, 0.0, "{id}");
            assert_eq!((s.vibrance, s.saturation), (0.0, 0.0), "{id}");
            assert_eq!((s.detail.clarity, s.detail.texture), (0.0, 0.0), "{id}");
            for c in [&s.curve, &s.curve_red, &s.curve_green, &s.curve_blue] {
                assert!(c.iter().all(|p| (p.x - p.y).abs() < 1e-6), "{id}");
            }
            assert!(
                s.mixer_hue
                    .iter()
                    .chain(&s.mixer_saturation)
                    .chain(&s.mixer_luminance)
                    .all(|v| *v == 0.0),
                "{id}"
            );
            let g = s.grading;
            for r in [g.shadows, g.midtones, g.highlights] {
                assert_eq!((r.saturation, r.luminance), (0.0, 0.0), "{id}");
            }
            assert_eq!((g.blending, g.balance), (0.0, 0.0), "{id}");
            let e = s.effects;
            assert_eq!((e.grain, e.halation, e.vignette), (0.0, 0.0, 0.0), "{id}");
        }
    }

    #[test]
    fn amount_is_clamped_and_stays_in_range() {
        let base = EditStack::default();
        let over = apply("mono-high-contrast", 9.0, &base).unwrap();
        assert_eq!(over.preset.amount, MAX_AMOUNT);
        assert_eq!(over.mixer_saturation, [-1.0; BANDS], "past -1 is clamped");
        for id in ids() {
            let s = apply(id, MAX_AMOUNT, &base).unwrap();
            for c in [&s.curve, &s.curve_red, &s.curve_green, &s.curve_blue] {
                assert!(c.iter().all(|p| (0.0..=1.0).contains(&p.y)), "{id}");
            }
        }
    }

    /// A pack of two presets with tone offsets, for the tone tests.
    fn toned() -> Pack {
        Pack::parse(
            r#"{"groups": [{"id": "g", "name": "G", "description": "D", "kind": "style", "presets": [
                {"id": "lift", "name": "Lift", "description": "D", "settings": {"shadows": 0.2, "whites": 0.1}},
                {"id": "deep", "name": "Deep", "description": "D", "settings": {"shadows": -0.1, "blacks": -0.1}}
            ]}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn tone_offsets_add_to_the_photo_and_come_off_exactly() {
        let mut base = EditStack::default();
        base.highlights = -0.6; // a sky pulled back
        base.shadows = 0.3;
        let on = toned().apply("lift", 1.0, &base).unwrap();
        assert!((on.shadows - 0.5).abs() < 1e-6);
        assert!((on.whites - 0.1).abs() < 1e-6);
        assert_eq!(on.highlights, -0.6, "not the preset's to touch");
        let off = remove(&on);
        assert!((off.shadows - 0.3).abs() < 1e-6);
        assert!(off.whites.abs() < 1e-6);
        assert_eq!(off.highlights, -0.6);
        assert_eq!(off.preset, PresetRef::default());
    }

    #[test]
    fn tone_offsets_scale_with_amount_without_creeping() {
        let mut base = EditStack::default();
        base.shadows = 0.3;
        let pack = toned();
        let mut s = pack.apply("lift", 1.0, &base).unwrap();
        // A drag of the Amount slider: each step re-applies the same preset.
        for amount in [0.8, 0.5, 1.2, 0.5] {
            s = pack.apply("lift", amount, &s).unwrap();
        }
        assert!((s.shadows - (0.3 + 0.2 * 0.5)).abs() < 1e-5);
        assert!((remove(&s).shadows - 0.3).abs() < 1e-5);
    }

    #[test]
    fn switching_presets_gives_back_the_first_ones_tone() {
        let mut base = EditStack::default();
        base.shadows = 0.3;
        let pack = toned();
        let lift = pack.apply("lift", 1.0, &base).unwrap();
        let deep = pack.apply("deep", 1.0, &lift).unwrap();
        assert_eq!(deep, pack.apply("deep", 1.0, &base).unwrap());
        assert!((deep.shadows - 0.2).abs() < 1e-6);
        assert!(deep.whites.abs() < 1e-6, "lift's whites came back off");
    }

    #[test]
    fn a_tone_slider_moved_since_belongs_to_the_photo() {
        let pack = toned();
        let mut s = pack.apply("lift", 1.0, &EditStack::default()).unwrap();
        s.shadows = 0.05; // moved by hand, or by Auto
                          // A change of amount leaves it alone, and still moves the rest.
        let s = pack.apply("lift", 0.5, &s).unwrap();
        assert_eq!(s.shadows, 0.05);
        assert!((s.whites - 0.05).abs() < 1e-6);
        // Taking the preset off leaves it too.
        assert_eq!(remove(&s).shadows, 0.05);
        // Another preset takes it as the photo's and adds to it.
        let deep = pack.apply("deep", 1.0, &s).unwrap();
        assert!((deep.shadows - (0.05 - 0.1)).abs() < 1e-6);
    }

    #[test]
    fn a_tone_offset_is_clamped_and_still_comes_off_exactly() {
        let mut base = EditStack::default();
        base.shadows = 0.95;
        let on = toned().apply("lift", 1.5, &base).unwrap();
        assert_eq!(on.shadows, 1.0);
        assert!((remove(&on).shadows - 0.95).abs() < 1e-6);
    }

    #[test]
    fn removing_a_preset_goes_back_to_neutral_and_keeps_the_rest() {
        let mut base = EditStack::default();
        base.exposure = 0.5;
        let on = apply("film-warm", 1.0, &base).unwrap();
        assert_eq!(remove(&on), base);
        assert_eq!(remove(&base), base);
    }
}
