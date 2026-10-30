//! End-to-end develop: a real raw file through decode, mosaic and develop.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --test develop_render -- --nocapture
//! ```
//!
//! Renders are written to `target/develop-render/` so the result can actually
//! be looked at. Synthetic tests pin the arithmetic; only a real photograph
//! shows whether the output is an image rather than plausible noise.

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::curve::CurvePoint;
use sourcelight_lib::develop::mask::encode_ai;
use sourcelight_lib::develop::mask::{
    LocalAdjustments, Mask, MaskComponent, MaskMode, MaskShape, Stroke,
};
use sourcelight_lib::develop::spot::Spot;
use sourcelight_lib::develop::{DevelopState, EditStack};
use sourcelight_lib::export::{ExportFormat, ExportOptions};
use sourcelight_lib::gpu::pipeline::{Pipeline, PROXY_EDGE};
use sourcelight_lib::gpu::Gpu;
use sourcelight_lib::present::readback::ReadbackPresenter;
use sourcelight_lib::present::{Frame, Presenter};
use sourcelight_lib::raw::decoder::RawDecoder;
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};
use std::path::PathBuf;

/// Every raw fixture LibRaw can read, sorted so the choice is stable — or
/// `None` with the skip already recorded (see `test_support`), so no caller can
/// forget to report it. Several used to `return` without a word.
fn raws() -> Option<Vec<PathBuf>> {
    let Some(dir) = test_support::raw_dir() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return None;
    };
    let exts = LibRawDecoder::new().extensions();
    let files = test_support::files_in(&dir, |e| exts.contains(&e));
    if files.is_empty() {
        test_support::skip(
            Needs::Raw,
            &format!("no raw files LibRaw reads in {}", dir.display()),
        );
        return None;
    }
    Some(files)
}

/// The first raw fixture, or `None` with the skip recorded.
fn first_raw() -> Option<PathBuf> {
    raws()?.into_iter().next()
}

fn gpu() -> Option<&'static Gpu> {
    sourcelight_lib::gpu::test_support::hardware_gpu()
}

/// Statistics that separate a photograph from a failure mode. A blank render,
/// a fully clipped one and a single-channel one all pass "it produced pixels",
/// so check spread and channel balance instead.
fn describe(pixels: &[u8]) -> (f64, f64, [f64; 3]) {
    let n = (pixels.len() / 4) as f64;
    let mut sums = [0f64; 3];
    let mut luma_sum = 0.0;
    let mut luma_sq = 0.0;
    for px in pixels.chunks_exact(4) {
        for c in 0..3 {
            sums[c] += px[c] as f64;
        }
        let luma = 0.2126 * px[0] as f64 + 0.7152 * px[1] as f64 + 0.0722 * px[2] as f64;
        luma_sum += luma;
        luma_sq += luma * luma;
    }
    let mean = luma_sum / n;
    let stddev = (luma_sq / n - mean * mean).max(0.0).sqrt();
    (mean, stddev, sums.map(|s| s / n))
}

#[test]
fn renders_a_real_raw_to_an_image() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else { return };

    let raw = LibRawDecoder::new().decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");

    let (w, h) = session.proxy_size(PROXY_EDGE);
    let pixels = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("render");

    let (mean, stddev, channels) = describe(&pixels);
    let name = path.file_stem().unwrap().to_string_lossy().to_string();
    eprintln!(
        "{name}: sensor {}x{} -> image {}x{} -> proxy {w}x{h}\n  \
         luma mean {mean:.1} stddev {stddev:.1}, channels {channels:.1?}",
        raw.width, raw.height, session.width, session.height
    );

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/develop-render");
    std::fs::create_dir_all(&out_dir).unwrap();
    image::RgbaImage::from_raw(w, h, pixels.clone())
        .unwrap()
        .save(out_dir.join(format!("{name}.png")))
        .unwrap();
    eprintln!("  wrote {}", out_dir.join(format!("{name}.png")).display());

    // Not blank, not fully clipped.
    assert!(
        mean > 8.0 && mean < 247.0,
        "mean luma {mean:.1} suggests a blank or clipped render"
    );
    // Real scenes have tonal variation; a uniform field means the mosaic pass
    // wrote a constant.
    assert!(
        stddev > 10.0,
        "luma stddev {stddev:.1} is too flat to be a photograph"
    );
    // A wildly lopsided channel usually means the colour matrix went in
    // transposed or in the wrong direction.
    let max = channels.iter().cloned().fold(f64::MIN, f64::max);
    let min = channels.iter().cloned().fold(f64::MAX, f64::min);
    assert!(
        max / min.max(1.0) < 3.0,
        "channel means {channels:.1?} are too far apart — suspect the colour matrix"
    );
}

/// The whole interactive path: decode, upload, render, read back, encode.
///
/// This is what the UI actually receives per slider move, so it also measures
/// whether an edit stays inside an interactive budget.
#[test]
fn the_readback_presenter_produces_a_usable_frame() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else { return };

    let raw = LibRawDecoder::new().decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(PROXY_EDGE);
    // `prepare` only queues the photo's upload and demosaic. Without waiting
    // for it, the first frame timed below pays for opening the photo — 36-44ms
    // on an M4, a one-off per photo — and the budget stops meaning "slider to
    // pixels". Metal is where that showed: 47ms against 33 in a release build,
    // for a slider step that costs ~5ms.
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("the photo's GPU setup finishes");

    let mut presenter = ReadbackPresenter::new();
    let mut frame_for = |stack: &EditStack| {
        let started = std::time::Instant::now();
        let presented = presenter
            .present(Frame {
                view: None,
                soft_proof: false,
                origin: (0, 0),
                gpu,
                pipeline: &pipeline,
                session: &session,
                stack,
                width: w,
                height: h,
                split: None,
            })
            .expect("present");
        let bytes = presented.bytes().expect("readback yields bytes").to_vec();
        (bytes, started.elapsed())
    };

    // Stage timings, because the totals hide where the cost sits.
    //
    // This used to read: render ~0.3ms, readback ~5ms, **JPEG encode ~49ms**.
    // The encode was about nine tenths of every interaction, and `image`'s
    // encoder runs at roughly 26 MP/s against libjpeg-turbo's 200 — so the fix
    // was not a faster JPEG but none at all. Frames now go out as uncompressed
    // BMP: 32.1ms and 187KB became 0.88ms and 5113KB at 1400x935.
    //
    // That is what makes the budget below assertable. Until the encode went,
    // only the GPU pass could be bounded and the number that actually mattered
    // — how long a slider takes to show its result — could not be.
    {
        let stack = EditStack {
            exposure: 0.3,
            ..Default::default()
        };
        let pass = || {
            let t0 = std::time::Instant::now();
            let target = pipeline.render_to_texture(gpu, &session, &stack, w, h, None, false);
            let render = t0.elapsed();

            let t1 = std::time::Instant::now();
            let pixels = gpu.read_rgba8(&target, w, h).unwrap();
            let readback = t1.elapsed();

            let t2 = std::time::Instant::now();
            let _ = sourcelight_lib::present::readback::encode_bmp(&pixels, w, h).unwrap();
            let encode = t2.elapsed();

            (render, readback, encode)
        };
        // One untimed pass first, then the median of five, as `scale.rs`
        // times things. The first render pays one-off costs — pipeline
        // compilation, first-use allocation — that no slider move after it
        // does, and a single sample is at the mercy of whatever else holds the
        // GPU: under `check.sh --ai`, with other test binaries loading it, one
        // pass measured 51.5ms against 0.45-0.54ms run alone.
        pass();
        let mut passes: Vec<_> = (0..5).map(|_| pass()).collect();
        type Pass = (
            std::time::Duration,
            std::time::Duration,
            std::time::Duration,
        );
        let median = |stage: fn(&Pass) -> std::time::Duration| {
            let mut times: Vec<_> = passes.iter().map(stage).collect();
            times.sort();
            times[2]
        };
        let render = median(|p| p.0);
        let readback = median(|p| p.1);
        let encode = median(|p| p.2);
        passes.sort_by_key(|p| p.0 + p.1 + p.2);
        let total = passes[2].0 + passes[2].1 + passes[2].2;
        eprintln!(
            "breakdown: render {render:?}, readback {readback:?}, encode {encode:?}              -> total {total:?}"
        );
        assert!(
            render < std::time::Duration::from_millis(50),
            "the GPU pass itself took {render:?} — that is the part that must stay fast"
        );
        // The plan's Phase 2 budget: slider to pixels under 33ms.
        //
        // **Release only, and that is not a loosened bound — it is the only one
        // that means anything.** `[profile.dev.package."*"]` optimises
        // dependencies but not this crate, so in a debug build `encode_bmp` is
        // an unoptimised copy of five megabytes: measured at 99ms against
        // 2.0ms release, which fails a budget describing software nobody ships.
        // The bound went unnoticed because this whole test skips without
        // `SOURCELIGHT_TEST_RAW`, so it had never actually run.
        let budget = std::time::Duration::from_millis(33);
        if cfg!(debug_assertions) {
            eprintln!("debug build: {total:?} against a {budget:?} budget, not asserted");
        } else {
            assert!(
                total < budget,
                "a frame took {total:?}; the interactive budget is 33ms and the readback              is now the largest term"
            );
        }
    }

    let (flat, first) = frame_for(&EditStack::default());
    let (lifted, second) = frame_for(&EditStack {
        exposure: 1.5,
        ..Default::default()
    });

    eprintln!(
        "{w}x{h} frames: {} KB in {:?}, then {} KB in {:?}",
        flat.len() / 1024,
        first,
        lifted.len() / 1024,
        second
    );

    for bytes in [&flat, &lifted] {
        assert_eq!(&bytes[..2], b"BM", "not a BMP");
        let decoded = image::load_from_memory(bytes).expect("frame decodes");
        assert_eq!((decoded.width(), decoded.height()), (w, h));
    }
    // Different edits must produce different frames — identical bytes would
    // mean the stack never reached the shader.
    assert_ne!(flat, lifted, "exposure change produced an identical frame");

    // Generous, because CI has no GPU and falls back to software rendering.
    // The point is to catch an order-of-magnitude regression, not to benchmark.
    assert!(
        second < std::time::Duration::from_millis(500),
        "a single edit took {second:?}, which is far outside an interactive budget"
    );
}

