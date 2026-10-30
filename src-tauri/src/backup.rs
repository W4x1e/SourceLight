//! Catalogue backups.
//!
//! The library database is the only copy of everything the user decided:
//! ratings, flags, colour labels, tags, places, trips, collections, people and
//! every develop edit. None of it exists anywhere else — XMP sidecars carry
//! only rating, label and keywords, and only for photos the user chose to
//! export. Lose `library.db` and the photos survive; the work does not.
//!
//! So the rule here is that the risky moment gets a copy taken for it, without
//! being asked. A schema migration rewrites tables in place, and migration 11
//! already rebuilds `photos` wholesale — that is exactly the operation where a
//! bug costs someone their library rather than their afternoon.
//!
//! **`VACUUM INTO`, not a file copy.** The database runs in WAL mode, so at any
//! moment the committed truth is split between `library.db` and its `-wal`
//! sidecar. Copying the one file yields a torn, stale database that opens
//! cleanly and is quietly missing recent work — the worst possible failure for
//! a backup. `VACUUM INTO` asks SQLite for a consistent snapshot instead, and
//! compacts it on the way out.

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// How many automatic backups to keep before the oldest is dropped.
///
/// Three covers the realistic case — a bad upgrade noticed a version or two
/// later — without letting copies of a large catalogue accumulate unbounded.
/// Manual backups are never pruned; the user asked for those.
const KEEP_AUTOMATIC: usize = 3;

/// The pools kept apart from the routine risky ones, and how many of
/// each: binning after three shoots used to prune the only copy taken before
/// a schema upgrade, or the catalogue a restore replaced.
const SEPARATE_POOLS: [(&str, usize); 2] = [("library-auto-v", 2), ("library-auto-replaced", 2)];

/// How many daily backups to keep: a week of them. A pool of their own, so a
/// week of quiet days never pushes out the copy taken before a migration or a
/// re-tag.
const KEEP_DAILY: usize = 7;

/// Prefix of the daily backups' filenames (`library-daily-<stamp>.db`).
const DAILY: &str = "library-daily";

/// A daily backup is due when the newest is older than this: a day, less a
/// few hours, so opening the app at roughly the same time each day takes one.
pub const DAILY_EVERY_SECS: u64 = 20 * 60 * 60;

/// Where backups live, beside the database they came from.
pub fn dir_for(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("backups")
}

/// A backup on disk, as the UI wants to show it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct BackupInfo {
    pub path: String,
    pub filename: String,
    /// Bytes, so the UI can format it however it likes.
    pub bytes: f64,
    /// Seconds since the Unix epoch, or 0 if the filesystem would not say.
    pub modified: f64,
    /// False for one the user asked for, which is never pruned automatically.
    pub automatic: bool,
}

/// Writes a consistent snapshot of `conn` into the backup directory.
///
/// `reason` becomes part of the filename so the list reads as a history rather
/// than a pile of timestamps — `auto-v11` is the copy taken before migration 11.
pub fn create(conn: &Connection, db_path: &Path, reason: &str) -> Result<PathBuf> {
    let dir = dir_for(db_path);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create backup folder {}", dir.display()))?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe: String = reason
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let out = dir.join(format!("library-{safe}-{stamp}.db"));

    // VACUUM INTO refuses to overwrite, which is the behaviour we want — but it
    // means a second backup within the same second would fail. Rare, and a
    // clear error beats silently replacing the earlier one.
    if out.exists() {
        bail!("a backup for this second already exists: {}", out.display());
    }

    // The path is bound as a parameter rather than formatted in: a Windows path
    // is full of backslashes, and a SQLite string literal would need them
    // escaped. Binding sidesteps the question entirely.
    if let Err(e) = conn.execute("VACUUM INTO ?1", [out.to_string_lossy().as_ref()]) {
        // A failed VACUUM INTO can leave an empty file behind. Left there it
        // looked like the newest backup, and counted against the ones
        // [`prune`] keeps, so a few failed attempts could push the last good
        // copy out.
        let _ = std::fs::remove_file(&out);
        return Err(e).with_context(|| format!("cannot write backup to {}", out.display()));
    }

    Ok(out)
}

