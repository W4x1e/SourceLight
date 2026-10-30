//! XMP sidecars: reading them into the catalogue and writing them out. See
//! `xmp` for what crosses.

use std::path::Path;
use tauri::{AppHandle, Emitter, State};

use super::common::{count, err, push_undo};
use crate::models::{ScanProgress, XmpImport};
use crate::{
    backup, db, export, fsutil, jobs, models, shell, xmp, AppState, UndoStep, VerdictChange,
};

/// The sidecar Adobe writes next to `foo.arw` is `foo.xmp` — the extension
/// replaced, not appended. Some tools use `foo.arw.xmp`; import tries both, but
/// this canonical form is what we write — except where another photo shares
/// the stem, see [`shell::sidecar_path`].
fn sidecar_path(photo_path: &str, stems: &mut shell::StemIndex) -> std::path::PathBuf {
    shell::sidecar_path(std::path::Path::new(photo_path), stems)
}

/// The given photos' sidecar paths that exist (empty = the whole library):
/// Adobe's `foo.xmp`, or the appended `foo.arw.xmp` some tools write.
/// Where another photo shares the stem, only the appended form is this
/// photo's, and [`sidecar_path`] already gives that.
fn existing_sidecar(photo_path: &str, stems: &mut shell::StemIndex) -> Option<std::path::PathBuf> {
    [
        sidecar_path(photo_path, stems),
        std::path::PathBuf::from(format!("{photo_path}.xmp")),
    ]
    .into_iter()
    .find(|p| p.exists())
}

/// How many of the given photos (empty = the whole library) have a sidecar to
/// read: what "Import all" asks about before it overwrites anything.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn count_sidecars(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<u32, String> {
    let conn = state.conn()?;
    let ids = db::sidecar_photos(&conn, &photo_ids).map_err(err)?;
    let mut n = 0;
    let mut stems = shell::StemIndex::default();
    for id in ids {
        if let Some(photo) = db::get_photo(&conn, id).map_err(err)? {
            n += existing_sidecar(&photo.path, &mut stems).is_some() as u32;
        }
    }
    Ok(n)
}

/// Every photo's verdicts, in id order, a chunk at a time: a whole library is
/// more ids than one statement may bind.
fn verdicts_in_chunks(
    conn: &rusqlite::Connection,
    ids: &[i32],
) -> anyhow::Result<Vec<models::Verdict>> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = Vec::with_capacity(sorted.len());
    for chunk in sorted.chunks(10_000) {
        out.extend(db::verdicts_of(conn, chunk)?);
    }
    Ok(out)
}

/// Reads the sidecars of the given photos (empty = the whole library) into the
/// catalogue — see [`db::import_xmp_meta`] for what each field does. A missing
/// or unreadable sidecar is skipped, not an error — most photos will not have
/// one. A virtual copy reads its original's, which is the file's.
///
/// A sidecar is the authority on rating, label and flag, so reading one
/// overwrites the catalogue's — across a whole library, a month of culling
/// at once. So a read of more than one photo backs the catalogue up
/// first (and reads nothing if it can't), and every verdict it changes is
/// one step on the verdict undo stack.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn import_xmp(
    app: AppHandle,
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<XmpImport, String> {
    let mut conn = state.conn()?;
    let run = |total| SidecarRun::start(&app, &state, total);
    let (out, change) = read_sidecars(&mut conn, &state.db_path, &photo_ids, run)?;
    if let Some(change) = change {
        push_undo(&state, UndoStep::Verdicts(change));
    }
    Ok(out)
}

/// The job id of a sidecar read or write across many photos.
const SIDECARS: &str = "sidecars";

/// Says how far a [`SidecarRun`] has got: `(done, phase)`.
type Report = Box<dyn Fn(usize, &str) + Send>;

