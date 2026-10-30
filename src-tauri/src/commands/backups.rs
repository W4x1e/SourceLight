//! Catalogue backups, and restoring one.

use std::path::Path;
use tauri::{AppHandle, State};

use super::common::err;
use crate::{backup, sync, AppState};

/// Takes a snapshot of the catalogue now, at the user's request.
///
/// Returns the path so the UI can name it. Manual backups are never pruned —
/// someone who took one before a risky bulk edit should find it there a month
/// later.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn backup_catalogue(state: State<'_, AppState>) -> Result<String, String> {
    let conn = state.conn()?;
    backup::create(&conn, &state.db_path, "manual")
        .map(|p| p.to_string_lossy().to_string())
        .map_err(err)
}

/// Takes the day's catalogue backup if one is due (see `backup::daily_if_due`).
/// The interface asks once per launch, unless daily backups are switched off
/// in Settings → Library. Returns the new backup's path, or `None`.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn backup_catalogue_if_due(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let conn = state.conn()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    backup::daily_if_due(&conn, &state.db_path, now)
        .map(|p| p.map(|p| p.to_string_lossy().to_string()))
        .map_err(err)
}

/// Every backup on disk, newest first.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn list_catalogue_backups(
    state: State<'_, AppState>,
) -> Result<Vec<backup::BackupInfo>, String> {
    backup::list(&state.db_path).map_err(err)
}

/// Queues a backup to replace the live catalogue at the next launch.
///
/// What startup did about a damaged catalogue, if anything — once: the
/// interface tells the user, and a second call returns nothing.
#[tauri::command]
#[specta::specta]
pub(crate) fn catalogue_recovery(state: State<'_, AppState>) -> Option<backup::Recovery> {
    sync::lock_recover(&state.recovery).take()
}

/// Stages the newest backup that passes an integrity check, for the restart
/// that follows. For the damage the background check finds after startup.
#[tauri::command]
#[specta::specta]
pub(crate) fn restore_newest_backup(
    state: State<'_, AppState>,
) -> Result<backup::BackupInfo, String> {
    let good = backup::newest_good(&state.db_path)
        .ok_or_else(|| "there is no backup that passes its integrity check".to_string())?;
    backup::stage_restore(Path::new(&good.path), &state.db_path).map_err(err)?;
    Ok(good)
}

/// Restarts the app, for a staged restore to take effect.
#[tauri::command]
#[specta::specta]
pub(crate) fn restart_app(app: AppHandle) {
    app.restart();
}

/// Staged rather than applied, because the app is holding the database open
/// right now — see `backup::stage_restore`. The caller is expected to tell the
/// user to restart; nothing changes until they do, which also means they can
/// change their mind by calling `cancel_catalogue_restore`. The backup is
/// named by its file name in the backups folder, never by a path.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn restore_catalogue_backup(
    state: State<'_, AppState>,
    filename: String,
) -> Result<(), String> {
    let backup = backup::named(&state.db_path, &filename).map_err(err)?;
    backup::stage_restore(&backup, &state.db_path).map_err(err)
}

/// Discards a staged restore, if one is waiting.
#[tauri::command]
#[specta::specta]
pub(crate) fn cancel_catalogue_restore(state: State<'_, AppState>) -> Result<bool, String> {
    backup::cancel_pending(&state.db_path).map_err(err)
}

/// Whether a restore is staged for the next launch, so the UI can say so.
#[tauri::command]
#[specta::specta]
pub(crate) fn catalogue_restore_pending(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(backup::pending_for(&state.db_path).is_some())
}
