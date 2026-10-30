//! Suggested edits — picking which looks to offer for a given photo.
//!
//! The Presets panel opens on a short row of **live previews** for this photo:
//! the auto baseline plus a few built-in presets, each rendered onto *this*
//! photo. This module decides *which* presets to put forward, from the photo's
//! scene genre (the same vocabulary the library tagger assigns), and the
//! rendering itself lives in `session.rs`.
//!
//! The ordering is a hand-curated affinity table — landscapes lean vivid and
//! golden, portraits lean to skin — kept pure so it is a unit test rather than
//! a guess buried in the render path.

/// One suggested edit, ready for the UI: an identity plus a rendered preview.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// `"auto"` or `"preset"` — how the frontend applies it when clicked.
    pub kind: String,
    /// `"auto"`, or the built-in preset's id.
    pub id: String,
    /// Display label.
    pub name: String,
    /// A small JPEG preview of this look applied to the current photo, as a
    /// `data:image/jpeg;base64,…` URL.
    pub preview: String,
}

/// A rendered thumbnail for something the user can apply, keyed by its list id:
/// a built-in preset's id, a saved preset's numeric id as a string, or a LUT's
/// file name. Just the identity and the preview — the name and how to apply it
/// are already in the list the panel was built from.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LookPreview {
    /// Built-in preset id, saved preset id as a string, or LUT file name.
    pub id: String,
    /// A small JPEG preview of this look on the current photo, as a
    /// `data:image/jpeg;base64,…` URL.
    pub preview: String,
}

/// Preferred presets per scene genre, best first. Genres are the tagger's
/// vocabulary (`models::GENRES`); any not listed here falls back to [`DEFAULT`].
const AFFINITY: &[(&str, &[&str])] = &[
    (
        "landscape",
        &[
            "landscape-vivid",
            "landscape-golden-hour",
            "landscape-deep-sky",
            "landscape-alpine",
        ],
    ),
    (
        "wildlife",
        &[
            "natural-crisp",
            "landscape-lush",
            "natural-rich",
            "bold-punch",
        ],
    ),
    (
        "portrait",
        &[
            "portrait-natural",
            "portrait-warm",
            "portrait-soft-light",
            "portrait-matte",
            "mono-orange-filter",
        ],
    ),
    (
        "street",
        &[
            "city-street",
            "mono-high-contrast",
            "cinematic-teal-orange",
            "film-everyday",
        ],
    ),
    ("macro", &["natural-crisp", "bold-vivid", "landscape-lush"]),
    ("astro", &["natural-cool", "night-city", "cinematic-arctic"]),
    (
        "architecture",
        &[
            "city-cool",
            "mono-high-contrast",
            "city-concrete",
            "natural-crisp",
        ],
    ),
    ("sports", &["bold-punch", "natural-crisp", "bold-pop"]),
    (
        "event",
        &[
            "natural-warm",
            "film-warm",
            "night-tungsten",
            "portrait-natural",
        ],
    ),
    (
        "still-life",
        &["food-fresh", "food-warm-table", "food-dark", "food-bright"],
    ),
    (
        "abstract",
        &["bold-electric", "cinematic-future", "bold-pop"],
    ),
];

/// The order used when the genre is unknown or unlisted — a spread across the
/// pack.
const DEFAULT: &[&str] = &[
    "natural-clean",
    "bold-punch",
    "film-everyday",
    "cinematic-teal-orange",
    "natural-warm",
    "moody-moody",
    "mono-neutral",
];

/// The preset ids to suggest for a photo, best first, at most `max`. The
/// genre's preferred presets come first, then the general spread (so the row
/// is always full), de-duplicated.
pub fn suggested_preset_ids(genre: Option<&str>, max: usize) -> Vec<&'static str> {
    let preferred = genre
        .and_then(|g| AFFINITY.iter().find(|(k, _)| *k == g))
        .map(|(_, ids)| *ids)
        .unwrap_or(DEFAULT);

    let mut out: Vec<&'static str> = Vec::new();
    for id in preferred.iter().chain(DEFAULT.iter()) {
        if !out.contains(id) {
            out.push(id);
        }
        if out.len() == max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_genre_leads_with_its_affinities() {
        let ids = suggested_preset_ids(Some("portrait"), 4);
        assert_eq!(
            ids,
            vec![
                "portrait-natural",
                "portrait-warm",
                "portrait-soft-light",
                "portrait-matte"
            ]
        );
    }

    #[test]
    fn an_unknown_or_missing_genre_uses_the_default_order() {
        let expected = vec!["natural-clean", "bold-punch", "film-everyday"];
        assert_eq!(suggested_preset_ids(Some("does-not-exist"), 3), expected);
        assert_eq!(suggested_preset_ids(None, 3), expected);
    }

    #[test]
    fn the_row_is_padded_and_deduped_to_fill_the_count() {
        // Astro prefers only three; the general spread fills the row without
        // repeating any of the first three.
        let ids = suggested_preset_ids(Some("astro"), 6);
        assert_eq!(ids.len(), 6);
        assert_eq!(
            &ids[0..3],
            &["natural-cool", "night-city", "cinematic-arctic"]
        );
        let mut seen = std::collections::HashSet::new();
        assert!(ids.iter().all(|id| seen.insert(*id)), "no duplicates");
    }

    #[test]
    fn every_suggested_id_is_a_real_preset() {
        let real: std::collections::HashSet<_> =
            crate::develop::presets::ids().into_iter().collect();
        for g in crate::models::GENRES.iter().map(|g| Some(*g)).chain([None]) {
            for id in suggested_preset_ids(g, 7) {
                assert!(real.contains(id), "suggested id '{id}' is not a preset");
            }
        }
    }
}
