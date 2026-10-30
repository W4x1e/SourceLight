//! Turning a `PhotoFilter` into SQL, and reading photos back.

use super::*;

/// Membership test for a tag filter, as an **uncorrelated** semijoin rather than
/// a correlated `EXISTS`. The old form referenced `p.id` inside the subquery, so
/// the planner re-ran it for every row of a full `photos` scan — 50,000 subquery
/// evaluations for one genre click. This resolves the matching photo ids
/// once, driving from `photo_tags` through `idx_phototags_tag_photo` — covering,
/// so it never touches the table (migration 15) — and `tags` through
/// `idx_tags_category_name` (migration 13): the small side. It then probes
/// `photos` by primary key. `category` is bound like the names, not
/// interpolated: every caller passes a constant today, but a clause that is
/// only safe while that stays true is one refactor from an injection.
/// It is pushed first because it is the first `?` in the clause.
fn tag_membership_clause(category: &str, names: &[String], params: &mut Vec<Value>) -> String {
    params.push(Value::Text(category.to_string()));
    for n in names {
        params.push(Value::Text(n.clone()));
    }
    format!(
        "p.id IN (SELECT pt.photo_id FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
                  WHERE t.category = ? AND t.name IN ({}))",
        placeholders(names.len())
    )
}

/// Membership in a keyword **or anything filed beneath it**: choosing
/// "animals" finds the dogs and the birds. The chosen keywords' subtrees are
/// resolved first — through `idx_tags_category_name`, then `idx_tags_parent`
/// a level at a time — and the photos then driven from `photo_tags` by tag, as
/// [`tag_membership_clause`] does. With no tree, the subtree is the keyword
/// itself, and this matches exactly what that one does.
///
/// `own` counts only the links a person made or confirmed.
fn keyword_membership_clause(names: &[String], own: bool, params: &mut Vec<Value>) -> String {
    for n in names {
        params.push(Value::Text(n.clone()));
    }
    format!(
        "p.id IN (SELECT kpt.photo_id FROM photo_tags kpt WHERE kpt.tag_id IN (
             WITH RECURSIVE kw(id) AS (
                 SELECT id FROM tags WHERE category = 'subject' AND name IN ({})
                 UNION
                 SELECT kc.id FROM tags kc JOIN kw ON kc.parent_id = kw.id)
             SELECT id FROM kw){})",
        placeholders(names.len()),
        if own { " AND kpt.source != 'ai'" } else { "" }
    )
}

/// A range facet's options: each a key the filter carries and the SQL
/// test for it over `photos p`. Constants, never user input — a key the table
/// does not know is ignored rather than interpolated.
pub(super) type Ranges = [(&'static str, &'static str)];

/// Focal length, in mm: wide, normal, short tele, tele, long tele.
pub const FOCAL_RANGES: &Ranges = &[
    ("0-24", "p.focal_length <= 24"),
    ("25-50", "p.focal_length > 24 AND p.focal_length <= 50"),
    ("51-105", "p.focal_length > 50 AND p.focal_length <= 105"),
    ("106-200", "p.focal_length > 105 AND p.focal_length <= 200"),
    ("201-", "p.focal_length > 200"),
];

/// ISO: base, everyday, low light, very high.
pub const ISO_RANGES: &Ranges = &[
    ("0-400", "p.iso <= 400"),
    ("401-1600", "p.iso > 400 AND p.iso <= 1600"),
    ("1601-6400", "p.iso > 1600 AND p.iso <= 6400"),
    ("6401-", "p.iso > 6400"),
];

/// Aperture, as an f-number: wide open, middling, stopped down.
pub const APERTURE_RANGES: &Ranges = &[
    ("0-2.8", "p.aperture <= 2.8"),
    ("2.9-5.6", "p.aperture > 2.8 AND p.aperture <= 5.6"),
    ("5.7-", "p.aperture > 5.6"),
];

/// File types, from the extension: each rendered format by name, and
/// anything else a raw. A RAW+JPEG pair is catalogued as its raw, so it is one.
/// `LIKE` is case-blind for ASCII, so `.JPG` is a JPEG.
pub const FILE_KINDS: &Ranges = &[
    (
        "raw",
        "NOT (p.path LIKE '%.jpg' OR p.path LIKE '%.jpeg' OR p.path LIKE '%.heic'
              OR p.path LIKE '%.heif' OR p.path LIKE '%.hif' OR p.path LIKE '%.tif'
              OR p.path LIKE '%.tiff' OR p.path LIKE '%.png' OR p.path LIKE '%.webp')",
    ),
    ("jpeg", "(p.path LIKE '%.jpg' OR p.path LIKE '%.jpeg')"),
    (
        "heic",
        "(p.path LIKE '%.heic' OR p.path LIKE '%.heif' OR p.path LIKE '%.hif')",
    ),
    ("tiff", "(p.path LIKE '%.tif' OR p.path LIKE '%.tiff')"),
    ("png", "p.path LIKE '%.png'"),
    ("webp", "p.path LIKE '%.webp'"),
];

