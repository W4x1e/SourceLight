//! Colours: reading each photo's palette and finding similar ones,
//! and checking previews against the camera's colour space.

use super::*;

/// A photo whose colours have not been read: its id, the image to read them
/// from, and whether it has been edited, which means a render instead.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteTodo {
    pub id: i32,
    pub image: String,
    pub edited: bool,
}

pub fn palette_backlog(conn: &Connection) -> Result<Vec<PaletteTodo>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, COALESCE(p.thumb_path, p.preview_path), p.develop_settings IS NOT NULL
           FROM photos p
          WHERE p.missing = 0 AND COALESCE(p.thumb_path, p.preview_path) IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM photo_palette pp WHERE pp.photo_id = p.id)
          ORDER BY p.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PaletteTodo {
            id: r.get(0)?,
            image: r.get(1)?,
            edited: r.get::<_, i64>(2)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn palette_backlog_count(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM photos p
          WHERE p.missing = 0 AND COALESCE(p.thumb_path, p.preview_path) IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM photo_palette pp WHERE pp.photo_id = p.id)",
        [],
        |r| r.get::<_, i64>(0),
    )? as u32)
}

/// Stores a photo's colours. One that could not be read stores an empty
/// palette, so it is not tried on every pass.
pub fn set_palette(
    conn: &Connection,
    photo_id: i32,
    palette: &crate::palette::Palette,
) -> Result<()> {
    conn.execute(
        "INSERT INTO photo_palette (photo_id, swatches, signature) VALUES (?1, ?2, ?3)
         ON CONFLICT(photo_id) DO UPDATE SET swatches = excluded.swatches, signature = excluded.signature",
        params![photo_id, palette.swatches, palette.signature],
    )?;
    Ok(())
}

