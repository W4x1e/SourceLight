//! Search embeddings: storing them, scoring a query against them, and
//! loading them for grouping.

use super::*;

/// Photos with no search embedding yet, as (id, preview-or-thumb path) — the
/// queue for the indexing job. Mirrors [`untagged_photos`].
pub fn unindexed_photos(conn: &Connection, limit: i32) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(preview_path, thumb_path) FROM photos
         WHERE id NOT IN (SELECT photo_id FROM photo_embeddings)
           AND variant = 0 AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many photos still need a search embedding — the denominator for a
/// "N of M indexed" status line.
pub fn unindexed_count(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM photos
         WHERE id NOT IN (SELECT photo_id FROM photo_embeddings)
           AND variant = 0 AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)",
        [],
        |r| r.get(0),
    )?)
}

/// Stores a photo's CLIP search embedding (raw little-endian f32 blob), in
/// `photo_embeddings`. A photo deleted while it was being indexed gets
/// none, as the update this replaces matched nothing.
pub fn set_embedding(conn: &Connection, photo_id: i32, blob: &[u8]) -> Result<()> {
    conn.execute(
        "INSERT INTO photo_embeddings (photo_id, embedding)
         SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM photos WHERE id = ?1)
         ON CONFLICT (photo_id) DO UPDATE SET embedding = excluded.embedding",
        params![photo_id, blob],
    )?;
    Ok(())
}

/// One photo's CLIP search embedding blob, if it has been indexed.
pub fn get_embedding(conn: &Connection, photo_id: i32) -> Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT embedding FROM photo_embeddings WHERE photo_id = ?1",
            params![photo_id],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten())
}

/// Every indexed photo's (id, embedding blob), loaded for a search rank. Brute
/// force over the whole set is fine into the tens of thousands; an ANN index is
/// a later refinement noted in the plan.
pub fn all_embeddings(conn: &Connection) -> Result<Vec<(i64, Vec<u8>)>> {
    let mut stmt = conn.prepare("SELECT photo_id, embedding FROM photo_embeddings")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i32>(0)? as i64, r.get::<_, Vec<u8>>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Every indexed photo's similarity to `query`, scored as the rows are read.
///
/// What a search needs of the library is one number a photo, and
/// [`all_embeddings`] held every vector at once to get it — some 100 MB at
/// 50,000 photos, copied twice more on the way to a score. This reads each
/// blob in place and keeps only the score.
pub fn embedding_scores(conn: &Connection, query: &[f32]) -> Result<Vec<(i64, f32)>> {
    let mut stmt = conn.prepare("SELECT photo_id, embedding FROM photo_embeddings")?;
    let mut rows = stmt.query([])?;
    let mut scratch = Vec::with_capacity(query.len());
    let mut scores = Vec::new();
    while let Some(row) = rows.next()? {
        let blob = row.get_ref(1)?.as_blob()?;
        if let Some(score) = crate::search::score_blob(query, blob, &mut scratch) {
            scores.push((row.get::<_, i64>(0)?, score));
        }
    }
    Ok(scores)
}

/// The embeddings of just these photos, where they have one — for the work
/// that needs a few vectors rather than a score for every photo.
pub fn embeddings_of(conn: &Connection, ids: &[i32]) -> Result<Vec<(i32, Vec<f32>)>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(500) {
        let sql = format!(
            "SELECT photo_id, embedding FROM photo_embeddings WHERE photo_id IN ({})",
            placeholders(chunk.len())
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(chunk))?;
        while let Some(row) = rows.next()? {
            if let Some(v) = crate::search::decode_embedding(row.get_ref(1)?.as_blob()?) {
                out.push((row.get(0)?, v));
            }
        }
    }
    // In id order, as a scan of the table reads them.
    out.sort_by_key(|(id, _)| *id);
    Ok(out)
}

/// Like [`all_embeddings`] but carrying each photo's `capture_date`, and ordered
/// by it — the input `search::group_near_duplicates` needs to restrict its
/// comparisons to a time window (a burst) instead of comparing every photo
/// against every other, which is what kept it from finishing at library scale.
/// `capture_date` is `None` for a photo whose date is unknown.
/// Ids bound into one `IN (…)` at most: well under SQLite's 32,766.
pub(super) const ID_CHUNK: usize = 10_000;

