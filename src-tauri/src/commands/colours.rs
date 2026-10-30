//! Colour. Previews remade for the colour space the camera recorded, and
//! each photo's palette, for finding photos with similar colours.

use std::path::Path;
use tauri::{AppHandle, Emitter, Manager, State};

use super::common::{count, data_dir, err, fills_dir, luts_dir};
use super::export::{export_items, run_export};
use crate::models::{Photo, ScanProgress};
use crate::{db, export, palette, scan, AppState};

/// How many photos' previews have not been checked against the colour space
/// the camera recorded — a library catalogued before it was read.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn colours_status(state: State<'_, AppState>) -> Result<u32, String> {
    let conn = state.conn()?;
    Ok(db::photos_needing_colours(&conn).map_err(err)?.len() as u32)
}

/// Remakes the previews of photos a camera recorded in Adobe RGB, which were
/// made before that was read and show those photos flat: a cancellable
/// `colours` job, progress on `colours-progress`. Returns how many were
/// remade.
///
/// Reads the start of each file, as reading locations does, and extracts a
/// preview only for the photos that need one. The new previews get new names
/// — the webview holds the old ones as immutable — and the old files go.
#[tauri::command]
#[specta::specta]
pub(crate) async fn read_colours(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<u32, String> {
    let db_path = state.db_path.clone();
    let cache_dir = state.cache_dir.clone();
    let cancel = state.jobs.start("colours");
    let emitter = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<u32, String> {
        let conn = db::connect(&db_path).map_err(err)?;
        let todo = db::photos_needing_colours(&conn).map_err(err)?;
        let total = todo.len();
        let mut remade = 0u32;
        let progress = |done: usize, phase: &str| {
            let _ = emitter.emit(
                "colours-progress",
                ScanProgress {
                    phase: phase.into(),
                    current: count(done),
                    total: count(total),
                    message: "Checking colours".into(),
                },
            );
        };
        for (i, photo) in todo.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            let path = Path::new(&photo.path);
            let Ok(stale) = scan::stale_colours(path) else {
                continue; // Gone since the last scan; it stays unchecked.
            };
            let new = stale.and_then(|space| {
                let key = format!("{}-c", photo.file_hash);
                scan::remake_previews(path, photo.orientation, &cache_dir, &key, Some(space)).ok()
            });
            match &new {
                Some(d) => {
                    let (thumb, preview) = (
                        d.thumb_path.to_string_lossy(),
                        d.preview_path.to_string_lossy(),
                    );
                    db::set_colours_read(&conn, photo.id, Some((&thumb, &preview))).map_err(err)?;
                    for old in [&photo.thumb_path, &photo.preview_path]
                        .into_iter()
                        .flatten()
                    {
                        let path = Path::new(old);
                        // Only once nothing else uses it: a copy of the
                        // file at another path shares its previews by hash.
                        if path.starts_with(&cache_dir)
                            && path != d.thumb_path
                            && path != d.preview_path
                        {
                            db::drop_unused_derivative(&conn, old).map_err(err)?;
                        }
                    }
                    remade += 1;
                }
                None => db::set_colours_read(&conn, photo.id, None).map_err(err)?,
            }
            if i % 25 == 0 {
                progress(i + 1, "read");
            }
        }
        progress(total, "done");
        Ok(remade)
    })
    .await
    .map_err(err)?
}

/// How many photos' colours have not been read yet.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn palette_status(state: State<'_, AppState>) -> Result<u32, String> {
    db::palette_backlog_count(&state.conn()?).map_err(err)
}

