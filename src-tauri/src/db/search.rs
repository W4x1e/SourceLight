//! The search box: the names it recognises in a query, and captions with
//! their full-text index.

use super::*;

/// The names the search box recognises in a query: named people, most
/// photographed first; places; the genres and conditions photos have; and
/// the cameras and lenses.
pub fn search_vocabulary(conn: &Connection) -> Result<crate::models::SearchVocabulary> {
    let strings = |sql: &str, arg: Option<&str>| -> Result<Vec<String>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(arg), |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    };
    let tagged = "SELECT t.name FROM tags t
                   WHERE t.category = ?1
                     AND EXISTS (SELECT 1 FROM photo_tags pt WHERE pt.tag_id = t.id)
                   ORDER BY t.name";
    let people = {
        let mut stmt = conn.prepare(
            "SELECT id, name FROM people
              WHERE name IS NOT NULL AND TRIM(name) != ''
              ORDER BY size DESC, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(crate::models::NamedPerson {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(crate::models::SearchVocabulary {
        people,
        places: list_places(conn)?,
        genres: strings(tagged, Some("genre"))?,
        keywords: strings(tagged, Some("subject"))?,
        conditions: strings(tagged, Some("condition"))?,
        cameras: strings(
            "SELECT DISTINCT camera FROM photos WHERE TRIM(COALESCE(camera, '')) != '' ORDER BY 1",
            None,
        )?,
        lenses: strings(
            "SELECT DISTINCT lens FROM photos WHERE TRIM(COALESCE(lens, '')) != '' ORDER BY 1",
            None,
        )?,
    })
}

/// Stores (or clears, with `None`) a photo's VLM caption. The `captions_fts`
/// index follows automatically through the migration-6 triggers.
pub fn set_caption(conn: &Connection, photo_id: i32, caption: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE photos SET caption = ?2 WHERE id = ?1",
        params![photo_id, caption],
    )?;
    Ok(())
}

/// Photos with no caption yet, as (id, preview-or-thumb path) — the captioning
/// job's queue. Mirrors [`unindexed_photos`].
pub fn uncaptioned_photos(conn: &Connection, limit: i32) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(preview_path, thumb_path) FROM photos
         WHERE caption IS NULL AND variant = 0 AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many captionable photos still lack a caption — the captioning job's
/// "N to caption" status, mirroring `unindexed_count` for embeddings.
pub fn uncaptioned_count(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM photos
         WHERE caption IS NULL AND variant = 0 AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)",
        [],
        |r| r.get(0),
    )?)
}

