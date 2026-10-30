//! Exporting: one photo, a batch, the clipboard, and dragging files out of
//! the app.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

use super::common::{count, data_dir, err, fills_dir, luts_dir, with_develop};
#[cfg(feature = "ai")]
use super::masks::denoise_engine;
use super::masks::ensure_denoised;
use crate::models::ScanProgress;
use crate::{db, develop, export, fsutil, hardware, shell, AppState};
#[cfg(feature = "libraw")]
use crate::{gpu, raw};

/// Stops the running export batch after the file it is on.
#[tauri::command]
#[specta::specta]
pub(crate) fn cancel_export(state: State<'_, AppState>) {
    state.cancel_export.store(true, Ordering::Relaxed);
}

/// Drags photos out of the app — onto Finder, Mail, an editor — as their
/// original files, for the receiver to **copy**.
///
/// The drag offers copy and nothing else, and that is fixed here rather than
/// left to the page: a file dropped on Finder on the same disk is *moved* if
/// the source allows it, and the app promises never to move an original. The
/// page names photos, not paths; missing files are left out; the first photo's
/// thumbnail is the image under the pointer.
#[tauri::command]
#[specta::specta]
pub(crate) async fn drag_photos_out(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    ids: Vec<i32>,
) -> Result<(), String> {
    let photos = {
        let conn = state.conn()?;
        db::photos_by_ids(&conn, &ids).map_err(err)?
    };
    let files: Vec<PathBuf> = photos
        .iter()
        .map(|p| PathBuf::from(&p.path))
        .filter(|p| p.is_file())
        .collect();
    if files.is_empty() {
        return Err("none of these photos' files are there to drag".into());
    }
    let icon = photos.iter().find_map(|p| {
        p.thumb_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|t| t.is_file())
    });
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let image = match icon {
            Some(path) => drag::Image::File(path),
            None => drag::Image::Raw(Vec::new()),
        };
        let started = drag::start_drag(
            &window,
            drag::DragItem::Files(files),
            image,
            |_, _| {},
            drag::Options {
                skip_animatation_on_cancel_or_failure: false,
                mode: drag::DragMode::Copy,
            },
        );
        let _ = tx.send(started.map_err(|e| e.to_string()));
    })
    .map_err(err)?;
    rx.recv().map_err(err)?
}

/// Renders the open photo at full resolution and writes it to `path`.
///
/// Runs on a blocking thread: a full-resolution render plus encode takes
/// hundreds of milliseconds to seconds, and holding the async runtime for that
/// would freeze every other command.
#[tauri::command]
#[specta::specta]
pub(crate) async fn export_develop(
    app: AppHandle,
    path: String,
    options: export::ExportOptions,
) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // The chosen filename wins over the requested format — someone who
        // types ".png" means it.
        let target = std::path::PathBuf::from(&path);
        let options = export::ExportOptions {
            format: export::ExportFormat::from_path(&target).unwrap_or(options.format),
            ..options
        };

        // Read before the develop lock is taken, so the two are never held
        // together.
        let photo_id = with_develop(&state, |dev| Ok(dev.open_photo_id()))?;
        ensure_denoised(&app, &state)?;
        let metadata = match photo_id {
            Some(id) => export::exif::ExportMetadata::load(&state.conn()?, id).map_err(err)?,
            None => None,
        };
        refuse_photo_target(&state.conn()?, &target, photo_id)?;
        let bytes = with_develop(&state, |dev| {
            dev.export(&options, metadata.as_ref()).map_err(err)
        })?;
        // The save dialog has already asked about replacing a file that is
        // there; this makes the replacement all-or-nothing.
        fsutil::write_atomic(&target, &bytes).map_err(|e| {
            err(anyhow::Error::new(e).context(format!("cannot write {}", target.display())))
        })?;
        // A shoot counts its exported picks. A record, not the export:
        // failing to write it doesn't undo the file.
        if let Some(id) = photo_id {
            if let Ok(conn) = state.conn() {
                let _ = db::record_exports(&conn, &[id]);
            }
        }
        Ok(count(bytes.len()))
    })
    .await
    .map_err(err)?
}

