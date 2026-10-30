//! The phone's copy of the library (docs/MOBILE.md, "What syncs"): the small
//! fields of every photo, the cursor into the desktop's change feed, and the
//! decisions made here that the desktop has not had yet.
//!
//! The desktop is the source of truth. This copy is rebuilt from scratch when
//! the desktop's catalogue id changes (another library) or its cursor goes
//! backwards (a restored backup).

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sl_link::protocol::{EditOn, PhotoRow, Verdict};

pub struct Replica {
    conn: Connection,
}

/// A grid tile: what the grid and culling need, in display order: id, flag,
/// rating, whether it is edited, where it can be edited (`EDIT_*`), the
/// computer's tag for its edits (for the loupe's edited preview), and when it
/// was taken (for the grid's month headings and the loupe).
pub type Tile = (i64, i8, u8, bool, u8, Option<String>, Option<String>);

/// Where a library photo can be edited, as kept (sl-protocol's `EditOn`):
/// not said, here too, a raw (on the computer), or on the computer only.
pub const EDIT_UNKNOWN: u8 = 0;
pub const EDIT_PHONE: u8 = 1;
pub const EDIT_RAW: u8 = 2;
pub const EDIT_COMPUTER: u8 = 3;

pub fn edit_on_code(on: EditOn) -> u8 {
    match on {
        EditOn::Phone => EDIT_PHONE,
        EditOn::Raw => EDIT_RAW,
        EditOn::Computer => EDIT_COMPUTER,
        EditOn::Unknown => EDIT_UNKNOWN,
    }
}

/// A library photo's edits as the phone has them (docs/MOBILE.md section 4,
/// step 6): `base` as the computer sent them, with their tag, and `stack`,
/// the edit now, `pending` until the computer has it. Edits are the
/// desktop's JSON; none is no edit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibEdit {
    pub base: Option<String>,
    pub base_tag: Option<String>,
    pub stack: Option<String>,
    pub pending: bool,
}

/// An edit waiting for the computer: photo, base, edit.
pub type PendingEdit = (i64, Option<String>, Option<String>);

/// A shoot's GPS track recorded on this phone.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    /// The GPX file.
    #[serde(skip)]
    pub file: String,
    pub name: String,
    pub started_ms: i64,
    pub ended_ms: i64,
    pub points: u32,
    /// The paired computer has it.
    pub sent: bool,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS photos (
        id       INTEGER PRIMARY KEY,
        filename TEXT NOT NULL,
        captured TEXT,
        width    INTEGER NOT NULL,
        height   INTEGER NOT NULL,
        rating   INTEGER NOT NULL,
        flag     INTEGER NOT NULL,
        -- The computer's tag for its edits (none when unedited), and where
        -- it can be edited (EDIT_*).
        edit_tag TEXT,
        edit_on  INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_photos_captured ON photos(captured DESC, id DESC);
    CREATE TABLE IF NOT EXISTS meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    -- Decisions made on the phone, oldest first, until the desktop has them.
    CREATE TABLE IF NOT EXISTS pending (
        seq    INTEGER PRIMARY KEY AUTOINCREMENT,
        id     INTEGER NOT NULL,
        flag   INTEGER,
        rating INTEGER
    );
    -- GPS tracks recorded here. They belong to the phone, not to a
    -- catalogue: forgetting a computer keeps them, for the next one.
    CREATE TABLE IF NOT EXISTS tracks (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        file       TEXT NOT NULL,
        name       TEXT NOT NULL,
        started_ms INTEGER NOT NULL,
        ended_ms   INTEGER NOT NULL,
        points     INTEGER NOT NULL,
        sent       INTEGER NOT NULL DEFAULT 0
    );
    -- Library photos edited here: see LibEdit.
    CREATE TABLE IF NOT EXISTS lib_edits (
        id       INTEGER PRIMARY KEY,
        base     TEXT,
        base_tag TEXT,
        stack    TEXT,
        pending  INTEGER NOT NULL DEFAULT 0
    );
";

/// Brings a copy made by an older build up to this one's tables.
fn upgrade(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        // Editing library photos (step 6). A copy from before it has no
        // word of anyone's edits, so it fetches every photo again.
        let old = !conn
            .prepare("SELECT 1 FROM pragma_table_info('photos') WHERE name = 'edit_on'")?
            .exists([])?;
        if old {
            conn.execute_batch(
                "ALTER TABLE photos ADD COLUMN edit_tag TEXT;
                 ALTER TABLE photos ADD COLUMN edit_on INTEGER NOT NULL DEFAULT 0;
                 DELETE FROM meta WHERE key = 'cursor';",
            )?;
        }
        conn.pragma_update(None, "user_version", 1)?;
    }
    Ok(())
}