/// Ranks photos by caption full-text match (best first, bm25). `raw` is free
/// search-box text; an empty/punctuation-only query returns no ids (the caller
/// reads that as "no caption search"), never an FTS syntax error.
pub fn caption_search(conn: &Connection, raw: &str, limit: u32) -> Result<Vec<i32>> {
    let Some(query) = crate::fts::build_query(raw) else {
        return Ok(Vec::new());
    };
    // A title or caption someone wrote first, then the model's.
    let mut out: Vec<i32> = Vec::new();
    for table in ["iptc_fts", "captions_fts"] {
        let mut stmt = conn.prepare(&format!(
            "SELECT rowid FROM {table} WHERE {table} MATCH ?1 ORDER BY rank LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![query, limit], |r| r.get::<_, i32>(0))?;
        for id in rows {
            let id = id?;
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out.truncate(limit as usize);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    #[test]
    fn caption_fts_indexes_searches_reindexes_and_deletes() {
        let conn = open_memory().unwrap();

        // Three captioned photos plus one still awaiting a caption.
        let captions = [
            Some("a red car parked on a sunny street"),
            Some("a blue bicycle by the beach at sunset"),
            Some("a red bus in the rain"),
            None,
        ];
        let mut ids = Vec::new();
        for (i, cap) in captions.iter().enumerate() {
            conn.execute(
                "INSERT INTO photos (path, filename, file_hash, thumb_path, indexed_at)
                 VALUES (?1, 'f.arw', 'h', ?2, '2026-01-01')",
                params![format!("/p{i}.arw"), format!("/t{i}.jpg")],
            )
            .unwrap();
            let id = conn.last_insert_rowid() as i32;
            set_caption(&conn, id, *cap).unwrap();
            ids.push(id);
        }

        // Only the captionless photo is still pending.
        assert_eq!(uncaptioned_count(&conn).unwrap(), 1);

        // "red" hits the two red photos, not the bicycle.
        let red = caption_search(&conn, "red", 10).unwrap();
        assert_eq!(red.len(), 2);
        assert!(red.contains(&ids[0]) && red.contains(&ids[2]));

        // Implicit AND narrows "red car" to just the first.
        assert_eq!(caption_search(&conn, "red car", 10).unwrap(), vec![ids[0]]);

        // The last token is a prefix, so an as-you-type "sun" reaches "sunny"
        // and "sunset".
        let sun = caption_search(&conn, "sun", 10).unwrap();
        assert_eq!(sun.len(), 2);
        assert!(sun.contains(&ids[0]) && sun.contains(&ids[1]));

        // Editing a caption re-indexes it: photo 2 stops being "red".
        set_caption(&conn, ids[2], Some("a green tractor in a field")).unwrap();
        assert_eq!(caption_search(&conn, "red", 10).unwrap(), vec![ids[0]]);
        assert_eq!(caption_search(&conn, "tractor", 10).unwrap(), vec![ids[2]]);

        // Deleting a photo drops it from the index.
        conn.execute("DELETE FROM photos WHERE id = ?1", params![ids[0]])
            .unwrap();
        assert!(caption_search(&conn, "car", 10).unwrap().is_empty());

        // A punctuation-only query is empty, never an FTS syntax error.
        assert!(caption_search(&conn, "  ??  ", 10).unwrap().is_empty());
    }

    /// Typing matches who is in a photo and what its caption says, as
    /// well as its filename, tags and place — a copy through its original.
    #[test]
    fn typing_matches_people_and_captions() {
        let conn = gear_library();
        set_caption(&conn, 3, Some("A red tractor in a field")).unwrap();
        let typed = |s: &str| {
            let mut got = ids(
                &conn,
                &PhotoFilter {
                    search: Some(s.into()),
                    ..Default::default()
                },
            );
            got.sort();
            got
        };
        assert_eq!(typed("ada"), vec![1, 2, 5]);
        assert_eq!(typed("Tractor"), vec![3]);
        assert_eq!(typed("karoo"), vec![1]);
        assert_eq!(typed("nobody"), Vec::<i32>::new());
    }

    /// What the search box can recognise: named people (hidden ones too —
    /// their names are still theirs), places, the tags photos have, and gear.
    #[test]
    fn the_search_vocabulary_has_names_places_tags_and_gear() {
        let conn = gear_library();
        add_manual_tag(&conn, 1, "landscape", "genre").unwrap();
        add_manual_tag(&conn, 2, "golden hour", "condition").unwrap();
        tag_id(&conn, "unused", "genre").unwrap();
        let v = search_vocabulary(&conn).unwrap();
        let people: Vec<&str> = v.people.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(people, vec!["Ada", "Bo", "Cy"]);
        let places: Vec<&str> = v.places.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(places, vec!["Karoo"]);
        assert_eq!(
            v.genres,
            vec!["landscape".to_string()],
            "a tag no photo has is left out"
        );
        assert_eq!(v.conditions, vec!["golden hour".to_string()]);
        assert_eq!(v.cameras, vec!["A7".to_string(), "Z6".to_string()]);
        assert_eq!(v.lenses, vec!["24-70".to_string(), "70-200".to_string()]);
    }

    #[test]
    fn the_vocabulary_offers_keywords_to_leave_out() {
        let mut conn = open_memory().unwrap();
        beach_and_dog(&mut conn);
        let v = search_vocabulary(&conn).unwrap();
        assert_eq!(v.keywords, vec!["beach", "car", "dog"]);
    }
}
