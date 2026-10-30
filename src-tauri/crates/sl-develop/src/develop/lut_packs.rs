//! Built-in creative LUT pack — original film-emulation "looks" that ship with
//! the app, so the Looks browser has real content the moment a library is opened
//! rather than an empty list until the user imports a `.cube`.
//!
//! Every look here is an *original* colour transform built from the film
//! primitives in [`crate::develop::film`]: a characteristic curve per channel
//! with its own toe and shoulder, hue-dependent colour bias, and where the look
//! calls for it a second print stage. Nothing is copied from a commercial LUT
//! product — no curve data, no tables, no names. What is modelled is the
//! published *behaviour* of a class of film: how a negative holds a highlight,
//! why a slide's neutrals stay clean, what a bleach bypass does to chroma.
//!
//! The earlier pack was composed from four toys — a red/blue trade, a
//! smoothstep, a saturation scale and a linear lift. It could produce a tint;
//! it could not produce a response. That is the whole difference, and it is why
//! this file was rewritten rather than extended.
//!
//! **Why write real files instead of resolving looks in memory?** A LUT's whole
//! value is that it is a portable, standard artifact: once seeded, a built-in
//! look is an ordinary `.cube` in the user's `luts` folder — it lists, resolves,
//! is referenced by presets, and can be copied out to another tool exactly like
//! one the user imported. The seed is guarded by a marker file, so a look the
//! user deletes stays deleted rather than reappearing.
//!
//! The transforms operate in the display-encoded space the LUT stage runs in
//! (see [`crate::develop::lut`]), on values in `[0, 1]`, and every primitive
//! clamps — so a look can push colour around but never emits an out-of-range or
//! non-finite texel.

use crate::develop::film::{
    channel_response, clamp01, hue_bias, lift_gain, luma, mix3, print_stage, saturation,
    split_tone, HdCurve, HueBand,
};
use crate::develop::lut::LUT_3D_DIM;
use anyhow::Result;
use std::path::Path;

/// Bumped when the pack's *set* changes, so a new release seeds looks added
/// since the user's last run without disturbing the ones already on disk (or
/// resurrecting ones they deleted from an earlier version).
///
/// v2 rebuilt every look on the film primitives. The seven v1 filenames are
/// kept deliberately: a saved preset references a LUT by filename, so renaming
/// them would silently unhook every preset built on one.
const MARKER: &str = ".northlight-luts-v2";

/// How the Looks browser groups the pack. Four families, because the four
/// answer different questions: what flatters a person, what makes a landscape
/// sing, what makes a frame look like a film still, and what black and white
/// does with colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Family {
    /// Colour negative: forgiving highlights, warm skin, gentle toe.
    Negative,
    /// Colour reversal (slide): clean neutrals, deep saturation, hard shoulder.
    Reversal,
    /// Print-emulation grades — a second characteristic curve on top.
    Cinema,
    /// Black and white, by which colour filter the response models.
    Mono,
}

impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Negative => "Negative",
            Self::Reversal => "Reversal",
            Self::Cinema => "Cinema",
            Self::Mono => "Mono",
        }
    }
}

/// One built-in look: the file it seeds to, how it is presented, and the colour
/// transform baked into it. A plain `fn` so the table stays a `const`.
struct Look {
    file: &'static str,
    title: &'static str,
    family: Family,
    blurb: &'static str,
    map: fn([f32; 3]) -> [f32; 3],
}

// --- shared response shapes ------------------------------------------------

/// A colour negative's tone response: a long toe that keeps shadow detail off
/// the floor, a generous shoulder so a bright sky compresses instead of
/// clipping, and slightly less contrast in blue than in red — which is what
/// makes a negative forgiving and warm rather than merely warm.
const NEGATIVE: [HdCurve; 3] = [
    HdCurve::new(0.10, 0.20, 1.06),
    HdCurve::new(0.09, 0.18, 1.02),
    HdCurve::new(0.07, 0.14, 0.96),
];

/// A reversal (slide) response: almost no toe — slide film loses its shadows
/// rather than softening them — a hard shoulder, and steep mid-tones.
const REVERSAL: [HdCurve; 3] = [
    HdCurve::new(0.02, 0.26, 1.22),
    HdCurve::new(0.02, 0.24, 1.20),
    HdCurve::new(0.03, 0.22, 1.18),
];

/// Skin: the band a portrait look must not wreck. Roughly 15°–55° hue.
const SKIN: HueBand = HueBand {
    center_deg: 32.0,
    width_deg: 70.0,
    saturation: 1.0,
    shift_deg: 0.0,
};