/// A sidecar read or write across many photos, as a job: it ran for
/// minutes with nothing shown and nothing to stop it, and a second click,
/// since nothing seemed to happen, started a second pass racing the first
/// over the same files. Registered, so a second is refused and it can be
/// stopped from Activity, with its progress on `sidecars-progress`. One
/// photo's is quick and stays out of the list.
struct SidecarRun {
    cancel: Option<jobs::CancelToken>,
    // Behind a closure made in `start`, so nothing a test reaches names the
    // app handle: linking its event code into the test binary stopped it
    // loading on Windows (STATUS_ENTRYPOINT_NOT_FOUND).
    report: Option<Report>,
    total: usize,
}

impl SidecarRun {
    fn start(app: &AppHandle, state: &AppState, total: usize) -> Result<SidecarRun, String> {
        if total <= 1 {
            return Ok(SidecarRun::quiet());
        }
        if state.jobs.is_running(SIDECARS) {
            return Err(
                "Sidecars are already being read or written. Wait for that to finish, or stop it in Activity."
                    .into(),
            );
        }
        let app = app.clone();
        Ok(SidecarRun {
            cancel: Some(state.jobs.start(SIDECARS)),
            report: Some(Box::new(move |done, phase| {
                let _ = app.emit(
                    "sidecars-progress",
                    ScanProgress {
                        phase: phase.into(),
                        current: count(done),
                        total: count(total),
                        message: "Sidecars".into(),
                    },
                );
            })),
            total,
        })
    }

    /// No job: a single photo, or a test.
    fn quiet() -> SidecarRun {
        SidecarRun {
            cancel: None,
            report: None,
            total: 0,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.is_cancelled())
    }

    fn progress(&self, done: usize) {
        if done.is_multiple_of(100) {
            self.emit(done, "run");
        }
    }

    fn emit(&self, done: usize, phase: &str) {
        if let Some(report) = &self.report {
            report(done, phase);
        }
    }
}

impl Drop for SidecarRun {
    // However it ends: finished, stopped or failed, the Activity row goes.
    fn drop(&mut self) {
        self.emit(self.total, "done");
    }
}

/// [`import_xmp`]'s work, with the catalogue it backs up named by `db_path`:
/// what was read, and the verdicts it replaced as one undoable change.
fn read_sidecars(
    conn: &mut rusqlite::Connection,
    db_path: &Path,
    photo_ids: &[i32],
    run: impl FnOnce(usize) -> Result<SidecarRun, String>,
) -> Result<(XmpImport, Option<VerdictChange>), String> {
    let ids = db::sidecar_photos(conn, photo_ids).map_err(err)?;
    let run = run(ids.len())?;
    if ids.len() > 1 {
        backup::create(conn, db_path, "auto-before-sidecar-import").map_err(|e| {
            format!("couldn't back up the catalogue first, so no sidecars were read: {e:#}")
        })?;
        let _ = backup::prune(db_path);
    }
    let before = verdicts_in_chunks(conn, &ids).map_err(err)?;

    let mut out = XmpImport::default();
    let mut regroup = false;
    let mut stems = shell::StemIndex::default();
    for (i, id) in ids.iter().copied().enumerate() {
        // Stopped: what was read so far stays read, and is one undo.
        if run.cancelled() {
            break;
        }
        run.progress(i);
        let Some(photo) = db::get_photo(conn, id).map_err(err)? else {
            continue;
        };
        let Some(path) = existing_sidecar(&photo.path, &mut stems) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match xmp::read(&text) {
            Ok(sidecar) => {
                let people = db::import_xmp_meta(conn, id, &sidecar).map_err(err)?;
                if let Some(t) = db::sidecar_time_offer(conn, &photo, &sidecar).map_err(err)? {
                    out.times.push(t);
                }
                out.read += 1;
                out.people += people.named;
                out.kept += people.kept;
                regroup |= people.regroup;
            }
            Err(e) => eprintln!("skipping malformed sidecar {}: {e}", path.display()),
        }
    }
    // Faces linked to someone by name join them on a regroup — one, however
    // many there were.
    if regroup {
        db::regroup_people(conn).map_err(err)?;
    }
    // What the sidecars replaced, as one undoable change.
    let after = verdicts_in_chunks(conn, &ids).map_err(err)?;
    let changed: Vec<models::Verdict> = before
        .into_iter()
        .zip(after)
        .filter(|(was, now)| was != now)
        .map(|(was, _)| was)
        .collect();
    if changed.is_empty() {
        return Ok((out, None));
    }
    let n = changed.len();
    out.verdicts_changed = n as u32;
    let label = format!(
        "Sidecar import on {n} photo{}",
        if n == 1 { "" } else { "s" }
    );
    Ok((
        out,
        Some(VerdictChange {
            label,
            before: changed,
        }),
    ))
}

