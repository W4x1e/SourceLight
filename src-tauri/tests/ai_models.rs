//! End-to-end checks against the *real* model files, when they are installed.
//!
//! These are the only tests that exercise actual inference: everything else in
//! the AI path is pure maths with the model mocked out. They skip cleanly when
//! the downloaded models or ONNX Runtime are absent (a fresh clone), so they
//! cost nothing to keep.
//!
//! Run with:
//! ```text
//! cargo test --features ai --test ai_models -- --nocapture
//! ```
#![cfg(feature = "ai")]

use sourcelight_lib::test_support::{self, Needs};
use std::path::PathBuf;

/// One real model on the GPU at a time. The harness runs tests in parallel,
/// and DirectML creating several sessions at once can fail with "Catastrophic
/// failure" in `DmlGraphFusionHelper` (seen with SAM 2 once there were five of
/// these). The app never loads models concurrently like that, so the tests
/// shouldn't either.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether there is an ONNX Runtime to load, or `false` with the skip recorded.
/// `ensure_ort_dylib` sets `ORT_DYLIB_PATH` when it finds one, in
/// `vendor/onnxruntime` above all. Without one, Windows loads its own older
/// copy from System32 and `ort` panics. `scripts/fetch-ort.sh` fetches the
/// right one.
fn onnx_runtime() -> bool {
    sourcelight_lib::develop::ai::ensure_ort_dylib();
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return true;
    }
    test_support::skip(
        Needs::Model,
        "no ONNX Runtime in vendor/onnxruntime (scripts/fetch-ort.sh) and no ORT_DYLIB_PATH",
    );
    false
}

/// The app's model directory, matching what the running app uses — or `None`
/// with the skip recorded, so every caller is counted (two used to `return`
/// without a word).
fn models_dir() -> Option<PathBuf> {
    // A development catalogue first (`SOURCELIGHT_DATA_DIR`), then where
    // the installed app keeps its data: Tauri's local data dir on each OS.
    let installed = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    };
    let dir = std::env::var_os("SOURCELIGHT_DATA_DIR")
        .filter(|d| !d.is_empty())
        .map(|data| PathBuf::from(data).join("models"))
        .or_else(|| installed.map(|d| d.join("app.sourcelight").join("models")));
    match dir {
        Some(dir) if dir.is_dir() => Some(dir),
        _ => {
            test_support::skip(Needs::Model, "no app model directory");
            None
        }
    }
}

/// A synthetic scene: a bright disc on a dark ground. Any segmentation model
/// should separate the two, which is all these tests assert — this is a wiring
/// check, not a quality benchmark.
fn disc(w: u32, h: u32) -> Vec<u8> {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let r = (w.min(h) as f32) * 0.3;
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let inside = d < r;
            let i = ((y * w + x) * 4) as usize;
            let v = if inside { 235 } else { 25 };
            px[i] = v;
            px[i + 1] = if inside { 210 } else { 30 };
            px[i + 2] = if inside { 60 } else { 35 };
            px[i + 3] = 255;
        }
    }
    px
}

/// Mean coverage over a centred box and over a corner box, as fractions.
fn centre_and_corner(cov: &[u8], w: u32, h: u32) -> (f32, f32) {
    let at = |x: u32, y: u32| cov[(y * w + x) as usize] as f32 / 255.0;
    let mut c = 0.0f32;
    let mut n = 0.0f32;
    for y in (h * 4 / 10)..(h * 6 / 10) {
        for x in (w * 4 / 10)..(w * 6 / 10) {
            c += at(x, y);
            n += 1.0;
        }
    }
    let mut k = 0.0f32;
    let mut m = 0.0f32;
    for y in 0..(h / 10).max(1) {
        for x in 0..(w / 10).max(1) {
            k += at(x, y);
            m += 1.0;
        }
    }
    (c / n.max(1.0), k / m.max(1.0))
}