const fn skin(saturation: f32, shift_deg: f32) -> HueBand {
    HueBand {
        saturation,
        shift_deg,
        ..SKIN
    }
}

const fn band(center_deg: f32, width_deg: f32, saturation: f32, shift_deg: f32) -> HueBand {
    HueBand {
        center_deg,
        width_deg,
        saturation,
        shift_deg,
    }
}

/// A monochrome conversion by channel weighting — the classic coloured-filter
/// response. Red weighting darkens a blue sky and brightens skin; green
/// weighting does the opposite and flatters foliage.
fn mono(c: [f32; 3], weights: [f32; 3], curve: HdCurve) -> [f32; 3] {
    let g = clamp01(weights[0] * c[0] + weights[1] * c[1] + weights[2] * c[2]);
    let g = curve.apply(g);
    [g, g, g]
}

const LOOKS: &[Look] = &[
    // --- Negative ---------------------------------------------------------
    Look {
        file: "portrait-negative.cube",
        title: "Portrait Negative",
        family: Family::Negative,
        blurb: "Warm skin tones and muted greens. A good everyday portrait film.",
        map: |c| {
            let c = channel_response(c, NEGATIVE);
            let c = hue_bias(
                c,
                &[
                    skin(1.06, -3.0),
                    // Foliage pulled back so a green background stops competing
                    // with the person in front of it.
                    band(120.0, 110.0, 0.80, 6.0),
                ],
            );
            split_tone(c, [0.49, 0.50, 0.52], [0.53, 0.51, 0.47], 0.10)
        },
    },
    Look {
        file: "golden-portrait.cube",
        title: "Golden Portrait",
        family: Family::Negative,
        blurb: "Warm highlights with clean shadows.",
        map: |c| {
            let c = channel_response(
                c,
                [
                    HdCurve::new(0.08, 0.22, 1.04),
                    HdCurve::new(0.09, 0.18, 1.00),
                    HdCurve::new(0.12, 0.12, 0.94),
                ],
            );
            let c = hue_bias(c, &[skin(1.10, -4.0), band(200.0, 120.0, 0.88, 0.0)]);
            // Warmth in the highlights only, so the shadows stay neutral.
            split_tone(c, [0.50, 0.50, 0.50], [0.55, 0.51, 0.45], 0.14)
        },
    },
    Look {
        file: "sunlit-negative.cube",
        title: "Sunlit Negative",
        family: Family::Negative,
        blurb: "Warm and bright, with open shadows.",
        map: |c| {
            let c = channel_response(
                c,
                [
                    HdCurve::new(0.14, 0.24, 1.00),
                    HdCurve::new(0.14, 0.20, 0.98),
                    HdCurve::new(0.16, 0.14, 0.92),
                ],
            );
            let c = [
                lift_gain(c[0], 0.05, 1.0),
                lift_gain(c[1], 0.05, 0.99),
                lift_gain(c[2], 0.06, 0.97),
            ];
            let c = hue_bias(c, &[skin(1.08, -2.0), band(60.0, 70.0, 1.12, 0.0)]);
            saturation(c, 1.04)
        },
    },
    Look {
        file: "overcast-negative.cube",
        title: "Overcast Negative",
        family: Family::Negative,
        blurb: "Adds contrast to flat, grey light without making it cold.",
        map: |c| {
            let c = channel_response(
                c,
                [
                    HdCurve::new(0.05, 0.16, 1.20),
                    HdCurve::new(0.05, 0.15, 1.18),
                    HdCurve::new(0.06, 0.14, 1.14),
                ],
            );
            let c = hue_bias(c, &[skin(1.12, -5.0), band(210.0, 120.0, 0.92, -4.0)]);
            saturation(c, 1.10)
        },
    },
    Look {
        file: "faded-matte.cube",
        title: "Faded Matte",
        family: Family::Negative,
        blurb: "Lifted blacks and softer whites for a faded print look.",
        map: |c| {
            let c = channel_response(
                c,
                [
                    HdCurve::new(0.18, 0.24, 0.88),
                    HdCurve::new(0.18, 0.22, 0.88),
                    HdCurve::new(0.16, 0.20, 0.90),
                ],
            );
            // The matte base: black point up, white point down.
            let c = [
                lift_gain(c[0], 0.07, 0.93),
                lift_gain(c[1], 0.07, 0.93),
                lift_gain(c[2], 0.08, 0.92),
            ];
            let c = saturation(c, 0.86);
            split_tone(c, [0.48, 0.50, 0.54], [0.52, 0.50, 0.47], 0.12)
        },
    },
    // --- Reversal ---------------------------------------------------------
    Look {
        file: "northlight-chrome.cube",
        title: "SourceLight Chrome",
        family: Family::Reversal,
        blurb: "Slide film with cool shadows and warm highlights.",
        map: |c| {
            let c = channel_response(c, REVERSAL);
            let c = hue_bias(c, &[skin(1.02, 0.0), band(200.0, 130.0, 1.15, -3.0)]);
            let c = saturation(c, 1.10);
            split_tone(c, [0.47, 0.48, 0.55], [0.55, 0.51, 0.44], 0.16)
        },
    },
    Look {
        file: "velvet-chrome.cube",
        title: "Velvet Chrome",
        family: Family::Reversal,
        blurb: "Saturated slide film for landscapes: deep greens, strong blue skies, dark shadows.",
        map: |c| {
            let c = channel_response(
                c,
                [
                    HdCurve::new(0.01, 0.30, 1.30),
                    HdCurve::new(0.01, 0.28, 1.30),
                    HdCurve::new(0.02, 0.24, 1.26),
                ],
            );
            let c = hue_bias(
                c,
                &[
                    band(120.0, 110.0, 1.35, -8.0),
                    band(230.0, 110.0, 1.30, 4.0),
                    // Skin held back, because this much saturation makes a
                    // person in the frame look sunburnt.
                    skin(0.94, 0.0),
                ],
            );
            saturation(c, 1.12)
        },
    },
    Look {
        file: "daylight-chrome.cube",
        title: "Daylight Chrome",
        family: Family::Reversal,
        blurb: "Neutral slide film with clean whites and accurate colour.",
        map: |c| {
            let c = channel_response(c, REVERSAL);
            hue_bias(
                c,
                &[band(60.0, 80.0, 1.06, 0.0), band(240.0, 100.0, 1.06, 0.0)],
            )
        },
    },
    // --- Cinema -----------------------------------------------------------
    Look {
        file: "cinescope.cube",
        title: "Cinescope",
        family: Family::Cinema,
        blurb: "Teal shadows and orange highlights, with strong contrast.",
        map: |c| {
            let c = channel_response(c, NEGATIVE);
            let c = print_stage(c, 0.55);
            let c = hue_bias(c, &[skin(1.05, -4.0), band(195.0, 120.0, 1.18, 0.0)]);
            let c = saturation(c, 0.96);
            split_tone(c, [0.42, 0.52, 0.56], [0.58, 0.50, 0.42], 0.26)
        },
    },
    Look {
        file: "nocturne.cube",
        title: "Nocturne",
        family: Family::Cinema,
        blurb: "Cool and dark, for blue hour and night.",
        map: |c| {
            let c = channel_response(
                c,
                [
                    HdCurve::new(0.06, 0.14, 1.14),
                    HdCurve::new(0.06, 0.16, 1.12),
                    HdCurve::new(0.04, 0.22, 1.06),
                ],
            );
            let c = [
                lift_gain(c[0], 0.0, 0.90),
                lift_gain(c[1], 0.0, 0.91),
                lift_gain(c[2], 0.01, 0.96),
            ];
            let c = hue_bias(c, &[band(220.0, 140.0, 1.20, 0.0), skin(1.05, 0.0)]);
            let c = saturation(c, 0.92);
            split_tone(c, [0.44, 0.48, 0.58], [0.48, 0.50, 0.55], 0.22)
        },
    },
    Look {
        file: "ember.cube",
        title: "Ember",
        family: Family::Cinema,
        blurb: "Warm and dark, for firelight and tungsten.",
        map: |c| {
            let c = channel_response(
                c,
                [
                    HdCurve::new(0.06, 0.24, 1.02),
                    HdCurve::new(0.10, 0.16, 1.10),
                    HdCurve::new(0.16, 0.10, 1.18),
                ],
            );
            let c = print_stage(c, 0.35);
            let c = hue_bias(c, &[skin(1.12, -6.0), band(200.0, 120.0, 0.70, 0.0)]);
            split_tone(c, [0.53, 0.49, 0.46], [0.56, 0.51, 0.43], 0.20)
        },
    },
    Look {
        file: "silverline.cube",
        title: "Silverline",
        family: Family::Cinema,
        blurb: "Bleach bypass: less saturation, more contrast.",
        map: |c| {
            // The defining move: retain the silver layer — pull colour toward
            // luminance — then drive the print hard.
            let l = luma(c);
            let c = mix3(c, [l, l, l], 0.55);
            let c = channel_response(c, NEGATIVE);
            print_stage(c, 0.9)
        },
    },
    // --- Mono -------------------------------------------------------------
    Look {
        file: "mono-neutral.cube",
        title: "Mono (Panchromatic)",
        family: Family::Mono,
        blurb: "Neutral black and white.",
        map: |c| mono(c, [0.2126, 0.7152, 0.0722], HdCurve::new(0.06, 0.14, 1.12)),
    },
    Look {
        file: "mono-red.cube",
        title: "Mono (Red Filter)",
        family: Family::Mono,
        blurb: "Black and white with dark skies and light skin.",
        map: |c| mono(c, [0.55, 0.36, 0.09], HdCurve::new(0.04, 0.16, 1.24)),
    },
    Look {
        file: "mono-yellow.cube",
        title: "Mono (Yellow Filter)",
        family: Family::Mono,
        blurb: "Black and white with slightly darker skies.",
        map: |c| mono(c, [0.36, 0.54, 0.10], HdCurve::new(0.06, 0.14, 1.14)),
    },
    Look {
        file: "mono-green.cube",
        title: "Mono (Green Filter)",
        family: Family::Mono,
        blurb: "Black and white with lighter foliage and darker skin.",
        map: |c| mono(c, [0.18, 0.70, 0.12], HdCurve::new(0.06, 0.16, 1.16)),
    },
];

