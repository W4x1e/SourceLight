//! Culling: ratings, flags, colour labels and quality scores.

use super::*;

pub fn set_rating(conn: &Connection, photo_id: i32, rating: i32) -> Result<()> {
    conn.execute(
        "UPDATE photos SET rating = ?2 WHERE id = ?1",
        params![photo_id, rating.clamp(0, 5)],
    )?;
    Ok(())
}

/// Pick/reject flag: clamped to -1 (reject), 0 (none), 1 (pick).
pub fn set_flag(conn: &Connection, photo_id: i32, flag: i32) -> Result<()> {
    conn.execute(
        "UPDATE photos SET flag = ?2 WHERE id = ?1",
        params![photo_id, flag.clamp(-1, 1)],
    )?;
    Ok(())
}

/// Colour label: clamped to 0 (none) or 1..=5.
pub fn set_color_label(conn: &Connection, photo_id: i32, label: i32) -> Result<()> {
    conn.execute(
        "UPDATE photos SET color_label = ?2 WHERE id = ?1",
        params![photo_id, label.clamp(0, 5)],
    )?;
    Ok(())
}

/// Sets one integer column to `value` across many photos in a single statement.
/// `column` is a fixed literal from the callers below, never user input.
fn set_int_column_many(conn: &Connection, column: &str, ids: &[i32], value: i32) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE photos SET {column} = ? WHERE id IN ({})",
        placeholders(ids.len())
    );
    let mut p: Vec<Value> = Vec::with_capacity(ids.len() + 1);
    p.push(Value::Integer(i64::from(value)));
    for id in ids {
        p.push(Value::Integer(i64::from(*id)));
    }
    conn.execute(&sql, rusqlite::params_from_iter(p))?;
    Ok(())
}

/// Bulk variants of the single-photo setters — one statement for a whole
/// selection instead of a round trip (and a commit) per photo. Clamped
/// identically. An empty list is a no-op.
pub fn set_rating_many(conn: &Connection, ids: &[i32], rating: i32) -> Result<()> {
    set_int_column_many(conn, "rating", ids, rating.clamp(0, 5))
}

pub fn set_flag_many(conn: &Connection, ids: &[i32], flag: i32) -> Result<()> {
    set_int_column_many(conn, "flag", ids, flag.clamp(-1, 1))
}

/// Marks these photos' flags as accepted suggestions, after the flags
/// themselves are written: writing a flag clears the mark (migration 42).
pub fn mark_flags_suggested(conn: &Connection, ids: &[i32]) -> Result<()> {
    set_int_column_many(conn, "flag_suggested", ids, 1)
}

pub fn set_color_label_many(conn: &Connection, ids: &[i32], label: i32) -> Result<()> {
    set_int_column_many(conn, "color_label", ids, label.clamp(0, 5))
}

