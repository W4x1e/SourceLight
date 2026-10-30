//! Photo rows: what a scan writes, virtual copies, and forgetting photos.

use super::*;

pub fn photo_exists(conn: &Connection, path: &str, hash: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM photos WHERE path = ?1 AND file_hash = ?2 AND variant = 0",
        params![path, hash],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// A file's size and modified time in whole seconds, as a scan saw them —
/// enough for a rescan to know an unchanged file without opening it
/// (migration 27).
pub type FileStamp = (i64, i64);

/// The file size a row's stamp is given when its thumbnail or preview has
/// gone, so the next scan makes them again even though the file is known
/// and unchanged. No real file has it.
pub const REDERIVE: i64 = -1;

/// Marks every photo whose thumbnail or preview file is missing with
/// [`REDERIVE`]. Run after a backup is restored: previews of photos removed
/// since are deleted with them, and the scan otherwise skips a file it knows.
pub fn rederive_missing(conn: &Connection) -> Result<usize> {
    let gone: Vec<i32> = {
        let mut stmt =
            conn.prepare("SELECT id, thumb_path, preview_path FROM photos WHERE variant = 0")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i32>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, thumb, preview) = row?;
            if [thumb, preview]
                .iter()
                .flatten()
                .any(|p| !Path::new(p).exists())
            {
                out.push(id);
            }
        }
        out
    };
    for id in &gone {
        conn.execute(
            "UPDATE photos SET file_size = ?2 WHERE id = ?1",
            params![id, REDERIVE],
        )?;
    }
    Ok(gone.len())
}

