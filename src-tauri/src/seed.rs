//! Builds a synthetic catalogue large enough to find out how the app behaves at
//! the size it is built for.
//!
//! The largest library this project had demonstrably run against was **59
//! photos**, while the app is meant for people culling thousands per shoot. The
//! obvious objection to testing that properly is disk: 50,000 ARWs is roughly
//! 480 GB, which is not a reasonable thing to ask of anyone.
//!
//! It is also not necessary. What is untested at scale is the **catalogue and
//! the UI at row count** — query plans, facet counts, the fusion path, the grid
//! — none of which read sensor data. `photos.thumb_path` and `preview_path` are
//! plain TEXT with no uniqueness constraint, so fifty thousand rows can share a
//! pool of fifty real derivatives and every one of those code paths behaves
//! exactly as it would against fifty thousand distinct files. The cost is a few
//! megabytes.
//!
//! Two things this deliberately cannot measure, because they genuinely need the
//! bytes: **scan throughput** and **CLIP indexing**. Those are timed on a real
//! library and extrapolated; see `docs/BENCHMARKS.md`.
//!
//! ## Why this is in the library and not in `tests/`
//!
//! Same reason as [`crate::gpu::test_support`]: integration tests link this
//! crate from outside, so `#[cfg(test)]` would hide it from them — and a
//! `src/bin/` target cannot reach into `tests/` at all. Both the benchmark
//! (`tests/scale.rs`) and the seeder binary (`src/bin/seed_library.rs`) need the
//! same generator, and a second copy of it would stop matching the first within
//! a month.
//!
//! ## Why the randomness is deterministic
//!
//! A benchmark whose input changes between runs measures the input. The
//! generator is a seeded SplitMix64, so the same `Shape` produces identical rows
//! on every machine and a timing difference is a real one.

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::models::GENRES;
use crate::tagger::builtin::{subject_names, CONDITIONS};

/// SplitMix64. Small, fast, and — the only property that matters here —
/// reproducible without adding a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`. `n` of 0 would divide by zero, so it yields 0.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next() % n as u64) as usize
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

/// What to generate. [`Shape::library`] is the realistic default; the individual
/// fields exist so a benchmark can isolate one dimension.
#[derive(Debug, Clone)]
pub struct Shape {
    pub photos: usize,
    /// Distinct camera bodies. A real mixed library is a handful.
    pub cameras: usize,
    /// Distinct named places; roughly 85% of photos get one.
    pub places: usize,
    /// Capture dates spread back this many years from 2026-01-01.
    pub years: i64,
    /// Write a 512-float CLIP embedding per photo. ~2 KB each, so 50,000 photos
    /// is ~100 MB — which is the actual point: `all_embeddings` loads every one
    /// of them into memory on each search.
    pub embeddings: bool,
    /// Write a caption, which also populates the FTS index through the triggers.
    pub captions: bool,
    /// Detect-and-group faces on a share of the library.
    pub faces: bool,
    /// Real derivative files to point `thumb_path`/`preview_path` at, cycled.
    /// Empty means synthetic paths — fine for anything that never opens them.
    pub derivatives: Vec<String>,
    pub seed: u64,
}

impl Shape {
    /// The proportions of the real library, scaled up: ~9 tag links per photo,
    /// most photos placed, a third with faces, everything indexed.
    pub fn library(photos: usize) -> Self {
        Self {
            photos,
            cameras: 8,
            places: 60,
            years: 6,
            embeddings: true,
            captions: true,
            faces: true,
            derivatives: Vec::new(),
            seed: 0x5EED,
        }
    }
}

/// What was actually written. Returned so a caller can assert on it rather than
/// trusting the shape it asked for.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub photos: usize,
    pub tag_links: usize,
    pub placed: usize,
    pub faces: usize,
    pub people: usize,
    pub variants: usize,
    pub captions: usize,
    /// Frames that continue the previous photo's burst — near-copies a few
    /// seconds apart, the thing the near-duplicate grouping is meant to find.
    pub bursts: usize,
}

/// Probability that a photo continues the previous one's burst rather than
/// starting a fresh scene. About one frame in eight, which yields plenty of
/// short bursts without making the library read as one long panning shot.
const BURST_CONTINUE_P: f32 = 0.12;

