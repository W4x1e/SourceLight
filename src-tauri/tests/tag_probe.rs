//! Eyeball check of the built-in (CLIP) tagger against real photos.
//!
//! Not a pass/fail test — tag quality has no ground truth here — so it is
//! `#[ignore]`d and prints one line per photo for a person to read, or to diff
//! against a run of an older tagger. It skips cleanly when the model or the
//! photos are absent.
//!
//! Run with:
//! ```text
//! cargo test --features ai --test tag_probe -- --ignored --nocapture
//! ```
//! `TAG_PROBE_DIR` points at a folder of JPEGs (default: the app's own preview
//! cache), `TAG_PROBE_LIMIT` caps how many are tagged (default 40), and
//! `TAG_PROBE_OUT` also writes the report to a file.
#![cfg(feature = "ai")]

use sourcelight_lib::tagger;
use std::path::{Path, PathBuf};

/// The catalogue to read: a development one (`SOURCELIGHT_DATA_DIR`) first,
/// then where the installed app keeps its data on each OS.
fn app_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SOURCELIGHT_DATA_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    };
    Some(base.join("app.sourcelight"))
}

/// Every `.jpg` under `dir`, sorted so two runs tag the same photos in order.
/// Only previews (`_p.jpg`) when reading the app's cache, which also holds
/// thumbnails of the same photos.
fn photos(dir: &Path, previews_only: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let jpg = name.to_lowercase().ends_with(".jpg");
            if jpg && (!previews_only || name.ends_with("_p.jpg")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
#[ignore = "prints tags for a person to read; needs the CLIP model and photos"]
fn print_builtin_tags() {
    let Some(app) = app_dir() else {
        eprintln!("skipped: no app data folder");
        return;
    };
    let clip = app.join("models").join("clip");
    if !clip.join("vision_model.onnx").exists() {
        eprintln!("skipped: CLIP model not downloaded");
        return;
    }
    let (dir, previews_only) = match std::env::var_os("TAG_PROBE_DIR") {
        Some(d) => (PathBuf::from(d), false),
        None => (app.join("thumbs"), true),
    };
    let limit: usize = std::env::var("TAG_PROBE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let files: Vec<PathBuf> = photos(&dir, previews_only)
        .into_iter()
        .take(limit)
        .collect();
    if files.is_empty() {
        eprintln!("skipped: no photos under {}", dir.display());
        return;
    }

    let started = std::time::Instant::now();
    let engine = tagger::build(&tagger::ProviderConfig {
        provider: tagger::Provider::Builtin,
        model: String::new(),
        model_dir: Some(app.join("models")),
        runtime_dir: None,
        parallel: 1,
        caption_language: None,
    })
    .expect("loading the built-in tagger");
    let loaded = started.elapsed();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut report = String::new();
    let tagging = std::time::Instant::now();
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let r = rt.block_on(engine.tag(&bytes)).expect("tagging");
        let name = f.file_name().unwrap().to_string_lossy();
        report.push_str(&format!(
            "{name}\n  genre={} ({:.2})  subjects=[{}]  conditions=[{}]\n",
            r.genre,
            r.confidence,
            r.subjects.join(", "),
            r.conditions.join(", "),
        ));
    }
    report.push_str(&format!(
        "\n{} photos · load {:.1}s · tag {:.1}s\n",
        files.len(),
        loaded.as_secs_f32(),
        tagging.elapsed().as_secs_f32()
    ));

    println!("{report}");
    if let Some(out) = std::env::var_os("TAG_PROBE_OUT") {
        std::fs::write(out, &report).unwrap();
    }
}