/// Shapes, as the photo is shown: `width` and `height` are the file's,
/// before its orientation, so a frame turned a quarter (orientations 5–8) is
/// the other way round. A panorama is twice as long as it is high, or longer;
/// square is within 5%. A photo whose size isn't known yet has no shape.
pub const SHAPES: &Ranges = &[
    (
        "portrait",
        "p.width > 0 AND p.height > 0 AND max(p.width, p.height) < 2 * min(p.width, p.height)
         AND abs(p.width - p.height) > 0.05 * max(p.width, p.height)
         AND (p.width > p.height) != (p.orientation NOT IN (5, 6, 7, 8))",
    ),
    (
        "landscape",
        "p.width > 0 AND p.height > 0 AND max(p.width, p.height) < 2 * min(p.width, p.height)
         AND abs(p.width - p.height) > 0.05 * max(p.width, p.height)
         AND (p.width > p.height) = (p.orientation NOT IN (5, 6, 7, 8))",
    ),
    (
        "square",
        "p.width > 0 AND p.height > 0 AND abs(p.width - p.height) <= 0.05 * max(p.width, p.height)",
    ),
    (
        "panorama",
        "p.width > 0 AND p.height > 0 AND max(p.width, p.height) >= 2 * min(p.width, p.height)",
    ),
];

/// How many faces the face pass found: solo portraits, couples, groups.
/// A copy has its original's; a photo not yet looked at has no count.
pub const FACE_COUNTS: &Ranges = &[
    ("0", "COALESCE(p.face_count, (SELECT m.face_count FROM photos m WHERE m.id = p.variant_of)) = 0"),
    ("1", "COALESCE(p.face_count, (SELECT m.face_count FROM photos m WHERE m.id = p.variant_of)) = 1"),
    ("2", "COALESCE(p.face_count, (SELECT m.face_count FROM photos m WHERE m.id = p.variant_of)) = 2"),
    ("3-5", "COALESCE(p.face_count, (SELECT m.face_count FROM photos m WHERE m.id = p.variant_of)) BETWEEN 3 AND 5"),
    ("6-", "COALESCE(p.face_count, (SELECT m.face_count FROM photos m WHERE m.id = p.variant_of)) >= 6"),
];

/// "Any of these ranges", for the keys the table knows; `None` when none of the
/// chosen keys is known, so a stale key narrows nothing rather than everything.
fn ranges_clause(ranges: &Ranges, keys: &[String]) -> Option<String> {
    let tests: Vec<String> = ranges
        .iter()
        .filter(|(k, _)| keys.iter().any(|c| c == k))
        .map(|(_, sql)| format!("({sql})"))
        .collect();
    (!tests.is_empty()).then(|| format!("({})", tests.join(" OR ")))
}

/// The key of the range a photo falls in, as a `CASE` over the table.
pub(super) fn ranges_case(ranges: &Ranges) -> String {
    let arms: String = ranges
        .iter()
        .map(|(k, sql)| format!(" WHEN {sql} THEN '{k}'"))
        .collect();
    format!("CASE{arms} END")
}

/// The status toggles, each a name `build_where` can skip and the SQL
/// test it adds. A virtual copy has no faces of its own — they belong to the
/// photo it was made from — and is made without its position, so those two
/// tests fall back to its master's; only for a copy, so an ordinary photo
/// never pays for the lookup.
pub(super) const STATUS: [(&str, &str); 5] = [
    ("edited", "p.develop_settings IS NOT NULL"),
    ("copies", "p.variant > 0"),
    (
        "faces",
        "(p.face_count > 0 OR (p.variant_of IS NOT NULL
          AND (SELECT m.face_count FROM photos m WHERE m.id = p.variant_of) > 0))",
    ),
    (
        "located",
        "(p.lat IS NOT NULL OR (p.variant_of IS NOT NULL
          AND (SELECT m.lat FROM photos m WHERE m.id = p.variant_of) IS NOT NULL))",
    ),
    (
        "unplaced",
        "NOT EXISTS (SELECT 1 FROM photo_places pp WHERE pp.photo_id = p.id)",
    ),
];

/// Whether a status toggle is on in `f`, by its name in [`STATUS`].
fn status_on(f: &PhotoFilter, name: &str) -> bool {
    match name {
        "edited" => f.edited_only,
        "copies" => f.copies_only,
        "faces" => f.faces_only,
        "located" => f.located_only,
        "unplaced" => f.unplaced_only,
        _ => false,
    }
}

/// What [`build_where`] returns when nothing constrains the photos at all, so a
/// caller can skip evaluating the filter rather than pay for a tautology.
pub(super) const NO_FILTER: &str = "1 = 1";

/// A WHERE clause over `photos p` and the parameters it binds, as
/// [`build_where`] returns it.
pub(super) type Clause = (String, Vec<Value>);

