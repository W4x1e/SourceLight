//! Facet counts: how many photos in view have each value of each filter.

use super::*;

/// The tag categories the filter rail counts, in the order [`Facets`] holds them.
const TAG_FACETS: [&str; 3] = ["genre", "subject", "condition"];

/// Restricts a facet count to the photos a filter admits — or to nothing extra
/// at all when there is no filter, so the unfiltered rail never touches `photos`.
///
/// A semijoin on the photo id, so the filter is evaluated once per *photo* and
/// its result probed by id, rather than once per tag link the way a join through
/// `photos` evaluates it (a photo carries ~9 links).
fn restrict_to(photo_id: &str, where_sql: &str) -> String {
    if where_sql == NO_FILTER {
        String::new()
    } else {
        format!("AND {photo_id} IN (SELECT p.id FROM photos p WHERE {where_sql})")
    }
}

/// Every tag facet's counts, sibling-aware, in as few statements as the filter
/// allows.
///
/// Each facet is counted with its own constraint lifted, so its options never
/// collapse to zero while it is in use. That makes a facet's WHERE differ from
/// the full filter only while that facet is *active* — every inactive one is
/// counted under the same clause, so they share one statement. With nothing
/// selected, which is how the rail opens, all three are a single query.
///
/// The counts read `photo_tags` grouped by tag rather than joining through
/// `photos` and counting `DISTINCT p.id`. Nothing is lost: the primary key
/// `(photo_id, tag_id)` makes each link a distinct photo, `UNIQUE(name,
/// category)` makes each tag id a distinct name, and foreign keys cascade a
/// deleted photo's links away. The join cost a `photos` probe per link plus a
/// temp b-tree for the `DISTINCT` — 1.26 s for the subject facet alone at 50,000
/// photos, against ~17 ms for all three without it.
fn count_tag_facets(
    conn: &Connection,
    f: &PhotoFilter,
    whitelist: Option<&[i32]>,
) -> Result<[Vec<FacetCount>; 3]> {
    // Categories grouped by the clause they are counted under.
    let mut groups: Vec<(Clause, Vec<&str>)> = Vec::new();
    for category in TAG_FACETS {
        let clause = build_where(f, Some(category), whitelist);
        match groups.iter_mut().find(|(c, _)| *c == clause) {
            Some((_, categories)) => categories.push(category),
            None => groups.push((clause, vec![category])),
        }
    }

    let mut out: [Vec<FacetCount>; 3] = Default::default();
    for ((where_sql, p), categories) in groups {
        // Categories are the constants above, never user input.
        let in_list = categories
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(",");
        // `ft`/`fpt`, not `t`/`pt`: the filter's own tag subqueries use those.
        // With only the user's keywords shown, the tagger's count for nothing
        // among the subjects.
        let own = if f.own_keywords {
            "AND (ft.category != 'subject' OR fpt.source != 'ai')"
        } else {
            ""
        };
        let sql = format!(
            "SELECT ft.category, ft.name, COUNT(*)
             FROM tags ft
             JOIN photo_tags fpt ON fpt.tag_id = ft.id
             WHERE ft.category IN ({in_list}) {own} {}
             GROUP BY ft.id",
            restrict_to("fpt.photo_id", &where_sql)
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(p))?;
        while let Some(r) = rows.next()? {
            let category: String = r.get(0)?;
            if let Some(i) = TAG_FACETS.iter().position(|c| *c == category) {
                out[i].push(FacetCount {
                    value: r.get(1)?,
                    count: r.get(2)?,
                });
            }
        }
    }
    // A keyword with others beneath it counts the photos of its whole branch,
    // since choosing it finds them all.
    let subject = TAG_FACETS.iter().position(|c| *c == "subject").unwrap_or(1);
    let subjects = &mut out[subject];
    let clause = build_where(f, Some("subject"), whitelist);
    for (name, count) in keyword_branch_counts(conn, &clause, f.own_keywords)? {
        match subjects.iter_mut().find(|c| c.value == name) {
            Some(c) => c.count = count,
            None => subjects.push(FacetCount { value: name, count }),
        }
    }
    // Most used first, then by name — byte order, as SQLite's default collation.
    for counts in &mut out {
        counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    }
    Ok(out)
}