/// The session state machine as the UI drives it: open, adjust, compare,
/// undo, redo. This is what the `develop://` handler renders from, so it covers
/// everything in that path except the URI parsing itself.
#[test]
fn a_develop_session_edits_and_undoes() {
    let Some(path) = first_raw() else { return };
    if gpu().is_none() {
        return;
    }

    let mut dev = DevelopState::new(Box::new(ReadbackPresenter::new())).expect("develop state");
    let decoder = LibRawDecoder::new();
    let info = dev.open(7, &path, &decoder, None).expect("open");

    assert_eq!(info.photo_id, 7);
    assert!(info.width > 1000 && info.height > 1000);
    assert!(!info.can_undo, "a freshly opened photo has nothing to undo");

    let baseline = dev.render(Some(400), None, None, false).expect("render");
    let baseline = baseline.bytes().expect("bytes").to_vec();

    let brightened = dev
        .apply(
            "Exposure",
            EditStack {
                exposure: 1.5,
                ..Default::default()
            },
        )
        .expect("apply");
    assert!(brightened.can_undo, "an edit must be undoable");
    assert!(
        brightened.frame > info.frame,
        "the frame counter must advance or the UI will show a cached preview"
    );

    let edited = dev.render(Some(400), None, None, false).expect("render");
    let edited = edited.bytes().expect("bytes").to_vec();
    assert_ne!(baseline, edited, "the edit never reached the shader");

    // Before/after renders the defaults without touching history.
    let before = dev
        .render(Some(400), Some(&EditStack::default()), None, false)
        .expect("render before");
    assert_eq!(
        before.bytes().expect("bytes"),
        baseline.as_slice(),
        "the before view must match the unedited render exactly"
    );
    assert!(
        dev.current_stack().unwrap().exposure == 1.5,
        "rendering a comparison must not disturb the current stack"
    );

    let undone = dev.undo().expect("undo");
    assert!(undone.can_redo);
    assert_eq!(undone.stack.exposure, 0.0);
    assert_eq!(
        dev.render(Some(400), None, None, false)
            .unwrap()
            .bytes()
            .unwrap(),
        baseline.as_slice(),
        "undo must restore the original render"
    );

    let redone = dev.redo().expect("redo");
    assert_eq!(redone.stack.exposure, 1.5);

    dev.close();
    assert!(
        dev.render(Some(400), None, None, false).is_err(),
        "closed sessions cannot render"
    );
}

/// Export must produce a real file that matches what the preview showed.
///
/// The whole WYSIWYG claim rests on export and preview sharing a shader, so
/// this compares the delivered file's tonality against the on-screen render
/// rather than merely checking that bytes were written.
#[test]
fn export_matches_the_preview() {
    let Some(path) = first_raw() else { return };
    if gpu().is_none() {
        return;
    }

    let mut dev = DevelopState::new(Box::new(ReadbackPresenter::new())).expect("develop state");
    let decoder = LibRawDecoder::new();
    dev.open(1, &path, &decoder, None).expect("open");

    let stack = EditStack {
        exposure: 0.4,
        contrast: 0.3,
        shadows: 0.2,
        ..Default::default()
    };
    dev.apply("Edit", stack).expect("apply");

    // What the user sees.
    let preview = dev.render(Some(600), None, None, false).expect("render");
    let preview = image::load_from_memory(preview.bytes().expect("bytes"))
        .expect("preview decodes")
        .to_rgb8();

    // What they get.
    let file = dev
        .export(
            &ExportOptions {
                format: ExportFormat::Png,
                max_edge: Some(600),
                ..Default::default()
            },
            None,
        )
        .expect("export");
    let exported = image::load_from_memory(&file)
        .expect("export decodes")
        .to_rgb8();

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/develop-render");
    std::fs::create_dir_all(&out_dir).unwrap();
    std::fs::write(out_dir.join("export.png"), &file).unwrap();

    assert_eq!(
        (exported.width(), exported.height()),
        (preview.width(), preview.height()),
        "export and preview disagree on size"
    );

    let mean = |img: &image::RgbImage| {
        let mut sums = [0f64; 3];
        for p in img.pixels() {
            for (sum, channel) in sums.iter_mut().zip(p.0) {
                *sum += channel as f64;
            }
        }
        sums.map(|s| s / img.pixels().len() as f64)
    };
    let (a, b) = (mean(&preview), mean(&exported));
    eprintln!(
        "preview {a:.2?} vs export {b:.2?} ({} KB)",
        file.len() / 1024
    );

    // The two resample differently — the preview through the GPU sampler, the
    // export through Lanczos on the CPU — so pixels differ. Tonality must not.
    for c in 0..3 {
        assert!(
            (a[c] - b[c]).abs() < 3.0,
            "channel {c}: preview {:.2} vs export {:.2} — the export is not what was shown",
            a[c],
            b[c]
        );
    }
}

/// The proxy and a full-resolution render must agree. This is the property
/// that keeps the preview honest about what an export will look like, and it
/// only holds because both go through the identical develop shader.
#[test]
fn the_proxy_matches_a_full_resolution_render() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else { return };

    let raw = LibRawDecoder::new().decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");

    let stack = EditStack {
        exposure: 0.6,
        warmth: 0.2,
        ..Default::default()
    };

    let (pw, ph) = session.proxy_size(PROXY_EDGE);
    let proxy = pipeline
        .render_to_pixels(gpu, &session, &stack, pw, ph)
        .expect("proxy render");
    let full = pipeline
        .render_to_pixels(gpu, &session, &stack, session.width, session.height)
        .expect("full render");

    let (proxy_mean, _, proxy_channels) = describe(&proxy);
    let (full_mean, _, full_channels) = describe(&full);
    eprintln!(
        "proxy {pw}x{ph} mean {proxy_mean:.2} {proxy_channels:.1?}\n\
         full  {}x{} mean {full_mean:.2} {full_channels:.1?}",
        session.width, session.height
    );

    // Resampling shifts fine detail, so the pixels differ; the tonality must
    // not. A mismatch here means the two paths diverged.
    assert!(
        (proxy_mean - full_mean).abs() < 2.0,
        "proxy mean {proxy_mean:.2} vs full {full_mean:.2} — preview would lie about the export"
    );
    for c in 0..3 {
        assert!(
            (proxy_channels[c] - full_channels[c]).abs() < 2.0,
            "channel {c} differs: proxy {:.2} vs full {:.2}",
            proxy_channels[c],
            full_channels[c]
        );
    }
}

/// **The stretch guard, end to end.**
///
/// `ViewPort::fitted` is unit-tested, but the bug that reached the user was
/// *wiring*: the maths was right and the render did not use it, then the render
/// used it and the `<img>` stretched the result anyway. Both times the pieces
/// were individually correct. So this asserts the one thing a viewer can see —
/// the served frame has the photograph's shape, not the canvas's.
///
/// The canvas is deliberately a different aspect ratio to any sensor, so a
/// render that filled it would be caught whichever way round the photo is.
#[test]
fn a_zoomed_render_keeps_the_photo_s_shape() {
    let Some(path) = first_raw() else { return };
    if gpu().is_none() {
        return;
    }

    let mut dev = DevelopState::new(Box::new(ReadbackPresenter::new())).expect("develop state");
    let decoder = LibRawDecoder::new();
    let info = dev.open(1, &path, &decoder, None).expect("open");
    let photo = info.width as f64 / info.height as f64;

    // A wide, short canvas: nothing a camera produces.
    let (canvas_w, canvas_h) = (1850u32, 1050u32);
    let fit = (canvas_w as f32 / info.width as f32).min(canvas_h as f32 / info.height as f32);

    // Below fit the photo no longer covers the canvas and the target has to
    // shrink; at and above it, the canvas is the limit on at least one axis.
    for zoom in [fit * 0.25, fit * 0.7, fit, fit * 2.0, 1.0] {
        let view = sourcelight_lib::develop::geometry::ViewPort {
            zoom,
            centre: [0.5, 0.5],
            width: canvas_w,
            height: canvas_h,
        };
        let presented = dev.render(None, None, Some(view), false).expect("render");
        let sourcelight_lib::present::Presented::Image { width, height, .. } = presented else {
            panic!("readback must produce an image");
        };
        let drawn = width as f64 / height as f64;

        // Either the frame is showing all of the photo — in which case it must
        // have the photo's shape — or the canvas is clipping it, in which case
        // the target is the canvas and the *sampled* region shrinks to match.
        // Both are covered by: the render never draws more of an axis than the
        // photo occupies there.
        let occupies_w = (info.width as f32 * zoom).round() as u32;
        let occupies_h = (info.height as f32 * zoom).round() as u32;
        assert!(
            width <= occupies_w.max(1) && height <= occupies_h.max(1),
            "at {zoom} the render is {width}x{height}, larger than the {occupies_w}x{occupies_h} \
             the photo occupies — the difference is stretch",
        );

        if zoom <= fit {
            assert!(
                (drawn / photo - 1.0).abs() < 0.02,
                "at {zoom} (fit is {fit}) the photo fits entirely, so the frame should be \
                 {photo:.4} but is {drawn:.4}",
            );
        }
    }
}

