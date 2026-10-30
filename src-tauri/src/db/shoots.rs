//! Shoots: card imports, with their files, backups and exports.

use super::*;

/// A shoot's photos, once scanned: an import's template goes on them.
pub fn shoot_photo_ids(conn: &Connection, shoot: i32) -> Result<Vec<i32>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM photos WHERE variant = 0
            AND path IN (SELECT path FROM shoot_files WHERE shoot_id = ?1) ORDER BY id",
    )?;
    let rows = stmt.query_map(params![shoot], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Remembers a card import as a shoot: its name, where it came from
/// and went, and the files it copied. Returns the shoot's id.
pub fn create_shoot(
    conn: &mut Connection,
    name: &str,
    source: &str,
    destination: &str,
    paths: &[String],
) -> Result<i32> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO shoots (name, source, destination, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![
            name.trim(),
            source,
            destination,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    let id = tx.last_insert_rowid() as i32;
    {
        let mut ins =
            tx.prepare("INSERT OR IGNORE INTO shoot_files (shoot_id, path) VALUES (?1, ?2)")?;
        for path in paths {
            ins.execute(params![id, path])?;
        }
    }
    tx.commit()?;
    Ok(id)
}

/// Adds files a retry copied to an existing shoot.
pub fn add_shoot_files(conn: &mut Connection, id: i32, paths: &[String]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut ins =
            tx.prepare("INSERT OR IGNORE INTO shoot_files (shoot_id, path) VALUES (?1, ?2)")?;
        for path in paths {
            ins.execute(params![id, path])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Records what a shoot's import could not do: photos not in the
/// library, and photos in it but not on the backup. `backup` is kept when
/// `None`.
pub fn set_shoot_report(
    conn: &Connection,
    id: i32,
    backup: Option<&str>,
    not_copied: u32,
    not_backed_up: u32,
) -> Result<()> {
    conn.execute(
        "UPDATE shoots SET backup = COALESCE(?2, backup), not_copied = ?3, not_backed_up = ?4
          WHERE id = ?1",
        params![id, backup, not_copied, not_backed_up],
    )?;
    Ok(())
}

/// A shoot's `(not_copied, not_backed_up)` counts.
pub fn shoot_report(conn: &Connection, id: i32) -> Result<(u32, u32)> {
    Ok(conn.query_row(
        "SELECT not_copied, not_backed_up FROM shoots WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// What finishing a shoot's backup needs: where it was imported to, its
/// backup folder if it had one, and its files.
pub fn shoot_backup_plan(
    conn: &Connection,
    id: i32,
) -> Result<(String, Option<String>, Vec<String>)> {
    let (destination, backup) = conn.query_row(
        "SELECT destination, backup FROM shoots WHERE id = ?1",
        params![id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )?;
    let mut stmt =
        conn.prepare("SELECT path FROM shoot_files WHERE shoot_id = ?1 ORDER BY path")?;
    let paths = stmt
        .query_map(params![id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok((destination, backup, paths))
}

/// Forgets a shoot; its photos stay in the library.
pub fn delete_shoot(conn: &Connection, id: i32) -> Result<()> {
    conn.execute("DELETE FROM shoot_files WHERE shoot_id = ?1", params![id])?;
    conn.execute("DELETE FROM shoots WHERE id = ?1", params![id])?;
    Ok(())
}

/// Records that these photos were written out.
pub fn record_exports(conn: &Connection, photo_ids: &[i32]) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut ins = conn.prepare("INSERT INTO exports (photo_id, exported_at) VALUES (?1, ?2)")?;
    for id in photo_ids {
        ins.execute(params![id, now])?;
    }
    Ok(())
}

/// The most recent shoots, newest first, each with how far along it is.
pub fn list_shoots(conn: &Connection, limit: u32) -> Result<Vec<crate::models::Shoot>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.created_at, s.not_copied, s.not_backed_up,
                COUNT(p.id),
                COALESCE(SUM(p.rating > 0 OR p.flag != 0), 0),
                COALESCE(SUM(p.flag = 1), 0),
                COALESCE(SUM(p.flag = 1 AND p.develop_settings IS NOT NULL), 0),
                COALESCE(SUM(p.flag = 1 AND EXISTS (SELECT 1 FROM exports e WHERE e.photo_id = p.id)), 0)
           FROM shoots s
           LEFT JOIN shoot_files sf ON sf.shoot_id = s.id
           LEFT JOIN photos p ON p.path = sf.path AND p.variant = 0
          GROUP BY s.id
          ORDER BY s.created_at DESC, s.id DESC
          LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok(crate::models::Shoot {
            id: r.get(0)?,
            name: r.get(1)?,
            created_at: r.get(2)?,
            not_copied: r.get(3)?,
            not_backed_up: r.get(4)?,
            photos: r.get(5)?,
            judged: r.get(6)?,
            picks: r.get(7)?,
            edited_picks: r.get(8)?,
            exported_picks: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// A card import remembered as a shoot: its photos found by path,
    /// counted along the way from judged to exported, and filtered to — a
    /// copy with its original.
    #[test]
    fn a_shoot_counts_its_way_from_import_to_export() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, rating, flag)
             VALUES (1, 'D:/Shoot/a.arw', 'a.arw', 'h1', 'now', 0, 1),
                    (2, 'D:/Shoot/b.arw', 'b.arw', 'h2', 'now', 3, 0),
                    (3, 'D:/Shoot/c.arw', 'c.arw', 'h3', 'now', 0, 0),
                    (4, 'D:/Other/d.arw', 'd.arw', 'h4', 'now', 0, 1);
             UPDATE photos SET develop_settings = '{}' WHERE id = 1;",
        )
        .unwrap();
        let paths: Vec<String> = ["a", "b", "c", "gone"]
            .iter()
            .map(|n| format!("D:/Shoot/{n}.arw"))
            .collect();
        let id = create_shoot(&mut conn, "Smith wedding", "E:/", "D:/Shoot", &paths).unwrap();
        let copy = create_variant(&mut conn, 1, None).unwrap();

        let shoot = |conn: &Connection| list_shoots(conn, 10).unwrap().into_iter().next().unwrap();
        let s = shoot(&conn);
        assert_eq!(
            (
                s.name.as_str(),
                s.photos,
                s.judged,
                s.picks,
                s.edited_picks,
                s.exported_picks
            ),
            ("Smith wedding", 3, 2, 1, 1, 0)
        );
        record_exports(&conn, &[1]).unwrap();
        assert_eq!(shoot(&conn).exported_picks, 1);

        // What its import could not do stays on it, and a retry's
        // copies join it.
        assert_eq!((s.not_copied, s.not_backed_up), (0, 0));
        set_shoot_report(&conn, id, Some("F:/Backup"), 37, 12).unwrap();
        assert_eq!(shoot_report(&conn, id).unwrap(), (37, 12));
        add_shoot_files(&mut conn, id, &["D:/Shoot/later.arw".to_string()]).unwrap();
        set_shoot_report(&conn, id, None, 0, 12).unwrap();
        let s = shoot(&conn);
        assert_eq!((s.not_copied, s.not_backed_up), (0, 12));
        let (dest, backup, files) = shoot_backup_plan(&conn, id).unwrap();
        assert_eq!(
            (dest.as_str(), backup.as_deref(), files.len()),
            ("D:/Shoot", Some("F:/Backup"), 5),
            "the backup folder kept when not given again"
        );

        let mut only = ids(
            &conn,
            &PhotoFilter {
                shoot_id: Some(id),
                ..Default::default()
            },
        );
        only.sort();
        assert_eq!(only, vec![1, 2, 3, copy]);

        delete_shoot(&conn, id).unwrap();
        assert!(list_shoots(&conn, 10).unwrap().is_empty());
        assert!(get_photo(&conn, 1).unwrap().is_some(), "the photos stay");
    }
}
