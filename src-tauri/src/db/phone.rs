//! The phone link (docs/MOBILE.md): paired phones, the change feed they sync
//! from, exports waiting for them, and the photos they sent.

use super::*;

/// One page of the change feed (migration 49), for a phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePage {
    pub rows: Vec<crate::phone_link::PhoneRow>,
    /// Deleted, missing, or a copy (phones show originals only).
    pub removed: Vec<i64>,
    /// The cursor to ask from next.
    pub next: i64,
    pub more: bool,
    /// The clock now: lower than a phone's cursor means a restored backup.
    pub head: i64,
}

/// The photos changed after `since`, oldest change first, at most `limit`.
pub fn changes_since(conn: &Connection, since: i64, limit: u32) -> Result<ChangePage> {
    let limit = limit.clamp(1, 5000);
    let head: i64 = conn.query_row("SELECT seq FROM sync_clock WHERE id = 1", [], |r| r.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT seq, id, gone, filename, capture_date, width, height, orientation,
                rating, flag, variant, missing, path, develop_settings
         FROM (
             SELECT sync_seq AS seq, id, 0 AS gone, filename, capture_date, width, height,
                    orientation, rating, flag, variant, missing, path, develop_settings
             FROM photos WHERE sync_seq > ?1
             UNION ALL
             SELECT seq, photo_id, 1, '', NULL, 0, 0, 1, 0, 0, 0, 0, '', NULL
             FROM sync_deleted WHERE seq > ?1
         )
         ORDER BY seq LIMIT ?2",
    )?;
    let mut rows = Vec::new();
    let mut removed = Vec::new();
    let mut next = since;
    let mut seen = 0u32;
    let mut more = false;
    let mut query = stmt.query(params![since, i64::from(limit) + 1])?;
    while let Some(r) = query.next()? {
        if seen == limit {
            more = true;
            break;
        }
        seen += 1;
        let seq: i64 = r.get(0)?;
        let id: i64 = r.get(1)?;
        next = seq;
        let gone: bool = r.get(2)?;
        let variant: i64 = r.get(10)?;
        let missing: bool = r.get(11)?;
        if gone || variant > 0 || missing {
            removed.push(id);
            continue;
        }
        let (w, h): (i64, i64) = (r.get(5)?, r.get(6)?);
        let orientation: i64 = r.get(7)?;
        // EXIF orientations 5 to 8 turn the frame a quarter: the phone wants
        // the shape it will show.
        let (width, height) = if (5..=8).contains(&orientation) {
            (h, w)
        } else {
            (w, h)
        };
        let path: String = r.get(12)?;
        let settings: Option<String> = r.get(13)?;
        rows.push(crate::phone_link::PhoneRow {
            id,
            filename: r.get(3)?,
            captured: r.get(4)?,
            width: width.clamp(0, u32::MAX as i64) as u32,
            height: height.clamp(0, u32::MAX as i64) as u32,
            rating: r.get::<_, i64>(8)?.clamp(0, 5) as u8,
            flag: r.get::<_, i64>(9)?.clamp(-1, 1) as i8,
            edit_tag: crate::phone_link::edit_tag(settings.as_deref()),
            edit_on: crate::phone_link::edit_on(&path, settings.as_deref()),
        });
    }
    Ok(ChangePage {
        rows,
        removed,
        next,
        more,
        head,
    })
}

/// An export waiting for a phone: id, file name, size, path.
pub type OutboxRow = (i64, String, i64, String);

pub fn outbox_add(conn: &Connection, path: &str, name: &str, size: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO phone_outbox (path, name, size) VALUES (?1, ?2, ?3)",
        params![path, name, size],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Everything waiting, oldest first.
pub fn outbox(conn: &Connection) -> Result<Vec<OutboxRow>> {
    let mut stmt = conn.prepare("SELECT id, name, size, path FROM phone_outbox ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The photo a phone's file (by SHA-256) became, if it is still in the
/// library.
pub fn phone_import(conn: &Connection, sha256: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT i.photo_id FROM phone_imports i JOIN photos p ON p.id = i.photo_id
             WHERE i.sha256 = ?1",
            [sha256],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn phone_import_add(conn: &Connection, sha256: &str, photo_id: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO phone_imports (sha256, photo_id) VALUES (?1, ?2)",
        params![sha256, photo_id],
    )?;
    Ok(())
}

/// The photo at `path`, as the scan stored it.
pub fn photo_id_for_path(conn: &Connection, path: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM photos WHERE path = ?1", [path], |r| {
            r.get(0)
        })
        .optional()?)
}

/// Forgets these and returns their files' paths, for the caller to delete.
pub fn outbox_take(conn: &mut Connection, ids: &[i64]) -> Result<Vec<String>> {
    let tx = conn.transaction()?;
    let mut paths = Vec::new();
    for id in ids {
        let path: Option<String> = tx
            .query_row("SELECT path FROM phone_outbox WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(path) = path {
            tx.execute("DELETE FROM phone_outbox WHERE id = ?1", [id])?;
            paths.push(path);
        }
    }
    tx.commit()?;
    Ok(paths)
}

/// Where a photo's thumbnail (`preview == false`) or preview is on disk.
pub fn photo_image_path(conn: &Connection, id: i64, preview: bool) -> Result<Option<String>> {
    let column = if preview {
        "preview_path"
    } else {
        "thumb_path"
    };
    let sql = format!("SELECT {column} FROM photos WHERE id = ?1");
    Ok(conn
        .query_row(&sql, params![id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

pub fn paired_phones(conn: &Connection) -> Result<Vec<crate::phone_link::PairedPhone>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, platform, paired_at, last_seen FROM paired_phones
         ORDER BY paired_at, name",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(crate::phone_link::PairedPhone {
            id: r.get(0)?,
            name: r.get(1)?,
            platform: r.get(2)?,
            paired_at: r.get(3)?,
            last_seen: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Adds a phone. Pairing the same phone again keeps one row, renamed.
pub fn add_paired_phone(conn: &Connection, id: &str, name: &str, platform: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO paired_phones(id, name, platform) VALUES(?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, platform = excluded.platform,
             paired_at = datetime('now')",
        params![id, name, platform],
    )?;
    Ok(())
}

pub fn remove_paired_phone(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM paired_phones WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn paired_phone_seen(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE paired_phones SET last_seen = datetime('now') WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}
