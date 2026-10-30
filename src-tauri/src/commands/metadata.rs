//! Metadata: keywords, capture times and IPTC fields.

use tauri::State;

use super::common::{beyond_undo, count, err};
use crate::{db, models, AppState};

#[tauri::command]
#[specta::specta]
pub(crate) fn add_tag(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    name: String,
    category: String,
) -> Result<(), String> {
    let mut conn = state.conn()?;
    let done = db::add_manual_tag_many(&mut conn, &photo_ids, &name, &category).map_err(err);
    beyond_undo(&state, "Keyword", photo_ids.len(), done)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn remove_tag(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    name: String,
) -> Result<(), String> {
    let conn = state.conn()?;
    for id in &photo_ids {
        db::remove_tag(&conn, *id, &name).map_err(err)?;
    }
    beyond_undo(&state, "Keyword removed", photo_ids.len(), Ok(()))
}

/// Puts one photo's place, and position where they have none, on others.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn paste_location(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    from: i32,
) -> Result<models::PastedLocation, String> {
    let mut conn = state.conn()?;
    let done = db::paste_location(&mut conn, &photo_ids, from).map_err(err);
    beyond_undo(&state, "Pasted location", photo_ids.len(), done)
}

/// Puts one photo's own keywords on others; how many were copied.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn paste_keywords(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    from: i32,
) -> Result<u32, String> {
    let mut conn = state.conn()?;
    let done = db::paste_keywords(&mut conn, &photo_ids, from)
        .map(|n| n as u32)
        .map_err(err);
    beyond_undo(&state, "Pasted keywords", photo_ids.len(), done)
}

/// Changes the selection's capture times: shift, set the first, or set
/// all. Virtual copies move with their originals.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn set_capture_time(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    change: models::CaptureTimeChange,
) -> Result<models::CaptureTimeOutcome, String> {
    let mut conn = state.conn()?;
    let done = db::set_capture_time(&mut conn, &photo_ids, &change).map_err(err);
    beyond_undo(&state, "Capture time", photo_ids.len(), done)
}

/// One photo's own title, caption and rights.
#[tauri::command]
#[specta::specta]
pub(crate) fn photo_iptc(
    state: State<'_, AppState>,
    photo_id: i32,
) -> Result<models::IptcFields, String> {
    db::photo_iptc(&state.conn()?, photo_id).map_err(err)
}

/// The fields across a selection, for editing them together.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn iptc_selection(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<models::IptcSelection, String> {
    db::iptc_selection(&state.conn()?, &photo_ids).map_err(err)
}

