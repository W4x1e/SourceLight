//! What the GPU test measures, and the live renderer behind its sliders.
//!
//! Every number is wall time on the calling thread, GPU work included: each
//! render here ends in a readback, which waits for the GPU to finish.

use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use sl_develop::develop::EditStack;
use sl_develop::gpu::pipeline::{DevelopSession, Pipeline};
use sl_develop::gpu::Gpu;

/// Frames timed per photo, after the first (which compiles the shaders).
const FRAMES: usize = 20;

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub adapter: String,
    /// Vulkan, Metal, GL, Dx12.
    pub backend: String,
    pub driver: String,
    pub device_type: String,
    /// The limits Develop asks for that this GPU does not have, one line each.
    /// Empty when it has them all.
    pub short_of: Vec<String>,
    /// Why Develop's own device request failed, when it did and the test
    /// carried on with the GPU's own limits instead: whether the pipeline
    /// really needs what it asks for.
    pub fallback: Option<String>,
    pub device_ms: f64,
    pub pipeline_ms: f64,
    pub photos: Vec<PhotoTimes>,
    /// What stopped the test, if something did.
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoTimes {
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub decode_ms: f64,
    /// Into the pipeline's linear image on the GPU.
    pub upload_ms: f64,
    /// The size frames are rendered at: the photo fitted to the screen.
    pub frame_width: u32,
    pub frame_height: u32,
    /// The first frame, which includes compiling the shaders.
    pub first_frame_ms: f64,
    /// Frames with the exposure moving, as a slider would: render and
    /// readback.
    pub frame_median_ms: f64,
    pub frame_worst_ms: f64,
    /// Encoding one frame as a JPEG, for the webview.
    pub jpeg_ms: f64,
    /// The whole photo at full size, as Save to gallery would render it.
    pub full_size_ms: f64,
}

/// Runs the test on each photo in `photos` (a label and a file), rendering
/// frames at the size the photo fills a `screen` (width, height in device
/// pixels). Stops at the first failure and says what it was, with whatever
/// was measured before it.
pub fn run(photos: &[(&str, &Path)], screen: (u32, u32)) -> (Report, Option<Live>) {
    let mut report = Report::default();
    let mut live = None;
    if let Err(e) = run_into(&mut report, &mut live, photos, screen) {
        report.error = Some(format!("{e:#}"));
    }
    (report, live)
}

fn run_into(
    report: &mut Report,
    live: &mut Option<Live>,
    photos: &[(&str, &Path)],
    screen: (u32, u32),
) -> Result<()> {
    let (instance, adapter) = adapter()?;
    let info = adapter.get_info();
    report.adapter = info.name.clone();
    report.backend = format!("{:?}", info.backend);
    report.driver = format!("{} {}", info.driver, info.driver_info)
        .trim()
        .to_string();
    report.device_type = format!("{:?}", info.device_type);
    let available = adapter.limits();
    let wanted = develop_limits(&available);
    wanted.check_limits_with_fail_fn(&available, false, |name, want, have| {
        report.short_of.push(format!(
            "{name}: Develop asks for {want}, this GPU has {have}"
        ));
    });

    let t = Instant::now();
    let gpu = match device(&instance, &adapter, wanted) {
        Ok(gpu) => gpu,
        Err(e) => {
            report.fallback = Some(format!("{e:#}"));
            device(&instance, &adapter, available)
                .context("making a GPU device with this GPU's own limits")?
        }
    };
    report.device_ms = ms(t);
    let t = Instant::now();
    let pipeline = gpu.checked("preparing the editing code", || Ok(Pipeline::new(&gpu)))?;
    report.pipeline_ms = ms(t);

    let mut first = None;
    for (label, path) in photos {
        let (times, session) = photo(&gpu, &pipeline, label, path, screen)
            .with_context(|| format!("testing {label}"))?;
        report.photos.push(times);
        first.get_or_insert(session);
    }
    // The sliders work on the first photo, the one a person would edit, with
    // the pipeline whose shaders are already compiled.
    *live = first.map(|session| Live {
        gpu,
        pipeline,
        session,
        screen,
    });
    Ok(())
}

/// The GPU device editing on the phone uses: as Develop makes one, and with
/// the GPU's own limits where Develop's desktop floor is refused (the GPU
/// test found the pipeline needs far less than it asks for).
pub fn phone_gpu() -> Result<Gpu> {
    let (instance, adapter) = adapter()?;
    let available = adapter.limits();
    device(&instance, &adapter, develop_limits(&available))
        .or_else(|_| device(&instance, &adapter, available))
}

/// The instance and adapter, with a release build's flags even in a debug
/// APK. wgpu's default in a debug build names every GPU object for debuggers,
/// and that call crashes the Android emulator's Vulkan driver outright; a
/// crash leaves nothing to report. A shipped app runs with these flags anyway.
fn adapter() -> Result<(wgpu::Instance, wgpu::Adapter)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        flags: wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .context("no GPU adapter")?;
    Ok((instance, adapter))
}

/// What `Gpu::new` asks for (src-tauri/crates/sl-develop/src/gpu/mod.rs).
fn develop_limits(available: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_buffer_size: available.max_buffer_size,
        ..wgpu::Limits::default().using_resolution(available.clone())
    }
}

