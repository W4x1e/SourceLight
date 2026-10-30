//! The vision model end to end, through the app's own code: the pinned
//! llama.cpp runtime downloaded and unpacked (`llama_pack`), a catalogue
//! model's pinned files downloaded (`vlm`, `ai_models`), `llama-server`
//! started on the device the app would choose (`sidecar`), and real previews
//! tagged and described by the tagger (`tagger::llama`).
//!
//! It downloads a runtime and a model (about 1.6 GB for the 2B), so it is
//! `#[ignore]`d and runs only when `VISION_E2E_DIR` names a scratch folder to
//! put them in. A second run reuses what the first downloaded.
//!
//! ```text
//! VISION_E2E_DIR=/tmp/vision cargo test --features ai --test vision_e2e -- --ignored --nocapture
//! ```
//! `VISION_E2E_MODEL` picks the model (default `qwen3-vl-2b`),
//! `VISION_E2E_PHOTOS` a folder of JPEGs (default: the app's previews), and
//! `TAG_PROBE_LIMIT` how many photos (default 6).
#![cfg(feature = "ai")]

use sourcelight_lib::{ai_models, hardware, jobs, llama_pack, sidecar, tagger, vlm};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// `limit` photos spread across the folder: `VISION_E2E_PHOTOS`, or the
/// app's own previews (`_p.jpg`, beside thumbnails of the same photos).
fn previews(limit: usize) -> Vec<PathBuf> {
    let (dir, previews_only) = match std::env::var_os("VISION_E2E_PHOTOS") {
        Some(dir) => (PathBuf::from(dir), false),
        None => {
            let base = if cfg!(target_os = "macos") {
                PathBuf::from(std::env::var_os("HOME").unwrap()).join("Library/Application Support")
            } else {
                PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
            };
            (base.join("app.sourcelight").join("thumbs"), true)
        }
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("reading the photo folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.to_lowercase().ends_with(".jpg") && (!previews_only || name.ends_with("_p.jpg"))
        })
        .collect();
    files.sort();
    let step = (files.len() / limit.max(1)).max(1);
    files.into_iter().step_by(step).take(limit).collect()
}

#[test]
#[ignore = "downloads the vision runtime and a model; set VISION_E2E_DIR"]
fn the_vision_model_downloads_starts_tags_and_describes() {
    let Some(root) = std::env::var_os("VISION_E2E_DIR").map(PathBuf::from) else {
        eprintln!("skipped: set VISION_E2E_DIR to a scratch folder");
        return;
    };
    let id = std::env::var("VISION_E2E_MODEL").unwrap_or_else(|_| "qwen3-vl-2b".into());
    let limit: usize = std::env::var("TAG_PROBE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let model = vlm::find(&id).expect("a catalogue model");
    let runtime_dir = root.join("llama-runtime");
    let models_dir = root.join("models");
    let jobs = jobs::Jobs::default();
    let rt = tokio::runtime::Runtime::new().unwrap();

    // The runtime and the model, as the setup ladder fetches them.
    if !llama_pack::installed(&runtime_dir) {
        let started = Instant::now();
        let version = rt
            .block_on(llama_pack::install(
                &runtime_dir,
                &jobs.start("vision-runtime"),
                |_, _, _| {},
            ))
            .expect("installing the runtime");
        println!(
            "runtime {version} in {:.1}s",
            started.elapsed().as_secs_f32()
        );
    }
    if !ai_models::files_installed(model.files, &models_dir) {
        let started = Instant::now();
        rt.block_on(ai_models::download_files(
            model.name,
            vlm::size_mb(model.id),
            model.files,
            &models_dir,
            &jobs.start("vlm-model"),
            |_, _| {},
        ))
        .expect("downloading the model");
        println!("{} in {:.1}s", model.name, started.elapsed().as_secs_f32());
    }

    // The server, on the device the app would choose.
    let binary = sidecar::llama_server_binary(&runtime_dir).expect("llama-server is installed");
    let discrete = hardware::vram_mb().is_some_and(|mb| mb >= 2048);
    let devices = sidecar::list_devices(&binary);
    let device = if cfg!(windows) {
        sidecar::choose_device(&devices, &hardware::ai_gpu().1, discrete)
    } else {
        None
    };
    println!("devices {devices:?} → {device:?}");
    let parallel = tagger::resolve_parallel(0, hardware::vram_mb());
    let args = sidecar::llama_server_args(
        &models_dir.join(model.weights()),
        &models_dir.join(model.projector()),
        sidecar::llama_server_slots(parallel),
        device.as_deref(),
    );
    let mut server =
        sidecar::spawn_llama_server(&binary, &args, None).expect("starting llama-server");
    let started = Instant::now();
    let loaded = rt.block_on(async {
        let health = format!("http://127.0.0.1:{}/health", sidecar::LLAMA_SERVER_PORT);
        let client = reqwest::Client::new();
        for _ in 0..480 {
            if client
                .get(&health)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        false
    });
    println!("loaded {loaded} in {:.1}s", started.elapsed().as_secs_f32());

    let engine = tagger::build(&tagger::ProviderConfig {
        provider: tagger::Provider::Vision,
        model: id.clone(),
        model_dir: Some(models_dir.clone()),
        runtime_dir: Some(runtime_dir.clone()),
        parallel,
        caption_language: None,
    })
    .expect("building the tagger");

    let photos = previews(limit);
    let mut failures = Vec::new();
    for (task, run) in [("tag", 0), ("describe", 1), ("both", 2)] {
        let started = Instant::now();
        for photo in &photos {
            let bytes = std::fs::read(photo).unwrap();
            let name = photo.file_name().unwrap().to_string_lossy();
            let outcome = rt.block_on(async {
                match run {
                    0 => engine.tag(&bytes).await.map(|t| {
                        format!(
                            "{} / {} / {}",
                            t.genre,
                            t.main_subject,
                            t.subjects.join(", ")
                        )
                    }),
                    1 => engine.caption(&bytes).await,
                    _ => engine
                        .tag_and_caption(&bytes)
                        .await
                        .map(|(t, d)| format!("{} / {d}", t.genre)),
                }
            });
            match outcome {
                Ok(text) => println!("  {task} {name}: {text:.160}"),
                Err(e) => failures.push(format!("{task} {name}: {e:#}")),
            }
        }
        println!(
            "{task}: {:.2}s a photo, one at a time",
            started.elapsed().as_secs_f32() / photos.len() as f32
        );
    }
    sidecar::stop(&mut server, sidecar::STOP_GRACE);

    assert!(loaded, "the server never answered /health");
    assert!(!photos.is_empty(), "no photos to try");
    assert!(failures.is_empty(), "failed: {failures:#?}");
}
