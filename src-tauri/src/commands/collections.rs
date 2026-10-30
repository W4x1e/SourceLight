//! Collections, smart collections and the groups they are filed in.

use tauri::State;

use super::common::{beyond_undo, err};
use crate::models::{Collection, PhotoFilter, SmartCollection};
use crate::{db, models, AppState};

/// Collection groups.
#[tauri::command]
#[specta::specta]
pub(crate) fn list_collection_groups(
    state: State<'_, AppState>,
) -> Result<Vec<models::CollectionGroup>, String> {
    db::list_collection_groups(&state.conn()?).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn create_collection_group(
    state: State<'_, AppState>,
    name: String,
) -> Result<i32, String> {
    db::create_collection_group(&state.conn()?, &name).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn rename_collection_group(
    state: State<'_, AppState>,
    id: i32,
    name: String,
) -> Result<(), String> {
    db::rename_collection_group(&state.conn()?, id, &name).map_err(err)
}

/// Deletes a group; what was in it moves up a level.
#[tauri::command]
#[specta::specta]
pub(crate) fn delete_collection_group(state: State<'_, AppState>, id: i32) -> Result<(), String> {
    db::delete_collection_group(&state.conn()?, id).map_err(err)
}

/// Files a collection (or, with `smart`, a saved view) in a group, or takes it out.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_collection_group(
    state: State<'_, AppState>,
    id: i32,
    smart: bool,
    group_id: Option<i32>,
) -> Result<(), String> {
    db::set_collection_group(&state.conn()?, id, smart, group_id).map_err(err)
}

/// Moves photos within a collection's own order, before another or to the end.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn move_in_collection(
    state: State<'_, AppState>,
    collection_id: i32,
    photo_ids: Vec<i32>,
    before: Option<i32>,
) -> Result<(), String> {
    let mut conn = state.conn()?;
    db::move_in_collection(&mut conn, collection_id, &photo_ids, before).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn list_collections(state: State<'_, AppState>) -> Result<Vec<Collection>, String> {
    let conn = state.conn()?;
    db::list_collections(&conn).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn create_collection(state: State<'_, AppState>, name: String) -> Result<i32, String> {
    let conn = state.conn()?;
    db::create_collection(&conn, &name).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn rename_collection(
    state: State<'_, AppState>,
    id: i32,
    name: String,
) -> Result<(), String> {
    let conn = state.conn()?;
    db::rename_collection(&conn, id, &name).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn delete_collection(state: State<'_, AppState>, id: i32) -> Result<(), String> {
    let conn = state.conn()?;
    db::delete_collection(&conn, id).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn add_to_collection(
    state: State<'_, AppState>,
    id: i32,
    photo_ids: Vec<i32>,
) -> Result<(), String> {
    let mut conn = state.conn()?;
    let done = db::add_to_collection(&mut conn, id, &photo_ids).map_err(err);
    beyond_undo(&state, "Added to a collection", photo_ids.len(), done)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn remove_from_collection(
    state: State<'_, AppState>,
    id: i32,
    photo_ids: Vec<i32>,
) -> Result<(), String> {
    let mut conn = state.conn()?;
    let done = db::remove_from_collection(&mut conn, id, &photo_ids).map_err(err);
    beyond_undo(&state, "Removed from a collection", photo_ids.len(), done)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn list_smart_collections(
    state: State<'_, AppState>,
) -> Result<Vec<SmartCollection>, String> {
    let conn = state.conn()?;
    db::list_smart_collections(&conn).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn save_smart_collection(
    state: State<'_, AppState>,
    name: String,
    filter: PhotoFilter,
) -> Result<i32, String> {
    let conn = state.conn()?;
    db::save_smart_collection(&conn, &name, &filter).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn delete_smart_collection(state: State<'_, AppState>, id: i32) -> Result<(), String> {
    let conn = state.conn()?;
    db::delete_smart_collection(&conn, id).map_err(err)
}