/// **SAM2 click-to-select, against the real encoder + decoder.**
///
/// Clicking the middle of the disc must select the disc: high coverage at the
/// centre, near-zero in a corner. This is what proves the whole chain — point
/// mapping, the seven decoder inputs, output naming, candidate choice and the
/// logits-to-coverage conversion.
#[test]
fn sam2_selects_the_object_under_the_click() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::develop::sam2::{Point, Sam2};

    let Some(models) = models_dir() else {
        return;
    };
    let dir = models.join("sam2");
    if !dir.join("encoder.onnx").exists() {
        test_support::skip(
            Needs::Model,
            &format!("no SAM2 models in {}", dir.display()),
        );
        return;
    }

    let (w, h) = (512u32, 512u32);
    let image = disc(w, h);

    let started = std::time::Instant::now();
    let mut sam = Sam2::load(&dir).expect("loading SAM2");
    let load_ms = started.elapsed().as_millis();

    let started = std::time::Instant::now();
    let emb = sam.encode(&image, w, h).expect("encoding");
    let encode_ms = started.elapsed().as_millis();

    let started = std::time::Instant::now();
    let cov = sam
        .decode(
            &emb,
            &[Point {
                x: 0.5,
                y: 0.5,
                include: true,
            }],
            w,
            h,
        )
        .expect("decoding");
    let decode_ms = started.elapsed().as_millis();

    assert_eq!(cov.len(), (w * h) as usize);
    let (centre, corner) = centre_and_corner(&cov, w, h);
    eprintln!(
        "SAM2: load {load_ms}ms, encode {encode_ms}ms, decode {decode_ms}ms; \
         centre {centre:.3}, corner {corner:.3}"
    );

    assert!(
        centre > 0.7,
        "the clicked object should be selected, got {centre:.3}"
    );
    assert!(
        corner < 0.3,
        "the background should not be, got {corner:.3}"
    );

    // The decoder is the interactive half: a second click must be far cheaper
    // than the encode, which is the entire reason the embedding is cached.
    let started = std::time::Instant::now();
    let _ = sam
        .decode(
            &emb,
            &[
                Point {
                    x: 0.5,
                    y: 0.5,
                    include: true,
                },
                Point {
                    x: 0.05,
                    y: 0.05,
                    include: false,
                },
            ],
            w,
            h,
        )
        .expect("second decode");
    let again_ms = started.elapsed().as_millis();
    eprintln!("SAM2: refine decode {again_ms}ms");
    assert!(
        again_ms <= encode_ms.max(1),
        "refining ({again_ms}ms) should not cost more than encoding ({encode_ms}ms)"
    );
}

/// **BiRefNet Select Subject, against the real model.** The user has confirmed
/// this works in the app; this pins it so a refactor cannot silently break it.
#[test]
fn birefnet_separates_subject_from_background() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::develop::ai::{Segmenter, BIREFNET};

    let Some(models) = models_dir() else {
        return;
    };
    // In the precision this machine downloads: fp16 where DirectML runs it.
    // It looked for `model.onnx`, a name the app never downloads, so this
    // never ran.
    let Some(model) = sourcelight_lib::ai_models::best_file(&models, "birefnet.onnx") else {
        test_support::skip(
            Needs::Model,
            &format!("no BiRefNet model in {}", models.display()),
        );
        return;
    };

    let (w, h) = (512u32, 512u32);
    let started = std::time::Instant::now();
    let mut seg = Segmenter::load(&model, BIREFNET).expect("loading BiRefNet");
    let (cw, ch, cov) = seg.segment(&disc(w, h), w, h).expect("segmenting");
    eprintln!(
        "BiRefNet: {}ms, coverage {cw}x{ch}",
        started.elapsed().as_millis()
    );

    let (centre, corner) = centre_and_corner(&cov, cw, ch);
    eprintln!("BiRefNet: centre {centre:.3}, corner {corner:.3}");
    assert!(
        centre > corner + 0.3,
        "subject should stand out: {centre:.3} vs {corner:.3}"
    );
}

