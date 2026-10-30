//! Importing from a card or a folder, and the shoots an import makes.

use std::path::Path;
use tauri::{AppHandle, Emitter, Manager, State};

use super::common::{count, err};
use crate::models::{ScanProgress, Shoot};
use crate::{db, hardware, ingest, AppState};

/// Camera cards plugged in now — volumes with a `DCIM` folder.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn import_sources() -> Vec<ingest::ImportSource> {
    ingest::sources()
}

/// The catalogue's quick hashes, to tell which files are already imported.
///
/// Only copies that are there and are not on `source` itself count: a photo
/// skipped as already imported is one the report calls safe to format, and a
/// row whose file has gone, or that is the card's own file (a card added to
/// the library in place), is no copy at all.
fn known_hashes(
    state: &AppState,
    source: &Path,
) -> Result<std::collections::HashSet<String>, String> {
    let conn = state.conn()?;
    let mut stmt = conn
        .prepare("SELECT file_hash, path FROM photos WHERE variant = 0 AND missing = 0")
        .map_err(err)?;
    let hashes = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?
        .filter_map(|r| r.ok())
        .filter(|(_, path)| !Path::new(path).starts_with(source))
        .map(|(hash, _)| hash)
        .collect();
    Ok(hashes)
}

/// What importing from `source` would copy, by day, and what is already in
/// the library.
#[tauri::command]
#[specta::specta]
pub(crate) async fn import_preview(
    state: State<'_, AppState>,
    source: String,
) -> Result<ingest::ImportPreview, String> {
    let known = known_hashes(&state, Path::new(&source))?;
    tauri::async_runtime::spawn_blocking(move || {
        let plan = ingest::plan(Path::new(&source)).map_err(err)?;
        Ok(ingest::preview(&plan, &|h| known.contains(h)))
    })
    .await
    .map_err(err)?
}

/// Copies a card into a library folder (and a backup, if asked), verified,
/// skipping what the library already has. Progress on `import-progress`,
/// cancellable as the `import` job. The destination is added to the library
/// if it is not already in it; the caller then scans the folders returned.
#[tauri::command]
#[specta::specta]
pub(crate) async fn import_run(
    app: AppHandle,
    state: State<'_, AppState>,
    options: ingest::ImportOptions,
) -> Result<ingest::ImportOutcome, String> {
    {
        let mut conn = state.conn()?;
        if db::folder_id_for(&conn, &options.destination)
            .map_err(err)?
            .is_none()
        {
            db::add_library_folder(&mut conn, &options.destination).map_err(err)?;
        }
    }
    let known = known_hashes(&state, Path::new(&options.source))?;
    let dest = options.destination.clone();
    let plan = ingest::plan(Path::new(&options.source)).map_err(err)?;
    let (bytes, backup_bytes) = {
        let only: Option<std::collections::HashSet<&str>> = options
            .only
            .as_ref()
            .map(|o| o.iter().map(String::as_str).collect());
        let wanted: Vec<&ingest::PlannedFile> = plan
            .files
            .iter()
            .filter(|f| {
                only.as_ref()
                    .is_none_or(|o| o.contains(f.src.to_string_lossy().as_ref()))
            })
            .collect();
        (
            ingest::bytes_of(
                wanted
                    .iter()
                    .copied()
                    .filter(|f| !ingest::already_have(f, &|h| known.contains(h))),
            ),
            // The backup gets the ones the library already has too.
            ingest::bytes_of(wanted.iter().copied()),
        )
    };
    // Refused up front, not discovered at 90% of a card — the backup
    // too, which used to fill and fail every later frame.
    hardware::ensure_space(Path::new(&dest), bytes, "the import").map_err(err)?;
    let backup = options.backup.clone().filter(|b| !b.trim().is_empty());
    if let Some(b) = &backup {
        if !Path::new(b).is_dir() {
            return Err(format!(
                "The backup folder {b} isn't there. Connect its drive, or choose another."
            ));
        }
        hardware::ensure_space(Path::new(b), backup_bytes, "the backup copy").map_err(err)?;
    }
    let cancel = state.jobs.start("import");
    let emitter = app.clone();
    let shoot_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let out = ingest::run(
            &options,
            &plan,
            &|h| known.contains(h),
            &cancel,
            |done, total| {
                let _ = emitter.emit(
                    "import-progress",
                    ScanProgress {
                        phase: "import".into(),
                        current: count(done),
                        total: count(total),
                        message: "Importing".into(),
                    },
                );
            },
        );
        let _ = emitter.emit(
            "import-progress",
            ScanProgress {
                phase: "done".into(),
                current: 0,
                total: 0,
                message: String::new(),
            },
        );
        // Remember the import as a shoot, under its job name, with
        // what did not make it: the shoot's line keeps saying so after
        // the dialog is closed. A retry adds to the shoot it is retrying.
        let mut out = out;
        if let Ok(o) = &mut out {
            let not_copied = (o.failed.len() + o.unreadable.len() + o.not_reached.len()) as u32;
            let state = shoot_app.state::<AppState>();
            let remembered = state
                .conn()
                .map_err(anyhow::Error::msg)
                .and_then(|mut conn| {
                    let id = match options.shoot_id {
                        Some(id) => {
                            db::add_shoot_files(&mut conn, id, &o.copied_paths)?;
                            let (_, earlier) = db::shoot_report(&conn, id)?;
                            let not_backed_up = earlier + o.not_backed_up.len() as u32;
                            db::set_shoot_report(&conn, id, None, not_copied, not_backed_up)?;
                            Some(id)
                        }
                        None if !o.copied_paths.is_empty() => {
                            let name = if options.job.trim().is_empty() {
                                format!("Card import, {}", chrono::Local::now().format("%-d %b %Y"))
                            } else {
                                options.job.trim().to_string()
                            };
                            let id = db::create_shoot(
                                &mut conn,
                                &name,
                                &options.source,
                                &options.destination,
                                &o.copied_paths,
                            )?;
                            let not_backed_up = o.not_backed_up.len() as u32;
                            db::set_shoot_report(
                                &conn,
                                id,
                                backup.as_deref(),
                                not_copied,
                                not_backed_up,
                            )?;
                            Some(id)
                        }
                        None => None,
                    };
                    anyhow::Ok(id)
                });
            match remembered {
                Ok(id) => o.shoot_id = id,
                Err(e) => eprintln!("the import was not remembered as a shoot: {e:#}"),
            }
        }
        out.map_err(err)
    })
    .await
    .map_err(err)?
}