impl Replica {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        upgrade(&conn)?;
        Ok(Replica { conn })
    }

    #[cfg(test)]
    fn memory() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        upgrade(&conn).unwrap();
        Replica { conn }
    }

    fn meta(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    fn set_meta(tx: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// The catalogue this copy is of, if any.
    pub fn catalogue(&self) -> rusqlite::Result<Option<String>> {
        self.meta("catalogue")
    }

    /// Where to ask the change feed from: -1 for everything.
    pub fn cursor(&self) -> rusqlite::Result<i64> {
        Ok(self
            .meta("cursor")?
            .and_then(|c| c.parse().ok())
            .unwrap_or(-1))
    }

    /// When the phone last synced with the computer, in ms since the epoch.
    pub fn last_synced(&self) -> rusqlite::Result<Option<i64>> {
        Ok(self.meta("synced_ms")?.and_then(|v| v.parse().ok()))
    }

    pub fn set_last_synced(&mut self, ms: i64) -> rusqlite::Result<()> {
        Self::set_meta(&self.conn, "synced_ms", &ms.to_string())
    }

    /// Forgets everything, ready to copy `catalogue` afresh. Decisions not yet
    /// sent go too: they were about photos of another catalogue, or of a
    /// state the desktop has gone back from.
    pub fn reset(&mut self, catalogue: &str) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM photos; DELETE FROM pending; DELETE FROM meta; DELETE FROM lib_edits;",
        )?;
        Self::set_meta(&tx, "catalogue", catalogue)?;
        tx.commit()
    }

    /// Applies one page of the change feed. Local decisions not yet sent keep
    /// their value: the desktop's echo of an older state must not undo a tap
    /// made since.
    pub fn apply(&mut self, rows: &[PhotoRow], removed: &[i64], next: i64) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare(
                "INSERT INTO photos (id, filename, captured, width, height, rating, flag,
                                     edit_tag, edit_on)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    filename = excluded.filename, captured = excluded.captured,
                    width = excluded.width, height = excluded.height,
                    edit_tag = excluded.edit_tag, edit_on = excluded.edit_on,
                    rating = CASE WHEN EXISTS (SELECT 1 FROM pending p
                                 WHERE p.id = excluded.id AND p.rating IS NOT NULL)
                             THEN photos.rating ELSE excluded.rating END,
                    flag = CASE WHEN EXISTS (SELECT 1 FROM pending p
                               WHERE p.id = excluded.id AND p.flag IS NOT NULL)
                           THEN photos.flag ELSE excluded.flag END",
            )?;
            for r in rows {
                upsert.execute(params![
                    r.id,
                    r.filename,
                    r.captured,
                    r.width,
                    r.height,
                    r.rating,
                    r.flag,
                    r.edit_tag,
                    edit_on_code(r.edit_on)
                ])?;
            }
            let mut remove = tx.prepare("DELETE FROM photos WHERE id = ?1")?;
            // A photo that leaves takes its settled edit with it; an edit
            // still to send stays, as the computer says what became of the
            // photo when it goes. The feed also lists copies as removals,
            // which the phone never had: an edit it carries on making to
            // one (see `lib_moved`) is not touched.
            let mut forget = tx.prepare("DELETE FROM lib_edits WHERE id = ?1 AND pending = 0")?;
            for id in removed {
                if remove.execute([id])? > 0 {
                    forget.execute([id])?;
                }
            }
        }
        Self::set_meta(&tx, "cursor", &next.to_string())?;
        tx.commit()
    }

    pub fn count(&self) -> rusqlite::Result<u32> {
        self.conn
            .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
    }

    /// Every photo, newest first, as the desktop's grid orders them. An
    /// edit made here and not yet sent counts over the computer's word.
    pub fn tiles(&self) -> rusqlite::Result<Vec<Tile>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.flag, p.rating,
                    COALESCE((SELECT e.stack IS NOT NULL FROM lib_edits e
                              WHERE e.id = p.id AND e.pending = 1),
                             p.edit_tag IS NOT NULL),
                    p.edit_on, p.edit_tag, p.captured
             FROM photos p ORDER BY p.captured DESC, p.id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?;
        rows.collect()
    }

    /// Photo `id`'s file name, if it is here: for the loupe.
    pub fn filename(&self, id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT filename FROM photos WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
    }

    /// Photo `id`'s edit tag and where it can be edited, if it is here.
    pub fn edit_state(&self, id: i64) -> rusqlite::Result<Option<(Option<String>, u8)>> {
        self.conn
            .query_row(
                "SELECT edit_tag, edit_on FROM photos WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
    }

    pub fn lib_edit(&self, id: i64) -> rusqlite::Result<Option<LibEdit>> {
        self.conn
            .query_row(
                "SELECT base, base_tag, stack, pending FROM lib_edits WHERE id = ?1",
                [id],
                |r| {
                    Ok(LibEdit {
                        base: r.get(0)?,
                        base_tag: r.get(1)?,
                        stack: r.get(2)?,
                        pending: r.get(3)?,
                    })
                },
            )
            .optional()
    }

    /// Photo `id`'s edits as the computer has them now: `base`, tagged
    /// `tag`, and edited here only from now. An edit waiting to be sent is
    /// left as it is.
    pub fn lib_fetched(
        &mut self,
        id: i64,
        base: Option<&str>,
        tag: Option<&str>,
        edit_on: u8,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO lib_edits (id, base, base_tag, stack, pending) VALUES (?1, ?2, ?3, ?2, 0)
             ON CONFLICT(id) DO UPDATE SET base = excluded.base, base_tag = excluded.base_tag,
                                           stack = excluded.stack
             WHERE lib_edits.pending = 0",
            params![id, base, tag],
        )?;
        tx.execute(
            "UPDATE photos SET edit_tag = ?2, edit_on = ?3 WHERE id = ?1",
            params![id, tag, edit_on],
        )?;
        tx.commit()
    }

    /// The edit made here to photo `id` now; `pending` when it is not the
    /// computer's already.
    pub fn lib_keep(
        &mut self,
        id: i64,
        stack: Option<&str>,
        pending: bool,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO lib_edits (id, stack, pending) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET stack = excluded.stack, pending = excluded.pending",
            params![id, stack, pending],
        )?;
        Ok(())
    }

    /// Edits waiting for the computer, by photo.
    pub fn lib_pending(&self) -> rusqlite::Result<Vec<PendingEdit>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, base, stack FROM lib_edits WHERE pending = 1 ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }

    /// The computer kept `sent` on photo `id`, as edits tagged `tag`. An edit
    /// made since it went is still to send, from there.
    pub fn lib_sent(
        &mut self,
        id: i64,
        sent: Option<&str>,
        tag: Option<&str>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE lib_edits SET base = ?2, base_tag = ?3,
                    pending = CASE WHEN stack IS ?2 THEN 0 ELSE 1 END
             WHERE id = ?1",
            params![id, sent, tag],
        )?;
        tx.execute(
            "UPDATE photos SET edit_tag = ?2 WHERE id = ?1",
            params![id, tag],
        )?;
        tx.commit()
    }

    /// The computer put `sent` on `copy`, a new copy of photo `id`, whose own
    /// edits it kept: the phone fetches those when the photo is next edited.
    /// The phone's edit now belongs to the copy, kept as sent with the copy's
    /// tag; an edit made since it went is still to send, to the copy.
    pub fn lib_moved(
        &mut self,
        id: i64,
        copy: i64,
        sent: Option<&str>,
        tag: Option<&str>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO lib_edits (id, base, base_tag, stack, pending)
             SELECT ?2, ?3, ?4, stack, CASE WHEN stack IS ?3 THEN 0 ELSE 1 END
             FROM lib_edits WHERE id = ?1",
            params![id, copy, sent, tag],
        )?;
        tx.execute("DELETE FROM lib_edits WHERE id = ?1", [id])?;
        tx.commit()
    }

    pub fn lib_forget(&mut self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM lib_edits WHERE id = ?1", [id])?;
        Ok(())
    }

    /// A pick, a reject, or clearing either, made here.
    pub fn set_flag(&mut self, id: i64, flag: i8) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE photos SET flag = ?2 WHERE id = ?1",
            params![id, flag],
        )?;
        tx.execute(
            "INSERT INTO pending (id, flag) VALUES (?1, ?2)",
            params![id, flag],
        )?;
        tx.commit()
    }

    /// A rating, 0 to 5, made here.
    pub fn set_rating(&mut self, id: i64, rating: u8) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE photos SET rating = ?2 WHERE id = ?1",
            params![id, rating],
        )?;
        tx.execute(
            "INSERT INTO pending (id, rating) VALUES (?1, ?2)",
            params![id, rating],
        )?;
        tx.commit()
    }

    /// Decisions the desktop has not had, oldest first, with the last one's
    /// sequence so exactly those can be cleared once sent.
    pub fn pending(&self) -> rusqlite::Result<(Vec<Verdict>, i64)> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, id, flag, rating FROM pending ORDER BY seq")?;
        let mut last = 0;
        let mut verdicts = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            last = r.get(0)?;
            verdicts.push(Verdict {
                id: r.get(1)?,
                flag: r.get(2)?,
                rating: r.get(3)?,
            });
        }
        Ok((verdicts, last))
    }

    pub fn add_track(
        &mut self,
        file: &str,
        name: &str,
        started_ms: i64,
        ended_ms: i64,
        points: u32,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO tracks (file, name, started_ms, ended_ms, points) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![file, name, started_ms, ended_ms, points],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Every track, newest first.
    pub fn tracks(&self) -> rusqlite::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file, name, started_ms, ended_ms, points, sent FROM tracks
             ORDER BY started_ms DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Track {
                id: r.get(0)?,
                file: r.get(1)?,
                name: r.get(2)?,
                started_ms: r.get(3)?,
                ended_ms: r.get(4)?,
                points: r.get(5)?,
                sent: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn track_sent(&mut self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE tracks SET sent = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Clears decisions up to `seq`, once the desktop has applied them.
    pub fn sent(&mut self, seq: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM pending WHERE seq <= ?1", [seq])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tiles without their capture time, which the rows set.
    fn tiles6(r: &Replica) -> Vec<(i64, i8, u8, bool, u8, Option<String>)> {
        r.tiles()
            .unwrap()
            .into_iter()
            .map(|t| (t.0, t.1, t.2, t.3, t.4, t.5))
            .collect()
    }

    fn row(id: i64, captured: &str, flag: i8) -> PhotoRow {
        PhotoRow {
            id,
            filename: format!("{id}.ARW"),
            captured: Some(captured.into()),
            width: 6000,
            height: 4000,
            rating: 0,
            flag,
            edit_tag: None,
            edit_on: EditOn::Phone,
        }
    }

    #[test]
    fn pages_apply_and_order_newest_first() {
        let mut r = Replica::memory();
        assert_eq!(r.cursor().unwrap(), -1);
        r.reset("cat").unwrap();
        r.apply(
            &[
                row(1, "2026-07-01 10:00:00", 0),
                row(2, "2026-09-01 10:00:00", 1),
            ],
            &[],
            2,
        )
        .unwrap();
        r.apply(&[row(3, "2026-08-01 10:00:00", 0)], &[1], 5)
            .unwrap();
        assert_eq!(r.cursor().unwrap(), 5);
        assert_eq!(r.catalogue().unwrap().as_deref(), Some("cat"));
        assert_eq!(
            tiles6(&r),
            [
                (2, 1, 0, false, EDIT_PHONE, None),
                (3, 0, 0, false, EDIT_PHONE, None)
            ]
        );
    }

    /// A tap on the phone survives the desktop echoing the state from before
    /// it, until the desktop has the tap.
    #[test]
    fn an_unsent_decision_outlives_an_older_echo() {
        let mut r = Replica::memory();
        r.reset("cat").unwrap();
        r.apply(&[row(1, "2026-07-01 10:00:00", 0)], &[], 1)
            .unwrap();
        r.set_flag(1, -1).unwrap();
        r.apply(&[row(1, "2026-07-01 10:00:00", 0)], &[], 2)
            .unwrap();
        assert_eq!(tiles6(&r), [(1, -1, 0, false, EDIT_PHONE, None)]);

        let (verdicts, last) = r.pending().unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].flag, Some(-1));
        r.sent(last).unwrap();
        assert!(r.pending().unwrap().0.is_empty());
        // Once sent, the desktop's word is final again.
        r.apply(&[row(1, "2026-07-01 10:00:00", 1)], &[], 3)
            .unwrap();
        assert_eq!(tiles6(&r), [(1, 1, 0, false, EDIT_PHONE, None)]);
    }

    /// A library edit: fetched, changed here, sent, changed again while it
    /// went, and the grid's mark following along.
    #[test]
    fn library_edits_wait_go_and_keep_what_came_after() {
        let mut r = Replica::memory();
        r.reset("cat").unwrap();
        let mut edited = row(1, "2026-07-01 10:00:00", 0);
        edited.edit_tag = Some("t1".into());
        r.apply(&[edited], &[], 1).unwrap();
        assert!(r.tiles().unwrap()[0].3);

        r.lib_fetched(1, Some("{\"exposure\":1}"), Some("t1"), EDIT_PHONE)
            .unwrap();
        let kept = r.lib_edit(1).unwrap().unwrap();
        assert_eq!(
            (kept.base_tag.as_deref(), kept.pending),
            (Some("t1"), false)
        );

        // Every edit taken off here: unedited in the grid, and waiting.
        r.lib_keep(1, None, true).unwrap();
        assert!(!r.tiles().unwrap()[0].3);
        assert_eq!(
            r.lib_pending().unwrap(),
            [(1, Some("{\"exposure\":1}".into()), None)]
        );
        // A fetch meanwhile leaves the waiting edit alone.
        r.lib_fetched(1, Some("{}"), Some("t9"), EDIT_PHONE)
            .unwrap();
        assert_eq!(r.lib_pending().unwrap().len(), 1);

        // Sent; then changed again while the first went.
        r.lib_keep(1, Some("{\"warmth\":1}"), true).unwrap();
        r.lib_sent(1, None, None).unwrap();
        let kept = r.lib_edit(1).unwrap().unwrap();
        assert_eq!((kept.base, kept.pending), (None, true));
        r.lib_sent(1, Some("{\"warmth\":1}"), Some("t2")).unwrap();
        assert!(r.lib_pending().unwrap().is_empty());
        assert_eq!(
            r.edit_state(1).unwrap(),
            Some((Some("t2".into()), EDIT_PHONE))
        );

        // Kept on a copy, 7: the edit moves there, and the photo's own are
        // fetched next time. A change made while it went goes to the copy.
        r.lib_keep(1, Some("{\"tint\":1}"), true).unwrap();
        r.lib_keep(1, Some("{\"tint\":2}"), true).unwrap();
        r.lib_moved(1, 7, Some("{\"tint\":1}"), Some("c7")).unwrap();
        assert_eq!(r.lib_edit(1).unwrap(), None);
        assert_eq!(
            r.lib_pending().unwrap(),
            [(7, Some("{\"tint\":1}".into()), Some("{\"tint\":2}".into()))]
        );
        // The feed lists the copy as a removal; its edit stays, settled.
        r.lib_sent(7, Some("{\"tint\":2}"), Some("c8")).unwrap();
        r.apply(&[], &[7], 2).unwrap();
        assert_eq!(
            r.lib_edit(7).unwrap().unwrap().base_tag.as_deref(),
            Some("c8")
        );

        // A photo gone from the library takes its settled edit with it.
        r.lib_fetched(1, None, None, EDIT_PHONE).unwrap();
        r.apply(&[], &[1], 2).unwrap();
        assert_eq!(r.lib_edit(1).unwrap(), None);
    }

    /// A copy from before library editing gains the columns and fetches
    /// everything again; decisions waiting to go stay.
    #[test]
    fn an_older_copy_is_upgraded_and_fetched_again() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE photos (id INTEGER PRIMARY KEY, filename TEXT NOT NULL, captured TEXT,
                 width INTEGER NOT NULL, height INTEGER NOT NULL, rating INTEGER NOT NULL,
                 flag INTEGER NOT NULL);
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO photos VALUES (1, 'a.jpg', NULL, 4, 3, 0, 0);
             INSERT INTO meta VALUES ('catalogue', 'cat'), ('cursor', '40');",
        )
        .unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute("INSERT INTO pending (id, flag) VALUES (1, 1)", [])
            .unwrap();
        upgrade(&conn).unwrap();
        let r = Replica { conn };
        assert_eq!(r.cursor().unwrap(), -1);
        assert_eq!(r.catalogue().unwrap().as_deref(), Some("cat"));
        assert_eq!(tiles6(&r), [(1, 0, 0, false, EDIT_UNKNOWN, None)]);
        assert_eq!(r.pending().unwrap().0.len(), 1);
        // Once is enough.
        upgrade(&r.conn).unwrap();
    }

    /// A rating made here goes as a decision, and outlives an older echo as
    /// a pick does.
    #[test]
    fn a_rating_waits_and_outlives_an_older_echo() {
        let mut r = Replica::memory();
        r.reset("cat").unwrap();
        r.apply(&[row(1, "2026-07-01 10:00:00", 0)], &[], 1)
            .unwrap();
        r.set_rating(1, 4).unwrap();
        r.apply(&[row(1, "2026-07-01 10:00:00", 0)], &[], 2)
            .unwrap();
        assert_eq!(r.tiles().unwrap()[0].2, 4);
        let (verdicts, _) = r.pending().unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!((verdicts[0].flag, verdicts[0].rating), (None, Some(4)));
    }

    #[test]
    fn the_last_sync_is_kept() {
        let mut r = Replica::memory();
        assert_eq!(r.last_synced().unwrap(), None);
        r.set_last_synced(1_791_000_000_000).unwrap();
        assert_eq!(r.last_synced().unwrap(), Some(1_791_000_000_000));
    }

    #[test]
    fn tracks_outlive_a_reset_and_are_marked_sent() {
        let mut r = Replica::memory();
        let a = r.add_track("a.gpx", "Morning", 1_000, 2_000, 12).unwrap();
        r.add_track("b.gpx", "Evening", 5_000, 9_000, 40).unwrap();
        r.reset("another library").unwrap();
        let tracks = r.tracks().unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "Evening", "newest first");
        r.track_sent(a).unwrap();
        let sent: Vec<bool> = r.tracks().unwrap().iter().map(|t| t.sent).collect();
        assert_eq!(sent, [false, true]);
    }

    #[test]
    fn reset_starts_afresh() {
        let mut r = Replica::memory();
        r.reset("one").unwrap();
        r.apply(&[row(1, "2026-07-01 10:00:00", 0)], &[], 9)
            .unwrap();
        r.set_flag(1, 1).unwrap();
        r.reset("two").unwrap();
        assert_eq!(r.count().unwrap(), 0);
        assert_eq!(r.cursor().unwrap(), -1);
        assert!(r.pending().unwrap().0.is_empty());
        assert_eq!(r.catalogue().unwrap().as_deref(), Some("two"));
    }
}