/// **LaMa, for magic erase.** A hole in the middle of a flat field is filled
/// with that field, the pixels around it come back as they went in, and the
/// answer is in 0..1 as `Inpainter::fill` promises.
#[test]
fn lama_fills_a_hole_with_what_surrounds_it() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::develop::erase::{Inpainter, ModelInput, MODEL_EDGE, MODEL_FILE};

    let Some(models) = models_dir() else {
        return;
    };
    if !models.join(MODEL_FILE).exists() {
        test_support::skip(
            Needs::Model,
            &format!("no LaMa model at {}", models.join(MODEL_FILE).display()),
        );
        return;
    }
    let m = MODEL_EDGE as usize;
    // Left half warm, right half cool; the hole is well inside the left.
    let warm = [0.8f32, 0.5, 0.3];
    let cool = [0.2f32, 0.4, 0.7];
    let mut rgb = vec![0.0f32; m * m * 3];
    let mut hole = vec![0.0f32; m * m];
    for y in 0..m {
        for x in 0..m {
            let k = y * m + x;
            rgb[k * 3..k * 3 + 3].copy_from_slice(if x < m / 2 { &warm } else { &cool });
            if (96..160).contains(&x) && (224..288).contains(&y) {
                hole[k] = 1.0;
                // What is under the hole must not matter.
                rgb[k * 3..k * 3 + 3].copy_from_slice(&[0.0, 1.0, 0.0]);
            }
        }
    }
    let input = ModelInput {
        rgb,
        hole,
        scale: 1.0,
    };
    let started = std::time::Instant::now();
    let lama = Inpainter::load(&models).expect("loading LaMa");
    let loaded = started.elapsed();
    let out = lama.fill(&input).expect("filling");
    eprintln!(
        "LaMa: loaded in {}ms, filled in {}ms",
        loaded.as_millis(),
        (started.elapsed() - loaded).as_millis()
    );
    assert_eq!(out.len(), m * m * 3);
    assert!(out.iter().all(|v| (0.0..=1.0).contains(v)));
    let at = |x: usize, y: usize| &out[(y * m + x) * 3..(y * m + x) * 3 + 3];
    for c in 0..3 {
        assert!(
            (at(128, 256)[c] - warm[c]).abs() < 0.06,
            "the hole is filled warm: {:?}",
            at(128, 256)
        );
        assert!(
            (at(400, 100)[c] - cool[c]).abs() < 0.01,
            "outside the hole is unchanged: {:?}",
            at(400, 100)
        );
    }
}

/// **The built-in CLIP tagger, against the real models.** Tags a synthetic
/// scene; asserts the pipeline produces a genre from the closed set and a
/// subject, which is what proves the tokenizer, both encoders and the
/// vocabulary embedding all line up.
#[test]
fn builtin_tagger_produces_tags() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::tagger::builtin::BuiltinTagger;
    use sourcelight_lib::tagger::Tagger;

    let Some(models) = models_dir() else {
        return;
    };
    let dir = models.join("clip");
    if !dir.join("vision_model.onnx").exists() {
        test_support::skip(
            Needs::Model,
            &format!("no CLIP models in {}", dir.display()),
        );
        return;
    }

    // The tagger takes an encoded image, as the library hands it previews.
    let (w, h) = (256u32, 256u32);
    let rgba = disc(w, h);
    let img = image::RgbaImage::from_raw(w, h, rgba).expect("image");
    let mut jpeg = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .to_rgb8()
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .expect("encoding jpeg");

    let started = std::time::Instant::now();
    let tagger = BuiltinTagger::load(&dir).expect("loading CLIP");
    let load_ms = started.elapsed().as_millis();

    let started = std::time::Instant::now();
    let result = pollster::block_on(tagger.tag(jpeg.get_ref())).expect("tagging");
    eprintln!(
        "CLIP: load {load_ms}ms (embeds the whole vocabulary), tag {}ms",
        started.elapsed().as_millis()
    );
    eprintln!(
        "CLIP: genre={} conf={:.2} subject={} subjects={:?} conditions={:?}",
        result.genre, result.confidence, result.main_subject, result.subjects, result.conditions
    );

    assert!(
        sourcelight_lib::models::GENRES.contains(&result.genre.as_str()),
        "genre must come from the closed set, got {:?}",
        result.genre
    );
    assert!(
        !result.main_subject.is_empty(),
        "a subject should be chosen"
    );
    // The bug this test caught: an absolute similarity threshold attached every
    // condition in the vocabulary to the photo.
    assert!(
        result.conditions.len() <= 2,
        "conditions must be a confident shortlist, got {:?}",
        result.conditions
    );
    assert!(
        result.subjects.len() <= 6,
        "subjects must stay a shortlist, got {:?}",
        result.subjects
    );
    assert!(
        result.confidence > 0.0 && result.confidence <= 1.0,
        "confidence should be a probability, got {}",
        result.confidence
    );
}

