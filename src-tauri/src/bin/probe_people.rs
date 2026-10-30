//! People-grouping probe: the shipped face pipeline, run outside the app so it
//! can be measured against labelled identities (`scripts/model-eval/faces_eval.py`).
//!
//! ```sh
//! cargo run --release --features ai,dev-tools --bin probe_people -- \
//!     dump <models/faces dir> <photos.json> <out dir>
//! cargo run --release --features ai,dev-tools --bin probe_people -- \
//!     cluster <embeddings.json> [core merge member_floor]
//! ```
//!
//! `dump` runs detection and alignment exactly as `analyze_faces` does. Per
//! face, it writes the aligned 112×112 crop the embedder sees
//! (`aligned/<photo>_<n>.png`), a wider crop for a person to label
//! (`context/<photo>_<n>.jpg`) and a line of `faces.jsonl` with the box,
//! score, landmarks, the `is_groupable` verdict and the FaceX embedding.
//! Aligned crops are lossless PNG, so any other embedder scored on them sees
//! exactly the pixels FaceX did.
//!
//! `cluster` runs `people::cluster` over `{"embeddings": [[f32]]}` and prints
//! the labels as JSON, so the thresholds can be tried on any embedder's
//! vectors without re-implementing the algorithm.
//!
//! `library` is the end-to-end check: it opens a *copy* of a catalogue (so
//! its migrations run), re-analyses every photo through `db::store_faces`
//! exactly as `analyze_faces` does, runs `db::regroup_people`, and prints each
//! face's photo, box and person as JSON lines.
//!
//! ```sh
//! cargo run --release --features ai,dev-tools --bin probe_people -- //!     library <models/faces dir> <copy of library.db>
//! ```

use image::GenericImageView;
use serde_json::{json, Value};
use sourcelight_lib::faces::{align_112, FaceEngine, ALIGN_EDGE};
use sourcelight_lib::people::{cluster, is_groupable, Thresholds};
use std::io::Write;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dump") if args.len() == 4 => dump(&args[1], &args[2], &args[3]),
        Some("library") if args.len() == 3 => library(&args[1], &args[2]),
        Some("cluster") if args.len() == 2 || args.len() == 5 => {
            let mut t = Thresholds::default();
            if args.len() == 5 {
                t.core = args[2].parse()?;
                t.merge = args[3].parse()?;
                t.member_floor = args[4].parse()?;
            }
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&args[1])?)?;
            let embs: Vec<Vec<f32>> = serde_json::from_value(v["embeddings"].clone())?;
            println!("{}", json!(cluster(&embs, t)));
            Ok(())
        }
        _ => {
            eprintln!(
                "usage: probe_people dump <faces dir> <photos.json> <out> | cluster <json> [core merge floor]                  | library <faces dir> <library.db copy>"
            );
            std::process::exit(2);
        }
    }
}