/// Reads the rating, flag and colour label of each photo in `ids` — what a
/// verdict change is about to overwrite, so it can be put back. Photos that
/// no longer exist are simply absent.
pub fn verdicts_of(conn: &Connection, ids: &[i32]) -> Result<Vec<Verdict>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT id, rating, flag, color_label FROM photos WHERE id IN ({}) ORDER BY id",
        placeholders(ids.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(ids), |r| {
        Ok(Verdict {
            id: r.get(0)?,
            rating: r.get(1)?,
            flag: r.get(2)?,
            color_label: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Puts back verdicts read by [`verdicts_of`], all or none. A photo deleted in
/// the meantime is skipped rather than resurrected — there is no row to write.
pub fn restore_verdicts(conn: &mut Connection, verdicts: &[Verdict]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt =
            tx.prepare("UPDATE photos SET rating = ?2, flag = ?3, color_label = ?4 WHERE id = ?1")?;
        for v in verdicts {
            stmt.execute(params![v.id, v.rating, v.flag, v.color_label])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Photos with no focus/sharpness score yet — or no brightness, which arrived
/// later — as (id, preview-or-thumb path): the queue for the quality-
/// scoring backfill. Mirrors [`unindexed_photos`]; the scores need no model, so
/// this queue drains in any build.
pub fn unscored_photos(conn: &Connection, limit: i32) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(preview_path, thumb_path) FROM photos
         WHERE (sharpness IS NULL OR brightness IS NULL) AND variant = 0
           AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many photos still need a focus score — the denominator for an
/// "N of M scored" status line. Mirrors [`unindexed_count`].
pub fn unscored_count(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM photos
         WHERE (sharpness IS NULL OR brightness IS NULL) AND variant = 0
           AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)",
        [],
        |r| r.get(0),
    )?)
}

/// Stores a photo's quality scores: focus (higher = sharper) and brightness.
pub fn set_scores(conn: &Connection, photo_id: i32, scores: &crate::quality::Scores) -> Result<()> {
    conn.execute(
        "UPDATE photos SET sharpness = ?2, brightness = ?3 WHERE id = ?1",
        params![photo_id, scores.sharpness, scores.brightness],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bulk setters touch exactly the selection, clamp like the single-photo
    /// versions, and no-op on an empty list.
    #[test]
    fn bulk_setters_apply_to_every_id_and_leave_others_alone() {
        let mut conn = open_memory().unwrap();
        let ids: Vec<i32> = (0..4)
            .map(|i| {
                conn.execute(
                    "INSERT INTO photos (path, filename, file_hash, indexed_at)
                     VALUES (?1, 'a.arw', 'h', '2026-01-01')",
                    params![format!("/p{i}.arw")],
                )
                .unwrap();
                conn.last_insert_rowid() as i32
            })
            .collect();

        let target = &ids[..3];
        set_rating_many(&conn, target, 4).unwrap();
        set_flag_many(&conn, target, 1).unwrap();
        set_color_label_many(&conn, target, 2).unwrap();
        add_manual_tag_many(&mut conn, target, "Beach", "subject").unwrap();

        for id in target {
            let p = get_photo(&conn, *id).unwrap().unwrap();
            assert_eq!((p.rating, p.flag, p.color_label), (4, 1, 2));
            assert!(p.subjects.contains(&"beach".to_string()));
        }
        let untouched = get_photo(&conn, ids[3]).unwrap().unwrap();
        assert_eq!(
            (untouched.rating, untouched.flag, untouched.color_label),
            (0, 0, 0)
        );
        assert!(untouched.subjects.is_empty());

        // Clamping still applies through the bulk path.
        set_rating_many(&conn, &ids, 99).unwrap();
        assert_eq!(get_photo(&conn, ids[0]).unwrap().unwrap().rating, 5);

        // Empty selections are harmless no-ops.
        set_flag_many(&conn, &[], 1).unwrap();
        add_manual_tag_many(&mut conn, &[], "x", "subject").unwrap();
    }

    /// A verdict change can be taken back exactly: each photo returns to
    /// its own previous rating, flag and label — not to one shared value — and a
    /// photo deleted in the meantime stays deleted.
    #[test]
    fn verdicts_restore_exactly_what_was_overwritten() {
        let mut conn = open_memory().unwrap();
        let ids: Vec<i32> = (0..3)
            .map(|i| {
                conn.execute(
                    "INSERT INTO photos (path, filename, file_hash, indexed_at)
                     VALUES (?1, 'a.arw', 'h', '2026-01-01')",
                    params![format!("/v{i}.arw")],
                )
                .unwrap();
                conn.last_insert_rowid() as i32
            })
            .collect();
        set_rating_many(&conn, &ids[0..1], 5).unwrap();
        set_rating_many(&conn, &ids[1..2], 2).unwrap();
        set_flag_many(&conn, &ids[2..3], -1).unwrap();
        set_color_label_many(&conn, &ids[0..2], 3).unwrap();

        let before = verdicts_of(&conn, &ids).unwrap();
        assert_eq!(before.len(), 3);

        // One keypress over the whole selection.
        set_rating_many(&conn, &ids, 1).unwrap();
        set_flag_many(&conn, &ids, 1).unwrap();
        conn.execute("DELETE FROM photos WHERE id = ?1", params![ids[1]])
            .unwrap();

        restore_verdicts(&mut conn, &before).unwrap();
        let after = verdicts_of(&conn, &ids).unwrap();
        let expected: Vec<Verdict> = before.into_iter().filter(|v| v.id != ids[1]).collect();
        assert_eq!(after, expected);
    }

    /// An accepted suggestion stays marked as one until the flag
    /// changes any other way — by key, by undo, by a sidecar.
    #[test]
    fn an_accepted_suggestion_is_marked_until_the_flag_changes() {
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, '/a.arw', 'a.arw', 'h1', 'x'), (2, '/b.arw', 'b.arw', 'h2', 'x');",
        )
        .unwrap();
        let marked = |id: i32| -> i64 {
            conn.query_row(
                "SELECT flag_suggested FROM photos WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        set_flag_many(&conn, &[1, 2], -1).unwrap();
        mark_flags_suggested(&conn, &[1, 2]).unwrap();
        assert_eq!((marked(1), marked(2)), (1, 1));
        let photo = |id: i32| get_photo(&conn, id).unwrap().unwrap();
        assert!(photo(1).flag_suggested);

        // The person changes their mind about one: it is theirs now.
        set_flag(&conn, 1, 1).unwrap();
        assert_eq!((marked(1), marked(2)), (0, 1));
        // Writing the same flag again is no change, and keeps the mark.
        set_flag(&conn, 2, -1).unwrap();
        assert_eq!(marked(2), 1);
    }
}
