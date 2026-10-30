//! XMP sidecars: what a photo's sidecar carries from the catalogue,
//! and reading one back in, people included.

use super::*;

/// The photos whose sidecars `ids` stand for — every original when `ids` is
/// empty. A virtual copy shares its original's file, and so its sidecar:
/// writing both left whichever came last, so the original speaks for the file
/// and a copy stands for it.
pub fn sidecar_photos(conn: &Connection, ids: &[i32]) -> Result<Vec<i32>> {
    if ids.is_empty() {
        let mut stmt =
            conn.prepare("SELECT id FROM photos WHERE variant_of IS NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    let mut stmt = conn.prepare("SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1")?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &id in ids {
        if let Some(m) = stmt
            .query_row(params![id], |r| r.get::<_, i32>(0))
            .optional()?
        {
            if seen.insert(m) {
                out.push(m);
            }
        }
    }
    Ok(out)
}

/// A photo's tags of one category, by name.
fn photo_tag_names(conn: &Connection, photo_id: i32, category: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT t.name FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
          WHERE pt.photo_id = ?1 AND t.category = ?2
          ORDER BY t.name",
    )?;
    let rows = stmt.query_map(params![photo_id, category], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// What a photo's sidecar carries from the catalogue — see
/// [`crate::xmp`] for where each field goes. A virtual copy has its own
/// verdicts, tags and caption, and its original's people.
pub fn xmp_meta(conn: &Connection, photo: &Photo) -> Result<crate::xmp::XmpMeta> {
    use crate::xmp::{Region, CONDITIONS_ROOT, GENRE_ROOT};
    let conditions = photo_tag_names(conn, photo.id, "condition")?;
    let mut keywords = photo.subjects.clone();
    let mut hierarchy = Vec::new();
    // A filed keyword goes out as its whole path, and its parents join the
    // flat list, as other editors do on export by default:
    // most tools read only the flat list, and "france" is also "europe" there.
    // Export all asks this of every photo, so a library with no tree at all —
    // most of them — pays for one index probe, not three per keyword.
    let has_tree: bool = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM tags WHERE parent_id IS NOT NULL)")?
        .query_row([], |r| r.get(0))?;
    for subject in photo.subjects.iter().filter(|_| has_tree) {
        if let Some(path) = keyword_filed_path(conn, subject)? {
            // The folder keywords' container is ours, not a keyword:
            // "folders" went out on every photo filed under it.
            let path: Vec<String> = path.into_iter().skip_while(|p| p == FOLDERS_ROOT).collect();
            for parent in &path[..path.len().saturating_sub(1)] {
                if !keywords.contains(parent) {
                    keywords.push(parent.clone());
                }
            }
            if !path.is_empty() {
                hierarchy.push(path.join("|"));
            }
        }
    }
    // The tagger often names the genre as a subject too ("landscape" both
    // ways): one keyword, filed under Genre.
    let mut file = |root: &str, name: &String| {
        if !keywords.contains(name) {
            keywords.push(name.clone());
        }
        hierarchy.push(format!("{root}|{name}"));
    };
    if let Some(g) = &photo.genre {
        file(GENRE_ROOT, g);
    }
    for c in &conditions {
        file(CONDITIONS_ROOT, c);
    }

    // Who is in it: each named person the user hasn't hidden, once, from the
    // face the detector found in preference to one added by hand.
    let mut stmt = conn.prepare(
        "SELECT pe.name, f.x, f.y, f.w, f.h, f.img_w, f.img_h
           FROM faces f JOIN people pe ON pe.id = f.person_id
          WHERE f.photo_id = (SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1)
            AND pe.name IS NOT NULL AND pe.hidden = 0
          ORDER BY f.manual, f.x, f.id",
    )?;
    let rows = stmt.query_map(params![photo.id], |r| {
        let dims: (Option<f64>, Option<f64>) = (r.get(5)?, r.get(6)?);
        Ok((
            r.get::<_, String>(0)?,
            box_fraction(r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, dims),
            dims,
        ))
    })?;
    let mut people: Vec<String> = Vec::new();
    let mut regions = Vec::new();
    // Whether the photo is shown landscape, from a face measured on it.
    let mut landscape = None;
    for row in rows {
        let (name, frac, dims) = row?;
        if let (Some(w), Some(h)) = dims {
            // A face added by hand keeps its box in fractions, over 1 x 1.
            if w > 1.0 && h > 1.0 {
                landscape.get_or_insert(w >= h);
            }
        }
        if people.contains(&name) {
            continue;
        }
        if let [x, y, w, h] = frac[..] {
            if w > 0.0 && h > 0.0 {
                regions.push(Region::from_displayed(
                    &name,
                    [x, y, w, h],
                    photo.orientation,
                ));
            }
        }
        people.push(name);
    }
    let region_dims = (!regions.is_empty() && photo.width > 0 && photo.height > 0).then(|| {
        let edges = (photo.width as u32, photo.height as u32);
        // With no face measured, the catalogue's size is taken as the file's.
        landscape.map_or(edges, |l| {
            crate::xmp::stored_dims(edges, l, photo.orientation)
        })
    });

    // The photo's own title, caption and rights: a caption someone
    // wrote goes out in preference to the model's.
    let own = photo_iptc(conn, photo.id)?;
    let text = crate::xmp::XmpText {
        title: own.title,
        creator: own.creator,
        rights: own.copyright,
        credit: own.credit,
        usage_terms: own.usage_terms,
    };

    // A corrected capture time, with the file's own beside it. An
    // uncorrected photo's sidecar gets no date: the file already has it.
    let (captured, captured_original) = if photo.capture_edited {
        let original: Option<String> = conn
            .query_row(
                "SELECT original_capture_date FROM photos WHERE id = ?1",
                params![photo.id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        (
            photo.capture_date.as_deref().and_then(parse_capture),
            original.as_deref().and_then(parse_capture),
        )
    } else {
        (None, None)
    };

    // Keywords only the tagger gave, none of them the user's.
    let ai_keywords: Vec<String> = {
        let mut stmt = conn.prepare_cached(
            "SELECT t.name FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
              WHERE pt.photo_id = ?1
              GROUP BY t.name HAVING MIN(pt.source = 'ai') = 1",
        )?;
        let ai: std::collections::HashSet<String> = stmt
            .query_map(params![photo.id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        keywords
            .iter()
            .filter(|k| ai.contains(*k))
            .cloned()
            .collect()
    };

    Ok(crate::xmp::XmpMeta {
        ai_keywords,
        rating: (photo.rating > 0).then_some(photo.rating),
        color_label: (photo.color_label > 0).then_some(photo.color_label),
        keywords,
        hierarchy,
        flag: (photo.flag != 0).then_some(photo.flag),
        description_is_own: own.caption.is_some(),
        description: own.caption.or_else(|| photo.caption.clone()),
        text,
        people,
        regions,
        region_dims,
        location: crate::export::location::PhotoLocation::load(conn, photo.id)?,
        captured,
        captured_original,
    })
}

/// What reading sidecars did to People.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PeopleImport {
    /// Names put on a person, a face or a photo.
    pub named: u32,
    /// Names left off a face because People already calls it someone else.
    pub kept: u32,
    /// Whether a face was linked to someone, which only a regroup acts on.
    pub regroup: bool,
}

/// How much a sidecar's face region and a detected face must overlap to be
/// the same face. Loose: other editors' boxes take in more forehead and chin than
/// the detector's, and two faces never overlap this much.
pub(super) const REGION_MATCH: f32 = 0.3;

/// Applies a sidecar to a photo. Rating, label and flag overwrite (a
/// sidecar is the authority on those). Keywords, conditions and people are
/// *added*, so an import never silently drops what the photo already had; a
/// genre replaces the photo's, since it has one. A caption a person wrote comes
/// in as the photo's own, apart from the vision model's; one this app
/// wrote out comes back only where the photo has no caption at all. The title
/// and rights someone set elsewhere come in over the photo's own, as ratings
/// do. A field the sidecar omits is left untouched.
pub fn import_xmp_meta(
    conn: &mut Connection,
    photo_id: i32,
    sidecar: &crate::xmp::Sidecar,
) -> Result<PeopleImport> {
    let Some(photo) = get_photo(conn, photo_id)? else {
        return Ok(PeopleImport::default());
    };
    let meta = &sidecar.meta;
    if let Some(r) = meta.rating {
        set_rating(conn, photo_id, r)?;
    }
    if let Some(l) = meta.color_label {
        set_color_label(conn, photo_id, l)?;
    }
    if let Some(f) = meta.flag {
        set_flag(conn, photo_id, f)?;
    }
    let sorted = meta.sorted_keywords();
    for path in &sorted.subject_paths {
        ensure_keyword_path(conn, path)?;
    }
    // A keyword this app wrote there, and the photo still has, is left as it
    // is: Export all then Import all turned every tagger guess and
    // genre into the user's own tags for good. Only keywords written
    // elsewhere — another editor's — or lost from the catalogue come in as the
    // user's.
    let ours = |kw: &str| {
        sidecar
            .keywords_ours
            .iter()
            .any(|k| *k == kw.to_lowercase())
    };
    for kw in &sorted.subjects {
        if !(ours(kw) && has_tag(conn, photo_id, kw, "subject")?) {
            add_manual_tag(conn, photo_id, kw, "subject")?;
        }
    }
    for c in &sorted.conditions {
        if !(ours(c) && has_tag(conn, photo_id, c, "condition")?) {
            add_manual_tag(conn, photo_id, c, "condition")?;
        }
    }
    if let Some(g) = &sorted.genre {
        if !(ours(g) && photo.genre.is_some()) {
            set_manual_genre(conn, photo_id, g)?;
        }
    }
    if let Some(d) = &meta.description {
        if !sidecar.description_is_ours {
            set_iptc_field(conn, &[photo_id], IptcField::Caption, Some(d))?;
        } else if photo.caption.is_none() && photo_iptc(conn, photo_id)?.caption.is_none() {
            set_caption(conn, photo_id, Some(d))?;
        }
    }
    let theirs = &sidecar.text_from_elsewhere;
    apply_iptc(
        conn,
        &[photo_id],
        &IptcFields {
            title: theirs.title.clone(),
            caption: None,
            creator: theirs.creator.clone(),
            copyright: theirs.rights.clone(),
            credit: theirs.credit.clone(),
            usage_terms: theirs.usage_terms.clone(),
        },
    )?;
    let mut named: Vec<(String, Option<[f32; 4]>)> = meta
        .regions
        .iter()
        .map(|r| (r.name.clone(), Some(r.displayed_box(photo.orientation))))
        .collect();
    for name in meta.people.iter().chain(&sorted.people) {
        if !named.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            named.push((name.clone(), None));
        }
    }
    import_people(conn, photo_id, &named)
}

/// Makes `genre` the photo's genre, as the user's own: any other it had — the
/// tagger's guess included — goes, since a photo has one genre.
/// Whether a photo has a tag of `category` named `name`, from any source.
fn has_tag(conn: &Connection, photo_id: i32, name: &str, category: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
                         WHERE pt.photo_id = ?1 AND t.name = ?2 COLLATE NOCASE AND t.category = ?3)",
        params![photo_id, name, category],
        |r| r.get(0),
    )?)
}

