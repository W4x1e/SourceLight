//! The point of this app is combining date + place + type in one query, so that
//! intersection is what these tests pin down.

use rusqlite::Connection;
use sourcelight_lib::db;
use sourcelight_lib::models::{PhotoFilter, TagResult};
use sourcelight_lib::scan::IngestedPhoto;

fn photo(path: &str, date: &str) -> IngestedPhoto {
    IngestedPhoto {
        path: path.into(),
        filename: path.into(),
        file_hash: format!("h{path}"),
        capture_date: Some(date.into()),
        camera: Some("SONY ILCE-6400".into()),
        lens: None,
        iso: Some(100),
        aperture: Some(2.8),
        shutter: Some("1/250".into()),
        focal_length: Some(35.0),
        width: 6000,
        height: 4000,
        orientation: 1,
        thumb_path: Some(format!("/thumbs/{path}.jpg")),
        preview_path: Some(format!("/thumbs/{path}_p.jpg")),
        sharpness: None,
        brightness: None,
        gps: None,
        stamp: None,
    }
}

fn tags(genre: &str, subjects: &[&str]) -> TagResult {
    TagResult {
        main_subject: subjects.first().copied().unwrap_or_default().to_string(),
        genre: genre.into(),
        subjects: subjects.iter().map(|s| s.to_string()).collect(),
        conditions: vec![],
        confidence: 0.9,
    }
}

/// Three photos that differ along each axis, so a combined filter has exactly
/// one correct answer.
fn fixture() -> Connection {
    let mut conn = db::open_memory().unwrap();

    let a = db::upsert_photo(&conn, &photo("a.arw", "2025-03-10 08:00:00")).unwrap();
    let b = db::upsert_photo(&conn, &photo("b.arw", "2025-03-11 09:00:00")).unwrap();
    let c = db::upsert_photo(&conn, &photo("c.arw", "2024-06-01 07:00:00")).unwrap();

    db::apply_ai_tags(&mut conn, a, &tags("wildlife", &["bird", "tree"])).unwrap();
    db::apply_ai_tags(&mut conn, b, &tags("landscape", &["mountain"])).unwrap();
    db::apply_ai_tags(&mut conn, c, &tags("wildlife", &["bird"])).unwrap();

    let karoo = db::upsert_place(&conn, "Karoo", None, None).unwrap();
    let coast = db::upsert_place(&conn, "Coast", None, None).unwrap();
    db::assign_place(&mut conn, &[a, b], karoo).unwrap();
    db::assign_place(&mut conn, &[c], coast).unwrap();

    conn
}

fn names(conn: &Connection, f: &PhotoFilter) -> Vec<String> {
    let mut v: Vec<String> = db::query_photos(conn, f, 100, 0)
        .unwrap()
        .into_iter()
        .map(|p| p.filename)
        .collect();
    v.sort();
    v
}

#[test]
fn no_filter_returns_everything() {
    let conn = fixture();
    assert_eq!(names(&conn, &PhotoFilter::default()).len(), 3);
}

#[test]
fn genre_filter_is_or_within_the_facet() {
    let conn = fixture();
    let f = PhotoFilter {
        genres: vec!["wildlife".into(), "landscape".into()],
        ..Default::default()
    };
    assert_eq!(names(&conn, &f), vec!["a.arw", "b.arw", "c.arw"]);
}

#[test]
fn date_place_and_genre_combine_as_an_intersection() {
    let conn = fixture();
    let karoo = db::list_places(&conn)
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Karoo")
        .unwrap();

    // Wildlife AND in the Karoo AND during March 2025 -> only "a".
    let f = PhotoFilter {
        genres: vec!["wildlife".into()],
        place_ids: vec![karoo.id],
        date_from: Some("2025-03-01".into()),
        date_to: Some("2025-03-31".into()),
        ..Default::default()
    };
    assert_eq!(names(&conn, &f), vec!["a.arw"]);
}

#[test]
fn date_range_end_is_inclusive_of_the_whole_day() {
    let conn = fixture();
    let f = PhotoFilter {
        date_from: Some("2025-03-11".into()),
        date_to: Some("2025-03-11".into()),
        ..Default::default()
    };
    assert_eq!(names(&conn, &f), vec!["b.arw"]);
}

