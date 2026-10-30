//! Tags on photos: the tagger's, and the ones the user adds or takes off.

use super::*;

pub(super) fn tag_id(conn: &Connection, name: &str, category: &str) -> Result<i32> {
    conn.execute(
        "INSERT OR IGNORE INTO tags(name, category) VALUES(?1, ?2)",
        params![name, category],
    )?;
    let id: i32 = conn.query_row(
        "SELECT id FROM tags WHERE name = ?1 AND category = ?2",
        params![name, category],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// Replaces AI-sourced tags for a photo. Manual tags are deliberately left
/// untouched so a re-run of the tagger never destroys the user's own curation.
pub fn apply_ai_tags(conn: &mut Connection, photo_id: i32, result: &TagResult) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM photo_tags WHERE photo_id = ?1 AND source = 'ai'",
        params![photo_id],
    )?;
    // A genre the user gave — or a sidecar did — is not second-guessed:
    // a photo has one, and two would put it under both.
    let manual_genre: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
                         WHERE pt.photo_id = ?1 AND pt.source = 'manual' AND t.category = 'genre')",
        params![photo_id],
        |r| r.get(0),
    )?;

    let insert = |name: &str, category: &str, conf: f64| -> Result<()> {
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            return Ok(());
        }
        let tid = tag_id(&tx, &name, category)?;
        // Not one the user took off this photo.
        tx.execute(
            "INSERT OR IGNORE INTO photo_tags(photo_id, tag_id, confidence, source)
             SELECT ?1, ?2, ?3, 'ai'
              WHERE NOT EXISTS (SELECT 1 FROM rejected_tags WHERE photo_id = ?1 AND tag_id = ?2)",
            params![photo_id, tid, conf],
        )?;
        Ok(())
    };

    if !result.genre.is_empty() && !manual_genre {
        insert(&result.genre, "genre", result.confidence)?;
    }
    for s in &result.subjects {
        insert(s, "subject", result.confidence)?;
    }
    for c in &result.conditions {
        insert(c, "condition", result.confidence)?;
    }

    tx.execute(
        "UPDATE photos SET tagged_at = ?2 WHERE id = ?1",
        params![photo_id, chrono::Utc::now().to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn add_manual_tag(conn: &Connection, photo_id: i32, name: &str, category: &str) -> Result<()> {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return Ok(());
    }
    let tid = tag_id(conn, &name, category)?;
    conn.execute(
        "INSERT INTO photo_tags(photo_id, tag_id, confidence, source)
         VALUES(?1, ?2, 1.0, 'manual')
         ON CONFLICT(photo_id, tag_id) DO UPDATE SET source = 'manual', confidence = 1.0",
        params![photo_id, tid],
    )?;
    conn.execute(
        "DELETE FROM rejected_tags WHERE photo_id = ?1 AND tag_id = ?2",
        params![photo_id, tid],
    )?;
    Ok(())
}

/// Takes a tag off a photo. One the tagger gave, or a folder name did,
/// is also remembered as turned down, so neither puts it back; one the
/// user gave is simply gone.
pub fn remove_tag(conn: &Connection, photo_id: i32, name: &str) -> Result<()> {
    let name = name.trim().to_lowercase();
    conn.execute(
        "INSERT OR IGNORE INTO rejected_tags (photo_id, tag_id)
         SELECT photo_id, tag_id FROM photo_tags
          WHERE photo_id = ?1 AND source IN ('ai', 'folder')
            AND tag_id IN (SELECT id FROM tags WHERE name = ?2)",
        params![photo_id, name],
    )?;
    conn.execute(
        "DELETE FROM photo_tags WHERE photo_id = ?1 AND tag_id IN
           (SELECT id FROM tags WHERE name = ?2)",
        params![photo_id, name],
    )?;
    Ok(())
}

/// The keywords the tagger gave these photos that nobody has confirmed,
/// per photo — for the selection panel to tell them from the user's.
/// Asked for the selection only, so the grid's photo query stays index-only.
pub fn ai_keywords(conn: &Connection, ids: &[i32]) -> Result<Vec<AiKeywords>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT pt.photo_id, t.name FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
          WHERE pt.photo_id IN ({}) AND pt.source = 'ai' AND t.category = 'subject'
          ORDER BY pt.photo_id, t.name",
        placeholders(ids.len())
    ))?;
    let mut out: Vec<AiKeywords> = Vec::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(ids))?;
    while let Some(r) = rows.next()? {
        let (photo_id, name): (i32, String) = (r.get(0)?, r.get(1)?);
        match out.last_mut() {
            Some(last) if last.photo_id == photo_id => last.names.push(name),
            _ => out.push(AiKeywords {
                photo_id,
                names: vec![name],
            }),
        }
    }
    Ok(out)
}

