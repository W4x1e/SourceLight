//! AI performance probe: how long each of the app's own models takes, and how
//! much memory it holds, on the hardware and settings you point it at
//! (docs/BENCHMARKS.md, "Slower machines").
//!
//! ```sh
//! cargo build --release --features ai,dev-tools --bin probe_ai
//! probe_ai raw <model.onnx> [--ep cpu|dml|dml:N] [--threads N] [--no-spin]
//!              [--opt 0-3] [--runs N] [--low-priority]
//! probe_ai index <models dir> <folder of JPEG previews> [--n N]
//! probe_ai faces <models dir> <folder of JPEG previews> [--n N] [--no-embed]
//! probe_ai subject|click|erase|denoise <models dir> [--runs N]
//! ```
//!
//! `raw` builds the session itself, with exactly the options given, and feeds
//! it inputs of the declared shapes: the cost of the model alone. The other
//! modes go through the app's own loaders and pre- and post-processing, so
//! they measure what a person waits for. Set `SOURCELIGHT_AI_DEVICE=cpu`
//! (or a DirectML adapter index) to choose where those run; to simulate a
//! smaller CPU, pin the process to some of the cores (`start /affinity 55`).
//!
//! Every mode prints one JSON line. Peak memory is the process's peak working
//! set and peak private (committed) bytes, read from Windows; run one mode
//! per process so each figure belongs to one model.

use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().context("usage: probe_ai <mode> …")?.clone();
    let rest = &args[1..];
    let mut out = match mode.as_str() {
        "raw" => raw(rest)?,
        "index" => index(rest)?,
        "faces" => faces(rest)?,
        "subject" | "click" | "erase" | "denoise" => interactive(&mode, rest)?,
        "adapters" => adapters()?,
        "jitter" => jitter(rest)?,
        other => anyhow::bail!("unknown mode {other}"),
    };
    let (peak_ws, peak_private) = peak_memory();
    out["peak_ws_mb"] = json!(peak_ws / 1_048_576);
    out["peak_private_mb"] = json!(peak_private / 1_048_576);
    println!("{out}");
    sourcelight_lib::develop::ai::release_ort();
    Ok(())
}

/// The value after `--name`, if given.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Median, 90th percentile and minimum of `ms`, in that order.
fn stats(ms: &mut [f64]) -> (f64, f64, f64) {
    if ms.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    ms.sort_by(f64::total_cmp);
    let at = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.9), ms[0])
}

fn since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn tenth(ms: f64) -> f64 {
    (ms * 10.0).round() / 10.0
}