/// What the catalogue knows of each photo file, by path: its quick hash, and
/// the stamp the last scan saw, if one has since migration 27. One query for
/// a whole scan, where asking per file was a query a photo.
pub fn known_files(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, (String, Option<FileStamp>)>> {
    let mut stmt = conn
        .prepare("SELECT path, file_hash, file_size, file_mtime FROM photos WHERE variant = 0")?;
    let rows = stmt.query_map([], |r| {
        let size: Option<i64> = r.get(2)?;
        let mtime: Option<i64> = r.get(3)?;
        Ok((r.get(0)?, (r.get(1)?, size.zip(mtime))))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// What earlier scans know about a file they couldn't read.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanAttempt {
    /// The file as it was then; a changed file is tried afresh.
    pub stamp: FileStamp,
    /// Reads begun and never finished cleanly.
    pub attempts: i64,
    /// Why it failed, when the read ended in an error rather than a crash.
    pub reason: Option<String>,
}

/// Every file earlier scans couldn't read, by path.
pub fn scan_attempts(conn: &Connection) -> Result<std::collections::HashMap<String, ScanAttempt>> {
    let mut stmt = conn.prepare("SELECT path, size, mtime, attempts, reason FROM scan_attempts")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            ScanAttempt {
                stamp: (r.get(1)?, r.get(2)?),
                attempts: r.get(3)?,
                reason: r.get(4)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Counts a read of each file as begun, before it starts: what is left
/// counted if the app goes down mid-read. A file whose stamp changed starts
/// again from one.
pub fn begin_scan_attempts(conn: &mut Connection, files: &[(String, FileStamp)]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut up = tx.prepare(
            "INSERT INTO scan_attempts (path, size, mtime, attempts) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(path) DO UPDATE SET
                attempts = CASE WHEN size = ?2 AND mtime = ?3 THEN attempts + 1 ELSE 1 END,
                reason = NULL, size = ?2, mtime = ?3",
        )?;
        for (path, (size, mtime)) in files {
            up.execute(params![path, size, mtime])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Ends reads, `(path, failure)`, in one transaction: a file read cleanly is
/// forgotten here; one that failed keeps why, and is skipped until it changes.
pub fn end_scan_attempts(conn: &mut Connection, ended: &[(String, Option<String>)]) -> Result<()> {
    if ended.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut forget = tx.prepare("DELETE FROM scan_attempts WHERE path = ?1")?;
        let mut fail = tx.prepare("UPDATE scan_attempts SET reason = ?2 WHERE path = ?1")?;
        for (path, failure) in ended {
            match failure {
                None => forget.execute(params![path])?,
                Some(why) => fail.execute(params![path, why])?,
            };
        }
    }
    tx.commit()?;
    Ok(())
}

/// Records the stamps a scan saw on files it already had, in one transaction,
/// so the next scan knows them by a `stat` alone.
pub fn stamp_files(conn: &mut Connection, stamps: &[(String, FileStamp)]) -> Result<()> {
    if stamps.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "UPDATE photos SET file_size = ?2, file_mtime = ?3 WHERE path = ?1 AND variant = 0",
        )?;
        for (path, (size, mtime)) in stamps {
            stmt.execute(params![path, size, mtime])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Gives files whose modified time moved, and nothing else, their new quick
/// hash and stamp, keeping everything worked out from them.
pub fn restamp_files(conn: &mut Connection, files: &[(String, String, FileStamp)]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "UPDATE photos SET file_hash = ?2, file_size = ?3, file_mtime = ?4
              WHERE path = ?1 AND variant = 0",
        )?;
        for (path, hash, (size, mtime)) in files {
            stmt.execute(params![path, hash, size, mtime])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn upsert_photo(conn: &Connection, p: &crate::scan::IngestedPhoto) -> Result<i32> {
    // What the row held before, to tell a file edited in place.
    let before: Option<OldContent> = conn
        .query_row(
            "SELECT file_hash, lat, lon, thumb_path, preview_path
               FROM photos WHERE path = ?1 AND variant = 0",
            params![p.path],
            |r| {
                Ok(OldContent {
                    hash: r.get(0)?,
                    gps: r.get::<_, Option<f64>>(1)?.zip(r.get::<_, Option<f64>>(2)?),
                    derivatives: [r.get(3)?, r.get(4)?],
                })
            },
        )
        .optional()?;
    conn.execute(
        // The scanner only ever writes master rows, so `variant` is 0 here and
        // the conflict target is the (path, variant) pair — a virtual copy
        // shares the path and must not be updated by a rescan of the file.
        "INSERT INTO photos (path, filename, file_hash, capture_date, camera, lens, iso,
                             aperture, shutter, focal_length, width, height, orientation,
                             thumb_path, preview_path, sharpness, indexed_at, variant,
                             lat, lon, gps_read, brightness, file_size, file_mtime,
                             colours_read)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,0,?18,?19,1,?20,
                 ?21,?22,1)
         ON CONFLICT(path, variant) DO UPDATE SET
            file_size = excluded.file_size,
            file_mtime = excluded.file_mtime,
            -- A position a GPX track gave stays when the file has none
            -- of its own; the camera's own replaces it.
            lat = CASE WHEN excluded.lat IS NULL AND photos.gps_track IS NOT NULL
                       THEN photos.lat ELSE excluded.lat END,
            lon = CASE WHEN excluded.lat IS NULL AND photos.gps_track IS NOT NULL
                       THEN photos.lon ELSE excluded.lon END,
            gps_track = CASE WHEN excluded.lat IS NULL THEN photos.gps_track END,
            gps_offset = CASE WHEN excluded.lat IS NULL THEN photos.gps_offset END,
            gps_read = 1,
            colours_read = 1,
            -- A render as edited is of the old content.
            edited_path = CASE WHEN photos.file_hash IS excluded.file_hash
                               THEN photos.edited_path END,
            file_hash = excluded.file_hash,
            -- A time the user corrected is kept; the file's own is
            -- kept beside it, to go back to.
            capture_date = CASE WHEN photos.capture_edited = 1 THEN photos.capture_date
                                ELSE excluded.capture_date END,
            original_capture_date = CASE WHEN photos.capture_edited = 1 THEN excluded.capture_date
                                         ELSE NULL END,
            camera = excluded.camera,
            lens = excluded.lens,
            iso = excluded.iso,
            aperture = excluded.aperture,
            shutter = excluded.shutter,
            focal_length = excluded.focal_length,
            width = excluded.width,
            height = excluded.height,
            orientation = excluded.orientation,
            thumb_path = excluded.thumb_path,
            preview_path = excluded.preview_path,
            sharpness = excluded.sharpness,
            brightness = excluded.brightness",
        params![
            p.path,
            p.filename,
            p.file_hash,
            p.capture_date,
            p.camera,
            p.lens,
            p.iso,
            p.aperture,
            p.shutter,
            p.focal_length,
            p.width,
            p.height,
            p.orientation,
            p.thumb_path,
            p.preview_path,
            p.sharpness,
            chrono::Utc::now().to_rfc3339(),
            p.gps.map(|g| g.0),
            p.gps.map(|g| g.1),
            p.brightness,
            p.stamp.map(|s| s.0),
            p.stamp.map(|s| s.1),
        ],
    )?;
    let id: i32 = conn.query_row(
        "SELECT id FROM photos WHERE path = ?1 AND variant = 0",
        params![p.path],
        |r| r.get(0),
    )?;
    if let Some(old) = before {
        if old.hash.as_deref() != Some(p.file_hash.as_str()) {
            forget_old_content(conn, id, &old)?;
        }
    }
    Ok(id)
}

/// A photo's file hash, `None` if there is no such photo: what the
/// full-resolution cache is keyed by beside the id.
pub fn photo_hash(conn: &Connection, id: i32) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(file_hash, '') FROM photos WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?)
}

/// A catalogued file's content before a rescan, for [`forget_old_content`].
struct OldContent {
    hash: Option<String>,
    gps: Option<(f64, f64)>,
    /// Its thumbnail and preview.
    derivatives: [Option<String>; 2],
}

/// Forgets what was worked out from a file's old content once it changed in
/// place: saved over after a crop, a face blurred, GPS stripped.
/// Kept, the person cropped out was still listed in People and named in
/// exports, with the old AI description and the town the old GPS gave.
///
/// Its faces are detected again, as a re-detect does: not deleted
/// first, so [`store_faces`] carries who each was — names, merges, "not the
/// same person" — over to the face found in the same place, and only a face
/// no longer there goes. Deleting them first emptied every person seen only
/// in such photos, and [`recount_people`] then deleted the named ones. Its
/// CLIP vector, description, AI keywords and colours go, and the place its
/// old GPS put it in. The analysis passes pick up what is cleared and work it
/// out again from the new content. Faces added by hand stay — the user's
/// say-so, not the detector's — and so do keywords, ratings and edits. The
/// old thumbnail, preview and face portraits are deleted if no other photo
/// shares them.
fn forget_old_content(conn: &Connection, id: i32, old: &OldContent) -> Result<()> {
    // A virtual copy is the same file, so its own analysis goes too.
    conn.execute(
        "UPDATE photos SET faces_at = NULL, face_count = NULL, face_sharpness = NULL,
                           eyes_open = NULL, caption = NULL, tagged_at = NULL
          WHERE id = ?1 OR variant_of = ?1",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM photo_embeddings
          WHERE photo_id IN (SELECT id FROM photos WHERE id = ?1 OR variant_of = ?1)",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM photo_tags WHERE source = 'ai'
            AND photo_id IN (SELECT id FROM photos WHERE id = ?1 OR variant_of = ?1)",
        params![id],
    )?;
    conn.execute(
        "DELETE FROM photo_palette
          WHERE photo_id IN (SELECT id FROM photos WHERE id = ?1 OR variant_of = ?1)",
        params![id],
    )?;
    // Only the place the old GPS gives: one the user chose stays. Placing
    // runs again on whatever GPS the file has now.
    if let Some((name, _, _)) = old
        .gps
        .and_then(|(lat, lon)| crate::geo::place_for(lat, lon))
    {
        conn.execute(
            "DELETE FROM photo_places
              WHERE photo_id = ?1 AND place_id IN (SELECT id FROM places WHERE name = ?2)",
            params![id, name],
        )?;
    }
    conn.execute(
        "UPDATE photos SET place_auto = 0 WHERE id = ?1",
        params![id],
    )?;
    for path in old.derivatives.iter().flatten() {
        drop_unused_derivative(conn, path)?;
    }
    Ok(())
}

/// Deletes a thumbnail or preview no photo refers to any more, and the face
/// portraits cut from it (`<cache>/faces/<stem>_*.jpg`). By path, never by
/// hash: duplicates at two paths, and virtual copies, share one file.
pub fn drop_unused_derivative(conn: &Connection, path: &str) -> Result<()> {
    let used: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM photos WHERE thumb_path = ?1 OR preview_path = ?1)",
        params![path],
        |r| r.get(0),
    )?;
    if used {
        return Ok(());
    }
    let file = Path::new(path);
    let _ = std::fs::remove_file(file);
    if let (Some(dir), Some(stem)) = (file.parent(), file.file_stem()) {
        let prefix = format!("{}_", stem.to_string_lossy());
        if let Ok(entries) = std::fs::read_dir(dir.join("faces")) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    Ok(())
}

/// Every photo id, for whole-library operations like a bulk XMP import/export.
pub fn all_photo_ids(conn: &Connection) -> Result<Vec<i32>> {
    let mut stmt = conn.prepare("SELECT id FROM photos ORDER BY id")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Makes a virtual copy of `photo_id`: a second row on the same raw file,
/// carrying the edits it has now and nothing else.
///
/// Everything the file itself determines — path, hash, EXIF, thumbnails — is
/// copied, because it is the same photo. Everything the *user* determined about
/// the original is not: a copy starts unrated, unflagged, unlabelled and
/// untagged, because the whole reason to make one is to take the photo
/// somewhere the original is not going.
///
/// Returns the new row's id. Copying a copy is allowed and attaches the new one
/// to the same master, so a photo has one flat set of variants rather than a
/// tree nobody asked for.
pub fn create_variant(conn: &mut Connection, photo_id: i32, name: Option<&str>) -> Result<i32> {
    let tx = conn.transaction()?;
    let id = create_variant_in(&tx, photo_id, name)?;
    // Who made it, and on what terms, is true of every copy; a title
    // and caption are the user's, and a copy starts without them.
    tx.execute(
        "INSERT INTO photo_iptc (photo_id, creator, copyright, credit, usage_terms)
         SELECT ?2, creator, copyright, credit, usage_terms FROM photo_iptc
          WHERE photo_id = ?1
            AND COALESCE(creator, copyright, credit, usage_terms) IS NOT NULL",
        params![photo_id, id],
    )?;
    tx.commit()?;
    Ok(id)
}

/// [`create_variant`]'s work, inside a transaction the caller already holds —
/// so [`save_as_copy`] can make the copy and restore the original atomically.
fn create_variant_in(tx: &Connection, photo_id: i32, name: Option<&str>) -> Result<i32> {
    let master: i32 = tx.query_row(
        "SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1",
        params![photo_id],
        |r| r.get(0),
    )?;
    // Next free variant number for this file. Reusing a gap would collide with
    // UNIQUE(path, variant) only if the gap were not a gap, so max + 1 it is.
    let next: i32 = tx.query_row(
        "SELECT COALESCE(MAX(variant), 0) + 1 FROM photos
         WHERE COALESCE(variant_of, id) = ?1",
        params![master],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO photos (
             path, filename, file_hash, capture_date, camera, lens, iso, aperture,
             shutter, focal_length, width, height, orientation, thumb_path, preview_path,
             develop_settings, indexed_at, variant, variant_of, variant_name,
             capture_edited, original_capture_date
         )
         SELECT path, filename, file_hash, capture_date, camera, lens, iso, aperture,
                shutter, focal_length, width, height, orientation, thumb_path, preview_path,
                develop_settings, ?2, ?3, ?4, ?5, capture_edited, original_capture_date
         FROM photos WHERE id = ?1",
        params![
            photo_id,
            chrono::Utc::now().to_rfc3339(),
            next,
            master,
            name.map(str::trim).filter(|n| !n.is_empty()),
        ],
    )?;
    Ok(tx.last_insert_rowid() as i32)
}

/// Keeps `edits` as a virtual copy of `photo_id` and puts the original back to
/// `original`, returning the copy's id — "Save as copy" in Develop.
///
/// Develop saves as it goes, so the original's row already holds the
/// edits on screen by the time this runs, and the only record of how it looked
/// before is the session's baseline, passed in as `original`. One transaction,
/// so a failure can never leave the original carrying the new look *and* no
/// copy to show for it.
pub fn save_as_copy(
    conn: &mut Connection,
    photo_id: i32,
    edits: &EditStack,
    original: &EditStack,
    name: Option<&str>,
) -> Result<i32> {
    let tx = conn.transaction()?;
    set_develop_settings(&tx, photo_id, edits)?;
    let copy = create_variant_in(&tx, photo_id, name)?;
    set_develop_settings(&tx, photo_id, original)?;
    tx.commit()?;
    Ok(copy)
}

/// Every catalogue row for the file at `path` — the photo and its virtual
/// copies — with its flag, so a caller can check they are all rejects before
/// anything touches the file.
pub fn rows_for_path(conn: &Connection, path: &str) -> Result<Vec<(i32, i32)>> {
    let mut stmt = conn.prepare("SELECT id, flag FROM photos WHERE path = ?1")?;
    let rows = stmt
        .query_map(params![path], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Catalogue rows whose path is another name for `file`: the same file
/// reached by a different path, by the file system's own identity (volume and
/// file index), not by string. Candidates share its file name, which a share
/// and its mapped drive, or two spellings of a drive root, always do.
/// `(path, (id, flag))`.
pub fn same_file_rows(conn: &Connection, file: &Path) -> Result<Vec<(String, (i32, i32))>> {
    let Some(name) = file.file_name().map(|n| n.to_string_lossy().to_string()) else {
        return Ok(Vec::new());
    };
    let own = file.to_string_lossy().to_string();
    let mut stmt = conn.prepare(
        "SELECT path, id, flag FROM photos WHERE filename = ?1 COLLATE NOCASE AND path != ?2",
    )?;
    let rows = stmt
        .query_map(params![name, own], |r| {
            Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(path, _)| same_file::is_same_file(file, path).unwrap_or(false))
        .collect())
}

/// Forgets the photos at `paths` — every row, copies included — once their
/// files have gone to the Recycle Bin. Returns how many rows went.
///
/// Tags, places and collection links cascade from `photos`; faces do not, so
/// they are removed here, and the people they made up are recounted: a person
/// left with no faces goes, and one whose cover face went gets another.
pub fn remove_photos_at(conn: &mut Connection, paths: &[String]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut removed = 0;
    // Their previews and face portraits go with them once nothing else uses
    // them: a 1600 px copy of every binned frame stayed for ever.
    let mut derived: Vec<String> = Vec::new();
    for path in paths {
        derived.extend(derivatives_where(&tx, "path = ?1", &[path])?);
        tx.execute(
            "DELETE FROM faces WHERE photo_id IN (SELECT id FROM photos WHERE path = ?1)",
            params![path],
        )?;
        // Counted first: deleting the photo cascades to its copies, and SQLite
        // does not count cascaded rows among a statement's changes.
        let rows: i64 = tx.query_row(
            "SELECT COUNT(*) FROM photos WHERE path = ?1",
            params![path],
            |r| r.get(0),
        )?;
        tx.execute("DELETE FROM photos WHERE path = ?1", params![path])?;
        removed += rows as usize;
    }
    recount_people(&tx)?;
    tx.commit()?;
    derived.sort();
    derived.dedup();
    for path in &derived {
        drop_unused_derivative(conn, path)?;
    }
    Ok(removed)
}

/// Forgets the virtual copies among `ids`, leaving their file and its
/// original alone. Any other id is ignored. Returns how many rows went.
///
/// A copy has no faces of its own, so there are none to remove. Its
/// thumbnail and preview are usually its original's and stay with it.
pub fn remove_copies(conn: &mut Connection, ids: &[i32]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut removed = 0;
    let mut derived: Vec<String> = Vec::new();
    for id in ids {
        derived.extend(derivatives_where(
            &tx,
            "id = ?1 AND variant_of IS NOT NULL",
            &[id],
        )?);
        removed += tx.execute(
            "DELETE FROM photos WHERE id = ?1 AND variant_of IS NOT NULL",
            params![id],
        )?;
    }
    tx.commit()?;
    derived.sort();
    derived.dedup();
    for path in &derived {
        drop_unused_derivative(conn, path)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Deleting a copy leaves the original, and deleting an original this
    /// way does nothing: that goes through the Recycle Bin instead.
    #[test]
    fn removing_a_copy_keeps_its_original() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (1, '/lib/a.arw', 'a.arw', 'h1', '2026-01-01');",
        )
        .unwrap();
        let copy = create_variant(&mut conn, 1, None).unwrap();

        assert_eq!(remove_copies(&mut conn, &[1, copy]).unwrap(), 1);
        assert!(get_photo(&conn, copy).unwrap().is_none());
        assert!(get_photo(&conn, 1).unwrap().is_some());
    }

    /// Binning a reject forgets it completely — copies, tags, faces — and the
    /// people its faces made up are recounted, not left pointing at nothing.
    #[test]
    fn removing_a_binned_photo_takes_its_faces_and_fixes_people() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, flag)
                 VALUES (1, '/lib/a.arw', 'a.arw', 'h1', '2026-01-01', -1),
                        (2, '/lib/b.arw', 'b.arw', 'h2', '2026-01-01', 1);
             INSERT INTO faces (id, photo_id, x, y, w, h, person_id)
                 VALUES (10, 1, 0, 0, 1, 1, 100),
                        (11, 1, 0, 0, 1, 1, 200),
                        (12, 2, 0, 0, 1, 1, 200);
             INSERT INTO people (id, name, cover_face_id, size)
                 VALUES (100, 'Only in a', 10, 1),
                        (200, 'In both', 11, 2);",
        )
        .unwrap();
        let copy = create_variant(&mut conn, 1, None).unwrap();
        assert_eq!(rows_for_path(&conn, "/lib/a.arw").unwrap().len(), 2);

        let removed = remove_photos_at(&mut conn, &["/lib/a.arw".to_string()]).unwrap();
        assert_eq!(removed, 2, "the photo and its copy");
        assert!(get_photo(&conn, 1).unwrap().is_none());
        assert!(get_photo(&conn, copy).unwrap().is_none());
        assert!(
            get_photo(&conn, 2).unwrap().is_some(),
            "other photos untouched"
        );

        let faces: i64 = conn
            .query_row("SELECT COUNT(*) FROM faces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(faces, 1);
        // A person with no faces left is gone; one with faces left is
        // recounted and re-covered from what remains.
        let people: Vec<(i64, i64, i64)> = conn
            .prepare("SELECT id, size, cover_face_id FROM people")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(people, vec![(200, 1, 12)]);
    }

    /// "Save as copy" after Develop has already saved the new look onto the
    /// original: the copy gets the look, the original gets its old self
    /// back.
    #[test]
    fn save_as_copy_keeps_the_look_on_the_copy_and_restores_the_original() {
        let mut conn = open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, '/lib/a.arw', 'a.arw', 'h1', '2026-01-01')",
            [],
        )
        .unwrap();
        let before = EditStack {
            exposure: 0.3,
            ..EditStack::default()
        };
        let look = EditStack {
            exposure: 1.2,
            ..EditStack::default()
        };
        // What autosave leaves behind: the look is already on the original.
        set_develop_settings(&conn, 1, &look).unwrap();

        let copy = save_as_copy(&mut conn, 1, &look, &before, Some("Bright")).unwrap();

        assert_eq!(
            get_develop_settings(&conn, copy).unwrap().unwrap().exposure,
            1.2
        );
        assert_eq!(
            get_develop_settings(&conn, 1).unwrap().unwrap().exposure,
            0.3
        );

        // An original that was unedited when opened goes back to no stack at
        // all, which is how an unedited photo is stored.
        let copy2 = save_as_copy(&mut conn, 1, &look, &EditStack::default(), None).unwrap();
        assert!(get_develop_settings(&conn, 1).unwrap().is_none());
        assert_eq!(
            get_develop_settings(&conn, copy2)
                .unwrap()
                .unwrap()
                .exposure,
            1.2
        );
    }

    #[test]
    fn a_virtual_copy_shares_the_file_and_owns_its_own_edits() {
        let mut conn = open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, rating, flag)
             VALUES (1, '/lib/a.arw', 'a.arw', 'h1', '2026-01-01', 5, 1)",
            [],
        )
        .unwrap();
        let stack = EditStack {
            exposure: 0.8,
            ..EditStack::default()
        };
        set_develop_settings(&conn, 1, &stack).unwrap();

        let copy = create_variant(&mut conn, 1, Some("Mono")).unwrap();
        assert_ne!(copy, 1);

        let (path, variant, of, name): (String, i32, i32, String) = conn
            .query_row(
                "SELECT path, variant, variant_of, variant_name FROM photos WHERE id = ?1",
                params![copy],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(path, "/lib/a.arw", "a copy renders from the same raw file");
        assert_eq!((variant, of, name.as_str()), (1, 1, "Mono"));

        // It carries the edits it was made from...
        assert_eq!(
            get_develop_settings(&conn, copy).unwrap().unwrap().exposure,
            0.8
        );
        // ...and none of the decisions the user made about the original, which
        // is the point of taking the photo somewhere else.
        let (rating, flag): (i64, i64) = conn
            .query_row(
                "SELECT rating, flag FROM photos WHERE id = ?1",
                params![copy],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((rating, flag), (0, 0));

        // A second copy gets the next number, and a copy of a copy attaches to
        // the same master rather than building a tree.
        let third = create_variant(&mut conn, copy, None).unwrap();
        let (variant, of): (i32, i32) = conn
            .query_row(
                "SELECT variant, variant_of FROM photos WHERE id = ?1",
                params![third],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((variant, of), (2, 1));
    }

    #[test]
    fn copies_are_kept_out_of_the_ai_queues_and_out_of_the_scanner_s_way() {
        let mut conn = open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, thumb_path)
             VALUES (1, '/lib/a.arw', 'a.arw', 'h1', '2026-01-01', '/t/a.jpg')",
            [],
        )
        .unwrap();
        create_variant(&mut conn, 1, None).unwrap();

        // A copy shares its master's pixels: tagging, embedding, captioning,
        // scoring or detecting faces on it again is pure waste, and for faces it
        // would count every person in the frame twice.
        assert_eq!(untagged_photos(&conn, 10).unwrap().len(), 1);
        assert_eq!(unindexed_count(&conn).unwrap(), 1);
        assert_eq!(unscored_count(&conn).unwrap(), 1);
        assert_eq!(unanalysed_count(&conn).unwrap(), 1);
        assert_eq!(uncaptioned_count(&conn).unwrap(), 1);

        // And the scan still sees the file as already indexed — the copy must
        // not make a rescan think it found something new, nor be updated by one.
        assert!(photo_exists(&conn, "/lib/a.arw", "h1").unwrap());
    }

    #[test]
    fn deleting_a_photo_takes_its_copies_with_it() {
        // A copy has no file of its own; leaving it behind would be a row that
        // renders from nothing.
        let mut conn = open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, '/lib/a.arw', 'a.arw', 'h1', '2026-01-01')",
            [],
        )
        .unwrap();
        create_variant(&mut conn, 1, None).unwrap();
        conn.execute("DELETE FROM photos WHERE id = 1", []).unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    /// A binned photo's thumbnail, preview and face portraits are deleted,
    /// unless another photo still uses them.
    #[test]
    fn binned_photos_take_their_previews_with_them() {
        let dir = crate::test_support::Scratch::new("derivatives-gc");
        std::fs::create_dir_all(dir.join("faces")).unwrap();
        let file = |name: &str| {
            let p = dir.join(name);
            std::fs::write(&p, b"jpeg").unwrap();
            p.to_string_lossy().to_string()
        };
        let (own, shared) = (file("own_p.jpg"), file("shared_p.jpg"));
        let portrait = dir.join("faces").join("own_p_1_2_3_4.jpg");
        std::fs::write(&portrait, b"jpeg").unwrap();
        let mut conn = open_memory().unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, preview_path)
             VALUES (1, 'D:/a.arw', 'a.arw', 'h1', 'x', '{own}'),
                    (2, 'D:/b.arw', 'b.arw', 'h2', 'x', '{shared}'),
                    (3, 'E:/b.arw', 'b.arw', 'h2', 'x', '{shared}');"
        ))
        .unwrap();
        remove_photos_at(&mut conn, &["D:/a.arw".into(), "D:/b.arw".into()]).unwrap();
        assert!(!Path::new(&own).exists());
        assert!(!portrait.exists());
        assert!(Path::new(&shared).exists(), "photo 3 still shows it");
    }

    /// After a restore, photos whose previews are gone are marked for the
    /// next scan to make them again.
    #[test]
    fn a_restored_catalogue_asks_again_for_missing_previews() {
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, preview_path, file_size)
             VALUES (1, 'D:/a.arw', 'a.arw', 'h1', 'x', 'Z:/nowhere/a_p.jpg', 10),
                    (2, 'D:/b.arw', 'b.arw', 'h2', 'x', NULL, 10);",
        )
        .unwrap();
        assert_eq!(rederive_missing(&conn).unwrap(), 1);
        let size: i64 = conn
            .query_row("SELECT file_size FROM photos WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(size, REDERIVE);
    }

    /// A file saved over in place loses what was worked out from its
    /// old content — the person cropped out, the old description, the AI
    /// keywords, the colours, the town its old GPS gave — and keeps what the
    /// user said: their keywords and the faces they added by hand.
    #[test]
    fn a_file_saved_over_forgets_what_its_old_content_said() {
        let conn = open_memory().unwrap();
        dated(&conn, 1, "a.jpg", None);
        let (town, _, _) = crate::geo::place_for(51.5, -0.12).expect("a town for London");
        conn.execute_batch(
            "UPDATE photos SET caption = 'a sign reading 14 Elm Road', tagged_at = 'x',
                               faces_at = 'x', face_count = 1, lat = 51.5, lon = -0.12,
                               place_auto = 1
              WHERE id = 1;
             INSERT INTO people (id, name, size, cover_face_id) VALUES (7, 'Anna', 2, 10);
             INSERT INTO faces (id, photo_id, x, y, w, h, person_id) VALUES (10, 1, 0, 0, 90, 90, 7);
             INSERT INTO faces (id, photo_id, x, y, w, h, person_id, manual)
                  VALUES (11, 1, 0, 0, 0, 0, 7, 1);
             INSERT INTO tags (id, name, category) VALUES (91, 'beer', 'subject'), (92, 'mine', 'subject');
             INSERT INTO photo_tags (photo_id, tag_id, confidence, source)
                  VALUES (1, 91, 0.5, 'ai'), (1, 92, 1.0, 'manual');
             INSERT INTO photo_palette (photo_id, swatches, signature) VALUES (1, 1, x'00');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO places (id, name, lat, lon) VALUES (5, ?1, 51.5, -0.12)",
            params![town],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO photo_places (photo_id, place_id) VALUES (1, 5)",
            [],
        )
        .unwrap();
        let rescan = |hash: &str, gps: Option<(f64, f64)>| {
            upsert_photo(
                &conn,
                &crate::scan::IngestedPhoto {
                    path: "a.jpg".into(),
                    filename: "a.jpg".into(),
                    file_hash: hash.into(),
                    capture_date: None,
                    camera: None,
                    lens: None,
                    iso: None,
                    aperture: None,
                    shutter: None,
                    focal_length: None,
                    width: 6000,
                    height: 4000,
                    orientation: 1,
                    thumb_path: None,
                    preview_path: None,
                    sharpness: None,
                    brightness: None,
                    gps,
                    stamp: None,
                },
            )
            .unwrap()
        };
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };

        // The same content: nothing is forgotten.
        assert_eq!(rescan("a.jpg", Some((51.5, -0.12))), 1);
        assert_eq!(count("SELECT COUNT(*) FROM faces"), 2);
        assert_eq!(count("SELECT COUNT(caption) FROM photos"), 1);

        // Saved over, GPS stripped.
        assert_eq!(rescan("new content", None), 1);
        assert_eq!(
            count("SELECT COUNT(caption) + COUNT(tagged_at) + COUNT(faces_at) FROM photos"),
            0
        );
        // Its faces stay until the face pass looks again (`faces_at` is
        // cleared above), so it can carry Anna over to the face it finds in
        // the same place: she, and her name, are not deleted.
        assert_eq!(count("SELECT COUNT(*) FROM faces"), 2);
        assert_eq!(count("SELECT size FROM people WHERE id = 7"), 2);
        assert_eq!(
            count("SELECT tag_id FROM photo_tags"),
            92,
            "the user's keyword stays"
        );
        assert_eq!(count("SELECT COUNT(*) FROM photo_palette"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM photo_places"), 0);
        assert_eq!(count("SELECT place_auto FROM photos"), 0);
    }

    /// A row for the same file under another path is found by the file's
    /// identity, not its path string.
    #[test]
    fn the_same_file_under_another_path_is_found() {
        let dir = crate::test_support::Scratch::new("db-same-file");
        let a = dir.join("DSC1.ARW");
        std::fs::write(&a, "raw").unwrap();
        let alias_dir = dir.join("alias");
        std::fs::create_dir_all(&alias_dir).unwrap();
        let b = alias_dir.join("DSC1.ARW");
        std::fs::hard_link(&a, &b).unwrap();
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let c = other.join("DSC1.ARW");
        std::fs::write(&c, "raw").unwrap();
        let conn = open_memory().unwrap();
        for (id, p, flag) in [(1, &a, -1), (2, &b, 1), (3, &c, 0)] {
            conn.execute(
                "INSERT INTO photos (id, path, filename, file_hash, indexed_at, flag)
                 VALUES (?1, ?2, 'DSC1.ARW', 'h', 'x', ?3)",
                params![id, p.to_string_lossy(), flag],
            )
            .unwrap();
        }
        let found = same_file_rows(&conn, &a).unwrap();
        assert_eq!(found, vec![(b.to_string_lossy().to_string(), (2, 1))]);
    }
}