fn dump(models: &str, photos: &str, out: &str) -> anyhow::Result<()> {
    let engine = FaceEngine::load(&PathBuf::from(models))?;
    anyhow::ensure!(
        engine.can_embed(),
        "the face embedder is missing from {models}"
    );
    let out = PathBuf::from(out);
    std::fs::create_dir_all(out.join("aligned"))?;
    std::fs::create_dir_all(out.join("context"))?;
    std::fs::create_dir_all(out.join("portrait"))?;
    let photos: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(photos)?)?;
    let mut lines = std::fs::File::create(out.join("faces.jsonl"))?;
    let mut total = 0;
    for p in &photos {
        let id = p["id"].as_i64().unwrap_or_default();
        let Some(path) = p["preview"].as_str().or(p["thumb"].as_str()) else {
            continue;
        };
        let bytes = std::fs::read(path)?;
        let rgb = image::load_from_memory(&bytes)?.to_rgb8();
        let faces = engine.detect(&bytes)?;
        for (n, f) in faces.iter().enumerate() {
            let emb = engine.embed(&bytes, &f.landmarks)?;

            // The embedder's input, back to pixels: (px - 127.5) / 127.5 inverted.
            let planar = align_112(&rgb, &f.landmarks);
            let e = ALIGN_EDGE;
            let plane = (e * e) as usize;
            let aligned = image::RgbImage::from_fn(e, e, |x, y| {
                let i = (y * e + x) as usize;
                let px = |c: usize| {
                    (planar[c * plane + i] * 127.5 + 127.5)
                        .round()
                        .clamp(0.0, 255.0) as u8
                };
                image::Rgb([px(0), px(1), px(2)])
            });
            aligned.save(out.join("aligned").join(format!("{id}_{n}.png")))?;

            // The People page's portrait, exactly as the app cuts it.
            sourcelight_lib::faces::portrait(&rgb, [f.x, f.y, f.w, f.h])
                .save(out.join("portrait").join(format!("{id}_{n}.jpg")))?;

            // A looser crop for labelling by eye.
            let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
            let half = f.w.max(f.h) * 1.1;
            let x0 = (cx - half).max(0.0) as u32;
            let y0 = (cy - half).max(0.0) as u32;
            let side = (half * 2.0) as u32;
            let w = side.min(rgb.width().saturating_sub(x0)).max(1);
            let h = side.min(rgb.height().saturating_sub(y0)).max(1);
            let ctx = image::imageops::crop_imm(&rgb, x0, y0, w, h).to_image();
            image::imageops::resize(&ctx, 160, 160, image::imageops::FilterType::Triangle)
                .save(out.join("context").join(format!("{id}_{n}.jpg")))?;

            writeln!(
                lines,
                "{}",
                json!({
                    "photo": id, "n": n, "box": [f.x, f.y, f.w, f.h], "score": f.score,
                    "landmarks": f.landmarks, "groupable": is_groupable(Some(f.score), f.w, f.h),
                    "img": [rgb.width(), rgb.height()], "embedding": emb,
                    "pose": sourcelight_lib::people::pose_offset(&f.landmarks),
                })
            )?;
            total += 1;
        }
    }
    eprintln!("{total} faces from {} photos", photos.len());
    Ok(())
}

fn library(models: &str, db_path: &str) -> anyhow::Result<()> {
    use sourcelight_lib::db;
    let engine = FaceEngine::load(&PathBuf::from(models))?;
    anyhow::ensure!(
        engine.can_embed(),
        "the face embedder is missing from {models}"
    );
    let mut conn = db::open(&PathBuf::from(db_path))?;
    let queue = db::unanalysed_photos(&conn, 1_000_000)?;
    eprintln!("{} photos queued for faces after migrating", queue.len());
    for (photo_id, path) in queue {
        let bytes = std::fs::read(&path)?;
        let found = engine.detect(&bytes)?;
        let marks: Vec<[[f32; 2]; 5]> = found.iter().map(|f| f.landmarks).collect();
        let embs = engine.embed_all(&bytes, &marks)?;
        let size = image::load_from_memory(&bytes)?.dimensions();
        db::store_faces(&mut conn, photo_id, &found, &embs, size)?;
    }
    let people = db::regroup_people(&mut conn)?;
    eprintln!("{people} people");
    let mut stmt = conn.prepare(
        "SELECT f.photo_id, f.x, f.y, f.w, f.h, f.person_id, pe.name
           FROM faces f LEFT JOIN people pe ON pe.id = f.person_id ORDER BY f.photo_id, f.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(json!({
            "photo": r.get::<_, i64>(0)?,
            "box": [r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?, r.get::<_, f64>(4)?],
            "person": r.get::<_, Option<i64>>(5)?, "name": r.get::<_, Option<String>>(6)?,
        }))
    })?;
    for row in rows {
        println!("{}", row?);
    }
    for (a, b, s) in db::people_suggestions(&conn, 5)? {
        println!("{}", json!({"suggest": [a, b], "score": s}));
    }
    Ok(())
}