/// A device as `Gpu::new` makes one (src-tauri/crates/sl-develop/src/gpu/mod.rs), with these
/// limits: the features it asks for when offered, and errors nothing waited
/// for logged rather than a panic.
fn device(instance: &wgpu::Instance, adapter: &wgpu::Adapter, limits: wgpu::Limits) -> Result<Gpu> {
    let deep = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    let renders_deep = adapter.features().contains(deep)
        && adapter
            .get_texture_format_features(wgpu::TextureFormat::Rgba16Unorm)
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("sourcelight"),
        required_features: if renders_deep {
            deep
        } else {
            wgpu::Features::empty()
        },
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::Performance,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        trace: wgpu::Trace::Off,
    }))
    .context("failed to create GPU device")?;
    device.on_uncaptured_error(std::sync::Arc::new(|e| {
        eprintln!("GPU error nothing was waiting for: {e}");
    }));
    Ok(Gpu {
        device,
        queue,
        info: adapter.get_info(),
        instance: instance.clone(),
        adapter: adapter.clone(),
    })
}

fn photo(
    gpu: &Gpu,
    pipeline: &Pipeline,
    label: &str,
    path: &Path,
    screen: (u32, u32),
) -> Result<(PhotoTimes, DevelopSession)> {
    let t = Instant::now();
    let img = sl_develop::raster::decode(path)?;
    let decode_ms = ms(t);
    let (width, height) = img.size();

    let t = Instant::now();
    let session = gpu.checked("uploading the photo", || {
        pipeline.prepare_rendered(gpu, &img)
    })?;
    // Until the queue has run, the upload has only been recorded.
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| anyhow!("GPU poll failed: {e}"))?;
    let upload_ms = ms(t);
    drop(img);

    let mut stack = EditStack::default();
    let (fw, fh) = session.framed_proxy(&stack, fitted_edge(&session, &stack, screen));
    let t = Instant::now();
    let mut frame = gpu.checked("rendering", || {
        pipeline.render_to_pixels(gpu, &session, &stack, fw, fh)
    })?;
    let first_frame_ms = ms(t);

    let mut times = Vec::with_capacity(FRAMES);
    for i in 0..FRAMES {
        // A slider being dragged: a different exposure every frame.
        stack.exposure = (i as f32 / FRAMES as f32 - 0.5) * 2.0;
        let t = Instant::now();
        frame = pipeline.render_to_pixels(gpu, &session, &stack, fw, fh)?;
        times.push(ms(t));
    }
    times.sort_by(f64::total_cmp);

    let t = Instant::now();
    let _jpeg = jpeg(&frame, fw, fh)?;
    let jpeg_ms = ms(t);

    let (full_w, full_h) = session.framed_size(&stack);
    let t = Instant::now();
    gpu.checked("rendering at full size", || {
        pipeline.render_to_pixels(gpu, &session, &stack, full_w, full_h)
    })?;
    let full_size_ms = ms(t);

    Ok((
        PhotoTimes {
            label: label.to_string(),
            width,
            height,
            decode_ms,
            upload_ms,
            frame_width: fw,
            frame_height: fh,
            first_frame_ms,
            frame_median_ms: times[FRAMES / 2],
            frame_worst_ms: times[FRAMES - 1],
            jpeg_ms,
            full_size_ms,
        },
        session,
    ))
}

/// The first photo, kept on the GPU for the test screen's sliders.
pub struct Live {
    gpu: Gpu,
    pipeline: Pipeline,
    session: DevelopSession,
    screen: (u32, u32),
}

/// One frame for the sliders, and what it cost.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// RGBA, or a JPEG when asked for one.
    pub bytes: Vec<u8>,
    pub render_ms: f64,
    pub encode_ms: f64,
}

impl Live {
    /// Renders with these settings, as RGBA or, with `as_jpeg`, a JPEG.
    pub fn frame(&self, exposure: f32, warmth: f32, as_jpeg: bool) -> Result<Frame> {
        let stack = EditStack {
            exposure,
            warmth,
            ..EditStack::default()
        };
        let edge = fitted_edge(&self.session, &stack, self.screen);
        let (width, height) = self.session.framed_proxy(&stack, edge);
        let t = Instant::now();
        let rgba =
            self.pipeline
                .render_to_pixels(&self.gpu, &self.session, &stack, width, height)?;
        let render_ms = ms(t);
        let t = Instant::now();
        let bytes = if as_jpeg {
            jpeg(&rgba, width, height)?
        } else {
            rgba
        };
        Ok(Frame {
            width,
            height,
            bytes,
            render_ms,
            encode_ms: if as_jpeg { ms(t) } else { 0.0 },
        })
    }
}

/// The longer edge of the photo fitted inside the screen: a landscape photo
/// on an upright phone is as wide as the screen, not as tall.
fn fitted_edge(session: &DevelopSession, stack: &EditStack, screen: (u32, u32)) -> u32 {
    let (w, h) = session.framed_size(stack);
    let scale = (screen.0 as f64 / w as f64).min(screen.1 as f64 / h as f64);
    ((w.max(h) as f64 * scale).round() as u32).max(1)
}

/// `source` enlarged to `edge` px on its longer side and saved as a JPEG at
/// `to`: a stand-in for a full-size phone photo (12 MP at 4,000 px).
pub fn enlarge(source: &Path, to: &Path, edge: u32) -> Result<()> {
    let img = image::open(source).with_context(|| format!("reading {}", source.display()))?;
    let (w, h) = (img.width(), img.height());
    let scale = edge as f32 / w.max(h) as f32;
    let big = img.resize_exact(
        (w as f32 * scale).round() as u32,
        (h as f32 * scale).round() as u32,
        image::imageops::FilterType::Triangle,
    );
    let mut file = std::io::BufWriter::new(std::fs::File::create(to)?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 90)
        .encode_image(&big.to_rgb8())?;
    Ok(())
}

/// A frame as the editor sends one (`encode` in mobile/src-tauri/src/edit.rs):
/// quality 85, colour at half resolution, straight from RGBA.
fn jpeg(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, 85);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    encoder.encode(
        rgba,
        u16::try_from(width)?,
        u16::try_from(height)?,
        jpeg_encoder::ColorType::Rgba,
    )?;
    Ok(out)
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}