/// Builds the WHERE clause for a filter. `skip` omits one facet's own constraint
/// so that facet's counts stay meaningful while it is being used. `whitelist`, if
/// given, restricts to a specific set of photo ids — how a natural-language
/// (`semantic`) query, resolved to a ranked id set outside SQL, is intersected
/// with the ordinary facets and with the facet counts alike.
pub(super) fn build_where(
    f: &PhotoFilter,
    skip: Option<&str>,
    whitelist: Option<&[i32]>,
) -> Clause {
    let mut clauses: Vec<String> = Vec::new();
    let mut p: Vec<Value> = Vec::new();

    if let Some(ids) = whitelist {
        // An empty whitelist means "the semantic query matched nothing" — force
        // no rows rather than degenerating to `IN ()` (a syntax error) or, worse,
        // matching everything.
        if ids.is_empty() {
            clauses.push("0 = 1".into());
        } else {
            clauses.push(format!("p.id IN ({})", placeholders(ids.len())));
            for id in ids {
                p.push(Value::Integer(i64::from(*id)));
            }
        }
    }

    if let Some(d) = &f.date_from {
        clauses.push("p.capture_date >= ?".into());
        p.push(Value::Text(d.clone()));
    }
    if let Some(d) = &f.date_to {
        // Inclusive of the whole end day.
        clauses.push("p.capture_date <= ?".into());
        p.push(Value::Text(format!("{d} 23:59:59")));
    }
    if !f.genres.is_empty() && skip != Some("genre") {
        clauses.push(tag_membership_clause("genre", &f.genres, &mut p));
    }
    // Keywords chosen together: any of them, or all of them. Counting
    // the Keywords facet lifts an "any" choice, as every facet does, so the
    // others stay on offer; an "all" choice stays, so each option's count is
    // what adding it would leave.
    if !f.subjects.is_empty() && (skip != Some("subject") || f.subjects_all) {
        if f.subjects_all {
            for s in &f.subjects {
                clauses.push(keyword_membership_clause(
                    std::slice::from_ref(s),
                    f.own_keywords,
                    &mut p,
                ));
            }
        } else {
            clauses.push(keyword_membership_clause(
                &f.subjects,
                f.own_keywords,
                &mut p,
            ));
        }
    }
    if !f.conditions.is_empty() && skip != Some("condition") {
        clauses.push(tag_membership_clause("condition", &f.conditions, &mut p));
    }
    // Left out, and never lifted, even while their own facet is
    // counted: a count that included photos the filter hides would promise
    // more than a click delivers.
    if !f.exclude_genres.is_empty() {
        clauses.push(format!(
            "NOT ({})",
            tag_membership_clause("genre", &f.exclude_genres, &mut p)
        ));
    }
    if !f.exclude_subjects.is_empty() {
        clauses.push(format!(
            "NOT ({})",
            keyword_membership_clause(&f.exclude_subjects, f.own_keywords, &mut p)
        ));
    }
    if !f.exclude_conditions.is_empty() {
        clauses.push(format!(
            "NOT ({})",
            tag_membership_clause("condition", &f.exclude_conditions, &mut p)
        ));
    }
    if !f.place_ids.is_empty() && skip != Some("place") {
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM photo_places pp WHERE pp.photo_id = p.id AND pp.place_id IN ({}))",
            placeholders(f.place_ids.len())
        ));
        for id in &f.place_ids {
            p.push(Value::Integer(i64::from(*id)));
        }
    }
    if let Some(r) = f.min_rating {
        clauses.push("p.rating >= ?".into());
        p.push(Value::Integer(i64::from(r)));
    }
    if !f.ratings.is_empty() && skip != Some("rating") {
        clauses.push(format!("p.rating IN ({})", placeholders(f.ratings.len())));
        for v in &f.ratings {
            p.push(Value::Integer(i64::from(*v)));
        }
    }
    if !f.flags.is_empty() && skip != Some("flag") {
        clauses.push(format!("p.flag IN ({})", placeholders(f.flags.len())));
        for v in &f.flags {
            p.push(Value::Integer(i64::from(*v)));
        }
    }
    if !f.color_labels.is_empty() && skip != Some("color") {
        clauses.push(format!(
            "p.color_label IN ({})",
            placeholders(f.color_labels.len())
        ));
        for v in &f.color_labels {
            p.push(Value::Integer(i64::from(*v)));
        }
    }
    if let Some(cid) = f.collection_id {
        clauses.push(
            "EXISTS (SELECT 1 FROM collection_photos cp \
             WHERE cp.photo_id = p.id AND cp.collection_id = ?)"
                .into(),
        );
        p.push(Value::Integer(i64::from(cid)));
    }
    // Each person must be in the photo — AND, so one clause per person. Faces
    // belong to the master, so a virtual copy is matched through it.
    for pid in f.person_id.iter().chain(f.person_ids.iter()) {
        clauses.push(
            "EXISTS (SELECT 1 FROM faces fa
                     WHERE fa.photo_id = COALESCE(p.variant_of, p.id) AND fa.person_id = ?)"
                .into(),
        );
        p.push(Value::Integer(i64::from(*pid)));
    }
    for (column, values, name) in [
        ("camera", &f.cameras, "camera"),
        ("lens", &f.lenses, "lens"),
    ] {
        if !values.is_empty() && skip != Some(name) {
            clauses.push(format!("p.{column} IN ({})", placeholders(values.len())));
            for v in values {
                p.push(Value::Text(v.clone()));
            }
        }
    }
    for (ranges, keys, name) in [
        (FOCAL_RANGES, &f.focal_ranges, "focal"),
        (ISO_RANGES, &f.iso_ranges, "iso"),
        (APERTURE_RANGES, &f.aperture_ranges, "aperture"),
        (FILE_KINDS, &f.file_kinds, "kind"),
        (SHAPES, &f.shapes, "shape"),
        (FACE_COUNTS, &f.face_counts, "facecount"),
    ] {
        if skip != Some(name) {
            if let Some(clause) = ranges_clause(ranges, keys) {
                clauses.push(clause);
            }
        }
    }
    if !f.palette.is_empty() && skip != Some("palette") {
        let mask = palette_mask(&f.palette);
        clauses.push("p.id IN (SELECT photo_id FROM photo_palette WHERE swatches & ? != 0)".into());
        p.push(Value::Integer(mask));
    }
    for (name, test) in STATUS {
        if status_on(f, name) && skip != Some(name) {
            clauses.push(test.to_string());
        }
    }
    if f.untagged_only {
        clauses.push("p.tagged_at IS NULL".into());
    }
    if !f.folder_ids.is_empty() && skip != Some("folder") {
        clauses.push(format!(
            "p.folder_id IN ({})",
            placeholders(f.folder_ids.len())
        ));
        for id in &f.folder_ids {
            p.push(Value::Integer(i64::from(*id)));
        }
    }
    if let Some(shoot) = f.shoot_id {
        // A copy shares its original's path, so it belongs to the shoot too.
        clauses.push("p.path IN (SELECT path FROM shoot_files WHERE shoot_id = ?)".into());
        p.push(Value::Integer(i64::from(shoot)));
    }
    if f.missing_only && skip != Some("missing") {
        clauses.push("p.missing = 1".into());
    }
    if let Some(s) = &f.search {
        let s = s.trim();
        if !s.is_empty() {
            // As typed: the filename, the caption, a tag, a place, or someone
            // in it — on the original, for a copy. The last three are
            // lists, not correlated EXISTS: the few tags, places and people
            // that match are found once, and each photo is looked up in what
            // they hold. Matching them per photo cost a pattern test on every
            // tag of every photo — 314 ms of the rail's counts at 10,000.
            //
            // And the whole match is a list too, found once per
            // statement from idx_photos_text: the filename and caption tested
            // in a small index entry instead of a photo row, which carries a
            // ~3 KB embedding, then each matching photo looked up by id. As a
            // condition on `p` it read every row, in every one of the rail's
            // statements.
            let like = format!("%{s}%");
            clauses.push(
                "p.id IN (SELECT s.id FROM photos s
                  WHERE s.filename LIKE ?
                     OR s.caption LIKE ?
                     OR s.id IN (SELECT pt.photo_id FROM photo_tags pt
                                  WHERE pt.tag_id IN (SELECT id FROM tags WHERE name LIKE ?))
                     OR s.id IN (SELECT pp.photo_id FROM photo_places pp
                                  WHERE pp.place_id IN (SELECT id FROM places WHERE name LIKE ?))
                     OR COALESCE(s.variant_of, s.id) IN
                          (SELECT f.photo_id FROM faces f
                            WHERE f.person_id IN (SELECT id FROM people WHERE name LIKE ?)))"
                    .into(),
            );
            for _ in 0..5 {
                p.push(Value::Text(like.clone()));
            }
        }
    }

    let sql = if clauses.is_empty() {
        NO_FILTER.to_string()
    } else {
        clauses.join(" AND ")
    };
    (sql, p)
}