/// Refuses an export over a photo: the open photo's own file — a JPEG
/// or TIFF exported as itself proposes its own name — one of its companions
/// (a RAW+JPEG camera JPEG), or any file the catalogue holds as a photo.
/// The save dialog's "replace?" names the file, but a re-encoded render in
/// place of a camera original is a loss SourceLight promises never to cause.
fn refuse_photo_target(
    conn: &rusqlite::Connection,
    target: &Path,
    photo_id: Option<i32>,
) -> Result<(), String> {
    // Compared as the file system would: canonical where the file exists, and
    // without case on Windows and macOS.
    let key = |p: &Path| {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let s = p.to_string_lossy().to_string();
        if cfg!(any(windows, target_os = "macos")) {
            s.to_lowercase()
        } else {
            s
        }
    };
    let wanted = key(target);
    let refuse = || {
        Err(format!(
            "{} is one of your photos, so it wasn't replaced. Choose another name.",
            target.display()
        ))
    };
    if let Some(photo) = photo_id.and_then(|id| db::get_photo(conn, id).ok().flatten()) {
        let source = Path::new(&photo.path);
        if std::iter::once(source.to_path_buf())
            .chain(shell::companions(source))
            .any(|p| key(&p) == wanted)
        {
            return refuse();
        }
    }
    if target.exists()
        && !db::rows_for_path(conn, &target.to_string_lossy())
            .map_err(err)?
            .is_empty()
    {
        return refuse();
    }
    Ok(())
}

/// Why a batch export may not fit on its drive — see `export_space_check`.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpaceWarning {
    /// About how much the export will write, e.g. "3.2 GB".
    needed: String,
    /// How much the destination drive has free.
    free: String,
}

/// Whether a batch export looks likely to fill the destination drive.
///
/// Asked before the export starts, so the user can choose another folder
/// instead of discovering a full disk at photo 300 of 400. A warning, not a
/// refusal: the size is an estimate (`export::estimate_bytes`, deliberately
/// high), and the user may know they are about to free space. `None` when it
/// fits, or when free space cannot be read.
#[tauri::command]
#[specta::specta]
pub(crate) fn export_space_check(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    directory: String,
    options: export::ExportOptions,
) -> Result<Option<SpaceWarning>, String> {
    let conn = state.conn()?;
    let mut needed: u64 = 0;
    for id in &photo_ids {
        // A photo with unknown dimensions is counted as a common 24 MP frame
        // rather than as nothing.
        let (w, h) = match db::get_photo(&conn, *id).map_err(err)? {
            Some(p) if p.width > 0 && p.height > 0 => (p.width as u32, p.height as u32),
            _ => (6000, 4000),
        };
        needed += export::estimate_bytes(w, h, &options);
    }
    Ok(
        hardware::shortfall(std::path::Path::new(&directory), needed).map(|(_, free)| {
            SpaceWarning {
                needed: hardware::human_bytes(needed),
                free: hardware::human_bytes(free),
            }
        }),
    )
}

/// The photographer's name and copyright line, written into every export's
/// EXIF. Empty strings when unset.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportRights {
    pub creator: String,
    pub copyright: String,
}

#[tauri::command]
#[specta::specta]
pub(crate) fn get_export_rights(state: State<'_, AppState>) -> Result<ExportRights, String> {
    let conn = state.conn()?;
    let read = |key| {
        db::get_setting(&conn, key)
            .map(Option::unwrap_or_default)
            .map_err(err)
    };
    Ok(ExportRights {
        creator: read(export::exif::CREATOR_KEY)?,
        copyright: read(export::exif::COPYRIGHT_KEY)?,
    })
}

#[tauri::command]
#[specta::specta]
pub(crate) fn set_export_rights(
    state: State<'_, AppState>,
    rights: ExportRights,
) -> Result<(), String> {
    let conn = state.conn()?;
    db::set_setting(&conn, export::exif::CREATOR_KEY, rights.creator.trim()).map_err(err)?;
    db::set_setting(&conn, export::exif::COPYRIGHT_KEY, rights.copyright.trim()).map_err(err)
}

/// Renders one photo, with its edits, for the clipboard: a PNG at most
/// `max_edge` on its long side (2,048 unless asked), sRGB, with no metadata —
/// a photo pasted into a chat should not carry where it was taken. It goes
/// into `<data>/clipboard/`, the only place the `thumb://` scheme serves
/// from, for the page to fetch and put on the clipboard; the folder is emptied
/// first, so it never holds more than the last one. Each render gets a folder
/// of its own because `thumb://` answers are cached as immutable. Returns the
/// file's path.
#[tauri::command]
#[specta::specta]
pub(crate) async fn render_for_clipboard(
    app: AppHandle,
    photo_id: i32,
    max_edge: Option<u32>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let root = data_dir(&app).map_err(err)?.join("clipboard");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(chrono::Utc::now().timestamp_millis().to_string());
        std::fs::create_dir_all(&dir).map_err(err)?;
        let items = export_items(
            &state.conn()?,
            &[photo_id],
            &luts_dir(&state),
            &fills_dir(&state),
        )?;
        if items.is_empty() {
            return Err("that photo is no longer in the library".into());
        }
        // PNG: the only image the page's clipboard takes.
        let options = export::ExportOptions {
            format: export::ExportFormat::Png,
            max_edge: Some(max_edge.unwrap_or(2048).clamp(256, 8192)),
            strip_metadata: true,
            ..Default::default()
        };
        let outcome = run_export(
            &app,
            &state,
            &items,
            &dir,
            &options,
            None,
            &|| false,
            |_, _, _| {},
            &|_, _| {},
        )?;
        if outcome.written == 0 {
            return Err(outcome
                .errors
                .first()
                .cloned()
                .unwrap_or_else(|| "the photo could not be rendered".into()));
        }
        let file = std::fs::read_dir(&dir)
            .map_err(err)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.is_file())
            .ok_or_else(|| "the render went missing".to_string())?;
        Ok(file.to_string_lossy().into_owned())
    })
    .await
    .map_err(err)?
}