/// Every backup, newest first.
pub fn list(db_path: &Path) -> Result<Vec<BackupInfo>> {
    let dir = dir_for(db_path);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("db") {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata()?;
        out.push(BackupInfo {
            path: path.to_string_lossy().to_string(),
            automatic: filename.starts_with("library-auto") || filename.starts_with(DAILY),
            filename,
            bytes: meta.len() as f64,
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0) as f64,
        });
    }
    out.sort_by(|a, b| b.modified.total_cmp(&a.modified));
    Ok(out)
}

/// The backup [`list`] shows as `filename`. Anything else is refused, so only
/// a file in the backups folder can be restored.
pub fn named(db_path: &Path, filename: &str) -> Result<PathBuf> {
    list(db_path)?
        .into_iter()
        .find(|b| b.filename == filename)
        .map(|b| PathBuf::from(b.path))
        .with_context(|| format!("there is no backup called {filename}"))
}

/// Drops the oldest automatic backups beyond [`KEEP_AUTOMATIC`], and the oldest
/// daily ones beyond [`KEEP_DAILY`].
///
/// Never touches a manual one. Only real databases count towards the ones
/// kept: an empty or truncated automatic file (a failed backup from an older
/// build) is removed, and never pushes a good copy out. Failing to prune is not
/// worth failing a startup over, so the caller is free to ignore the result.
pub fn prune(db_path: &Path) -> Result<()> {
    let all = list(db_path)?;
    let (real, broken): (Vec<_>, Vec<_>) = all
        .iter()
        .filter(|b| b.automatic)
        .partition(|b| looks_like_a_database(Path::new(&b.path)));
    let (daily, mut risky): (Vec<_>, Vec<_>) = real
        .into_iter()
        .partition(|b| b.filename.starts_with(DAILY));
    let mut doomed: Vec<&BackupInfo> = broken;
    for (prefix, keep) in SEPARATE_POOLS {
        let (pool, rest): (Vec<_>, Vec<_>) = risky
            .into_iter()
            .partition(|b| b.filename.starts_with(prefix));
        doomed.extend(pool.into_iter().skip(keep));
        risky = rest;
    }
    for old in doomed
        .into_iter()
        .chain(risky.into_iter().skip(KEEP_AUTOMATIC))
        .chain(daily.into_iter().skip(KEEP_DAILY))
    {
        let _ = std::fs::remove_file(&old.path);
    }
    Ok(())
}

/// Erases face data from every automatic backup, as
/// [`crate::db::erase_face_data`] does from the catalogue: a week of daily
/// copies otherwise kept every template and name the user had just deleted.
/// One that will not open to be scrubbed is deleted instead.
///
/// Returns `(scrubbed, kept)`. Kept are the backups the user made, which this
/// never touches, and damaged catalogues set aside; both still hold face data.
pub fn scrub_face_data(db_path: &Path) -> Result<(usize, usize)> {
    let (mut scrubbed, mut kept) = (0, 0);
    for b in list(db_path)? {
        if !b.automatic {
            kept += 1;
            continue;
        }
        if crate::db::scrub_face_data(Path::new(&b.path)).is_err() {
            std::fs::remove_file(&b.path)
                .with_context(|| format!("removing a backup that would not scrub: {}", b.path))?;
        }
        scrubbed += 1;
    }
    if let Ok(entries) = std::fs::read_dir(dir_for(db_path).join("damaged")) {
        kept += entries.flatten().count();
    }
    Ok((scrubbed, kept))
}

