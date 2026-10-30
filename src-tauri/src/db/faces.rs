//! Faces: what the face pass stores for each photo, eye readings, and
//! listing faces.

use super::*;

/// Clears the face pass's marks so it re-runs over the whole library.
///
/// Needed whenever what the pass *records* changes, not just how well it
/// detects: a face row from before its image size was stored cannot be turned
/// into a fraction, so it cannot be cropped to, and no amount of regrouping
/// will fix that. Deletes the faces too — leaving them would mean the queue
/// re-detects into rows that already exist.
pub fn reset_faces(conn: &Connection) -> Result<usize> {
    // Every photo is detected again; nothing is deleted first. Each
    // photo's faces are replaced as it is re-analysed, and `store_faces`
    // carries their people, links and "not the same" answers over to the
    // faces found in the same place. Deleting them all first cascaded every
    // merge away, and took the faces added by hand with them, while the
    // prompt promised merges were kept.
    let n = conn.execute(
        "UPDATE photos SET faces_at = NULL WHERE faces_at IS NOT NULL",
        [],
    )?;
    Ok(n)
}

/// Photos not yet analysed for faces (`faces_at IS NULL`), as (id, preview path)
/// — the queue for the face pass. Mirrors [`unscored_photos`].
pub fn unanalysed_photos(conn: &Connection, limit: i32) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(preview_path, thumb_path) FROM photos
         WHERE faces_at IS NULL AND variant = 0 AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many photos still need the face pass — the "N of M analysed" denominator.
pub fn unanalysed_count(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM photos
         WHERE faces_at IS NULL AND variant = 0 AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)",
        [],
        |r| r.get(0),
    )?)
}

/// A photo whose faces were found before the face mesh read eyes: its
/// preview, and each such face's id and box in the preview's pixels.
pub type EyesToRead = (i32, String, Vec<(i64, [f32; 4])>);

/// Photos with detected faces whose eyes haven't been read with the face
/// mesh yet, and those faces.
pub fn photos_needing_eyes(conn: &Connection) -> Result<Vec<EyesToRead>> {
    let mut stmt = conn.prepare(
        "SELECT ph.id, COALESCE(ph.preview_path, ph.thumb_path), f.id, f.x, f.y, f.w, f.h
           FROM faces f JOIN photos ph ON ph.id = f.photo_id
          WHERE f.eyes_model = 0 AND f.manual = 0 AND ph.variant = 0
            AND (ph.preview_path IS NOT NULL OR ph.thumb_path IS NOT NULL)
          ORDER BY ph.id, f.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i32>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            [
                r.get::<_, f64>(3)? as f32,
                r.get::<_, f64>(4)? as f32,
                r.get::<_, f64>(5)? as f32,
                r.get::<_, f64>(6)? as f32,
            ],
        ))
    })?;
    let mut out: Vec<EyesToRead> = Vec::new();
    for row in rows {
        let (photo, path, face, bx) = row?;
        match out.last_mut() {
            Some((p, _, faces)) if *p == photo => faces.push((face, bx)),
            _ => out.push((photo, path, vec![(face, bx)])),
        }
    }
    Ok(out)
}

/// How many photos [`photos_needing_eyes`] would return.
pub fn eyes_pending_count(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row(
        "SELECT COUNT(DISTINCT f.photo_id) FROM faces f JOIN photos ph ON ph.id = f.photo_id
          WHERE f.eyes_model = 0 AND f.manual = 0 AND ph.variant = 0
            AND (ph.preview_path IS NOT NULL OR ph.thumb_path IS NOT NULL)",
        [],
        |r| r.get(0),
    )?)
}

