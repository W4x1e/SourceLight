//! Presets and LUTs, and their previews.

use tauri::State;

use super::common::{count, ensure_luts_dir, err, luts_dir, persist_develop, with_develop};
use crate::{db, develop, fsutil, AppState};

/// One installed LUT, as the picker lists it.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LutInfo {
    /// The `.cube` filename, which is also what an `EditStack`'s `lut.name`
    /// references.
    name: String,
    /// How the Looks browser titles it: the built-in's own name, or a tidied
    /// filename for one the user imported.
    title: String,
    /// The group it is listed under — a user-assigned name when there is one,
    /// otherwise the built-in family, or "Imported" for a `.cube` we know
    /// nothing about.
    family: String,
    /// One line describing the look, empty for an imported LUT.
    blurb: String,
    /// True when the user has filed this LUT under a group of their own, so
    /// the browser can offer "Remove from group" rather than only "Move to".
    grouped: bool,
    /// True when this is not one of ours — an imported `.cube`.
    imported: bool,
}

/// Where the user's own LUT groupings live.
///
/// Importing a dozen `.cube` files from three different packs dropped all
/// twelve into one flat "Imported" heap, which is where a look you paid for
/// goes to be lost. Groups are a filename → group-name map rather than
/// subfolders on disk, because an `EditStack` references a LUT by bare
/// filename (`develop::lut`): moving the file would break every edit using it,
/// where renaming a group is free.
const LUT_GROUPS_KEY: &str = "lut_groups";

/// The user's filename → group-name map, empty if unset or unreadable.
fn lut_groups(conn: &rusqlite::Connection) -> std::collections::HashMap<String, String> {
    db::get_setting(conn, LUT_GROUPS_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Presents a `.cube` filename as the Looks browser lists it, using the pack's
/// own catalogue for a built-in and a tidied filename for anything else. A
/// user-assigned group wins over both.
fn describe_lut(name: &str, groups: &std::collections::HashMap<String, String>) -> LutInfo {
    let assigned = groups.get(name).filter(|g| !g.trim().is_empty()).cloned();
    match develop::lut_packs::describe(name) {
        Some(look) => LutInfo {
            name: name.to_string(),
            title: look.title,
            family: assigned.clone().unwrap_or(look.family),
            blurb: look.blurb,
            grouped: assigned.is_some(),
            imported: false,
        },
        None => LutInfo {
            name: name.to_string(),
            title: name
                .trim_end_matches(".cube")
                .trim_end_matches(".CUBE")
                .into(),
            family: assigned.clone().unwrap_or_else(|| "Imported".into()),
            blurb: String::new(),
            grouped: assigned.is_some(),
            imported: true,
        },
    }
}

/// Files a LUT under a group of the user's naming, or clears it back to its
/// default heading when `group` is empty. Takes a list because "move these
/// four into Portra" is one action, not four.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_lut_group(
    state: State<'_, AppState>,
    names: Vec<String>,
    group: Option<String>,
) -> Result<Vec<LutInfo>, String> {
    let conn = state.conn()?;
    let mut groups = lut_groups(&conn);
    let group = group
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    for name in &names {
        match &group {
            Some(g) => groups.insert(name.clone(), g.clone()),
            None => groups.remove(name),
        };
    }
    let json = serde_json::to_string(&groups).map_err(err)?;
    db::set_setting(&conn, LUT_GROUPS_KEY, &json).map_err(err)?;
    let installed = installed_lut_names(&ensure_luts_dir(&state));
    Ok(installed.iter().map(|n| describe_lut(n, &groups)).collect())
}

/// Imports a `.cube` file: validates it parses, then copies it into the LUTs
/// folder under its base filename, numbered if another LUT has that name.
/// Returns the stored entry.
#[tauri::command]
#[specta::specta]
pub(crate) fn import_lut(
    state: State<'_, AppState>,
    path: String,
    group: Option<String>,
) -> Result<LutInfo, String> {
    let src = std::path::Path::new(&path);
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("that path has no filename")?
        .to_string();
    if !name.to_ascii_lowercase().ends_with(".cube") {
        return Err("only .cube LUTs are supported".into());
    }
    let text = std::fs::read_to_string(src).map_err(err)?;
    // Validate before storing, so a broken file never lands in the library.
    develop::lut::CubeLut::parse(&text).map_err(err)?;

    let dir = ensure_luts_dir(&state);
    std::fs::create_dir_all(&dir).map_err(err)?;
    // Never over a LUT already there: edits name their LUT, so replacing one
    // changed every photo that used it, while Develop kept showing the old
    // one from its cache. The same file again is that LUT; another of the
    // same name is kept beside it as `Name (2).cube`.
    let (stem, ext) = name.split_at(name.len() - ".cube".len());
    let (stem, ext) = (stem.to_string(), ext.to_string());
    let mut n = 1;
    let name = loop {
        let candidate = if n == 1 {
            name.clone()
        } else {
            format!("{stem} ({n}){ext}")
        };
        let target = dir.join(&candidate);
        match fsutil::write_new(&target, text.as_bytes()) {
            Ok(()) => break candidate,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read_to_string(&target).is_ok_and(|there| there == text) {
                    break candidate;
                }
                n += 1;
            }
            Err(e) => return Err(err(e)),
        }
    };

    // File it as it lands. Asking where a LUT belongs at import time is the
    // only moment the user still remembers which pack it came out of.
    let conn = state.conn()?;
    let mut groups = lut_groups(&conn);
    if let Some(g) = group
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
    {
        groups.insert(name.clone(), g);
        let json = serde_json::to_string(&groups).map_err(err)?;
        db::set_setting(&conn, LUT_GROUPS_KEY, &json).map_err(err)?;
    }
    Ok(describe_lut(&name, &groups))
}

