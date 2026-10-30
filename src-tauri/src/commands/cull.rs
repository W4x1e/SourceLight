//! Culling: ratings, flags and colour labels, and undoing them. Quality
//! scores, duplicates and binning rejects.

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

use super::common::{count, err, photos_removed, push_undo, undo_label};
use crate::models::ScanProgress;
use crate::{
    backup, db, dedup, models, quality, search, shell, sync, AppState, UndoStep, VerdictChange,
};

/// Records what a verdict change is about to overwrite, then makes it. Every
/// rating, flag and colour-label write goes through here, so every one can be
/// undone — see `undo_verdict`.
fn change_verdicts(
    state: &AppState,
    photo_ids: &[i32],
    what: &str,
    write: impl FnOnce(&rusqlite::Connection) -> anyhow::Result<()>,
) -> Result<(), String> {
    let conn = state.conn()?;
    let before = db::verdicts_of(&conn, photo_ids).map_err(err)?;
    write(&conn).map_err(err)?;
    if !before.is_empty() {
        let label = undo_label(what, before.len());
        push_undo(state, UndoStep::Verdicts(VerdictChange { label, before }));
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) fn set_rating(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    rating: i32,
) -> Result<(), String> {
    change_verdicts(&state, &photo_ids, "Rating", |c| {
        db::set_rating_many(c, &photo_ids, rating)
    })
}

#[tauri::command]
#[specta::specta]
pub(crate) fn set_flag(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    flag: i32,
) -> Result<(), String> {
    change_verdicts(&state, &photo_ids, "Flag", |c| {
        db::set_flag_many(c, &photo_ids, flag)
    })
}

/// Sets a different flag on each photo as **one** undoable change — accepting a
/// suggested cull, or settling every duplicate group, is a single
/// decision, so a single Ctrl+Z takes it back. `what` names it for Undo.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_flags(
    state: State<'_, AppState>,
    flags: Vec<models::FlagChange>,
    what: Option<String>,
    suggested: bool,
) -> Result<(), String> {
    let ids: Vec<i32> = flags.iter().map(|f| f.id).collect();
    let what = what.unwrap_or_else(|| "Suggested flags".into());
    change_verdicts(&state, &ids, &what, |c| {
        // Accepted together or not at all.
        let tx = c.unchecked_transaction()?;
        for flag in [-1, 0, 1] {
            let these: Vec<i32> = flags
                .iter()
                .filter(|f| f.flag == flag)
                .map(|f| f.id)
                .collect();
            db::set_flag_many(&tx, &these, flag)?;
        }
        // Remembered as the machine's, so they can be found and told
        // apart from the person's own after a restart.
        if suggested {
            let set: Vec<i32> = flags.iter().filter(|f| f.flag != 0).map(|f| f.id).collect();
            db::mark_flags_suggested(&tx, &set)?;
        }
        tx.commit()?;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub(crate) fn set_color_label(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    label: i32,
) -> Result<(), String> {
    change_verdicts(&state, &photo_ids, "Colour label", |c| {
        db::set_color_label_many(c, &photo_ids, label)
    })
}

/// Takes back the most recent rating, flag or colour-label change this session,
/// restoring every photo it touched to what it was before. `None` when
/// there is nothing left to undo.
#[tauri::command]
#[specta::specta]
pub(crate) fn undo_verdict(
    state: State<'_, AppState>,
) -> Result<Option<models::UndoneVerdicts>, String> {
    let Some(step) = sync::lock_recover(&state.verdict_undo).pop() else {
        return Ok(None);
    };
    let mut conn = state.conn()?;
    let undone = match &step {
        UndoStep::Verdicts(change) => {
            db::restore_verdicts(&mut conn, &change.before).map(|()| models::UndoneVerdicts {
                label: change.label.clone(),
                restored: change.before.clone(),
                ..Default::default()
            })
        }
        UndoStep::Settings { label, before } => db::restore_develop_settings(&mut conn, before)
            .map(|()| models::UndoneVerdicts {
                label: label.clone(),
                settings: before.iter().map(|(id, _)| *id).collect(),
                ..Default::default()
            }),
        // Not undone: said so, and Undo goes on from the step before it.
        UndoStep::Beyond { label } => Ok(models::UndoneVerdicts {
            label: label.clone(),
            beyond: true,
            ..Default::default()
        }),
    };
    match undone {
        Ok(undone) => Ok(Some(undone)),
        Err(e) => {
            // Put it back: the restore rolled back, so the step is still undoable.
            sync::lock_recover(&state.verdict_undo).push(step);
            Err(err(e))
        }
    }
}

/// What moving rejects to the Recycle Bin did.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BinOutcome {
    /// Raw files moved to the Recycle Bin (their sidecars and JPEGs went too).
    files: u32,
    /// Catalogue rows forgotten — the photos and any copies of them.
    rows: u32,
    /// Bytes freed, sidecars included.
    bytes: f64,
    /// Files left alone because a copy of the photo is not a reject.
    kept: u32,
    /// Files left alone because they are not where the catalogue has them.
    missing: u32,
}

/// Moves the files of rejected photos to the Recycle Bin and forgets them.
///
/// One of the two places SourceLight removes a file, with [`delete_photos`], so
/// it is fenced three ways: it only
/// acts on a file whose every catalogue row is a reject (a picked virtual copy
/// keeps the file); it backs the catalogue up first, so ratings and edits can
/// be restored; and it uses the OS Recycle Bin, so the files can be too. The
/// UI confirms before calling it, naming the count and size.
#[tauri::command]
#[specta::specta]
pub(crate) async fn bin_rejects(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<BinOutcome, String> {
    bin(&state, photo_ids, Vec::new(), true, "Binning rejects").await
}

/// Deletes photos the user chose, whatever their flag: "Delete" in the photo
/// menu and the Lightbox.
///
/// A virtual copy is only a catalogue row, so deleting one forgets that row
/// and leaves the file and its original alone. An original's file goes to the
/// Recycle Bin, as [`bin_rejects`] does it, and takes its copies with it.
/// The catalogue is backed up first either way. The UI confirms before
/// calling it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_photos(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<BinOutcome, String> {
    let (mut copies, mut originals) = (Vec::new(), Vec::new());
    {
        let conn = state.conn()?;
        for id in photo_ids {
            match db::get_photo(&conn, id).map_err(err)? {
                Some(p) if p.variant > 0 => copies.push(id),
                Some(_) => originals.push(id),
                None => {}
            }
        }
    }
    bin(&state, originals, copies, false, "Deleting photos").await
}

/// The work of [`bin_rejects`] and [`delete_photos`]: bins the files of
/// `photo_ids` and forgets them, and forgets the virtual copies in `copies`.
/// With `rejects_only`, a file with any row that is not a reject is kept.
async fn bin(
    state: &AppState,
    photo_ids: Vec<i32>,
    copies: Vec<i32>,
    rejects_only: bool,
    label: &str,
) -> Result<BinOutcome, String> {
    let mut conn = state.conn()?;
    let mut paths: Vec<String> = Vec::new();
    for id in &photo_ids {
        if let Some(p) = db::get_photo(&conn, *id).map_err(err)? {
            if !paths.contains(&p.path) {
                paths.push(p.path);
            }
        }
    }

    let mut outcome = BinOutcome {
        files: 0,
        rows: 0,
        bytes: 0.0,
        kept: 0,
        missing: 0,
    };
    let mut going: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stems = shell::StemIndex::default();
    for path in paths {
        // Every row for this file: at this path, and at any other path that is
        // the same file — a NAS added as a share and a mapped drive, a
        // drive root caught under two spellings. Binning one binned them all.
        let raw = PathBuf::from(&path);
        let aliases = db::same_file_rows(&conn, &raw).map_err(err)?;
        let rows = db::rows_for_path(&conn, &path).map_err(err)?;
        if rejects_only
            && rows
                .iter()
                .chain(aliases.iter().map(|(_, row)| row))
                .any(|&(_, flag)| flag != -1)
        {
            outcome.kept += 1;
            continue;
        }
        if !raw.is_file() {
            outcome.missing += 1;
            continue;
        }
        for f in std::iter::once(raw.clone()).chain(shell::bin_companions(&raw, &mut stems)) {
            // Belt and braces: nothing the catalogue holds as a photo of its
            // own leaves as another's companion.
            if f != raw
                && !db::rows_for_path(&conn, &f.to_string_lossy())
                    .map_err(err)?
                    .is_empty()
            {
                continue;
            }
            outcome.bytes += std::fs::metadata(&f).map(|m| m.len() as f64).unwrap_or(0.0);
            files.push(f);
        }
        going.push(path);
        going.extend(aliases.into_iter().map(|(alias, _)| alias));
    }
    if going.is_empty() && copies.is_empty() {
        return Ok(outcome);
    }

    // The work on these photos is about to leave the catalogue. Like a re-tag,
    // that is the moment worth a copy, and a failed copy stops the removal.
    backup::create(&conn, &state.db_path, "auto-before-recycle").map_err(|e| {
        format!("couldn't back up the catalogue first, so nothing was removed: {e:#}")
    })?;
    let _ = backup::prune(&state.db_path);
    outcome.rows = db::remove_copies(&mut conn, &copies).map_err(err)? as u32;

    let result = tauri::async_runtime::spawn_blocking(move || shell::to_recycle_bin(&files))
        .await
        .map_err(err)?;
    // Forget only what actually left the disk: an error part-way through may
    // still have moved some of them.
    let gone: Vec<String> = going
        .into_iter()
        .filter(|p| !Path::new(p).exists())
        .collect();
    outcome.files = gone.len() as u32;
    outcome.rows += db::remove_photos_at(&mut conn, &gone).map_err(err)? as u32;
    state.bump_search_gen();
    photos_removed(state, label);
    result.map_err(err)?;
    Ok(outcome)
}

/// Number of photos still needing a focus/sharpness score — a status line for
/// the cull UI. Mirrors [`search_index_status`].
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn quality_status(state: State<'_, AppState>) -> Result<i32, String> {
    let conn = state.conn()?;
    db::unscored_count(&conn).map_err(err)
}

/// Scores every not-yet-scored photo's focus/sharpness and brightness from its
/// preview, storing them on the row and reporting progress on `quality-progress`. Classical CV, no
/// model — so unlike [`index_for_search`] it needs no download and runs in any
/// build. Sync so the pixel work runs on a worker thread, off the async reactor.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn score_quality(app: AppHandle, state: State<'_, AppState>) -> Result<u32, String> {
    let db_path = state.db_path.clone();
    let queue = {
        let conn = db::connect(&db_path).map_err(err)?;
        db::unscored_photos(&conn, 1_000_000).map_err(err)?
    };
    let total = queue.len();

    let mut done = 0usize;
    for (photo_id, image_path) in queue {
        match std::fs::read(&image_path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                quality::scores_of_bytes(&bytes).map_err(|e| format!("scoring {photo_id}: {e:#}"))
            }) {
            Ok(scores) => {
                let conn = db::connect(&db_path).map_err(err)?;
                if let Err(e) = db::set_scores(&conn, photo_id, &scores) {
                    eprintln!("quality: cannot store score for {photo_id}: {e}");
                }
            }
            // One unreadable/odd file must not abort the whole library.
            Err(e) => eprintln!("quality: {e}"),
        }
        done += 1;
        let _ = app.emit(
            "quality-progress",
            ScanProgress {
                phase: "quality".into(),
                current: count(done),
                total: count(total),
                message: "Scoring focus".into(),
            },
        );
    }

    let _ = app.emit(
        "quality-progress",
        ScanProgress {
            phase: "done".into(),
            current: count(total),
            total: count(total),
            message: "Scored".into(),
        },
    );
    Ok(total as u32)
}