/// Makes the tagger's keywords on these photos the user's own: a
/// re-run leaves them alone from then on, and they count when only the
/// user's keywords are shown. `names` empty means every one of them. Returns
/// how many were confirmed.
pub fn confirm_keywords(conn: &Connection, ids: &[i32], names: &[String]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut p: Vec<Value> = ids
        .iter()
        .map(|&id| Value::Integer(i64::from(id)))
        .collect();
    let by_name = if names.is_empty() {
        String::new()
    } else {
        p.extend(names.iter().map(|n| Value::Text(n.trim().to_lowercase())));
        format!("AND name IN ({})", placeholders(names.len()))
    };
    let n = conn.execute(
        &format!(
            "UPDATE photo_tags SET source = 'manual', confidence = 1.0
              WHERE photo_id IN ({}) AND source = 'ai'
                AND tag_id IN (SELECT id FROM tags WHERE category = 'subject' {by_name})",
            placeholders(ids.len())
        ),
        rusqlite::params_from_iter(p),
    )?;
    Ok(n)
}

/// Adds one manual tag to many photos in a single transaction (manual source, so
/// a later AI re-tag never removes it). Empty name or list is a no-op.
///
/// A subject typed as a path — `places > europe > france` — files the keyword
/// there, making the levels it needs, and tags the photos with the last one.
pub fn add_manual_tag_many(
    conn: &mut Connection,
    ids: &[i32],
    name: &str,
    category: &str,
) -> Result<()> {
    let path = if category == "subject" {
        keyword_path(name)
    } else {
        vec![name.trim().to_lowercase()]
    };
    let Some(name) = path.last().filter(|n| !n.is_empty()).cloned() else {
        return Ok(());
    };
    if ids.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    if path.len() > 1 {
        ensure_keyword_path(&tx, &path)?;
    }
    let tid = tag_id(&tx, &name, category)?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO photo_tags(photo_id, tag_id, confidence, source)
             VALUES(?1, ?2, 1.0, 'manual')
             ON CONFLICT(photo_id, tag_id) DO UPDATE SET source = 'manual', confidence = 1.0",
        )?;
        let mut lift =
            tx.prepare("DELETE FROM rejected_tags WHERE photo_id = ?1 AND tag_id = ?2")?;
        for id in ids {
            stmt.execute(params![id, tid])?;
            lift.execute(params![id, tid])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Puts one photo's keywords on others — the user's own, and those its
/// folder names gave, not the tagger's unconfirmed guesses. They arrive as
/// the user's. Returns how many keywords were copied.
pub fn paste_keywords(conn: &mut Connection, ids: &[i32], from: i32) -> Result<usize> {
    let names: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT t.name FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
              WHERE pt.photo_id = ?1 AND t.category = 'subject' AND pt.source != 'ai'
              ORDER BY t.name",
        )?;
        let rows = stmt.query_map(params![from], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };
    if names.is_empty() {
        bail!("that photo has no keywords of your own to copy");
    }
    let targets: Vec<i32> = ids.iter().copied().filter(|&id| id != from).collect();
    for name in &names {
        add_manual_tag_many(conn, &targets, name, "subject")?;
    }
    Ok(names.len())
}

/// Clears the "already tagged" marks so the next batch re-runs over everything.
/// Existing AI tags are left in place until each photo is re-tagged (and manual
/// tags are never touched), so the library stays usable while the batch runs.
pub fn reset_tagging(conn: &Connection) -> Result<usize> {
    let n = conn.execute("UPDATE photos SET tagged_at = NULL", [])?;
    Ok(n)
}