/// Every photo query's columns, read from [`PHOTO_FROM`], in the order
/// [`PHOTO_COLUMN_NAMES`] lists them.
const PHOTO_COLUMNS: &str = r#"
    p.id, p.path, p.filename, p.capture_date, p.camera, p.lens, p.iso, p.aperture,
    p.shutter, p.focal_length, p.width, p.height, p.orientation, p.thumb_path,
    p.preview_path, p.rating, p.flag, p.color_label, p.tagged_at, p.caption, p.sharpness,
    p.brightness, p.face_count, p.face_sharpness, p.eyes_open, p.eyes_override,
    p.variant, p.variant_name, p.missing, p.capture_edited, p.flag_suggested,
    -- The grid has no other way to know a photo carries edits: the stack is a
    -- JSON blob it never sees, and `set_develop_settings` stores NULL for a
    -- default stack, so "not null" is exactly "edited".
    (p.develop_settings IS NOT NULL) AS edited,
    p.edited_path,
    -- The photo's tags as ids alone, read from the link table's own index;
    -- `TagNames` turns them into the genre and the subjects. Joining `tags` here
    -- cost a lookup per link, nine a photo, and was most of a large page.
    (SELECT group_concat(pt.tag_id) FROM photo_tags pt WHERE pt.photo_id = p.id) AS tag_ids,
    photo_place.place_id AS place_id,
    place_row.name AS place
"#;

/// [`PHOTO_COLUMNS`] for a list of photos, with no caption: the vision
/// model's few sentences a photo, which only the one photo on show displays,
/// fetched with [`get_photo`]. At 20,000 photos they were most of the page
/// sent to the window on every reload. The same columns otherwise, in the
/// same places.
const LIST_COLUMNS: &str = r#"
    p.id, p.path, p.filename, p.capture_date, p.camera, p.lens, p.iso, p.aperture,
    p.shutter, p.focal_length, p.width, p.height, p.orientation, p.thumb_path,
    p.preview_path, p.rating, p.flag, p.color_label, p.tagged_at, NULL, p.sharpness,
    p.brightness, p.face_count, p.face_sharpness, p.eyes_open, p.eyes_override,
    p.variant, p.variant_name, p.missing, p.capture_edited, p.flag_suggested,
    -- The grid has no other way to know a photo carries edits: the stack is a
    -- JSON blob it never sees, and `set_develop_settings` stores NULL for a
    -- default stack, so "not null" is exactly "edited".
    (p.develop_settings IS NOT NULL) AS edited,
    p.edited_path,
    -- The photo's tags as ids alone, read from the link table's own index;
    -- `TagNames` turns them into the genre and the subjects. Joining `tags` here
    -- cost a lookup per link, nine a photo, and was most of a large page.
    (SELECT group_concat(pt.tag_id) FROM photo_tags pt WHERE pt.photo_id = p.id) AS tag_ids,
    photo_place.place_id AS place_id,
    place_row.name AS place
"#;