#[test]
fn facet_counts_ignore_their_own_facet_but_respect_others() {
    let conn = fixture();
    let f = PhotoFilter {
        genres: vec!["wildlife".into()],
        date_from: Some("2025-01-01".into()),
        ..Default::default()
    };
    let facets = db::facets(&conn, &f, None).unwrap();

    // Genre counts ignore the genre filter, so landscape is still offered...
    let genre_names: Vec<&str> = facets.genres.iter().map(|g| g.value.as_str()).collect();
    assert!(genre_names.contains(&"landscape"));
    // ...but the date filter still applies, so the 2024 wildlife shot is excluded.
    let wildlife = facets
        .genres
        .iter()
        .find(|g| g.value == "wildlife")
        .unwrap();
    assert_eq!(wildlife.count, 1);

    // Total reflects every active filter.
    assert_eq!(facets.total, 1);
}

#[test]
fn manual_tags_survive_a_retag() {
    let mut conn = db::open_memory().unwrap();
    let id = db::upsert_photo(&conn, &photo("x.arw", "2025-01-01 00:00:00")).unwrap();

    db::apply_ai_tags(&mut conn, id, &tags("street", &["car"])).unwrap();
    db::add_manual_tag(&conn, id, "keeper", "subject").unwrap();

    // Re-running the tagger replaces AI tags only.
    db::apply_ai_tags(&mut conn, id, &tags("street", &["bicycle"])).unwrap();

    let p = db::get_photo(&conn, id).unwrap().unwrap();
    assert!(
        p.subjects.contains(&"keeper".to_string()),
        "manual tag lost"
    );
    assert!(p.subjects.contains(&"bicycle".to_string()));
    assert!(
        !p.subjects.contains(&"car".to_string()),
        "stale AI tag kept"
    );
}

#[test]
fn untagged_filter_tracks_tagging_state() {
    let mut conn = db::open_memory().unwrap();
    let id = db::upsert_photo(&conn, &photo("y.arw", "2025-01-01 00:00:00")).unwrap();

    let f = PhotoFilter {
        untagged_only: true,
        ..Default::default()
    };
    assert_eq!(names(&conn, &f).len(), 1);

    db::apply_ai_tags(&mut conn, id, &tags("macro", &["flower"])).unwrap();
    assert_eq!(names(&conn, &f).len(), 0);
}

#[test]
fn trips_split_on_a_time_gap() {
    let conn = fixture();
    // a and b are a day apart, c is a year earlier -> two clusters at 48h.
    let clusters = sourcelight_lib::trips::cluster(&conn, 48).unwrap();
    assert_eq!(clusters.len(), 2);
    let sizes: Vec<i32> = clusters.iter().map(|c| c.count).collect();
    assert!(sizes.contains(&2) && sizes.contains(&1));
}

// ---------------------------------------------------------------------------
// Library depth: flags, colour labels, collections.

/// Maps filename -> id for the three-photo fixture, so tests can address a
/// specific photo without threading ids out of `fixture`.
fn ids(conn: &Connection) -> std::collections::HashMap<String, i32> {
    db::query_photos(conn, &PhotoFilter::default(), 100, 0)
        .unwrap()
        .into_iter()
        .map(|p| (p.filename, p.id))
        .collect()
}

#[test]
fn flags_default_to_zero_and_filter_by_pick_or_reject() {
    let conn = fixture();
    let id = ids(&conn);

    // Everything starts unflagged.
    let all = db::query_photos(&conn, &PhotoFilter::default(), 100, 0).unwrap();
    assert!(all.iter().all(|p| p.flag == 0));

    db::set_flag(&conn, id["a.arw"], 1).unwrap(); // pick
    db::set_flag(&conn, id["b.arw"], -1).unwrap(); // reject

    let picks = PhotoFilter {
        flags: vec![1],
        ..Default::default()
    };
    assert_eq!(names(&conn, &picks), vec!["a.arw"]);

    // Rejects filtered out is the cull's most common view: keep pick + unflagged.
    let kept = PhotoFilter {
        flags: vec![0, 1],
        ..Default::default()
    };
    assert_eq!(names(&conn, &kept), vec!["a.arw", "c.arw"]);
}