/// Photos still needing AI tags, oldest first, for the resumable batch.
pub fn untagged_photos(conn: &Connection, limit: i32) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(preview_path, thumb_path) FROM photos
         WHERE tagged_at IS NULL AND variant = 0 AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    #[test]
    fn a_photo_says_which_keywords_are_the_taggers() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 1);
        tagger_says(&mut conn, 1, &["dog", "beach"]);
        add_manual_tag_many(&mut conn, &[1], "pier", "subject").unwrap();
        add_photos_from(&conn, 2);
        let got = ai_keywords(&conn, &[1, 2]).unwrap();
        assert_eq!(
            got,
            vec![AiKeywords {
                photo_id: 1,
                names: vec!["beach".into(), "dog".into()]
            }]
        );
        assert!(photo_of(&conn, 1).subjects.contains(&"pier".to_string()));
    }

    #[test]
    fn a_tagger_keyword_taken_off_stays_off() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        tagger_says(&mut conn, 1, &["dog", "beach"]);
        tagger_says(&mut conn, 2, &["dog"]);
        remove_tag(&conn, 1, "dog").unwrap();
        // Tagging again, as "Re-tag everything" would.
        tagger_says(&mut conn, 1, &["dog", "beach"]);
        tagger_says(&mut conn, 2, &["dog"]);
        assert_eq!(photo_of(&conn, 1).subjects, vec!["beach"]);
        assert_eq!(
            photo_of(&conn, 2).subjects,
            vec!["dog"],
            "only where it was taken off"
        );

        // Put back by hand, it is the user's, and the rejection is lifted.
        add_manual_tag_many(&mut conn, &[1], "dog", "subject").unwrap();
        remove_tag(&conn, 1, "dog").unwrap();
        tagger_says(&mut conn, 1, &["dog", "beach"]);
        let p = photo_of(&conn, 1);
        assert!(
            p.subjects.contains(&"dog".to_string()),
            "a user's own keyword is just gone"
        );
    }

    #[test]
    fn confirmed_keywords_are_the_users_and_outlast_a_retag() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        tagger_says(&mut conn, 1, &["dog", "beach"]);
        tagger_says(&mut conn, 2, &["dog", "sky"]);
        assert_eq!(confirm_keywords(&conn, &[1], &["Dog".into()]).unwrap(), 1);
        assert_eq!(ai_keywords(&conn, &[1]).unwrap()[0].names, vec!["beach"]);
        // Everything on photo 2.
        assert_eq!(confirm_keywords(&conn, &[2], &[]).unwrap(), 2);
        assert!(ai_keywords(&conn, &[2]).unwrap().is_empty());
        // The tagger changes its mind; what was confirmed stays.
        tagger_says(&mut conn, 1, &["cat"]);
        let mut subjects = photo_of(&conn, 1).subjects;
        subjects.sort();
        assert_eq!(subjects, vec!["cat", "dog"]);
        assert_eq!(confirm_keywords(&conn, &[], &[]).unwrap(), 0);
    }

    /// With only the user's keywords shown, the tagger's neither match nor
    /// count — a branch's included.
    #[test]
    fn own_keywords_leave_the_taggers_out_of_matching_and_counts() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        tagger_says(&mut conn, 1, &["dog"]);
        add_manual_tag_many(&mut conn, &[2], "animals > dog", "subject").unwrap();
        tagger_says(&mut conn, 3, &["cat"]);
        move_keyword(&conn, "cat", Some("animals")).unwrap();
        let own = |f: PhotoFilter| PhotoFilter {
            own_keywords: true,
            ..f
        };
        assert_eq!(matching(&conn, &subjects_filter(&["dog"])), vec![1, 2]);
        assert_eq!(matching(&conn, &own(subjects_filter(&["dog"]))), vec![2]);
        assert_eq!(
            matching(&conn, &own(subjects_filter(&["animals"]))),
            vec![2]
        );
        let all = own(PhotoFilter::default());
        assert_eq!(subject_count(&conn, &all, "dog"), 1);
        assert_eq!(subject_count(&conn, &all, "cat"), 0);
        assert_eq!(subject_count(&conn, &all, "animals"), 1);
        assert_eq!(subject_count(&conn, &PhotoFilter::default(), "animals"), 3);
        // Genres are not keywords: the tagger's still count.
        let genres = facets(&conn, &all, None).unwrap().genres;
        assert_eq!(genres.first().map(|g| g.count), Some(2));
    }

    #[test]
    fn pasted_keywords_are_the_users_own_and_arrive_as_theirs() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        add_manual_tag_many(&mut conn, &[1], "pier", "subject").unwrap();
        add_manual_tag_many(&mut conn, &[1], "places > skye", "subject").unwrap();
        tagger_says(&mut conn, 1, &["sky"]);
        assert_eq!(paste_keywords(&mut conn, &[2, 3], 1).unwrap(), 2);
        assert_eq!(sorted_subjects(&conn, 2), vec!["pier", "skye"]);
        assert!(
            ai_keywords(&conn, &[2]).unwrap().is_empty(),
            "arrive as the user's"
        );
        tagger_says(&mut conn, 3, &["sky"]);
        assert!(
            paste_keywords(&mut conn, &[1], 3).is_ok(),
            "photo 3's pasted ones are its own"
        );
        let mut fresh = open_memory().unwrap();
        add_photos(&fresh, 2);
        tagger_says(&mut fresh, 1, &["sky"]);
        assert!(
            paste_keywords(&mut fresh, &[2], 1).is_err(),
            "only the tagger's to copy"
        );
    }
}