/// What [`PHOTO_COLUMNS`] reads from: the photo, and its place. A photo has at
/// most one place (`photo_places` is keyed by the photo), so the joins never
/// repeat a row — they replace two subqueries per row that looked it up twice.
const PHOTO_FROM: &str = "photos p
    LEFT JOIN photo_places photo_place ON photo_place.photo_id = p.id
    LEFT JOIN places place_row ON place_row.id = photo_place.place_id";

/// The names of [`PHOTO_COLUMNS`]' result columns, in order.
///
/// [`row_to_photo`] reads each column by its position here, worked out at
/// compile time. `Row::get("name")` finds a column by asking SQLite for each
/// column's name in turn and comparing — 33 columns a row, that way, was about
/// 100 ms of a 20,000-photo page at 50,000 photos, as much as the query itself.
/// `photo_columns_are_where_row_to_photo_reads_them` keeps the two in step.
const PHOTO_COLUMN_NAMES: [&str; 36] = [
    "id",
    "path",
    "filename",
    "capture_date",
    "camera",
    "lens",
    "iso",
    "aperture",
    "shutter",
    "focal_length",
    "width",
    "height",
    "orientation",
    "thumb_path",
    "preview_path",
    "rating",
    "flag",
    "color_label",
    "tagged_at",
    "caption",
    "sharpness",
    "brightness",
    "face_count",
    "face_sharpness",
    "eyes_open",
    "eyes_override",
    "variant",
    "variant_name",
    "missing",
    "capture_edited",
    "flag_suggested",
    "edited",
    "edited_path",
    "tag_ids",
    "place_id",
    "place",
];

/// The position of `name` in [`PHOTO_COLUMN_NAMES`]. Only ever evaluated at
/// compile time, where a name that is not in the list fails the build.
const fn photo_column(name: &str) -> usize {
    let mut i = 0;
    while i < PHOTO_COLUMN_NAMES.len() {
        let known = PHOTO_COLUMN_NAMES[i].as_bytes();
        let wanted = name.as_bytes();
        if known.len() == wanted.len() {
            let mut j = 0;
            while j < known.len() && known[j] == wanted[j] {
                j += 1;
            }
            if j == known.len() {
                return i;
            }
        }
        i += 1;
    }
    panic!("not one of PHOTO_COLUMN_NAMES")
}

/// The genre and subject tags by id, which is all [`PHOTO_COLUMNS`] carries of
/// a photo's tags. A few thousand rows, read once per query.
pub(super) struct TagNames(pub(super) std::collections::HashMap<i64, (bool, String)>);

impl TagNames {
    /// Every genre and subject, or with `photo` only that photo's.
    pub(super) fn load(conn: &Connection, photo: Option<i32>) -> Result<Self> {
        let mut stmt = conn.prepare(
            "SELECT id, category = 'genre', name FROM tags
              WHERE category IN ('genre', 'subject')
                AND (?1 IS NULL OR id IN (SELECT tag_id FROM photo_tags WHERE photo_id = ?1))",
        )?;
        let names = stmt
            .query_map(params![photo], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Self(names))
    }

    /// The genre and the subjects among `tag_ids`, a comma-separated list in
    /// the link table's order. A photo has one genre; should it somehow carry
    /// two, the first is the one shown, as the `LIMIT 1` this replaced did.
    fn split(&self, tag_ids: Option<&str>) -> (Option<String>, Vec<String>) {
        let mut genre = None;
        let mut subjects = Vec::new();
        for id in tag_ids.into_iter().flat_map(|t| t.split(',')) {
            match id.parse().ok().and_then(|id: i64| self.0.get(&id)) {
                Some((true, name)) => {
                    genre.get_or_insert_with(|| name.clone());
                }
                Some((false, name)) => subjects.push(name.clone()),
                None => {}
            }
        }
        (genre, subjects)
    }
}

fn row_to_photo(r: &Row, tags: &TagNames) -> rusqlite::Result<Photo> {
    macro_rules! col {
        ($name:literal) => {
            const { photo_column($name) }
        };
    }
    let tagged_at: Option<String> = r.get(col!("tagged_at"))?;
    let tag_ids: Option<String> = r.get(col!("tag_ids"))?;
    let (genre, subjects) = tags.split(tag_ids.as_deref());
    Ok(Photo {
        id: r.get(col!("id"))?,
        path: r.get(col!("path"))?,
        filename: r.get(col!("filename"))?,
        capture_date: r.get(col!("capture_date"))?,
        camera: r.get(col!("camera"))?,
        lens: r.get(col!("lens"))?,
        iso: r.get(col!("iso"))?,
        aperture: r.get(col!("aperture"))?,
        shutter: r.get(col!("shutter"))?,
        focal_length: r.get(col!("focal_length"))?,
        width: r.get(col!("width"))?,
        height: r.get(col!("height"))?,
        orientation: r.get(col!("orientation"))?,
        thumb_path: r.get(col!("thumb_path"))?,
        preview_path: r.get(col!("preview_path"))?,
        rating: r.get(col!("rating"))?,
        flag: r.get(col!("flag"))?,
        color_label: r.get(col!("color_label"))?,
        tagged: tagged_at.is_some(),
        genre,
        subjects,
        place: r.get(col!("place"))?,
        place_id: r.get(col!("place_id"))?,
        caption: r.get(col!("caption"))?,
        sharpness: r.get(col!("sharpness"))?,
        brightness: r.get(col!("brightness"))?,
        face_count: r.get(col!("face_count"))?,
        face_sharpness: r.get(col!("face_sharpness"))?,
        eyes_open: r.get(col!("eyes_open"))?,
        eyes_override: r.get(col!("eyes_override"))?,
        variant: r.get(col!("variant"))?,
        variant_name: r.get(col!("variant_name"))?,
        edited: r.get::<_, i64>(col!("edited"))? != 0,
        edited_path: r.get(col!("edited_path"))?,
        missing: r.get::<_, i64>(col!("missing"))? != 0,
        capture_edited: r.get::<_, i64>(col!("capture_edited"))? != 0,
        flag_suggested: r.get::<_, i64>(col!("flag_suggested"))? != 0,
    })
}