/// The installed LUT filenames (every `.cube` in the folder), name-sorted.
fn installed_lut_names(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if name.to_ascii_lowercase().ends_with(".cube") {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// Lists the installed LUTs (every `.cube` in the LUTs folder), name-sorted.
#[tauri::command]
#[specta::specta]
pub(crate) fn list_luts(state: State<'_, AppState>) -> Result<Vec<LutInfo>, String> {
    let names = installed_lut_names(&ensure_luts_dir(&state));
    let groups = lut_groups(&state.conn()?);
    Ok(names.iter().map(|n| describe_lut(n, &groups)).collect())
}

/// Live thumbnails of every installed LUT applied to the open photo, keyed by the
/// LUT's filename — so the LUTs gallery shows the look on *this* photo rather than
/// a blank tile, exactly as the Styles and Presets galleries do.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn lut_previews(
    state: State<'_, AppState>,
) -> Result<Vec<develop::suggest::LookPreview>, String> {
    let names = installed_lut_names(&ensure_luts_dir(&state));
    with_develop(&state, |dev| {
        Ok(dev
            .lut_previews(&names)
            .map_err(err)?
            .into_iter()
            .map(|(id, preview)| develop::suggest::LookPreview { id, preview })
            .collect())
    })
}

/// Deletes an installed LUT. The name must be a bare filename — no path parts —
/// so it can only ever remove a file from the LUTs folder.
/// How many photos' edits use a LUT, asked before it is deleted.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn lut_usage(state: State<'_, AppState>, name: String) -> Result<u32, String> {
    let conn = state.conn()?;
    db::lut_usage(&conn, &name).map(count).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn delete_lut(state: State<'_, AppState>, name: String) -> Result<(), String> {
    if !develop::lut::is_lut_name(&name) {
        return Err("invalid LUT name".into());
    }
    let path = luts_dir(&state).join(&name);
    if path.exists() {
        std::fs::remove_file(&path).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) fn list_presets(state: State<'_, AppState>) -> Result<Vec<db::Preset>, String> {
    let conn = state.conn()?;
    db::list_presets(&conn).map_err(err)
}

/// Saves the open photo's current settings as a named preset — only the
/// chosen parts of it, or all of it when `groups` is empty.
#[tauri::command]
#[specta::specta]
pub(crate) fn save_preset(
    state: State<'_, AppState>,
    name: String,
    groups: Vec<develop::stack::Group>,
) -> Result<i32, String> {
    let stack = with_develop(&state, |dev| dev.current_stack().map_err(err))?;
    let conn = state.conn()?;
    let groups = (!groups.is_empty()).then_some(groups.as_slice());
    db::save_preset(&conn, &name, &stack, groups).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn delete_preset(state: State<'_, AppState>, id: i32) -> Result<(), String> {
    let conn = state.conn()?;
    db::delete_preset(&conn, id).map_err(err)
}

/// Applies a preset to the open photo, as a single undoable step.
#[tauri::command]
#[specta::specta]
pub(crate) fn apply_preset(
    state: State<'_, AppState>,
    id: i32,
) -> Result<develop::DevelopInfo, String> {
    let conn = state.conn()?;
    let preset = db::list_presets(&conn)
        .map_err(err)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("preset {id} not found"))?;

    let info = with_develop(&state, |dev| {
        let applied = preset.applied_to(&dev.current_stack().map_err(err)?);
        dev.apply(&preset.name, applied).map_err(err)
    })?;
    persist_develop(&state, info)
}

/// The built-in presets, by group, in display order.
#[tauri::command]
#[specta::specta]
pub(crate) fn list_preset_groups() -> Vec<develop::presets::PresetGroupInfo> {
    develop::presets::catalogue()
}

/// `stack` with built-in preset `id` on it at `amount` (1 as designed), or with
/// its preset taken off when `id` is empty. Only the preset's own fields change
/// (see `develop::presets`); the caller applies the result like any edit, so a
/// preset and its amount slider go through the same history as everything else.
#[tauri::command]
#[specta::specta]
pub(crate) fn preset_stack(
    stack: develop::EditStack,
    id: String,
    amount: f32,
) -> Result<develop::EditStack, String> {
    if id.is_empty() {
        return Ok(develop::presets::remove(&stack));
    }
    develop::presets::apply(&id, amount, &stack).ok_or_else(|| format!("no preset called {id}"))
}

/// Suggested edits for the open photo: the auto baseline plus a few presets
/// chosen from the photo's scene genre, each with a live preview rendered onto
/// this photo. Clicking one (in the UI) applies it.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn suggest_edits(
    state: State<'_, AppState>,
) -> Result<Vec<develop::suggest::Suggestion>, String> {
    // The photo's stored genre (from library tagging) tailors which presets lead;
    // absent, the suggestions fall back to a sensible general order.
    let photo_id = with_develop(&state, |dev| {
        dev.open_photo_id()
            .ok_or_else(|| "no photo is open".to_string())
    })?;
    let genre = {
        let conn = state.conn()?;
        db::get_photo(&conn, photo_id)
            .map_err(err)?
            .and_then(|p| p.genre)
    };
    let ids = develop::suggest::suggested_preset_ids(genre.as_deref(), 5);
    with_develop(&state, |dev| dev.suggest_edits(&ids).map_err(err))
}

/// Live thumbnails of the given built-in presets on the open photo, so the
/// Presets panel shows each on *this* photo rather than a blank swatch. Each is
/// a GPU render, so the panel asks for a group's when it opens that group.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn builtin_preset_previews(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<develop::suggest::LookPreview>, String> {
    with_develop(&state, |dev| {
        let previews = dev.preset_previews(&ids).map_err(err)?;
        Ok(previews
            .into_iter()
            .map(|(id, preview)| develop::suggest::LookPreview { id, preview })
            .collect())
    })
}