#[test]
fn rating_filter_matches_exact_stars_or_within() {
    let conn = fixture();
    let id = ids(&conn);
    db::set_rating(&conn, id["a.arw"], 3).unwrap();
    db::set_rating(&conn, id["b.arw"], 5).unwrap();
    // c.arw stays unrated.

    // Three stars means the threes, not three and up: the five is left out.
    let threes = PhotoFilter {
        ratings: vec![3],
        ..Default::default()
    };
    assert_eq!(names(&conn, &threes), vec!["a.arw"]);

    let three_or_five = PhotoFilter {
        ratings: vec![3, 5],
        ..Default::default()
    };
    assert_eq!(names(&conn, &three_or_five), vec!["a.arw", "b.arw"]);

    let unrated = PhotoFilter {
        ratings: vec![0],
        ..Default::default()
    };
    assert_eq!(names(&conn, &unrated), vec!["c.arw"]);

    // A smart collection saved with the old minimum still means "and up".
    let legacy = PhotoFilter {
        min_rating: Some(3),
        ..Default::default()
    };
    assert_eq!(names(&conn, &legacy), vec!["a.arw", "b.arw"]);

    // The facet counts every rating while one is selected, like the flags.
    let facets = db::facets(&conn, &threes, None).unwrap();
    let count = |v: &str| {
        facets
            .ratings
            .iter()
            .find(|f| f.value == v)
            .map_or(0, |f| f.count)
    };
    assert_eq!((count("0"), count("3"), count("5")), (1, 1, 1));
}

#[test]
fn flag_is_clamped_to_the_tri_state() {
    let conn = fixture();
    let id = ids(&conn);
    db::set_flag(&conn, id["a.arw"], 9).unwrap();
    let p = db::get_photo(&conn, id["a.arw"]).unwrap().unwrap();
    assert_eq!(p.flag, 1, "an out-of-range flag must clamp to pick");
}

#[test]
fn colour_labels_round_trip_and_filter() {
    let conn = fixture();
    let id = ids(&conn);
    db::set_color_label(&conn, id["a.arw"], 1).unwrap(); // red
    db::set_color_label(&conn, id["b.arw"], 3).unwrap(); // green

    let reds = PhotoFilter {
        color_labels: vec![1],
        ..Default::default()
    };
    assert_eq!(names(&conn, &reds), vec!["a.arw"]);

    // OR within the facet: red or green.
    let two = PhotoFilter {
        color_labels: vec![1, 3],
        ..Default::default()
    };
    assert_eq!(names(&conn, &two), vec!["a.arw", "b.arw"]);
}

#[test]
fn collections_add_remove_and_filter_membership() {
    let mut conn = fixture();
    let id = ids(&conn);

    let cid = db::create_collection(&conn, "Best of 2025").unwrap();
    db::add_to_collection(&mut conn, cid, &[id["a.arw"], id["b.arw"]]).unwrap();
    // Adding an already-present photo is a no-op, not an error.
    db::add_to_collection(&mut conn, cid, &[id["a.arw"]]).unwrap();

    let listed = db::list_collections(&conn).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].count, 2, "duplicate add must not double-count");

    let in_coll = PhotoFilter {
        collection_id: Some(cid),
        ..Default::default()
    };
    assert_eq!(names(&conn, &in_coll), vec!["a.arw", "b.arw"]);

    db::remove_from_collection(&mut conn, cid, &[id["b.arw"]]).unwrap();
    assert_eq!(names(&conn, &in_coll), vec!["a.arw"]);
}