fn set_manual_genre(conn: &Connection, photo_id: i32, genre: &str) -> Result<()> {
    let genre = genre.trim().to_lowercase();
    if genre.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM photo_tags WHERE photo_id = ?1 AND tag_id IN
           (SELECT id FROM tags WHERE category = 'genre' AND name != ?2)",
        params![photo_id, genre],
    )?;
    add_manual_tag(conn, photo_id, &genre, "genre")
}

/// Stores a link between two faces, both ways round as every link is.
pub(super) fn link_faces(tx: &Connection, a: i64, b: i64) -> Result<()> {
    let mut ins = tx.prepare_cached("INSERT OR IGNORE INTO face_links (a, b) VALUES (?1, ?2)")?;
    ins.execute(params![a, b])?;
    ins.execute(params![b, a])?;
    Ok(())
}

/// Puts the people a sidecar names into a photo. Each of `names` is a
/// name and, when the sidecar placed them, a box `[x, y, w, h]` on the photo
/// as displayed.
///
/// - A box on a detected face whose person has no name names that person —
///   or, when someone already has the name, links the face to them, which the
///   next regroup acts on (the caller runs one, once).
/// - A box on a face People calls someone else is left alone: the user's own
///   naming wins, and it is counted in `kept`.
/// - Anyone else is added by hand, keeping the box when there is one. A box
///   waits for the photo to be analysed: the face found there takes over the
///   name (see [`store_faces`]).
///
/// A name the photo already has is skipped, so reading again changes nothing.
pub fn import_people(
    conn: &mut Connection,
    photo_id: i32,
    names: &[(String, Option<[f32; 4]>)],
) -> Result<PeopleImport> {
    struct Found {
        id: i64,
        person: Option<i32>,
        name: Option<String>,
        frac: Vec<f32>,
        detected: bool,
    }
    let mut out = PeopleImport::default();
    if names.is_empty() {
        return Ok(out);
    }
    let Some(master) = conn
        .query_row(
            "SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1",
            params![photo_id],
            |r| r.get::<_, i32>(0),
        )
        .optional()?
    else {
        return Ok(out);
    };
    let tx = conn.transaction()?;
    let faces: Vec<Found> = {
        let mut stmt = tx.prepare(
            "SELECT f.id, f.person_id, pe.name, f.x, f.y, f.w, f.h, f.img_w, f.img_h,
                    f.manual = 0 AND f.detached = 0
               FROM faces f LEFT JOIN people pe ON pe.id = f.person_id
              WHERE f.photo_id = ?1",
        )?;
        let rows = stmt.query_map(params![master], |r| {
            Ok(Found {
                id: r.get(0)?,
                person: r.get(1)?,
                name: r.get(2)?,
                frac: box_fraction(
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    (r.get(7)?, r.get(8)?),
                ),
                detected: r.get(9)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut claimed: Vec<i64> = Vec::new();
    for (name, bx) in names {
        let name = name.trim();
        let there = |f: &Found| {
            f.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        };
        if name.is_empty() || faces.iter().any(there) {
            continue;
        }
        let known: Option<i32> = tx
            .query_row(
                "SELECT id FROM people WHERE name = ?1 COLLATE NOCASE ORDER BY size DESC, id LIMIT 1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        // The detected face the box is on, if any.
        let on = bx.and_then(|b| {
            faces
                .iter()
                .filter(|f| f.detected && !claimed.contains(&f.id))
                .filter_map(|f| match f.frac[..] {
                    [x, y, w, h] => Some((f, crate::faces::iou(b, [x, y, w, h]))),
                    _ => None,
                })
                .filter(|&(_, v)| v >= REGION_MATCH)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(f, _)| f)
        });
        if let Some(f) = on {
            claimed.push(f.id);
            match (f.person, &f.name, known) {
                (Some(_), Some(_), _) => {
                    out.kept += 1;
                    continue;
                }
                (Some(p), None, None) => {
                    tx.execute(
                        "UPDATE people SET name = ?2, hidden = 0 WHERE id = ?1",
                        params![p, name],
                    )?;
                    out.named += 1;
                    continue;
                }
                (Some(_), None, Some(q)) => {
                    for a in person_anchors(&tx, q)? {
                        link_faces(&tx, f.id, a)?;
                    }
                    tx.execute("UPDATE people SET hidden = 0 WHERE id = ?1", params![q])?;
                    out.named += 1;
                    out.regroup = true;
                    continue;
                }
                // A face too weak to group: named by hand, over it.
                (None, _, _) => {}
            }
        }
        let person = match known {
            Some(q) => q,
            None => {
                tx.execute(
                    "INSERT INTO people (name, size) VALUES (?1, 0)",
                    params![name],
                )?;
                tx.last_insert_rowid() as i32
            }
        };
        let anchors = person_anchors(&tx, person)?;
        // A box is kept in fractions, over an image of 1 x 1.
        let (b, unit) = match bx {
            Some(b) => (b.map(f64::from), Some(1.0)),
            None => ([0.0; 4], None),
        };
        tx.execute(
            "INSERT INTO faces (photo_id, x, y, w, h, img_w, img_h, manual, person_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 1, ?7)",
            params![master, b[0], b[1], b[2], b[3], unit, person],
        )?;
        let face = tx.last_insert_rowid();
        for a in anchors {
            link_faces(&tx, face, a)?;
        }
        tx.execute(
            "UPDATE people SET size = size + 1, hidden = 0,
                    cover_face_id = COALESCE(cover_face_id, ?2)
              WHERE id = ?1",
            params![person, face],
        )?;
        out.named += 1;
    }
    tx.commit()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Writing sidecars and reading them back leaves the tagger's keywords
    /// the tagger's; a catalogue that lost them takes them as the
    /// user's, since nothing else says otherwise.
    #[test]
    fn a_sidecar_round_trip_keeps_ai_keywords_ai() {
        let mut conn = open_memory().unwrap();
        photo_at(&conn, 1, "/p/1.arw");
        conn.execute_batch(
            "INSERT INTO tags (id, name, category) VALUES (71, 'beer', 'subject'), (72, 'castle', 'subject');
             INSERT INTO photo_tags (photo_id, tag_id, confidence, source)
                  VALUES (1, 71, 0.4, 'ai'), (1, 72, 1.0, 'manual');",
        )
        .unwrap();
        let packet =
            crate::xmp::write(&xmp_meta(&conn, &get_photo(&conn, 1).unwrap().unwrap()).unwrap());
        let sidecar = crate::xmp::read(&packet).unwrap();
        assert!(sidecar.keywords_ours.contains(&"beer".to_string()));

        import_xmp_meta(&mut conn, 1, &sidecar).unwrap();
        let source = |conn: &Connection, name: &str| -> String {
            conn.query_row(
                "SELECT pt.source FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
                  WHERE pt.photo_id = 1 AND t.name = ?1",
                params![name],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(source(&conn, "beer"), "ai", "still the tagger's guess");
        assert_eq!(source(&conn, "castle"), "manual");

        // A fresh catalogue has nothing else to go on.
        let mut fresh = open_memory().unwrap();
        photo_at(&fresh, 1, "/p/1.arw");
        import_xmp_meta(&mut fresh, 1, &sidecar).unwrap();
        assert_eq!(source(&fresh, "beer"), "manual");
    }

    /// Face at (100, 100), 100px, in a 1600x1067 preview — `face_at`'s box —
    /// as a fraction of the frame.
    const FACE_FRAC: [f32; 4] = [
        100.0 / 1600.0,
        100.0 / 1067.0,
        100.0 / 1600.0,
        100.0 / 1067.0,
    ];

    /// A face region read from a sidecar names the person the detector put
    /// there, when nobody has yet. A face People already calls someone else
    /// keeps its name, and reading again changes nothing.
    #[test]
    fn a_sidecar_region_names_the_unnamed_person_it_is_on() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 3);
        let named = import_people(&mut conn, 3, &[("Bo".into(), Some(FACE_FRAC))]).unwrap();
        assert_eq!(
            named,
            PeopleImport {
                named: 1,
                kept: 0,
                regroup: false
            }
        );
        assert_eq!(person_of_photo(&conn, 3).as_deref(), Some("Bo"));
        let again = import_people(&mut conn, 3, &[("bo".into(), Some(FACE_FRAC))]).unwrap();
        assert_eq!(again, PeopleImport::default());

        let kept = import_people(&mut conn, 1, &[("Cy".into(), Some(FACE_FRAC))]).unwrap();
        assert_eq!((kept.named, kept.kept), (0, 1));
        assert_eq!(person_of_photo(&conn, 1).as_deref(), Some("Ada"));
        assert!(
            !list_people(&conn)
                .unwrap()
                .iter()
                .any(|p| p.name.as_deref() == Some("Cy")),
            "nobody new was made"
        );
    }

    /// A region on an unnamed face, naming someone People already has, joins
    /// the face to them at the next regroup, as a merge by hand would.
    #[test]
    fn a_sidecar_region_joins_its_face_to_the_person_of_that_name() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 3);
        let got = import_people(&mut conn, 3, &[("ada".into(), Some(FACE_FRAC))]).unwrap();
        assert!(got.regroup);
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 3).as_deref(), Some("Ada"));
    }

    /// A sidecar written from one catalogue and read into an
    /// empty one brings back the verdicts, the caption, the keywords as what
    /// they were, and who is in the photo — named, and where they were.
    #[test]
    fn sidecars_round_trip_into_an_empty_catalogue() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 3);
        set_rating(&conn, 1, 4).unwrap();
        set_flag(&conn, 1, 1).unwrap();
        set_color_label(&conn, 1, 2).unwrap();
        set_caption(&conn, 1, Some("A woman on a pier")).unwrap();
        add_manual_tag(&conn, 1, "pier", "subject").unwrap();
        add_manual_tag(&conn, 1, "portrait", "genre").unwrap();
        add_manual_tag(&conn, 1, "golden hour", "condition").unwrap();
        let photo = get_photo(&conn, 1).unwrap().unwrap();
        let packet = crate::xmp::write(&xmp_meta(&conn, &photo).unwrap());

        let mut empty = open_memory().unwrap();
        add_photos(&empty, 1);
        let sidecar = crate::xmp::read(&packet).unwrap();
        let got = import_xmp_meta(&mut empty, 1, &sidecar).unwrap();
        assert_eq!(got.named, 1, "{packet}");
        let back = get_photo(&empty, 1).unwrap().unwrap();
        assert_eq!((back.rating, back.flag, back.color_label), (4, 1, 2));
        assert_eq!(back.caption.as_deref(), Some("A woman on a pier"));
        assert_eq!(back.genre.as_deref(), Some("portrait"));
        assert_eq!(back.subjects, vec!["pier".to_string()]);
        assert_eq!(
            photo_tag_names(&empty, 1, "condition").unwrap(),
            vec!["golden hour".to_string()]
        );
        assert_eq!(person_of_photo(&empty, 1).as_deref(), Some("Ada"));

        // Where she was: a box, waiting for the photo to be analysed ...
        let waiting: [f64; 4] = empty
            .query_row(
                "SELECT x, y, w, h FROM faces WHERE photo_id = 1 AND manual = 1",
                [],
                |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
            )
            .unwrap();
        for (got, want) in waiting.iter().zip(FACE_FRAC) {
            assert!((*got as f32 - want).abs() < 1e-3, "{waiting:?}");
        }
        // ... which the face found there takes over, name and all.
        store_faces(
            &mut empty,
            1,
            &[face_at(100.0, 100.0, false)],
            &[unit(&[1.0, 0.0, 0.0])],
            (1600, 1067),
        )
        .unwrap();
        let hand: i64 = empty
            .query_row("SELECT COUNT(*) FROM faces WHERE manual = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(hand, 0, "the stand-in went");
        regroup_people(&mut empty).unwrap();
        assert_eq!(person_of_photo(&empty, 1).as_deref(), Some("Ada"));
    }

    /// A sidecar another editor wrote with a named face comes in as a
    /// named person: on the face the detector found there when the photo has
    /// been analysed, and by hand, waiting for it, when it hasn't.
    #[test]
    fn a_lightroom_face_region_imports_as_a_named_person() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        // Where the sidecar's region is: centred at (0.5, 0.4) of the frame.
        store_faces(
            &mut conn,
            1,
            &[face_at(720.0, 400.0, false)],
            &[unit(&[1.0, 0.0, 0.0])],
            (1600, 1067),
        )
        .unwrap();
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 1), None, "grouped, not named");

        let sidecar = crate::xmp::read(crate::xmp::tests::LIGHTROOM).unwrap();
        for photo in [1, 2] {
            let got = import_xmp_meta(&mut conn, photo, &sidecar).unwrap();
            assert_eq!((got.named, got.kept), (1, 0), "photo {photo}");
            assert_eq!(
                person_of_photo(&conn, photo).as_deref(),
                Some("Ada Lovelace"),
                "photo {photo}"
            );
            let p = get_photo(&conn, photo).unwrap().unwrap();
            assert_eq!(p.rating, 3);
            assert_eq!(
                p.subjects,
                vec!["beach".to_string()],
                "her name isn't a subject"
            );
        }
        let people = list_people(&conn).unwrap();
        assert_eq!(people.len(), 1, "one Ada: {people:?}");
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 2).as_deref(), Some("Ada Lovelace"));
    }

    /// A virtual copy shares its original's file, so it stands for the
    /// original's sidecar rather than writing a second one over it.
    #[test]
    fn a_copy_stands_for_its_originals_sidecar() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        let copy = create_variant(&mut conn, 1, None).unwrap();
        assert_eq!(sidecar_photos(&conn, &[copy, 1, 2]).unwrap(), vec![1, 2]);
        assert_eq!(sidecar_photos(&conn, &[]).unwrap(), vec![1, 2]);
    }

    /// A genre a sidecar gave replaces the tagger's, and a later tagging run
    /// doesn't put its guess back beside it.
    #[test]
    fn a_genre_from_a_sidecar_outlasts_a_retag() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 1);
        let guess = |genre: &str| TagResult {
            main_subject: String::new(),
            genre: genre.into(),
            subjects: vec![],
            conditions: vec![],
            confidence: 0.9,
        };
        apply_ai_tags(&mut conn, 1, &guess("street")).unwrap();
        set_manual_genre(&conn, 1, "Portrait").unwrap();
        apply_ai_tags(&mut conn, 1, &guess("street")).unwrap();
        assert_eq!(
            photo_tag_names(&conn, 1, "genre").unwrap(),
            vec!["portrait".to_string()]
        );
    }

    /// A sidecar follows the export's choices: the tagger's keywords
    /// are told from the user's, the model's caption is left out unless
    /// asked and marked as its own when not, and the folder keywords'
    /// container never goes out as a keyword.
    #[test]
    fn sidecars_follow_the_metadata_choices() {
        use crate::xmp::MetadataChoices;
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, caption)
             VALUES (1, 'D:/a.arw', 'a.arw', 'h', 'x', 'A sign reading 14 Elm Road');
             INSERT INTO tags (id, name, category) VALUES
                 (1, 'castle', 'subject'), (2, 'beer', 'subject'), (3, 'skye', 'subject');
             INSERT INTO photo_tags (photo_id, tag_id, confidence, source) VALUES
                 (1, 1, 1.0, 'manual'), (1, 2, 0.4, 'ai'), (1, 3, 1.0, 'manual');",
        )
        .unwrap();
        file_keyword_under_folders(&conn, "skye");
        let meta = xmp_meta(&conn, &photo_of(&conn, 1)).unwrap();
        assert_eq!(meta.ai_keywords, vec!["beer".to_string()]);
        assert!(
            !meta.keywords.iter().any(|k| k == FOLDERS_ROOT),
            "{:?}",
            meta.keywords
        );
        assert!(meta.hierarchy.iter().all(|h| !h.starts_with(FOLDERS_ROOT)));

        let mut delivered = meta.clone();
        MetadataChoices::default().apply(&mut delivered);
        assert!(!delivered.keywords.contains(&"beer".to_string()));
        assert!(delivered.keywords.contains(&"castle".to_string()));
        assert_eq!(delivered.description, None);
        assert!(!crate::xmp::write(&delivered).contains("CaptionWriter"));

        let mut everything = meta;
        MetadataChoices::ALL.apply(&mut everything);
        let packet = crate::xmp::write(&everything);
        assert!(packet.contains("14 Elm Road"));
        assert!(packet.contains("AI (on-device model)"), "{packet}");
    }

    fn file_keyword_under_folders(conn: &Connection, name: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO tags (name, category) VALUES (?1, 'subject')",
            params![FOLDERS_ROOT],
        )
        .unwrap();
        move_keyword(conn, name, Some(FOLDERS_ROOT)).unwrap();
    }
}