const CAMERAS: &[(&str, &str)] = &[
    ("SONY ILCE-6400", "E 18-135mm F3.5-5.6 OSS"),
    ("SONY ILCE-7M4", "FE 24-70mm F2.8 GM II"),
    ("NIKON Z 6", "NIKKOR Z 24-70mm f/4 S"),
    ("Canon EOS R6", "RF24-105mm F4 L IS USM"),
    ("FUJIFILM X-T3", "XF16-55mmF2.8 R LM WR"),
    ("OLYMPUS E-M10 Mark III", "M.12-40mm F2.8"),
    ("Panasonic DC-G9", "LEICA DG 12-60mm F2.8-4.0"),
    ("SONY ILCE-6400", "E 70-350mm F4.5-6.3 G OSS"),
];

const APERTURES: &[f64] = &[1.8, 2.0, 2.8, 4.0, 5.6, 8.0, 11.0];
const SHUTTERS: &[&str] = &[
    "1/2000", "1/1000", "1/500", "1/250", "1/125", "1/60", "1/15",
];
const FOCALS: &[f64] = &[16.0, 24.0, 35.0, 50.0, 85.0, 135.0, 200.0, 350.0];
const ISOS: &[i64] = &[100, 200, 400, 800, 1600, 3200, 6400];

/// CLIP embeddings in a real library are not uniform noise — photos cluster by
/// content, which is what makes near-duplicate grouping and relevance ranking
/// cost anything. Generating around a set of centroids reproduces that; uniform
/// random vectors would make every similarity about zero and flatter the search
/// path.
const CENTROIDS: usize = 24;
const EMBED_DIM: usize = 512;