/// Stores eye readings for a photo's faces (`None`: can't tell), marks them
/// read, and resets the photo's worst-eye summary from its readable faces.
pub fn store_eye_readings(
    conn: &mut Connection,
    photo_id: i32,
    readings: &[(i64, Option<f32>)],
) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut upd = tx.prepare("UPDATE faces SET eye_open = ?2, eyes_model = 1 WHERE id = ?1")?;
        for (face, eye) in readings {
            upd.execute(params![face, eye])?;
        }
    }
    tx.execute(
        "UPDATE photos SET eyes_open =
             (SELECT MIN(eye_open) FROM faces WHERE photo_id = ?1 AND manual = 0)
          WHERE id = ?1",
        params![photo_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// A face row as [`store_faces`] finds it before replacing it: id, box,
/// person, detached flag, and the width/height of the image it was measured on.
type OldFace = (i64, [f32; 4], Option<i64>, i64, f64, f64);

/// Photos already analysed whose faces have no identity embedding: analysed
/// while the People recogniser was not installed. As (id, preview).
pub fn unembedded_photos(conn: &Connection) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(preview_path, thumb_path) FROM photos
          WHERE faces_at IS NOT NULL AND variant = 0
            AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)
            AND EXISTS (SELECT 1 FROM faces f WHERE f.photo_id = photos.id AND f.manual = 0
                           AND NOT EXISTS (SELECT 1 FROM face_embeddings e WHERE e.face_id = f.id))
          ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many photos [`unembedded_photos`] would return.
pub fn unembedded_count(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM photos
          WHERE faces_at IS NOT NULL AND variant = 0
            AND (preview_path IS NOT NULL OR thumb_path IS NOT NULL)
            AND EXISTS (SELECT 1 FROM faces f WHERE f.photo_id = photos.id AND f.manual = 0
                           AND NOT EXISTS (SELECT 1 FROM face_embeddings e WHERE e.face_id = f.id))",
        [],
        |r| r.get(0),
    )?)
}