/// Photos per keyword **branch**, for each keyword with others filed beneath
/// it: a photo counts once however many of the branch's keywords it has.
///
/// Only the tree's own links are read — those of keywords that are filed or
/// have others filed under them — so a library that has never built a tree
/// reads nothing here, and one that has reads a single indexed pass over its
/// filed keywords' links, never the whole of `photo_tags`.
///
/// The union is taken here rather than in SQL. `COUNT(DISTINCT)` over each
/// link joined to each of its ancestors cost 350 ms on a 50,000-photo
/// catalogue with every keyword filed three deep; sorting the same pairs is a
/// fraction of that, and the read is the only real cost left.
fn keyword_branch_counts(
    conn: &Connection,
    (where_sql, p): &Clause,
    own: bool,
) -> Result<Vec<(String, i32)>> {
    use std::collections::HashMap;
    let mut parent: HashMap<i64, i64> = HashMap::new();
    let mut name: HashMap<i64, String> = HashMap::new();
    {
        let mut stmt = conn.prepare_cached(
            "SELECT t.id, t.parent_id, pt.name FROM tags t JOIN tags pt ON pt.id = t.parent_id
              WHERE t.category = 'subject'",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            parent.insert(r.get(0)?, r.get(1)?);
            name.insert(r.get(1)?, r.get(2)?);
        }
    }
    if parent.is_empty() {
        return Ok(Vec::new());
    }
    // Each keyword's branches: itself when others are filed under it, and
    // every keyword above it. A loop — which nothing makes — is cut short.
    let branches = |tag: i64| -> Vec<i64> {
        let mut out: Vec<i64> = Vec::new();
        if name.contains_key(&tag) {
            out.push(tag);
        }
        let mut at = tag;
        while let Some(&up) = parent.get(&at) {
            if out.contains(&up) || out.len() > 64 {
                break;
            }
            out.push(up);
            at = up;
        }
        out
    };

    let sql = format!(
        "SELECT bpt.tag_id, bpt.photo_id FROM photo_tags bpt
          WHERE bpt.tag_id IN (SELECT id FROM tags WHERE parent_id IS NOT NULL
                               UNION SELECT parent_id FROM tags WHERE parent_id IS NOT NULL) {} {}",
        if own { "AND bpt.source != 'ai'" } else { "" },
        // `+`: the filter checks each link rather than steering the index.
        // Left to itself the planner walks `(tag_id, photo_id)` once per
        // filed keyword *per photo the filter admits* — 7 s at 50,000 photos
        // under a filter that admits most of them.
        restrict_to("+bpt.photo_id", where_sql)
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(p))?;
    let mut memo: HashMap<i64, Vec<i64>> = HashMap::new();
    // (branch, photo), packed so one sort and dedup make each photo count once.
    let mut pairs: Vec<u64> = Vec::new();
    while let Some(r) = rows.next()? {
        let (tag, photo): (i64, i64) = (r.get(0)?, r.get(1)?);
        for &b in memo.entry(tag).or_insert_with(|| branches(tag)).iter() {
            pairs.push(((b as u64) << 32) | (photo as u32 as u64));
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    let mut counts: Vec<(String, i32)> = Vec::new();
    for chunk in pairs.chunk_by(|a, b| a >> 32 == b >> 32) {
        if let Some(n) = name.get(&((chunk[0] >> 32) as i64)) {
            counts.push((n.clone(), chunk.len() as i32));
        }
    }
    Ok(counts)
}

/// Photos per value of an integer column of `photos`, under the filter minus
/// that column's own constraint — so choosing "Picks" does not collapse
/// "Rejects" to zero, the same rule every tag facet follows.
/// `column` and `skip` are constants from the caller, never user input.
fn count_column_facet(
    conn: &Connection,
    f: &PhotoFilter,
    whitelist: Option<&[i32]>,
    skip: &str,
    column: &str,
) -> Result<Vec<FacetCount>> {
    let (where_sql, p) = build_where(f, Some(skip), whitelist);
    let sql =
        format!("SELECT p.{column}, COUNT(*) FROM photos p WHERE {where_sql} GROUP BY p.{column}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(p), |r| {
        Ok(FacetCount {
            value: r.get::<_, i64>(0)?.to_string(),
            count: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The gear facets — each a name `build_where` can skip and the SQL for a
/// photo's value (a blank text value is NULL, so it is counted nowhere).
///
/// Then the kind facets — file type, shape, number of faces — counted in
/// the same pass: they read only columns every scan already has.
pub(super) fn gear_dims() -> [(&'static str, String); DIMS] {
    [
        ("camera", "NULLIF(p.camera, '')".to_string()),
        ("lens", "NULLIF(p.lens, '')".to_string()),
        ("focal", ranges_case(FOCAL_RANGES)),
        ("iso", ranges_case(ISO_RANGES)),
        ("aperture", ranges_case(APERTURE_RANGES)),
        ("kind", ranges_case(FILE_KINDS)),
        ("shape", ranges_case(SHAPES)),
        ("facecount", ranges_case(FACE_COUNTS)),
    ]
}

/// How many dimensions [`gear_dims`] has: the gear's five, then the kinds' three.
const DIMS: usize = 8;

const GEAR_DIMS: std::ops::Range<usize> = 0..5;

/// The gear and status counts: photos per camera, lens and range, and
/// how many each status toggle would leave — each under the filter minus its
/// own constraint, like every facet.
struct GearCounts {
    /// Per dimension, in [`gear_dims`] order: value → photos.
    values: [std::collections::HashMap<String, i32>; DIMS],
    status: [i32; 5],
}

/// Counts every gear dimension and status toggle, in **one scan per distinct
/// clause**. With none of them chosen — how the rail usually is — they all share
/// the full filter, so the whole lot is a single `GROUP BY` over the value
/// combinations, with the status tests summed alongside and each facet's counts
/// added up from the combinations here. Counting them one query each scanned
/// the photos ten times: ~200 ms extra per rail refresh at 10,000 photos.
fn count_gear_and_status(
    conn: &Connection,
    f: &PhotoFilter,
    whitelist: Option<&[i32]>,
    gear: bool,
    kind: bool,
    status: bool,
) -> Result<GearCounts> {
    let dims = gear_dims();
    // Each dimension and toggle, by index: dims 0..DIMS, then the statuses.
    let mut groups: Vec<(Clause, Vec<usize>)> = Vec::new();
    let names = dims
        .iter()
        .map(|(n, _)| *n)
        .chain(STATUS.iter().map(|(n, _)| *n));
    for (i, name) in names.enumerate() {
        let wanted = if GEAR_DIMS.contains(&i) {
            gear
        } else if i < DIMS {
            kind
        } else {
            status
        };
        if !wanted {
            continue;
        }
        let clause = build_where(f, Some(name), whitelist);
        match groups.iter_mut().find(|(c, _)| *c == clause) {
            Some((_, members)) => members.push(i),
            None => groups.push((clause, vec![i])),
        }
    }

    let mut out = GearCounts {
        values: Default::default(),
        status: [0; 5],
    };
    for ((where_sql, p), members) in groups {
        let gear: Vec<usize> = members.iter().copied().filter(|&i| i < DIMS).collect();
        let status: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&i| i >= DIMS)
            .map(|i| i - DIMS)
            .collect();
        let mut select: Vec<String> = gear.iter().map(|&d| dims[d].1.clone()).collect();
        select.push("COUNT(*)".into());
        select.extend(status.iter().map(|&s| {
            format!(
                "COALESCE(SUM(CASE WHEN {} THEN 1 ELSE 0 END), 0)",
                STATUS[s].1
            )
        }));
        let group_by = if gear.is_empty() {
            String::new()
        } else {
            format!(
                " GROUP BY {}",
                (1..=gear.len())
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let sql = format!(
            "SELECT {} FROM photos p WHERE {where_sql}{group_by}",
            select.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(p))?;
        while let Some(r) = rows.next()? {
            let n: i32 = r.get(gear.len())?;
            for (col, &d) in gear.iter().enumerate() {
                if let Some(v) = r.get::<_, Option<String>>(col)? {
                    *out.values[d].entry(v).or_insert(0) += n;
                }
            }
            for (k, &s) in status.iter().enumerate() {
                out.status[s] += r.get::<_, i32>(gear.len() + 1 + k)?;
            }
        }
    }
    Ok(out)
}

/// A text facet's counts, most first, then by name.
fn by_count(values: &std::collections::HashMap<String, i32>) -> Vec<FacetCount> {
    let mut out: Vec<FacetCount> = values
        .iter()
        .map(|(value, &count)| FacetCount {
            value: value.clone(),
            count,
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    out
}

/// A range facet's counts, in the table's order; ranges nothing falls in are
/// left out.
fn in_range_order(
    ranges: &Ranges,
    values: &std::collections::HashMap<String, i32>,
) -> Vec<FacetCount> {
    ranges
        .iter()
        .filter_map(|(k, _)| {
            values.get(*k).map(|&count| FacetCount {
                value: (*k).to_string(),
                count,
            })
        })
        .collect()
}

/// Named people and how many photos each would leave, most first. Counted under
/// the whole filter: people combine with AND, so a chosen person's count is the
/// current total and anyone else's is "with everyone chosen, and them". Hidden
/// and unnamed people are left out — the rail lists who someone would look for.
///
/// Counted from `faces`, far smaller than `photos`, grouped straight off its
/// person index with the filter applied as a semijoin on the photo — the same
/// shape as the tag counts. A virtual copy has no faces of its own, so copies
/// are counted through the photo they came from, in a second query that only
/// runs when there are any. (One query joining on `id = … OR variant_of = …`
/// lost both indexes under any filter — 16 s for one genre at 10,000 photos —
/// and a `UNION` of two joins still cost ~60 ms unfiltered in sorts.)
fn count_people(
    conn: &Connection,
    f: &PhotoFilter,
    whitelist: Option<&[i32]>,
) -> Result<Vec<PersonFacet>> {
    let named: std::collections::HashMap<i32, String> = conn
        .prepare(
            "SELECT id, name FROM people WHERE name IS NOT NULL AND name != '' AND hidden = 0",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if named.is_empty() {
        return Ok(Vec::new());
    }
    let (where_sql, p) = build_where(f, None, whitelist);
    let mut counts: std::collections::HashMap<i32, i32> = std::collections::HashMap::new();
    let mut add = |sql: &str| -> Result<()> {
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(p.iter()))?;
        while let Some(r) = rows.next()? {
            *counts.entry(r.get(0)?).or_insert(0) += r.get::<_, i32>(1)?;
        }
        Ok(())
    };
    add(&format!(
        "SELECT fa.person_id, COUNT(DISTINCT fa.photo_id) FROM faces fa
          WHERE fa.person_id IS NOT NULL {} GROUP BY fa.person_id",
        restrict_to("fa.photo_id", &where_sql)
    ))?;
    let copies: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM photos WHERE variant_of IS NOT NULL)",
        [],
        |r| r.get(0),
    )?;
    if copies {
        // `CROSS JOIN` keeps SQLite's join order as written: from the copies,
        // few, to their master's faces by index — not from every face in the
        // library to a copy that is almost never there.
        add(&format!(
            "SELECT fa.person_id, COUNT(DISTINCT p.id) FROM photos p
              CROSS JOIN faces fa ON fa.photo_id = p.variant_of
              WHERE p.variant_of IS NOT NULL AND fa.person_id IS NOT NULL AND {where_sql}
              GROUP BY fa.person_id"
        ))?;
    }
    let mut out: Vec<PersonFacet> = counts
        .into_iter()
        .filter_map(|(id, count)| {
            named.get(&id).map(|name| PersonFacet {
                id,
                name: name.clone(),
                count,
            })
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

/// The rail's optional counts, only the groups asked for. `whitelist`
/// confines them to a natural-language query's candidates, as in [`facets`].
///
/// Kept apart from [`facets`] because they cost what the base counts don't:
/// every row of `photos` carries its search embedding, so a pass that reads the
/// rows rather than an index is ~20 ms per 10,000 photos, and these need one.
/// The rail asks only while their section is open, so a library nobody filters
/// by lens never pays for counting lenses.
pub fn extra_facets(
    conn: &Connection,
    f: &PhotoFilter,
    whitelist: Option<&[i32]>,
    groups: FacetGroups,
) -> Result<ExtraFacets> {
    let mut out = ExtraFacets::default();
    if groups.gear || groups.kind || groups.status {
        let counts =
            count_gear_and_status(conn, f, whitelist, groups.gear, groups.kind, groups.status)?;
        if groups.gear {
            out.cameras = by_count(&counts.values[0]);
            out.lenses = by_count(&counts.values[1]);
            out.focal_ranges = in_range_order(FOCAL_RANGES, &counts.values[2]);
            out.iso_ranges = in_range_order(ISO_RANGES, &counts.values[3]);
            out.aperture_ranges = in_range_order(APERTURE_RANGES, &counts.values[4]);
        }
        if groups.kind {
            out.file_kinds = in_range_order(FILE_KINDS, &counts.values[5]);
            out.shapes = in_range_order(SHAPES, &counts.values[6]);
            out.face_counts = in_range_order(FACE_COUNTS, &counts.values[7]);
        }
        if groups.status {
            [
                out.edited,
                out.copies,
                out.with_faces,
                out.located,
                out.unplaced,
            ] = counts.status;
        }
    }
    if groups.people {
        out.people = count_people(conn, f, whitelist)?;
    }
    if groups.palette {
        out.palette = count_palette(conn, f, whitelist)?;
    }
    Ok(out)
}

/// The bits of these named colours, in a photo's palette mask.
pub(super) fn palette_mask(names: &[String]) -> i64 {
    crate::palette::SWATCHES
        .iter()
        .enumerate()
        .filter(|(_, s)| names.iter().any(|n| n == *s))
        .fold(0, |m, (i, _)| m | (1 << i))
}

/// Photos each named colour covers, under the other filters: one pass
/// over the palettes of the photos they let through.
fn count_palette(
    conn: &Connection,
    f: &PhotoFilter,
    whitelist: Option<&[i32]>,
) -> Result<Vec<FacetCount>> {
    let (where_sql, p) = build_where(f, Some("palette"), whitelist);
    let sums: Vec<String> = (0..crate::palette::SWATCHES.len())
        .map(|i| format!("SUM((pp.swatches >> {i}) & 1)"))
        .collect();
    let sql = format!(
        "SELECT {} FROM photo_palette pp JOIN photos p ON p.id = pp.photo_id WHERE {where_sql}",
        sums.join(", ")
    );
    let counts: Vec<i64> = conn.query_row(&sql, rusqlite::params_from_iter(p.iter()), |r| {
        (0..crate::palette::SWATCHES.len())
            .map(|i| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0)))
            .collect()
    })?;
    Ok(crate::palette::SWATCHES
        .iter()
        .zip(counts)
        .filter(|(_, c)| *c > 0)
        .map(|(name, count)| FacetCount {
            value: name.to_string(),
            count: count as i32,
        })
        .collect())
}

/// Facet counts for a filter. `whitelist`, when a natural-language query is
/// active, confines every count to that ranked candidate set so the facets
/// describe the search results rather than the whole library.
pub fn facets(conn: &Connection, f: &PhotoFilter, whitelist: Option<&[i32]>) -> Result<Facets> {
    let [genres, subjects, conditions] = count_tag_facets(conn, f, whitelist)?;

    // `photo_places.photo_id` is the primary key — one place per photo — so a
    // plain count is already a count of distinct photos, as with the tags.
    let (place_where, place_params) = build_where(f, Some("place"), whitelist);
    let place_sql = format!(
        "SELECT pl.id, pl.name, COUNT(*) AS c
         FROM places pl
         JOIN photo_places pp ON pp.place_id = pl.id
         WHERE 1 = 1 {}
         GROUP BY pl.id ORDER BY c DESC, pl.name ASC",
        restrict_to("pp.photo_id", &place_where)
    );
    let mut stmt = conn.prepare(&place_sql)?;
    let places = stmt
        .query_map(rusqlite::params_from_iter(place_params), |r| {
            Ok(PlaceFacet {
                id: r.get(0)?,
                name: r.get(1)?,
                count: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let flags = count_column_facet(conn, f, whitelist, "flag", "flag")?;
    let color_labels = count_column_facet(conn, f, whitelist, "color", "color_label")?;
    let ratings = count_column_facet(conn, f, whitelist, "rating", "rating")?;

    // By folder: a column count like the flags, leaving out photos that
    // belong to no folder rather than inventing a bucket for them.
    let (folder_where, folder_params) = build_where(f, Some("folder"), whitelist);
    let mut stmt = conn.prepare(&format!(
        "SELECT p.folder_id, COUNT(*) FROM photos p
         WHERE {folder_where} AND p.folder_id IS NOT NULL GROUP BY p.folder_id"
    ))?;
    let folders = stmt
        .query_map(rusqlite::params_from_iter(folder_params), |r| {
            Ok(FacetCount {
                value: r.get::<_, i64>(0)?.to_string(),
                count: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (missing_where, missing_params) = build_where(f, Some("missing"), whitelist);
    let missing: i32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM photos p WHERE {missing_where} AND p.missing = 1"),
        rusqlite::params_from_iter(missing_params),
        |r| r.get(0),
    )?;

    // Totals reflect the full active filter (and the semantic whitelist, if any).
    let (where_sql, p) = build_where(f, None, whitelist);
    let total: i32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM photos p WHERE {where_sql}"),
        rusqlite::params_from_iter(p.clone()),
        |r| r.get(0),
    )?;
    let untagged: i32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM photos p WHERE {where_sql} AND p.tagged_at IS NULL"),
        rusqlite::params_from_iter(p),
        |r| r.get(0),
    )?;

    // The date range spans the whole library so the picker bounds stay stable.
    let (date_min, date_max): (Option<String>, Option<String>) = conn.query_row(
        "SELECT MIN(capture_date), MAX(capture_date) FROM photos",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    Ok(Facets {
        genres,
        subjects,
        conditions,
        places,
        flags,
        color_labels,
        ratings,
        folders,
        missing,
        total,
        untagged,
        date_min,
        date_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Rewriting the tag filter to an uncorrelated semijoin must not change
    /// what it counts: a genre filter selects exactly its photos, the sibling
    /// facets are counted within that selection, and the genre facet — its own
    /// constraint lifted — still sees the whole library, which is what stops an
    /// option vanishing the moment it is used.
    #[test]
    fn a_tag_filter_counts_the_intersection_and_lifts_its_own_facet() {
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

        add_manual_tag_many(&mut conn, &ids[0..2], "landscape", "genre").unwrap();
        add_manual_tag_many(&mut conn, &ids[2..4], "portrait", "genre").unwrap();
        add_manual_tag_many(&mut conn, &[ids[0], ids[2]], "beach", "subject").unwrap();
        add_manual_tag_many(&mut conn, &[ids[1], ids[3]], "mountain", "subject").unwrap();

        let f = PhotoFilter {
            genres: vec!["landscape".into()],
            ..Default::default()
        };
        let facets = facets(&conn, &f, None).unwrap();

        let count = |v: &[FacetCount], name: &str| {
            v.iter()
                .find(|c| c.value == name)
                .map(|c| c.count)
                .unwrap_or(0)
        };

        // Own constraint lifted: the whole library, both genres still offered.
        assert_eq!(count(&facets.genres, "landscape"), 2);
        assert_eq!(count(&facets.genres, "portrait"), 2);
        // Siblings counted within the landscape selection (p0 beach, p1 mountain).
        assert_eq!(count(&facets.subjects, "beach"), 1);
        assert_eq!(count(&facets.subjects, "mountain"), 1);
        // The total is the two landscape photos, not the four in the library.
        assert_eq!(facets.total, 2);
    }

    fn names(v: &[FacetCount]) -> Vec<(&str, i32)> {
        v.iter().map(|c| (c.value.as_str(), c.count)).collect()
    }

    /// The counts now come back grouped by tag id and are ordered in Rust, so
    /// the order the rail shows — most used first, ties alphabetical — is pinned
    /// here rather than left to whichever order SQLite happened to group in.
    #[test]
    fn facet_options_are_ordered_by_count_then_name() {
        let mut conn = open_memory().unwrap();
        let ids = photos(&conn, 3);
        add_manual_tag_many(&mut conn, &ids, "tree", "subject").unwrap();
        add_manual_tag_many(&mut conn, &ids[0..1], "bird", "subject").unwrap();
        add_manual_tag_many(&mut conn, &ids[1..2], "abbey", "subject").unwrap();

        let facets = facets(&conn, &PhotoFilter::default(), None).unwrap();
        assert_eq!(
            names(&facets.subjects),
            vec![("tree", 3), ("abbey", 1), ("bird", 1)]
        );
    }

    /// Flags and colour labels are facets like any other: counted
    /// under every filter but their own, so choosing Picks leaves Rejects'
    /// count showing what adding it would bring back, and the colour counts
    /// narrow to the picks.
    #[test]
    fn flags_and_colour_labels_are_counted_like_facets() {
        let conn = open_memory().unwrap();
        let ids = photos(&conn, 4);
        set_flag_many(&conn, &ids[0..2], 1).unwrap();
        set_flag(&conn, ids[2], -1).unwrap();
        set_color_label_many(&conn, &[ids[0], ids[2]], 3).unwrap();

        let f = PhotoFilter {
            flags: vec![1],
            ..Default::default()
        };
        let got = facets(&conn, &f, None).unwrap();
        let count = |v: &[FacetCount], key: &str| {
            v.iter()
                .find(|c| c.value == key)
                .map(|c| c.count)
                .unwrap_or(0)
        };
        // The flag facet ignores its own selection...
        assert_eq!(count(&got.flags, "1"), 2);
        assert_eq!(count(&got.flags, "-1"), 1);
        assert_eq!(count(&got.flags, "0"), 1);
        // ...while the colour facet is narrowed by it: of the two picks, one is
        // label 3 and one has none.
        assert_eq!(count(&got.color_labels, "3"), 1);
        assert_eq!(count(&got.color_labels, "0"), 1);
    }

    /// With several facets active at once each one is counted under a different
    /// clause — every constraint but its own. The rewrite batches facets that
    /// share a clause into one statement, so this is the case where a wrong
    /// grouping would quietly count one facet under another's filter.
    #[test]
    fn each_active_facet_lifts_only_its_own_constraint() {
        let mut conn = open_memory().unwrap();
        let ids = photos(&conn, 4);
        // p0 landscape/beach/coast, p1 landscape/mountain/alps,
        // p2 portrait/beach/coast,  p3 portrait/mountain/alps.
        add_manual_tag_many(&mut conn, &ids[0..2], "landscape", "genre").unwrap();
        add_manual_tag_many(&mut conn, &ids[2..4], "portrait", "genre").unwrap();
        add_manual_tag_many(&mut conn, &[ids[0], ids[2]], "beach", "subject").unwrap();
        add_manual_tag_many(&mut conn, &[ids[1], ids[3]], "mountain", "subject").unwrap();
        let coast = upsert_place(&conn, "Coast", None, None).unwrap();
        let alps = upsert_place(&conn, "Alps", None, None).unwrap();
        assign_place(&mut conn, &[ids[0], ids[2]], coast).unwrap();
        assign_place(&mut conn, &[ids[1], ids[3]], alps).unwrap();

        let f = PhotoFilter {
            genres: vec!["landscape".into()],
            subjects: vec!["beach".into()],
            place_ids: vec![coast],
            ..Default::default()
        };
        let facets = facets(&conn, &f, None).unwrap();

        // Genres, under subject=beach and place=Coast: p0 and p2.
        assert_eq!(
            names(&facets.genres),
            vec![("landscape", 1), ("portrait", 1)]
        );
        // Subjects, under genre=landscape and place=Coast: p0 only.
        assert_eq!(names(&facets.subjects), vec![("beach", 1)]);
        // Places, under genre=landscape and subject=beach: p0 only.
        let places: Vec<(&str, i32)> = facets
            .places
            .iter()
            .map(|p| (p.name.as_str(), p.count))
            .collect();
        assert_eq!(places, vec![("Coast", 1)]);
        // Everything at once: p0.
        assert_eq!(facets.total, 1);

        // And with nothing selected, every facet counts the whole library.
        let all = facets_for_nothing(&conn);
        assert_eq!(names(&all.genres), vec![("landscape", 2), ("portrait", 2)]);
        assert_eq!(names(&all.subjects), vec![("beach", 2), ("mountain", 2)]);
        assert_eq!(all.total, 4);
    }

    fn facets_for_nothing(conn: &Connection) -> Facets {
        facets(conn, &PhotoFilter::default(), None).unwrap()
    }

    /// The unfiltered counts read `photo_tags` without joining `photos`, which is
    /// only right because deleting a photo cascades its tag links away. If that
    /// ever stopped being true, a removed photo would linger in the rail.
    #[test]
    fn a_deleted_photo_leaves_the_facet_counts() {
        let mut conn = open_memory().unwrap();
        let ids = photos(&conn, 2);
        add_manual_tag_many(&mut conn, &ids, "landscape", "genre").unwrap();
        let place = upsert_place(&conn, "Coast", None, None).unwrap();
        assign_place(&mut conn, &ids, place).unwrap();

        conn.execute("DELETE FROM photos WHERE id = ?1", params![ids[0]])
            .unwrap();

        let all = facets_for_nothing(&conn);
        assert_eq!(names(&all.genres), vec![("landscape", 1)]);
        assert_eq!(all.places[0].count, 1);
        assert_eq!(all.total, 1);
    }

    const ALL: FacetGroups = FacetGroups {
        gear: true,
        people: true,
        status: true,
        kind: true,
        palette: true,
    };

    fn counts(list: &[FacetCount]) -> Vec<(String, i32)> {
        list.iter().map(|c| (c.value.clone(), c.count)).collect()
    }

    #[test]
    fn camera_and_lens_filter_and_count_against_the_rest() {
        let conn = gear_library();
        let f = PhotoFilter {
            cameras: vec!["Z6".into()],
            ..Default::default()
        };
        assert_eq!(ids(&conn, &f), vec![4, 3]);
        let fc = extra_facets(&conn, &f, None, ALL).unwrap();
        // The camera facet ignores its own choice: A7 (two photos and the copy)
        // stays countable…
        assert_eq!(
            counts(&fc.cameras),
            vec![("A7".into(), 3), ("Z6".into(), 2)]
        );
        // …while lenses count within the Z6 photos only, blanks left out.
        assert_eq!(counts(&fc.lenses), vec![("24-70".into(), 1)]);
    }

    #[test]
    fn range_facets_bucket_and_filter() {
        let conn = gear_library();
        let fc = extra_facets(&conn, &PhotoFilter::default(), None, ALL).unwrap();
        // In table order; the copy shares photo 1's EXIF.
        assert_eq!(
            counts(&fc.focal_ranges),
            vec![
                ("0-24".into(), 2),
                ("25-50".into(), 1),
                ("106-200".into(), 1),
                ("201-".into(), 1)
            ]
        );
        assert_eq!(
            counts(&fc.iso_ranges),
            vec![
                ("0-400".into(), 2),
                ("401-1600".into(), 1),
                ("1601-6400".into(), 1),
                ("6401-".into(), 1)
            ]
        );
        assert_eq!(
            counts(&fc.aperture_ranges),
            vec![
                ("0-2.8".into(), 2),
                ("2.9-5.6".into(), 2),
                ("5.7-".into(), 1)
            ]
        );

        let tele = PhotoFilter {
            focal_ranges: vec!["106-200".into(), "201-".into()],
            ..Default::default()
        };
        assert_eq!(ids(&conn, &tele), vec![4, 2]);
        // A key the table doesn't know narrows nothing.
        let stale = PhotoFilter {
            iso_ranges: vec!["bogus".into()],
            ..Default::default()
        };
        assert_eq!(ids(&conn, &stale).len(), 5);
    }

    #[test]
    fn status_toggles_filter_and_count() {
        let conn = gear_library();
        let fc = extra_facets(&conn, &PhotoFilter::default(), None, ALL).unwrap();
        // Edited: photo 2 (a copy starts with no edits of its own). Copies: 1.
        // Faces: 1, 2, and the copy through its master. Located: 1 and its copy,
        // through its master too. Unplaced: all but photo 1.
        assert_eq!(
            (fc.edited, fc.copies, fc.with_faces, fc.located, fc.unplaced),
            (1, 1, 3, 2, 4)
        );
        let faces_and_unplaced = PhotoFilter {
            faces_only: true,
            unplaced_only: true,
            ..Default::default()
        };
        let mut got = ids(&conn, &faces_and_unplaced);
        got.sort();
        assert_eq!(got, vec![2, 5]);
        // Each toggle's count ignores itself but honours the others: with faces
        // among the unplaced (2, 3, 4, copy) is 2 and the copy; unplaced among
        // those with faces (1, 2, copy) is the same two.
        let fc = extra_facets(&conn, &faces_and_unplaced, None, ALL).unwrap();
        assert_eq!((fc.with_faces, fc.unplaced), (2, 2));
        // Every other count honours both toggles: of photo 2 and the copy, one is edited.
        assert_eq!(fc.edited, 1);
    }

    #[test]
    fn people_combine_with_and_and_count_under_the_whole_filter() {
        let conn = gear_library();
        let ada = PhotoFilter {
            person_ids: vec![7],
            ..Default::default()
        };
        let mut got = ids(&conn, &ada);
        got.sort();
        assert_eq!(got, vec![1, 2, 5], "the copy of photo 1 is in it too");

        let both = PhotoFilter {
            person_ids: vec![7, 8],
            ..Default::default()
        };
        let mut got = ids(&conn, &both);
        got.sort();
        assert_eq!(got, vec![1, 5]);

        // Named, visible people only; under Ada, Bo's count is "Ada and Bo".
        let fc = extra_facets(&conn, &ada, None, ALL).unwrap();
        let people: Vec<(String, i32)> = fc
            .people
            .iter()
            .map(|p| (p.name.clone(), p.count))
            .collect();
        assert_eq!(people, vec![("Ada".into(), 3), ("Bo".into(), 2)]);

        // A saved view from before `person_ids` still matches.
        let old = PhotoFilter {
            person_id: Some(8),
            ..Default::default()
        };
        let mut got = ids(&conn, &old);
        got.sort();
        assert_eq!(got, vec![1, 5]);
    }

    #[test]
    fn extra_counts_come_back_only_for_the_groups_asked_for() {
        let conn = gear_library();
        let only_people = FacetGroups {
            people: true,
            ..Default::default()
        };
        let fc = extra_facets(&conn, &PhotoFilter::default(), None, only_people).unwrap();
        assert!(fc.cameras.is_empty() && fc.focal_ranges.is_empty());
        assert_eq!((fc.edited, fc.with_faces), (0, 0));
        assert_eq!(fc.people.len(), 2);
    }

    // --- File type, shape and faces -------------------------------------

    /// A raw landscape and its portrait twin turned a quarter, a JPEG square, a
    /// PNG panorama, a HEIC with no size yet; faces 0, 1, 2, 4, unknown.
    fn kinds_library() -> Connection {
        let conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, width, height, orientation, face_count) VALUES
               (1, 'a.ARW', 'a', 'h1', '2026', 6000, 4000, 1, 0),
               (2, 'b.arw', 'b', 'h2', '2026', 6000, 4000, 6, 1),
               (3, 'c.JPG', 'c', 'h3', '2026', 2000, 2050, 1, 2),
               (4, 'd.png', 'd', 'h4', '2026', 9000, 3000, 1, 4),
               (5, 'e.heic', 'e', 'h5', '2026', 0, 0, 1, NULL);",
        )
        .unwrap();
        conn
    }

    fn kind_counts(conn: &Connection, f: &PhotoFilter) -> ExtraFacets {
        let only = FacetGroups {
            kind: true,
            ..Default::default()
        };
        extra_facets(conn, f, None, only).unwrap()
    }

    fn pairs(v: &[FacetCount]) -> Vec<(String, i32)> {
        v.iter().map(|c| (c.value.clone(), c.count)).collect()
    }

    #[test]
    fn file_type_shape_and_faces_are_counted_in_their_own_order() {
        let conn = kinds_library();
        let fc = kind_counts(&conn, &PhotoFilter::default());
        let own = |v: &[(&str, i32)]| {
            v.iter()
                .map(|(k, n)| (k.to_string(), *n))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pairs(&fc.file_kinds),
            own(&[("raw", 2), ("jpeg", 1), ("heic", 1), ("png", 1)])
        );
        // Photo 2 is stored landscape and shown portrait.
        assert_eq!(
            pairs(&fc.shapes),
            own(&[
                ("portrait", 1),
                ("landscape", 1),
                ("square", 1),
                ("panorama", 1)
            ])
        );
        assert_eq!(
            pairs(&fc.face_counts),
            own(&[("0", 1), ("1", 1), ("2", 1), ("3-5", 1)])
        );
        assert!(fc.cameras.is_empty(), "only the groups asked for");
    }

    #[test]
    fn file_type_shape_and_faces_filter_and_lift_their_own_counts() {
        let conn = kinds_library();
        let raw_portraits = PhotoFilter {
            file_kinds: vec!["raw".into()],
            shapes: vec!["portrait".into()],
            ..Default::default()
        };
        assert_eq!(matching(&conn, &raw_portraits), vec![2]);
        let groups = PhotoFilter {
            face_counts: vec!["2".into(), "3-5".into(), "6-".into()],
            ..Default::default()
        };
        assert_eq!(matching(&conn, &groups), vec![3, 4]);
        // Choosing a file type leaves the others on offer.
        let fc = kind_counts(
            &conn,
            &PhotoFilter {
                file_kinds: vec!["raw".into()],
                ..Default::default()
            },
        );
        assert_eq!(fc.file_kinds.len(), 4);
        // ...and counts the shapes within it.
        assert_eq!(fc.shapes.iter().map(|c| c.count).sum::<i32>(), 2);
    }

    #[test]
    fn a_copy_counts_its_originals_faces() {
        let mut conn = kinds_library();
        let copy = create_variant(&mut conn, 4, None).unwrap();
        conn.execute(
            "UPDATE photos SET face_count = NULL WHERE id = ?1",
            params![copy],
        )
        .unwrap();
        let groups = PhotoFilter {
            face_counts: vec!["3-5".into()],
            ..Default::default()
        };
        assert_eq!(matching(&conn, &groups), vec![4, copy]);
    }
}
