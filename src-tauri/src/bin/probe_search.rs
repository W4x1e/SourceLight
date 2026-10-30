//! Search probe: the shipped search path run outside the app, so it can be
//! scored against labelled queries (`scripts/model-eval/searchcheck.py`).
//!
//! ```sh
//! cargo run --release --features ai,dev-tools --bin probe_search -- \
//!     <models/siglip2 dir> <copy of library.db> <queries.json>
//! ```
//!
//! Indexes every photo of the catalogue copy exactly as indexing does
//! (`BuiltinTagger::embed_image` on the preview, stored with
//! `db::set_embedding`), then for each query in `queries.json` (a JSON array
//! of strings) embeds it and ranks the library with `db::all_embeddings` and
//! `search::rank`, as the semantic half of search does. Prints one JSON line
//! per query with the ranked ids, then one per photo with the tags the
//! built-in tagger gives it.

use serde_json::json;
use sourcelight_lib::db;
use sourcelight_lib::search;
use sourcelight_lib::tagger::builtin::BuiltinTagger;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: probe_search <siglip2 dir> <library.db copy> <queries.json>"
    );
    let tagger = BuiltinTagger::load(&PathBuf::from(&args[0]))?;
    let conn = db::open(&PathBuf::from(&args[1]))?;
    let queries: Vec<String> = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;

    let photos: Vec<(i32, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(preview_path, thumb_path) FROM photos
              WHERE variant = 0 AND COALESCE(preview_path, thumb_path) IS NOT NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let started = std::time::Instant::now();
    let mut tags = Vec::new();
    for (id, path) in &photos {
        let emb = tagger.embed_image(&std::fs::read(path)?)?;
        db::set_embedding(&conn, *id, &search::encode_embedding(&emb))?;
        tags.push((*id, tagger.tag_embedding(&emb)));
    }
    eprintln!(
        "indexed {} photos in {:.1}s",
        photos.len(),
        started.elapsed().as_secs_f32()
    );

    let candidates = db::all_embeddings(&conn)?;
    for q in &queries {
        let qvec = tagger.embed_query(q)?;
        let ranked: Vec<i64> = search::rank(&qvec, &candidates, 1000, 0.0)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        println!("{}", json!({ "query": q, "ranked": ranked }));
    }
    for (id, t) in tags {
        let t = t.map(
            |t| json!({ "genre": t.genre, "subjects": t.subjects, "conditions": t.conditions }),
        );
        println!("{}", json!({ "photo": id, "tags": t }));
    }
    drop(tagger);
    sourcelight_lib::develop::ai::release_ort();
    Ok(())
}