/// A short, actionable reason a sidecar could not be written, naming the two
/// causes the user can do something about. `io::ErrorKind::StorageFull` is still
/// unstable, so out-of-space is matched on the raw OS code: ENOSPC is 28 on Unix,
/// and Windows reports ERROR_DISK_FULL (112) or ERROR_HANDLE_DISK_FULL (39).
fn sidecar_write_error(path: &Path, e: &std::io::Error) -> String {
    let name = sidecar_name(path);
    if is_disk_full(e) {
        format!("{name}: the disk is full, so it and the rest were left as they were")
    } else if e.kind() == std::io::ErrorKind::PermissionDenied {
        format!("{name}: permission denied")
    } else {
        format!("{name}: {e}")
    }
}

/// Out of space: ENOSPC is 28 on Unix, and Windows reports ERROR_DISK_FULL
/// (112) or ERROR_HANDLE_DISK_FULL (39).
fn is_disk_full(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(28) | Some(112) | Some(39))
}

/// Why an existing sidecar was not updated: it is left untouched, and says so.
fn sidecar_left_alone(path: &Path, why: &str) -> String {
    format!(
        "{}: left unchanged, couldn't read it — {why}",
        sidecar_name(path)
    )
}

fn sidecar_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Writes an XMP sidecar next to each given photo (empty = the whole library)
/// carrying its rating, label, flag, keywords, caption, people and location
/// ([`db::xmp_meta`]). An existing sidecar is merged into, never replaced:
/// values the catalogue has win, but a rating, label or keyword it lacks is
/// only taken out if this app wrote it, and a caption or face regions from
/// elsewhere are kept (see [`xmp::merge`]). One that cannot be
/// read or parsed is reported and left exactly as it was. A virtual copy
/// writes its original's, which is the file's.
///
/// Returns a [`BatchOutcome`](export::batch::BatchOutcome), not a bare count: a
/// full disk used to silently under-count the sidecars written and still report
/// success, so a photo could lose its metadata with nothing said. Failures
/// are now collected and surfaced the way the raster exporter does, each naming
/// its cause.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn export_xmp(
    app: AppHandle,
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    // The export dialog's choices, remembered; everything when absent.
    choices: Option<xmp::MetadataChoices>,
) -> Result<export::batch::BatchOutcome, String> {
    let choices = choices.unwrap_or(xmp::MetadataChoices::ALL);
    let conn = state.conn()?;
    let ids = db::sidecar_photos(&conn, &photo_ids).map_err(err)?;
    let run = SidecarRun::start(&app, &state, ids.len())?;

    let mut outcome = export::batch::BatchOutcome::default();
    let mut stems = shell::StemIndex::default();
    for (i, id) in ids.into_iter().enumerate() {
        if run.cancelled() {
            outcome.cancelled = true;
            break;
        }
        run.progress(i);
        let Some(photo) = db::get_photo(&conn, id).map_err(err)? else {
            continue;
        };
        let mut meta = db::xmp_meta(&conn, &photo).map_err(err)?;
        choices.apply(&mut meta);
        let path = sidecar_path(&photo.path, &mut stems);
        // Non-destructive: if a sidecar already exists (e.g. another editor's), keep
        // its develop settings and everything else it holds that we didn't
        // write. A fresh packet is written only when there is no file at all —
        // one we can't read (not UTF-8, locked) or can't parse is skipped, not
        // overwritten, since replacing it is exactly the loss this avoids.
        let packet = match std::fs::read_to_string(&path) {
            Ok(existing) => match xmp::merge(&existing, &meta) {
                // Already says this: left alone, so its date doesn't
                // change and another editor or a backup doesn't take every
                // sidecar in the library for new.
                Ok(packet) if xmp::same_content(&existing, &packet) => {
                    outcome.unchanged += 1;
                    continue;
                }
                Ok(packet) => packet,
                Err(e) => {
                    outcome.failed += 1;
                    outcome
                        .errors
                        .push(sidecar_left_alone(&path, &format!("not valid XMP ({e})")));
                    continue;
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => xmp::write(&meta),
            Err(e) => {
                outcome.failed += 1;
                outcome
                    .errors
                    .push(sidecar_left_alone(&path, &e.to_string()));
                continue;
            }
        };
        // Written beside and renamed over, never truncated first: a full
        // disk or a crash used to leave another editor's sidecar empty, with the
        // develop settings the merge exists to keep.
        match fsutil::write_atomic(&path, packet.as_bytes()) {
            Ok(()) => outcome.written += 1,
            Err(e) => {
                outcome.failed += 1;
                outcome.errors.push(sidecar_write_error(&path, &e));
                // Every sidecar after this would fail the same way.
                if is_disk_full(&e) {
                    break;
                }
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed sidecar write must name the two causes the user can act on, so a
    /// full disk is never reported as a bare success.
    #[test]
    fn a_sidecar_write_error_names_the_cause() {
        let path = Path::new("/photos/DSC01234.arw.xmp");

        // ENOSPC (28) on Unix; ERROR_DISK_FULL (112) / ERROR_HANDLE_DISK_FULL (39)
        // on Windows — all mean the same thing to the user.
        for code in [28, 112, 39] {
            let e = std::io::Error::from_raw_os_error(code);
            assert_eq!(
                sidecar_write_error(path, &e),
                // The batch stops there, and nothing was emptied.
                "DSC01234.arw.xmp: the disk is full, so it and the rest were left as they were"
            );
        }

        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            sidecar_write_error(path, &denied),
            "DSC01234.arw.xmp: permission denied"
        );

        // Anything else keeps the OS detail rather than inventing a reason.
        let other = std::io::Error::other("gremlins");
        assert_eq!(
            sidecar_write_error(path, &other),
            "DSC01234.arw.xmp: gremlins"
        );
    }

    /// Reading sidecars across several photos backs the catalogue up first and
    /// records what it overwrote as one undoable change; a photo whose
    /// verdicts it left alone isn't in it.
    #[test]
    fn reading_sidecars_backs_up_and_can_be_undone() {
        let dir = crate::test_support::Scratch::new("lib-read-sidecars");
        let db_path = dir.join("library.db");
        let mut conn = db::open(&db_path).unwrap();
        for (id, name, rating) in [(1, "a.ARW", 2), (2, "b.ARW", 3)] {
            let photo = dir.join(name);
            std::fs::write(&photo, "raw").unwrap();
            conn.execute(
                "INSERT INTO photos (id, path, filename, file_hash, indexed_at, rating)
                 VALUES (?1, ?2, ?3, 'h', 'x', ?4)",
                rusqlite::params![id, photo.to_string_lossy(), name, rating],
            )
            .unwrap();
        }
        // a's sidecar rates it 5; b's says 3, which it already is.
        for (name, rating) in [("a.xmp", 5), ("b.xmp", 3)] {
            let meta = xmp::XmpMeta {
                rating: Some(rating),
                ..Default::default()
            };
            std::fs::write(dir.join(name), xmp::write(&meta)).unwrap();
        }

        let (read, change) =
            read_sidecars(&mut conn, &db_path, &[1, 2], |_| Ok(SidecarRun::quiet())).unwrap();
        assert_eq!((read.read, read.verdicts_changed), (2, 1));
        let change = change.expect("an undoable change");
        assert_eq!(change.before.len(), 1);
        assert_eq!((change.before[0].id, change.before[0].rating), (1, 2));
        assert!(
            backup::list(&db_path)
                .unwrap()
                .iter()
                .any(|b| b.filename.contains("sidecar-import")),
            "backed up first"
        );
        db::restore_verdicts(&mut conn, &change.before).unwrap();
        let rating: i32 = conn
            .query_row("SELECT rating FROM photos WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rating, 2, "undone");
    }
}
