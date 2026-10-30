//! Library insights, and the timeline's counts by month or day.

use super::*;

/// Photos in view by month, or by day within `days` (inclusive `YYYY-MM-DD`
/// bounds) — the timeline strip. The filter's own dates are left out,
/// so the whole span stays in sight while a range is chosen from it. One
/// grouped count; no photo row is read when nothing else filters.
pub fn timeline(
    conn: &Connection,
    f: &PhotoFilter,
    days: Option<(&str, &str)>,
) -> Result<Vec<TimelineBar>> {
    let undated = PhotoFilter {
        date_from: None,
        date_to: None,
        ..f.clone()
    };
    let (where_sql, mut p) = build_where(&undated, None, None);
    let (len, range) = match days {
        Some((from, to)) => {
            p.push(Value::Text(from.to_string()));
            p.push(Value::Text(format!("{to} 23:59:59")));
            (10, " AND p.capture_date >= ? AND p.capture_date <= ?")
        }
        None => (7, ""),
    };
    let sql = format!(
        "SELECT substr(p.capture_date, 1, {len}) AS period, COUNT(*) FROM photos p
          WHERE {where_sql} AND p.capture_date IS NOT NULL{range}
          GROUP BY period ORDER BY period"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(p), |r| {
        Ok(TimelineBar {
            period: r.get(0)?,
            count: r.get::<_, i64>(1)? as u32,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many of the most used lenses are looked at closer.
const INSIGHT_LENSES: usize = 4;

/// A shutter speed as EXIF presents it, in seconds, for ordering.
fn shutter_seconds(text: &str) -> f64 {
    let t = text.trim().trim_end_matches('s').trim();
    match t.split_once('/') {
        Some((n, d)) => {
            n.trim().parse::<f64>().unwrap_or(1.0)
                / d.trim().parse::<f64>().unwrap_or(1.0).max(1e-9)
        }
        None => t.parse().unwrap_or(f64::MAX),
    }
}

/// Folds `(key, keep, n)` rows into counts, most first — or, with `ordered`,
/// in the order the keys come.
fn tally(
    rows: impl IntoIterator<Item = (String, bool, u32)>,
    ordered: Option<&[&str]>,
) -> Vec<InsightCount> {
    let mut map: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();
    for (key, keep, n) in rows {
        let e = map.entry(key).or_default();
        e.0 += n;
        if keep {
            e.1 += n;
        }
    }
    let mut out: Vec<InsightCount> = map
        .into_iter()
        .map(|(key, (count, keepers))| InsightCount {
            key,
            count,
            keepers,
        })
        .collect();
    match ordered {
        Some(keys) => {
            out.retain(|c| keys.contains(&c.key.as_str()));
            out.sort_by_key(|c| keys.iter().position(|k| *k == c.key));
        }
        None => out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key))),
    }
    out
}

/// What the photos in view say about how they were made: when, with
/// what, at which settings, and how many were keepers. One grouped pass over
/// the photos — by day, so the busiest days come from it too, and by whole mm
/// as well as by range, so the lenses' focal lengths do. What each lens took
/// is [`lens_keywords`], asked for after, as it reads every keyword.
pub fn insights(conn: &Connection, f: &PhotoFilter) -> Result<Insights> {
    let (where_sql, p) = build_where(f, None, None);
    let keep = "(p.flag = 1 OR p.rating >= 3)";
    let sql = format!(
        "SELECT substr(p.capture_date, 1, 10), NULLIF(p.camera, ''), NULLIF(p.lens, ''),
                {focal}, CAST(ROUND(p.focal_length) AS INTEGER), {iso}, {aperture},
                NULLIF(p.shutter, ''), {keep}, COUNT(*)
           FROM photos p WHERE {where_sql}
          GROUP BY 1, 2, 3, 4, 5, 6, 7, 8, 9",
        focal = ranges_case(FOCAL_RANGES),
        iso = ranges_case(ISO_RANGES),
        aperture = ranges_case(APERTURE_RANGES),
    );
    struct Row {
        day: Option<String>,
        camera: Option<String>,
        lens: Option<String>,
        focal: Option<String>,
        mm: Option<i64>,
        iso: Option<String>,
        aperture: Option<String>,
        shutter: Option<String>,
        keep: bool,
        n: u32,
    }
    let rows: Vec<Row> = conn
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(Row {
                day: r.get(0)?,
                camera: r.get(1)?,
                lens: r.get(2)?,
                focal: r.get(3)?,
                mm: r.get(4)?,
                iso: r.get(5)?,
                aperture: r.get(6)?,
                shutter: r.get(7)?,
                keep: r.get::<_, i64>(8)? != 0,
                n: r.get::<_, i64>(9)? as u32,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let by = |pick: &dyn Fn(&Row) -> Option<String>| {
        rows.iter()
            .filter_map(|r| pick(r).map(|k| (k, r.keep, r.n)))
            .collect::<Vec<_>>()
    };
    let keys = |ranges: &Ranges| ranges.iter().map(|(k, _)| *k).collect::<Vec<_>>();
    let span = |len: usize| {
        let mut c = tally(
            by(&|r: &Row| r.day.as_ref().map(|d| d[..len.min(d.len())].to_string())),
            None,
        );
        c.sort_by(|a, b| a.key.cmp(&b.key));
        c
    };
    let mut shutter = tally(by(&|r: &Row| r.shutter.clone()), None);
    shutter.sort_by(|a, b| shutter_seconds(&a.key).total_cmp(&shutter_seconds(&b.key)));
    let mut busiest_days = tally(by(&|r: &Row| r.day.clone()), None);
    busiest_days.truncate(10);
    let lenses = tally(by(&|r: &Row| r.lens.clone()), None);
    let top: Vec<String> = lenses
        .iter()
        .take(INSIGHT_LENSES)
        .map(|l| l.key.clone())
        .collect();

    let lens_detail = top
        .iter()
        .map(|lens| {
            let mut focals = tally(
                by(&|r: &Row| {
                    (r.lens.as_ref() == Some(lens))
                        .then_some(r.mm)
                        .flatten()
                        .map(|m| m.to_string())
                }),
                None,
            );
            focals.sort_by_key(|c| c.key.parse::<i64>().unwrap_or(0));
            LensInsight {
                lens: lens.clone(),
                focals,
            }
        })
        .collect();

    Ok(Insights {
        total: rows.iter().map(|r| r.n).sum(),
        keepers: rows.iter().filter(|r| r.keep).map(|r| r.n).sum(),
        years: span(4),
        months: span(7),
        cameras: tally(by(&|r: &Row| r.camera.clone()), None),
        focal: tally(by(&|r: &Row| r.focal.clone()), Some(&keys(FOCAL_RANGES))),
        iso: tally(by(&|r: &Row| r.iso.clone()), Some(&keys(ISO_RANGES))),
        aperture: tally(
            by(&|r: &Row| r.aperture.clone()),
            Some(&keys(APERTURE_RANGES)),
        ),
        shutter,
        lens_detail,
        busiest_days,
        lenses,
    })
}

/// The most common subjects and genres of each of these lenses' photos in
/// view. Grouped by tag id, and named after: looking each link's tag up
/// by name cost more than the whole count.
pub fn lens_keywords(
    conn: &Connection,
    f: &PhotoFilter,
    lenses: &[String],
) -> Result<Vec<LensKeywords>> {
    if lenses.is_empty() {
        return Ok(Vec::new());
    }
    let (where_sql, mut p) = build_where(f, None, None);
    let first = p.len() + 1;
    let marks: Vec<String> = (first..first + lenses.len())
        .map(|i| format!("?{i}"))
        .collect();
    p.extend(lenses.iter().map(|l| Value::Text(l.clone())));
    let rows: Vec<(String, i64, bool, u32)> = conn
        .prepare(&format!(
            "SELECT p.lens, pt.tag_id, (p.flag = 1 OR p.rating >= 3), COUNT(*)
               FROM photos p JOIN photo_tags pt ON pt.photo_id = p.id
              WHERE {where_sql} AND p.lens IN ({})
              GROUP BY 1, 2, 3",
            marks.join(", ")
        ))?
        .query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, i64>(2)? != 0,
                r.get::<_, i64>(3)? as u32,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let names = TagNames::load(conn, None)?;
    Ok(lenses
        .iter()
        .map(|lens| {
            let of = |genre: bool, n: usize| {
                let mut c = tally(
                    rows.iter()
                        .filter(|r| &r.0 == lens)
                        .filter_map(|r| {
                            let (is_genre, name) = names.0.get(&r.1)?;
                            (*is_genre == genre).then(|| (name.clone(), r.2, r.3))
                        })
                        .collect::<Vec<_>>(),
                    None,
                );
                c.truncate(n);
                c
            };
            LensKeywords {
                lens: lens.clone(),
                subjects: of(false, 6),
                genres: of(true, 3),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    #[test]
    fn the_timeline_counts_by_month_whatever_dates_are_chosen() {
        let conn = open_memory().unwrap();
        dated(&conn, 1, "a.arw", Some("2024-05-01 10:00:00"));
        dated(&conn, 2, "b.arw", Some("2024-05-31 23:00:00"));
        dated(&conn, 3, "c.arw", Some("2024-07-02 09:00:00"));
        dated(&conn, 4, "d.arw", None);
        conn.execute("UPDATE photos SET rating = 4 WHERE id IN (1, 3)", [])
            .unwrap();
        let bars = |f: &PhotoFilter, days| {
            timeline(&conn, f, days)
                .unwrap()
                .into_iter()
                .map(|b| (b.period, b.count))
                .collect::<Vec<_>>()
        };
        let all = PhotoFilter::default();
        assert_eq!(
            bars(&all, None),
            vec![("2024-05".to_string(), 2), ("2024-07".to_string(), 1)]
        );
        // A chosen range leaves the whole span in sight; other filters apply.
        let chosen = PhotoFilter {
            date_from: Some("2024-07-01".into()),
            date_to: Some("2024-07-31".into()),
            min_rating: Some(4),
            ..Default::default()
        };
        assert_eq!(
            bars(&chosen, None),
            vec![("2024-05".to_string(), 1), ("2024-07".to_string(), 1)]
        );
        assert_eq!(
            bars(&all, Some(("2024-05-01", "2024-05-31"))),
            vec![("2024-05-01".to_string(), 1), ("2024-05-31".to_string(), 1)]
        );
    }

    // --- Library insights -----------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn shot(
        conn: &Connection,
        id: i32,
        date: &str,
        camera: &str,
        lens: &str,
        focal: f64,
        iso: i32,
        aperture: f64,
        shutter: &str,
        rating: i32,
        flag: i32,
    ) {
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date, camera,
                                 lens, focal_length, iso, aperture, shutter, rating, flag)
             VALUES (?1, ?2, ?2, ?2, 'x', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                format!("{id}.arw"),
                date,
                camera,
                lens,
                focal,
                iso,
                aperture,
                shutter,
                rating,
                flag
            ],
        )
        .unwrap();
    }

    #[test]
    fn insights_count_what_the_photos_in_view_were_made_with() {
        let conn = open_memory().unwrap();
        let z = "FE 24-70mm F2.8 GM";
        let p = "FE 85mm F1.8";
        shot(
            &conn,
            1,
            "2024-05-14 10:00:00",
            "ILCE-7M4",
            z,
            24.0,
            100,
            2.8,
            "1/250",
            4,
            0,
        );
        shot(
            &conn,
            2,
            "2024-05-14 11:00:00",
            "ILCE-7M4",
            z,
            70.0,
            800,
            4.0,
            "1/60",
            0,
            1,
        );
        shot(
            &conn,
            3,
            "2024-05-15 09:00:00",
            "ILCE-7M4",
            z,
            70.0,
            3200,
            8.0,
            "2",
            1,
            -1,
        );
        shot(
            &conn,
            4,
            "2025-01-02 12:00:00",
            "ILCE-6400",
            p,
            85.0,
            100,
            1.8,
            "1/1000",
            2,
            0,
        );
        for (id, kw) in [(1, "beach"), (2, "beach"), (3, "dog"), (4, "portrait")] {
            add_manual_tag(&conn, id, kw, "subject").unwrap();
        }

        let i = insights(&conn, &PhotoFilter::default()).unwrap();
        let pairs = |c: &[InsightCount]| {
            c.iter()
                .map(|c| (c.key.clone(), c.count, c.keepers))
                .collect::<Vec<_>>()
        };
        let s = |k: &str, n: u32, keep: u32| (k.to_string(), n, keep);
        assert_eq!(
            (i.total, i.keepers),
            (4, 2),
            "photo 1 rated 4, photo 2 picked"
        );
        assert_eq!(pairs(&i.years), vec![s("2024", 3, 2), s("2025", 1, 0)]);
        assert_eq!(
            pairs(&i.months),
            vec![s("2024-05", 3, 2), s("2025-01", 1, 0)]
        );
        assert_eq!(
            pairs(&i.cameras),
            vec![s("ILCE-7M4", 3, 2), s("ILCE-6400", 1, 0)]
        );
        assert_eq!(pairs(&i.focal), vec![s("0-24", 1, 1), s("51-105", 3, 1)]);
        assert_eq!(
            pairs(&i.iso),
            vec![s("0-400", 2, 1), s("401-1600", 1, 1), s("1601-6400", 1, 0)]
        );
        assert_eq!(
            pairs(&i.aperture),
            vec![s("0-2.8", 2, 1), s("2.9-5.6", 1, 1), s("5.7-", 1, 0)]
        );
        let speeds: Vec<&str> = i.shutter.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            speeds,
            vec!["1/1000", "1/250", "1/60", "2"],
            "fastest first"
        );
        assert_eq!(pairs(&i.busiest_days[..1]), vec![s("2024-05-14", 2, 2)]);

        let zoom = &i.lens_detail[0];
        assert_eq!(zoom.lens, z);
        assert_eq!(pairs(&zoom.focals), vec![s("24", 1, 1), s("70", 2, 1)]);
        let kw = lens_keywords(&conn, &PhotoFilter::default(), &[z.to_string()]).unwrap();
        assert_eq!(
            pairs(&kw[0].subjects),
            vec![s("beach", 2, 2), s("dog", 1, 0)]
        );

        // Over a filter: only what it lets through.
        let picks = PhotoFilter {
            flags: vec![1],
            ..Default::default()
        };
        let i = insights(&conn, &picks).unwrap();
        assert_eq!((i.total, i.cameras.len()), (1, 1));
    }
}