/// The photos whose colours are most like this one's, most alike
/// first, at least `min` alike (0–1), not counting the photo itself or its
/// copies. Empty until its colours have been read.
pub fn similar_colours(
    conn: &Connection,
    photo_id: i32,
    limit: usize,
    min: f32,
) -> Result<Vec<i32>> {
    let target: Option<Vec<u8>> = conn
        .query_row(
            "SELECT signature FROM photo_palette WHERE photo_id = ?1",
            params![photo_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(target) = target.filter(|t| !t.is_empty()) else {
        return Ok(Vec::new());
    };
    let path: String = conn.query_row(
        "SELECT path FROM photos WHERE id = ?1",
        params![photo_id],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT pp.photo_id, pp.signature FROM photo_palette pp JOIN photos p ON p.id = pp.photo_id
          WHERE p.path != ?1 AND length(pp.signature) > 0",
    )?;
    let mut scored: Vec<(i32, f32)> = stmt
        .query_map(params![path], |r| {
            Ok((r.get::<_, i32>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?
        .filter_map(|row| row.ok())
        .map(|(id, sig)| (id, crate::palette::likeness(&target, &sig)))
        .filter(|(_, l)| *l >= min)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(limit);
    Ok(scored.into_iter().map(|(id, _)| id).collect())
}

/// A photo whose previews were made before the camera's colour space was
/// read (migration 28), for `read_colours`.
#[derive(Debug, Clone, PartialEq)]
pub struct ColoursToRead {
    pub id: i32,
    pub path: String,
    pub file_hash: String,
    pub orientation: u32,
    pub thumb_path: Option<String>,
    pub preview_path: Option<String>,
}

/// Photos whose previews have not been checked against the colour space the
/// camera recorded. Files that are missing are left for when they come back.
pub fn photos_needing_colours(conn: &Connection) -> Result<Vec<ColoursToRead>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, file_hash, orientation, thumb_path, preview_path FROM photos
          WHERE variant = 0 AND colours_read = 0 AND missing = 0",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ColoursToRead {
                id: r.get(0)?,
                path: r.get(1)?,
                file_hash: r.get(2)?,
                orientation: r.get::<_, i64>(3)?.clamp(1, 8) as u32,
                thumb_path: r.get(4)?,
                preview_path: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Records that a photo's previews are in the right colours, pointing it —
/// and its virtual copies, which share its file — at new ones if they were
/// remade.
pub fn set_colours_read(conn: &Connection, id: i32, remade: Option<(&str, &str)>) -> Result<()> {
    match remade {
        Some((thumb, preview)) => conn.execute(
            "UPDATE photos SET thumb_path = ?2, preview_path = ?3, colours_read = 1
              WHERE path = (SELECT path FROM photos WHERE id = ?1)",
            params![id, thumb, preview],
        )?,
        None => conn.execute(
            "UPDATE photos SET colours_read = 1 WHERE id = ?1",
            params![id],
        )?,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Photos catalogued before the camera's colour space was read are each
    /// checked once; remade previews reach the master and its virtual copies,
    /// which share its file and its previews.
    #[test]
    fn colours_are_checked_once_and_remade_previews_reach_every_copy() {
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, thumb_path,
                                 preview_path, orientation, variant)
                 VALUES (1, '/a.nef', 'a.nef', 'h1', '2026-01-01', '/c/h1_t.jpg', '/c/h1_p.jpg', 6, 0),
                        (2, '/a.nef', 'a.nef', 'h1', '2026-01-01', '/c/h1_t.jpg', '/c/h1_p.jpg', 6, 1),
                        (3, '/b.nef', 'b.nef', 'h2', '2026-01-01', '/c/h2_t.jpg', '/c/h2_p.jpg', 1, 0);",
        )
        .unwrap();
        let todo = photos_needing_colours(&conn).unwrap();
        assert_eq!(
            todo.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![1, 3],
            "masters only: a copy shares its master's file"
        );
        assert_eq!(todo[0].orientation, 6);
        assert_eq!(todo[0].thumb_path.as_deref(), Some("/c/h1_t.jpg"));

        set_colours_read(&conn, 1, Some(("/c/h1-c_t.jpg", "/c/h1-c_p.jpg"))).unwrap();
        set_colours_read(&conn, 3, None).unwrap();
        assert!(photos_needing_colours(&conn).unwrap().is_empty());
        let thumbs: Vec<String> = conn
            .prepare("SELECT thumb_path FROM photos ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(thumbs, ["/c/h1-c_t.jpg", "/c/h1-c_t.jpg", "/c/h2_t.jpg"]);
    }

    // --- Colours ------------------------------------------------------

    fn painted(conn: &Connection, id: i32, parts: &[([u8; 3], usize)]) {
        let palette =
            crate::palette::read(parts.iter().flat_map(|&(c, n)| std::iter::repeat_n(c, n)));
        set_palette(conn, id, &palette).unwrap();
    }

    const ORANGE: [u8; 3] = [250, 120, 30];

    const PURPLE: [u8; 3] = [110, 50, 140];

    const GREEN: [u8; 3] = [40, 110, 40];

    #[test]
    fn the_palette_filters_and_counts_by_named_colour() {
        let conn = open_memory().unwrap();
        add_photos(&conn, 3);
        painted(&conn, 1, &[(ORANGE, 60), (PURPLE, 40)]);
        painted(&conn, 2, &[(GREEN, 100)]);
        painted(&conn, 3, &[(ORANGE, 90), (GREEN, 10)]);

        let orange = PhotoFilter {
            palette: vec!["orange".into()],
            ..Default::default()
        };
        let mut got: Vec<i32> = query_photos(&conn, &orange, 10, 0)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        got.sort();
        assert_eq!(got, vec![1, 3], "green is only a tenth of photo 3");

        let groups = FacetGroups {
            palette: true,
            ..Default::default()
        };
        let counts: Vec<(String, i32)> = extra_facets(&conn, &orange, None, groups)
            .unwrap()
            .palette
            .into_iter()
            .map(|c| (c.value, c.count))
            .collect();
        // Counted with the palette's own choice lifted, as every facet is.
        assert_eq!(
            counts,
            vec![
                ("orange".into(), 2),
                ("green".into(), 1),
                ("purple".into(), 1)
            ]
        );
    }

    #[test]
    fn an_edit_sends_a_photo_back_to_have_its_colours_read() {
        let conn = open_memory().unwrap();
        add_photos(&conn, 2);
        conn.execute("UPDATE photos SET thumb_path = '/t/' || id || '.jpg'", [])
            .unwrap();
        assert_eq!(palette_backlog_count(&conn).unwrap(), 2);
        painted(&conn, 1, &[(GREEN, 1)]);
        painted(&conn, 2, &[(GREEN, 1)]);
        assert_eq!(palette_backlog_count(&conn).unwrap(), 0);

        let stack = crate::develop::EditStack {
            exposure: 1.0,
            ..Default::default()
        };
        set_develop_settings(&conn, 2, &stack).unwrap();
        let todo = palette_backlog(&conn).unwrap();
        assert_eq!(
            todo,
            vec![PaletteTodo {
                id: 2,
                image: "/t/2.jpg".into(),
                edited: true
            }]
        );
    }

    #[test]
    fn similar_colours_are_ranked_and_leave_out_the_photo_and_its_copies() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 4);
        painted(&conn, 1, &[(ORANGE, 50), (PURPLE, 50)]);
        painted(&conn, 2, &[(ORANGE, 40), (PURPLE, 60)]);
        painted(&conn, 3, &[(GREEN, 100)]);
        painted(&conn, 4, &[(ORANGE, 90), (PURPLE, 10)]);
        let copy = create_variant(&mut conn, 1, None).unwrap();
        painted(&conn, copy, &[(ORANGE, 50), (PURPLE, 50)]);

        assert_eq!(similar_colours(&conn, 1, 10, 0.5).unwrap(), vec![2, 4]);
        assert!(similar_colours(&conn, 3, 10, 0.5).unwrap().is_empty());
        // Not read yet: nothing to compare.
        conn.execute("DELETE FROM photo_palette WHERE photo_id = 3", [])
            .unwrap();
        assert!(similar_colours(&conn, 3, 10, 0.0).unwrap().is_empty());
    }
}