/// Groups of duplicate photos — the cull assistant. Two kinds, merged where
/// they overlap:
///
/// - **Byte-identical files** on different paths: the same card imported twice.
///   Found by size, then head, then full digest — see [`dedup`]. Needs
///   no index, so these show even in a library never indexed for search.
/// - **Near-identical frames** (bursts) by embedding similarity at
///   `threshold`, for the indexed part of the library.
///
/// Exact groups come first. Each group is sorted so its first id is a stable
/// keeper. Async, because the exact check touches the disk.
#[tauri::command]
#[specta::specta]
pub(crate) async fn duplicate_groups(
    app: AppHandle,
    threshold: f32,
    within: Option<Vec<i32>>,
) -> Result<Vec<Vec<i32>>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // Only the photos asked about: Cull's suggestions grouped the
        // whole library — a stat of every file and every embedding in memory
        // — to suggest verdicts for one shoot.
        let (paths, all) = {
            let conn = state.conn()?;
            match &within {
                Some(ids) => (
                    db::photo_paths_in(&conn, ids).map_err(err)?,
                    db::embeddings_for_grouping_in(&conn, ids).map_err(err)?,
                ),
                None => (
                    db::photo_paths(&conn).map_err(err)?,
                    db::embeddings_for_grouping(&conn).map_err(err)?,
                ),
            }
        };
        let mut groups = dedup::exact_groups(&paths);
        groups.extend(
            search::group_near_duplicates(&all, threshold)
                .into_iter()
                .map(|g| g.into_iter().map(|id| id as i32).collect::<Vec<_>>()),
        );
        Ok(dedup::merge_overlapping(groups))
    })
    .await
    .map_err(err)?
}