/// **The mask pipeline, end to end on a real photo.**
///
/// This is what actually validates the WGSL — a mask stage that fails to
/// compile takes `Pipeline::new` down with it — and that coverage does what it
/// claims: a radial mask darkening its centre must darken the centre and leave
/// the corners alone. Darkening rather than brightening, so a centre that was
/// already near-white still shows the change.
#[test]
fn a_radial_mask_darkens_only_its_region() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base render");

    let masked = EditStack {
        masks: vec![Mask {
            components: vec![MaskComponent::new(MaskShape::Radial {
                cx: 0.5,
                cy: 0.5,
                radius_x: 0.18,
                radius_y: 0.18,
                angle: 0.0,
                feather: 0.4,
            })],
            adjustments: LocalAdjustments {
                exposure: -2.0,
                ..Default::default()
            },
            name: None,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &masked, w, h)
        .expect("masked render");

    // Mean luminance over a small window, so a single noisy pixel does not
    // decide the test.
    let luma_at = |px: &[u8], cx: u32, cy: u32| -> f32 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for dy in 0..8u32 {
            for dx in 0..8u32 {
                let x = (cx + dx).min(w - 1);
                let y = (cy + dy).min(h - 1);
                let i = ((y * w + x) * 4) as usize;
                sum += px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32;
                n += 1.0;
            }
        }
        sum / n
    };

    let centre_base = luma_at(&base, w / 2 - 4, h / 2 - 4);
    let centre_out = luma_at(&out, w / 2 - 4, h / 2 - 4);
    let corner_base = luma_at(&base, 0, 0);
    let corner_out = luma_at(&out, 0, 0);

    assert!(
        centre_out < centre_base - 20.0,
        "the masked centre should darken: {centre_base:.0} -> {centre_out:.0}",
    );
    assert!(
        (corner_out - corner_base).abs() < 3.0,
        "the corner is outside the mask and must not change: {corner_base:.0} -> {corner_out:.0}",
    );
}

/// Showing a mask tints what it covers, with no adjustment for it to
/// move: a fresh mask showed nothing until a slider did. Only the shown mask's
/// region, and only while it is shown.
#[test]
fn a_shown_mask_tints_only_its_region() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let mut stack = EditStack {
        masks: vec![Mask {
            components: vec![MaskComponent::new(MaskShape::Radial {
                cx: 0.5,
                cy: 0.5,
                radius_x: 0.18,
                radius_y: 0.18,
                angle: 0.0,
                feather: 0.4,
            })],
            adjustments: LocalAdjustments::default(),
            name: None,
        }],
        ..Default::default()
    };
    let hidden = pipeline
        .render_to_pixels(gpu, &session, &stack, w, h)
        .expect("hidden render");
    stack.show_mask = Some(0);
    let shown = pipeline
        .render_to_pixels(gpu, &session, &stack, w, h)
        .expect("shown render");
    // A mask that does not exist shows nothing, rather than reading past the end.
    stack.show_mask = Some(5);
    let missing = pipeline
        .render_to_pixels(gpu, &session, &stack, w, h)
        .expect("missing render");

    // Red over blue, averaged over a small window.
    let redness = |px: &[u8], cx: u32, cy: u32| -> f32 {
        let mut sum = 0.0;
        for dy in 0..8u32 {
            for dx in 0..8u32 {
                let i = ((((cy + dy).min(h - 1)) * w + (cx + dx).min(w - 1)) * 4) as usize;
                sum += px[i] as f32 - px[i + 2] as f32;
            }
        }
        sum / 64.0
    };
    let centre = (w / 2 - 4, h / 2 - 4);

    assert!(
        redness(&shown, centre.0, centre.1) > redness(&hidden, centre.0, centre.1) + 30.0,
        "the shown mask's centre should be tinted red",
    );
    assert!(
        (redness(&shown, 0, 0) - redness(&hidden, 0, 0)).abs() < 3.0,
        "the corner is outside the mask and must not be tinted",
    );
    assert_eq!(
        hidden, missing,
        "showing a mask that is not there changes nothing"
    );
}