/// Records a photo's face-analysis result: replaces its `faces` rows, stamps
/// `faces_at`, and caches the per-photo aggregates the cull UI reads. Runs in one
/// transaction so a photo is never left half-analysed. `faces` empty is a valid
/// result (analysed, nobody in frame) and still stamps `faces_at`.
pub fn store_faces(
    conn: &mut Connection,
    photo_id: i32,
    faces: &[crate::faces::Face],
    embeddings: &[Vec<f32>],
    // `image_size` is the image the detector ran on — the 1600px preview, not
    // the capture. Without it a box cannot be turned into a fraction, and a
    // fraction is the only form that is right against every rendition.
    image_size: (u32, u32),
) -> Result<()> {
    let summary = crate::faces::summarize(faces);
    let tx = conn.transaction()?;
    // What the photo's faces were before: re-analysing (a better model, a
    // re-detect) must not throw away who the user said anyone was.
    let old: Vec<OldFace> = {
        let mut stmt = tx.prepare(
            "SELECT id, x, y, w, h, person_id, detached, COALESCE(img_w, 0), COALESCE(img_h, 0)
               FROM faces WHERE photo_id = ?1 AND manual = 0",
        )?;
        let rows = stmt.query_map(params![photo_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                [
                    r.get::<_, f64>(1)? as f32,
                    r.get::<_, f64>(2)? as f32,
                    r.get::<_, f64>(3)? as f32,
                    r.get::<_, f64>(4)? as f32,
                ],
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, f64>(7)?,
                r.get::<_, f64>(8)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // The user's "same person" links touching those faces, read before the
    // delete below cascades them away. They used to be re-pointed
    // after it, at rows that were already gone, so re-detecting a photo lost
    // every merge it was part of.
    let old_links: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT a, b FROM face_links
              WHERE a IN (SELECT id FROM faces WHERE photo_id = ?1 AND manual = 0)
                 OR b IN (SELECT id FROM faces WHERE photo_id = ?1 AND manual = 0)",
        )?;
        let rows = stmt.query_map(params![photo_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // Only what the detector found is replaced: a person the user put in the
    // photo by hand is their say-so, not a detection to redo.
    tx.execute(
        "DELETE FROM faces WHERE photo_id = ?1 AND manual = 0",
        params![photo_id],
    )?;
    let mut moved: Vec<(i64, i64)> = Vec::new();
    // Each new face: its id, its box as fractions, and whether it may take
    // over a name waiting for it (see `take_over_waiting_faces`).
    let mut fresh: Vec<(i64, [f32; 4], bool)> = Vec::new();
    {
        let mut ins = tx.prepare(
            "INSERT INTO faces
                 (photo_id, x, y, w, h, score, sharpness, eye_open, img_w, img_h,
                  pose, person_id, detached, eyes_model)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?;
        let mut ins_embedding =
            tx.prepare("INSERT INTO face_embeddings (face_id, embedding) VALUES (?1, ?2)")?;
        let mut taken = vec![false; old.len()];
        for (i, f) in faces.iter().enumerate() {
            // The old face this one is: the best box overlap above half, in
            // the old row's own image scale (the preview may have been
            // re-rendered at another size).
            let matched = old
                .iter()
                .enumerate()
                .filter(|(k, _)| !taken[*k])
                .map(|(k, o)| {
                    let scale = if o.4 > 0.0 && image_size.0 > 0 {
                        image_size.0 as f32 / o.4 as f32
                    } else {
                        1.0
                    };
                    let b = [
                        o.1[0] * scale,
                        o.1[1] * scale,
                        o.1[2] * scale,
                        o.1[3] * scale,
                    ];
                    (k, crate::faces::iou(f.bbox(), b))
                })
                .filter(|&(_, v)| v >= 0.5)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| k);
            if let Some(k) = matched {
                taken[k] = true;
            }
            let (person, detached) = matched.map_or((None, 0), |k| (old[k].2, old[k].3));
            // Parallel to `faces`; an empty vec (embedder absent) stores NULL.
            let blob = embeddings
                .get(i)
                .filter(|e| !e.is_empty())
                .map(|e| crate::search::encode_embedding(e));
            ins.execute(params![
                photo_id,
                f.x,
                f.y,
                f.w,
                f.h,
                // Detector confidence was computed and then discarded here.
                // Grouping needs it: a weak detection is where the embeddings
                // that chain two people together come from.
                f.score,
                f.sharpness,
                f.eye_open,
                image_size.0 as f64,
                image_size.1 as f64,
                crate::people::pose_offset(&f.landmarks),
                person,
                detached,
                i64::from(f.eyes_read),
            ])?;
            let id = tx.last_insert_rowid();
            if let Some(blob) = &blob {
                ins_embedding.execute(params![id, blob])?;
            }
            if let Some(k) = matched {
                moved.push((old[k].0, id));
            }
            let (iw, ih) = (image_size.0.max(1) as f32, image_size.1.max(1) as f32);
            let pose = crate::people::pose_offset(&f.landmarks);
            let free = person.is_none()
                && blob.is_some()
                && crate::people::votes(Some(f.score), f.w, f.h, Some(pose));
            fresh.push((id, [f.x / iw, f.y / ih, f.w / iw, f.h / ih], free));
        }
    }
    take_over_waiting_faces(&tx, photo_id, &fresh)?;
    // Decisions recorded against the old face ids follow the faces. A link
    // whose old face was not found again goes with it.
    {
        let old_ids: std::collections::HashSet<i64> = old.iter().map(|o| o.0).collect();
        let to: std::collections::HashMap<i64, i64> = moved.iter().copied().collect();
        let follow = |id: i64| {
            if old_ids.contains(&id) {
                to.get(&id).copied()
            } else {
                Some(id)
            }
        };
        let mut ins = tx.prepare("INSERT OR IGNORE INTO face_links (a, b) VALUES (?1, ?2)")?;
        for &(a, b) in &old_links {
            if let (Some(a), Some(b)) = (follow(a), follow(b)) {
                ins.execute(params![a, b])?;
            }
        }
    }
    for (from, to) in moved {
        tx.execute(
            "UPDATE OR IGNORE people_apart SET a = ?2 WHERE a = ?1",
            params![from, to],
        )?;
        tx.execute(
            "UPDATE OR IGNORE people_apart SET b = ?2 WHERE b = ?1",
            params![from, to],
        )?;
        tx.execute(
            "UPDATE people SET cover_face_id = ?2 WHERE cover_face_id = ?1",
            params![from, to],
        )?;
    }
    tx.execute(
        "UPDATE photos SET faces_at = ?2, face_count = ?3, face_sharpness = ?4, eyes_open = ?5
         WHERE id = ?1",
        params![
            photo_id,
            chrono::Utc::now().to_rfc3339(),
            summary.count as i32,
            summary.primary_sharpness,
            summary.min_eye_open,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Sets (or clears) the user's verdict on a photo's eye-open reading.
///
/// `None` returns the photo to whatever the model thinks; `Some(0)` says the
/// eyes are not closed — the downcast gaze the classifier keeps calling a blink
/// — and `Some(1)` confirms they are. Returns the number of photos changed.
pub fn set_eyes_override(conn: &Connection, ids: &[i32], value: Option<i32>) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("UPDATE photos SET eyes_override = ? WHERE id IN ({placeholders})");
    let mut params: Vec<Value> = vec![match value {
        Some(v) => Value::Integer(i64::from(v)),
        None => Value::Null,
    }];
    params.extend(ids.iter().map(|&i| Value::Integer(i64::from(i))));
    Ok(conn.execute(&sql, rusqlite::params_from_iter(params))?)
}

/// A face box as a fraction of the image it was measured in, `[x, y, w, h]`.
///
/// Empty when the image size was not recorded (a row from before migration 12),
/// which the UI reads as "show the whole frame" — a wrong crop is worse than an
/// uncropped one, because it looks like the detector found grass.
pub(super) fn box_fraction(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    dims: (Option<f64>, Option<f64>),
) -> Vec<f32> {
    match dims {
        (Some(iw), Some(ih)) if iw > 0.0 && ih > 0.0 => vec![
            (x / iw) as f32,
            (y / ih) as f32,
            (w / iw) as f32,
            (h / ih) as f32,
        ],
        _ => Vec::new(),
    }
}

/// One face of a person, for the "is this really them?" review grid.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonFace {
    /// `i32`, not the row's native `i64`: specta refuses to export 64-bit
    /// integers (JavaScript would lose precision), and every other id crossing
    /// this boundary is already `i32`. A library would need two billion faces
    /// to notice.
    pub face_id: i32,
    pub photo_id: i32,
    pub thumb_path: Option<String>,
    /// `[x, y, w, h]` as **fractions of the image**, 0..1.
    ///
    /// Not pixels. Detection runs on the 1600px preview while the UI draws on
    /// the 512px thumbnail and the capture is 6000px wide, so a pixel box is
    /// only meaningful alongside the size it was measured in — and passing that
    /// around is how it came to be measured against the wrong one. A fraction
    /// is right against every rendition by construction.
    ///
    /// Empty when the face was analysed before those dimensions were recorded:
    /// the box cannot be placed, and showing the whole frame is better than
    /// cropping to the wrong part of it.
    pub box_frac: Vec<f32>,
    /// A cached portrait of the face (see `faces::portrait`), when one could
    /// be made; the page crops `thumb_path` otherwise.
    pub portrait_path: Option<String>,
    /// True when this face was excluded from grouping — too small or too weakly
    /// detected to be trusted, or detached by hand.
    pub ungrouped: bool,
    /// Put here by the user rather than found by the detector: no box, so the
    /// whole photo stands in for the face.
    pub manual: bool,
}

/// Where to cut a face's portrait from: the image the box was measured on
/// (the preview) and the box in its pixels. For the cover of every person,
/// keyed by person id, or for every face of one person, keyed by face id.
pub fn face_sources(
    conn: &Connection,
    person: Option<i32>,
) -> Result<Vec<(i32, String, [f32; 4])>> {
    let sql = match person {
        None => {
            "SELECT pe.id, ph.preview_path, f.x, f.y, f.w, f.h
               FROM people pe JOIN faces f ON f.id = pe.cover_face_id
               JOIN photos ph ON ph.id = f.photo_id
              WHERE ph.preview_path IS NOT NULL AND f.manual = 0"
        }
        Some(_) => {
            "SELECT f.id, ph.preview_path, f.x, f.y, f.w, f.h
               FROM faces f JOIN photos ph ON ph.id = f.photo_id
              WHERE f.person_id = ?1 AND ph.preview_path IS NOT NULL AND f.manual = 0"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map = |r: &rusqlite::Row| {
        Ok((
            r.get::<_, i32>(0)?,
            r.get::<_, String>(1)?,
            [
                r.get::<_, f64>(2)? as f32,
                r.get::<_, f64>(3)? as f32,
                r.get::<_, f64>(4)? as f32,
                r.get::<_, f64>(5)? as f32,
            ],
        ))
    };
    let rows = match person {
        None => stmt
            .query_map([], map)?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        Some(id) => stmt
            .query_map(params![id], map)?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    };
    Ok(rows)
}

/// The faces attributed to a person, largest first.
pub fn person_faces(conn: &Connection, person_id: i32) -> Result<Vec<PersonFace>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.photo_id, COALESCE(ph.thumb_path, ph.preview_path),
                f.x, f.y, f.w, f.h, f.img_w, f.img_h, f.score, f.detached, f.manual
         FROM faces f JOIN photos ph ON ph.id = f.photo_id
         WHERE f.person_id = ?1
         ORDER BY f.manual, f.w * f.h DESC",
    )?;
    let mapped = stmt.query_map(params![person_id], |r| {
        let (w, h): (f64, f64) = (r.get(5)?, r.get(6)?);
        let dims: (Option<f64>, Option<f64>) = (r.get(7)?, r.get(8)?);
        let score: Option<f64> = r.get(9)?;
        let detached: i64 = r.get(10)?;
        let manual = r.get::<_, i64>(11)? != 0;
        Ok(PersonFace {
            face_id: r.get(0)?,
            photo_id: r.get(1)?,
            thumb_path: r.get(2)?,
            box_frac: box_fraction(r.get(3)?, r.get(4)?, w, h, dims),
            portrait_path: None,
            ungrouped: !manual
                && (detached != 0
                    || !crate::people::is_groupable(score.map(|s| s as f32), w as f32, h as f32)),
            manual,
        })
    })?;
    Ok(mapped.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Detected faces no person holds: too small, too turned away or too
/// faint for grouping to place, so until now nothing showed them. Largest and
/// sharpest first, the ones worth naming; what the user has said is not a
/// face stays out.
pub fn unplaced_faces(conn: &Connection, limit: u32) -> Result<Vec<PersonFace>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.photo_id, COALESCE(ph.thumb_path, ph.preview_path),
                f.x, f.y, f.w, f.h, f.img_w, f.img_h
           FROM faces f JOIN photos ph ON ph.id = f.photo_id
          WHERE f.person_id IS NULL AND f.manual = 0 AND f.dismissed = 0
          ORDER BY f.w * f.h DESC, COALESCE(f.sharpness, 0) DESC
          LIMIT ?1",
    )?;
    let mapped = stmt.query_map(params![limit], |r| {
        let (w, h): (f64, f64) = (r.get(5)?, r.get(6)?);
        Ok(PersonFace {
            face_id: r.get(0)?,
            photo_id: r.get(1)?,
            thumb_path: r.get(2)?,
            box_frac: box_fraction(r.get(3)?, r.get(4)?, w, h, (r.get(7)?, r.get(8)?)),
            portrait_path: None,
            ungrouped: true,
            manual: false,
        })
    })?;
    Ok(mapped.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many faces [`unplaced_faces`] would list, without the limit.
pub fn unplaced_face_count(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM faces
          WHERE person_id IS NULL AND manual = 0 AND dismissed = 0",
        [],
        |r| r.get::<_, i64>(0),
    )? as u32)
}

/// Says a detection is not a face: it leaves the unplaced queue.
pub fn dismiss_face(conn: &Connection, face_id: i32) -> Result<()> {
    conn.execute(
        "UPDATE faces SET dismissed = 1 WHERE id = ?1 AND person_id IS NULL",
        params![face_id],
    )?;
    Ok(())
}

/// The preview and pixel box of each face in `ids`, for cutting portraits.
pub fn face_sources_of(conn: &Connection, ids: &[i32]) -> Result<Vec<(i32, String, [f32; 4])>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, ph.preview_path, f.x, f.y, f.w, f.h
           FROM faces f JOIN photos ph ON ph.id = f.photo_id
          WHERE f.id = ?1 AND ph.preview_path IS NOT NULL",
    )?;
    let mut out = Vec::new();
    for id in ids {
        let row = stmt
            .query_row(params![id], |r| {
                Ok((
                    r.get::<_, i32>(0)?,
                    r.get::<_, String>(1)?,
                    [
                        r.get::<_, f64>(2)? as f32,
                        r.get::<_, f64>(3)? as f32,
                        r.get::<_, f64>(4)? as f32,
                        r.get::<_, f64>(5)? as f32,
                    ],
                ))
            })
            .optional()?;
        out.extend(row);
    }
    Ok(out)
}

/// One detected face in a photo, for Cull's close-ups and Compare's faces
/// across a burst.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhotoFace {
    pub face_id: i32,
    /// The photo it was asked for, even when that is a virtual copy: faces are
    /// stored against the master, and the copy shows the same people.
    pub photo_id: i32,
    /// Grouped into a person, when it has been.
    pub person_id: Option<i32>,
    pub person_name: Option<String>,
    /// `[x, y, w, h]` as fractions of the image — see [`PersonFace::box_frac`].
    pub box_frac: Vec<f32>,
    /// Focus of the face region. Only comparable between faces of about the
    /// same size — the same person across a burst, not two faces in one frame.
    pub sharpness: Option<f32>,
    /// 0 shut … 1 open, the worse of the two eyes.
    pub eye_open: Option<f32>,
    /// A portrait cut from the preview, filled in by the command.
    pub portrait_path: Option<String>,
}

/// A [`PhotoFace`] with where to cut its portrait from: the preview, and the
/// face's box in that preview's pixels.
pub type PhotoFaceSource = (PhotoFace, Option<String>, [f32; 4]);

/// The detected faces in each of `photo_ids` (its master's, for a virtual
/// copy), largest first within a photo.
pub fn photo_faces(conn: &Connection, photo_ids: &[i32]) -> Result<Vec<PhotoFaceSource>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.person_id, pe.name, f.x, f.y, f.w, f.h, f.img_w, f.img_h,
                f.sharpness, f.eye_open, ph.preview_path
           FROM faces f
           JOIN photos ph ON ph.id = f.photo_id
           LEFT JOIN people pe ON pe.id = f.person_id
          WHERE f.manual = 0
            AND f.photo_id = (SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1)
          ORDER BY f.w * f.h DESC, f.id",
    )?;
    let mut out = Vec::new();
    for &photo_id in photo_ids {
        let rows = stmt.query_map(params![photo_id], |r| {
            let (x, y, w, h): (f64, f64, f64, f64) = (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?);
            let face = PhotoFace {
                face_id: r.get(0)?,
                photo_id,
                person_id: r.get(1)?,
                person_name: r.get(2)?,
                box_frac: box_fraction(x, y, w, h, (r.get(7)?, r.get(8)?)),
                sharpness: r.get::<_, Option<f64>>(9)?.map(|v| v as f32),
                eye_open: r.get::<_, Option<f64>>(10)?.map(|v| v as f32),
                portrait_path: None,
            };
            Ok((face, r.get(11)?, [x as f32, y as f32, w as f32, h as f32]))
        })?;
        out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Faces found before the face mesh wait to have their eyes read;
    /// reading them marks them done and resets the photo's worst eye from the
    /// faces that could be read.
    #[test]
    fn eyes_found_before_the_mesh_are_read_again() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 4);
        // As migration 26 leaves an old library: faces kept, readings dropped.
        conn.execute("UPDATE faces SET eyes_model = 0, eye_open = NULL", [])
            .unwrap();
        conn.execute("UPDATE photos SET preview_path = 'p' || id || '.jpg'", [])
            .unwrap();
        let queue = photos_needing_eyes(&conn).unwrap();
        assert_eq!(queue.len(), 3, "photos 1-3 have a face each");
        assert_eq!(eyes_pending_count(&conn).unwrap(), 3);
        let (photo, _, faces) = &queue[0];
        assert_eq!(faces.len(), 1);
        assert!(faces[0].1[2] > 0.0, "the stored box comes along");

        store_eye_readings(&mut conn, *photo, &[(faces[0].0, Some(0.3))]).unwrap();
        let eyes: Option<f64> = conn
            .query_row(
                "SELECT eyes_open FROM photos WHERE id = ?1",
                params![photo],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(eyes, Some(0.3_f32 as f64));
        assert_eq!(eyes_pending_count(&conn).unwrap(), 2);

        // Can't tell is stored too, so the face isn't read again every pass.
        let (photo, _, faces) = &queue[1];
        store_eye_readings(&mut conn, *photo, &[(faces[0].0, None)]).unwrap();
        let eyes: Option<f64> = conn
            .query_row(
                "SELECT eyes_open FROM photos WHERE id = ?1",
                params![photo],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(eyes, None);
        assert_eq!(eyes_pending_count(&conn).unwrap(), 1);
    }

    /// Faces grouping could not place are listed until they're named or
    /// said not to be faces.
    #[test]
    fn unplaced_faces_are_listed_until_named_or_dismissed() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 6);
        let (a, b) = (loose_face(&mut conn, 4), loose_face(&mut conn, 5));
        let listed = |conn: &Connection| -> Vec<i32> {
            unplaced_faces(conn, 50)
                .unwrap()
                .iter()
                .map(|f| f.face_id)
                .collect()
        };
        let mut both = listed(&conn);
        both.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(both, want);
        assert_eq!(unplaced_face_count(&conn).unwrap(), 2);

        dismiss_face(&conn, a).unwrap();
        assert_eq!(listed(&conn), vec![b], "not a face");
        name_face(&mut conn, b, "Ada").unwrap();
        assert!(listed(&conn).is_empty(), "named, so placed");
        assert_eq!(unplaced_face_count(&conn).unwrap(), 0);

        // A face someone holds can't be dismissed by mistake.
        let held = face_on(&conn, 1);
        dismiss_face(&conn, held).unwrap();
        let dismissed: i64 = conn
            .query_row(
                "SELECT dismissed FROM faces WHERE id = ?1",
                params![held],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dismissed, 0);
    }

    /// Faces found while the People model was missing wait for it: once it is
    /// installed their photos are queued again, and embedding them empties
    /// the queue.
    #[test]
    fn faces_without_embeddings_wait_for_the_recogniser() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        conn.execute("UPDATE photos SET preview_path = 'p.jpg'", [])
            .unwrap();
        // Photo 1 analysed without the recogniser; photo 2 found nobody.
        store_faces(
            &mut conn,
            1,
            &[face_at(100.0, 100.0, false)],
            &[],
            (1600, 1067),
        )
        .unwrap();
        store_faces(&mut conn, 2, &[], &[], (1600, 1067)).unwrap();
        assert_eq!(unanalysed_count(&conn).unwrap(), 0);
        assert_eq!(unembedded_count(&conn).unwrap(), 1);
        assert_eq!(
            unembedded_photos(&conn).unwrap(),
            vec![(1, "p.jpg".to_string())]
        );

        store_faces(
            &mut conn,
            1,
            &[face_at(100.0, 100.0, false)],
            &[unit(&[1.0, 0.0])],
            (1600, 1067),
        )
        .unwrap();
        assert_eq!(unembedded_count(&conn).unwrap(), 0);
    }

    /// Storing a photo's faces while another connection is writing: the scan
    /// adding photos, say. The transaction used to be deferred: it read the
    /// photo's old faces, then wrote. When the other writer committed in
    /// between, SQLite could not upgrade the stale read and failed at once
    /// with "database is locked", without waiting out the busy timeout. On a
    /// real library during a startup scan that dropped 7 photos' faces.
    #[test]
    fn storing_faces_waits_for_another_writer() {
        let dir = std::env::temp_dir()
            .join("sourcelight-tests")
            .join("store-faces-busy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.db");
        let setup = open(&path).unwrap();
        let id = photos(&setup, 1)[0];
        drop(setup);

        // The other writer takes the lock and holds it a moment, as a scan
        // inserting a photo does.
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            let mut conn = connect(&writer_path).unwrap();
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            tx.execute(
                "INSERT INTO photos (path, filename, file_hash, indexed_at)
                 VALUES ('/other.arw', 'b.arw', 'h2', '2026-01-01')",
                [],
            )
            .unwrap();
            locked_tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
            tx.commit().unwrap();
        });
        locked_rx.recv().unwrap();

        let mut conn = connect(&path).unwrap();
        let face = crate::faces::Face {
            x: 10.0,
            y: 10.0,
            w: 50.0,
            h: 50.0,
            score: 0.9,
            landmarks: [[0.0, 0.0]; 5],
            sharpness: 1.0,
            eye_open: Some(1.0),
            eyes_read: true,
        };
        let stored = store_faces(&mut conn, id, &[face], &[], (100, 100));
        writer.join().unwrap();
        stored.expect("store_faces must wait for the other writer, not fail");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM faces WHERE photo_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cull's close-ups: a photo's detected faces, largest first, with
    /// who they are; hand-added people have no face to show and are left out,
    /// and a virtual copy shows its master's faces under its own id.
    #[test]
    fn photo_faces_lists_detected_faces_largest_first_for_copies_too() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, preview_path)
                  VALUES (1, 'a.arw', 'a.arw', 'h', 'now', '/p/a.jpg');
             INSERT INTO people (id, name) VALUES (7, 'Ada');
             INSERT INTO faces (id, photo_id, x, y, w, h, img_w, img_h, sharpness, eye_open, person_id)
                  VALUES (10, 1, 0, 0, 100, 100, 1000, 500, 0.5, 0.9, NULL),
                         (11, 1, 400, 100, 200, 250, 1000, 500, 0.8, 0.1, 7);
             INSERT INTO faces (photo_id, x, y, w, h, manual, person_id)
                  VALUES (1, 0, 0, 0, 0, 1, 7);",
        )
        .unwrap();
        let copy = create_variant(&mut conn, 1, Some("Mono")).unwrap();

        let got = photo_faces(&conn, &[1, copy]).unwrap();
        let ids: Vec<(i32, i32)> = got
            .iter()
            .map(|(f, _, _)| (f.photo_id, f.face_id))
            .collect();
        assert_eq!(ids, vec![(1, 11), (1, 10), (copy, 11), (copy, 10)]);

        let (ada, preview, px) = &got[0];
        assert_eq!(ada.person_name.as_deref(), Some("Ada"));
        assert_eq!(ada.eye_open, Some(0.1));
        assert_eq!(ada.box_frac, vec![0.4, 0.2, 0.2, 0.5]);
        assert_eq!(preview.as_deref(), Some("/p/a.jpg"));
        assert_eq!(
            *px,
            [400.0, 100.0, 200.0, 250.0],
            "portraits are cut in preview pixels"
        );
        assert_eq!(got[1].0.person_id, None);
    }
}