/// Renders a photo as edited for the grid and loupe, and keeps the file on
/// its row; `None` when it has no edits. A change to its edits forgets the
/// file (migration 53), and the page asks again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn render_edited_preview(
    app: AppHandle,
    photo_id: i32,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let Some((Some(settings), _)) =
            crate::phone_link::photo_edits(&state.conn()?, photo_id.into()).map_err(err)?
        else {
            return Ok(None);
        };
        let tag = crate::phone_link::edit_tag(Some(&settings)).unwrap_or_default();
        let file = render_edited(&app, &state, photo_id, &tag)?
            .to_string_lossy()
            .into_owned();
        db::set_edited_path(&state.conn()?, photo_id, &settings, &file).map_err(err)?;
        Ok(Some(file))
    })
    .await
    .map_err(err)?
}

/// The long side of [`render_edited`]'s JPEGs: a preview's.
const EDITED_PREVIEW_EDGE: u32 = 1600;

/// One photo as edited, as a JPEG the size of a preview, for the library's
/// grid and loupe and a phone's loupe (docs/MOBILE.md): raws as developed,
/// every edit applied. Kept in `thumbs/edited/`, named by the photo and its
/// edits' `tag`, so asking again is a file read, and the copy of older edits
/// goes when a newer one is made. One at a time, as each is an export run on
/// the GPU. Without AI noise reduction, which would run a full-size model for
/// a preview.
pub(crate) fn render_edited(
    app: &AppHandle,
    state: &AppState,
    photo_id: i32,
    tag: &str,
) -> Result<PathBuf, String> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let dir = state.cache_dir.join("edited");
    let file = dir.join(format!("{photo_id}_{tag}.jpg"));
    if file.is_file() {
        return Ok(file);
    }
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    // Made while this one waited.
    if file.is_file() {
        return Ok(file);
    }
    let run = dir.join(format!("run-{photo_id}"));
    let _ = std::fs::remove_dir_all(&run);
    std::fs::create_dir_all(&run).map_err(err)?;
    let mut items = export_items(
        &state.conn()?,
        &[photo_id],
        &luts_dir(state),
        &fills_dir(state),
    )?;
    for item in &mut items {
        item.stack.detail.ai_denoise = 0.0;
    }
    let options = export::ExportOptions {
        max_edge: Some(EDITED_PREVIEW_EDGE),
        strip_metadata: true,
        ..Default::default()
    };
    let ran = run_export(
        app,
        state,
        &items,
        &run,
        &options,
        Some("{seq}"),
        &|| false,
        |_, _, _| {},
        &|_, _| {},
    );
    let jpeg = ran.ok().and_then(|_| {
        std::fs::read_dir(&run)
            .ok()?
            .flatten()
            .find_map(|e| std::fs::read(e.path()).ok())
    });
    let _ = std::fs::remove_dir_all(&run);
    let jpeg = jpeg.ok_or("the photo could not be rendered as edited")?;
    let older = format!("{photo_id}_");
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(&older) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let part = file.with_extension("part");
    std::fs::write(&part, &jpeg).map_err(err)?;
    std::fs::rename(&part, &file).map_err(err)?;
    Ok(file)
}

