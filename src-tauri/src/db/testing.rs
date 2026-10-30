//! Helpers for the tests of more than one part of the catalogue.

use super::*;

pub(super) fn photo_at(conn: &Connection, id: i32, path: &str) {
    conn.execute(
        "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (?1, ?2, 'x.arw', ?3, '2026-01-01')",
        params![id, path, format!("h{id}")],
    )
    .unwrap();
}

/// `n` bare photos, for the facet tests.
pub(super) fn photos(conn: &Connection, n: usize) -> Vec<i32> {
    (0..n)
        .map(|i| {
            conn.execute(
                "INSERT INTO photos (path, filename, file_hash, indexed_at)
                     VALUES (?1, 'a.arw', 'h', '2026-01-01')",
                params![format!("/f{i}.arw")],
            )
            .unwrap();
            conn.last_insert_rowid() as i32
        })
        .collect()
}

/// A face at `(x, y)`, 100px square, face-on or in profile.
pub(super) fn face_at(x: f32, y: f32, profile: bool) -> crate::faces::Face {
    let nose_x = if profile { x + 95.0 } else { x + 50.0 };
    crate::faces::Face {
        x,
        y,
        w: 100.0,
        h: 100.0,
        score: 0.95,
        landmarks: [
            [x + 35.0, y + 40.0],
            [x + 65.0, y + 40.0],
            [nose_x, y + 55.0],
            [x + 38.0, y + 75.0],
            [x + 62.0, y + 75.0],
        ],
        sharpness: 100.0,
        eye_open: Some(1.0),
        eyes_read: true,
    }
}

pub(super) fn unit(v: &[f32]) -> Vec<f32> {
    let mut v = v.to_vec();
    crate::people::l2_normalize(&mut v);
    v
}

pub(super) fn add_photos(conn: &Connection, n: i32) {
    for id in 1..=n {
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (?1, ?2, ?2, ?2, '2026-01-01')",
            params![id, format!("p{id}.arw")],
        )
        .unwrap();
    }
}

pub(super) fn person_of_photo(conn: &Connection, photo: i32) -> Option<String> {
    conn.query_row(
        "SELECT pe.name FROM faces f JOIN people pe ON pe.id = f.person_id WHERE f.photo_id = ?1",
        params![photo],
        |r| r.get(0),
    )
    .ok()
    .flatten()
}

/// Two photos of Ada (1, 2) and one of Bo (3), grouped; returns their ids.
pub(super) fn ada_and_bo(conn: &mut Connection, photos: i32) -> (i32, i32) {
    add_photos(conn, photos);
    let ada = unit(&[1.0, 0.0, 0.0]);
    let bo = unit(&[0.0, 1.0, 0.0]);
    for (photo, e) in [(1, &ada), (2, &ada), (3, &bo)] {
        store_faces(
            conn,
            photo,
            &[face_at(100.0, 100.0, false)],
            std::slice::from_ref(e),
            (1600, 1067),
        )
        .unwrap();
    }
    regroup_people(conn).unwrap();
    let of = |photo: i32| -> i32 {
        conn.query_row(
            "SELECT person_id FROM faces WHERE photo_id = ?1 AND manual = 0",
            params![photo],
            |r| r.get(0),
        )
        .unwrap()
    };
    let (a, b) = (of(1), of(3));
    set_person_name(conn, a, "Ada").unwrap();
    (a, b)
}

/// A face too small to group: detected, but left out of every person.
pub(super) fn loose_face(conn: &mut Connection, photo: i32) -> i32 {
    let tiny = crate::faces::Face {
        w: 20.0,
        h: 20.0,
        ..face_at(300.0, 300.0, false)
    };
    store_faces(
        conn,
        photo,
        &[tiny],
        &[unit(&[0.0, 0.0, 1.0])],
        (1600, 1067),
    )
    .unwrap();
    regroup_people(conn).unwrap();
    conn.query_row(
        "SELECT id FROM faces WHERE photo_id = ?1 AND manual = 0",
        params![photo],
        |r| r.get(0),
    )
    .unwrap()
}

pub(super) fn face_on(conn: &Connection, photo: i32) -> i32 {
    conn.query_row(
        "SELECT id FROM faces WHERE photo_id = ?1 AND manual = 0",
        params![photo],
        |r| r.get(0),
    )
    .unwrap()
}

/// Ada's current person id, whatever the last regroup made it.
pub(super) fn ada(conn: &Connection) -> i32 {
    conn.query_row("SELECT id FROM people WHERE name = 'Ada'", [], |r| r.get(0))
        .unwrap()
}

/// The person a photo's detected face is in.
pub(super) fn person_id_of(conn: &Connection, photo: i32) -> Option<i32> {
    conn.query_row(
        "SELECT person_id FROM faces WHERE photo_id = ?1 AND manual = 0",
        params![photo],
        |r| r.get(0),
    )
    .unwrap()
}

pub(super) fn person_size(conn: &Connection, id: i32) -> i64 {
    conn.query_row("SELECT size FROM people WHERE id = ?1", params![id], |r| {
        r.get(0)
    })
    .unwrap()
}