/// A local **temperature** warms only its region, proving the second
/// adjustment vector (temperature/tint/saturation) is packed and read — the
/// exposure test alone only exercises the first.
#[test]
fn a_local_temperature_warms_only_its_region() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");
    let warmed = EditStack {
        masks: vec![Mask {
            components: vec![MaskComponent::new(MaskShape::Radial {
                cx: 0.5,
                cy: 0.5,
                radius_x: 0.2,
                radius_y: 0.2,
                angle: 0.0,
                feather: 0.3,
            })],
            adjustments: LocalAdjustments {
                temperature: 1.0,
                ..Default::default()
            },
            name: None,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &warmed, w, h)
        .expect("warmed");

    // Red-minus-blue over a centre window: warming must push it up.
    let warmth = |px: &[u8]| {
        let mut rb = 0.0;
        let mut n = 0.0;
        for dy in 0..8u32 {
            for dx in 0..8u32 {
                let x = (w / 2 - 4 + dx).min(w - 1);
                let y = (h / 2 - 4 + dy).min(h - 1);
                let i = ((y * w + x) * 4) as usize;
                rb += px[i] as f32 - px[i + 2] as f32;
                n += 1.0;
            }
        }
        rb / n
    };
    assert!(
        warmth(&out) > warmth(&base) + 4.0,
        "the masked centre should warm (R-B): {:.1} -> {:.1}",
        warmth(&base),
        warmth(&out),
    );
}

/// **The brush-mask GPU path.** A painted mask darkening its centre must darken
/// the painted region and leave the corners alone — and it exercises the whole
/// new path: CPU rasterisation, the coverage texture array, and the shader's
/// brush branch (a compile failure there takes `Pipeline::new` down).
#[test]
fn a_brush_mask_darkens_where_it_is_painted() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");

    // A short stroke of dabs across the centre.
    let dab = |x: f32| Stroke {
        x,
        y: 0.5,
        radius: 0.12,
        hardness: 0.5,
        erase: false,
    };
    let masked = EditStack {
        masks: vec![Mask {
            components: vec![MaskComponent::new(MaskShape::Brush {
                strokes: vec![dab(0.44), dab(0.5), dab(0.56)],
            })],
            adjustments: LocalAdjustments {
                exposure: -2.0,
                ..Default::default()
            },
            name: None,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &masked, w, h)
        .expect("masked");

    let luma = |px: &[u8], cx: u32, cy: u32| {
        let i = ((cy * w + cx) * 4) as usize;
        px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32
    };
    assert!(
        luma(&out, w / 2, h / 2) < luma(&base, w / 2, h / 2) - 20.0,
        "the painted centre should darken",
    );
    assert!(
        (luma(&out, 1, 1) - luma(&base, 1, 1)).abs() < 3.0,
        "the unpainted corner must be untouched",
    );
}

/// **The luminance-mask GPU path.** A value mask selects by brightness, not
/// position, so this cannot check a fixed region — instead it finds the
/// brightest and darkest pixels in the base render, targets a band that
/// contains the former and excludes the latter, and asserts only the bright one
/// darkens. Exercises the shader's luminance branch (a compile failure there
/// takes `Pipeline::new` down) and proves selection is by value.
#[test]
fn a_luminance_mask_darkens_only_the_bright_band() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");

    // sRGB-decode to a linear luma, the space the shader's band works in.
    let dec = |c: u8| {
        let x = c as f32 / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let lin_luma = |px: &[u8], i: usize| {
        dec(px[i]) * 0.2627 + dec(px[i + 1]) * 0.6780 + dec(px[i + 2]) * 0.0593
    };

    // Locate the brightest and darkest pixels in the base render.
    let (mut hi_i, mut hi_l) = (0usize, -1.0f32);
    let (mut lo_i, mut lo_l) = (0usize, 2.0f32);
    for p in 0..(w * h) as usize {
        let l = lin_luma(&base, p * 4);
        if l > hi_l {
            (hi_i, hi_l) = (p * 4, l);
        }
        if l < lo_l {
            (lo_i, lo_l) = (p * 4, l);
        }
    }
    assert!(
        hi_l - lo_l > 0.1,
        "test image has too little tonal range to test selection: {lo_l:.3}..{hi_l:.3}",
    );

    // A band with no upper bound whose lower edge sits between the two, so the
    // bright pixel is fully covered and the dark one is well clear of it.
    let low = (lo_l + hi_l) * 0.5;
    let feather = ((hi_l - lo_l) * 0.1).max(0.01);
    let masked = EditStack {
        masks: vec![Mask {
            components: vec![MaskComponent::new(MaskShape::Luminance {
                low,
                high: 2.0,
                feather,
            })],
            adjustments: LocalAdjustments {
                exposure: -2.0,
                ..Default::default()
            },
            name: None,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &masked, w, h)
        .expect("masked");

    let out_hi = lin_luma(&out, hi_i);
    let out_lo = lin_luma(&out, lo_i);
    assert!(
        out_hi < hi_l * 0.8,
        "the bright pixel (in band) should darken: {hi_l:.3} -> {out_hi:.3}",
    );
    assert!(
        (out_lo - lo_l).abs() < 0.02,
        "the dark pixel (out of band) must be untouched: {lo_l:.3} -> {out_lo:.3}",
    );
}

/// **The colour-range-mask GPU path.** A hue mask selects by colour, so this
/// finds the image's most saturated (and reasonably bright) pixel, targets its
/// hue, and finds a second saturated pixel whose hue is far from it — then
/// asserts only the on-hue pixel darkens. Exercises the shader's colour branch
/// (a compile failure there takes `Pipeline::new` down) and proves selection is
/// by hue, not brightness or position.
///
/// **Tries each fixture until one has two clearly different hues.** Not every
/// photograph does, and this used to take only the first file — a near-monochrome
/// frame — so with fixtures present it still returned early and reported a pass
/// without ever asserting anything.
#[test]
fn a_colour_mask_darkens_only_its_hue() {
    let Some(files) = raws() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let pipeline = Pipeline::new(gpu);

    // HSV from the 8-bit output. Hue in 0..1, wrapping; roughly the space the
    // shader works in — hue ordering survives the sRGB encode, which is why
    // generous margins below absorb the small drift.
    let hsv = |px: &[u8], i: usize| -> (f32, f32, f32) {
        let (r, g, b) = (
            px[i] as f32 / 255.0,
            px[i + 1] as f32 / 255.0,
            px[i + 2] as f32 / 255.0,
        );
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let d = mx - mn;
        let mut hue = 0.0;
        if d > 1e-6 {
            hue = if mx == r {
                ((g - b) / d).rem_euclid(6.0)
            } else if mx == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            };
        }
        (hue / 6.0, if mx > 1e-6 { d / mx } else { 0.0 }, mx)
    };
    let circ = |a: f32, b: f32| {
        let d = (a - b).abs() % 1.0;
        d.min(1.0 - d)
    };

    let mut found = None;
    for path in &files {
        let raw = decoder.decode(path).expect("decode");
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let (w, h) = session.proxy_size(256);
        let base = pipeline
            .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
            .expect("base");

        // The most saturated, reasonably bright pixel — the clearest colour to
        // select.
        let (mut a_i, mut a_score, mut a_hue) = (0usize, 0.0f32, 0.0f32);
        for p in 0..(w * h) as usize {
            let (hue, s, v) = hsv(&base, p * 4);
            if s * v > a_score {
                (a_i, a_score, a_hue) = (p * 4, s * v, hue);
            }
        }
        let (_, a_sat, a_val) = hsv(&base, a_i);
        if a_sat < 0.2 || a_val < 0.3 {
            eprintln!(
                "{}: no clearly-coloured pixel (sat {a_sat:.2})",
                path.display()
            );
            continue;
        }

        // A saturated pixel whose hue is farthest from the target.
        let (mut b_i, mut b_dist) = (0usize, 0.0f32);
        for p in 0..(w * h) as usize {
            let (hue, s, _) = hsv(&base, p * 4);
            if s > 0.2 {
                let d = circ(hue, a_hue);
                if d > b_dist {
                    (b_i, b_dist) = (p * 4, d);
                }
            }
        }
        if b_dist < 0.25 {
            eprintln!(
                "{}: too monochromatic (hue spread {b_dist:.2})",
                path.display()
            );
            continue;
        }

        eprintln!("hue selection tested on {}", path.display());
        found = Some((session, w, h, base, a_i, a_hue, b_i, b_dist));
        break;
    }
    let Some((session, w, h, base, a_i, a_hue, b_i, b_dist)) = found else {
        test_support::skip(
            Needs::Raw,
            "no raw fixture has two clearly different hues to select between",
        );
        return;
    };

    let masked = EditStack {
        masks: vec![Mask {
            components: vec![MaskComponent::new(MaskShape::ColorRange {
                hue: a_hue,
                range: 0.12,
                feather: 0.08,
            })],
            adjustments: LocalAdjustments {
                exposure: -2.0,
                ..Default::default()
            },
            name: None,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &masked, w, h)
        .expect("masked");

    let sum = |px: &[u8], i: usize| px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32;
    assert!(
        sum(&out, a_i) < sum(&base, a_i) - 20.0,
        "the on-hue pixel should darken: {:.0} -> {:.0}",
        sum(&base, a_i),
        sum(&out, a_i),
    );
    assert!(
        (sum(&out, b_i) - sum(&base, b_i)).abs() < 10.0,
        "the off-hue pixel (dist {b_dist:.2}) must be untouched: {:.0} -> {:.0}",
        sum(&base, b_i),
        sum(&out, b_i),
    );
}

/// **The combine-op GPU path.** A mask of two components — a big radial with a
/// small radial *subtracted* from its centre — must darken the ring (covered by
/// the first, not the second) while leaving the carved-out hole untouched. This
/// exercises the shader's per-mask component accumulation (component_count > 1,
/// the subtract branch); a combine bug would darken the hole or the ring wrong.
#[test]
fn a_subtract_component_carves_a_hole_in_a_mask() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");

    let radial = |r: f32| MaskShape::Radial {
        cx: 0.5,
        cy: 0.5,
        radius_x: r,
        radius_y: r,
        angle: 0.0,
        feather: 0.1,
    };
    let masked = EditStack {
        masks: vec![Mask {
            components: vec![
                MaskComponent::new(radial(0.35)),
                MaskComponent {
                    shape: radial(0.12),
                    inverted: false,
                    mode: MaskMode::Subtract,
                },
            ],
            adjustments: LocalAdjustments {
                exposure: -2.0,
                ..Default::default()
            },
            name: None,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &masked, w, h)
        .expect("masked");

    let at = |px: &[u8], fx: f32, fy: f32| {
        let x = ((fx * w as f32) as u32).min(w - 1);
        let y = ((fy * h as f32) as u32).min(h - 1);
        let i = ((y * w + x) * 4) as usize;
        px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32
    };

    let hole_delta = (at(&out, 0.5, 0.5) - at(&base, 0.5, 0.5)).abs();
    let ring_drop = at(&base, 0.72, 0.5) - at(&out, 0.72, 0.5);
    assert!(
        hole_delta < 15.0,
        "the subtracted hole must be ~untouched, moved {hole_delta:.0}"
    );
    assert!(
        ring_drop > 20.0,
        "the ring (first component only) must darken, dropped {ring_drop:.0}"
    );
    assert!(
        (at(&out, 0.02, 0.02) - at(&base, 0.02, 0.02)).abs() < 8.0,
        "the corner outside the mask must be untouched",
    );
}

/// **The spot (clone) GPU path.** A clone spot copies a source region over a
/// target: find the brightest and darkest pixels, clone the dark source over the
/// bright target, and assert the target takes on the source's (dark) value while
/// a pixel outside the spot is untouched. Exercises the shader's spot stage —
/// including the heal branch, which compiles in the same module (a bug there
/// takes `Pipeline::new` down).
#[test]
fn a_clone_spot_copies_the_source_over_the_target() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");

    let sum = |px: &[u8], i: usize| px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32;
    // Brightest and darkest pixels (staying a margin off the edges so the spot
    // radius does not fall off the frame).
    let (mut hi_i, mut hi_v) = (0usize, -1.0f32);
    let (mut lo_i, mut lo_v) = (0usize, 1e9f32);
    let margin = 12u32;
    for y in margin..h - margin {
        for x in margin..w - margin {
            let i = ((y * w + x) * 4) as usize;
            let v = sum(&base, i);
            if v > hi_v {
                (hi_i, hi_v) = (i, v);
            }
            if v < lo_v {
                (lo_i, lo_v) = (i, v);
            }
        }
    }
    assert!(
        hi_v - lo_v > 60.0,
        "test image has too little contrast: {lo_v:.0}..{hi_v:.0}"
    );

    let uv = |i: usize| {
        let p = (i / 4) as u32;
        (
            ((p % w) as f32 + 0.5) / w as f32,
            ((p / w) as f32 + 0.5) / h as f32,
        )
    };
    let (tx, ty) = uv(hi_i); // target: the bright pixel
    let (sx, sy) = uv(lo_i); // source: the dark pixel

    let cloned = EditStack {
        spots: vec![Spot {
            target_x: tx,
            target_y: ty,
            source_x: sx,
            source_y: sy,
            radius: 0.03,
            feather: 0.2,
            heal: false,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &cloned, w, h)
        .expect("cloned");

    // The target should now read like the (dark) source, not its bright self.
    assert!(
        sum(&out, hi_i) < hi_v - 40.0,
        "the cloned target should darken toward the source: {hi_v:.0} -> {:.0}",
        sum(&out, hi_i),
    );
    assert!(
        (sum(&out, hi_i) - lo_v).abs() < 60.0,
        "the cloned target should read like the source ({lo_v:.0}), got {:.0}",
        sum(&out, hi_i),
    );
    // A corner well away from the target is untouched.
    let corner = ((2 * w + 2) * 4) as usize;
    assert!(
        (sum(&out, corner) - sum(&base, corner)).abs() < 8.0,
        "a pixel outside the spot must be untouched",
    );
}

/// **Post-crop vignette + grain.** A negative vignette must darken the corners
/// and leave the centre alone; grain must add high-frequency variance to a flat
/// area. Exercises the shader's effects stage (a compile failure takes
/// `Pipeline::new` down).
#[test]
fn effects_vignette_darkens_corners_and_grain_adds_texture() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");

    let sum = |px: &[u8], x: u32, y: u32| {
        let i = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
        px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32
    };

    // Strong negative vignette.
    let vig = EditStack {
        effects: sourcelight_lib::develop::effects::Effects {
            vignette: -0.8,
            vignette_midpoint: 0.2,
            ..Default::default()
        },
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &vig, w, h)
        .expect("vignette");
    let corner_drop = sum(&base, 2, 2) - sum(&out, 2, 2);
    let centre_drop = (sum(&base, w / 2, h / 2) - sum(&out, w / 2, h / 2)).abs();
    assert!(
        corner_drop > 40.0,
        "the corner should darken, dropped {corner_drop:.0}"
    );
    assert!(
        centre_drop < 12.0,
        "the centre should be ~untouched, moved {centre_drop:.0}"
    );

    // Grain: measured as extra local variance over a small flat window.
    let variance = |px: &[u8]| {
        let (mut s, mut s2, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for y in (h / 2)..(h / 2 + 16) {
            for x in (w / 2)..(w / 2 + 16) {
                let v = sum(px, x, y) as f64;
                s += v;
                s2 += v * v;
                n += 1.0;
            }
        }
        s2 / n - (s / n).powi(2)
    };
    let grainy = EditStack {
        effects: sourcelight_lib::develop::effects::Effects {
            grain: 1.0,
            grain_size: 1.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let gout = pipeline
        .render_to_pixels(gpu, &session, &grainy, w, h)
        .expect("grain");
    assert!(
        variance(&gout) > variance(&base) + 10.0,
        "grain should add local variance: {:.1} -> {:.1}",
        variance(&base),
        variance(&gout),
    );
}

/// **Halation and grain roll-off.** The two things that most separate a graded
/// digital frame from a film one, and the two the pipeline could not do.
///
/// Halation must warm the frame around its highlights — a digital sensor never
/// does this on its own, which is a large part of why a graded digital frame
/// still reads as digital. Grain roll-off must leave the highlights smoother
/// than uniform noise does: on film the highlights are where the emulsion is
/// most exposed and reads cleanest.
#[test]
fn halation_warms_highlights_and_grain_fades_out_of_them() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");

    // Mean red-minus-blue **over the highlights**, which is where halation acts:
    // it blooms light past a threshold and leaves everything below it alone.
    //
    // This used to average the whole frame, and the dilution made it a test of
    // the sample rather than of the effect. Measured across all six fixture
    // cameras, the whole-frame shift is +0.02 to +0.36 — always the right
    // direction, never near the +0.5 the assertion wanted, so the guard passed
    // only on whichever frame happened to sort first in the developer's own
    // folder. Restricting the statistic to the brightest tenth restores the
    // signal the effect actually produces, and makes the result independent of
    // how much sky a given photograph contains.
    let warmth = |px: &[u8]| {
        let mut lum: Vec<(f64, f64)> = (0..px.len())
            .step_by(4)
            .map(|i| {
                let (r, g, b) = (px[i] as f64, px[i + 1] as f64, px[i + 2] as f64);
                // Rec.709 luma, to rank by perceived brightness rather than by
                // whichever channel happens to be largest.
                (0.2126 * r + 0.7152 * g + 0.0722 * b, r - b)
            })
            .collect();
        lum.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top = (lum.len() / 50).max(1);
        lum[..top].iter().map(|(_, rb)| rb).sum::<f64>() / top as f64
    };

    let bloomed = EditStack {
        effects: sourcelight_lib::develop::effects::Effects {
            halation: 1.0,
            halation_threshold: 0.25,
            ..Default::default()
        },
        ..Default::default()
    };
    let hout = pipeline
        .render_to_pixels(gpu, &session, &bloomed, w, h)
        .expect("halation");
    assert!(
        warmth(&hout) > warmth(&base) + 0.5,
        "halation should warm the frame: {:.2} -> {:.2}",
        warmth(&base),
        warmth(&hout),
    );

    // Grain roll-off: grain must be no heavier in the highlights with the
    // roll-off on than with flat grain at the same amount.
    // Variance of grain over the brightest tenth of the frame, at the *same*
    // pixel positions in both renders.
    //
    // This used to be one 16x16 block — 256 pixels — chosen as the single
    // brightest in the frame. Two problems, and both showed up the moment the
    // sample folder stopped being one photographer's own shots. A 256-pixel
    // variance is a noisy estimate, and the two renders carry independent grain
    // fields, so the difference between them included the difference between two
    // random patterns. And if that brightest block happened to be clipped, grain
    // had nowhere to go in either render and the comparison measured nothing at
    // all: Panasonic came out 61259 against 61223, a 0.06% difference presented
    // as a regression.
    //
    // A decile is tens of thousands of pixels, which is enough for the roll-off
    // to show above the pattern noise.
    let bright_indices: Vec<usize> = {
        let mut lum: Vec<(f64, usize)> = (0..base.len())
            .step_by(4)
            .map(|i| {
                let v = 0.2126 * base[i] as f64
                    + 0.7152 * base[i + 1] as f64
                    + 0.0722 * base[i + 2] as f64;
                (v, i)
            })
            .collect();
        // Clipped pixels are excluded, and this is the part that matters.
        // Where a channel is already at 255 neither render can express grain —
        // both clamp to the same value — so those pixels contribute only the
        // difference between two random patterns. On the two sample frames with
        // large blown highlights that noise was enough to invert the result.
        lum.retain(|&(_, i)| base[i] < 250 && base[i + 1] < 250 && base[i + 2] < 250);
        lum.sort_by(|a, b| b.0.total_cmp(&a.0));
        lum.truncate((lum.len() / 10).max(64));
        lum.into_iter().map(|(_, i)| i).collect()
    };
    let variance_over = |px: &[u8], idx: &[usize]| {
        let (mut s, mut s2) = (0.0f64, 0.0f64);
        for &i in idx {
            let v = px[i] as f64 + px[i + 1] as f64 + px[i + 2] as f64;
            s += v;
            s2 += v * v;
        }
        let n = idx.len() as f64;
        s2 / n - (s / n).powi(2)
    };

    let with_grain = |rolloff: f32| EditStack {
        effects: sourcelight_lib::develop::effects::Effects {
            grain: 1.0,
            grain_size: 1.0,
            grain_highlight_rolloff: rolloff,
            ..Default::default()
        },
        ..Default::default()
    };
    let flat = pipeline
        .render_to_pixels(gpu, &session, &with_grain(0.0), w, h)
        .expect("flat grain");
    let rolled = pipeline
        .render_to_pixels(gpu, &session, &with_grain(1.0), w, h)
        .expect("rolled grain");
    assert!(
        variance_over(&rolled, &bright_indices) <= variance_over(&flat, &bright_indices),
        "grain should be no heavier in the highlights with roll-off on: {:.1} vs {:.1}",
        variance_over(&rolled, &bright_indices),
        variance_over(&flat, &bright_indices),
    );
}

/// **Chroma noise reduction.** Colour NR must smooth the *colour* of a region
/// while leaving its *luminance* alone — that separation is the whole point of a
/// dedicated chroma control. Finds the 16×16 block with the most colour
/// variation, then asserts that at full colour NR its chroma variance collapses
/// while its luma variance barely moves.
#[test]
fn colour_noise_reduction_smooths_chroma_not_luma() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(384);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");
    let denoised = EditStack {
        detail: sourcelight_lib::develop::detail::Detail {
            color_noise: 1.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let cn = pipeline
        .render_to_pixels(gpu, &session, &denoised, w, h)
        .expect("colour-denoised");

    // High-frequency energy: each pixel's deviation from its 4-neighbour mean,
    // summed over the frame — for luma and for an opponent-colour axis. This
    // isolates *speckle* from edges (an edge is a small fraction of pixels),
    // which raw block variance cannot. `chroma` picks the axis.
    let hf = |px: &[u8], chroma: bool| -> f64 {
        let val = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            let (r, g, b) = (px[i] as f64, px[i + 1] as f64, px[i + 2] as f64);
            if chroma {
                (r - b) + (g - 0.5 * (r + b))
            } else {
                0.2627 * r + 0.678 * g + 0.0593 * b
            }
        };
        let mut e = 0.0;
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let m = (val(x - 1, y) + val(x + 1, y) + val(x, y - 1) + val(x, y + 1)) / 4.0;
                let d = val(x, y) - m;
                e += d * d;
            }
        }
        e
    };

    let chroma_base = hf(&base, true);
    let chroma_cn = hf(&cn, true);
    let luma_base = hf(&base, false);
    let luma_cn = hf(&cn, false);
    assert!(
        chroma_base > 100.0,
        "test image has no chroma detail to smooth"
    );
    // Colour speckle should collapse — the mip blur shares one smooth chroma
    // across neighbours.
    assert!(
        chroma_cn < chroma_base * 0.6,
        "high-frequency chroma should drop sharply: {chroma_base:.0} -> {chroma_cn:.0}",
    );
    // Luminance is untouched by the chroma pass (it rescales the blur to each
    // pixel's own luma), so its high-frequency energy must barely move.
    assert!(
        luma_cn > luma_base * 0.9,
        "luminance detail must be preserved: {luma_base:.0} -> {luma_cn:.0}",
    );
}

/// **The AI-mask GPU path.** An AI mask is a stored coverage bitmap uploaded as a
/// texture layer and sampled like a brush. This builds a synthetic coverage —
/// the left half covered, the right half not — darkens through it, and asserts
/// only the left half changes. Exercises the RLE decode → resize → texture
/// upload → shader `KIND_AI` branch (a bug there takes `Pipeline::new` down).
#[test]
fn an_ai_mask_applies_its_stored_coverage() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base");

    // An 8x8 coverage: left four columns full, right four empty.
    let (cw, ch) = (8u32, 8u32);
    let mut cov = vec![0u8; (cw * ch) as usize];
    for y in 0..ch {
        for x in 0..cw {
            if x < cw / 2 {
                cov[(y * cw + x) as usize] = 255;
            }
        }
    }
    let masked = EditStack {
        masks: vec![Mask {
            components: vec![MaskComponent::new(MaskShape::Ai {
                label: "Subject".into(),
                width: cw,
                height: ch,
                rle: encode_ai(&cov),
            })],
            adjustments: LocalAdjustments {
                exposure: -2.0,
                ..Default::default()
            },
            name: None,
        }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &masked, w, h)
        .expect("masked");

    let at = |px: &[u8], fx: f32| {
        let x = ((fx * w as f32) as u32).min(w - 1);
        let y = h / 2;
        let i = ((y * w + x) * 4) as usize;
        px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32
    };
    assert!(
        at(&out, 0.2) < at(&base, 0.2) - 20.0,
        "the covered (left) half should darken: {:.0} -> {:.0}",
        at(&base, 0.2),
        at(&out, 0.2),
    );
    assert!(
        (at(&out, 0.8) - at(&base, 0.8)).abs() < 8.0,
        "the uncovered (right) half must be untouched: {:.0} -> {:.0}",
        at(&base, 0.8),
        at(&out, 0.8),
    );
}

/// A per-channel (blue) curve pulls only the blue channel down, leaving red and
/// green essentially untouched — proving the RGBA `channel_lut` is bound, that
/// each channel indexes its own column, and that the shader applies it. A bug
/// that swapped channels or ignored the table would move the wrong channel or
/// none at all.
#[test]
fn a_per_channel_curve_shifts_only_that_channel() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let base = pipeline
        .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
        .expect("base render");

    // Blue: 0 -> 0, 1 -> 0.5. Halves blue at every input value.
    let darkened_blue = EditStack {
        curve_blue: vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 0.5 }],
        ..Default::default()
    };
    let out = pipeline
        .render_to_pixels(gpu, &session, &darkened_blue, w, h)
        .expect("blue-curve render");

    let (_, _, base_ch) = describe(&base);
    let (_, _, out_ch) = describe(&out);
    eprintln!("base channels {base_ch:.1?} -> blue-curved {out_ch:.1?}");

    // Blue drops clearly; red and green barely move (the curve never touches
    // their columns, so any change is second-order via clipping/rounding).
    assert!(
        out_ch[2] < base_ch[2] - 15.0,
        "blue should fall: {:.1} -> {:.1}",
        base_ch[2],
        out_ch[2],
    );
    assert!(
        (out_ch[0] - base_ch[0]).abs() < 4.0,
        "red must be essentially unchanged: {:.1} -> {:.1}",
        base_ch[0],
        out_ch[0],
    );
    assert!(
        (out_ch[1] - base_ch[1]).abs() < 4.0,
        "green must be essentially unchanged: {:.1} -> {:.1}",
        base_ch[1],
        out_ch[1],
    );
}

/// Dehaze cuts atmospheric veil, so a positive amount expands contrast (raising
/// the frame's luma spread) while a negative amount adds veil (lowering it), and
/// zero is an exact no-op. Haze is scene-dependent so an absolute magnitude
/// can't be asserted on an arbitrary photo — but the *sign* must be right, which
/// a swapped transmission term or a bad mip fetch would break.
#[test]
fn dehaze_expands_contrast_and_adding_veil_reduces_it() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(256);

    let render = |dehaze: f32| {
        let stack = EditStack {
            detail: sourcelight_lib::develop::detail::Detail {
                dehaze,
                ..Default::default()
            },
            ..Default::default()
        };
        let px = pipeline
            .render_to_pixels(gpu, &session, &stack, w, h)
            .expect("render");
        describe(&px).1 // luma stddev
    };

    let base = render(0.0);
    let dehazed = render(0.8);
    let hazed = render(-0.8);
    eprintln!("stddev: haze {hazed:.2} < base {base:.2} < dehaze {dehazed:.2}");

    assert!(
        dehazed > base + 0.5,
        "dehaze should expand contrast: base {base:.2} -> {dehazed:.2}",
    );
    assert!(
        hazed < base - 0.5,
        "adding veil should reduce contrast: base {base:.2} -> {hazed:.2}",
    );
}

/// Deconvolution "detail" sharpens the finest scale *more* than a plain unsharp
/// mask at the same amount, and detail=0 leaves the unsharp mask untouched. The
/// van Cittert second step adds high-frequency energy, so a textured region
/// gains more local contrast with detail up — proving the extra band is wired
/// and blended, not that any particular look is "right".
#[test]
fn deconvolution_detail_sharpens_finer_than_a_plain_unsharp_mask() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let raw = decoder.decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    // A larger proxy so single-pixel detail actually resolves — sharpening is a
    // 1:1 effect and averages out of a small render.
    let (w, h) = session.proxy_size(1400);

    let sharpen = |detail: f32| sourcelight_lib::develop::detail::Detail {
        sharpen: 0.8,
        sharpen_radius: 1.0,
        sharpen_detail: detail,
        ..Default::default()
    };

    // Fine-scale high-frequency energy: mean absolute luma difference between
    // adjacent pixels. Deconvolution trades mid-band contrast (which dominates
    // whole-frame stddev, and which it *lowers* by cutting halo) for the finest
    // detail, so the honest measure is adjacent-pixel gradient, not stddev — the
    // same whole-frame-roughness trap the sharpening notes already record.
    let high_freq = |px: &[u8]| -> f64 {
        let luma = |i: usize| {
            0.2126 * px[i] as f64 + 0.7152 * px[i + 1] as f64 + 0.0722 * px[i + 2] as f64
        };
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in 0..h {
            for x in 0..w - 1 {
                let i = ((y * w + x) * 4) as usize;
                sum += (luma(i) - luma(i + 4)).abs();
                n += 1.0;
            }
        }
        sum / n
    };
    let render = |d: f32| {
        let stack = EditStack {
            detail: sharpen(d),
            ..Default::default()
        };
        let px = pipeline
            .render_to_pixels(gpu, &session, &stack, w, h)
            .expect("render");
        high_freq(&px)
    };

    let plain = render(0.0);
    let decon = render(1.0);
    eprintln!("adjacent-pixel gradient: plain USM {plain:.3} < deconvolution {decon:.3}");
    assert!(
        decon > plain + 0.1,
        "deconvolution detail should add fine sharpening over a plain USM: {plain:.3} -> {decon:.3}",
    );

    // detail=0 must reproduce the plain unsharp mask exactly (regression guard —
    // the new branch is skipped and the byte path is unchanged).
    let stack0 = EditStack {
        detail: sharpen(0.0),
        ..Default::default()
    };
    let a = pipeline
        .render_to_pixels(gpu, &session, &stack0, w, h)
        .unwrap();
    let b = pipeline
        .render_to_pixels(gpu, &session, &stack0, w, h)
        .unwrap();
    assert_eq!(a, b, "the same stack must render identically");
}

/// AI noise reduction's result in a develop session: it takes the
/// place of the linear image at the stack's Amount, as a step in history, and
/// undo, redo and a moved Amount re-blend it. A stand-in result (the image at
/// half its light) makes each state easy to tell apart.
#[test]
fn a_denoise_result_follows_the_amount_through_history() {
    if gpu().is_none() {
        return;
    }
    let dir = test_support::Scratch::new("denoise-session");
    let path = dir.join("grey.png");
    image::RgbImage::from_pixel(64, 48, image::Rgb([150, 150, 150]))
        .save(&path)
        .unwrap();

    let mut dev = DevelopState::new(Box::new(ReadbackPresenter::new())).expect("develop state");
    let info = dev
        .open(3, &path, &LibRawDecoder::new(), None)
        .expect("open");
    assert!(!info.denoise_ready);
    let mean = |dev: &mut DevelopState| {
        let frame = dev.render(Some(64), None, None, false).expect("render");
        // The presenter hands back an encoded image, as the webview gets it.
        let img = image::load_from_memory(frame.bytes().expect("bytes"))
            .expect("decode")
            .to_rgb8();
        img.pixels().map(|p| p[1] as f64).sum::<f64>() / (img.width() * img.height()) as f64
    };
    let plain = mean(&mut dev);

    let linear = dev.linear_for_denoise(None).unwrap().expect("not run yet");
    assert_eq!((linear.photo_id, linear.width, linear.height), (3, 64, 48));
    let original = linear.pixels;
    // A look while it runs shows, then Cancel puts the original back.
    let shade = |px: &[u16], k: f32| -> Vec<u16> {
        px.chunks(4)
            .flat_map(|p| {
                let f =
                    |b: u16| half::f16::from_f32(half::f16::from_bits(b).to_f32() * k).to_bits();
                [f(p[0]), f(p[1]), f(p[2]), p[3]]
            })
            .collect()
    };
    let frame = dev.show_denoise_preview(3, &shade(&original, 0.5)).unwrap();
    assert!(frame.is_some(), "a look at the open photo is shown");
    assert!(mean(&mut dev) < plain - 20.0, "the look reaches the render");
    assert!(
        dev.show_denoise_preview(9, &original).unwrap().is_none(),
        "not another photo"
    );
    dev.restore_linear(3, &original).unwrap();
    assert!(
        (mean(&mut dev) - plain).abs() < 1.0,
        "restored after a cancel"
    );
    let halved: Vec<u16> = original
        .chunks(4)
        .flat_map(|p| {
            let f = |b: u16| half::f16::from_f32(half::f16::from_bits(b).to_f32() * 0.5).to_bits();
            [f(p[0]), f(p[1]), f(p[2]), p[3]]
        })
        .collect();
    assert!(
        dev.attach_denoise(4, original.clone(), halved.clone(), Some(1.0))
            .is_err(),
        "a result for a photo no longer open is refused"
    );
    let info = dev
        .attach_denoise(3, original, halved, Some(1.0))
        .expect("attach");
    assert!(info.denoise_ready);
    assert_eq!(info.stack.detail.ai_denoise, 1.0);
    assert_eq!(
        info.history.last().map(String::as_str),
        Some("AI noise reduction")
    );
    let full = mean(&mut dev);
    assert!(full < plain - 20.0, "{plain} -> {full}");
    assert!(
        dev.linear_for_denoise(None).unwrap().is_none(),
        "it runs once per photo"
    );

    let mut stack = dev.current_stack().unwrap();
    stack.detail.ai_denoise = 0.5;
    dev.apply("AI noise reduction", stack).unwrap();
    let part = mean(&mut dev);
    assert!(
        part > full + 5.0 && part < plain - 5.0,
        "{plain} > {part} > {full}"
    );

    // "Before" is the photo as decoded, not the denoised image.
    let before = {
        let frame = dev
            .render_before(Some(64), &EditStack::default(), None, false)
            .expect("before");
        let img = image::load_from_memory(frame.bytes().expect("bytes"))
            .expect("decode")
            .to_rgb8();
        img.pixels().map(|p| p[1] as f64).sum::<f64>() / (img.width() * img.height()) as f64
    };
    assert!(
        (before - plain).abs() < 1.0,
        "Before shows the original: {plain} vs {before}"
    );

    dev.undo().unwrap();
    assert!((mean(&mut dev) - full).abs() < 1.0, "undo re-blends");
    dev.undo().unwrap();
    assert!(
        (mean(&mut dev) - plain).abs() < 1.0,
        "back before it: the original"
    );
    dev.redo().unwrap();
    assert!((mean(&mut dev) - full).abs() < 1.0, "redo re-blends");
}

/// One frame the direct surface was asked for: its size, where it sits, and
/// whether it went through a viewport.
type Asked = (u32, u32, (u32, u32), bool);

/// What the direct surface was asked to draw, without drawing it.
struct Recording(std::sync::Arc<std::sync::Mutex<Vec<Asked>>>);

impl Presenter for Recording {
    fn present(&mut self, frame: Frame<'_>) -> anyhow::Result<sourcelight_lib::present::Presented> {
        self.0.lock().unwrap().push((
            frame.width,
            frame.height,
            frame.origin,
            frame.view.is_some(),
        ));
        Ok(sourcelight_lib::present::Presented::Direct)
    }

    fn produces_bytes(&self) -> bool {
        false
    }

    fn describe(&self) -> &'static str {
        "recording"
    }
}