/// What exporting these photos needs from the catalogue, read up front: each
/// file, its edit, its copy's name and its EXIF. Photos no longer there are
/// left out.
pub(crate) fn export_items(
    conn: &rusqlite::Connection,
    photo_ids: &[i32],
    luts: &Path,
    fills: &Path,
) -> Result<Vec<export::batch::BatchItem>, String> {
    let creator = db::get_setting(conn, export::exif::CREATOR_KEY).map_err(err)?;
    let copyright = db::get_setting(conn, export::exif::COPYRIGHT_KEY).map_err(err)?;
    let mut items = Vec::with_capacity(photo_ids.len());
    // Each LUT read once however many photos use it.
    let mut loaded: std::collections::HashMap<String, Option<std::sync::Arc<Vec<f32>>>> =
        std::collections::HashMap::new();
    let fill_store = develop::erase::FillStore::new(fills.to_path_buf());
    for id in photo_ids {
        let Some(photo) = db::get_photo(conn, *id).map_err(err)? else {
            continue;
        };
        // A photo that was never developed exports at its defaults, which
        // is the same thing the library preview shows.
        let stack = db::get_develop_settings(conn, *id)
            .map_err(err)?
            .unwrap_or_default();
        let lut = (!stack.lut.name.is_empty())
            .then(|| {
                loaded
                    .entry(stack.lut.name.clone())
                    .or_insert_with(|| {
                        develop::lut::load_texels(luts, &stack.lut.name).map(std::sync::Arc::new)
                    })
                    .clone()
            })
            .flatten();
        // Its erases' fills; one that cannot be read fails the photo when
        // it is rendered.
        let fills = develop::erase::fill_names(&stack.erases)
            .into_iter()
            .filter_map(|name| match fill_store.load(name) {
                Ok(fill) => Some((name.to_string(), std::sync::Arc::new(fill))),
                Err(e) => {
                    eprintln!("export: {e:#}");
                    None
                }
            })
            .collect();
        items.push(export::batch::BatchItem {
            photo_id: *id,
            path: std::path::PathBuf::from(&photo.path),
            stack,
            lut,
            fills,
            // Copies share their master's filename, so they carry their
            // own name into the export — or their number, if unnamed.
            suffix: (photo.variant > 0).then(|| {
                photo
                    .variant_name
                    .clone()
                    .unwrap_or_else(|| format!("copy{}", photo.variant))
            }),
            metadata: Some(
                export::exif::ExportMetadata::for_photo(
                    conn,
                    &photo,
                    creator.clone(),
                    copyright.clone(),
                )
                .map_err(err)?,
            ),
        });
    }
    Ok(items)
}

/// Renders these photos with their edits and writes them into `directory`, the
/// one path every export takes: a batch, and the single photo copied to the
/// clipboard. AI noise reduction is loaded only when an edit asks for it.
///
/// `cancel` is the caller's own: it was the batch export's flag for
/// every caller, and that flag stayed set after a cancelled export, so every
/// later palette render and Copy image in the session was cancelled too.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_export(
    app: &AppHandle,
    state: &AppState,
    items: &[export::batch::BatchItem],
    directory: &std::path::Path,
    options: &export::ExportOptions,
    rename: Option<&str>,
    cancel: &(dyn Fn() -> bool + Sync),
    progress: impl FnMut(usize, usize, &str),
    finished: &(dyn Fn(i32, Option<&str>) + Sync),
) -> Result<export::batch::BatchOutcome, String> {
    // Exporting means decoding the raw at full resolution, so this whole
    // tail needs LibRaw — gated the same way `open_develop` and the
    // `develop://` handler are. Without it the batch is prepared and then
    // honestly refused, rather than failing to compile.
    #[cfg(feature = "libraw")]
    {
        // The app's device, with a pipeline of its own: a render rewrites
        // the pipeline's tables, and Develop may be rendering beside it.
        let gpu = gpu::shared().map_err(err)?;
        let pipeline = gpu::pipeline::Pipeline::new(&gpu);
        let decoder = raw::libraw::LibRawDecoder::new();

        // AI noise reduction for the photos whose edit asks for it,
        // the model loaded only if one does. A photo it cannot run for
        // fails with the reason rather than exporting noisier than it
        // looks in Develop.
        #[cfg(feature = "ai")]
        let engine = items
            .iter()
            .any(|i| i.stack.detail.ai_denoise > 0.0)
            .then(|| denoise_engine(app, state));
        #[cfg(feature = "ai")]
        let denoiser = |px: &[u16], w: u32, h: u32| -> anyhow::Result<Vec<u16>> {
            match &engine {
                Some(Ok(engine)) => engine.denoise(px, w, h, |_, _| {}, cancel),
                Some(Err(e)) => anyhow::bail!("AI noise reduction: {e}"),
                None => anyhow::bail!("AI noise reduction is not available"),
            }
        };
        #[cfg(feature = "ai")]
        let denoise: Option<&export::batch::Denoiser> = Some(&denoiser);
        #[cfg(not(feature = "ai"))]
        let denoise: Option<&export::batch::Denoiser> = None;
        #[cfg(not(feature = "ai"))]
        let _ = (app, state);

        let outcome = export::batch::export_batch_reporting(
            &gpu, &pipeline, &decoder, items, directory, options, rename, denoise, cancel,
            progress, finished,
        );
        Ok(outcome)
    }
    #[cfg(not(feature = "libraw"))]
    {
        let _ = (
            app, state, items, directory, options, rename, cancel, progress, finished,
        );
        Err("this build was compiled without raw decoding".to_string())
    }
}