/// Takes the day's backup if one is due — none yet, or the newest daily one
/// older than [`DAILY_EVERY_SECS`] as of `now` (seconds since the epoch) —
/// and prunes. `None` when none was due.
///
/// The catalogue is the only copy of every rating, edit and name, and a
/// backup taken only before risky operations left a week of ordinary work
/// with none. Taken on launch, from a live connection: `VACUUM INTO` gives a
/// consistent snapshot whatever the app is doing.
pub fn daily_if_due(conn: &Connection, db_path: &Path, now: u64) -> Result<Option<PathBuf>> {
    let newest = list(db_path)?
        .into_iter()
        .filter(|b| b.filename.starts_with(DAILY) && looks_like_a_database(Path::new(&b.path)))
        .map(|b| b.modified as u64)
        .max();
    if newest.is_some_and(|t| now.saturating_sub(t) < DAILY_EVERY_SECS) {
        return Ok(None);
    }
    let path = create(conn, db_path, "daily")?;
    let _ = prune(db_path);
    Ok(Some(path))
}

/// Whether `path` starts like an SQLite database. Cheap: sixteen bytes.
fn looks_like_a_database(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic == b"SQLite format 3\0"
}

/// The newest backup that passes SQLite's integrity check, if any.
///
/// Newest first, stopping at the first good one, so a large catalogue pays for
/// one check in the usual case.
pub fn newest_good(db_path: &Path) -> Option<BackupInfo> {
    list(db_path).ok()?.into_iter().find(|b| {
        looks_like_a_database(Path::new(&b.path))
            && Connection::open_with_flags(&b.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .and_then(|c| c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)))
                .is_ok_and(|v| v == "ok")
    })
}

/// What happened when the catalogue was found damaged at startup.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    /// The backup put in its place, or `None` when there was no good one and
    /// the app started with an empty catalogue.
    pub restored: Option<BackupInfo>,
    /// Where the damaged file was kept, for a support case or a repair tool.
    pub damaged_copy: String,
}

/// Moves a damaged catalogue aside and puts the newest good backup in its
/// place. The damaged file (and its WAL) is kept under `backups/damaged/`,
/// outside the backup list, so it is never offered as a restore and never
/// pruned.
pub fn recover_damaged(db_path: &Path) -> Result<Recovery> {
    let restored = newest_good(db_path);
    let damaged = set_aside_damaged(db_path)?;
    if let Some(good) = &restored {
        std::fs::copy(&good.path, db_path)
            .with_context(|| format!("cannot restore {}", good.path))?;
    }
    Ok(Recovery {
        restored,
        damaged_copy: damaged.to_string_lossy().to_string(),
    })
}

/// `db_path` with `suffix` on the end: its `-wal` or `-shm`.
fn sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let mut p = db_path.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}

/// Moves the catalogue to `backups/damaged/` **together with its WAL**, or not
/// at all. Returns where it went.
///
/// The WAL is part of the database, not a cache beside it: after an unclean
/// exit it holds committed work the main file has never seen, and after a
/// part-finished checkpoint the main file is *meant* to be inconsistent until
/// the WAL is read over it. So it is never deleted. Left behind, it would be
/// replayed over whatever is put in the catalogue's place and damage that too,
/// so if it cannot be moved the catalogue is put back and nothing changes.
fn set_aside_damaged(db_path: &Path) -> Result<PathBuf> {
    let dir = dir_for(db_path).join("damaged");
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let to = dir.join(format!("library-damaged-{stamp}.db"));
    std::fs::rename(db_path, &to).with_context(|| {
        format!(
            "cannot move the damaged catalogue aside to {}",
            to.display()
        )
    })?;
    let wal = sidecar(db_path, "-wal");
    if wal.exists() {
        if let Err(e) = std::fs::rename(&wal, sidecar(&to, "-wal")) {
            let _ = std::fs::rename(&to, db_path);
            return Err(e)
                .context("cannot move the damaged catalogue's log aside, so it was left in place");
        }
    }
    // Only an index of the WAL, rebuilt from it; stale, SQLite resets it.
    let shm = sidecar(db_path, "-shm");
    if shm.exists() && std::fs::rename(&shm, sidecar(&to, "-shm")).is_err() {
        let _ = std::fs::remove_file(&shm);
    }
    Ok(to)
}