/// Sets one field on every photo given; a blank clears it.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_iptc_field(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    field: models::IptcField,
    value: Option<String>,
) -> Result<u32, String> {
    let mut conn = state.conn()?;
    let done = db::set_iptc_field(&mut conn, &photo_ids, field, value.as_deref())
        .map(count)
        .map_err(err);
    beyond_undo(&state, "Title, caption or rights", photo_ids.len(), done)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn list_iptc_templates(
    state: State<'_, AppState>,
) -> Result<Vec<models::IptcTemplate>, String> {
    db::list_iptc_templates(&state.conn()?).map_err(err)
}

/// Saves a template of these fields; a new one without an id.
#[tauri::command]
#[specta::specta]
pub(crate) fn save_iptc_template(
    state: State<'_, AppState>,
    id: Option<i32>,
    name: String,
    fields: models::IptcFields,
) -> Result<i32, String> {
    db::save_iptc_template(&state.conn()?, id, &name, &fields).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn delete_iptc_template(state: State<'_, AppState>, id: i32) -> Result<(), String> {
    db::delete_iptc_template(&state.conn()?, id).map_err(err)
}

/// Puts a template on these photos, or on an import's shoot once it
/// has been scanned; how many photos.
#[tauri::command]
#[specta::specta]
pub(crate) fn apply_iptc_template(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    shoot_id: Option<i32>,
    template_id: i32,
) -> Result<u32, String> {
    let mut conn = state.conn()?;
    let ids = match shoot_id {
        Some(shoot) => db::shoot_photo_ids(&conn, shoot).map_err(err)?,
        None => photo_ids,
    };
    let done = db::apply_iptc_template(&mut conn, &ids, template_id)
        .map(count)
        .map_err(err);
    beyond_undo(&state, "Metadata template", ids.len(), done)
}

/// Takes the capture times reading sidecars offered; each can be put
/// back like any other correction. Off the main thread: thousands of
/// them froze the window.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn use_sidecar_times(
    state: State<'_, AppState>,
    times: Vec<models::SidecarTime>,
) -> Result<models::CaptureTimeOutcome, String> {
    let mut conn = state.conn()?;
    db::use_sidecar_times(&mut conn, &times).map_err(err)
}

/// Puts capture times back to the files' own; how many photos changed.
#[tauri::command]
#[specta::specta]
pub(crate) fn revert_capture_time(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<u32, String> {
    let conn = state.conn()?;
    db::revert_capture_time(&conn, &photo_ids)
        .map(|n| n as u32)
        .map_err(err)
}

/// Turns folder names into keywords for one library folder's photos,
/// one shoot's, or — both `None` — every photo.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn add_folder_keywords(
    state: State<'_, AppState>,
    folder_id: Option<i32>,
    shoot_id: Option<i32>,
) -> Result<models::FolderKeywords, String> {
    let mut conn = state.conn()?;
    db::add_folder_keywords(&mut conn, folder_id, shoot_id).map_err(err)
}

/// Takes keywords made from folder names back off; how many photos
/// lost one.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn remove_folder_keywords(
    state: State<'_, AppState>,
    folder_id: Option<i32>,
) -> Result<u32, String> {
    let mut conn = state.conn()?;
    db::remove_folder_keywords(&mut conn, folder_id)
        .map(|n| n as u32)
        .map_err(err)
}

/// The tagger's unconfirmed keywords on these photos.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn ai_keywords(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<Vec<models::AiKeywords>, String> {
    let conn = state.conn()?;
    db::ai_keywords(&conn, &photo_ids).map_err(err)
}

/// Makes the tagger's keywords on these photos the user's own; `names`
/// empty confirms every one. Returns how many were confirmed.
#[tauri::command]
#[specta::specta]
pub(crate) fn confirm_keywords(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    names: Vec<String>,
) -> Result<u32, String> {
    let conn = state.conn()?;
    db::confirm_keywords(&conn, &photo_ids, &names)
        .map(|n| n as u32)
        .map_err(err)
}

/// The keyword tree's shape: every keyword it shows, and what each is
/// filed under. Counts come with the facets.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn keyword_tree(state: State<'_, AppState>) -> Result<Vec<models::KeywordNode>, String> {
    let conn = state.conn()?;
    db::keyword_tree(&conn).map_err(err)
}

/// A new keyword, under `parent` or at the top, for photos to have later.
#[tauri::command]
#[specta::specta]
pub(crate) fn create_keyword(
    state: State<'_, AppState>,
    name: String,
    parent: Option<String>,
) -> Result<(), String> {
    let conn = state.conn()?;
    db::create_keyword(&conn, &name, parent.as_deref()).map_err(err)
}

/// Files a keyword, and its branch, under `parent` — or at the top.
#[tauri::command]
#[specta::specta]
pub(crate) fn move_keyword(
    state: State<'_, AppState>,
    name: String,
    parent: Option<String>,
) -> Result<(), String> {
    let conn = state.conn()?;
    db::move_keyword(&conn, &name, parent.as_deref()).map_err(err)
}

/// Renames a keyword; to another keyword's name, merges the two. Returns the
/// name it ends up with.
#[tauri::command]
#[specta::specta]
pub(crate) fn rename_keyword(
    state: State<'_, AppState>,
    name: String,
    new_name: String,
) -> Result<String, String> {
    let mut conn = state.conn()?;
    db::rename_keyword(&mut conn, &name, &new_name).map_err(err)
}

/// Deletes a keyword from every photo; its branch moves up a level. Returns
/// how many photos had it.
#[tauri::command]
#[specta::specta]
pub(crate) fn delete_keyword(state: State<'_, AppState>, name: String) -> Result<u32, String> {
    let mut conn = state.conn()?;
    db::delete_keyword(&mut conn, &name)
        .map(|n| n as u32)
        .map_err(err)
}