/// The search model through the app's own path — `BuiltinTagger`, its session
/// setup, preprocessing and tokenisation — against SigLIP 2 run in fp32 on the
/// CPU in Python (`tests/fixtures/siglip2_reference.json`, made by
/// `scripts/model-eval/`). This is the check that the fp16 model is *right*,
/// not just loaded: with the wrong graph optimisation on DirectML it loads
/// and runs and returns embeddings 0.72 away from these.
///
/// `SOURCELIGHT_TEST_SIGLIP` may point at a folder holding the three
/// files; otherwise the app's own `models/siglip2` is used.
#[test]
fn siglip2_matches_the_fp32_reference() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::tagger::builtin::{BuiltinTagger, MODEL_DIR};

    let dir = match std::env::var_os("SOURCELIGHT_TEST_SIGLIP") {
        Some(d) => PathBuf::from(d),
        None => {
            let Some(models) = models_dir() else {
                return;
            };
            models.join(MODEL_DIR)
        }
    };
    if !dir.join("vision_model.onnx").exists() {
        test_support::skip(
            Needs::Model,
            &format!("no SigLIP 2 model in {}", dir.display()),
        );
        return;
    }
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/siglip2_reference.json")).unwrap();
    let want = |v: &serde_json::Value| -> Vec<f32> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap() as f32)
            .collect()
    };
    let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();

    // The same synthetic scene the reference was made from: a red disc on blue.
    let (w, h) = (256u32, 256u32);
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as i32 - 128, y as i32 - 128);
        if dx * dx + dy * dy < 70 * 70 {
            image::Rgb([220, 30, 30])
        } else {
            image::Rgb([30, 60, 150])
        }
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let tagger = BuiltinTagger::load(&dir).expect("loading SigLIP 2");
    let got = tagger.embed_image(&png).expect("embedding the image");
    let image_cos = cos(&got, &want(&reference["image"]));
    // Rust resamples 256 -> 224 with a different filter than Python's, so not
    // bit-exact; a broken session lands near 0.7.
    assert!(
        image_cos > 0.99,
        "image embedding off the reference: cosine {image_cos}"
    );

    for (query, expected) in reference["text"].as_object().unwrap() {
        let got = tagger.embed_query(query).expect("embedding the query");
        let c = cos(&got, &want(expected));
        assert!(c > 0.999, "{query:?} off the reference: cosine {c}");
    }
    // And the model means what it says: the scene matches its description.
    let red = tagger
        .embed_query("a red circle on a blue background")
        .unwrap();
    let dog = tagger.embed_query("a dog").unwrap();
    assert!(cos(&got, &red) > cos(&got, &dog) + 0.1);
}

/// The People recogniser runs on the GPU and gives an identity per face.
///
/// It used to send a face and its mirror image as one batch of two. DirectML
/// rejects that at AuraFace's final BatchNormalization, so every embed on a
/// Windows GPU failed, the failure was swallowed, and People stayed empty. Any
/// image and any five points will do here: this checks the session runs, and
/// that a mirrored face is not a different person.
#[test]
fn face_embedder_runs_and_is_mirror_stable() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::faces::FaceEngine;

    let Some(models) = models_dir() else {
        return;
    };
    let dir = models.join("faces");
    if !dir.join("auraface.onnx").exists() {
        test_support::skip(
            Needs::Model,
            &format!("no People recogniser in {}", dir.display()),
        );
        return;
    }
    let engine = FaceEngine::load(&dir).expect("loading the face models");
    assert!(engine.can_embed());

    let (w, h) = (256u32, 256u32);
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    // Eyes, nose, mouth corners: roughly where the alignment template has them.
    let marks = [
        [96.0, 110.0],
        [160.0, 110.0],
        [128.0, 140.0],
        [104.0, 170.0],
        [152.0, 170.0],
    ];

    let got = engine
        .embed_all(&png, &[marks, marks])
        .expect("embedding two faces from one photo");
    assert_eq!(got.len(), 2);
    for v in &got {
        assert_eq!(v.len(), 512);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "not L2-normalised: {norm}");
    }
    // Same input, same identity.
    let cos: f32 = got[0].iter().zip(&got[1]).map(|(a, b)| a * b).sum();
    assert!(
        cos > 0.999,
        "the same face embedded twice differs: cosine {cos}"
    );
}