/// A small library for the camera and status filters: two cameras and lenses, focal
/// lengths, ISOs and apertures across the ranges, an edit, a virtual copy, a
/// GPS fix, a named place, and faces of Ada and Bo (and someone hidden).
pub(super) fn gear_library() -> Connection {
    let mut conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date, camera, lens,
                                 focal_length, iso, aperture, rating, sharpness, face_count, lat, lon)
             VALUES
              (1, 'a.arw', 'a.arw', 'h1', '2026-01-05', '2025-01-01 10:00:00', 'A7', '24-70', 24, 100, 2.8, 3, 0.5, 2, 51.5, -0.1),
              (2, 'b.arw', 'B.arw', 'h2', '2026-01-04', '2025-01-02 10:00:00', 'A7', '70-200', 135, 3200, 4.0, 5, 0.9, 1, NULL, NULL),
              (3, 'c.arw', 'c.arw', 'h3', '2026-01-03', '2025-01-03 10:00:00', 'Z6', '24-70', 50, 12800, 8.0, 0, 0.1, 0, NULL, NULL),
              (4, 'd.arw', 'd.arw', 'h4', '2026-01-06', '2025-01-04 10:00:00', 'Z6', NULL, 400, 800, 5.6, 1, NULL, NULL, NULL, NULL);
             UPDATE photos SET develop_settings = '{}' WHERE id = 2;
             INSERT INTO places (id, name) VALUES (1, 'Karoo');
             INSERT INTO photo_places (photo_id, place_id) VALUES (1, 1);
             INSERT INTO people (id, name, hidden) VALUES (7, 'Ada', 0), (8, 'Bo', 0), (9, 'Cy', 1), (10, NULL, 0);
             INSERT INTO faces (photo_id, x, y, w, h, person_id) VALUES
               (1, 0, 0, 1, 1, 7), (1, 2, 2, 1, 1, 8),
               (2, 0, 0, 1, 1, 7),
               (1, 4, 4, 1, 1, 9), (2, 4, 4, 1, 1, 10);",
    )
    .unwrap();
    // A virtual copy of photo 1: its own id, its master's faces and place-less.
    create_variant(&mut conn, 1, Some("Mono")).unwrap();
    conn
}

pub(super) fn ids(conn: &Connection, f: &PhotoFilter) -> Vec<i32> {
    query_photos(conn, f, 100, 0)
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect()
}

pub(super) fn subjects_filter(names: &[&str]) -> PhotoFilter {
    PhotoFilter {
        subjects: names.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

pub(super) fn matching(conn: &Connection, f: &PhotoFilter) -> Vec<i32> {
    let mut ids: Vec<i32> = query_photos(conn, f, 100, 0)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    ids.sort();
    ids
}

pub(super) fn subject_count(conn: &Connection, f: &PhotoFilter, name: &str) -> i32 {
    facets(conn, f, None)
        .unwrap()
        .subjects
        .iter()
        .find(|c| c.value == name)
        .map_or(0, |c| c.count)
}

pub(super) fn add_photos_from(conn: &Connection, id: i32) {
    conn.execute(
        "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (?1, ?2, ?2, ?2, '2026-01-01')",
        params![id, format!("p{id}.arw")],
    )
    .unwrap();
}

/// Photo 1 a beach and a dog, 2 a beach, 3 a dog, 4 neither; dog filed
/// under animals.
pub(super) fn beach_and_dog(conn: &mut Connection) {
    add_photos(conn, 4);
    add_manual_tag_many(conn, &[1, 2], "beach", "subject").unwrap();
    add_manual_tag_many(conn, &[1, 3], "animals > dog", "subject").unwrap();
    add_manual_tag_many(conn, &[4], "car", "subject").unwrap();
}

pub(super) fn tagger_says(conn: &mut Connection, photo: i32, subjects: &[&str]) {
    let result = TagResult {
        main_subject: String::new(),
        genre: "street".into(),
        subjects: subjects.iter().map(|s| s.to_string()).collect(),
        conditions: vec![],
        confidence: 0.4,
    };
    apply_ai_tags(conn, photo, &result).unwrap();
}

pub(super) fn photo_of(conn: &Connection, id: i32) -> Photo {
    get_photo(conn, id).unwrap().unwrap()
}

pub(super) fn sorted_subjects(conn: &Connection, id: i32) -> Vec<String> {
    let mut s = photo_of(conn, id).subjects;
    s.sort();
    s
}

pub(super) fn dated(conn: &Connection, id: i32, path: &str, date: Option<&str>) {
    conn.execute(
        "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date)
             VALUES (?1, ?2, ?2, ?2, '2026-01-01', ?3)",
        params![id, path, date],
    )
    .unwrap();
}

pub(super) fn in_order(conn: &Connection, collection: i32) -> Vec<i32> {
    let f = PhotoFilter {
        collection_id: Some(collection),
        sort: PhotoSort::Custom,
        ..Default::default()
    };
    query_photos(conn, &f, 1000, 0)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect()
}