/// Live thumbnails of every saved Preset applied to the open photo, keyed by the
/// preset id (as a string). The preset stacks are read from the library first,
/// then rendered on the open session — the same preview the Styles gallery uses.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn preset_previews(
    state: State<'_, AppState>,
) -> Result<Vec<develop::suggest::LookPreview>, String> {
    // Read the presets before taking the develop lock, so the two mutexes are
    // never held at once.
    let presets = {
        let conn = state.conn()?;
        db::list_presets(&conn).map_err(err)?
    };
    with_develop(&state, |dev| {
        let mut out = Vec::with_capacity(presets.len());
        for preset in presets {
            let applied = preset.applied_to(&dev.current_stack().map_err(err)?);
            let preview = dev.render_look_preview(&applied).map_err(err)?;
            out.push(develop::suggest::LookPreview {
                id: preset.id.to_string(),
                preview,
            });
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A user's own grouping has to win over the pack's family, or filing a
    /// built-in under "My wedding looks" would silently do nothing.
    #[test]
    fn a_user_group_overrides_the_built_in_family() {
        let mut groups = HashMap::new();
        groups.insert("portrait-negative.cube".to_string(), "Wedding".to_string());

        let info = describe_lut("portrait-negative.cube", &groups);
        assert_eq!(info.family, "Wedding");
        assert!(info.grouped);
        assert!(!info.imported);
        // The title and blurb still come from the catalogue — grouping moves a
        // look, it does not turn it into an unknown file.
        assert!(!info.title.is_empty());
        assert!(!info.blurb.is_empty());
    }

    #[test]
    fn an_ungrouped_built_in_keeps_its_family() {
        let info = describe_lut("portrait-negative.cube", &HashMap::new());
        assert_ne!(info.family, "Imported");
        assert!(!info.grouped);
        assert!(!info.imported);
    }

    /// Everything we do not recognise lands in one "Imported" heap unless the
    /// user says otherwise — that heap is the thing groups exist to break up.
    #[test]
    fn an_unknown_cube_is_imported_until_it_is_grouped() {
        let plain = describe_lut("SomeVendor_K400.cube", &HashMap::new());
        assert_eq!(plain.family, "Imported");
        assert_eq!(plain.title, "SomeVendor_K400");
        assert!(plain.imported);
        assert!(!plain.grouped);

        let mut groups = HashMap::new();
        groups.insert("SomeVendor_K400.cube".to_string(), "SomeVendor".to_string());
        let filed = describe_lut("SomeVendor_K400.cube", &groups);
        assert_eq!(filed.family, "SomeVendor");
        assert!(filed.grouped);
    }

    /// A blank group name is a cleared group, not a heading called "".
    #[test]
    fn a_blank_group_name_falls_back_to_the_default_heading() {
        let mut groups = HashMap::new();
        groups.insert("SomeVendor_K400.cube".to_string(), "   ".to_string());
        let info = describe_lut("SomeVendor_K400.cube", &groups);
        assert_eq!(info.family, "Imported");
        assert!(!info.grouped);
    }
}
