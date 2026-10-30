//! The phone link: pairing, the outbox and the import folder, and the
//! searches a phone asks for.

use tauri::{AppHandle, Manager, State};

#[cfg(feature = "phone-link")]
use super::browse::{fused_search_ids, semantic_query};
use super::common::err;
#[cfg(feature = "phone-link")]
use crate::db;
#[cfg(feature = "phone-link")]
use crate::models::PhotoFilter;
use crate::{phone_link, AppState};

/// Send to phone (docs/MOBILE.md): a new folder in the outbox for a batch
/// export to write into.
#[tauri::command]
#[specta::specta]
pub(crate) fn phone_outbox_start(state: State<'_, AppState>) -> Result<String, String> {
    let data_dir = state
        .db_path
        .parent()
        .unwrap_or(&state.db_path)
        .to_path_buf();
    phone_link::start_outbox_batch(&data_dir)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(err)
}

/// Puts what a Send to phone batch exported on the list phones fetch from.
/// Returns how many files.
#[tauri::command]
#[specta::specta]
pub(crate) fn phone_outbox_queue(
    state: State<'_, AppState>,
    directory: String,
) -> Result<u32, String> {
    let data_dir = state
        .db_path
        .parent()
        .unwrap_or(&state.db_path)
        .to_path_buf();
    phone_link::queue_outbox_batch(&state.conn()?, &data_dir, std::path::Path::new(&directory))
        .map_err(err)
}

/// Where photos added from a phone go (Settings → Phone).
#[tauri::command]
#[specta::specta]
pub(crate) fn phone_import_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<phone_link::PhoneImportFolder, String> {
    let fallback = app
        .path()
        .picture_dir()
        .map(|p| p.join("SourceLight"))
        .map_err(|e| e.to_string())?;
    let conn = state.conn()?;
    Ok(phone_link::PhoneImportFolder {
        path: phone_link::import_folder(&conn, &fallback)
            .map_err(err)?
            .to_string_lossy()
            .into_owned(),
        chosen: phone_link::import_folder_chosen(&conn).map_err(err)?,
    })
}

/// Chooses where photos added from a phone go; `None` for the default.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_phone_import_folder(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<(), String> {
    phone_link::set_import_folder(&state.conn()?, path.as_deref()).map_err(err)
}

/// Settings → Phone (docs/MOBILE.md).
#[tauri::command]
#[specta::specta]
pub(crate) fn phone_link_status(
    state: State<'_, AppState>,
) -> Result<phone_link::PhoneLinkStatus, String> {
    phone_link::status(&state.phone, &state.conn()?).map_err(err)
}

/// Turns the phone link and away-from-home on or off, and starts or stops the
/// listener to match. A failure to start is reported in the status rather
/// than as an error, so the switches still show what was chosen.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_phone_link(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
    away_from_home: bool,
) -> Result<phone_link::PhoneLinkStatus, String> {
    phone_link::set_flags(&state.conn()?, enabled, away_from_home).map_err(err)?;
    let _ = phone_link::apply(&state.phone, &app, &state.db_path).await;
    phone_link::status(&state.phone, &state.conn()?).map_err(err)
}

/// Turns the phone link on if it is not, and makes a pairing code.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_phone_pairing(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<phone_link::PhonePairing, String> {
    phone_link::start_pairing(&state.phone, &app, &state.db_path).await
}

/// Retires the pairing code on screen.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cancel_phone_pairing(state: State<'_, AppState>) -> Result<(), String> {
    phone_link::cancel_pairing(&state.phone).await;
    Ok(())
}

/// The answer to a `phone-pair-request` event.
#[tauri::command]
#[specta::specta]
pub(crate) fn answer_phone_pairing(state: State<'_, AppState>, phone: String, allow: bool) {
    phone_link::answer_pairing(&state.phone, &phone, allow);
}

/// Forgets a paired phone; it is refused from its next request.
#[tauri::command]
#[specta::specta]
pub(crate) fn remove_phone(
    state: State<'_, AppState>,
    phone: String,
) -> Result<phone_link::PhoneLinkStatus, String> {
    let conn = state.conn()?;
    phone_link::remove_phone(&state.phone, &conn, &phone).map_err(err)?;
    phone_link::status(&state.phone, &conn).map_err(err)
}

/// The photos a phone's search finds (docs/MOBILE.md), as the search box
/// finds them with the same filter: best first, and `true`, when it asks what
/// is in the photos; otherwise every match, unordered, and `false`.
#[cfg(feature = "phone-link")]
pub(crate) fn phone_search(
    app: &AppHandle,
    state: &AppState,
    filter: &PhotoFilter,
) -> Result<(Vec<i64>, bool), String> {
    let conn = state.conn()?;
    if let Some(q) = semantic_query(filter) {
        let ranked = fused_search_ids(app, state, q, Some(filter))?;
        let photos =
            db::query_photos_ranked(&conn, filter, &ranked, PHONE_SEARCH_MOST, 0).map_err(err)?;
        return Ok((photos.into_iter().map(|p| i64::from(p.id)).collect(), true));
    }
    let ids = db::ids_matching(&conn, filter).map_err(err)?;
    Ok((ids.into_iter().map(i64::from).collect(), false))
}

/// The most a phone's search by content returns: past that, matches are
/// weak, as on the desktop.
#[cfg(feature = "phone-link")]
const PHONE_SEARCH_MOST: i32 = 2000;