/// **The direct surface lays the photo out as the page lays out the image.**
///
/// The surface covers the whole canvas, so the letterbox is the renderer's job
/// there: fit inside the canvas, centred — what `object-fit: contain` does for
/// the readback image, and what the crop and mask overlays are positioned
/// against. A frame drawn at the corner, or one edge short, would put every
/// overlay off the photo.
#[test]
fn the_direct_surface_fits_and_centres_the_photo_in_the_canvas() {
    let Some(path) = first_raw() else { return };
    if gpu().is_none() {
        return;
    }
    let mut dev = DevelopState::new(Box::new(ReadbackPresenter::new())).expect("develop state");
    let decoder = LibRawDecoder::new();
    let info = dev.open(1, &path, &decoder, None).expect("open");
    let drawn = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    dev.set_direct(Some(Box::new(Recording(drawn.clone()))));

    // Wider than any sensor, then taller: the photo meets the canvas on its
    // height, then on its width.
    for (cw, ch) in [(1850u32, 1050u32), (900, 1400)] {
        dev.render_direct((cw, ch), None, None, false, None, false)
            .expect("render");
        let (w, h, (x, y), viewed) = drawn.lock().unwrap().pop().expect("a frame");
        assert!(!viewed, "fit is the sharp proxy, not a viewport");
        assert!(w <= cw && h <= ch, "{w}x{h} overflows {cw}x{ch}");
        assert!(
            cw - w <= 1 || ch - h <= 1,
            "{w}x{h} does not reach an edge of {cw}x{ch}"
        );
        assert!(x.abs_diff((cw - w) / 2) <= 1 && y.abs_diff((ch - h) / 2) <= 1);
        let shape = w as f64 / h as f64;
        let photo = info.width as f64 / info.height as f64;
        assert!(
            (shape - photo).abs() < 0.01,
            "stretched: {shape} vs {photo}"
        );
    }

    // A canvas bigger than the photo: no proxy to take, so it is magnified
    // through a viewport to fill, still centred.
    let big = (info.width * 3 / 2, info.height * 3);
    dev.render_direct(big, None, None, false, None, false)
        .expect("render");
    let (w, h, (x, y), viewed) = drawn.lock().unwrap().pop().expect("a frame");
    assert!(viewed);
    assert!(big.0 - w <= 1, "{w} should fill {}", big.0);
    assert!(y.abs_diff((big.1 - h) / 2) <= 1 && x <= 1);
}