/// One built-in look, as the Looks browser lists it.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LookInfo {
    /// The `.cube` filename, which is what an `EditStack`'s `lut.name` uses.
    pub file: String,
    pub title: String,
    pub family: String,
    pub blurb: String,
}

/// Everything in the pack, in display order — how the Looks browser groups and
/// names the built-ins, since a filename on disk carries neither.
pub fn catalogue() -> Vec<LookInfo> {
    LOOKS
        .iter()
        .map(|l| LookInfo {
            file: l.file.to_string(),
            title: l.title.to_string(),
            family: l.family.as_str().to_string(),
            blurb: l.blurb.to_string(),
        })
        .collect()
}

/// The catalogue entry for a `.cube` filename, if it is one of ours.
pub fn describe(file: &str) -> Option<LookInfo> {
    LOOKS.iter().find(|l| l.file == file).map(|l| LookInfo {
        file: l.file.to_string(),
        title: l.title.to_string(),
        family: l.family.as_str().to_string(),
        blurb: l.blurb.to_string(),
    })
}

/// A built-in look's texels, as [`crate::develop::lut::load_texels`] reads
/// them from its seeded file: the same text, parsed the same way, so a look
/// renders the same where there is no file, as on a phone. `None` for a
/// name that is not one of ours.
pub fn texels(file: &str) -> Option<Vec<f32>> {
    let look = LOOKS.iter().find(|l| l.file == file)?;
    let cube = crate::develop::lut::CubeLut::parse(&cube_text(look)).ok()?;
    Some(cube.resample_to(LUT_3D_DIM))
}