pub fn query_photos(
    conn: &Connection,
    f: &PhotoFilter,
    limit: i32,
    offset: i32,
) -> Result<Vec<Photo>> {
    let (where_sql, mut p) = build_where(f, None, None);
    let order = order_by(f.sort, f.collection_id);
    let sql = format!(
        "SELECT {LIST_COLUMNS} FROM {PHOTO_FROM} WHERE {where_sql} ORDER BY {order} LIMIT ? OFFSET ?"
    );
    p.push(Value::Integer(i64::from(limit)));
    p.push(Value::Integer(i64::from(offset)));

    let tags = TagNames::load(conn, None)?;
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(p), |r| row_to_photo(r, &tags))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The grid's `ORDER BY` for a sort.
///
/// Copies sort immediately after the photo they came from — grouping on the
/// master's id, then ordering by variant within the group — rather than landing
/// wherever their own id falls. Seeing an edit next to its original is the
/// entire point of having made one. Where the key is something a copy does not
/// have of its own (when it was added, its focus score), it borrows its
/// master's, so it still sorts beside it.
///
/// Newest first is what the expression index of migration 14 answers; the
/// others pay a sort, measured in the scale test.
fn order_by(sort: PhotoSort, collection: Option<i32>) -> String {
    let order = match (sort, collection) {
        // A collection's hand order: its photos by their place in it.
        // The id is an integer from the filter, so it goes in as text safely.
        (PhotoSort::Custom, Some(c)) => {
            return format!(
                "(SELECT cp.position FROM collection_photos cp
                   WHERE cp.collection_id = {c} AND cp.photo_id = p.id) ASC NULLS LAST, p.id ASC"
            );
        }
        (PhotoSort::Custom, None) => PhotoSort::Newest,
        (other, _) => other,
    };
    match order {
        PhotoSort::Newest => {
            "p.capture_date DESC, COALESCE(p.variant_of, p.id) DESC, p.variant ASC"
        }
        PhotoSort::Oldest => "p.capture_date ASC, COALESCE(p.variant_of, p.id) ASC, p.variant ASC",
        PhotoSort::Added => {
            "CASE WHEN p.variant_of IS NULL THEN p.indexed_at
                  ELSE (SELECT m.indexed_at FROM photos m WHERE m.id = p.variant_of) END DESC,
             COALESCE(p.variant_of, p.id) DESC, p.variant ASC"
        }
        PhotoSort::Rating => {
            "p.rating DESC, p.capture_date DESC, COALESCE(p.variant_of, p.id) DESC, p.variant ASC"
        }
        PhotoSort::Name => "p.filename COLLATE NOCASE ASC, COALESCE(p.variant_of, p.id) ASC, p.variant ASC",
        PhotoSort::Sharpest => {
            "CASE WHEN p.variant_of IS NULL THEN p.sharpness
                  ELSE (SELECT m.sharpness FROM photos m WHERE m.id = p.variant_of) END DESC NULLS LAST,
             COALESCE(p.variant_of, p.id) DESC, p.variant ASC"
        }
        PhotoSort::Custom => unreachable!("resolved above"),
    }
    .to_string()
}