/// Where a staged restore waits for the next startup.
fn pending_path(db_path: &Path) -> PathBuf {
    let mut p = db_path.as_os_str().to_os_string();
    p.push(".restore");
    PathBuf::from(p)
}

/// Stages `backup` to replace the live database at the next launch.
///
/// **Deliberately not an immediate swap.** The app holds an open connection and
/// a WAL beside it; replacing the file underneath a live SQLite handle is how
/// you turn a recoverable mistake into a corrupt database. Copying it aside and
/// letting [`take_pending`] do the swap before anything opens it means the
/// dangerous step happens when nothing is attached.
pub fn stage_restore(backup: &Path, db_path: &Path) -> Result<()> {
    if !backup.exists() {
        bail!("that backup no longer exists: {}", backup.display());
    }
    // Open it first: restoring a file that is not a readable database would
    // otherwise only be discovered at the next launch, with the real one
    // already moved aside.
    let probe =
        Connection::open(backup).with_context(|| format!("cannot open {}", backup.display()))?;
    let verdict: String = probe
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .context("backup failed its integrity check")?;
    if verdict != "ok" {
        bail!("backup failed its integrity check: {verdict}");
    }
    drop(probe);

    std::fs::copy(backup, pending_path(db_path))
        .with_context(|| format!("cannot stage {}", backup.display()))?;
    Ok(())
}

/// The staged restore waiting for the next launch, if there is one.
pub fn pending_for(db_path: &Path) -> Option<PathBuf> {
    let p = pending_path(db_path);
    p.exists().then_some(p)
}