/// Fills `conn` with a synthetic library. The connection must already be
/// migrated ([`crate::db::open`] or [`crate::db::open_memory`]).
///
/// Everything runs inside one transaction. Fifty thousand photos is roughly half
/// a million inserts across seven tables, and committing per row turns a
/// twenty-second job into an overnight one.
pub fn seed(conn: &mut Connection, shape: &Shape) -> Result<Stats> {
    let mut rng = Rng(shape.seed);
    let mut stats = Stats::default();

    // Written straight to `photos` rather than through `upsert_photo`, because
    // the interesting columns at scale — rating, flag, colour label, embedding,
    // caption, sharpness, face aggregates — are exactly the ones the scanner's
    // ingest path does not set.
    let tx = conn.transaction()?;

    // --- vocabularies -----------------------------------------------------
    let mut tag_ids: Vec<(i64, &str)> = Vec::new();
    {
        let mut ins = tx.prepare("INSERT OR IGNORE INTO tags (name, category) VALUES (?1, ?2)")?;
        let mut find = tx.prepare("SELECT id FROM tags WHERE name = ?1 AND category = ?2")?;
        for (names, cat) in [
            (GENRES, "genre"),
            (subject_names(), "subject"),
            (CONDITIONS, "condition"),
        ] {
            for name in names {
                ins.execute(params![name, cat])?;
                tag_ids.push((find.query_row(params![name, cat], |r| r.get(0))?, cat));
            }
        }
    }
    let of_category = |cat: &str| -> Vec<i64> {
        tag_ids
            .iter()
            .filter(|(_, c)| *c == cat)
            .map(|(i, _)| *i)
            .collect()
    };
    let genre_tags = of_category("genre");
    let subject_tags = of_category("subject");
    let condition_tags = of_category("condition");

    let mut place_ids: Vec<i64> = Vec::with_capacity(shape.places);
    {
        let mut ins =
            tx.prepare("INSERT OR IGNORE INTO places (name, lat, lon) VALUES (?1, ?2, ?3)")?;
        let mut find = tx.prepare("SELECT id FROM places WHERE name = ?1")?;
        for i in 0..shape.places {
            let name = format!(
                "{} {}",
                PLACE_STEMS[i % PLACE_STEMS.len()],
                i / PLACE_STEMS.len()
            );
            let lat = 50.0 + i as f64 * 0.01;
            let lon = -3.0 + i as f64 * 0.02;
            ins.execute(params![name, lat, lon])?;
            place_ids.push(find.query_row(params![name], |r| r.get(0))?);
        }
    }

    // Content clusters, so similar photos really are similar.
    let centroids: Vec<Vec<f32>> = (0..CENTROIDS)
        .map(|_| {
            let v: Vec<f32> = (0..EMBED_DIM).map(|_| rng.unit() - 0.5).collect();
            normalise(v)
        })
        .collect();

    // --- photos -----------------------------------------------------------
    let mut ins_photo = tx.prepare(
        "INSERT INTO photos (
             path, filename, file_hash, capture_date, camera, lens, iso, aperture,
             shutter, focal_length, width, height, orientation, thumb_path, preview_path,
             rating, tagged_at, indexed_at, flag, color_label, caption,
             sharpness, faces_at, face_count, face_sharpness, eyes_open, variant
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,0)",
    )?;
    // In their own table, as the app keeps them.
    let mut ins_embedding =
        tx.prepare("INSERT INTO photo_embeddings (photo_id, embedding) VALUES (?1, ?2)")?;
    let mut ins_tag = tx.prepare(
        "INSERT OR IGNORE INTO photo_tags (photo_id, tag_id, confidence, source)
         VALUES (?1, ?2, ?3, 'ai')",
    )?;
    let mut ins_place =
        tx.prepare("INSERT OR IGNORE INTO photo_places (photo_id, place_id) VALUES (?1, ?2)")?;
    let mut ins_face = tx.prepare(
        "INSERT INTO faces (photo_id, x, y, w, h, sharpness, eye_open, score, person_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    )?;
    let mut ins_face_embedding =
        tx.prepare("INSERT INTO face_embeddings (face_id, embedding) VALUES (?1, ?2)")?;

    let now = "2026-01-01 12:00:00";
    let people_count = (shape.photos / 400).clamp(4, 200);

    // The previous photo's capture time and raw embedding, so a burst frame can
    // be built as a near-copy of the frame before it — same scene, a couple of
    // seconds later. Without this every seeded photo is scattered across six
    // years and no two ever share a burst window, which would make the
    // near-duplicate grouping test prove speed but never correctness.
    use chrono::{Duration, NaiveDate, NaiveDateTime};
    let base_dt: NaiveDateTime = NaiveDate::from_ymd_opt(2026, 1, 1)
        .expect("valid date")
        .and_hms_opt(0, 0, 0)
        .expect("valid time");
    let mut prev: Option<(NaiveDateTime, Vec<f32>)> = None;

    for i in 0..shape.photos {
        // A burst frame continues the previous photo; anything else is a fresh
        // scene at a random point in the library's span. Only possible with
        // embeddings on, since a burst is defined by looking like its neighbour.
        let continues = shape.embeddings && prev.is_some() && rng.chance(BURST_CONTINUE_P);

        let (capture_dt, raw_emb): (NaiveDateTime, Vec<f32>) = if continues {
            let (pt, pe) = prev.as_ref().unwrap();
            let dt = *pt + Duration::seconds(1 + rng.below(3) as i64);
            // A hair off the previous frame — comfortably inside the 0.92
            // duplicate threshold, so the burst groups.
            let e: Vec<f32> = pe.iter().map(|x| x + (rng.unit() - 0.5) * 0.03).collect();
            stats.bursts += 1;
            (dt, normalise(e))
        } else {
            // Dates walk backwards from 2026 in irregular steps, so a year holds
            // more than a month and the date facet has something to bound.
            let day = rng.below((shape.years * 365) as usize);
            let dt =
                base_dt - Duration::days(day as i64) + Duration::seconds(rng.below(86_400) as i64);
            let c = &centroids[rng.below(CENTROIDS)];
            // Jitter around the centroid: close enough to cluster, far enough
            // that ranking has to actually discriminate.
            let e: Vec<f32> = c.iter().map(|x| x + (rng.unit() - 0.5) * 0.6).collect();
            (dt, normalise(e))
        };
        let capture = capture_dt.format("%Y-%m-%d %H:%M:%S").to_string();

        let (camera, lens) = CAMERAS[rng.below(shape.cameras.min(CAMERAS.len()))];
        let (thumb, preview) = derivative_paths(shape, i);

        let embedding: Option<Vec<u8>> = shape
            .embeddings
            .then(|| crate::search::encode_embedding(&raw_emb));
        prev = Some((capture_dt, raw_emb));

        let caption = (shape.captions && rng.chance(0.8)).then(|| {
            format!(
                "A {} photograph showing {} and {} in {} light near {}.",
                GENRES[rng.below(GENRES.len())],
                subject_names()[rng.below(subject_names().len())],
                subject_names()[rng.below(subject_names().len())],
                CONDITIONS[rng.below(CONDITIONS.len())],
                PLACE_STEMS[rng.below(PLACE_STEMS.len())],
            )
        });
        if caption.is_some() {
            stats.captions += 1;
        }

        // A third of a real library has people in it; those are the photos the
        // cull actually spends its time on.
        let face_count = if shape.faces && rng.chance(0.35) {
            1 + rng.below(3) as i64
        } else {
            0
        };

        ins_photo.execute(params![
            format!("D:/Photos/{:04}/DSC{:06}.ARW", 2020 + (i % 6), i),
            format!("DSC{i:06}.ARW"),
            format!("{:016x}", rng.next()),
            capture,
            camera,
            lens,
            ISOS[rng.below(ISOS.len())],
            APERTURES[rng.below(APERTURES.len())],
            SHUTTERS[rng.below(SHUTTERS.len())],
            FOCALS[rng.below(FOCALS.len())],
            6000,
            4000,
            1,
            thumb,
            preview,
            rating_for(&mut rng),
            now,
            now,
            flag_for(&mut rng),
            color_for(&mut rng),
            caption,
            20.0 + rng.unit() * 180.0,
            shape.faces.then_some(now),
            shape.faces.then_some(face_count),
            (face_count > 0).then(|| f64::from(rng.unit()) * 200.0),
            (face_count > 0).then(|| f64::from(rng.unit())),
        ])?;
        let photo_id = tx.last_insert_rowid();
        stats.photos += 1;
        if let Some(embedding) = &embedding {
            ins_embedding.execute(params![photo_id, embedding])?;
        }

        // ~9 tag links per photo: one genre, five to eight subjects, one or two
        // conditions — the fan-out measured on the real library.
        ins_tag.execute(params![
            photo_id,
            genre_tags[rng.below(genre_tags.len())],
            0.7 + rng.unit() * 0.3
        ])?;
        stats.tag_links += 1;
        for _ in 0..(5 + rng.below(4)) {
            stats.tag_links += ins_tag.execute(params![
                photo_id,
                subject_tags[rng.below(subject_tags.len())],
                0.5 + rng.unit() * 0.5
            ])?;
        }
        for _ in 0..(1 + rng.below(2)) {
            stats.tag_links += ins_tag.execute(params![
                photo_id,
                condition_tags[rng.below(condition_tags.len())],
                0.5 + rng.unit() * 0.5
            ])?;
        }

        if !place_ids.is_empty() && rng.chance(0.85) {
            ins_place.execute(params![photo_id, place_ids[rng.below(place_ids.len())]])?;
            stats.placed += 1;
        }

        for _ in 0..face_count {
            let person = 1 + rng.below(people_count) as i64;
            let fe: Vec<f32> = {
                let c = &centroids[person as usize % CENTROIDS];
                normalise(c.iter().map(|x| x + (rng.unit() - 0.5) * 0.3).collect())
            };
            ins_face.execute(params![
                photo_id,
                f64::from(rng.unit()) * 0.6,
                f64::from(rng.unit()) * 0.6,
                0.1 + f64::from(rng.unit()) * 0.2,
                0.1 + f64::from(rng.unit()) * 0.2,
                f64::from(rng.unit()) * 200.0,
                f64::from(rng.unit()),
                0.6 + f64::from(rng.unit()) * 0.4,
                person,
            ])?;
            ins_face_embedding.execute(params![
                tx.last_insert_rowid(),
                crate::search::encode_embedding(&fe)
            ])?;
            stats.faces += 1;
        }
    }

    drop(ins_photo);
    drop(ins_embedding);
    drop(ins_tag);
    drop(ins_place);
    drop(ins_face);
    drop(ins_face_embedding);

    // `people` rows to match the person ids the faces were given, so the People
    // workspace and the person facet have something to read without a regroup.
    {
        let mut ins = tx.prepare(
            "INSERT INTO people (id, name, cover_face_id, size)
             SELECT ?1, ?2, MIN(id), COUNT(*) FROM faces WHERE person_id = ?1",
        )?;
        for p in 1..=people_count as i64 {
            // Half named, half not — the People workspace has to render both.
            let name = (p % 2 == 0).then(|| format!("Person {p}"));
            stats.people += ins.execute(params![p, name])?;
        }
        // The aggregate above still writes a row when a person drew no faces.
        stats.people -= tx.execute("DELETE FROM people WHERE size = 0", [])?;
    }

    // A few virtual copies, because they are the one thing that makes the grid's
    // ORDER BY non-trivial (`COALESCE(variant_of, id)`), and a plan that works
    // without them can still fall over with them.
    {
        let masters: Vec<i64> = tx
            .prepare("SELECT id FROM photos WHERE variant = 0 ORDER BY id LIMIT ?1")?
            .query_map(params![(shape.photos / 50).max(1) as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut ins = tx.prepare(
            "INSERT INTO photos (
                 path, filename, file_hash, capture_date, camera, lens, iso, aperture,
                 shutter, focal_length, width, height, orientation, thumb_path,
                 preview_path, rating, indexed_at, variant, variant_of, variant_name
             )
             SELECT path, filename, file_hash, capture_date, camera, lens, iso, aperture,
                    shutter, focal_length, width, height, orientation, thumb_path,
                    preview_path, 0, indexed_at, 1, id, 'Mono'
             FROM photos WHERE id = ?1",
        )?;
        for m in masters {
            stats.variants += ins.execute(params![m])?;
        }
    }

    tx.commit()?;

    // The planner chooses between the filter indexes on row-count statistics.
    // Refreshed the same way the app refreshes them after a scan or tagging run,
    // not with a full `ANALYZE` — this used to run one, and the app never did, so
    // every benchmark measured plans a real catalogue could not get.
    crate::db::refresh_planner_stats(conn)?;
    Ok(stats)
}

fn normalise(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Ratings in a real library are not uniform: most frames are unrated, and the
/// count falls off sharply above three stars. A uniform spread would make the
/// `min_rating` filter select about a fifth every time and hide how it behaves
/// when it selects a fiftieth.
fn rating_for(rng: &mut Rng) -> i64 {
    match rng.unit() {
        x if x < 0.55 => 0,
        x if x < 0.72 => 1,
        x if x < 0.85 => 2,
        x if x < 0.94 => 3,
        x if x < 0.99 => 4,
        _ => 5,
    }
}

fn flag_for(rng: &mut Rng) -> i64 {
    match rng.unit() {
        x if x < 0.70 => 0,
        x if x < 0.88 => 1,
        _ => -1,
    }
}

fn color_for(rng: &mut Rng) -> i64 {
    if rng.unit() < 0.80 {
        0
    } else {
        1 + rng.below(5) as i64
    }
}

fn derivative_paths(shape: &Shape, i: usize) -> (String, String) {
    if shape.derivatives.is_empty() {
        return (
            format!("D:/Cache/thumbs/{:06}.jpg", i % 50),
            format!("D:/Cache/previews/{:06}.jpg", i % 50),
        );
    }
    let p = &shape.derivatives[i % shape.derivatives.len()];
    (p.clone(), p.clone())
}

const PLACE_STEMS: &[&str] = &[
    "Karoo",
    "Cape Point",
    "Kirstenbosch",
    "Table Mountain",
    "Hout Bay",
    "Simons Town",
    "Stellenbosch",
    "Bettys Bay",
    "Cederberg",
    "West Coast",
    "Kruger",
    "Drakensberg",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn seeds_a_library_with_the_shape_it_promised() {
        let mut conn = db::open_memory().unwrap();
        let shape = Shape {
            // A megabyte of blobs proves nothing a unit test needs.
            embeddings: false,
            ..Shape::library(500)
        };
        let stats = seed(&mut conn, &shape).unwrap();

        assert_eq!(stats.photos, 500);
        // One genre, five to eight subjects, one or two conditions — averaged,
        // that is the ~9 links per photo the real library shows. `INSERT OR
        // IGNORE` drops the occasional repeated subject, so this is a range.
        let per_photo = stats.tag_links as f64 / 500.0;
        assert!(
            (7.0..10.0).contains(&per_photo),
            "expected ~9 tag links per photo, got {per_photo:.2}"
        );
        assert!(stats.placed > 380 && stats.placed <= 500);
        assert!(stats.people > 0);
        assert!(stats.faces > 0);
        assert_eq!(stats.variants, 10);
    }

    #[test]
    fn is_reproducible() {
        let shape = Shape {
            embeddings: false,
            captions: false,
            faces: false,
            ..Shape::library(100)
        };
        let digest = |shape: &Shape| {
            let mut conn = db::open_memory().unwrap();
            seed(&mut conn, shape).unwrap();
            conn.query_row(
                "SELECT group_concat(capture_date || rating || flag) FROM photos",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };
        assert_eq!(digest(&shape), digest(&shape));
    }

    #[test]
    fn the_filters_actually_select_something() {
        // A seeder that produced rows no filter matches would benchmark nothing.
        let mut conn = db::open_memory().unwrap();
        let shape = Shape {
            embeddings: false,
            ..Shape::library(400)
        };
        seed(&mut conn, &shape).unwrap();

        let f = crate::models::PhotoFilter {
            min_rating: Some(4),
            genres: vec!["landscape".into()],
            ..Default::default()
        };
        let facets = db::facets(&conn, &f, None).unwrap();
        assert!(facets.total > 0, "no photo matched a plausible filter");
        assert!(!facets.subjects.is_empty());
        // Sibling-aware: the genre facet is counted without its own constraint,
        // so it must still list every genre rather than collapsing to the one.
        assert!(facets.genres.len() > 1);
    }
}
