//! The library's folders: adding, removing and relocating them, and scanning
//! them for photos.

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

use super::common::{count, err, photos_removed};
use crate::models::{ScanProgress, ScanResult};
use crate::{backup, db, ingest, models, scan, AppState};

/// The library's folders with their counts, and whether each is reachable.
/// The library used to be a single `library_root` setting.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn list_library_folders(
    state: State<'_, AppState>,
) -> Result<models::LibraryFolders, String> {
    let conn = state.conn()?;
    let mut out = db::list_library_folders(&conn).map_err(err)?;
    for f in &mut out.folders {
        f.online = Path::new(&f.path).is_dir();
    }
    Ok(out)
}

/// Adds a folder to the library, returning its id. The caller scans it.
///
/// Only a folder: a path can now arrive by drag and drop as well as from the
/// folder picker, and a dropped photo is a file.
#[tauri::command]
#[specta::specta]
pub(crate) fn add_library_folder(state: State<'_, AppState>, path: String) -> Result<i32, String> {
    if !std::path::Path::new(&path).is_dir() {
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or(path.clone(), |n| n.to_string_lossy().into_owned());
        return Err(format!(
            "{name} isn't a folder — drop the folder your photos are in"
        ));
    }
    let mut conn = state.conn()?;
    db::add_library_folder(&mut conn, &path).map_err(err)
}

/// Whether a folder is on a camera card, so adding it can offer to
/// import instead: photos catalogued in place there vanish when the card is
/// formatted.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn folder_is_card(path: String) -> bool {
    ingest::is_on_card(Path::new(&path))
}

/// Takes a folder out of the library and forgets its photos — never touching
/// the files — after backing the catalogue up, since the work on them goes
/// too. Returns how many photos were forgotten.
///
/// Off the main thread: the backup is a full copy of the catalogue,
/// and the window went Not Responding while it and the delete ran.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn remove_library_folder(state: State<'_, AppState>, id: i32) -> Result<u32, String> {
    let mut conn = state.conn()?;
    backup::create(&conn, &state.db_path, "auto-before-folder-removal").map_err(|e| {
        format!("couldn't back up the catalogue first, so nothing was removed: {e:#}")
    })?;
    let _ = backup::prune(&state.db_path);
    let n = db::remove_library_folder(&mut conn, id).map_err(err)?;
    state.bump_search_gen();
    photos_removed(&state, "Removing a folder");
    Ok(n as u32)
}

/// Forgets photos that belong to no library folder, after a backup. Off the
/// main thread, as [`remove_library_folder`] is.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn forget_unfiled_photos(state: State<'_, AppState>) -> Result<u32, String> {
    let mut conn = state.conn()?;
    backup::create(&conn, &state.db_path, "auto-before-forget").map_err(|e| {
        format!("couldn't back up the catalogue first, so nothing was removed: {e:#}")
    })?;
    let _ = backup::prune(&state.db_path);
    let n = db::forget_unfiled_photos(&mut conn).map_err(err)?;
    state.bump_search_gen();
    photos_removed(&state, "Forgetting photos");
    Ok(n as u32)
}

/// "Locate folder…": points a folder, and every photo in it, at where it is
/// now. The caller rescans it.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn relocate_library_folder(
    state: State<'_, AppState>,
    id: i32,
    path: String,
) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("{path} isn't a folder"));
    }
    let mut conn = state.conn()?;
    // Every photo of the folder changes file: a copy first, as removing one
    // takes.
    backup::create(&conn, &state.db_path, "auto-before-relocate").map_err(|e| {
        format!("couldn't back up the catalogue first, so nothing was changed: {e:#}")
    })?;
    let _ = backup::prune(&state.db_path);
    db::relocate_library_folder(&mut conn, id, &path).map_err(err)?;
    state.bump_search_gen();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn scan_library(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ScanResult, String> {
    let db_path = state.db_path.clone();
    let cache_dir = state.cache_dir.clone();

    // Every scan is of a library folder; one that is not yet is added, so an
    // older frontend or a direct call still files what it finds. Then the
    // folder is scanned as stored, not as the page spelled it: the
    // picker's `E:\` and the stored form must walk to the same paths, or
    // every photo is catalogued twice.
    let (folder_id, path) = {
        let mut conn = state.conn()?;
        db::folder_for_scan(&mut conn, &path).map_err(err)?
    };
    let root = PathBuf::from(&path);
    let root_str = path.clone();

    // Scanning is CPU- and I/O-bound; keep it off the async runtime's threads.
    let counts =
        tauri::async_runtime::spawn_blocking(move || -> Result<scan::ScanCounts, String> {
            let mut conn = db::connect(&db_path).map_err(err)?;
            let counts = scan::scan_library(&mut conn, &root, &cache_dir, |done, total, label| {
                let _ = app.emit(
                    "scan-progress",
                    ScanProgress {
                        phase: "scan".into(),
                        current: count(done),
                        total: count(total),
                        message: label.to_string(),
                    },
                );
            })
            .map_err(err)?;
            db::assign_folder(&conn, folder_id, &root_str).map_err(err)?;
            // New photos with a position get their town now.
            db::place_from_gps(&mut conn).map_err(err)?;
            // A first scan takes `photos` from empty to the whole library, which
            // leaves the planner's statistics describing a table that no longer
            // exists. Cheap when nothing changed; never worth failing a scan over.
            if counts.added > 0 {
                if let Err(e) = db::refresh_planner_stats(&conn) {
                    eprintln!("could not refresh query-planner statistics after scanning: {e:#}");
                }
            }
            Ok(counts)
        })
        .await
        .map_err(err)??;

    // The photo set (and, via the caption triggers, the search index) may have
    // changed, so a cached resolve could now be stale.
    state.bump_search_gen();

    Ok(ScanResult {
        added: count(counts.added),
        skipped: count(counts.skipped),
        failed: count(counts.failed),
        relinked: count(counts.relinked),
        missing: count(counts.missing),
        first_failure: counts.first_failure,
    })
}