/// Seeds any missing built-in looks into `dir`, once. After the first
/// successful run a marker file is written and subsequent calls are a cheap
/// `exists()` check — so a look the user deletes is not re-created, and opening
/// the picker never pays to rewrite the pack.
pub fn seed_builtins(dir: &Path) -> Result<()> {
    if dir.join(MARKER).exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    for look in LOOKS {
        let path = dir.join(look.file);
        // Never clobber a file already there — a user may have imported a LUT
        // of the same name, and re-seeding must not overwrite their content.
        if !path.exists() {
            std::fs::write(&path, cube_text(look))?;
        }
    }
    std::fs::write(dir.join(MARKER), b"SourceLight built-in LUT pack seeded.\n")?;
    Ok(())
}

/// Bakes a look into `.cube` text at the app's standard grid size.
fn cube_text(look: &Look) -> String {
    let dim = LUT_3D_DIM;
    let last = (dim - 1) as f32;
    // Header plus one line per grid entry; a 33³ file is a few hundred KB.
    let mut s = String::with_capacity(dim * dim * dim * 24 + 64);
    s.push_str(&format!("TITLE \"{}\"\n", look.title));
    s.push_str(&format!("LUT_3D_SIZE {dim}\n"));
    // Red fastest, then green, then blue — the `.cube` ordering the parser and
    // the GPU 3D texture both expect.
    for b in 0..dim {
        for g in 0..dim {
            for r in 0..dim {
                let inp = [r as f32 / last, g as f32 / last, b as f32 / last];
                let o = (look.map)(inp);
                s.push_str(&format!(
                    "{:.6} {:.6} {:.6}\n",
                    clamp01(o[0]),
                    clamp01(o[1]),
                    clamp01(o[2])
                ));
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::lut::CubeLut;

    /// Without a file, a look is exactly what its seeded file loads as.
    #[test]
    fn a_looks_texels_match_its_seeded_file() {
        let dir = std::env::temp_dir().join(format!("sl-looks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        seed_builtins(&dir).unwrap();
        let first = &catalogue()[0].file;
        assert_eq!(
            texels(first).unwrap(),
            crate::develop::lut::load_texels(&dir, first).unwrap()
        );
        assert_eq!(texels("someone-elses.cube"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_look_has_a_unique_file_and_a_title() {
        let mut files = std::collections::HashSet::new();
        let mut titles = std::collections::HashSet::new();
        for l in LOOKS {
            assert!(files.insert(l.file), "duplicate file {}", l.file);
            assert!(titles.insert(l.title), "duplicate title {}", l.title);
            assert!(l.file.ends_with(".cube"));
            assert!(!l.blurb.is_empty(), "{} needs a blurb", l.title);
        }
    }

    /// A saved preset references a LUT by filename. Renaming one of these
    /// silently unhooks every preset built on it, which is why the v1 names are
    /// kept even where a better name exists.
    #[test]
    fn the_original_filenames_still_exist() {
        for file in [
            "northlight-chrome.cube",
            "cinescope.cube",
            "faded-matte.cube",
            "silverline.cube",
            "golden-portrait.cube",
            "mono-red.cube",
            "nocturne.cube",
        ] {
            assert!(
                LOOKS.iter().any(|l| l.file == file),
                "{file} was dropped — presets referencing it would break"
            );
        }
    }

    #[test]
    fn every_look_bakes_to_a_parseable_in_range_cube() {
        for look in LOOKS {
            let text = cube_text(look);
            let lut = CubeLut::parse(&text)
                .unwrap_or_else(|e| panic!("{} does not parse: {e}", look.file));
            assert_eq!(lut.size, LUT_3D_DIM);
            for texel in &lut.data {
                for v in texel {
                    assert!(
                        v.is_finite() && (0.0..=1.0).contains(v),
                        "{} emitted {v}",
                        look.file
                    );
                }
            }
        }
    }

    #[test]
    fn every_look_actually_does_something() {
        // A look that is the identity is a bug, not a subtle look.
        for look in LOOKS {
            let probe = [[0.2, 0.3, 0.45], [0.6, 0.45, 0.35], [0.85, 0.8, 0.7]];
            let moved = probe.iter().any(|&c| {
                let o = (look.map)(c);
                (0..3).any(|i| (o[i] - c[i]).abs() > 1e-3)
            });
            assert!(moved, "{} leaves the image unchanged", look.file);
        }
    }

    #[test]
    fn a_mono_look_emits_only_greys() {
        for look in LOOKS.iter().filter(|l| l.family == Family::Mono) {
            for c in [[0.9, 0.1, 0.2], [0.2, 0.7, 0.3], [0.1, 0.2, 0.9]] {
                let o = (look.map)(c);
                assert!(
                    (o[0] - o[1]).abs() < 1e-5 && (o[1] - o[2]).abs() < 1e-5,
                    "{} left colour in: {o:?}",
                    look.file
                );
            }
        }
    }

    #[test]
    fn a_colour_look_keeps_neutrals_close_to_neutral() {
        // A grey ramp must not pick up a wild cast. Split-toning is allowed to
        // move it a little — that is what split-toning is — but a look that
        // turns mid-grey visibly green is broken.
        for look in LOOKS.iter().filter(|l| l.family != Family::Mono) {
            for g in [0.25_f32, 0.5, 0.75] {
                let o = (look.map)([g, g, g]);
                let spread = o.iter().cloned().fold(f32::MIN, f32::max)
                    - o.iter().cloned().fold(f32::MAX, f32::min);
                assert!(
                    spread < 0.12,
                    "{} casts neutral {g} by {spread}: {o:?}",
                    look.file
                );
            }
        }
    }

    #[test]
    fn tone_looks_stay_monotonic_on_a_grey_ramp() {
        // A look that reverses tone somewhere in the middle produces posterised
        // banding that is very hard to attribute back to the look.
        for look in LOOKS {
            let mut prev = -1.0f32;
            for i in 0..=64 {
                let g = i as f32 / 64.0;
                let y = luma((look.map)([g, g, g]));
                assert!(y >= prev - 1e-4, "{} dips at {g}", look.file);
                prev = y;
            }
        }
    }

    #[test]
    fn describe_names_our_own_files_and_nothing_else() {
        assert_eq!(
            describe("cinescope.cube").map(|l| l.title),
            Some("Cinescope".to_string())
        );
        assert!(describe("somebody-elses.cube").is_none());
        assert_eq!(catalogue().len(), LOOKS.len());
    }
}