/// The eye models the installer carries load and run, and read a
/// frame with no face in it as "can't tell", never as shut. Runs against the
/// vendored copies, so it needs no download.
#[test]
fn the_eye_models_read_a_non_face_as_cant_tell() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::faces::FaceEngine;

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/models/faces");
    let engine = FaceEngine::load(&dir).expect("loading the vendored face models");
    assert!(engine.can_read_eyes());

    let img = image::RgbImage::from_fn(320, 240, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let got = engine
        .read_eyes(&png, &[[60.0, 40.0, 120.0, 120.0], [0.0, 0.0, 30.0, 30.0]])
        .expect("reading eyes");
    assert_eq!(got, vec![None, None]);
}

/// CoreML compiles a model once, into its cache, and loads it from there
/// afterwards. It used to compile into the temp dir for every session and
/// clean up only when the session was dropped, which the app never does: 214
/// folders, 4.4 GB, on the development Mac after two days.
#[cfg(target_os = "macos")]
#[test]
fn coreml_compiles_the_face_detector_once_and_leaves_nothing_in_temp() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use sourcelight_lib::develop::ai::coreml_cache_dir;
    use sourcelight_lib::faces::FaceEngine;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    fn find(dir: &Path, name: &str) -> Option<PathBuf> {
        for item in std::fs::read_dir(dir).ok()?.flatten() {
            let path = item.path();
            if item.file_name() == name {
                return Some(path);
            }
            if let Some(found) = path.is_dir().then(|| find(&path, name)).flatten() {
                return Some(found);
            }
        }
        None
    }
    // This process's folders in the temp dir, as the provider names them.
    let in_temp = || -> Vec<String> {
        let pid = format!("-{}-", std::process::id());
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .map(|item| item.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("onnxruntime-") && name.contains(&pid))
            .collect()
    };
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/models/faces");
    let cache = coreml_cache_dir(&dir.join("yunet.onnx")).unwrap();
    let compiled = || {
        let found = find(&cache, "compiled_model.mlmodelc")?;
        Some(std::fs::metadata(found).ok()?.ino())
    };

    let started = std::time::Instant::now();
    let first = FaceEngine::load(&dir).expect("loading the vendored face models");
    let first_load = started.elapsed();
    assert_eq!(
        in_temp(),
        Vec::<String>::new(),
        "CoreML compiled into the temp dir"
    );
    let inode = compiled().expect("YuNet's compile is not in the cache");
    drop(first);

    let started = std::time::Instant::now();
    let _second = FaceEngine::load(&dir).expect("loading them again");
    eprintln!(
        "face models loaded in {first_load:?}, then {:?}",
        started.elapsed()
    );
    assert_eq!(
        in_temp(),
        Vec::<String>::new(),
        "CoreML compiled into the temp dir"
    );
    assert_eq!(compiled(), Some(inode), "YuNet was compiled again");
}

/// Automatic analysis's models start their threads through the app's own
/// thread manager, which runs them below normal priority, and neither kind of
/// session spins (`develop::ai::cpu_threads`). The vendored face detector is
/// always there to try it on.
#[test]
fn background_sessions_start_their_threads_below_normal_priority() {
    let _gpu = one_at_a_time();
    if !onnx_runtime() {
        return;
    }
    use ort::execution_providers::CPUExecutionProvider;
    use sourcelight_lib::develop::ai::{
        background_threads_made, cpu_threads, ensure_ort_dylib, Workload,
    };
    let yunet = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/models/faces/yunet.onnx");
    ensure_ort_dylib();
    for workload in [Workload::Interactive, Workload::Background] {
        let before = background_threads_made();
        let Ok(builder) = ort::session::Session::builder()
            .and_then(|b| b.with_intra_threads(3))
            .and_then(|b| cpu_threads(b, workload))
        else {
            test_support::skip(Needs::Model, "ONNX Runtime did not load");
            return;
        };
        let mut session = builder
            .with_execution_providers([CPUExecutionProvider::default().build()])
            .unwrap()
            .commit_from_file(&yunet)
            .expect("loading the vendored face detector");
        let input =
            ort::value::Tensor::from_array(([1_i64, 3, 640, 640], vec![0f32; 3 * 640 * 640]))
                .unwrap();
        session.run(ort::inputs!["input" => input]).unwrap();
        let made = background_threads_made() - before;
        match workload {
            Workload::Interactive => assert_eq!(made, 0, "an interactive pool is ONNX Runtime's"),
            // Three intra-op threads are the caller and two in the pool.
            Workload::Background => assert!(made >= 2, "{made} threads from the manager"),
        }
    }
}