/// Discards a staged restore. Returns whether there was one to discard.
///
/// A restore is a decision someone should be able to take back before it
/// happens — the whole point of staging it is that nothing has changed yet.
pub fn cancel_pending(db_path: &Path) -> Result<bool> {
    match pending_for(db_path) {
        Some(p) => {
            std::fs::remove_file(&p)
                .with_context(|| format!("cannot discard the staged restore at {}", p.display()))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Applies a staged restore, if one is waiting. Call before opening anything.
///
/// The live database is kept as an automatic backup rather than deleted — a
/// restore is itself a destructive act, and someone who restores the wrong
/// night's work should not be out of options.
///
/// **Through SQLite, never by moving files.** This used to rename
/// `library.db` into the backups and delete its WAL, on the reasoning that
/// nothing was attached yet so the file was the truth. It is not: an app that
/// did not close cleanly — a crash, `tauri dev` restarting it, `app.restart()`,
/// which exits without closing a connection — leaves committed work in the
/// WAL, and the main file part-way through a checkpoint. The copy kept was then
/// torn ("wrong # of entries in index") and the work in the WAL was deleted.
/// Now the replaced catalogue is opened, which reads its WAL, copied out with
/// `VACUUM INTO`, and the backup is written over it with SQLite's backup API —
/// which also stays correct if the previous instance is still exiting.
///
/// A restore that cannot be applied stays staged for the next launch rather
/// than stopping the app from starting.
pub fn take_pending(db_path: &Path) -> Result<bool> {
    let pending = pending_path(db_path);
    if !pending.exists() {
        return Ok(false);
    }
    if db_path.exists() {
        match restore_over(db_path, &pending) {
            // Applied, so it must not be applied again at the next launch
            // over whatever work is done in between.
            Ok(()) => std::fs::remove_file(&pending).with_context(|| {
                format!("cannot remove the applied restore {}", pending.display())
            })?,
            // Too damaged to copy out or write over: keep it, with its WAL,
            // where the recovery path keeps damaged catalogues.
            Err(e) if crate::db::is_damage(&e) => {
                eprintln!("the catalogue a restore replaces is damaged ({e:#}); keeping it aside");
                set_aside_damaged(db_path)?;
            }
            Err(e) => {
                eprintln!("could not apply the staged restore, it stays staged: {e:#}");
                return Ok(false);
            }
        }
    }
    if pending.exists() {
        std::fs::rename(&pending, db_path).with_context(|| {
            format!(
                "cannot put the restored database in place at {}",
                db_path.display()
            )
        })?;
    }
    let _ = prune(db_path);
    Ok(true)
}

/// Keeps a consistent copy of the catalogue at `db_path`, then writes the
/// database at `pending` over it.
fn restore_over(db_path: &Path, pending: &Path) -> Result<()> {
    let mut conn = crate::db::connect(db_path)?;
    create(&conn, db_path, "auto-replaced")?;
    conn.restore(
        rusqlite::MAIN_DB,
        pending,
        None::<fn(rusqlite::backup::Progress)>,
    )
    .context("cannot write the backup over the catalogue")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny on-disk database with one recognisable row.
    fn seeded(path: &Path, marker: &str) -> Connection {
        let conn = crate::db::open(path).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('marker', ?1)",
            [marker],
        )
        .unwrap();
        conn
    }

    fn marker(path: &Path) -> String {
        let conn = crate::db::connect(path).unwrap();
        conn.query_row("SELECT value FROM settings WHERE key = 'marker'", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn a_backup_captures_work_still_sitting_in_the_wal() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "before");

        // Never checkpointed, so a naive file copy would miss this entirely —
        // which is the whole reason `create` uses VACUUM INTO.
        let out = create(&conn, &db, "manual").unwrap();

        assert_eq!(marker(&out), "before");
    }

    #[test]
    fn restoring_swaps_the_database_and_keeps_the_one_it_replaced() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "monday");
        let backup = create(&conn, &db, "manual").unwrap();

        conn.execute(
            "UPDATE settings SET value = 'tuesday' WHERE key = 'marker'",
            [],
        )
        .unwrap();
        drop(conn);
        assert_eq!(marker(&db), "tuesday");

        stage_restore(&backup, &db).unwrap();
        // Staging alone must not touch the live database — the app is still
        // running at that point.
        assert_eq!(marker(&db), "tuesday");

        assert!(take_pending(&db).unwrap());
        assert_eq!(marker(&db), "monday");

        // Tuesday's work is not gone, just displaced.
        let kept = list(&db)
            .unwrap()
            .into_iter()
            .find(|b| b.filename.contains("replaced"))
            .expect("the replaced database was kept");
        assert_eq!(marker(Path::new(&kept.path)), "tuesday");
    }

    /// Leaves `db` the way an app that did not close cleanly leaves it — killed
    /// by `tauri dev` on a rebuild, crashed, or ended by `app.restart()`, which
    /// exits without closing a single connection: committed work still in the
    /// WAL, and the main file part-way through a checkpoint.
    ///
    /// The main file on its own is then *meant* to be inconsistent — SQLite
    /// reads the rest from the WAL — which is why the WAL is not optional.
    /// Returns how many rows `settings` really holds.
    fn left_by_an_unclean_exit(db: &Path) -> i64 {
        use rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE;
        let writer = crate::db::connect(db).unwrap();
        writer
            .set_db_config(SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
            .unwrap();
        writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        writer
            .execute_batch(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
                 INSERT INTO settings (key, value) SELECT 'k' || i, hex(randomblob(40)) FROM n;",
            )
            .unwrap();

        // A reader part-way through, as a background job or a thumbnail
        // request always is, pins the checkpoint to its snapshot.
        let reader = crate::db::connect(db).unwrap();
        reader
            .set_db_config(SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
            .unwrap();
        reader.execute_batch("BEGIN").unwrap();
        reader
            .query_row("SELECT count(*) FROM settings", [], |r| r.get::<_, i64>(0))
            .unwrap();

        writer
            .execute_batch(
                "UPDATE settings SET value = hex(randomblob(60)) WHERE key LIKE 'k%';
                 UPDATE settings SET value = 'tuesday' WHERE key = 'marker';
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
                 INSERT INTO settings (key, value) SELECT 'j' || i, hex(randomblob(40)) FROM n;",
            )
            .unwrap();
        writer
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
            .unwrap();
        let rows = writer
            .query_row("SELECT count(*) FROM settings", [], |r| r.get(0))
            .unwrap();
        drop(reader);
        drop(writer);

        let mut wal = db.as_os_str().to_os_string();
        wal.push("-wal");
        assert!(
            std::fs::metadata(PathBuf::from(wal)).is_ok_and(|m| m.len() > 0),
            "the work is still in the WAL, as after a crash"
        );
        rows
    }

    /// The bug behind the damaged `library-auto-replaced-*.db` on 2026-09-25:
    /// applying a restore renamed `library.db` away without its WAL and then
    /// deleted the WAL, so the copy it kept "so that work is not lost" was a
    /// torn file — "wrong # of entries in index idx_photos_grid" — and the work
    /// in the WAL was gone for good.
    #[test]
    fn a_restore_keeps_the_work_it_replaces_even_after_an_unclean_exit() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "monday");
        let backup = create(&conn, &db, "manual").unwrap();
        drop(conn);
        stage_restore(&backup, &db).unwrap();

        let rows = left_by_an_unclean_exit(&db);

        assert!(take_pending(&db).unwrap());
        assert_eq!(marker(&db), "monday");

        let kept = list(&db)
            .unwrap()
            .into_iter()
            .find(|b| b.filename.contains("replaced"))
            .expect("the replaced database was kept");
        let kept =
            Connection::open_with_flags(&kept.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let verdict: String = kept
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(verdict, "ok", "the kept copy is a whole database");
        let (kept_rows, kept_marker): (i64, String) = kept
            .query_row(
                "SELECT count(*), (SELECT value FROM settings WHERE key = 'marker') FROM settings",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (kept_rows, kept_marker.as_str()),
            (rows, "tuesday"),
            "including the work that was only in the WAL"
        );
    }

    /// The usual reason to restore is a damaged catalogue, which cannot be
    /// copied out through SQLite. It is kept aside, and nothing of its log may
    /// stay beside the restored file, or it would be replayed over it.
    #[test]
    fn restoring_over_a_damaged_catalogue_keeps_it_aside_with_its_log() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "monday");
        let backup = create(&conn, &db, "manual").unwrap();
        drop(conn);
        stage_restore(&backup, &db).unwrap();
        damage(&db);
        // A leftover log. (Not a valid WAL, so SQLite itself discards it; a
        // real one is checkpointed into the damaged file as it closes.)
        std::fs::write(sidecar(&db, "-wal"), b"the damaged file's log").unwrap();

        assert!(take_pending(&db).unwrap());
        assert!(
            !sidecar(&db, "-wal").exists(),
            "no foreign log beside the restore"
        );
        assert_eq!(marker(&db), "monday");
        let dir = dir_for(&db).join("damaged");
        let kept: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(kept.iter().any(|f| f.ends_with(".db")), "{kept:?}");
        assert!(pending_for(&db).is_none());
    }

    #[test]
    fn a_staged_restore_can_be_taken_back() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "monday");
        let backup = create(&conn, &db, "manual").unwrap();
        conn.execute(
            "UPDATE settings SET value = 'tuesday' WHERE key = 'marker'",
            [],
        )
        .unwrap();
        drop(conn);

        stage_restore(&backup, &db).unwrap();
        assert!(pending_for(&db).is_some());
        assert!(cancel_pending(&db).unwrap());
        assert!(pending_for(&db).is_none());

        // The next launch leaves the database exactly as it was.
        assert!(!take_pending(&db).unwrap());
        assert_eq!(marker(&db), "tuesday");
        assert!(!cancel_pending(&db).unwrap());
    }

    #[test]
    fn nothing_staged_is_not_an_error() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        drop(seeded(&db, "x"));
        assert!(!take_pending(&db).unwrap());
        assert_eq!(marker(&db), "x");
    }

    #[test]
    fn a_file_that_is_not_a_database_is_refused_before_anything_moves() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        drop(seeded(&db, "safe"));
        let junk = tmp.join("junk.db");
        std::fs::write(&junk, b"not a database").unwrap();

        assert!(stage_restore(&junk, &db).is_err());
        assert!(!take_pending(&db).unwrap());
        assert_eq!(marker(&db), "safe");
    }

    /// A restore names a backup, and only one in the backups folder is found.
    #[test]
    fn a_backup_is_found_by_its_name_in_the_backups_folder_only() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "x");
        let made = create(&conn, &db, "manual").unwrap();
        drop(conn);
        let name = made.file_name().unwrap().to_str().unwrap();
        assert_eq!(named(&db, name).unwrap(), made);
        std::fs::write(tmp.join("other.db"), FAKE_DB).unwrap();
        let (db_path, made_path) = (db.to_string_lossy(), made.to_string_lossy());
        for outside in [
            "library.db",
            "../library.db",
            "..\\other.db",
            "other.db",
            db_path.as_ref(),
            made_path.as_ref(),
            "",
        ] {
            assert!(named(&db, outside).is_err(), "{outside}");
        }
    }

    #[test]
    fn pruning_keeps_the_newest_automatics_and_every_manual() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "x");
        let dir = dir_for(&db);
        std::fs::create_dir_all(&dir).unwrap();
        drop(conn);

        // Hand-written rather than via `create`, so the timestamps are distinct
        // without sleeping for five seconds.
        for i in 0..5 {
            std::fs::write(
                dir.join(format!("library-auto-before-recycle-{i}.db")),
                FAKE_DB,
            )
            .unwrap();
        }
        for i in 0..2 {
            std::fs::write(dir.join(format!("library-manual-{i}.db")), b"x").unwrap();
        }

        prune(&db).unwrap();
        let left = list(&db).unwrap();
        assert_eq!(left.iter().filter(|b| b.automatic).count(), KEEP_AUTOMATIC);
        assert_eq!(left.iter().filter(|b| !b.automatic).count(), 2);
    }

    /// Routine binning can't push out the copy taken before an upgrade, or
    /// the catalogue a restore replaced: each keeps a pool of its own.
    #[test]
    fn binning_never_prunes_the_copy_taken_before_an_upgrade() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "x");
        let dir = dir_for(&db);
        std::fs::create_dir_all(&dir).unwrap();
        drop(conn);
        let write = |name: String| std::fs::write(dir.join(name), FAKE_DB).unwrap();
        write("library-auto-v38-1.db".into());
        write("library-auto-replaced-1.db".into());
        for i in 0..6 {
            write(format!("library-auto-before-recycle-{i}.db"));
        }
        for i in 0..4 {
            write(format!("library-auto-v{}-{i}.db", 40 + i));
        }

        prune(&db).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|b| b.filename).collect();
        let count = |prefix: &str| names.iter().filter(|n| n.starts_with(prefix)).count();
        assert_eq!(count("library-auto-before-recycle"), KEEP_AUTOMATIC);
        assert_eq!(count("library-auto-replaced"), 1);
        assert_eq!(count("library-auto-v"), 2, "{names:?}");
    }

    /// Daily backups have a pool of their own: a week of them never pushes
    /// out the copy taken before a migration.
    #[test]
    fn daily_backups_are_pruned_apart_from_the_others() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "x");
        let dir = dir_for(&db);
        std::fs::create_dir_all(&dir).unwrap();
        drop(conn);
        for i in 0..10 {
            std::fs::write(dir.join(format!("{DAILY}-{i}.db")), FAKE_DB).unwrap();
        }
        std::fs::write(dir.join("library-auto-v24-1.db"), FAKE_DB).unwrap();

        prune(&db).unwrap();
        let left = list(&db).unwrap();
        let daily = left
            .iter()
            .filter(|b| b.filename.starts_with(DAILY))
            .count();
        assert_eq!(daily, KEEP_DAILY);
        assert!(left.iter().any(|b| b.filename == "library-auto-v24-1.db"));
        assert!(left.iter().all(|b| b.automatic));
    }

    #[test]
    fn a_daily_backup_is_taken_once_a_day() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "x");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let first = daily_if_due(&conn, &db, now)
            .unwrap()
            .expect("none yet: one is due");
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(DAILY));
        // Opened again the same day: nothing.
        assert!(daily_if_due(&conn, &db, now + 60).unwrap().is_none());
        // The next day: due again. Filenames carry the real second, so wait
        // one out rather than collide with the first.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(daily_if_due(&conn, &db, now + DAILY_EVERY_SECS + 1)
            .unwrap()
            .is_some());
        drop(conn);
    }

    /// Declared before any connection in each test, so it drops after them —
    /// Windows will not delete a database that is still open.
    fn tempdir() -> crate::test_support::Scratch {
        crate::test_support::Scratch::new("backup")
    }

    /// Enough of an SQLite file to pass [`looks_like_a_database`].
    const FAKE_DB: &[u8] = b"SQLite format 3\0 and the rest";

    /// Failed backups from an older build were empty files. They must not
    /// count as backups, or a few of them push the last good copy out.
    #[test]
    fn empty_failed_backups_never_push_a_good_one_out() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        drop(seeded(&db, "x"));
        let dir = dir_for(&db);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("library-auto-good-1.db"), FAKE_DB).unwrap();
        // Newer than the good one, as they were in the incident.
        for i in 2..6 {
            let empty = dir.join(format!("library-auto-before-folder-removal-{i}.db"));
            std::fs::write(&empty, b"").unwrap();
            let t = std::time::SystemTime::now() + std::time::Duration::from_secs(i);
            std::fs::File::options()
                .write(true)
                .open(&empty)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        prune(&db).unwrap();
        let left: Vec<String> = list(&db).unwrap().into_iter().map(|b| b.filename).collect();
        assert_eq!(left, ["library-auto-good-1.db"]);
    }

    /// Damage like the real incident's: the schema's own pages overwritten.
    fn damage(db: &Path) {
        let mut bytes = std::fs::read(db).unwrap();
        // Keep the 100-byte file header (so it still opens, as the real one
        // did) and overwrite the rest of page 1, the schema's root.
        let page = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
        for b in &mut bytes[100..page] {
            *b = 0xA5;
        }
        std::fs::write(db, bytes).unwrap();
    }

    #[test]
    fn a_damaged_catalogue_is_replaced_by_its_newest_good_backup() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        let conn = seeded(&db, "before");
        create(&conn, &db, "manual").unwrap();
        conn.execute(
            "UPDATE settings SET value = 'after' WHERE key = 'marker'",
            [],
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        drop(conn);
        damage(&db);
        assert!(crate::db::open(&db)
            .and_then(|c| {
                c.query_row("SELECT count(*) FROM photos", [], |r| r.get::<_, i64>(0))?;
                Ok(())
            })
            .is_err());

        let (conn, recovery) = crate::db::open_or_recover(&db).unwrap();
        drop(conn);
        let recovery = recovery.expect("it was damaged");
        assert!(recovery.restored.is_some());
        assert_eq!(marker(&db), "before", "the backup's contents");
        // The damaged copy is kept, but not offered as a backup.
        assert!(Path::new(&recovery.damaged_copy).exists());
        assert!(list(&db)
            .unwrap()
            .iter()
            .all(|b| !b.filename.contains("damaged")));

        // A healthy catalogue is left alone.
        let (_, again) = crate::db::open_or_recover(&db).unwrap();
        assert!(again.is_none());
    }

    #[test]
    fn with_no_good_backup_it_starts_empty_and_keeps_the_damaged_copy() {
        let tmp = tempdir();
        let db = tmp.join("library.db");
        drop(seeded(&db, "only copy"));
        damage(&db);
        let (conn, recovery) = crate::db::open_or_recover(&db).unwrap();
        let recovery = recovery.unwrap();
        assert!(recovery.restored.is_none());
        let photos: i64 = conn
            .query_row("SELECT count(*) FROM photos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(photos, 0);
        assert!(Path::new(&recovery.damaged_copy).exists());
    }
}