/// Like [`query_photos`] but ordered by an externally-computed relevance ranking
/// (a natural-language search) instead of by date. `ranked_ids` is the fused
/// candidate list, best first; this keeps only the ones that also pass the rest
/// of the filter, preserves that relevance order, and returns the requested page.
///
/// The ordering is done in Rust, not SQL: `IN (…)` does not preserve order, and
/// `ranked_ids` is already bounded (the command layer caps it), so filtering and
/// slicing a few hundred ids in memory is simpler and cheaper than a giant
/// `ORDER BY CASE`.
/// Every photo the filter's ordinary facets match, ignoring its content query:
/// the set a content search ranks within.
pub fn ids_matching(conn: &Connection, f: &PhotoFilter) -> Result<Vec<i32>> {
    let (where_sql, p) = build_where(f, None, None);
    let mut stmt = conn.prepare(&format!("SELECT p.id FROM photos p WHERE {where_sql}"))?;
    let ids = stmt
        .query_map(rusqlite::params_from_iter(p), |r| r.get::<_, i32>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

pub fn query_photos_ranked(
    conn: &Connection,
    f: &PhotoFilter,
    ranked_ids: &[i32],
    limit: i32,
    offset: i32,
) -> Result<Vec<Photo>> {
    if ranked_ids.is_empty() {
        return Ok(Vec::new());
    }
    // Which of the ranked candidates survive the ordinary facets.
    let (where_sql, p) = build_where(f, None, Some(ranked_ids));
    let sql = format!("SELECT p.id FROM photos p WHERE {where_sql}");
    let mut stmt = conn.prepare(&sql)?;
    let matched: std::collections::HashSet<i32> = stmt
        .query_map(rusqlite::params_from_iter(p), |r| r.get::<_, i32>(0))?
        .collect::<rusqlite::Result<_>>()?;

    let page: Vec<i32> = ranked_ids
        .iter()
        .copied()
        .filter(|id| matched.contains(id))
        .skip(offset.max(0) as usize)
        .take(limit.max(0) as usize)
        .collect();
    photos_by_ids(conn, &page)
}

pub fn get_photo(conn: &Connection, id: i32) -> Result<Option<Photo>> {
    let sql = format!("SELECT {PHOTO_COLUMNS} FROM {PHOTO_FROM} WHERE p.id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![id])?;
    Ok(match rows.next()? {
        Some(r) => Some(row_to_photo(r, &TagNames::load(conn, Some(id))?)?),
        None => None,
    })
}

/// Fetches photos by id, **returned in the given id order** — SQL `IN` does not
/// preserve order, so the rows are re-sorted to match the search ranking.
pub fn photos_by_ids(conn: &Connection, ids: &[i32]) -> Result<Vec<Photo>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let tags = TagNames::load(conn, None)?;
    // In chunks: SQLite binds at most 32,766 variables, and a library
    // with more burst frames than that made duplicate review fail with "too
    // many SQL variables".
    let mut photos: Vec<Photo> = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(ID_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!("SELECT {LIST_COLUMNS} FROM {PHOTO_FROM} WHERE p.id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            row_to_photo(r, &tags)
        })?;
        photos.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }

    let rank: std::collections::HashMap<i32, usize> =
        ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    photos.sort_by_key(|p| rank.get(&p.id).copied().unwrap_or(usize::MAX));
    Ok(photos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    #[test]
    fn ranked_query_keeps_relevance_order_intersects_facets_and_pages() {
        let conn = open_memory().unwrap();

        // Five photos; give a couple a rating so a facet can bite.
        let mut ids = Vec::new();
        for i in 0..5 {
            conn.execute(
                "INSERT INTO photos (path, filename, file_hash, indexed_at)
                 VALUES (?1, 'f.arw', 'h', '2026-01-01')",
                params![format!("/p{i}.arw")],
            )
            .unwrap();
            ids.push(conn.last_insert_rowid() as i32);
        }
        set_rating(&conn, ids[1], 5).unwrap();
        set_rating(&conn, ids[3], 5).unwrap();

        let empty = PhotoFilter {
            date_from: None,
            date_to: None,
            genres: vec![],
            subjects: vec![],
            conditions: vec![],
            place_ids: vec![],
            min_rating: None,
            ratings: vec![],
            flags: vec![],
            color_labels: vec![],
            collection_id: None,
            person_id: None,
            untagged_only: false,
            folder_ids: vec![],
            missing_only: false,
            search: None,
            semantic: Some("anything".into()),
            ..Default::default()
        };

        // A deliberately non-id, non-date order — this is what the ranking must
        // survive (query_photos would instead sort by date).
        let ranked = vec![ids[3], ids[0], ids[4], ids[1], ids[2]];

        // No other facet: the page is exactly the ranked order.
        let page = query_photos_ranked(&conn, &empty, &ranked, 10, 0).unwrap();
        assert_eq!(page.iter().map(|p| p.id).collect::<Vec<_>>(), ranked);

        // A rating facet intersects the ranked set, keeping relevance order —
        // only the two 5-star photos, in the order they appeared in `ranked`.
        let starred = PhotoFilter {
            min_rating: Some(5),
            ..empty.clone()
        };
        let page = query_photos_ranked(&conn, &starred, &ranked, 10, 0).unwrap();
        assert_eq!(
            page.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![ids[3], ids[1]]
        );

        // Paging slices the ranked list, not a date-ordered one.
        let p1 = query_photos_ranked(&conn, &empty, &ranked, 2, 0).unwrap();
        let p2 = query_photos_ranked(&conn, &empty, &ranked, 2, 2).unwrap();
        assert_eq!(
            p1.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![ids[3], ids[0]]
        );
        assert_eq!(
            p2.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![ids[4], ids[1]]
        );

        // An empty candidate list (semantic matched nothing) yields nothing —
        // never the whole library.
        assert!(query_photos_ranked(&conn, &empty, &[], 10, 0)
            .unwrap()
            .is_empty());
    }

    /// `row_to_photo` reads by position, so the query and the list of names
    /// it reads by must not drift apart.
    #[test]
    fn a_list_of_photos_leaves_the_caption_to_the_photo_on_show() {
        // The same columns in the same places, the caption alone left out.
        assert_eq!(LIST_COLUMNS, PHOTO_COLUMNS.replace("p.caption,", "NULL,"));
        let conn = open_memory().unwrap();
        add_photos(&conn, 1);
        conn.execute(
            "UPDATE photos SET caption = 'A heron at dawn' WHERE id = 1",
            [],
        )
        .unwrap();
        let listed = query_photos(&conn, &PhotoFilter::default(), 10, 0).unwrap();
        assert_eq!(listed[0].caption, None);
        assert_eq!(photos_by_ids(&conn, &[1]).unwrap()[0].caption, None);
        assert_eq!(
            get_photo(&conn, 1).unwrap().unwrap().caption.as_deref(),
            Some("A heron at dawn")
        );
    }

    #[test]
    fn photo_columns_are_where_row_to_photo_reads_them() {
        let conn = open_memory().unwrap();
        let stmt = conn
            .prepare(&format!("SELECT {PHOTO_COLUMNS} FROM {PHOTO_FROM}"))
            .unwrap();
        assert_eq!(stmt.column_names(), PHOTO_COLUMN_NAMES);
    }

    /// A photo's genre, subjects and place come back the same through every
    /// query that returns photos: from tag ids and a join now, where each was
    /// a subquery of its own.
    #[test]
    fn photos_carry_their_genre_subjects_and_place() {
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date)
               VALUES (1, '/a.arw', 'a.arw', 'h1', '2026-01-01', '2026-01-02'),
                      (2, '/b.arw', 'b.arw', 'h2', '2026-01-01', '2026-01-01');
             INSERT INTO tags (id, name, category) VALUES
               (10, 'tree', 'subject'), (11, 'landscape', 'genre'),
               (12, 'mountain', 'subject'), (13, 'fog', 'condition');
             INSERT INTO photo_tags (photo_id, tag_id) VALUES (1, 12), (1, 13), (1, 10), (1, 11);
             INSERT INTO places (id, name) VALUES (7, 'Karoo');
             INSERT INTO photo_places (photo_id, place_id) VALUES (1, 7);",
        )
        .unwrap();

        let check = |p: &Photo| {
            if p.id == 1 {
                assert_eq!(p.genre.as_deref(), Some("landscape"));
                // In the link table's order, and never a condition.
                assert_eq!(p.subjects, ["tree", "mountain"]);
                assert_eq!((p.place.as_deref(), p.place_id), (Some("Karoo"), Some(7)));
            } else {
                assert_eq!(p.genre, None);
                assert!(p.subjects.is_empty());
                assert_eq!((p.place.as_deref(), p.place_id), (None, None));
            }
        };
        let page = query_photos(&conn, &PhotoFilter::default(), 10, 0).unwrap();
        assert_eq!(page.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2]);
        page.iter().for_each(check);
        check(&get_photo(&conn, 1).unwrap().unwrap());
        check(&get_photo(&conn, 2).unwrap().unwrap());
        let by_ids = photos_by_ids(&conn, &[2, 1]).unwrap();
        assert_eq!(by_ids.iter().map(|p| p.id).collect::<Vec<_>>(), [2, 1]);
        by_ids.iter().for_each(check);
    }

    #[test]
    fn every_sort_keeps_a_copy_beside_its_original() {
        let conn = gear_library();
        let order = |sort| {
            ids(
                &conn,
                &PhotoFilter {
                    sort,
                    ..Default::default()
                },
            )
        };
        assert_eq!(order(PhotoSort::Newest), vec![4, 3, 2, 1, 5]);
        assert_eq!(order(PhotoSort::Oldest), vec![1, 5, 2, 3, 4]);
        assert_eq!(order(PhotoSort::Added), vec![4, 1, 5, 2, 3]);
        // The copy has its own rating (0) — rated, it sorts by that.
        assert_eq!(order(PhotoSort::Rating), vec![2, 1, 4, 3, 5]);
        // Case-insensitive: "B.arw" between "a" and "c".
        assert_eq!(order(PhotoSort::Name), vec![1, 5, 2, 3, 4]);
        // Unscored last; the copy borrows its master's score.
        assert_eq!(order(PhotoSort::Sharpest), vec![2, 1, 5, 3, 4]);
    }

    /// With no virtual copies — most catalogues — every `variant_of` is NULL,
    /// and the planner statistics say one value covers every photo. On an
    /// older SQLite that made the delete cascade scan the whole index for each
    /// photo, and removing a 20,000-photo folder took minutes. The
    /// bundled SQLite searches; this keeps it so if either changes.
    #[test]
    fn a_photos_copies_are_found_by_search_with_no_copies_anywhere() {
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000)
             INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             SELECT i, 'D:/p/' || i || '.arw', i || '.arw', 'h' || i, 'x' FROM n;",
        )
        .unwrap();
        refresh_planner_stats(&conn).unwrap();
        // What the foreign key's ON DELETE CASCADE runs for each deleted photo.
        let plan: String = conn
            .prepare("EXPLAIN QUERY PLAN SELECT 1 FROM photos WHERE variant_of = ?1")
            .unwrap()
            .query_map([7], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join(" | ");
        assert!(
            plan.contains("SEARCH") && plan.contains("idx_photos_variant_of"),
            "the copies of a photo are scanned for: {plan}"
        );
    }

    /// The set a content search ranks within: the ordinary facets, and not
    /// the content query itself.
    #[test]
    fn ids_matching_applies_the_facets_but_not_the_content_query() {
        let conn = open_memory().unwrap();
        add_photos(&conn, 3);
        conn.execute("UPDATE photos SET rating = 4 WHERE id = 2", [])
            .unwrap();
        let f = PhotoFilter {
            min_rating: Some(4),
            semantic: Some("a beach".into()),
            ..Default::default()
        };
        assert_eq!(ids_matching(&conn, &f).unwrap(), vec![2]);
    }

    /// More ids than SQLite binds at once still come back, in order.
    #[test]
    fn photos_by_ids_takes_more_ids_than_one_statement_binds() {
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 33000)
             INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             SELECT i, 'D:/p/' || i, i || '.arw', 'h' || i, 'x' FROM n;",
        )
        .unwrap();
        let ids: Vec<i32> = (1..=33000).rev().collect();
        let photos = photos_by_ids(&conn, &ids).unwrap();
        assert_eq!(photos.len(), 33000);
        assert_eq!(photos[0].id, 33000);
    }
}