/// [`embeddings_for_grouping`] for only `ids`, in capture order: the
/// set being culled or shown, not every photo in the library.
pub fn embeddings_for_grouping_in(
    conn: &Connection,
    ids: &[i32],
) -> Result<Vec<crate::search::DuplicateCandidate>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(ID_CHUNK) {
        let sql = format!(
            "SELECT p.id, p.capture_date, e.embedding
               FROM photo_embeddings e JOIN photos p ON p.id = e.photo_id
              WHERE e.photo_id IN ({})",
            vec!["?"; chunk.len()].join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            Ok((
                r.get::<_, i32>(0)? as i64,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    // NULL dates first, as the whole-library query's ORDER BY puts them.
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// [`photo_paths`] for only `ids`.
pub fn photo_paths_in(conn: &Connection, ids: &[i32]) -> Result<Vec<(i32, std::path::PathBuf)>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(ID_CHUNK) {
        let sql = format!(
            "SELECT id, path FROM photos WHERE variant = 0 AND id IN ({}) ORDER BY id",
            vec!["?"; chunk.len()].join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            Ok((
                r.get::<_, i32>(0)?,
                std::path::PathBuf::from(r.get::<_, String>(1)?),
            ))
        })?;
        out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    out.sort_by_key(|(id, _)| *id);
    Ok(out)
}

pub fn embeddings_for_grouping(
    conn: &Connection,
) -> Result<Vec<crate::search::DuplicateCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.capture_date, e.embedding
           FROM photo_embeddings e JOIN photos p ON p.id = e.photo_id
         ORDER BY p.capture_date ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i32>(0)? as i64,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Every original's id and file path, for the exact-duplicate check. Virtual
/// copies share their master's path and are left out.
pub fn photo_paths(conn: &Connection) -> Result<Vec<(i32, std::path::PathBuf)>> {
    let mut stmt = conn.prepare("SELECT id, path FROM photos WHERE variant = 0 ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i32>(0)?,
            std::path::PathBuf::from(r.get::<_, String>(1)?),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_embeddings_store_load_and_keep_rank_order() {
        use crate::search::{encode_embedding, rank};
        let conn = open_memory().unwrap();

        let mut ids = Vec::new();
        for i in 0..3 {
            conn.execute(
                "INSERT INTO photos (path, filename, file_hash, thumb_path, indexed_at)
                 VALUES (?1, 'f.arw', 'h', ?2, '2026-01-01')",
                params![format!("/p{i}.arw"), format!("/t{i}.jpg")],
            )
            .unwrap();
            ids.push(conn.last_insert_rowid() as i32);
        }

        // All three are waiting to be indexed until an embedding lands.
        assert_eq!(unindexed_count(&conn).unwrap(), 3);
        assert_eq!(get_embedding(&conn, ids[0]).unwrap(), None);
        assert_eq!(get_embedding(&conn, 9999).unwrap(), None, "no such photo");
        let vecs = [
            vec![1.0f32, 0.0],
            vec![0.0, 1.0],
            vec![
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ],
        ];
        for (id, v) in ids.iter().zip(&vecs) {
            set_embedding(&conn, *id, &encode_embedding(v)).unwrap();
        }
        assert_eq!(unindexed_count(&conn).unwrap(), 0);
        assert_eq!(all_embeddings(&conn).unwrap().len(), 3);
        assert_eq!(
            get_embedding(&conn, ids[1]).unwrap(),
            Some(encode_embedding(&vecs[1]))
        );

        // A query along +x ranks photo 0 (1,0) first, then the diagonal, then (0,1).
        let ranked = rank(&[1.0, 0.0], &all_embeddings(&conn).unwrap(), 3, 0.0);
        let ranked_ids: Vec<i32> = ranked.iter().map(|(id, _)| *id as i32).collect();
        assert_eq!(ranked_ids, vec![ids[0], ids[2], ids[1]]);
        // Scored as the rows are read, the ranking is the same, to the score.
        let streamed = crate::search::best(embedding_scores(&conn, &[1.0, 0.0]).unwrap(), 3, 0.0);
        assert_eq!(streamed, ranked);
        // A query of another width matches nothing rather than misreading.
        assert!(embedding_scores(&conn, &[1.0, 0.0, 0.0])
            .unwrap()
            .is_empty());

        // Just the photos asked for, in id order, and none that has no vector.
        let unindexed = {
            conn.execute(
                "INSERT INTO photos (path, filename, file_hash, indexed_at)
                 VALUES ('/p9.arw', 'f.arw', 'h', '2026-01-01')",
                [],
            )
            .unwrap();
            conn.last_insert_rowid() as i32
        };
        let some = embeddings_of(&conn, &[ids[2], unindexed, ids[0]]).unwrap();
        assert_eq!(
            some,
            vec![(ids[0], vecs[0].clone()), (ids[2], vecs[2].clone())]
        );

        // And fetching by those ids preserves the ranking order.
        let photos = photos_by_ids(&conn, &ranked_ids).unwrap();
        assert_eq!(photos.iter().map(|p| p.id).collect::<Vec<_>>(), ranked_ids);
    }
}