/// Reads the colours of every photo not read yet: from its thumbnail,
/// or — once it has been edited — from a small render of it as edited. A
/// cancellable `palette` job, progress on `palette-progress`; how many read.
#[tauri::command]
#[specta::specta]
pub(crate) async fn read_palettes(app: AppHandle) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<u32, String> {
        let state = app.state::<AppState>();
        let cancel = state.jobs.start("palette");
        let todo = db::palette_backlog(&state.conn()?).map_err(err)?;
        let total = todo.len();
        let scratch = data_dir(&app).map_err(err)?.join("palette-renders");
        let progress = |done: usize, phase: &str| {
            let _ = app.emit(
                "palette-progress",
                ScanProgress {
                    phase: phase.into(),
                    current: count(done),
                    total: count(total),
                    message: "Reading colours".into(),
                },
            );
        };
        let mut read = 0u32;
        let conn = state.conn()?;
        // In runs of photos: the edited ones in a run are rendered
        // together, so the GPU and its pipeline are made once a run, not once
        // a photo.
        'runs: for (r, run) in todo.chunks(PALETTE_RUN).enumerate() {
            let edited: Vec<i32> = run.iter().filter(|p| p.edited).map(|p| p.id).collect();
            let mut rendered = if edited.is_empty() || cancel.is_cancelled() {
                std::collections::HashMap::new()
            } else {
                render_small_batch(&app, &state, &edited, &scratch)
            };
            for (k, photo) in run.iter().enumerate() {
                if cancel.is_cancelled() {
                    break 'runs;
                }
                let bytes = rendered
                    .remove(&photo.id)
                    .or_else(|| std::fs::read(&photo.image).ok());
                let palette =
                    bytes
                        .and_then(|b| palette::read_image(&b).ok())
                        .unwrap_or(palette::Palette {
                            swatches: 0,
                            signature: Vec::new(),
                        });
                db::set_palette(&conn, photo.id, &palette).map_err(err)?;
                read += 1;
                let i = r * PALETTE_RUN + k;
                if i.is_multiple_of(50) {
                    progress(i + 1, "read");
                }
            }
        }
        let _ = std::fs::remove_dir_all(&scratch);
        progress(total, "done");
        Ok(read)
    })
    .await
    .map_err(err)?
}

/// Where in an export run a file named by `{seq}` came: `0003.jpg`, or a
/// virtual copy's `0003-1.jpg`, is the third, index 2.
fn run_place(file_name: &str) -> Option<usize> {
    let digits: String = file_name.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<usize>().ok()?.checked_sub(1)
}

/// How many photos [`read_palettes`] takes at a time.
const PALETTE_RUN: usize = 100;

/// Small JPEGs of these photos as edited, for reading their colours, by id;
/// one missing where it couldn't be rendered, and the thumbnail stands in.
/// One export run for them all: rendering each through its own made a
/// GPU device and pipeline per photo. Without AI noise reduction, which
/// doesn't move a palette and took a full-size model run per photo.
fn render_small_batch(
    app: &AppHandle,
    state: &AppState,
    photo_ids: &[i32],
    scratch: &Path,
) -> std::collections::HashMap<i32, Vec<u8>> {
    let mut out = std::collections::HashMap::new();
    let dir = scratch.join("run");
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return out;
    }
    let Ok(mut items) = state.conn().and_then(|c| {
        export_items(&c, photo_ids, &luts_dir(state), &fills_dir(state)).map_err(err)
    }) else {
        return out;
    };
    for item in &mut items {
        item.stack.detail.ai_denoise = 0.0;
    }
    let options = export::ExportOptions {
        max_edge: Some(256),
        strip_metadata: true,
        ..Default::default()
    };
    // Named by place in the run, so each file leads back to its photo; a
    // virtual copy's suffix follows the number.
    let ran = run_export(
        app,
        state,
        &items,
        &dir,
        &options,
        Some("{seq}"),
        &|| false,
        |_, _, _| {},
        &|_, _| {},
    );
    if ran.is_ok() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let item = run_place(&entry.file_name().to_string_lossy()).and_then(|at| items.get(at));
            if let (Some(item), Ok(bytes)) = (item, std::fs::read(entry.path())) {
                out.insert(item.photo_id, bytes);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Photos whose colours are like this one's, most alike first.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn similar_colours(
    state: State<'_, AppState>,
    photo_id: i32,
    limit: u32,
) -> Result<Vec<Photo>, String> {
    let conn = state.conn()?;
    let ids = db::similar_colours(&conn, photo_id, limit.max(1) as usize, SIMILAR_COLOURS)
        .map_err(err)?;
    db::photos_by_ids(&conn, &ids).map_err(err)
}

/// How alike two photos' colours must be to count as similar, 0–1.
const SIMILAR_COLOURS: f32 = 0.55;

#[cfg(test)]
mod tests {
    use super::*;

    /// A palette render's file leads back to its photo by its number.
    #[test]
    fn a_run_file_names_its_place() {
        assert_eq!(run_place("0003.jpg"), Some(2));
        assert_eq!(run_place("0003-1.jpg"), Some(2));
        assert_eq!(run_place("12.jpg"), Some(11));
        assert_eq!(run_place("0000.jpg"), None);
        assert_eq!(run_place("DSC0001.jpg"), None);
    }
}