/// Copies a shoot's photos to its backup folder where the import could not:
/// the drive filled up, or was unplugged. From the library copies, so
/// the card is no longer needed. Progress on `import-progress`, cancellable as
/// the `import` job.
#[tauri::command]
#[specta::specta]
pub(crate) async fn finish_shoot_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i32,
) -> Result<ingest::BackupOutcome, String> {
    let (destination, backup, paths) = {
        let conn = state.conn()?;
        db::shoot_backup_plan(&conn, id).map_err(err)?
    };
    let backup = backup.ok_or("This shoot was imported without a backup copy.")?;
    if !Path::new(&backup).is_dir() {
        return Err(format!(
            "The backup folder {backup} isn't there. Connect its drive and try again."
        ));
    }
    let bytes = paths
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();
    hardware::ensure_space(Path::new(&backup), bytes, "the backup copy").map_err(err)?;
    let cancel = state.jobs.start("import");
    let emitter = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let out = ingest::finish_backup(
            Path::new(&destination),
            Path::new(&backup),
            &paths,
            &cancel,
            |done, total| {
                let _ = emitter.emit(
                    "import-progress",
                    ScanProgress {
                        phase: "import".into(),
                        current: count(done),
                        total: count(total),
                        message: "Backing up".into(),
                    },
                );
            },
        );
        let _ = emitter.emit(
            "import-progress",
            ScanProgress {
                phase: "done".into(),
                current: 0,
                total: 0,
                message: String::new(),
            },
        );
        let state = app.state::<AppState>();
        let conn = state.conn()?;
        let (not_copied, _) = db::shoot_report(&conn, id).map_err(err)?;
        let missing = paths.len() as u32 - out.backed_up;
        db::set_shoot_report(&conn, id, None, not_copied, missing).map_err(err)?;
        Ok(out)
    })
    .await
    .map_err(err)?
}

/// The most recent card imports, with how far along each is.
#[tauri::command]
#[specta::specta]
pub(crate) fn list_shoots(state: State<'_, AppState>) -> Result<Vec<Shoot>, String> {
    let conn = state.conn()?;
    db::list_shoots(&conn, 12).map_err(err)
}

/// Forgets a shoot; its photos stay in the library.
#[tauri::command]
#[specta::specta]
pub(crate) fn delete_shoot(state: State<'_, AppState>, id: i32) -> Result<(), String> {
    let conn = state.conn()?;
    db::delete_shoot(&conn, id).map_err(err)
}