#[test]
fn deleting_a_collection_leaves_the_photos() {
    let mut conn = fixture();
    let id = ids(&conn);
    let cid = db::create_collection(&conn, "Temp").unwrap();
    db::add_to_collection(&mut conn, cid, &[id["a.arw"]]).unwrap();

    db::delete_collection(&conn, cid).unwrap();
    assert!(db::list_collections(&conn).unwrap().is_empty());
    // The photo itself survives — a collection is a grouping, not ownership.
    assert_eq!(
        db::query_photos(&conn, &PhotoFilter::default(), 100, 0)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn a_collection_filter_combines_with_facets() {
    let mut conn = fixture();
    let id = ids(&conn);
    let cid = db::create_collection(&conn, "Mixed").unwrap();
    db::add_to_collection(&mut conn, cid, &[id["a.arw"], id["b.arw"], id["c.arw"]]).unwrap();

    // In the collection AND wildlife -> a and c (b is landscape).
    let f = PhotoFilter {
        collection_id: Some(cid),
        genres: vec!["wildlife".into()],
        ..Default::default()
    };
    assert_eq!(names(&conn, &f), vec!["a.arw", "c.arw"]);
}

#[test]
fn smart_collections_save_and_reload_a_filter() {
    let conn = fixture();

    let f = PhotoFilter {
        genres: vec!["wildlife".into()],
        flags: vec![1],
        color_labels: vec![2],
        ..Default::default()
    };
    let cid = db::save_smart_collection(&conn, "Wildlife picks", &f).unwrap();

    let listed = db::list_smart_collections(&conn).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, cid);
    assert_eq!(listed[0].name, "Wildlife picks");
    // The stored filter round-trips exactly — a saved query and a live one are
    // the same shape.
    assert_eq!(listed[0].filter.genres, vec!["wildlife".to_string()]);
    assert_eq!(listed[0].filter.flags, vec![1]);
    assert_eq!(listed[0].filter.color_labels, vec![2]);

    // And it actually selects the right photos when applied.
    assert_eq!(names(&conn, &listed[0].filter).len(), 0); // none are flagged yet
}

#[test]
fn saving_a_smart_collection_name_twice_overwrites_it() {
    let conn = fixture();
    db::save_smart_collection(
        &conn,
        "Set",
        &PhotoFilter {
            genres: vec!["wildlife".into()],
            ..Default::default()
        },
    )
    .unwrap();
    db::save_smart_collection(
        &conn,
        "Set",
        &PhotoFilter {
            genres: vec!["landscape".into()],
            ..Default::default()
        },
    )
    .unwrap();

    let listed = db::list_smart_collections(&conn).unwrap();
    assert_eq!(
        listed.len(),
        1,
        "re-saving a name must overwrite, not duplicate"
    );
    assert_eq!(listed[0].filter.genres, vec!["landscape".to_string()]);
}

#[test]
fn deleting_a_smart_collection_removes_it() {
    let conn = fixture();
    let id = db::save_smart_collection(&conn, "Temp", &PhotoFilter::default()).unwrap();
    db::delete_smart_collection(&conn, id).unwrap();
    assert!(db::list_smart_collections(&conn).unwrap().is_empty());
}

#[test]
fn xmp_import_applies_rating_label_and_keywords() {
    use sourcelight_lib::xmp::{Sidecar, XmpMeta};
    let mut conn = fixture();
    let id = ids(&conn);
    let pid = id["a.arw"];

    db::import_xmp_meta(
        &mut conn,
        pid,
        &Sidecar {
            meta: XmpMeta {
                rating: Some(4),
                color_label: Some(2), // yellow
                keywords: vec!["sunset".into(), "beach".into()],
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();

    let p = db::get_photo(&conn, pid).unwrap().unwrap();
    assert_eq!(p.rating, 4);
    assert_eq!(p.color_label, 2);
    // Keywords land as subject tags, added to whatever the photo already had
    // (a.arw came in with "bird" and "tree").
    for kw in ["sunset", "beach", "bird", "tree"] {
        assert!(
            p.subjects.contains(&kw.to_string()),
            "missing {kw}: {:?}",
            p.subjects
        );
    }
}

#[test]
fn xmp_import_leaves_omitted_fields_untouched() {
    use sourcelight_lib::xmp::{Sidecar, XmpMeta};
    let mut conn = fixture();
    let id = ids(&conn);
    let pid = id["a.arw"];
    db::set_rating(&conn, pid, 3).unwrap();

    // A sidecar with only keywords must not reset the existing rating.
    db::import_xmp_meta(
        &mut conn,
        pid,
        &Sidecar {
            meta: XmpMeta {
                keywords: vec!["cat".into()],
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();

    let p = db::get_photo(&conn, pid).unwrap().unwrap();
    assert_eq!(
        p.rating, 3,
        "an absent rating must not clear the existing one"
    );
}