/// A deterministic stream of values in -1..1, so every run feeds the same input.
fn noise(n: usize, seed: u64) -> Vec<f32> {
    let step = |s: u64| {
        s.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)
    };
    let mut s = step(seed);
    (0..n)
        .map(|_| {
            s = step(s);
            ((s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

// ---------------------------------------------------------------------------
// raw: one model, the options given, inputs of the declared shapes.
// ---------------------------------------------------------------------------

fn raw(args: &[String]) -> Result<serde_json::Value> {
    use ort::execution_providers::{CPUExecutionProvider, DirectMLExecutionProvider};
    use ort::session::builder::GraphOptimizationLevel;
    use ort::value::ValueType;

    let path = PathBuf::from(args.first().context("raw <model.onnx>")?);
    let ep = flag(args, "--ep").unwrap_or("cpu").to_string();
    let threads: Option<usize> = flag(args, "--threads").map(str::parse).transpose()?;
    let spin = !has(args, "--no-spin");
    let opt: u8 = flag(args, "--opt")
        .map(str::parse)
        .transpose()?
        .unwrap_or(3);
    let runs: usize = flag(args, "--runs")
        .map(str::parse)
        .transpose()?
        .unwrap_or(10);
    let low = has(args, "--low-priority");

    sourcelight_lib::develop::ai::ensure_ort_dylib();
    let started = Instant::now();
    let mut b = ort::session::Session::builder()?.with_optimization_level(match opt {
        0 => GraphOptimizationLevel::Disable,
        1 => GraphOptimizationLevel::Level1,
        2 => GraphOptimizationLevel::Level2,
        _ => GraphOptimizationLevel::Level3,
    })?;
    if let Some(n) = threads {
        b = b.with_intra_threads(n)?;
    }
    if !spin {
        b = b
            .with_intra_op_spinning(false)?
            .with_inter_op_spinning(false)?;
    }
    if low {
        b = sourcelight_lib::develop::ai::low_priority_threads(b)?;
    }
    if has(args, "--no-mem-pattern") {
        b = b.with_memory_pattern(false)?;
    }
    // Writes the graph as optimised, to time loading it back (`--opt 0`).
    if let Some(out) = flag(args, "--save-optimized") {
        b = b.with_optimized_model_path(out)?;
    }
    let arena = has(args, "--arena");
    let providers = match ep.as_str() {
        "cpu" => vec![CPUExecutionProvider::default()
            .with_arena_allocator(arena)
            .build()],
        dml => {
            let device = dml
                .strip_prefix("dml:")
                .map(str::parse)
                .transpose()?
                .unwrap_or(sourcelight_lib::hardware::ai_gpu().0 as i32);
            b = b.with_memory_pattern(false)?;
            vec![
                DirectMLExecutionProvider::default()
                    .with_device_id(device)
                    .build()
                    .error_on_failure(),
                CPUExecutionProvider::default().build(),
            ]
        }
    };
    let mut session = b
        .with_execution_providers(providers)?
        .commit_from_file(&path)
        .with_context(|| format!("loading {}", path.display()))?;
    let load_ms = since(started);

    // `--shape 1x3x224x224` fixes the first input's dimensions, for a model
    // that leaves them all free (SigLIP 2's vision tower).
    let first_shape: Option<Vec<i64>> = flag(args, "--shape")
        .map(|s| s.split('x').map(str::parse).collect())
        .transpose()?;
    let mut inputs: Vec<(String, ort::value::DynValue)> = Vec::new();
    let mut shapes = Vec::new();
    for (k, input) in session.inputs.iter().enumerate() {
        let ValueType::Tensor { ty, shape, .. } = &input.input_type else {
            anyhow::bail!("input {} is not a tensor", input.name);
        };
        let given = first_shape.as_ref().filter(|_| k == 0);
        let dims: Vec<i64> = shape
            .iter()
            .enumerate()
            .map(|(i, &d)| match (given, d) {
                (Some(g), _) => g[i],
                (_, d) if d > 0 => d,
                // Batch of one; a text model's sequence is SigLIP's 64.
                _ if i == 0 => 1,
                _ if input.name.contains("ids") || input.name.contains("mask") => 64,
                _ => 1,
            })
            .collect();
        let n: usize = dims.iter().product::<i64>() as usize;
        let value: ort::value::DynValue = match ty {
            ort::tensor::TensorElementType::Float32 => {
                let v = if input.name == "mask" {
                    // A hole mask (LaMa) is 0 or 1: a square in the middle.
                    let side = (n as f64).sqrt() as usize;
                    let q = side / 4;
                    (0..n)
                        .map(|i| {
                            let (x, y) = (i % side, i / side);
                            ((q..3 * q).contains(&x) && (q..3 * q).contains(&y)) as u8 as f32
                        })
                        .collect()
                } else {
                    noise(n, k as u64)
                };
                ort::value::Tensor::from_array((dims.clone(), v))?.into_dyn()
            }
            ort::tensor::TensorElementType::Int64 => {
                let v: Vec<i64> = if input.name.contains("mask") {
                    vec![1; n]
                } else {
                    (0..n as i64).map(|i| 1 + (i * 37) % 1000).collect()
                };
                ort::value::Tensor::from_array((dims.clone(), v))?.into_dyn()
            }
            other => anyhow::bail!("input {} is {other:?}, not handled", input.name),
        };
        shapes.push(format!("{}={:?}", input.name, dims));
        inputs.push((input.name.clone(), value));
    }

    let run = |session: &mut ort::session::Session| -> Result<f64> {
        let t = Instant::now();
        let feed: Vec<(String, ort::session::SessionInputValue<'_>)> = inputs
            .iter()
            .map(|(name, v)| (name.clone(), v.view().into()))
            .collect();
        let outputs = session.run(feed)?;
        drop(outputs);
        Ok(since(t))
    };
    let first_ms = run(&mut session)?;
    let _ = run(&mut session)?;
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        times.push(run(&mut session)?);
    }
    let (median, p90, min) = stats(&mut times);
    Ok(json!({
        "mode": "raw",
        "model": path.file_name().map(|f| f.to_string_lossy().to_string()),
        "ep": ep, "threads": threads, "spin": spin, "opt": opt, "low_priority": low,
        "inputs": shapes,
        "load_ms": load_ms.round(), "first_ms": first_ms.round(),
        "median_ms": tenth(median), "p90_ms": tenth(p90), "min_ms": tenth(min),
        "runs": runs,
    }))
}

// ---------------------------------------------------------------------------
// The library passes, through the app's own code.
// ---------------------------------------------------------------------------

/// Up to `n` JPEGs from `dir`, in name order, so every run sees the same set.
fn jpegs(dir: &Path, n: usize) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        })
        .collect();
    files.sort();
    files.truncate(n);
    anyhow::ensure!(!files.is_empty(), "no JPEGs in {}", dir.display());
    Ok(files)
}

fn file_name(p: &Path) -> Option<String> {
    p.file_name().map(|n| n.to_string_lossy().to_string())
}

/// The search index's pass: read the preview and embed it (`embed_image`
/// decodes, resizes and runs SigLIP 2's vision tower), as `index_for_search`
/// does. The decode and resize are timed again on their own, to split the
/// cost. `--save` writes the embeddings, to compare two runs' agreement.
fn index(args: &[String]) -> Result<serde_json::Value> {
    use sourcelight_lib::tagger::builtin::{siglip_preprocess, BuiltinTagger};
    let models = PathBuf::from(args.first().context("index <models dir> <jpeg dir>")?);
    let dir = PathBuf::from(args.get(1).context("index <models dir> <jpeg dir>")?);
    let n: usize = flag(args, "--n").map(str::parse).transpose()?.unwrap_or(50);
    let files = jpegs(&dir, n)?;

    let started = Instant::now();
    let tagger = BuiltinTagger::load(&models.join("siglip2"))?;
    // The first photo pays for the session's load; it is reported apart.
    let first = std::fs::read(&files[0])?;
    let t = Instant::now();
    tagger.embed_image(&first)?;
    let first_ms = since(t);
    let load_ms = since(started) - first_ms;

    let (mut total, mut prep, mut decode) = (Vec::new(), Vec::new(), Vec::new());
    let mut embeddings = Vec::new();
    let whole = Instant::now();
    for f in &files {
        let t = Instant::now();
        let bytes = std::fs::read(f)?;
        embeddings.push(tagger.embed_image(&bytes)?);
        total.push(since(t));
        let t = Instant::now();
        let rgb = image::load_from_memory(&bytes)?.to_rgb8();
        decode.push(since(t));
        let t = Instant::now();
        let _ = siglip_preprocess(rgb.as_raw(), rgb.width(), rgb.height(), 224);
        prep.push(since(t) + decode.last().copied().unwrap_or(0.0));
    }
    // The second, split-out decode is not part of the pass.
    let wall = whole.elapsed().as_secs_f64() - prep.iter().sum::<f64>() / 1000.0;
    if let Some(out) = flag(args, "--save") {
        let rows: Vec<serde_json::Value> = files
            .iter()
            .zip(&embeddings)
            .map(|(f, e)| json!({"file": file_name(f), "emb": e}))
            .collect();
        std::fs::write(out, serde_json::to_string(&rows)?)?;
    }
    let (median, p90, _) = stats(&mut total);
    let (prep_median, _, _) = stats(&mut prep);
    let (decode_median, _, _) = stats(&mut decode);
    Ok(json!({
        "mode": "index", "photos": files.len(),
        "load_ms": load_ms.round(), "first_ms": first_ms.round(),
        "median_ms": tenth(median), "p90_ms": tenth(p90),
        "decode_resize_median_ms": tenth(prep_median),
        "decode_median_ms": tenth(decode_median),
        "photos_per_min": (files.len() as f64 / wall * 60.0).round(),
    }))
}

/// The face pass, as `analyze_faces` runs it: decode once, find the faces
/// (YuNet, then the face mesh three times and the blendshapes per face, and
/// each face's sharpness), then AuraFace on each face and its mirror image.
fn faces(args: &[String]) -> Result<serde_json::Value> {
    use sourcelight_lib::faces::FaceEngine;
    let models = PathBuf::from(args.first().context("faces <models dir> <jpeg dir>")?);
    let dir = PathBuf::from(args.get(1).context("faces <models dir> <jpeg dir>")?);
    let n: usize = flag(args, "--n").map(str::parse).transpose()?.unwrap_or(50);
    let embed = !has(args, "--no-embed");
    let files = jpegs(&dir, n)?;

    let started = Instant::now();
    let engine = FaceEngine::load(&models.join("faces"))?;
    let load_ms = since(started);

    let (mut per_photo, mut detect, mut per_face) = (Vec::new(), Vec::new(), Vec::new());
    let mut face_count = 0usize;
    let mut saved = Vec::new();
    let whole = Instant::now();
    for f in &files {
        let t = Instant::now();
        let bytes = std::fs::read(f)?;
        let rgb = image::load_from_memory(&bytes)?.to_rgb8();
        let td = Instant::now();
        let found = engine.detect_rgb(&rgb)?;
        detect.push(since(td));
        face_count += found.len();
        if embed && engine.can_embed() && !found.is_empty() {
            let marks: Vec<[[f32; 2]; 5]> = found.iter().map(|f| f.landmarks).collect();
            let te = Instant::now();
            let v = engine.embed_all_rgb(&rgb, &marks)?;
            per_face.push(since(te) / found.len() as f64);
            saved.push(json!({
                "file": file_name(f),
                "faces": found.iter().map(|f| json!({"box": f.bbox(), "eye": f.eye_open})).collect::<Vec<_>>(),
                "emb": v,
            }));
        }
        per_photo.push(since(t));
    }
    let wall = whole.elapsed().as_secs_f64();
    if let Some(out) = flag(args, "--save") {
        std::fs::write(out, serde_json::to_string(&saved)?)?;
    }
    let (median, p90, _) = stats(&mut per_photo);
    let (detect_median, _, _) = stats(&mut detect);
    let (face_median, face_p90, _) = stats(&mut per_face);
    Ok(json!({
        "mode": "faces", "photos": files.len(), "faces": face_count, "embed": embed,
        "load_ms": load_ms.round(),
        "median_ms": tenth(median), "p90_ms": tenth(p90),
        "detect_median_ms": tenth(detect_median),
        "embed_per_face_median_ms": tenth(face_median),
        "embed_per_face_p90_ms": tenth(face_p90),
        "photos_per_min": (files.len() as f64 / wall * 60.0).round(),
    }))
}

/// The interactive tools, through the app's loaders: one Select Subject on a
/// frame the size `selection_input` renders, one Click to Select encode (and
/// its decode, reported apart), one magic-erase window, one denoise of a
/// 1024×1024 image.
fn interactive(mode: &str, args: &[String]) -> Result<serde_json::Value> {
    let models = PathBuf::from(args.first().context("<mode> <models dir>")?);
    let runs: usize = flag(args, "--runs")
        .map(str::parse)
        .transpose()?
        .unwrap_or(5);
    let mut times = Vec::new();
    let mut extra = serde_json::Map::new();
    let first_ms;
    let load_ms;
    let frame = |seed: u64| -> Vec<u8> {
        noise(1024 * 683 * 4, seed)
            .iter()
            .map(|v| ((v + 1.0) * 127.0) as u8)
            .collect()
    };
    match mode {
        "subject" => {
            use sourcelight_lib::develop::ai::{Segmenter, BIREFNET};
            // `--model` another export of it; `--images` and `--save` segment
            // real photos and write their coverage, to compare two runs.
            let model = flag(args, "--model")
                .map(PathBuf::from)
                .or_else(|| sourcelight_lib::ai_models::best_file(&models, "birefnet.onnx"))
                .context("no BiRefNet model")?;
            extra.insert("model".into(), json!(file_name(&model)));
            let t = Instant::now();
            let mut seg = Segmenter::load(&model, BIREFNET)?;
            load_ms = since(t);
            if let (Some(images), Some(save)) = (flag(args, "--images"), flag(args, "--save")) {
                std::fs::create_dir_all(save)?;
                for f in jpegs(Path::new(images), runs)? {
                    let rgba = image::open(&f)?.to_rgba8();
                    let t = Instant::now();
                    let (w, h, coverage) =
                        seg.segment(rgba.as_raw(), rgba.width(), rgba.height())?;
                    times.push(since(t));
                    image::GrayImage::from_raw(w, h, coverage)
                        .context("coverage size")?
                        .save(Path::new(save).join(file_name(&f).unwrap_or_default() + ".png"))?;
                }
            }
            let rgba = frame(7);
            let t = Instant::now();
            seg.segment(&rgba, 1024, 683)?;
            first_ms = since(t);
            for _ in 0..runs {
                let t = Instant::now();
                seg.segment(&rgba, 1024, 683)?;
                times.push(since(t));
            }
            // The image work around the model, on its own.
            use sourcelight_lib::develop::ai::{coverage_size, postprocess, preprocess};
            let (mut pre, mut post) = (Vec::new(), Vec::new());
            let matte = noise(1024 * 1024, 3);
            for _ in 0..runs {
                let t = Instant::now();
                std::hint::black_box(preprocess(&rgba, 1024, 683, &BIREFNET));
                pre.push(since(t));
                let t = Instant::now();
                let (ow, oh) = coverage_size(1024, 683);
                std::hint::black_box(postprocess(&matte, &BIREFNET, ow, oh));
                post.push(since(t));
            }
            extra.insert("preprocess_ms".into(), json!(tenth(stats(&mut pre).0)));
            extra.insert("postprocess_ms".into(), json!(tenth(stats(&mut post).0)));
        }
        "click" => {
            use sourcelight_lib::develop::sam2::{Point, Sam2};
            let t = Instant::now();
            let mut sam = Sam2::load(&models.join("sam2"))?;
            load_ms = since(t);
            let rgba = frame(9);
            let point = [Point {
                x: 0.5,
                y: 0.5,
                include: true,
            }];
            let t = Instant::now();
            let emb = sam.encode(&rgba, 1024, 683)?;
            sam.decode(&emb, &point, 1024, 683)?;
            first_ms = since(t);
            let mut decode = Vec::new();
            for _ in 0..runs {
                let t = Instant::now();
                let emb = sam.encode(&rgba, 1024, 683)?;
                times.push(since(t));
                let t = Instant::now();
                sam.decode(&emb, &point, 1024, 683)?;
                decode.push(since(t));
            }
            let (d, _, _) = stats(&mut decode);
            extra.insert("decode_median_ms".into(), json!(tenth(d)));
        }
        "erase" => {
            use sourcelight_lib::develop::erase::Inpainter;
            let t = Instant::now();
            let lama = Inpainter::load(&models)?;
            load_ms = since(t);
            let t = Instant::now();
            lama.warm()?;
            first_ms = since(t);
            for _ in 0..runs {
                let t = Instant::now();
                lama.warm()?;
                times.push(since(t));
            }
        }
        _ => {
            use sourcelight_lib::denoise::DenoiseEngine;
            let t = Instant::now();
            let engine = DenoiseEngine::load(&models)?;
            load_ms = since(t);
            let (w, h) = (1024u32, 1024u32);
            let rgba: Vec<u16> = noise((w * h * 4) as usize, 11)
                .iter()
                .map(|v| half::f16::from_f32((v + 1.0) * 0.25).to_bits())
                .collect();
            let mut tiles = 0;
            let t = Instant::now();
            engine.denoise(&rgba, w, h, |_, n| tiles = n, || false)?;
            first_ms = since(t);
            for _ in 0..runs {
                let t = Instant::now();
                engine.denoise(&rgba, w, h, |_, _| {}, || false)?;
                times.push(since(t));
            }
            extra.insert("tiles".into(), json!(tiles));
        }
    }
    let (median, p90, _) = stats(&mut times);
    let mut out = json!({
        "mode": mode, "runs": runs,
        "load_ms": load_ms.round(), "first_ms": first_ms.round(),
        "median_ms": tenth(median), "p90_ms": tenth(p90),
    });
    for (k, v) in extra {
        out[k] = v;
    }
    Ok(out)
}

/// Stands in for the window while something else loads the CPU: sixty times
/// a second, a fixed slice of work (about 2 ms on an idle core), the way a
/// frame of the interface is. On a machine whose cores the models fill, the
/// slice takes longer whenever it has to wait for one; `--secs` long.
fn jitter(args: &[String]) -> Result<serde_json::Value> {
    let secs: f64 = flag(args, "--secs")
        .map(str::parse)
        .transpose()?
        .unwrap_or(15.0);
    let iterations: u64 = flag(args, "--work")
        .map(str::parse)
        .transpose()?
        .unwrap_or(2_000_000);
    let slice = |seed: u64| {
        let mut s = seed;
        for _ in 0..iterations {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
        }
        std::hint::black_box(s)
    };
    let frame = std::time::Duration::from_micros(16_667);
    let started = Instant::now();
    let mut next = started + frame;
    let (mut work, mut late) = (Vec::new(), Vec::new());
    let mut n = 0u64;
    while started.elapsed().as_secs_f64() < secs {
        let t = Instant::now();
        slice(n);
        work.push(since(t));
        n += 1;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        late.push(Instant::now().saturating_duration_since(next).as_secs_f64() * 1000.0);
        next += frame;
        if next < Instant::now() {
            next = Instant::now() + frame;
        }
    }
    let over = work.iter().filter(|&&w| w > 16.7).count();
    let total = work.len();
    work.sort_by(f64::total_cmp);
    late.sort_by(f64::total_cmp);
    let q = |v: &[f64], q: f64| tenth(v[((v.len() - 1) as f64 * q).round() as usize]);
    Ok(json!({
        "mode": "jitter", "frames": total,
        "work_median_ms": q(&work, 0.5), "work_p95_ms": q(&work, 0.95),
        "work_p99_ms": q(&work, 0.99), "work_max_ms": q(&work, 1.0),
        "frames_over_16ms": over,
        "late_p95_ms": q(&late, 0.95), "late_p99_ms": q(&late, 0.99),
    }))
}

/// The GPUs DirectML can be pointed at, in the order its `device_id` counts
/// them, and the one the app picks.
#[cfg(windows)]
fn adapters() -> Result<serde_json::Value> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    // SAFETY: plain DXGI queries on interfaces this function owns.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut found = Vec::new();
    let mut i = 0;
    // SAFETY: as above.
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
        // SAFETY: as above.
        let d = unsafe { adapter.GetDesc1()? };
        found.push(json!({
            "index": i,
            "name": String::from_utf16_lossy(&d.Description).trim_end_matches('\0'),
            "dedicated_mb": d.DedicatedVideoMemory / 1_048_576,
            "shared_mb": d.SharedSystemMemory / 1_048_576,
            "software": d.Flags & 2 != 0,
        }));
        i += 1;
    }
    let (index, name) = sourcelight_lib::hardware::ai_gpu();
    Ok(json!({"mode": "adapters", "adapters": found, "ai_gpu": [index, name]}))
}

#[cfg(not(windows))]
fn adapters() -> Result<serde_json::Value> {
    Ok(json!({"mode": "adapters", "adapters": []}))
}

// ---------------------------------------------------------------------------
// Peak memory, from Windows.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn peak_memory() -> (u64, u64) {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: a plain out-parameter struct of the size it declares.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0;
    if ok {
        (c.peak_working_set_size as u64, c.peak_pagefile_usage as u64)
    } else {
        (0, 0)
    }
}

#[cfg(not(windows))]
fn peak_memory() -> (u64, u64) {
    (0, 0)
}