/// Exports many photos into a directory, each with its own saved edits.
///
/// Runs entirely off the develop session. That slot holds one photo resident on
/// the GPU, and routing a batch through it would evict whatever the user has
/// open — so this builds its own pipeline on the app's device and walks the
/// list, using the same render call the preview does.
#[tauri::command]
#[specta::specta]
pub(crate) async fn export_batch(
    app: AppHandle,
    photo_ids: Vec<i32>,
    directory: String,
    options: export::ExportOptions,
    rename: Option<String>,
    subfolder: Option<String>,
) -> Result<export::batch::BatchOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // One batch at a time: a second one reset the first's Cancel
        // and both drove the one queue.
        if state.exporting.swap(true, Ordering::AcqRel) {
            return Err("An export is already running — wait for it, or cancel it first.".into());
        }
        struct Done<'a>(&'a AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _done = Done(&state.exporting);
        state.cancel_export.store(false, Ordering::Relaxed);
        let mut directory = std::path::PathBuf::from(&directory);
        if !directory.is_dir() {
            return Err(format!("{} is not a folder", directory.display()));
        }

        // An optional single-level output subfolder, created if missing. Sanitised
        // to one safe segment so it can't escape the chosen folder.
        if let Some(sub) = subfolder
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let safe: String = sub
                .chars()
                .map(|c| match c {
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    c if c.is_control() => '_',
                    c => c,
                })
                .collect();
            let safe = safe
                .trim_matches(|c: char| c == '.' || c == ' ')
                .to_string();
            directory = directory.join(if safe.is_empty() {
                "Exports".into()
            } else {
                safe
            });
            std::fs::create_dir_all(&directory).map_err(|e| {
                err(anyhow::Error::new(e).context(format!("cannot create {}", directory.display())))
            })?;
        }

        // Everything the batch needs is read up front, so the database is not
        // held across a job that can run for minutes.
        let items = export_items(
            &state.conn()?,
            &photo_ids,
            &luts_dir(&state),
            &fills_dir(&state),
        )?;

        let outcome = run_export(
            &app,
            &state,
            &items,
            &directory,
            &options,
            rename.as_deref(),
            &|| state.cancel_export.load(Ordering::Relaxed),
            |done, total, name| {
                let _ = app.emit(
                    "export-progress",
                    ScanProgress {
                        phase: "export".into(),
                        current: count(done),
                        total: count(total),
                        message: name.to_string(),
                    },
                );
            },
            // Each photo's own result, as it lands.
            &|photo_id, error| {
                let _ = app.emit(
                    "export-item",
                    serde_json::json!({ "photoId": photo_id, "error": error }),
                );
            },
        )?;
        let mut outcome = outcome;
        outcome.directory = Some(directory.to_string_lossy().into_owned());
        // What was written, for the shoots' counts.
        if let Ok(conn) = state.conn() {
            let _ = db::record_exports(&conn, &outcome.exported);
        }
        Ok(outcome)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Export never lands on a photo: not the open one, not its
    /// RAW+JPEG partner, not any other photo the catalogue holds; a new name
    /// beside them is fine.
    #[test]
    fn an_export_is_refused_over_a_photo() {
        let dir = crate::test_support::Scratch::new("lib-export-target");
        let conn = db::open_memory().unwrap();
        for (id, name) in [(1, "IMG_0412.JPG"), (2, "DSC1.ARW"), (3, "other.tif")] {
            std::fs::write(dir.join(name), "photo").unwrap();
            conn.execute(
                "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (?1, ?2, ?3, 'h', 'x')",
                rusqlite::params![id, dir.join(name).to_string_lossy(), name],
            )
            .unwrap();
        }
        std::fs::write(dir.join("DSC1.JPG"), "camera jpeg").unwrap();

        assert!(refuse_photo_target(&conn, &dir.join("img_0412.jpg"), Some(1)).is_err());
        assert!(refuse_photo_target(&conn, &dir.join("DSC1.JPG"), Some(2)).is_err());
        assert!(refuse_photo_target(&conn, &dir.join("other.tif"), Some(1)).is_err());
        assert!(refuse_photo_target(&conn, &dir.join("IMG_0412-edit.jpg"), Some(1)).is_ok());
    }
}