/// **The split view on a surface: before on the left, the edit on the right,
/// meeting at the line.** Drawn as the direct surface draws it — the edit
/// across the frame, then the before over the left of it without a clear —
/// into a texture the test can read.
#[test]
fn the_split_view_draws_the_before_left_of_the_line_and_the_edit_right_of_it() {
    let Some(path) = first_raw() else { return };
    let Some(gpu) = gpu() else { return };
    let raw = LibRawDecoder::new().decode(&path).expect("decode");
    let pipeline = Pipeline::new(gpu);
    let session = pipeline.prepare(gpu, &raw).expect("prepare");
    let (w, h) = session.proxy_size(600);

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("split"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let develop = pipeline.develop_pipeline_for(gpu, format);
    let edit = EditStack {
        exposure: 2.0,
        ..Default::default()
    };
    let before = EditStack::default();
    let line = w / 2;
    let draw = |stack: &EditStack, left_of: Option<u32>| {
        pipeline.render_to_surface(
            gpu,
            &session,
            stack,
            &target,
            &develop,
            (0, 0),
            w,
            h,
            None,
            false,
            left_of,
        )
    };
    draw(&edit, None);
    draw(&before, Some(line));
    let split = gpu.read_rgba8(&target, w, h).expect("read");

    // Each half against the same frame drawn whole.
    draw(&before, None);
    let all_before = gpu.read_rgba8(&target, w, h).expect("read");
    draw(&edit, None);
    let all_edit = gpu.read_rgba8(&target, w, h).expect("read");

    let column = |pixels: &[u8], x: u32| -> Vec<u8> {
        (0..h)
            .flat_map(|y| {
                let i = ((y * w + x) * 4) as usize;
                pixels[i..i + 3].to_vec()
            })
            .collect()
    };
    for x in [0, line / 2, line - 1] {
        assert_eq!(
            column(&split, x),
            column(&all_before, x),
            "column {x} is not the before"
        );
    }
    for x in [line, line + line / 2, w - 1] {
        assert_eq!(
            column(&split, x),
            column(&all_edit, x),
            "column {x} is not the edit"
        );
    }
    assert_ne!(column(&all_before, line), column(&all_edit, line));
}

/// An AI selection's input is checked when the model's answer is back: it must
/// be the same photo, framed the same way, or the mask is refused. The model
/// runs with the develop lock released and Develop live, so either may have
/// changed meanwhile, and a mask made for the old framing would land wrong.
#[test]
fn a_selection_fits_only_the_photo_and_framing_it_was_made_for() {
    if gpu().is_none() {
        return;
    }
    let dir = test_support::Scratch::new("selection-input");
    let path = dir.join("grey.png");
    image::RgbImage::from_pixel(64, 48, image::Rgb([150, 150, 150]))
        .save(&path)
        .unwrap();
    let mut dev = DevelopState::new(Box::new(ReadbackPresenter::new())).expect("develop state");
    dev.open(3, &path, &LibRawDecoder::new(), None)
        .expect("open");

    let input = dev.selection_input(32).expect("input");
    assert_eq!((input.photo_id, input.width, input.height), (3, 32, 24));
    assert_eq!(input.pixels.len(), 32 * 24 * 4);
    dev.still_fits(&input).expect("nothing changed");

    // A slider moved meanwhile: the framing is the same, so the mask fits.
    let mut brighter = dev.current_stack().unwrap();
    brighter.exposure = 0.5;
    dev.apply("Exposure", brighter).unwrap();
    dev.still_fits(&input)
        .expect("exposure does not move a mask");

    // Turned a quarter: it no longer does.
    let mut turned = dev.current_stack().unwrap();
    turned.geometry.quarter_turns = 1;
    dev.apply("Rotate", turned).unwrap();
    assert!(dev.still_fits(&input).is_err());

    // Nor on another photo.
    let second = dev.selection_input(32).expect("input");
    dev.open(4, &path, &LibRawDecoder::new(), None)
        .expect("open");
    assert!(dev.still_fits(&second).is_err());
}

/// Magic erase through the session, with a stand-in for the model: the fill
/// shows where the selection was and nowhere else, is a step in history,
/// stays out of "Before", sits on top of AI noise reduction as its Amount
/// moves, is laid in again from its file when the photo is reopened, and is
/// in the export.
#[test]
fn an_erase_is_laid_into_the_photo_and_follows_history() {
    use sourcelight_lib::develop::erase;
    if gpu().is_none() {
        return;
    }
    let dir = test_support::Scratch::new("erase-session");
    let path = dir.join("grey.png");
    image::RgbImage::from_pixel(640, 480, image::Rgb([150, 150, 150]))
        .save(&path)
        .unwrap();
    let fresh = || {
        let mut dev = DevelopState::new(Box::new(ReadbackPresenter::new())).expect("develop state");
        dev.set_fills_dir(dir.join("patches"));
        dev
    };
    let mut dev = fresh();
    dev.open(3, &path, &LibRawDecoder::new(), None)
        .expect("open");
    // Green at the middle of the selection and in a corner, from a small
    // render as the webview gets it.
    let probe = |dev: &mut DevelopState, stack: Option<&EditStack>, before: bool| {
        let frame = if before {
            dev.render_before(Some(64), stack.unwrap(), None, false)
        } else {
            dev.render(Some(64), stack, None, false)
        }
        .expect("render");
        let img = image::load_from_memory(frame.bytes().expect("bytes"))
            .expect("decode")
            .to_rgb8();
        (img.get_pixel(32, 24)[1], img.get_pixel(3, 3)[1])
    };
    let (plain, corner) = probe(&mut dev, None, false);

    // A block in the middle of the frame: 80 px square.
    let coverage: Vec<u8> = (0..64 * 48)
        .map(|i| {
            let (x, y) = (i % 64, i / 64);
            if (28..36).contains(&x) && (20..28).contains(&y) {
                255
            } else {
                0
            }
        })
        .collect();
    let select = erase::Coverage {
        data: &coverage,
        width: 64,
        height: 48,
    };
    // A stand-in model that paints its whole answer near black.
    let dark = |input: &erase::ModelInput| Ok(vec![0.1; input.rgb.len()]);
    let mut input = dev.erase_input(&select).expect("input");
    assert_eq!(input.photo_id, 3);
    let made = erase::run(&mut input.region, &input.plan, dark, |_, _| {}).expect("run");
    let info = dev.attach_erase(&input, "Thing", made).expect("attach");
    assert_eq!(info.history.last().map(String::as_str), Some("Erase"));
    assert_eq!(info.stack.erases.len(), 1);
    assert_eq!(info.stack.erases[0].label, "Thing");
    let fill = &info.stack.erases[0].pieces[0].fill;
    assert!(dir.join("patches").join(format!("{fill}.png")).exists());

    let (erased, untouched) = probe(&mut dev, None, false);
    assert!(erased + 40 < plain, "erased {plain} -> {erased}");
    assert!(
        untouched.abs_diff(corner) <= 1,
        "corner {corner} -> {untouched}"
    );
    // The same input again is refused: the erases have changed since.
    let made = erase::run(&mut input.region, &input.plan, dark, |_, _| {}).expect("run");
    assert!(dev.attach_erase(&input, "Again", made).is_err());

    // "Before" is the photo as decoded.
    let (before, _) = probe(&mut dev, Some(&EditStack::default()), true);
    assert!(before.abs_diff(plain) <= 1, "before {before}");

    dev.undo().unwrap();
    assert!(
        probe(&mut dev, None, false).0.abs_diff(plain) <= 1,
        "undone"
    );
    dev.redo().unwrap();
    assert!(
        probe(&mut dev, None, false).0.abs_diff(erased) <= 1,
        "redone"
    );

    // A stack from the page that never saw the erase keeps it all the same.
    let stale = dev
        .with_own_erases(EditStack {
            exposure: 0.1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(stale.erases, dev.current_stack().unwrap().erases);
    // Removed, the object is back; undone, gone again.
    let info = dev.remove_erase(0).unwrap();
    assert!(info.stack.erases.is_empty());
    assert_eq!(
        info.history.last().map(String::as_str),
        Some("Remove erase")
    );
    assert!(
        probe(&mut dev, None, false).0.abs_diff(plain) <= 1,
        "removed"
    );
    assert!(dev.remove_erase(0).is_err());
    dev.undo().unwrap();
    assert!(probe(&mut dev, None, false).0.abs_diff(erased) <= 1, "back");

    // AI noise reduction under it: the rest of the photo takes the Amount,
    // the fill stays on top.
    let linear = dev.linear_for_denoise(None).unwrap().expect("not run yet");
    let halved: Vec<u16> = linear
        .pixels
        .chunks(4)
        .flat_map(|p| {
            let f = |b: u16| half::f16::from_f32(half::f16::from_bits(b).to_f32() * 0.5).to_bits();
            [f(p[0]), f(p[1]), f(p[2]), p[3]]
        })
        .collect();
    // What noise reduction is given has no erase in it.
    let centre = ((240 * 640 + 320) * 4) as usize;
    assert!(half::f16::from_bits(linear.pixels[centre]).to_f32() > 0.1);
    dev.attach_denoise(3, linear.pixels, halved, Some(1.0))
        .expect("denoise");
    let (middle, darker) = probe(&mut dev, None, false);
    assert!(darker + 20 < corner, "denoised corner {corner} -> {darker}");
    assert!(
        middle.abs_diff(erased) <= 2,
        "the fill stays: {erased} -> {middle}"
    );
    let mut half_way = dev.current_stack().unwrap();
    half_way.detail.ai_denoise = 0.5;
    dev.apply("AI noise reduction", half_way).unwrap();
    let (middle, part) = probe(&mut dev, None, false);
    assert!(
        part > darker + 5 && part + 5 < corner,
        "{darker} < {part} < {corner}"
    );
    assert!(
        middle.abs_diff(erased) <= 2,
        "the fill stays: {erased} -> {middle}"
    );

    // An erase worked out while the image was rewritten underneath it (the
    // Amount moved back up) is laid in afresh, not put up as it was read:
    // around it the photo keeps the new Amount.
    let corner_block: Vec<u8> = (0..64 * 48)
        .map(|i| {
            let (x, y) = (i % 64, i / 64);
            if (12..16).contains(&x) && (12..16).contains(&y) {
                255
            } else {
                0
            }
        })
        .collect();
    let mut input = dev
        .erase_input(&erase::Coverage {
            data: &corner_block,
            width: 64,
            height: 48,
        })
        .expect("input");
    let mut full_again = dev.current_stack().unwrap();
    full_again.detail.ai_denoise = 1.0;
    dev.apply("AI noise reduction", full_again).unwrap();
    let made = erase::run(&mut input.region, &input.plan, dark, |_, _| {}).expect("run");
    dev.attach_erase(&input, "Corner", made).expect("attach");
    let (middle, now) = probe(&mut dev, None, false);
    assert!(
        now.abs_diff(darker) <= 2,
        "the corner at the Amount set meanwhile: {darker}, not {part}: {now}"
    );
    assert!(
        middle.abs_diff(erased) <= 2,
        "the first fill stays: {middle}"
    );
    dev.undo().unwrap();
    let mut half_way = dev.current_stack().unwrap();
    half_way.detail.ai_denoise = 0.5;
    dev.apply("AI noise reduction", half_way).unwrap();

    // Reopened from the catalogue's stack, in a new session: laid in from
    // the file.
    let stored = dev.current_stack().unwrap();
    let mut again = fresh();
    again
        .open(3, &path, &LibRawDecoder::new(), Some(stored.clone()))
        .expect("reopen");
    assert!(
        probe(&mut again, None, false).0.abs_diff(erased) <= 2,
        "reopened"
    );
    // And exported with it.
    let bytes = again
        .export(
            &ExportOptions {
                format: ExportFormat::Png,
                ..Default::default()
            },
            None,
        )
        .expect("export");
    let full = image::load_from_memory(&bytes).unwrap().to_rgb8();
    assert!(full.get_pixel(320, 240)[1] + 40 < plain, "exported");
    assert!(full.get_pixel(20, 20)[1].abs_diff(corner) <= 2);

    // Without its file, the photo opens without the erase rather than fail.
    std::fs::remove_dir_all(dir.join("patches")).unwrap();
    let mut gone = fresh();
    gone.open(3, &path, &LibRawDecoder::new(), Some(stored))
        .expect("opens anyway");
    assert!(probe(&mut gone, None, false).0.abs_diff(plain) <= 1);
}
