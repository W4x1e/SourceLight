//! What the phone keeps about its own photos (docs/MOBILE.md section 4):
//! their edits, as the desktop writes them (`EditStack` JSON), never the
//! photo; and which are waiting to be added to the library, or have been,
//! and as which library photo.
//!
//! In a database of their own (`device.db`), not the library copy's: that one
//! is thrown away and copied again whenever the computer's catalogue changes,
//! and these belong to the phone.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

/// A phone photo for the library: `(photo, library_id, error)`. No library
/// id and no error: still waiting.
pub type AddRow = (i64, Option<i64>, Option<String>);

pub struct Edits {
    conn: Connection,
}

impl Edits {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS edits (
                photo      INTEGER PRIMARY KEY,
                modified   INTEGER NOT NULL,
                stack      TEXT NOT NULL,
                updated_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS adds (
                photo      INTEGER PRIMARY KEY,
                queued_ms  INTEGER NOT NULL,
                library_id INTEGER,
                error      TEXT
            );",
        )?;
        Ok(Edits { conn })
    }

    /// Photo `photo`'s edit settings, if it has any.
    pub fn get(&self, photo: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT stack FROM edits WHERE photo = ?1", [photo], |r| {
                r.get(0)
            })
            .optional()
    }

    /// Keeps `stack` as photo `photo`'s edits, or with `None` (edited back to
    /// nothing) forgets them. `modified` is the photo's own modified time when
    /// it was edited, for telling later whether it changed underneath.
    pub fn put(&self, photo: i64, modified: i64, stack: Option<&str>) -> rusqlite::Result<()> {
        match stack {
            Some(stack) => self.conn.execute(
                "INSERT INTO edits (photo, modified, stack, updated_ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(photo) DO UPDATE SET
                    modified = excluded.modified,
                    stack = excluded.stack,
                    updated_ms = excluded.updated_ms",
                params![photo, modified, stack, now_ms()],
            )?,
            None => self
                .conn
                .execute("DELETE FROM edits WHERE photo = ?1", [photo])?,
        };
        Ok(())
    }

    /// Every photo with edits, for the grid's Edited marks.
    pub fn edited(&self) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT photo FROM edits ORDER BY photo")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }
}

impl Edits {
    /// Puts these photos in line to be added to the library. One already
    /// added, or already waiting, is left as it is; one that failed is tried
    /// again. Returns how many are newly waiting.
    pub fn queue_adds(&self, photos: &[i64]) -> rusqlite::Result<u32> {
        let mut queued = 0;
        for photo in photos {
            queued += self.conn.execute(
                "INSERT INTO adds (photo, queued_ms) VALUES (?1, ?2)
                 ON CONFLICT(photo) DO UPDATE SET error = NULL, queued_ms = excluded.queued_ms
                 WHERE adds.library_id IS NULL AND adds.error IS NOT NULL",
                params![photo, now_ms()],
            )? as u32;
        }
        Ok(queued)
    }

    /// The photos waiting to go, oldest first.
    pub fn waiting(&self) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT photo FROM adds WHERE library_id IS NULL AND error IS NULL ORDER BY queued_ms, photo",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// Photo `photo` is in the library now, as `library_id`.
    pub fn added(&self, photo: i64, library_id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE adds SET library_id = ?2, error = NULL WHERE photo = ?1",
            params![photo, library_id],
        )?;
        Ok(())
    }

    /// Photo `photo` could not be added, and why; it waits for the person to
    /// try again rather than failing on every sync.
    pub fn add_failed(&self, photo: i64, error: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE adds SET error = ?2 WHERE photo = ?1",
            params![photo, error],
        )?;
        Ok(())
    }

    /// Every photo in line or added.
    pub fn adds(&self) -> rusqlite::Result<Vec<AddRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT photo, library_id, error FROM adds ORDER BY photo")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Edits {
        Edits::open(Path::new(":memory:")).unwrap()
    }

    /// Add to library: in line, sent, or failed and waiting to be tried again.
    #[test]
    fn adds_wait_go_and_fail_once() {
        let edits = store();
        assert_eq!(edits.queue_adds(&[5, 3]).unwrap(), 2);
        // Asking again changes nothing while they wait.
        assert_eq!(edits.queue_adds(&[3]).unwrap(), 0);
        let mut waiting = edits.waiting().unwrap();
        waiting.sort();
        assert_eq!(waiting, vec![3, 5]);

        edits.added(3, 812).unwrap();
        edits.add_failed(5, "too big").unwrap();
        assert!(edits.waiting().unwrap().is_empty());
        assert_eq!(
            edits.adds().unwrap(),
            vec![(3, Some(812), None), (5, None, Some("too big".into()))]
        );
        // An added photo is not sent again; a failed one is, when asked.
        assert_eq!(edits.queue_adds(&[3, 5]).unwrap(), 1);
        assert_eq!(edits.waiting().unwrap(), vec![5]);
    }

    #[test]
    fn edits_are_kept_replaced_and_forgotten() {
        let edits = store();
        assert_eq!(edits.get(41).unwrap(), None);
        edits.put(41, 100, Some(r#"{"exposure":0.5}"#)).unwrap();
        edits.put(7, 100, Some(r#"{"contrast":0.2}"#)).unwrap();
        assert_eq!(
            edits.get(41).unwrap().as_deref(),
            Some(r#"{"exposure":0.5}"#)
        );
        edits.put(41, 101, Some(r#"{"exposure":1.0}"#)).unwrap();
        assert_eq!(
            edits.get(41).unwrap().as_deref(),
            Some(r#"{"exposure":1.0}"#)
        );
        assert_eq!(edits.edited().unwrap(), vec![7, 41]);
        // Edited back to nothing: no longer marked as edited.
        edits.put(41, 101, None).unwrap();
        assert_eq!(edits.get(41).unwrap(), None);
        assert_eq!(edits.edited().unwrap(), vec![7]);
    }
}
