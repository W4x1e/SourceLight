//! The develop render graph.
//!
//! Two passes with very different costs, which is the whole design:
//!
//! * **Mosaic** (compute, once per photo) — levels, as-shot white balance and
//!   demosaic, into a cached `rgba16float` linear image.
//! * **Develop** (render, once per interaction) — user white balance, exposure,
//!   colour transforms and display encode.
//!
//! Because the expensive pass is cached, moving a slider costs one fullscreen
//! draw over a proxy-sized target. Output resolution is a parameter of the
//! develop pass rather than a branch inside it, so the identical shader serves
//! the interactive preview and a full-resolution export — which is what keeps
//! "what you see is what you get" true.

use anyhow::{bail, Context, Result};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::color::{self, Mat3};
use super::Gpu;
use crate::develop::geometry::ViewPort;
use crate::develop::{curve, grading, mixer, tone, EditStack};
use crate::raw::decoder::{CfaPattern, RawImage};

/// A render as the develop pass read it back: tightly packed RGBA, eight or
/// sixteen bits a channel.
pub enum RenderedPixels {
    Eight(Vec<u8>),
    Sixteen(Vec<u16>),
}

/// Longest edge of the interactive proxy. Large enough to look sharp on a
/// typical editing window, small enough that a full render stays well inside a
/// frame budget.
pub const PROXY_EDGE: u32 = 1400;

/// Long edge of a brush mask's coverage texture. Coverage is smooth, so it does
/// not need the photo's resolution — a modest map keeps rasterising and
/// uploading cheap while staying smooth under the sampler.
const MASK_COVERAGE_EDGE: u32 = 512;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct MosaicParams {
    active_size: [u32; 2],
    active_origin: [u32; 2],
    period: u32,
    _pad0: u32,
    _pad1: [u32; 2],
    black: [f32; 4],
    white: [f32; 4],
    wb: [f32; 4],
    tile: [[u32; 4]; 9],
}

/// `highlights.wgsl`'s uniform.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct HighlightParams {
    /// Where each channel clips after white balance: the balance's gains.
    ceiling: [f32; 4],
    /// `[levels in the chroma chain, unused, unused, unused]`.
    levels: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct DevelopParams {
    cam_to_working: [[f32; 4]; 3],
    working_to_display: [[f32; 4]; 3],
    exposure: f32,
    tone_min_stops: f32,
    tone_max_stops: f32,
    tone_grey: f32,
    tone_lut_size: f32,
    mixer_lut_size: f32,
    grade_lut_size: f32,
    channel_lut_size: f32,
    orientation: f32,
    chroma_preserve: f32,
    /// Creative-look LUT grid size, and 0..1 blend (0 = no LUT). These two
    /// scalars sit exactly where the alignment padding before `wb` used to be —
    /// twelve scalars fill 96..144 and `wb` (a vec4) still lands at 144 — so the
    /// uniform's size is unchanged. See the matching note in `develop.wgsl`.
    lut_size: f32,
    lut_intensity: f32,
    wb: [f32; 4],
    crop_matrix: [f32; 4],
    /// Only `.xy` is read; a `vec2` would still be 8-byte aligned in WGSL and
    /// leave the tail of the struct at an awkward size, so this is a `vec4`.
    crop_centre: [f32; 4],
    /// `[texture amount, clarity amount, texture lod, clarity lod]`.
    detail: [f32; 4],
    /// `[sharpen amount, sharpen radius, sharpen masking, noise reduction]`.
    sharpen: [f32; 4],
    /// `[distortion, vignetting, chromatic aberration, unused]`.
    lens: [f32; 4],
    /// `[colour-noise amount, chroma-blur lod, luma-coarse lod, sharpen detail]`.
    nr: [f32; 4],
    /// `[post-crop vignette, vignette midpoint, grain, grain size]`.
    effects: [f32; 4],
    /// `[dehaze amount, atmospheric-light mip level, soft-proof flag,
    /// output transfer (0 sRGB curve, 1 gamma 2.2)]`.
    dehaze: [f32; 4],
    /// `[halation amount, halation threshold, grain highlight roll-off,
    /// rendering intent: 0 relative colorimetric, 1 perceptual]`.
    film: [f32; 4],
    /// `[vibrance, saturation, output luma red, output luma green]`. Global
    /// colour intensity, applied after the mixer and grade; the last two are
    /// the output space's luminance weights for `gamut_map` (blue is the rest).
    color: [f32; 4],
    /// `[perceptual mixer on, colour ranges read OkLCh, unused, unused]`, as
    /// 0 or 1: how a version-4 edit reads hue (`develop::oklab`).
    hue: [f32; 4],
}

// A mismatch here is reported by wgpu as "bound with size N where the shader
// expects M", which is clear but only shows up at draw time on a machine with a
// GPU. Pinning the size makes it a compile error instead.
const _: () = assert!(std::mem::size_of::<DevelopParams>() == 336);

/// Shaders, pipelines and the sampler. Built once and reused for every photo.
pub struct Pipeline {
    mosaic: wgpu::ComputePipeline,
    mosaic_layout: wgpu::BindGroupLayout,
    /// Blown highlights rebuilt from the colour around them, between the
    /// mosaic and the mip chain (`highlights.wgsl`).
    highlights: wgpu::ComputePipeline,
    highlights_layout: wgpu::BindGroupLayout,
    downsample: wgpu::ComputePipeline,
    downsample_layout: wgpu::BindGroupLayout,
    develop: wgpu::RenderPipeline,
    develop_layout: wgpu::BindGroupLayout,
    /// Kept so a presenter can build a second pipeline for its swapchain's
    /// format without re-parsing the shader.
    develop_module: wgpu::ShaderModule,
    /// The develop pass into [`Gpu::deep_format`], for sixteen-bit exports.
    /// Built the first time one is asked for, so opening the app does not
    /// compile the shader twice for a feature most sessions never use.
    develop_deep: std::sync::OnceLock<wgpu::RenderPipeline>,
    sampler: wgpu::Sampler,
    /// The baked tone curve. Allocated once and rewritten per render — it is
    /// 4 KB, so uploading it every frame is cheaper than tracking whether the
    /// tone controls happened to change.
    tone_lut: wgpu::Texture,
    tone_lut_view: wgpu::TextureView,
    mixer_lut: wgpu::Texture,
    mixer_lut_view: wgpu::TextureView,
    grade_lut: wgpu::Texture,
    grade_lut_view: wgpu::TextureView,
    channel_lut: wgpu::Texture,
    channel_lut_view: wgpu::TextureView,
    /// The creative-look 3D LUT, a fixed `LUT_3D_DIM³` RGBA32F cube. Created once
    /// (identity) and rewritten only when an active LUT is rendered.
    lut_3d: wgpu::Texture,
    lut_3d_view: wgpu::TextureView,
}

/// One photo, demosaiced and resident on the GPU.
pub struct DevelopSession {
    linear: wgpu::TextureView,
    /// The texture behind `linear`, kept so AI noise reduction can read it
    /// back and write its result in its place ([`Pipeline::read_linear`]).
    texture: wgpu::Texture,
    /// The texture's own size and mip count: sensor order, before rotation.
    stored: (u32, u32),
    mips: u32,
    /// Displayed dimensions, i.e. **after** EXIF rotation. These are what the
    /// UI lays out against and what an export produces; the underlying texture
    /// is still in sensor order, and the develop pass rotates as it samples.
    pub width: u32,
    pub height: u32,
    cam_to_working: Mat3,
    orientation: u8,
    /// The camera's baseline look, resolved from its model at prepare time and
    /// composed into the mixer LUT on every render. Identity for cameras with
    /// no profile, which is most of them today.
    profile: crate::develop::LookProfile,
    /// Where the photo opens on the exposure scale, before the user's own
    /// exposure. Like `profile`, the file's and not the edit's.
    baseline: Baseline,
    /// The selected creative LUT, already resampled to the fixed grid and ready
    /// to upload (`develop::lut::LUT_3D_DIM³` RGBA texels). `None` = no LUT. Set
    /// by the app layer (`DevelopState`) when the stack's LUT changes — the GPU
    /// session just carries the resolved bytes, never touches the filesystem.
    pub lut_texels: Option<Vec<f32>>,
    /// False for a JPEG, PNG or TIFF: no sensor data behind it, so far
    /// less to recover from the highlights and shadows.
    pub from_raw: bool,
    /// The linear image as it was decoded, kept once AI noise reduction has
    /// replaced it ([`Pipeline::keep_original`]), so "Before" shows the photo
    /// and not the denoised one. `None` until then.
    original: Option<(wgpu::Texture, wgpu::TextureView)>,
}

/// Where a photo opens on the exposure scale, in stops before the user's own
/// exposure.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Baseline {
    /// The placement that matches the camera's own JPEG (or a DNG's
    /// `BaselineExposure` when there is none to match).
    pub camera: f32,
}

impl Baseline {
    /// The stops `stack` renders with. A stack saved before version 2 was
    /// edited against no baseline, and keeps that: opening the camera's
    /// brightness must not move a photo someone already finished.
    pub fn for_stack(&self, stack: &EditStack) -> f32 {
        if stack.camera_matched() {
            self.camera
        } else {
            0.0
        }
    }
}

/// The median green photosite of a raw, normalised to its white level: the
/// scene half of `ToneParams::exposure_matching`. Sampled on a coarse grid of
/// whole tiles, so every colour phase is seen and the cost is a few thousand
/// reads.
pub fn scene_median(raw: &RawImage) -> Option<f32> {
    let a = raw.active_area;
    let period = raw.cfa_pattern.period();
    let stride = (period * 8).max(8);
    let (black, white) = (raw.black[1] as f32, raw.white[1] as f32);
    let range = (white - black).max(1.0);
    let mut samples = Vec::new();
    for ty in (0..a.height.saturating_sub(period)).step_by(stride as usize) {
        for tx in (0..a.width.saturating_sub(period)).step_by(stride as usize) {
            for dy in 0..period {
                for dx in 0..period {
                    if !matches!(raw.cfa_pattern.color_at(tx + dx, ty + dy), 1 | 3) {
                        continue;
                    }
                    if let Some(v) = raw.at(a.x + tx + dx, a.y + ty + dy) {
                        samples.push((v as f32 - black) / range);
                    }
                }
            }
        }
    }
    if samples.is_empty() {
        return None;
    }
    let mid = samples.len() / 2;
    let (_, median, _) = samples.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    Some(*median)
}

/// The median green of an sRGB render, in linear light: the render half of
/// matching a camera's JPEG (`Pipeline::open_at_camera`).
fn display_median(rgba: &[u8]) -> f32 {
    let lut = crate::raster::srgb_lut8();
    let mut greens: Vec<f32> = rgba.chunks_exact(4).map(|p| lut[p[1] as usize]).collect();
    if greens.is_empty() {
        return 0.0;
    }
    let mid = greens.len() / 2;
    *greens.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1
}

/// Number of mip levels needed to reach a single texel.
///
/// The chain has to be complete rather than stopping at some "good enough"
/// level: the develop pass renders at whatever size the window asks for,
/// including the small previews the export dialog and tests use, and a missing
/// level means the hardware falls back to the nearest one it has and aliases
/// again.
fn mip_levels(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// The smallest whole factor that brings `(width, height)` within `limit` on
/// both sides: 1 for anything that already fits.
fn downscale_factor((width, height): (u32, u32), limit: u32) -> u32 {
    width.max(height).div_ceil(limit.max(1)).max(1)
}

/// The cached linear image for a photo of `width` x `height`, with its full mip
/// chain. Written by the mosaic pass for a raw, or uploaded directly for a
/// rendered photo, hence both usages.
fn linear_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, u32) {
    let mips = mip_levels(width, height);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("linear rgb"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            // Read back for AI noise reduction.
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    (texture, mips)
}

/// A view of exactly one mip level, which is what a storage binding requires
/// and what the downsample pass reads.
fn mip_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

/// `--color-canvas-void` (#0e0e0e), the canvas behind the photo, as the value a
/// non-sRGB surface stores — the develop shader's output is already
/// sRGB-encoded, so the clear is too.
const CANVAS_VOID: wgpu::Color = wgpu::Color {
    r: 14.0 / 255.0,
    g: 14.0 / 255.0,
    b: 14.0 / 255.0,
    a: 1.0,
};

/// Fits `width` x `height` inside a box of `max_edge`, preserving aspect ratio
/// and never upscaling.
///
/// A free function rather than a method so the sizing rule can be tested
/// without a GPU or a live session.
pub fn fit_within(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_edge || longest == 0 {
        return (width, height);
    }
    let scale = max_edge as f32 / longest as f32;
    (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    )
}

/// As `orient` in develop.wgsl: the camera's EXIF orientation, applied to UV.
fn orient(orientation: u8, (u, v): (f32, f32)) -> (f32, f32) {
    match orientation {
        2 => (1.0 - u, v),
        3 => (1.0 - u, 1.0 - v),
        4 => (u, 1.0 - v),
        5 => (v, u),
        6 => (v, 1.0 - u),
        7 => (1.0 - v, 1.0 - u),
        8 => (1.0 - v, u),
        _ => (u, v),
    }
}

/// As `lens_distort` in develop.wgsl: `k` is the shader's distortion
/// strength, and the radius is measured in pixels of the stored image, a
/// corner being 1.
fn lens_distort((u, v): (f32, f32), k: f32, (w, h): (u32, u32)) -> (f32, f32) {
    if k.abs() < 1e-6 {
        return (u, v);
    }
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let (dx, dy) = ((u - 0.5) * w, (v - 0.5) * h);
    let r2 = (dx * dx + dy * dy) / (0.25 * (w * w + h * h));
    let s = 1.0 + k * r2;
    (0.5 + dx * s / w, 0.5 + dy * s / h)
}

/// [`lens_distort`] run backwards. A few fixed-point steps settle it: at the
/// strongest the slider allows, the scale changes by a quarter across the
/// whole frame.
fn lens_undistort((u, v): (f32, f32), k: f32, (w, h): (u32, u32)) -> (f32, f32) {
    if k.abs() < 1e-6 {
        return (u, v);
    }
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let (sx, sy) = ((u - 0.5) * w, (v - 0.5) * h);
    let corner = 0.25 * (w * w + h * h);
    let (mut dx, mut dy) = (sx, sy);
    for _ in 0..8 {
        let s = 1.0 + k * (dx * dx + dy * dy) / corner;
        if s.abs() < 1e-6 {
            break;
        }
        (dx, dy) = (sx / s, sy / s);
    }
    (0.5 + dx / w, 0.5 + dy / h)
}

impl DevelopSession {
    /// The photo's own colours into the working space: camera RGB for a raw.
    pub fn cam_to_working(&self) -> Mat3 {
        self.cam_to_working
    }

    /// The linear image's own size, in sensor order: `width`/`height` swapped
    /// for a photo turned a quarter.
    pub fn stored_size(&self) -> (u32, u32) {
        self.stored
    }

    /// Where a point of the framed photo (UV, as the viewer's pan gives it)
    /// lies in the stored image, as fractions of [`Self::stored_size`]: the
    /// develop pass's crop, turn and straighten, then the camera's
    /// orientation, run backwards on the CPU. Lens corrections are left out:
    /// it steers AI noise reduction's tile order, where near is near enough.
    pub fn stored_uv(&self, stack: &EditStack, framed: (f32, f32)) -> (f32, f32) {
        let (u, v) = self.oriented_uv(stack, framed);
        (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
    }

    /// [`Self::stored_uv`] unclamped.
    fn oriented_uv(&self, stack: &EditStack, framed: (f32, f32)) -> (f32, f32) {
        let t = stack.geometry.transform(self.width, self.height);
        let m = t.matrix;
        let (px, py) = (framed.0 - 0.5, framed.1 - 0.5);
        orient(
            self.orientation,
            (
                m[0] * px + m[1] * py + t.centre[0],
                m[2] * px + m[3] * py + t.centre[1],
            ),
        )
    }

    /// Exactly where the develop pass reads the stored image for a point of
    /// the framed photo: [`Self::stored_uv`]'s mapping and then the lens's
    /// distortion about the sensor's centre. Unclamped, in UV of
    /// [`Self::stored_size`].
    pub fn sample_uv(&self, stack: &EditStack, framed: (f32, f32)) -> (f32, f32) {
        lens_distort(
            self.oriented_uv(stack, framed),
            stack.lens.to_shader()[0],
            self.stored,
        )
    }

    /// The point of the framed photo that shows the stored image's pixel at
    /// `stored` (UV): [`Self::sample_uv`] run backwards. How a mask drawn on
    /// the framed photo is carried onto the sensor's own pixels. Outside 0..1
    /// for a pixel the crop leaves out.
    pub fn framed_uv(&self, stack: &EditStack, stored: (f32, f32)) -> (f32, f32) {
        let o = lens_undistort(stored, stack.lens.to_shader()[0], self.stored);
        // Every orientation is its own inverse but the two quarter turns,
        // which undo each other.
        let back = match self.orientation {
            6 => 8,
            8 => 6,
            o => o,
        };
        let (u, v) = orient(back, o);
        let t = stack.geometry.transform(self.width, self.height);
        let m = t.matrix;
        let det = m[0] * m[3] - m[1] * m[2];
        if det.abs() < 1e-12 {
            return (0.5, 0.5);
        }
        let (du, dv) = (u - t.centre[0], v - t.centre[1]);
        (
            (m[3] * du - m[1] * dv) / det + 0.5,
            (-m[2] * du + m[0] * dv) / det + 0.5,
        )
    }

    fn new(
        linear: &wgpu::Texture,
        (width, height): (u32, u32),
        cam_to_working: Mat3,
        orientation: u8,
        profile: crate::develop::LookProfile,
        from_raw: bool,
    ) -> Self {
        // Orientations 5..=8 involve a transpose, so the displayed frame is the
        // stored frame with its axes swapped.
        let rotated = matches!(orientation, 5..=8);
        DevelopSession {
            // The whole chain, so the develop pass can pick a level matching
            // the proxy it is drawing.
            linear: linear.create_view(&Default::default()),
            texture: linear.clone(),
            stored: (width, height),
            mips: linear.mip_level_count(),
            width: if rotated { height } else { width },
            height: if rotated { width } else { height },
            cam_to_working,
            orientation,
            profile,
            baseline: Baseline::default(),
            lut_texels: None,
            from_raw,
            original: None,
        }
    }

    /// Stops the photo opens at before the user's exposure, under `stack`.
    pub fn baseline(&self, stack: &EditStack) -> f32 {
        self.baseline.for_stack(stack)
    }

    /// How `stack`'s warmth and tint take this photo into the working space:
    /// gains on the camera's channels, then the camera's matrix. From version
    /// 3 the gains are one and the matrix carries an adaptation from the light
    /// they describe (`develop::white_balance`), the same for every camera; an
    /// edit saved before scales the channels as it always did.
    pub fn white_balance(&self, stack: &EditStack) -> ([f32; 3], Mat3) {
        if stack.by_temperature() {
            let adapt = crate::develop::white_balance::adaptation(stack.warmth, stack.tint);
            ([1.0; 3], color::multiply(adapt, self.cam_to_working))
        } else {
            (stack.wb_scale(), self.cam_to_working)
        }
    }

    /// This session drawing from the linear image as decoded, if it has been
    /// replaced: what "Before" renders. `None` when it never was.
    pub fn as_decoded(&self) -> Option<DevelopSession> {
        let (texture, view) = self.original.as_ref()?;
        Some(DevelopSession {
            linear: view.clone(),
            texture: texture.clone(),
            stored: self.stored,
            mips: self.mips,
            width: self.width,
            height: self.height,
            cam_to_working: self.cam_to_working,
            orientation: self.orientation,
            profile: self.profile,
            baseline: self.baseline,
            lut_texels: self.lut_texels.clone(),
            from_raw: self.from_raw,
            original: None,
        })
    }

    /// Output size for a preview whose longest edge is `max_edge`, ignoring
    /// any crop. Kept for tests and callers holding no edit stack.
    pub fn proxy_size(&self, max_edge: u32) -> (u32, u32) {
        fit_within(self.width, self.height, max_edge)
    }

    /// Full output size once the stack's crop and rotation are applied.
    ///
    /// This, not `width`/`height`, is what an export produces and what the UI
    /// has to lay out against — those two stay the uncropped frame because the
    /// crop is expressed as a fraction of it.
    pub fn framed_size(&self, stack: &EditStack) -> (u32, u32) {
        stack.geometry.output_size(self.width, self.height)
    }

    /// Preview size for a stack, respecting its crop.
    pub fn framed_proxy(&self, stack: &EditStack, max_edge: u32) -> (u32, u32) {
        let (w, h) = self.framed_size(stack);
        fit_within(w, h, max_edge)
    }
}

impl Pipeline {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;

        let mosaic_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mosaic"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mosaic.wgsl").into()),
        });
        let develop_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("develop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/develop.wgsl").into()),
        });
        let downsample_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("downsample"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/downsample.wgsl").into()),
        });
        let highlights_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("highlights"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/highlights.wgsl").into()),
        });

        let mosaic_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mosaic"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let highlights_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("highlights"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let downsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("downsample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        // Read with `textureLoad`, so filtering is never asked
                        // for and the format need not support it.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let develop_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("develop"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Local-adjustment masks.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Brush coverage: one array layer per brush mask.
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                // Spot / heal / clone patches.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Per-channel (R/G/B) point curves.
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Creative-look 3D LUT. Sampled with `textureLoad` + manual
                // trilinear (like the 1D LUTs), so the Rgba32Float format needs no
                // float-filtering feature.
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let mosaic = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("mosaic"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("mosaic"),
                    bind_group_layouts: &[Some(&mosaic_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &mosaic_module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let highlights = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("highlights"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("highlights"),
                    bind_group_layouts: &[Some(&highlights_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &highlights_module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let downsample = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("downsample"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("downsample"),
                    bind_group_layouts: &[Some(&downsample_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &downsample_module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let develop = develop_pipeline(
            device,
            &develop_module,
            &develop_layout,
            wgpu::TextureFormat::Rgba8Unorm,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("develop"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Without this the sampler stays on level 0 whatever the chain
            // holds, and the proxy aliases exactly as it did before.
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // A 1 x N strip rather than a true 1D texture: 2D sampling is
        // universally supported and the linear sampler interpolates between
        // entries for free.
        let tone_lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tone lut"),
            size: wgpu::Extent3d {
                width: tone::LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let tone_lut_view = tone_lut.create_view(&Default::default());

        // Rgba32Float because each entry carries three numbers — hue shift,
        // saturation and luminance — rather than the tone table's one. Two
        // rows: the HSV-indexed one the camera profile (and an older edit's
        // mixer) reads, and the OkLCh-indexed one a version-4 mixer reads.
        let mixer_lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mixer lut"),
            size: wgpu::Extent3d {
                width: mixer::LUT_SIZE as u32,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mixer_lut_view = mixer_lut.create_view(&Default::default());

        // Indexed by luminance rather than hue, and holding an RGB multiplier.
        // Smaller than the others because the grade is a slow function of
        // brightness with no fine structure to resolve.
        let grade_lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("grade lut"),
            size: wgpu::Extent3d {
                width: grading::LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let grade_lut_view = grade_lut.create_view(&Default::default());

        // Per-channel (R/G/B) point curves, one RGBA entry per input value.
        let channel_lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("channel lut"),
            size: wgpu::Extent3d {
                width: curve::CHANNEL_LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let channel_lut_view = channel_lut.create_view(&Default::default());

        // Creative-look 3D LUT: a fixed cube, seeded to identity so the binding is
        // always valid even before any LUT is chosen.
        let dim = crate::develop::lut::LUT_3D_DIM as u32;
        let lut_3d = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("look lut 3d"),
            size: wgpu::Extent3d {
                width: dim,
                height: dim,
                depth_or_array_layers: dim,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lut_3d_view = lut_3d.create_view(&Default::default());

        let pipeline = Self {
            mosaic,
            mosaic_layout,
            highlights,
            highlights_layout,
            downsample,
            downsample_layout,
            develop_module,
            develop_deep: std::sync::OnceLock::new(),
            develop,
            develop_layout,
            sampler,
            tone_lut,
            tone_lut_view,
            mixer_lut,
            mixer_lut_view,
            grade_lut,
            grade_lut_view,
            channel_lut,
            channel_lut_view,
            lut_3d,
            lut_3d_view,
        };
        pipeline.upload_lut_3d(
            gpu,
            &crate::develop::lut::identity_texels(crate::develop::lut::LUT_3D_DIM),
        );
        pipeline
    }

    /// Writes a `LUT_3D_DIM³` cube of RGBA32F texels into the look LUT texture.
    fn upload_lut_3d(&self, gpu: &Gpu, texels: &[f32]) {
        let dim = crate::develop::lut::LUT_3D_DIM as u32;
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.lut_3d,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dim * 4 * 4), // width * 4 channels * 4 bytes
                rows_per_image: Some(dim),
            },
            wgpu::Extent3d {
                width: dim,
                height: dim,
                depth_or_array_layers: dim,
            },
        );
    }

    fn upload_channel_lut(&self, gpu: &Gpu, lut: &[f32]) {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.channel_lut,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(lut),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((curve::CHANNEL_LUT_SIZE * 4 * 4) as u32),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: curve::CHANNEL_LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    fn upload_grade_lut(&self, gpu: &Gpu, grading: &grading::ColorGrading) {
        let lut = grading.build_lut();
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.grade_lut,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&lut),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((grading::LUT_SIZE * 16) as u32),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: grading::LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Takes the finished table rather than the parameters, because the
    /// parametric curve is no longer the only thing in it — `EditStack` owns
    /// the composition with the point curve.
    fn upload_tone_lut(&self, gpu: &Gpu, lut: &[f32]) {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.tone_lut,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(lut),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((tone::LUT_SIZE * 4) as u32),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: tone::LUT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    fn upload_mixer_lut(
        &self,
        gpu: &Gpu,
        mixer: &mixer::ColorMixer,
        perceptual: bool,
        profile: &crate::develop::LookProfile,
    ) {
        // The camera profile sits *under* the user's mixer, so a photo opens
        // with its look applied and the sliders adjust on top of it. For an
        // older edit both share the first row, as they always did; for a
        // version-4 one the profile keeps that row, in the HSV hue it was
        // measured in, and the mixer has the second, in OkLCh hue.
        let (legacy, perceptual) = if perceptual {
            (
                profile.build_lut_under(&mixer::ColorMixer::default()),
                mixer.build_perceptual_lut(),
            )
        } else {
            (
                profile.build_lut_under(mixer),
                vec![[0.0; 4]; mixer::LUT_SIZE],
            )
        };
        let lut = [legacy, perceptual].concat();
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.mixer_lut,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&lut),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((mixer::LUT_SIZE * 16) as u32),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: mixer::LUT_SIZE as u32,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Camera primaries into the working space for a raw, normalised so
    /// neutral stays neutral.
    fn raw_to_working(raw: &RawImage) -> Mat3 {
        // Falls back to the naive composition only if the matrix will not
        // invert, which means it was degenerate and the colour is already lost.
        color::camera_to_working(raw.cam_to_xyz).unwrap_or_else(|| {
            color::normalise_white(color::multiply(color::XYZ_TO_REC2020, raw.cam_to_xyz))
        })
    }

    /// Where a raw opens, in stops on top of the user's exposure: at the
    /// brightness of its camera's own JPEG, failing that a DNG's
    /// `BaselineExposure`.
    ///
    /// Estimated from the sensor's median through the tone curve alone
    /// (`ToneParams::exposure_matching`), then measured on a small render and
    /// corrected. The camera matrix, look profile and highlight handling move
    /// a frame's median too — half a stop on the Panasonic in the fixtures —
    /// and it is the render that has to match. Two corrections at most, each
    /// a proxy render of a few milliseconds.
    fn open_at_camera(&self, gpu: &Gpu, session: &mut DevelopSession, raw: &RawImage) {
        session.baseline = Baseline {
            camera: raw.baseline_exposure,
        };
        let estimate = raw
            .camera_median
            .zip(scene_median(raw))
            .and_then(|(camera, scene)| {
                tone::ToneParams::exposure_matching(scene, camera).map(|stops| (camera, stops))
            });
        let Some((camera, mut stops)) = estimate else {
            return;
        };
        // Measured with no baseline, so the stack's exposure is all of it.
        session.baseline = Baseline::default();
        let (w, h) = session.proxy_size(256);
        for _ in 0..2 {
            let stack = EditStack {
                exposure: stops,
                ..EditStack::default()
            };
            let Ok(pixels) = self.render_to_pixels(gpu, session, &stack, w, h) else {
                break;
            };
            let got = display_median(&pixels);
            let Some(step) =
                tone::ToneParams::exposure_matching(tone::ToneParams::neutral_inverse(got), camera)
            else {
                break;
            };
            stops = (stops + step).clamp(-2.0, 2.0);
            if step.abs() < 0.02 {
                break;
            }
        }
        session.baseline = Baseline { camera: stops };
    }

    /// Uploads a raw that has no mosaic (#linear DNG) as the linear image the
    /// mosaic pass would have made: the same levels, as-shot balance and
    /// blown-highlight handling as `mosaic.wgsl`, on the CPU, with no
    /// demosaic to run.
    fn prepare_linear(&self, gpu: &Gpu, raw: &RawImage) -> Result<DevelopSession> {
        use rayon::prelude::*;
        let area = raw.active_area;
        let (width, height) = (area.width, area.height);
        let (linear, mips) = linear_texture(&gpu.device, width, height);
        let params = mosaic_params(raw);
        let halves: Vec<u16> = (0..height)
            .into_par_iter()
            .flat_map_iter(|y| {
                let start = (((area.y + y) * raw.width + area.x) * 3) as usize;
                let row = &raw.cfa[start..start + (width * 3) as usize];
                let params = &params;
                row.chunks_exact(3).flat_map(move |px| {
                    let rgb = linear_pixel(px, params);
                    [rgb[0], rgb[1], rgb[2], 1.0].map(|v| half::f16::from_f32(v).to_bits())
                })
            })
            .collect();
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &linear,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&halves),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4 * 2),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("linear raw mips"),
            });
        self.encode_mips(&gpu.device, &mut encoder, &linear, width, height, mips);
        gpu.queue.submit([encoder.finish()]);
        let mut session = DevelopSession::new(
            &linear,
            (width, height),
            Self::raw_to_working(raw),
            raw.orientation,
            crate::develop::LookProfile::for_camera(&raw.make, &raw.model),
            true,
        );
        self.open_at_camera(gpu, &mut session, raw);
        Ok(session)
    }

    /// Uploads the sensor data and runs the mosaic pass. Expensive, and done
    /// once per photo rather than once per edit.
    pub fn prepare(&self, gpu: &Gpu, raw: &RawImage) -> Result<DevelopSession> {
        // A mosaic cannot be scaled down before it is demosaiced, so a sensor
        // past the card's texture limit is refused up front: it would fail
        // validation, and wgpu's answer to that is a panic. The limit
        // is 16,384 px on every GPU targeted, above any single-shot sensor.
        let limit = gpu.max_edge();
        if raw.width.max(raw.height) > limit {
            bail!(
                "this raw is {} × {} px, and this graphics card can develop at most {limit} px on a side",
                raw.width,
                raw.height
            );
        }
        if matches!(raw.cfa_pattern, CfaPattern::Linear) {
            return self.prepare_linear(gpu, raw);
        }
        let device = &gpu.device;
        let area = raw.active_area;

        let cfa = device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("cfa"),
                size: wgpu::Extent3d {
                    width: raw.width,
                    height: raw.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&raw.cfa),
        );

        let (linear, mips) = linear_texture(device, area.width, area.height);
        // The mosaic's own output, before highlights are rebuilt into
        // `linear`, and the colour map they borrow from, with its chain.
        let scratch = |label, levels| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: area.width,
                    height: area.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let demosaiced = scratch("demosaiced", 1);
        let chroma = scratch("highlight chroma", mips);

        let params = mosaic_params(raw);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mosaic params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let cfa_view = cfa.create_view(&Default::default());
        // A storage binding must name exactly one mip level, so the passes
        // write through views of level 0 while the develop pass samples a
        // view covering the whole chain.
        let linear_view = mip_view(&linear, 0);
        let demosaiced_view = demosaiced.create_view(&Default::default());
        let chroma_base = mip_view(&chroma, 0);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mosaic"),
            layout: &self.mosaic_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&cfa_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&demosaiced_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&chroma_base),
                },
            ],
        });
        let highlight_params = HighlightParams {
            ceiling: params.wb,
            levels: [mips as f32, 0.0, 0.0, 0.0],
        };
        let highlight_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("highlight params"),
            contents: bytemuck::bytes_of(&highlight_params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let chroma_view = chroma.create_view(&Default::default());
        let highlight_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("highlights"),
            layout: &self.highlights_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&demosaiced_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&chroma_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&linear_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: highlight_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mosaic"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mosaic"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.mosaic);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(area.width.div_ceil(8), area.height.div_ceil(8), 1);
        }
        self.encode_mips(device, &mut encoder, &chroma, area.width, area.height, mips);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("highlights"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.highlights);
            pass.set_bind_group(0, &highlight_group, &[]);
            pass.dispatch_workgroups(area.width.div_ceil(8), area.height.div_ceil(8), 1);
        }
        self.encode_mips(device, &mut encoder, &linear, area.width, area.height, mips);
        gpu.queue.submit([encoder.finish()]);

        let mut session = DevelopSession::new(
            &linear,
            (area.width, area.height),
            Self::raw_to_working(raw),
            raw.orientation,
            crate::develop::LookProfile::for_camera(&raw.make, &raw.model),
            true,
        );
        self.open_at_camera(gpu, &mut session, raw);
        Ok(session)
    }

    /// Uploads an already-rendered photo — a JPEG, PNG or TIFF — as the
    /// linear image the mosaic pass would have produced, so everything after
    /// it runs unchanged.
    ///
    /// The pixels are display light, and the develop pass expects scene light
    /// that its tone curve will render. So each one is taken into the working
    /// space and the neutral curve is undone ([`tone::ToneParams::neutral_inverse`]);
    /// at neutral settings the curve then gives back exactly what the file
    /// held. Rendered sessions tone per channel (see `chroma_preserve` in
    /// [`Self::develop_bind_group`]), which is what makes that exact.
    ///
    /// Written by band, converting to half floats as it goes: the linear
    /// texture is `rgba16float`, and a whole-frame float copy of a 24MP JPEG
    /// would be 400 MB of memory for the length of one upload.
    pub fn prepare_rendered(
        &self,
        gpu: &Gpu,
        img: &crate::raster::RenderedImage,
    ) -> Result<DevelopSession> {
        const BAND: u32 = 256;
        // Source rows read per band, at most: 256 rows of a 16,384 px frame
        // as float RGBA. Keeps a wide panorama's band as small as a photo's.
        const BAND_BYTES: u64 = 64 << 20;
        let device = &gpu.device;
        let (source_width, source_height) = img.size();
        // A stitched panorama or a large scan can be wider than the card can
        // hold a texture. It is developed at the largest size that
        // fits, by a whole factor so each texel averages a k × k block.
        let k = downscale_factor((source_width, source_height), gpu.max_edge());
        let (width, height) = (source_width / k, source_height / k);
        // A strip narrower than `k` would make a texture with no texels.
        anyhow::ensure!(
            width > 0 && height > 0,
            "{source_width} × {source_height} px is too narrow to develop"
        );
        if k > 1 {
            eprintln!(
                "develop: {source_width} × {source_height} px is past the GPU's {} px limit; developing at 1/{k} ({width} × {height})",
                gpu.max_edge()
            );
        }
        let (linear, mips) = linear_texture(device, width, height);

        use rayon::prelude::*;
        // The file's own primaries (sRGB unless its profile says otherwise).
        let to_working =
            color::normalise_white(color::multiply(color::XYZ_TO_REC2020, img.profile.to_xyz));
        let band = BAND_BYTES / (u64::from(k) * u64::from(source_width) * 16);
        let band = band.clamp(1, u64::from(BAND)) as u32;
        let (mut rows, mut halves) = (Vec::new(), Vec::<u16>::new());
        let mut y = 0;
        while y < height {
            let n = band.min(height - y);
            img.linear_rows(y * k, n * k, &mut rows);
            halves.clear();
            halves.resize((width * n * 4) as usize, 0);
            let rows = &rows;
            halves.par_chunks_mut(4).enumerate().for_each(|(i, out)| {
                // The k × k block of source pixels behind this texel,
                // averaged in linear light, before the tone curve's
                // inverse: after it, the mean would not be of the light.
                let (x, r) = ((i as u32 % width) * k, (i as u32 / width) * k);
                let mut sum = [0.0f32; 3];
                for dy in 0..k {
                    let row = ((r + dy) * source_width + x) as usize * 4;
                    for px in rows[row..row + k as usize * 4].chunks_exact(4) {
                        for (s, v) in sum.iter_mut().zip(px) {
                            *s += v;
                        }
                    }
                }
                let scale = 1.0 / (k * k) as f32;
                let working = color::apply(to_working, sum.map(|v| v * scale));
                for (o, v) in out.iter_mut().zip(working) {
                    let scene = tone::ToneParams::neutral_inverse(v.max(0.0));
                    *o = half::f16::from_f32(scene).to_bits();
                }
                out[3] = half::f16::ONE.to_bits();
            });
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &linear,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&halves),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4 * 2), // 4 channels of f16
                    rows_per_image: Some(n),
                },
                wgpu::Extent3d {
                    width,
                    height: n,
                    depth_or_array_layers: 1,
                },
            );
            y += n;
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rendered mips"),
        });
        self.encode_mips(device, &mut encoder, &linear, width, height, mips);
        gpu.queue.submit([encoder.finish()]);

        // Already in the working space: converted above, on the way in.
        const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        Ok(DevelopSession::new(
            &linear,
            (width, height),
            IDENTITY,
            img.orientation,
            // The camera already applied its look; adding ours would double it.
            crate::develop::LookProfile::IDENTITY,
            false,
        ))
    }

    /// The session's linear image as RGBA half floats (their bits), in the
    /// texture's own order and size ([`DevelopSession::stored_size`]): what AI
    /// noise reduction works on.
    pub fn read_linear(&self, gpu: &Gpu, session: &DevelopSession) -> Result<Vec<u16>> {
        let (w, h) = session.stored;
        self.read_linear_region(gpu, session, (0, 0, w, h), false)
    }

    /// A rectangle `(x, y, width, height)` of the linear image, as
    /// [`Self::read_linear`] gives the whole. With `decoded`, from the image as
    /// decoded ([`Self::keep_original`]) when it has been replaced since.
    pub fn read_linear_region(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        (x, y, w, h): (u32, u32, u32, u32),
        decoded: bool,
    ) -> Result<Vec<u16>> {
        let (sw, sh) = session.stored;
        anyhow::ensure!(
            w > 0 && h > 0 && x + w <= sw && y + h <= sh,
            "{w}x{h} at ({x}, {y}) is outside the {sw}x{sh} image"
        );
        let texture = match &session.original {
            Some((original, _)) if decoded => original,
            _ => &session.texture,
        };
        let bytes = gpu.read_texture_region(texture, (x, y), w, h, 8)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect())
    }

    /// Writes `rgba` over a rectangle of the linear image's full-size level,
    /// leaving its mips as they were: [`Self::rebuild_mips`] once every
    /// region is written.
    pub fn write_linear_region(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        (x, y, w, h): (u32, u32, u32, u32),
        rgba: &[u16],
    ) -> Result<()> {
        let (sw, sh) = session.stored;
        anyhow::ensure!(
            w > 0 && h > 0 && x + w <= sw && y + h <= sh,
            "{w}x{h} at ({x}, {y}) is outside the {sw}x{sh} image"
        );
        anyhow::ensure!(
            rgba.len() == (w * h * 4) as usize,
            "region is {} values, expected {}",
            rgba.len(),
            w * h * 4
        );
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &session.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(rgba),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4 * 2),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Rebuilds the linear image's mip chain from its full-size level.
    pub fn rebuild_mips(&self, gpu: &Gpu, session: &DevelopSession) {
        let (w, h) = session.stored;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rewritten mips"),
            });
        self.encode_mips(
            &gpu.device,
            &mut encoder,
            &session.texture,
            w,
            h,
            session.mips,
        );
        gpu.queue.submit([encoder.finish()]);
    }

    /// Copies the session's linear image, every mip level, into a texture of
    /// its own before it is first replaced, so "Before" can still show it
    /// ([`DevelopSession::as_decoded`]). A GPU-to-GPU copy, and a no-op once
    /// kept. About 250 MB of video memory for a 24 MP photo, held only while a
    /// denoised photo is open.
    pub fn keep_original(&self, gpu: &Gpu, session: &mut DevelopSession) {
        if session.original.is_some() {
            return;
        }
        let (w, h) = session.stored;
        let (copy, mips) = linear_texture(&gpu.device, w, h);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("keep original"),
            });
        for level in 0..mips.min(session.mips) {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &session.texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &copy,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: (w >> level).max(1),
                    height: (h >> level).max(1),
                    depth_or_array_layers: 1,
                },
            );
        }
        gpu.queue.submit([encoder.finish()]);
        let view = copy.create_view(&Default::default());
        session.original = Some((copy, view));
    }

    /// Replaces the session's linear image with `rgba` (as [`Self::read_linear`]
    /// returns it) and rebuilds the mip chain, so every render after it, the
    /// proxy and an export alike, develops the new pixels.
    pub fn write_linear(&self, gpu: &Gpu, session: &DevelopSession, rgba: &[u16]) -> Result<()> {
        let (w, h) = session.stored;
        self.write_linear_region(gpu, session, (0, 0, w, h), rgba)?;
        self.rebuild_mips(gpu, session);
        Ok(())
    }

    /// Whether the linear image as decoded is kept apart from the one
    /// rendered ([`Self::keep_original`]).
    pub fn keeps_original(&self, session: &DevelopSession) -> bool {
        session.original.is_some()
    }

    /// Opens either kind of photo.
    ///
    /// GPU errors come back as an `Err` rather than wgpu's panic: a
    /// photo this card cannot hold is refused with a reason, not a crash.
    pub fn prepare_source(
        &self,
        gpu: &Gpu,
        source: &crate::raster::Source,
    ) -> Result<DevelopSession> {
        gpu.checked("opening this photo", || match source {
            crate::raster::Source::Raw(raw) => self.prepare(gpu, raw),
            crate::raster::Source::Rendered(img) => self.prepare_rendered(gpu, img),
        })
    }

    /// Builds the linear image's mip chain. Each level reads the one above, so
    /// they have to run in order — separate passes rather than one dispatch,
    /// since a compute pass gives no ordering guarantee within it.
    fn encode_mips(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        linear: &wgpu::Texture,
        width: u32,
        height: u32,
        mips: u32,
    ) {
        let level_views: Vec<wgpu::TextureView> = (0..mips).map(|l| mip_view(linear, l)).collect();
        let mut level_groups = Vec::new();
        for level in 1..mips {
            level_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("downsample"),
                layout: &self.downsample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &level_views[level as usize - 1],
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&level_views[level as usize]),
                    },
                ],
            }));
        }
        for (i, group) in level_groups.iter().enumerate() {
            let level = i as u32 + 1;
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("downsample"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.downsample);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
    }

    /// Runs the develop pass into a fresh `Rgba8Unorm` texture and returns it.
    ///
    /// Cheap: one fullscreen draw. `width`/`height` choose the resolution, so
    /// the same call serves a proxy preview and a full-resolution export.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_texture(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
        view: Option<&ViewPort>,
        soft_proof: bool,
    ) -> wgpu::Texture {
        self.render_to_texture_in(
            gpu,
            session,
            stack,
            width,
            height,
            view,
            soft_proof,
            color::Output::SRGB,
        )
    }

    /// [`render_to_texture`](Self::render_to_texture) into a chosen output
    /// space: its matrix replaces the Rec.2020 → sRGB one, and its transfer
    /// curve the sRGB encode.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_texture_in(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
        view: Option<&ViewPort>,
        soft_proof: bool,
        output: color::Output,
    ) -> wgpu::Texture {
        let bind_group =
            self.develop_bind_group(gpu, session, stack, width, height, view, soft_proof, output);
        let target = gpu.create_render_target(width, height);
        self.draw_develop(gpu, &self.develop, &bind_group, &target, None, None);
        target
    }

    /// Uploads the per-frame tables and builds the develop pass's bind group.
    ///
    /// Shared by the offscreen and direct paths so the two cannot drift: every
    /// parameter a photo is rendered with is decided exactly once, here.
    #[allow(clippy::too_many_arguments)]
    fn develop_bind_group(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
        view: Option<&ViewPort>,
        soft_proof: bool,
        output: color::Output,
    ) -> wgpu::BindGroup {
        let device = &gpu.device;

        self.upload_tone_lut(gpu, &stack.tone_lut());
        self.upload_mixer_lut(
            gpu,
            &stack.mixer(),
            stack.perceptual_hue(),
            &session.profile,
        );
        self.upload_grade_lut(gpu, &stack.grading);
        self.upload_channel_lut(gpu, &stack.channel_lut());

        // Creative look LUT: active only when one is resolved *and* the intensity
        // is non-zero. Upload its cube when active; otherwise leave the texture as
        // it is and pass intensity 0, which makes the shader stage an exact no-op.
        let lut_intensity = match &session.lut_texels {
            Some(texels) if stack.lut.intensity > 0.0 => {
                self.upload_lut_3d(gpu, texels);
                stack.lut.intensity.min(1.0)
            }
            _ => 0.0,
        };

        let (wb, cam_to_working) = session.white_balance(stack);
        let crop = {
            let base = stack.geometry.transform(session.width, session.height);
            match view {
                Some(v) => base.through(v, session.framed_size(stack)),
                None => base,
            }
        };
        // Against the source frame and the *rendered* size, so the floor knows
        // how much this particular render is downsampling.
        let lods =
            crate::develop::Detail::lods(session.width.max(session.height), width.max(height));
        let params = DevelopParams {
            cam_to_working: color::to_wgsl(cam_to_working),
            working_to_display: color::to_wgsl(output.working_to_output),
            // The user's exposure on top of where the photo opens.
            exposure: (stack.exposure + session.baseline(stack)).exp2(),
            tone_min_stops: tone::LUT_MIN_STOPS,
            tone_max_stops: tone::LUT_MAX_STOPS,
            tone_grey: tone::MIDDLE_GREY_IN,
            tone_lut_size: tone::LUT_SIZE as f32,
            mixer_lut_size: mixer::LUT_SIZE as f32,
            grade_lut_size: grading::LUT_SIZE as f32,
            channel_lut_size: curve::CHANNEL_LUT_SIZE as f32,
            orientation: session.orientation as f32,
            // A rendered photo was brought in by undoing the curve channel by
            // channel, so only per-channel toning puts it back exactly.
            chroma_preserve: if session.from_raw {
                tone::CHROMA_PRESERVE
            } else {
                0.0
            },
            lut_size: crate::develop::lut::LUT_3D_DIM as f32,
            lut_intensity,
            wb: [wb[0], wb[1], wb[2], 1.0],
            // Built against the *oriented* frame, which is the one the crop is
            // expressed in and the one the user drags on.
            crop_matrix: crop.matrix,
            crop_centre: [crop.centre[0], crop.centre[1], 0.0, 0.0],
            // Levels chosen against the *source* frame, so the preview and a
            // full-size export sample the same blur and therefore agree.
            detail: [
                stack.detail.texture,
                stack.detail.clarity,
                lods.texture,
                lods.clarity,
            ],
            sharpen: [
                stack.detail.sharpen,
                stack.detail.sharpen_radius,
                stack.detail.sharpen_masking,
                stack.detail.noise,
            ],
            lens: stack.lens.to_shader(),
            nr: [
                stack.detail.color_noise,
                lods.noise_chroma,
                lods.noise_luma,
                stack.detail.sharpen_detail,
            ],
            effects: stack.effects.to_shader(),
            // `.w` was unused: it carries the rendering intent.
            film: {
                let mut film = stack.effects.to_film_shader();
                film[3] = if output.perceptual { 1.0 } else { 0.0 };
                film
            },
            // `.z` carries the soft-proof (gamut-check) flag; the shader paints
            // out-of-gamut pixels a marker when it is set. Never on for an export
            // or a thumbnail — only the interactive view proofs.
            dehaze: [
                stack.detail.dehaze,
                lods.dehaze,
                if soft_proof { 1.0 } else { 0.0 },
                // `.w` picks the encode: the sRGB curve, or Adobe RGB's pure
                // gamma. It was unused, so the uniform's layout is unchanged.
                match output.transfer {
                    color::Transfer::Srgb => 0.0,
                    color::Transfer::Gamma22 => 1.0,
                },
            ],
            color: [
                stack.vibrance,
                stack.saturation,
                output.luma_rg[0],
                output.luma_rg[1],
            ],
            hue: [
                f32::from(u8::from(
                    stack.perceptual_hue() && !stack.mixer().is_neutral(),
                )),
                f32::from(u8::from(stack.perceptual_hue())),
                0.0,
                0.0,
            ],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("develop params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Masks are placed in framed-image UV, so under a zoom the drawn-quad UV
        // has to be mapped back to it — the same `(extent, centre)` the crop
        // transform is built from. Identity at fit-to-window.
        let mask_view = match view {
            Some(v) => {
                let (ext, cen) = v.framed_mapping(session.framed_size(stack));
                [ext[0], ext[1], cen[0], cen[1]]
            }
            None => crate::develop::mask::IDENTITY_VIEW,
        };
        let masks = crate::develop::mask::pack_showing(&stack.masks, mask_view, stack.show_mask);
        let mask_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("develop masks"),
            contents: bytemuck::bytes_of(&masks),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Coverage texture array: one layer per brush *or AI* component. Brush
        // layers are rasterised on the CPU from strokes; AI layers are decoded
        // from their stored bitmap and resized to fit. Always `MAX_COMPONENTS`
        // layers so the binding matches the shader's indexing; 1x1 when there are
        // none, so the common case allocates almost nothing. Sized to the frame
        // aspect (long edge `MASK_COVERAGE_EDGE`) so a round brush stays round.
        let coverage_layers = crate::develop::mask::coverage_layers(&stack.masks);
        let (cw, ch) = if coverage_layers.is_empty() {
            (1, 1)
        } else {
            let (fw, fh) = session.framed_size(stack);
            fit_within(fw, fh, MASK_COVERAGE_EDGE)
        };
        let coverage = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mask coverage"),
            size: wgpu::Extent3d {
                width: cw,
                height: ch,
                depth_or_array_layers: crate::develop::mask::MAX_COMPONENTS as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, source) in &coverage_layers {
            let bytes = match source {
                crate::develop::mask::CoverageSource::Brush(strokes) => {
                    crate::develop::mask::rasterize(strokes, cw, ch)
                }
                crate::develop::mask::CoverageSource::Ai {
                    data,
                    width,
                    height,
                } => crate::develop::mask::resize_coverage(data, *width, *height, cw, ch),
            };
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &coverage,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: *layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(cw),
                    rows_per_image: Some(ch),
                },
                wgpu::Extent3d {
                    width: cw,
                    height: ch,
                    depth_or_array_layers: 1,
                },
            );
        }
        let coverage_view = coverage.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        // Spot / heal / clone patches, packed with the same framed-UV mapping as
        // the masks so a spot stays pinned to the photo under a zoom.
        let spots = crate::develop::spot::pack(&stack.spots, mask_view);
        let spot_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("develop spots"),
            contents: bytemuck::bytes_of(&spots),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("develop"),
            layout: &self.develop_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&session.linear),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.tone_lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.mixer_lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&self.grade_lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: mask_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&coverage_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: spot_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&self.channel_lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&self.lut_3d_view),
                },
            ],
        });

        bind_group
    }

    /// A develop pipeline targeting `format`.
    ///
    /// The presenter owns the result: a swapchain's format is a property of the
    /// window, not of this pipeline, and caching one here would mean guessing
    /// which window it belonged to.
    pub fn develop_pipeline_for(
        &self,
        gpu: &Gpu,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        develop_pipeline(
            &gpu.device,
            &self.develop_module,
            &self.develop_layout,
            format,
        )
    }

    /// Runs the develop pass into a texture the caller owns — a swapchain
    /// image, in practice.
    ///
    /// Deliberately the *same* parameter construction and bind group as the
    /// offscreen path, reached through `develop_params`. A presenter that built
    /// its own would be free to drift, and the direct and readback paths would
    /// stop agreeing about what a photo looks like.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_surface(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        target: &wgpu::Texture,
        pipeline: &wgpu::RenderPipeline,
        origin: (u32, u32),
        width: u32,
        height: u32,
        view: Option<&ViewPort>,
        soft_proof: bool,
        left_of: Option<u32>,
    ) {
        // The on-screen canvas is sRGB until display colour management lands.
        let bind_group = self.develop_bind_group(
            gpu,
            session,
            stack,
            width,
            height,
            view,
            soft_proof,
            color::Output::SRGB,
        );
        self.draw_develop(
            gpu,
            pipeline,
            &bind_group,
            target,
            Some((origin, (width, height))),
            left_of,
        );
    }

    /// `viewport` restricts the draw to part of the target, for a surface that
    /// covers a whole window. `None` fills it, which is what an offscreen
    /// target sized to the frame wants.
    ///
    /// `left_of` draws only the part of the photo left of that column, over
    /// what the target already holds rather than clearing it: the before half
    /// of the split view, laid over the after drawn first.
    fn draw_develop(
        &self,
        gpu: &Gpu,
        pipeline: &wgpu::RenderPipeline,
        bind_group: &wgpu::BindGroup,
        target: &wgpu::Texture,
        viewport: Option<((u32, u32), (u32, u32))>,
        left_of: Option<u32>,
    ) {
        let view = target.create_view(&Default::default());

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("develop"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("develop"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // A surface is the whole canvas, and what the photo
                        // leaves uncovered is its letterbox: the page's
                        // `--color-canvas-void`, not black. An offscreen
                        // target is exactly the photo, so it never shows.
                        load: match (left_of, viewport.is_some()) {
                            (Some(_), _) => wgpu::LoadOp::Load,
                            (None, true) => wgpu::LoadOp::Clear(CANVAS_VOID),
                            (None, false) => wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            let mut return_early = false;
            if let Some(((x, y), (w, h))) = viewport {
                // **Clamped to the target, because wgpu panics otherwise.** The
                // rectangle arrives from the webview, where the position and
                // the size are each rounded from CSS pixels independently — so
                // `x + w` can land a pixel past the surface even when both are
                // right. A validation panic here is not recoverable: this can
                // run inside a COM callback, where a panic cannot unwind and
                // aborts the process.
                let (limit_w, limit_h) = (target.width(), target.height());
                let x = x.min(limit_w.saturating_sub(1));
                let y = y.min(limit_h.saturating_sub(1));
                let w = w.min(limit_w - x).max(1);
                let h = h.min(limit_h - y).max(1);

                // Scissor as well as viewport: the viewport transforms the
                // triangle, but without a scissor the clear still covers the
                // whole surface and would wipe out anything else drawn there.
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                match left_of {
                    // Nothing of the photo left of the line: draw nothing.
                    Some(edge) if edge <= x => return_early = true,
                    Some(edge) => pass.set_scissor_rect(x, y, (edge - x).min(w), h),
                    None => pass.set_scissor_rect(x, y, w, h),
                }
            }
            if !return_early {
                pass.draw(0..3, 0..1);
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// Renders and reads back as tightly packed RGBA8, showing the whole frame.
    pub fn render_to_pixels(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        // Thumbnails and exports never soft-proof — the gamut check is a viewing
        // aid, so it is off for every path but the interactive one.
        self.render_view(gpu, session, stack, width, height, None, false)
    }

    /// An export's render: into its colour space (`output`), at sixteen bits
    /// a channel when the export asks for them.
    #[allow(clippy::too_many_arguments)]
    pub fn render_for_export(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
        output: color::Output,
        sixteen_bit: bool,
    ) -> Result<RenderedPixels> {
        // A full-size frame is the largest target and readback the app makes,
        // so the one most likely to meet the card's memory.
        gpu.checked("exporting this photo", || {
            Ok(if sixteen_bit {
                RenderedPixels::Sixteen(
                    self.render_to_pixels16_in(gpu, session, stack, width, height, output)?,
                )
            } else {
                RenderedPixels::Eight(
                    self.render_to_pixels_in(gpu, session, stack, width, height, output)?,
                )
            })
        })
    }

    /// [`render_to_pixels_in`](Self::render_to_pixels_in) at sixteen bits a
    /// channel, for a PNG or TIFF export that asks for them: the same pass,
    /// parameters and bind group, into a [`Gpu::deep_format`] target. RGBA,
    /// 0..=65535 for 0..1.
    pub fn render_to_pixels16_in(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
        output: color::Output,
    ) -> Result<Vec<u16>> {
        let format = gpu.deep_format();
        let pipeline = self
            .develop_deep
            .get_or_init(|| self.develop_pipeline_for(gpu, format));
        self.render_deep(
            gpu,
            pipeline,
            format,
            session,
            stack,
            (width, height),
            output,
        )
    }

    /// The sixteen-bit render, through `pipeline`, which draws into `format`.
    #[allow(clippy::too_many_arguments)]
    fn render_deep(
        &self,
        gpu: &Gpu,
        pipeline: &wgpu::RenderPipeline,
        format: wgpu::TextureFormat,
        session: &DevelopSession,
        stack: &EditStack,
        (width, height): (u32, u32),
        output: color::Output,
    ) -> Result<Vec<u16>> {
        let bind_group =
            self.develop_bind_group(gpu, session, stack, width, height, None, false, output);
        let target = gpu.create_render_target_as(width, height, format);
        self.draw_develop(gpu, pipeline, &bind_group, &target, None, None);
        gpu.read_rgba16(&target, width, height)
            .context("reading back the sixteen-bit develop pass")
    }

    /// [`render_to_pixels`](Self::render_to_pixels) into a chosen output space
    /// — how an export renders Display P3 rather than sRGB.
    pub fn render_to_pixels_in(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
        output: color::Output,
    ) -> Result<Vec<u8>> {
        let target =
            self.render_to_texture_in(gpu, session, stack, width, height, None, false, output);
        gpu.read_rgba8(&target, width, height)
            .context("reading back the develop pass")
    }

    /// The same, restricted to what a viewport is looking at.
    ///
    /// Zoom costs nothing: the viewport folds into the crop matrix the develop
    /// pass already applies, so a 1:1 view is the same single fullscreen draw
    /// as a fit-to-window one — it simply reads a smaller region of the cached
    /// mosaic, and reads it at a finer mip level.
    #[allow(clippy::too_many_arguments)]
    pub fn render_view(
        &self,
        gpu: &Gpu,
        session: &DevelopSession,
        stack: &EditStack,
        width: u32,
        height: u32,
        view: Option<&ViewPort>,
        soft_proof: bool,
    ) -> Result<Vec<u8>> {
        let target = self.render_to_texture(gpu, session, stack, width, height, view, soft_proof);
        gpu.read_rgba8(&target, width, height)
            .context("reading back the develop pass")
    }
}

/// Builds the develop render pipeline for a given target format.
///
/// Factored out because a swapchain rarely uses the same format as the
/// offscreen target — `Bgra8Unorm` is the usual one on Windows — and wgpu
/// requires a pipeline's target format to match the attachment exactly. The
/// shader is identical either way; only the declared target differs.
fn develop_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("develop"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("develop"),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            }),
        ),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs"),
            // Callers pass an Unorm format, never UnormSrgb: the shader encodes
            // sRGB explicitly as a pipeline stage, and letting the hardware do
            // it as well would encode twice and wash the image out.
            targets: &[Some(format.into())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn mosaic_params(raw: &RawImage) -> MosaicParams {
    // Tile of colour indices. X-Trans needs all 36; Bayer fills the first 4 and
    // the shader only reads what `period` implies.
    let mut tile = [[0u32; 4]; 9];
    let (period, indices): (u32, Vec<u8>) = match &raw.cfa_pattern {
        CfaPattern::Bayer(t) => (2, t.to_vec()),
        CfaPattern::XTrans(t) => (6, t.to_vec()),
        // No mosaic pass runs; `linear_pixel` reads the levels from here.
        CfaPattern::Linear => (1, vec![0]),
    };
    for (i, c) in indices.iter().enumerate() {
        tile[i / 4][i % 4] = *c as u32;
    }

    // Normalise the as-shot multipliers against green so white balance changes
    // colour without also changing overall brightness — exposure is the only
    // control that should do that.
    let green = if raw.wb_coeffs[1] > 0.0 {
        raw.wb_coeffs[1]
    } else {
        1.0
    };
    let mut wb = [0.0f32; 4];
    for (i, w) in wb.iter_mut().enumerate() {
        *w = raw.wb_coeffs[i] / green;
    }

    MosaicParams {
        active_size: [raw.active_area.width, raw.active_area.height],
        active_origin: [raw.active_area.x, raw.active_area.y],
        period,
        _pad0: 0,
        _pad1: [0; 2],
        black: raw.black.map(|b| b as f32),
        white: raw.white.map(|w| w as f32),
        wb,
        tile,
    }
}

/// One pixel of a linear raw (red, green, blue values): levels normalised,
/// the as-shot balance applied as `mosaic.wgsl`'s `tap` does, and a blown
/// highlight made neutral.
///
/// Neutral rather than rebuilt from its surroundings, as a mosaiced raw's
/// highlights are (`highlights.wgsl`): a linear DNG never goes through the
/// mosaic pass that measures the colour to borrow. It is a phone's merged
/// frame or an editor's output, whose highlights were handled before it was
/// written, so this is the fallback for the few clipped pixels left: green
/// saturating on something near neutral goes white, as every raw's did.
fn linear_pixel(px: &[u16], p: &MosaicParams) -> [f32; 3] {
    let v: [f32; 3] = std::array::from_fn(|c| {
        let range = (p.white[c] - p.black[c]).max(1.0);
        (px[c] as f32 - p.black[c]) / range * p.wb[c]
    });
    let smoothstep = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let saturated = smoothstep(0.88, 1.0, v[1] / p.wb[1].max(1e-6));
    let neutralish = smoothstep(0.5, 0.8, v[0].min(v[2]) / v[1].max(1e-6));
    let k = saturated * neutralish;
    let peak = v[0].max(v[1]).max(v[2]);
    v.map(|c| c + (peak - c) * k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::ToneParams;
    use crate::raw::decoder::Rect;

    /// Skips on a software adapter as well as a missing one — see
    /// `gpu::test_support::hardware_gpu`. These are the tests that took the
    /// binary down on WARP.
    fn gpu() -> Option<&'static Gpu> {
        crate::gpu::test_support::hardware_gpu()
    }

    /// A uniform, perfectly neutral synthetic sensor.
    ///
    /// The per-channel values are chosen so that after black subtraction and
    /// the as-shot white balance every channel lands on exactly `level`:
    /// red 500/2000 x 2, green 1000/2000 x 1, blue 250/2000 x 4. A uniform
    /// scene also makes the result independent of the demosaic algorithm, so
    /// this pins the *pipeline* rather than the interpolation.
    ///
    /// The tile is RGGB written as `[0, 1, 3, 2]` — index 3 being the second
    /// green, exactly as raw metadata reports it — so the shader's handling of
    /// that fourth channel is exercised rather than assumed.
    fn neutral_sensor(level: f32) -> RawImage {
        let (w, h) = (16u32, 16u32);
        let scale = level / 0.5;
        let mut cfa = vec![0u16; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let value = match (y % 2, x % 2) {
                    (0, 0) => 500.0, // red
                    (1, 1) => 250.0, // blue
                    _ => 1000.0,     // green and second green
                };
                cfa[(y * w + x) as usize] = (value * scale) as u16;
            }
        }

        RawImage {
            cfa,
            width: w,
            height: h,
            cfa_pattern: CfaPattern::Bayer([0, 1, 3, 2]),
            black: [0; 4],
            white: [2000; 4],
            wb_coeffs: [2.0, 1.0, 4.0, 1.0],
            // Identity, so the only colour transform under test is
            // XYZ->Rec.2020 followed by Rec.2020->sRGB.
            cam_to_xyz: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            baseline_exposure: 0.0,
            camera_median: None,
            active_area: Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
            orientation: 1,
            make: "Synthetic".into(),
            model: "Neutral".into(),
        }
    }

    /// A neutral scene with one hard vertical edge down the middle.
    ///
    /// Neutral on both sides, so any colour in the output is manufactured by
    /// the demosaic rather than present in the scene. The edge runs between two
    /// mosaic columns, which is the alignment that gives interpolation the most
    /// trouble.
    fn edge_sensor() -> RawImage {
        edge_sensor_of(16)
    }

    /// The same scene at an arbitrary size.
    ///
    /// Local contrast needs a large fixture, because its radii are fractions of
    /// the image rather than pixel counts: on a 16-pixel frame a 2% radius
    /// rounds down to a single pixel, both mip levels collapse onto the source,
    /// and clarity correctly does nothing at all. That is right behaviour and a
    /// useless test.
    fn edge_sensor_of(size: u32) -> RawImage {
        let mut raw = neutral_sensor(0.5);
        let (w, h) = (size, size);
        let mut cfa = vec![0u16; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let base: u16 = match (y % 2, x % 2) {
                    (0, 0) => 500, // red
                    (1, 1) => 250, // blue
                    _ => 1000,     // green and second green
                };
                // Same neutral mix on both sides, eight times darker on the
                // left. Scaling keeps each side neutral by construction.
                cfa[(y * w + x) as usize] = if x < w / 2 { base / 8 } else { base };
            }
        }
        raw.cfa = cfa;
        raw.width = w;
        raw.height = h;
        raw.active_area = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        raw
    }

    /// The same neutral edge on an X-Trans mosaic.
    ///
    /// Uses the real Fuji tile, so the phase — which sites are red and blue,
    /// and where the green pairs fall — matches what the shader will actually
    /// meet. An invented layout would exercise code that no camera produces.
    fn xtrans_edge_sensor(size: u32) -> RawImage {
        // As reported by LibRaw for a Fuji X-T series sensor: 20 green, 8 red,
        // 8 blue, with green in every row and column.
        const TILE: [u8; 36] = [
            1, 1, 0, 1, 1, 2, //
            2, 0, 1, 0, 2, 1, //
            1, 1, 2, 1, 1, 0, //
            1, 1, 0, 1, 1, 2, //
            0, 2, 1, 2, 0, 1, //
            1, 1, 2, 1, 1, 0,
        ];

        let mut raw = edge_sensor_of(size);
        let (w, h) = (raw.width, raw.height);
        // Levels chosen so a neutral subject reads correctly once the same
        // white balance is applied, exactly as `neutral_sensor` does for Bayer.
        for y in 0..h {
            for x in 0..w {
                let base: u16 = match TILE[((y % 6) * 6 + (x % 6)) as usize] {
                    0 => 500,
                    2 => 250,
                    _ => 1000,
                };
                raw.cfa[(y * w + x) as usize] = if x < w / 2 { base / 8 } else { base };
            }
        }
        raw.cfa_pattern = CfaPattern::XTrans(Box::new(TILE));
        raw
    }

    /// A bright disc on a dark field, centred.
    ///
    /// The fixture a *radial* transform needs. A straight edge through the
    /// middle is useless for it: points on the centre line displace along the
    /// line itself, so the edge maps onto itself and distortion measures as
    /// doing nothing at all.
    fn disc_sensor(size: u32) -> RawImage {
        let mut raw = edge_sensor_of(size);
        let (w, h) = (raw.width, raw.height);
        let centre = (w as f32 / 2.0, h as f32 / 2.0);
        let cutoff = (w.min(h) as f32 * 0.3).powi(2);

        for y in 0..h {
            for x in 0..w {
                let base: u16 = match (y % 2, x % 2) {
                    (0, 0) => 500,
                    (1, 1) => 250,
                    _ => 1000,
                };
                let d = (x as f32 - centre.0).powi(2) + (y as f32 - centre.1).powi(2);
                raw.cfa[(y * w + x) as usize] = if d < cutoff { base } else { base / 8 };
            }
        }
        raw.cfa_pattern = CfaPattern::Bayer([0, 1, 3, 2]);
        raw
    }

    fn render(gpu: &Gpu, raw: &RawImage, stack: &EditStack) -> Vec<u8> {
        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, raw).expect("prepare");
        pipeline
            .render_to_pixels(gpu, &session, stack, session.width, session.height)
            .expect("render")
    }

    /// A sixteen-bit render is the eight-bit one with the steps filled in: the
    /// same picture to within a level, and a smooth ramp keeps far more
    /// distinct tones than eight bits can.
    #[test]
    fn a_sixteen_bit_render_is_the_eight_bit_one_finer() {
        let Some(gpu) = gpu() else { return };
        let (w, h) = (512u32, 8u32);
        let mut raw = neutral_sensor(0.5);
        raw.width = w;
        raw.height = h;
        raw.active_area = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        raw.cfa = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    let base = match (y % 2, x % 2) {
                        (0, 0) => 500.0,
                        (1, 1) => 250.0,
                        _ => 1000.0,
                    };
                    (base * (0.02 + 1.4 * x as f32 / w as f32)) as u16
                })
            })
            .collect();
        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let stack = EditStack::default();
        let (w, h) = (session.width, session.height);
        let eight = pipeline
            .render_to_pixels_in(gpu, &session, &stack, w, h, color::Output::SRGB)
            .expect("8-bit render");
        eprintln!("sixteen-bit exports render to {:?}", gpu.deep_format());
        let deep = pipeline
            .render_to_pixels16_in(gpu, &session, &stack, w, h, color::Output::SRGB)
            .expect("16-bit render");
        // The half-float target too, which is what a device without
        // sixteen-bit unorm targets renders to.
        let float = wgpu::TextureFormat::Rgba16Float;
        let halves = pipeline
            .render_deep(
                gpu,
                &pipeline.develop_pipeline_for(gpu, float),
                float,
                &session,
                &stack,
                (w, h),
                color::Output::SRGB,
            )
            .expect("half-float render");
        // Down the middle row, where the ramp runs.
        let tones = |v: &dyn Fn(usize) -> u32| {
            let y = h as usize / 2;
            let mut seen: Vec<u32> = (0..w as usize)
                .map(|x| v((y * w as usize + x) * 4 + 1))
                .collect();
            seen.sort_unstable();
            seen.dedup();
            seen.len()
        };
        let shallow = tones(&|i| eight[i] as u32);
        for sixteen in [&deep, &halves] {
            assert_eq!(eight.len(), sixteen.len());
            for (a, b) in eight.iter().zip(sixteen.iter()) {
                assert!(
                    (*a as f32 - *b as f32 / 257.0).abs() <= 1.0,
                    "{a} at eight bits, {b} at sixteen"
                );
            }
            let finer = tones(&|i| sixteen[i] as u32);
            assert!(
                finer > shallow * 3 / 2,
                "sixteen bits kept {finer} tones of the ramp, eight {shallow}"
            );
        }
    }

    /// Mean of each channel, ignoring the outermost ring where demosaic clamps
    /// against the edge.
    fn interior_mean(pixels: &[u8], w: u32, h: u32) -> [f64; 3] {
        let mut sums = [0f64; 3];
        let mut count = 0u32;
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                let i = ((y * w + x) * 4) as usize;
                for c in 0..3 {
                    sums[c] += pixels[i + c] as f64;
                }
                count += 1;
            }
        }
        sums.map(|s| s / count as f64)
    }

    /// The display byte a scene-linear value should reach, computed the same
    /// way the shader does: through the tone curve, then the sRGB encode.
    ///
    /// Deriving it from `ToneParams` rather than hard-coding a number makes
    /// this a check that the GPU reproduces the CPU reference curve — so the
    /// tone maths stays owned by `tone.rs`, where it is exhaustively tested,
    /// and this only has to prove the shader samples it correctly.
    fn expected_display(linear: f32, tone: &ToneParams) -> f64 {
        let toned = tone.map_linear(linear);
        let encoded = if toned <= 0.0031308 {
            toned * 12.92
        } else {
            1.055 * toned.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0) as f64
    }

    /// The end-to-end assertion: a neutral scene must come out neutral, at the
    /// brightness the maths predicts. This covers black/white levels, as-shot
    /// white balance, demosaic, the camera-to-working and working-to-display
    /// matrices, and the sRGB encode — any one of them wrong moves this.
    #[test]
    fn a_neutral_scene_renders_neutral_at_the_predicted_level() {
        let Some(gpu) = gpu() else { return };
        let raw = neutral_sensor(0.5);
        let pixels = render(gpu, &raw, &EditStack::default());

        let mean = interior_mean(&pixels, 16, 16);
        let expected = expected_display(0.5, &EditStack::default().tone());
        eprintln!("neutral: {mean:?}, expected ~{expected:.1}");

        for (c, name) in ["red", "green", "blue"].iter().enumerate() {
            assert!(
                (mean[c] - expected).abs() < 3.0,
                "{name} came out {:.1}, expected {expected:.1} — full channel means {mean:?}",
                mean[c]
            );
        }
        // Neutral means the channels agree with each other, not merely that
        // each is near the target.
        let spread = mean.iter().cloned().fold(f64::MIN, f64::max)
            - mean.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread < 2.0, "channels disagree by {spread:.2}: {mean:?}");
    }

    /// One stop of exposure must be exactly a doubling of scene light.
    ///
    /// Stated as an invariant rather than an expected pixel value: brightening
    /// a scene by a stop has to land in the same place as a scene that was
    /// twice as bright to begin with. That holds whatever the tone curve does,
    /// which makes it immune to the curve being retuned — and it still catches
    /// exposure being applied after the display transform, where a stop is no
    /// longer a factor of two.
    /// A JPEG opened in Develop looks as it did: at neutral settings the
    /// pipeline gives back the file's own pixels — greys, saturated colour and
    /// near-white alike — rotated like a raw. Exposure still works on it.
    #[test]
    fn a_rendered_photo_opens_as_it_was() {
        let Some(gpu) = gpu() else { return };
        use crate::raster::{Pixels, RenderedImage};
        let pipeline = Pipeline::new(gpu);
        let open = |rgb: [u8; 3], orientation: u8| {
            let img = RenderedImage::new(
                Pixels::Eight(image::ImageBuffer::from_pixel(16, 8, image::Rgb(rgb))),
                orientation,
                None,
            );
            pipeline.prepare_rendered(gpu, &img).expect("prepare")
        };
        let mean = |session: &DevelopSession, stack: &EditStack| {
            let (w, h) = (session.width, session.height);
            interior_mean(
                &pipeline
                    .render_to_pixels(gpu, session, stack, w, h)
                    .expect("render"),
                w,
                h,
            )
        };

        let rotated = open([150; 3], 6);
        assert_eq!(
            (rotated.width, rotated.height),
            (8, 16),
            "rotated like a raw"
        );
        assert!(!rotated.from_raw);

        for rgb in [
            [150, 150, 150],
            [200, 60, 30],
            [40, 90, 210],
            [250, 248, 245],
            [8, 8, 8],
        ] {
            let got = mean(&open(rgb, 1), &EditStack::default());
            eprintln!("{rgb:?} -> {got:?}");
            for c in 0..3 {
                assert!(
                    (got[c] - rgb[c] as f64).abs() <= 2.0,
                    "{rgb:?} came back {got:?}"
                );
            }
        }

        let brighter = mean(
            &open([150; 3], 1),
            &EditStack {
                exposure: 1.0,
                ..Default::default()
            },
        );
        assert!(
            brighter[1] > 175.0 && brighter[1] < 250.0,
            "a stop up brightens without clipping: {brighter:?}"
        );
    }

    /// AI noise reduction reads the linear image back and writes its result in
    /// its place: the texture's own order and size, a lossless round
    /// trip, and every render after the write develops the new pixels, down
    /// to the mip levels a small preview samples.
    #[test]
    fn the_linear_image_reads_back_and_can_be_replaced() {
        let Some(gpu) = gpu() else { return };
        use crate::raster::{Pixels, RenderedImage};
        let pipeline = Pipeline::new(gpu);
        let img = RenderedImage::new(
            Pixels::Eight(image::ImageBuffer::from_pixel(
                40,
                24,
                image::Rgb([150, 150, 150]),
            )),
            6,
            None,
        );
        let session = pipeline.prepare_rendered(gpu, &img).expect("prepare");
        assert_eq!(session.stored_size(), (40, 24), "sensor order, not turned");
        assert_eq!((session.width, session.height), (24, 40));

        let linear = pipeline.read_linear(gpu, &session).expect("read");
        assert_eq!(linear.len(), 40 * 24 * 4);
        pipeline
            .write_linear(gpu, &session, &linear)
            .expect("write");
        assert_eq!(
            pipeline.read_linear(gpu, &session).unwrap(),
            linear,
            "lossless"
        );

        let mean = |w: u32, h: u32| {
            interior_mean(
                &pipeline
                    .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
                    .expect("render"),
                w,
                h,
            )[1]
        };
        let before = (mean(24, 40), mean(6, 10));
        // Half the light, as a (drastic) denoiser might hand back.
        let darker: Vec<u16> = linear
            .chunks(4)
            .flat_map(|p| {
                let half =
                    |b: u16| half::f16::from_f32(half::f16::from_bits(b).to_f32() * 0.5).to_bits();
                [half(p[0]), half(p[1]), half(p[2]), p[3]]
            })
            .collect();
        pipeline
            .write_linear(gpu, &session, &darker)
            .expect("write");
        let after = (mean(24, 40), mean(6, 10));
        assert!(
            after.0 < before.0 - 20.0,
            "full size: {before:?} -> {after:?}"
        );
        assert!(
            after.1 < before.1 - 20.0,
            "the mips too: {before:?} -> {after:?}"
        );
        assert!(pipeline.write_linear(gpu, &session, &darker[4..]).is_err());
    }

    /// The centre of the view maps to the right place in the stored image,
    /// through the camera's orientation and the user's quarter turns.
    #[test]
    fn a_framed_point_maps_to_the_stored_image() {
        let Some(gpu) = gpu() else { return };
        use crate::raster::{Pixels, RenderedImage};
        let pipeline = Pipeline::new(gpu);
        let open = |orientation: u8| {
            let img = RenderedImage::new(
                Pixels::Eight(image::ImageBuffer::from_pixel(
                    40,
                    24,
                    image::Rgb([150, 150, 150]),
                )),
                orientation,
                None,
            );
            pipeline.prepare_rendered(gpu, &img).expect("prepare")
        };
        let plain = EditStack::default();
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
        assert!(near(open(1).stored_uv(&plain, (0.1, 0.2)), (0.1, 0.2)));
        // Turned 90° clockwise by the camera: framed left is stored bottom.
        assert!(near(open(6).stored_uv(&plain, (0.1, 0.2)), (0.2, 0.9)));
        assert!(near(open(3).stored_uv(&plain, (0.1, 0.2)), (0.9, 0.8)));
        // The centre stays the centre, whatever the turn.
        let mut turned = EditStack::default();
        turned.geometry.quarter_turns = 1;
        assert!(near(open(1).stored_uv(&turned, (0.5, 0.5)), (0.5, 0.5)));
        // A corner under a quarter turn lands on a corner.
        let (u, v) = open(1).stored_uv(&turned, (0.0, 0.0));
        assert!(
            !(0.01..=0.99).contains(&u) && !(0.01..=0.99).contains(&v),
            "({u}, {v})"
        );
    }

    /// `framed_uv` undoes `sample_uv` under every orientation and through a
    /// crop, a straighten, a turn, a flip and lens distortion together: a
    /// mask carried onto the sensor lands where it was drawn.
    #[test]
    fn a_stored_point_maps_back_to_where_it_is_framed() {
        let Some(gpu) = gpu() else { return };
        use crate::raster::{Pixels, RenderedImage};
        let pipeline = Pipeline::new(gpu);
        let mut edit = EditStack::default();
        edit.geometry.crop = crate::develop::geometry::CropRect {
            x: 0.1,
            y: 0.2,
            width: 0.6,
            height: 0.5,
        };
        edit.geometry.straighten = 3.5;
        edit.geometry.quarter_turns = 1;
        edit.geometry.flip = true;
        edit.lens.distortion = 0.8;
        for orientation in 1..=8 {
            let session = pipeline
                .prepare_rendered(
                    gpu,
                    &RenderedImage::new(
                        Pixels::Eight(image::ImageBuffer::from_pixel(
                            60,
                            40,
                            image::Rgb([150, 150, 150]),
                        )),
                        orientation,
                        None,
                    ),
                )
                .expect("prepare");
            for stack in [&EditStack::default(), &edit] {
                for framed in [(0.5, 0.5), (0.1, 0.9), (0.95, 0.05), (0.3, 0.6)] {
                    let stored = session.sample_uv(stack, framed);
                    let back = session.framed_uv(stack, stored);
                    assert!(
                        (back.0 - framed.0).abs() < 1e-3 && (back.1 - framed.1).abs() < 1e-3,
                        "orientation {orientation}: {framed:?} -> {stored:?} -> {back:?}"
                    );
                }
            }
            // With nothing to undo, sampling is the clamped mapping.
            let plain = EditStack::default();
            assert_eq!(
                session.sample_uv(&plain, (0.2, 0.7)),
                session.stored_uv(&plain, (0.2, 0.7))
            );
        }
    }

    /// An Adobe RGB photo opens in Develop as the colours it holds: the
    /// develop pass starts from the profile's primaries, and at neutral
    /// settings lands where the two specifications say it should in sRGB.
    #[test]
    fn a_wide_gamut_photo_opens_in_its_own_colours() {
        let Some(gpu) = gpu() else { return };
        use crate::raster::{icc, Pixels, RenderedImage};
        let adobe = icc::build_for_test(
            icc::ADOBE_RGB_D50,
            &icc::gamma_curve_for_test(563.0 / 256.0),
        );
        let profile = icc::Profile::parse(&adobe);
        let pipeline = Pipeline::new(gpu);
        for rgb in [[100u8, 150, 200], [200, 60, 30], [128, 128, 128]] {
            let img = RenderedImage::new(
                Pixels::Eight(image::ImageBuffer::from_pixel(16, 8, image::Rgb(rgb))),
                1,
                profile.clone(),
            );
            let session = pipeline.prepare_rendered(gpu, &img).expect("prepare");
            let px = pipeline
                .render_to_pixels(gpu, &session, &EditStack::default(), 16, 8)
                .expect("render");
            let got = interior_mean(&px, 16, 8);
            let want = crate::raster::adobe_to_srgb_by_the_book(rgb);
            for c in 0..3 {
                assert!(
                    (got[c] - want[c] as f64).abs() <= 2.5,
                    "Adobe RGB {rgb:?} developed to {got:?}, the specifications say {want:?}"
                );
            }
        }
    }

    #[test]
    fn one_stop_of_exposure_equals_twice_the_light() {
        let Some(gpu) = gpu() else { return };

        let lifted = render(
            gpu,
            &neutral_sensor(0.25),
            &EditStack {
                exposure: 1.0,
                ..Default::default()
            },
        );
        let brighter_scene = render(gpu, &neutral_sensor(0.5), &EditStack::default());

        let a = interior_mean(&lifted, 16, 16)[1];
        let b = interior_mean(&brighter_scene, 16, 16)[1];
        eprintln!("0.25 linear at +1 EV -> {a:.1}; 0.5 linear at 0 EV -> {b:.1}");

        assert!(
            (a - b).abs() < 2.0,
            "+1 EV on 0.25 gave {a:.1} but 0.5 gave {b:.1} — a stop is not a doubling"
        );
    }

    /// The tone curve must actually be doing something, or a regression that
    /// bypassed it would leave every other assertion here still passing.
    #[test]
    fn the_tone_curve_is_applied() {
        let Some(gpu) = gpu() else { return };
        let raw = neutral_sensor(0.5);

        let neutral = interior_mean(&render(gpu, &raw, &EditStack::default()), 16, 16)[1];
        let punchy = interior_mean(
            &render(
                gpu,
                &raw,
                &EditStack {
                    contrast: 1.0,
                    ..Default::default()
                },
            ),
            16,
            16,
        )[1];

        eprintln!("contrast 0 -> {neutral:.1}, contrast +1 -> {punchy:.1}");
        // 0.5 linear sits above middle grey, so more contrast must brighten it.
        assert!(
            punchy > neutral + 2.0,
            "raising contrast did not change the render ({neutral:.1} -> {punchy:.1}) — \
             the tone LUT may not be reaching the shader"
        );
    }

    #[test]
    fn warming_shifts_red_up_and_blue_down() {
        let Some(gpu) = gpu() else { return };
        let raw = neutral_sensor(0.5);

        let neutral = interior_mean(&render(gpu, &raw, &EditStack::default()), 16, 16);
        let warm = interior_mean(
            &render(
                gpu,
                &raw,
                &EditStack {
                    warmth: 0.5,
                    ..Default::default()
                },
            ),
            16,
            16,
        );
        eprintln!("neutral {neutral:?} -> warm {warm:?}");

        assert!(warm[0] > neutral[0] + 2.0, "red should rise when warming");
        assert!(warm[2] < neutral[2] - 2.0, "blue should fall when warming");
    }

    /// A synthetic sensor whose scene is a single flat colour, produced by
    /// scaling the three channels away from neutral. Used to check that the
    /// mixer acts on one hue and leaves others alone.
    fn coloured_sensor(rgb_scale: [f32; 3]) -> RawImage {
        let mut raw = neutral_sensor(0.35);
        // The neutral sensor is built so each channel lands on the same value;
        // scaling the per-colour white balance tilts it to a chosen hue without
        // touching any of the pipeline under test.
        for (i, w) in raw.wb_coeffs.iter_mut().enumerate() {
            // Colour index 3 is the second green, so it follows green.
            *w *= rgb_scale[if i == 3 { 1 } else { i }];
        }
        raw
    }

    /// The eyedropper, through the real pipeline: pick a point on a
    /// scene with a colour cast, and the balance it sets renders that point
    /// grey.
    #[test]
    fn the_eyedropper_renders_its_point_grey() {
        let Some(gpu) = gpu() else { return };
        let pipeline = Pipeline::new(gpu);
        // A real camera's matrix mixes channels (a Sony a6400's, XYZ to
        // camera, from its DNG profile), and the test sensor's identity
        // matrix mixes them harder once taken as XYZ. Both must work.
        // A measured matrix; -0.4342 only looks like log10(e).
        #[allow(clippy::approx_constant)]
        let a6400 = crate::raw::decoder::invert3([
            [0.5991, -0.1456, -0.0217],
            [-0.4342, 1.1914, 0.2728],
            [-0.0580, 0.1418, 0.6089],
        ])
        .unwrap();
        // What a camera balanced for daylight makes of a neutral lit by
        // something else: a real light, since those are what the sliders
        // reach (`develop::white_balance`). Through the identity matrix a
        // cast picked by hand is more often a light no lamp gives — [1.25,
        // 1, 0.8] there is 0.03 Δuv magenta — and out of reach.
        let under = |cam_to_xyz: Mat3, mireds: f32, duv: f32| -> [f32; 3] {
            use crate::develop::white_balance::light;
            let camera = color::camera_to_working(cam_to_xyz).unwrap();
            let white = color::apply(color::XYZ_TO_REC2020, light(0.0, 0.0));
            let lit = color::apply(color::XYZ_TO_REC2020, light(mireds, duv));
            let seen: [f32; 3] = std::array::from_fn(|c| lit[c] / white[c]);
            color::apply(crate::raw::decoder::invert3(camera).unwrap(), seen)
        };
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for (cast, matrix) in [
            // Tungsten, a little green; shade, a little magenta; a green
            // fluorescent.
            (under(identity, 150.0, 0.006), identity),
            (under(identity, -45.0, -0.005), identity),
            (under(identity, 20.0, 0.011), identity),
            ([1.25, 1.0, 0.8], a6400),
            ([0.9, 1.08, 1.1], a6400),
            (under(a6400, 180.0, -0.004), a6400),
        ] {
            let mut raw = coloured_sensor(cast);
            raw.cam_to_xyz = matrix;
            let session = pipeline.prepare(gpu, &raw).expect("prepare");
            // Bright, as a white wall is: the probe finds its own exposure.
            let stack = EditStack::default();
            let wb = crate::develop::eyedropper::neutral_point(
                gpu, &pipeline, &session, &stack, 0.5, 0.5,
            )
            .expect("a mid-grey point");
            let mut balanced = stack.clone();
            wb.apply_to(&mut balanced);
            // Judged at a mid-tone, where a cast shows rather than clipping.
            balanced.exposure = -1.5;
            let px = pipeline
                .render_to_pixels(gpu, &session, &balanced, 16, 16)
                .expect("render");
            let [r, g, b] = interior_mean(&px, 16, 16);
            // Within a level or two: the probe reads 8-bit pixels, and a flat
            // test patch has no texture to average below one level.
            assert!(
                (r - g).abs() <= 2.5 && (b - g).abs() <= 2.5,
                "cast {cast:?}: {wb:?} rendered ({r:.1}, {g:.1}, {b:.1})"
            );
        }
    }

    /// The mixer must move the band it is pointed at, and leave the others
    /// alone — otherwise the eight sliders are one slider wearing a disguise.
    #[test]
    fn the_mixer_acts_only_on_its_own_hue_band() {
        let Some(gpu) = gpu() else { return };

        // Red-dominant and blue-dominant scenes, the red mild enough to fit on
        // an sRGB display, where the gamut map would hold a vivid one at its
        // edge whatever the mixer did.
        let reddish = coloured_sensor([1.3, 0.9, 0.9]);
        let blueish = coloured_sensor([0.7, 0.7, 1.8]);

        // Drain saturation from the red band only.
        let mut stack = EditStack::default();
        stack.mixer_saturation[0] = -1.0;

        let saturation = |pixels: &[u8]| {
            let m = interior_mean(pixels, 16, 16);
            let max = m.iter().cloned().fold(f64::MIN, f64::max);
            let min = m.iter().cloned().fold(f64::MAX, f64::min);
            (max - min) / max.max(1.0)
        };

        let red_before = saturation(&render(gpu, &reddish, &EditStack::default()));
        let red_after = saturation(&render(gpu, &reddish, &stack));
        let blue_before = saturation(&render(gpu, &blueish, &EditStack::default()));
        let blue_after = saturation(&render(gpu, &blueish, &stack));

        eprintln!(
            "red {red_before:.3} -> {red_after:.3}; blue {blue_before:.3} -> {blue_after:.3}"
        );
        // Measured as the OkLCh the mixer works in, of what is displayed.
        let displayed = |pixels: &[u8]| {
            let m = interior_mean(pixels, 16, 16);
            let linear = m.map(|v| crate::raster::srgb_to_linear(v as f32 / 255.0));
            let to_working = crate::raw::decoder::invert3(color::REC2020_TO_SRGB).unwrap();
            crate::develop::oklab::lch(color::apply(to_working, linear))
        };
        let before = displayed(&render(gpu, &reddish, &EditStack::default()));
        let after = displayed(&render(gpu, &reddish, &stack));
        eprintln!("red scene OkLCh {before:?} -> {after:?}");

        // Not a full desaturation, and correctly so: the synthetic scene is a
        // pinkish red, at hue 0 between the magenta (328) and red (29) band
        // centres, so the red slider carries about half its weight. That
        // partial effect is the interpolation working, not a weak mixer.
        assert!(
            after[1] < before[1] * 0.7,
            "desaturating the red band barely changed a red scene's chroma ({before:?} -> {after:?})"
        );
        // And only its chroma: in HSV, desaturating moved lightness too.
        assert!(
            (after[0] - before[0]).abs() < 0.03,
            "desaturating moved the red scene's lightness ({before:?} -> {after:?})"
        );
        assert!(red_after < red_before, "{red_before} -> {red_after}");
        assert!(
            (blue_after - blue_before).abs() < 0.05,
            "the red band leaked into a blue scene ({blue_before:.3} -> {blue_after:.3})"
        );
    }

    /// Desaturating a band takes the colour towards the grey of the same
    /// lightness, and a neutral mixer changes nothing — in either version.
    ///
    /// In HSV the grey a colour desaturates towards is its largest channel,
    /// so a blue, whose largest channel is far brighter than the colour looks,
    /// came out lighter as it lost its colour. OkLCh holds lightness.
    #[test]
    fn desaturating_holds_lightness_where_hsv_did_not() {
        let Some(gpu) = gpu() else { return };
        let blue = coloured_sensor([0.85, 0.9, 1.35]);
        let lch = |stack: &EditStack| {
            let m = interior_mean(&render(gpu, &blue, stack), 16, 16);
            let linear = m.map(|v| crate::raster::srgb_to_linear(v as f32 / 255.0));
            let to_working = crate::raw::decoder::invert3(color::REC2020_TO_SRGB).unwrap();
            crate::develop::oklab::lch(color::apply(to_working, linear))
        };
        let at = |version: u32, drained: bool| {
            let mut stack = EditStack {
                version,
                ..EditStack::default()
            };
            if drained {
                stack.mixer_saturation = [-1.0; crate::develop::mixer::BANDS];
            }
            lch(&stack)
        };
        // Neutral renders the same whichever way hue is read.
        assert_eq!(
            render(
                gpu,
                &blue,
                &EditStack {
                    version: 3,
                    ..EditStack::default()
                }
            ),
            render(gpu, &blue, &EditStack::default())
        );
        let (old_from, old_to) = (at(3, false), at(3, true));
        let (new_from, new_to) = (at(4, false), at(4, true));
        eprintln!("HSV {old_from:?} -> {old_to:?}; OkLCh {new_from:?} -> {new_to:?}");
        assert!(new_to[1] < 0.01, "OkLCh left chroma {new_to:?}");
        assert!(
            (new_to[0] - new_from[0]).abs() < 0.01,
            "desaturating moved lightness {new_from:?} -> {new_to:?}"
        );
        assert!(
            (old_to[0] - old_from[0]).abs() > 3.0 * (new_to[0] - new_from[0]).abs(),
            "HSV moved lightness no more than OkLCh: {old_from:?} -> {old_to:?}"
        );
    }

    /// The perceptual rendering intent reaches the shader and compresses
    /// out-of-gamut colour to just inside the edge rather than onto it. The
    /// curve itself is tested against its CPU reference in `gpu::color`.
    #[test]
    fn the_perceptual_intent_eases_out_of_gamut_colour_inside() {
        let Some(gpu) = gpu() else { return };
        let pipeline = Pipeline::new(gpu);
        let stack = EditStack::default();
        let render = |raw: &RawImage, output: color::Output| {
            let session = pipeline.prepare(gpu, raw).expect("prepare");
            let (w, h) = (session.width, session.height);
            let px = pipeline
                .render_to_pixels_in(gpu, &session, &stack, w, h, output)
                .expect("render");
            interior_mean(&px, 16, 16)
        };
        let relative = color::Output::SRGB;
        let perceptual = color::Output::SRGB.with_intent(true);

        // A green well beyond sRGB. Relative colorimetric pulls it onto the
        // gamut's edge, so its weakest channel lands on zero; perceptual eases it
        // to just inside, with the same hue (green still dominant, blue still
        // above red) and much the same brightness.
        let raw = coloured_sensor([0.5, 3.0, 0.5]);
        let (rel, per) = (render(&raw, relative), render(&raw, perceptual));
        assert!(rel[0] < 1.0, "relative should reach the edge: {rel:?}");
        assert!(per[0] > 10.0, "perceptual should stay inside: {per:?}");
        assert!(per[1] > per[2] && per[2] > per[0], "hue moved: {per:?}");
        assert!(
            (rel[1] - per[1]).abs() < 4.0,
            "brightness moved: {rel:?} vs {per:?}"
        );

        // A neutral has no chroma to compress: identical under both intents.
        let (a, b) = (
            render(&neutral_sensor(0.5), relative),
            render(&neutral_sensor(0.5), perceptual),
        );
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 0.5, "grey moved: {a:?} vs {b:?}");
        }
    }

    /// A Display P3 render encodes the same colour closer to neutral than
    /// sRGB does — P3's primaries sit further out, so it needs less of each —
    /// and leaves a neutral exactly neutral, the same grey as sRGB.
    ///
    /// The colour is dimmed to sit inside sRGB. Brighter, `gamut_map` (which
    /// runs after the output matrix) compresses it for sRGB but not for P3, so
    /// the P3 file keeps saturation sRGB had to give up — the point of P3, and
    /// not the same colour any more.
    #[test]
    fn a_display_p3_render_is_the_same_colour_in_wider_primaries() {
        let Some(gpu) = gpu() else { return };
        let p3 = color::Output::new(color::rec2020_to_display_p3(), color::Transfer::Srgb);
        let (s, p) = wide_gamut_spread(gpu, [1.3, 0.85, 0.85], p3, 1.5);
        eprintln!("red channel spread: sRGB {s:.1}, P3 {p:.1}");
        assert!(p < s, "P3 should need less channel spread for the same red");
    }

    /// Adobe RGB shares sRGB's red and blue primaries and is wider in green,
    /// so it is a green that it encodes closer to neutral. Its gamma differs
    /// from the sRGB curve, so a grey is the same *neutral* but not the same
    /// code value — close in the mid-tones where the two curves cross.
    #[test]
    fn an_adobe_rgb_render_encodes_green_in_wider_primaries_with_its_own_gamma() {
        let Some(gpu) = gpu() else { return };
        let adobe = color::Output::new(color::rec2020_to_adobe_rgb(), color::Transfer::Gamma22);
        let (s, a) = wide_gamut_spread(gpu, [0.85, 1.3, 0.85], adobe, 6.0);
        eprintln!("green channel spread: sRGB {s:.1}, Adobe RGB {a:.1}");
        assert!(
            a < s,
            "Adobe RGB should need less channel spread for the same green"
        );
    }

    /// Renders a tinted scene in sRGB and in `wide`, and returns each render's
    /// channel spread. The scene is dimmed to sit inside sRGB: brighter,
    /// `gamut_map` (which runs after the output matrix) compresses it for sRGB
    /// but not for the wider space, and it stops being the same colour. Also
    /// checks a grey stays neutral in `wide`, within `grey_tolerance` levels of
    /// the sRGB grey.
    fn wide_gamut_spread(
        gpu: &Gpu,
        tint: [f32; 3],
        wide: color::Output,
        grey_tolerance: f64,
    ) -> (f64, f64) {
        let pipeline = Pipeline::new(gpu);
        let stack = EditStack {
            exposure: -1.5,
            ..Default::default()
        };
        let both = |raw: &RawImage| {
            let session = pipeline.prepare(gpu, raw).expect("prepare");
            let (w, h) = (session.width, session.height);
            let srgb = pipeline
                .render_to_pixels(gpu, &session, &stack, w, h)
                .expect("sRGB render");
            let out = pipeline
                .render_to_pixels_in(gpu, &session, &stack, w, h, wide)
                .expect("wide render");
            (interior_mean(&srgb, 16, 16), interior_mean(&out, 16, 16))
        };
        let spread = |m: [f64; 3]| {
            let max = m.iter().cloned().fold(f64::MIN, f64::max);
            let min = m.iter().cloned().fold(f64::MAX, f64::min);
            max - min
        };

        let (a, b) = both(&neutral_sensor(0.5));
        assert!(
            spread(b) < 1.5,
            "grey is not neutral in the wide space: {b:?}"
        );
        for i in 0..3 {
            assert!(
                (a[i] - b[i]).abs() < grey_tolerance,
                "grey moved: {a:?} vs {b:?}"
            );
        }

        let (srgb, out) = both(&coloured_sensor(tint));
        (spread(srgb), spread(out))
    }

    /// Global saturation raises a colour's chroma and lowers it, and a grey
    /// patch is left neutral either way — the control acts on chroma, not tone.
    #[test]
    fn saturation_scales_chroma_and_spares_neutrals() {
        let Some(gpu) = gpu() else { return };

        let chroma = |pixels: &[u8]| {
            let m = interior_mean(pixels, 16, 16);
            let max = m.iter().cloned().fold(f64::MIN, f64::max);
            let min = m.iter().cloned().fold(f64::MAX, f64::min);
            (max - min) / max.max(1.0)
        };

        // A mid-gamut red with headroom on every channel. Pushed to the sensor's
        // ceiling it would already sit on the gamut edge, where `gamut_map`
        // absorbs any *increase* in saturation and the test would only ever see
        // the reduction move — so it is deliberately dimmed to leave both
        // directions room.
        let reddish = coloured_sensor([1.3, 0.85, 0.85]);
        let sat = |s: f32| EditStack {
            exposure: -1.5,
            saturation: s,
            ..Default::default()
        };
        let base = chroma(&render(gpu, &reddish, &sat(0.0)));
        let up = chroma(&render(gpu, &reddish, &sat(0.6)));
        let down = chroma(&render(gpu, &reddish, &sat(-0.6)));
        eprintln!("chroma {down:.3} < {base:.3} < {up:.3}");
        assert!(
            up > base * 1.1 && down < base * 0.9,
            "saturation did not scale chroma ({down:.3} < {base:.3} < {up:.3})"
        );

        // A neutral scene has no chroma to scale, so saturation must leave it
        // grey — the same no-op path an unedited photo relies on.
        let grey = render(
            gpu,
            &neutral_sensor(0.5),
            &EditStack {
                saturation: 1.0,
                ..Default::default()
            },
        );
        assert!(
            chroma(&grey) < 0.02,
            "saturation tinted a neutral scene (chroma {:.3})",
            chroma(&grey)
        );
    }

    /// Vibrance lifts a muted colour more than an already-vivid one — that
    /// protection is the whole reason it exists alongside a flat saturation.
    #[test]
    fn vibrance_favours_the_less_saturated_colour() {
        let Some(gpu) = gpu() else { return };

        let chroma = |pixels: &[u8]| {
            let m = interior_mean(pixels, 16, 16);
            let max = m.iter().cloned().fold(f64::MIN, f64::max);
            let min = m.iter().cloned().fold(f64::MAX, f64::min);
            (max - min) / max.max(1.0)
        };

        // Both dimmed to stay off the gamut edge (see the saturation test), so
        // the difference measured is the vibrance taper, not the gamut ceiling.
        let muted = coloured_sensor([1.15, 0.92, 0.92]);
        let vivid = coloured_sensor([1.6, 0.75, 0.75]);
        let vib = EditStack {
            exposure: -1.5,
            vibrance: 0.9,
            ..Default::default()
        };
        let flat = EditStack {
            exposure: -1.5,
            ..Default::default()
        };

        let muted_gain =
            chroma(&render(gpu, &muted, &vib)) / chroma(&render(gpu, &muted, &flat)).max(1e-4);
        let vivid_gain =
            chroma(&render(gpu, &vivid, &vib)) / chroma(&render(gpu, &vivid, &flat)).max(1e-4);
        eprintln!("vibrance gain: muted {muted_gain:.3} vs vivid {vivid_gain:.3}");
        assert!(
            muted_gain > vivid_gain,
            "vibrance lifted the vivid colour as much as the muted one ({muted_gain:.3} vs {vivid_gain:.3})"
        );
    }

    /// Soft-proof marks colours the output gamut cannot hold. A violently
    /// saturated red is inside the sensor's reach but far outside sRGB, so with
    /// the gamut check on it is painted the flat magenta marker; with it off the
    /// same pixel renders as an ordinary red. The two being unmistakably
    /// different is the whole feature — the overlay shows what will clip.
    #[test]
    fn the_gamut_check_marks_out_of_gamut_colour() {
        let Some(gpu) = gpu() else { return };
        let raw = coloured_sensor([3.0, 0.15, 0.15]);
        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let stack = EditStack::default();
        let (w, h) = (16, 16);

        let proofed = pipeline
            .render_view(gpu, &session, &stack, w, h, None, true)
            .expect("render proofed");
        let plain = pipeline
            .render_view(gpu, &session, &stack, w, h, None, false)
            .expect("render plain");

        let m_proof = interior_mean(&proofed, w, h);
        let m_plain = interior_mean(&plain, w, h);
        eprintln!("proof {m_proof:.0?} plain {m_plain:.0?}");

        // The marker is flat magenta (255, 0, 255): red and blue pinned high,
        // green at zero.
        assert!(
            m_proof[0] > 220.0 && m_proof[1] < 40.0 && m_proof[2] > 220.0,
            "soft-proof did not paint the out-of-gamut red as the magenta marker: {m_proof:.0?}"
        );
        // The control render keeps a real green channel (the gamut map
        // desaturates the red toward grey rather than blanking it), so it can
        // never be mistaken for the marker, whose green is pinned to zero.
        assert!(
            m_plain[1] > 40.0,
            "control render should carry the colour, not the marker: {m_plain:.0?}"
        );
    }

    /// A neutral mixer must be a true no-op. The HSV round trip in the shader
    /// is lossy if done carelessly, and that would tint every photo whether or
    /// not the panel was touched.
    #[test]
    fn a_neutral_mixer_leaves_the_image_untouched() {
        let Some(gpu) = gpu() else { return };
        let raw = coloured_sensor([1.5, 0.9, 0.6]);

        let baseline = render(gpu, &raw, &EditStack::default());
        // Explicitly zeroed rather than defaulted, so this still exercises the
        // mixer code path rather than any "skip if neutral" shortcut.
        let stack = EditStack {
            mixer_saturation: [0.0; crate::develop::mixer::BANDS],
            ..Default::default()
        };
        let through_mixer = render(gpu, &raw, &stack);

        let a = interior_mean(&baseline, 16, 16);
        let b = interior_mean(&through_mixer, 16, 16);
        for c in 0..3 {
            assert!(
                (a[c] - b[c]).abs() < 1.0,
                "channel {c} shifted {:.2} -> {:.2} with a neutral mixer",
                a[c],
                b[c]
            );
        }
    }

    /// A look LUT is authored for sRGB, so a Display P3 export must show it
    /// sRGB and not its own encoding: a LUT that paints everything
    /// sRGB red paints the P3 file sRGB red — which, in P3's wider primaries,
    /// is a less extreme encoding than P3's own red. An identity LUT still
    /// changes nothing.
    #[test]
    fn a_look_lut_on_a_display_p3_export_sees_srgb() {
        let Some(gpu) = gpu() else { return };
        let pipeline = Pipeline::new(gpu);
        let p3 = color::Output::new(color::rec2020_to_display_p3(), color::Transfer::Srgb);
        let mut session = pipeline
            .prepare(gpu, &neutral_sensor(0.4))
            .expect("prepare");
        let (w, h) = (session.width, session.height);
        let dim = crate::develop::lut::LUT_3D_DIM;
        let mut lut_stack = EditStack::default();
        lut_stack.lut.name = "x".into();
        lut_stack.lut.intensity = 1.0;
        let render = |session: &DevelopSession, stack: &EditStack| {
            let px = pipeline
                .render_to_pixels_in(gpu, session, stack, w, h, p3)
                .expect("render");
            interior_mean(&px, 16, 16)
        };

        // A saturated colour, so the identity check covers colour beyond sRGB.
        let vivid = pipeline
            .prepare(gpu, &coloured_sensor([0.5, 3.0, 0.5]))
            .expect("prepare");
        let mut vivid_lut = pipeline
            .prepare(gpu, &coloured_sensor([0.5, 3.0, 0.5]))
            .expect("prepare");
        vivid_lut.lut_texels = Some(crate::develop::lut::identity_texels(dim));
        let (a, b) = (
            render(&vivid, &EditStack::default()),
            render(&vivid_lut, &lut_stack),
        );
        for c in 0..3 {
            assert!(
                (a[c] - b[c]).abs() < 1.5,
                "identity LUT moved a P3 colour: {a:?} vs {b:?}"
            );
        }

        let red: Vec<f32> = std::iter::repeat_n([1.0f32, 0.0, 0.0, 1.0], dim * dim * dim)
            .flatten()
            .collect();
        session.lut_texels = Some(red);
        // sRGB red in P3 is about (0.82, 0.03, 0.02) linear: (234, 51, 35)
        // encoded, not P3's own (255, 0, 0).
        let painted = render(&session, &lut_stack);
        assert!(
            (painted[0] - 234.0).abs() < 6.0
                && (painted[1] - 51.0).abs() < 6.0
                && (painted[2] - 35.0).abs() < 6.0,
            "a red LUT should paint sRGB red in P3, got {painted:?}"
        );
    }

    #[test]
    fn a_look_lut_recolours_the_output_and_identity_is_a_no_op() {
        let Some(gpu) = gpu() else { return };
        let pipeline = Pipeline::new(gpu);
        let raw = neutral_sensor(0.4);
        let mut session = pipeline.prepare(gpu, &raw).expect("prepare");
        let (w, h) = (session.width, session.height);
        let dim = crate::develop::lut::LUT_3D_DIM;

        let base = pipeline
            .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
            .expect("base");

        let mut lut_stack = EditStack::default();
        lut_stack.lut.name = "x".into();
        lut_stack.lut.intensity = 1.0;

        // An identity LUT at full intensity changes nothing (within rounding).
        session.lut_texels = Some(crate::develop::lut::identity_texels(dim));
        let identity = pipeline
            .render_to_pixels(gpu, &session, &lut_stack, w, h)
            .expect("identity");
        for (a, b) in base.iter().zip(&identity) {
            assert!(
                (*a as i32 - *b as i32).abs() <= 2,
                "identity LUT must be a no-op"
            );
        }

        // A constant-red LUT paints every pixel red.
        let red: Vec<f32> = std::iter::repeat_n([1.0f32, 0.0, 0.0, 1.0], dim * dim * dim)
            .flatten()
            .collect();
        session.lut_texels = Some(red);
        let painted = pipeline
            .render_to_pixels(gpu, &session, &lut_stack, w, h)
            .expect("red");
        let c = (((h / 2) * w + w / 2) * 4) as usize;
        assert!(
            painted[c] > 200 && painted[c + 1] < 40 && painted[c + 2] < 40,
            "LUT should paint red, got {:?}",
            &painted[c..c + 3]
        );

        // Intensity 0 ignores the LUT even when texels are bound.
        let mut off = lut_stack.clone();
        off.lut.intensity = 0.0;
        let none = pipeline
            .render_to_pixels(gpu, &session, &off, w, h)
            .expect("off");
        for (a, b) in base.iter().zip(&none) {
            assert!(
                (*a as i32 - *b as i32).abs() <= 2,
                "intensity 0 must be a no-op"
            );
        }
    }

    /// A sensor with a bright marker in one corner, so a rotation is
    /// identifiable rather than merely "different".
    fn cornered_sensor(orientation: u8) -> RawImage {
        let mut raw = neutral_sensor(0.15);
        let w = raw.width;
        // Top-left quadrant much brighter than the rest.
        for y in 0..raw.height / 2 {
            for x in 0..w / 2 {
                raw.cfa[(y * w + x) as usize] = raw.cfa[(y * w + x) as usize].saturating_mul(6);
            }
        }
        raw.orientation = orientation;
        raw
    }

    /// Which quadrant is brightest, as (col, row) in a 2x2 split.
    fn bright_quadrant(pixels: &[u8], w: u32, h: u32) -> (u32, u32) {
        let mut best = (0, 0);
        let mut best_sum = -1.0f64;
        for qy in 0..2 {
            for qx in 0..2 {
                let mut sum = 0.0;
                for y in (qy * h / 2)..((qy + 1) * h / 2) {
                    for x in (qx * w / 2)..((qx + 1) * w / 2) {
                        sum += pixels[((y * w + x) * 4) as usize] as f64;
                    }
                }
                if sum > best_sum {
                    best_sum = sum;
                    best = (qx, qy);
                }
            }
        }
        best
    }

    /// Cameras store a portrait frame in landscape and record which way up they
    /// were held. Ignoring that leaves every rotated photo on its side in
    /// Develop while the library, whose thumbnails bake the rotation in, shows
    /// it upright — so the two views disagree and nothing else notices.
    #[test]
    fn exif_orientation_rotates_the_render() {
        let Some(gpu) = gpu() else { return };
        let pipeline = Pipeline::new(gpu);

        let quadrant_for = |orientation: u8| {
            let raw = cornered_sensor(orientation);
            let session = pipeline.prepare(gpu, &raw).expect("prepare");
            let (w, h) = (session.width, session.height);
            let px = pipeline
                .render_to_pixels(gpu, &session, &EditStack::default(), w, h)
                .expect("render");
            bright_quadrant(&px, w, h)
        };

        // Upright: the marker stays top-left.
        assert_eq!(quadrant_for(1), (0, 0), "unrotated");
        // 180: it moves to the opposite corner.
        assert_eq!(quadrant_for(3), (1, 1), "180 degrees");
        // 90 clockwise: top-left goes to top-right.
        assert_eq!(quadrant_for(6), (1, 0), "90 clockwise");
        // 90 anticlockwise: top-left goes to bottom-left.
        assert_eq!(quadrant_for(8), (0, 1), "90 anticlockwise");
    }

    /// A quarter turn has to swap the reported dimensions too, or the UI lays
    /// out a portrait photo in a landscape box and the export comes out squashed.
    #[test]
    fn a_quarter_turn_swaps_the_reported_dimensions() {
        let Some(gpu) = gpu() else { return };
        let pipeline = Pipeline::new(gpu);

        let mut wide = neutral_sensor(0.3);
        wide.active_area.width = 16;
        wide.active_area.height = 8;
        wide.orientation = 1;
        let upright = pipeline.prepare(gpu, &wide).unwrap();
        assert_eq!((upright.width, upright.height), (16, 8));

        wide.orientation = 6;
        let turned = pipeline.prepare(gpu, &wide).unwrap();
        assert_eq!(
            (turned.width, turned.height),
            (8, 16),
            "a quarter turn should swap width and height"
        );
    }

    /// A blown-out sensor: every photosite pegged at the white level, as an
    /// overexposed sky records.
    fn clipped_sensor() -> RawImage {
        let mut raw = neutral_sensor(0.5);
        for v in raw.cfa.iter_mut() {
            *v = raw.white[1];
        }
        // Multipliers with the same lopsided shape a real camera has — blue
        // nearly three times green — which is what turns clipped areas magenta.
        raw.wb_coeffs = [1.83, 1.0, 2.79, 1.0];
        raw
    }

    /// A 64x64 Bayer sensor painted with `scene(x, y)`, the colour each
    /// pixel should have once balanced by `wb` — clipped where that asks for
    /// more than the sensor holds, as a real one is.
    fn painted_sensor(wb: [f32; 4], scene: impl Fn(f32, f32) -> [f32; 3]) -> RawImage {
        let size = 64u32;
        let mut raw = neutral_sensor(0.5);
        raw.width = size;
        raw.height = size;
        raw.active_area = Rect {
            x: 0,
            y: 0,
            width: size,
            height: size,
        };
        raw.wb_coeffs = wb;
        raw.cfa = (0..size)
            .flat_map(|y| (0..size).map(move |x| (x, y)))
            .map(|(x, y)| {
                let c = match (y % 2, x % 2) {
                    (0, 0) => 0,
                    (1, 1) => 2,
                    _ => 1,
                };
                let target = scene(x as f32 - 31.5, y as f32 - 31.5)[c];
                ((target / wb[c]).clamp(0.0, 1.0) * raw.white[c] as f32) as u16
            })
            .collect();
        raw
    }

    /// The linear image the mosaic and highlight passes leave, averaged over
    /// the 4x4 block at the centre.
    fn linear_centre(gpu: &Gpu, raw: &RawImage) -> [f32; 3] {
        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, raw).expect("prepare");
        let halves = pipeline.read_linear(gpu, &session).expect("read back");
        let (w, _) = session.stored_size();
        let mut sum = [0.0f32; 3];
        for y in 30..34 {
            for x in 30..34 {
                let i = ((y * w + x) * 4) as usize;
                for c in 0..3 {
                    sum[c] += half::f16::from_bits(halves[i + c]).to_f32() / 16.0;
                }
            }
        }
        sum
    }

    /// An orange whose red clips keeps its hue. With red stuck at the
    /// ceiling and green still climbing, it used to drift yellow; now red is
    /// rebuilt from the orange around it, above where it clipped.
    #[test]
    fn a_partly_blown_orange_stays_orange() {
        let Some(gpu) = gpu() else { return };
        let orange = [1.0, 0.45, 0.2];
        let wb = [1.5, 1.0, 2.0, 1.0];
        let raw = painted_sensor(wb, |x, y| {
            let light = 1.0 + 0.75 * (1.0 - x.hypot(y) / 20.0).max(0.0);
            orange.map(|c| c * light)
        });
        let [r, g, b] = linear_centre(gpu, &raw);
        let (want, got) = (orange[0] / orange[1], r / g);
        eprintln!(
            "centre {:?}: red/green {got:.3}, the scene's {want:.3}",
            [r, g, b]
        );
        assert!(r > wb[0], "red was not rebuilt past its ceiling: {r}");
        assert!(
            (got / want - 1.0).abs() < 0.05,
            "red/green {got:.3} where the scene has {want:.3}: the hue moved"
        );
    }

    /// A blown cloud stays white. Its core clips in red and green and not
    /// blue, and the colour it borrows is its own white edge — not the blue
    /// sky beyond, which is darker and trusted less.
    ///
    /// The edge is twelve pixels, thin for a cloud at full resolution. At four
    /// the core leant 14% blue: the two pixels beside it are rightly not
    /// trusted (they are demosaiced from clipped sites), and colour came from
    /// the wider ring, blend into the sky and all.
    #[test]
    fn a_blown_cloud_stays_white() {
        let Some(gpu) = gpu() else { return };
        let wb = [1.83, 1.0, 2.79, 1.0];
        let raw = painted_sensor(wb, |x, y| match x.hypot(y) {
            d if d < 8.0 => [2.2; 3],
            d if d < 20.0 => [0.85; 3],
            _ => [0.25, 0.4, 0.9],
        });
        let rgb = linear_centre(gpu, &raw);
        let spread = rgb.iter().cloned().fold(f32::MIN, f32::max)
            / rgb.iter().cloned().fold(f32::MAX, f32::min);
        eprintln!("cloud core {rgb:?}, spread {spread:.3}");
        assert!(spread < 1.05, "the cloud's core came out {rgb:?}");
    }

    /// A white highlight against blue borrows blue it cannot be — the red it
    /// clipped at is more red than that blue has — and is eased towards grey
    /// until it agrees. Whatever it lands on, never pink: red and blue both
    /// above green is the artefact this whole pass exists to prevent.
    #[test]
    fn a_highlight_against_blue_never_goes_pink() {
        let Some(gpu) = gpu() else { return };
        let wb = [1.83, 1.0, 2.79, 1.0];
        let raw = painted_sensor(wb, |x, y| {
            if x.hypot(y) < 8.0 {
                [2.2; 3]
            } else {
                [0.3, 0.45, 0.9]
            }
        });
        let [r, g, b] = linear_centre(gpu, &raw);
        eprintln!("highlight against blue {:?}", [r, g, b]);
        assert!(
            g >= 0.95 * r.min(b),
            "red {r} and blue {b} both above green {g}: pink"
        );
        assert!(r >= 0.9 * wb[0], "red fell below where it clipped: {r}");
    }

    /// Clipped highlights must come out white, not coloured.
    ///
    /// At saturation every channel reads the same value and carries no colour;
    /// multiplying by white balance then spreads them apart and paints blown
    /// skies magenta. Nothing else in the suite looks at clipping, and the
    /// artefact is glaring in real photos, so it is pinned here.
    #[test]
    fn blown_highlights_render_neutral_rather_than_magenta() {
        let Some(gpu) = gpu() else { return };
        let pixels = render(gpu, &clipped_sensor(), &EditStack::default());
        let mean = interior_mean(&pixels, 16, 16);
        eprintln!("clipped sensor renders as {mean:?}");

        let max = mean.iter().cloned().fold(f64::MIN, f64::max);
        let min = mean.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            max - min < 6.0,
            "a fully clipped frame rendered as {mean:?} — that spread is a colour cast on \
             every blown highlight"
        );
        assert!(
            min > 250.0,
            "clipped highlights rendered at {mean:?}, which is not white — the tone curve's \
             shoulder is not reaching 1.0 at sensor saturation"
        );
    }

    /// Deep shadows at high ISO are noise either side of black. Clipping it at
    /// black before the channels are balanced keeps only the positive half — a
    /// bias white balance then multiplies by 2 on red and 4 on blue, which is
    /// the purple cast in a noisy night sky. Averaged, as the fitted preview's
    /// downsample and noise reduction average it, the noise must come out as
    /// grey.
    #[test]
    fn shadow_noise_averages_to_neutral() {
        let Some(gpu) = gpu() else { return };
        let size = 64u32;
        let black = 512u16;
        // A faint neutral: `signal` on green after balance, noise far larger.
        let (signal, noise) = (20.0f32, 150i32);
        let wb = [2.0f32, 1.0, 4.0, 1.0];
        let tile = [0u8, 1, 3, 2];
        let mut cfa = vec![0u16; (size * size) as usize];
        for y in 0..size {
            for x in 0..size {
                let c = tile[((y % 2) * 2 + x % 2) as usize] as usize;
                let hash = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 1024;
                let jitter = (hash as i32 - 512) * noise / 512;
                let v = black as f32 + signal / wb[c] + jitter as f32;
                cfa[(y * size + x) as usize] = v.round().clamp(0.0, 16383.0) as u16;
            }
        }
        let raw = RawImage {
            cfa,
            width: size,
            height: size,
            cfa_pattern: CfaPattern::Bayer(tile),
            black: [black; 4],
            white: [16383; 4],
            wb_coeffs: wb,
            cam_to_xyz: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            baseline_exposure: 0.0,
            camera_median: None,
            active_area: Rect {
                x: 0,
                y: 0,
                width: size,
                height: size,
            },
            orientation: 1,
            make: "Synthetic".into(),
            model: "Noisy".into(),
        };
        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        // Lifted into the midtones, where a cast is plain to see.
        let stack = EditStack {
            exposure: 6.0,
            ..EditStack::default()
        };
        let pixels = pipeline
            .render_to_pixels(gpu, &session, &stack, 4, 4)
            .expect("render");
        let mut mean = [0f64; 3];
        for px in pixels.chunks_exact(4) {
            for c in 0..3 {
                mean[c] += px[c] as f64 / 16.0;
            }
        }
        eprintln!("averaged shadow noise renders as {mean:?}");
        let max = mean.iter().cloned().fold(f64::MIN, f64::max);
        let min = mean.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            max - min < 8.0,
            "averaged shadow noise rendered as {mean:?} — a colour cast in every noisy shadow"
        );
    }

    /// Noise reduction divides by brightness in places, and a noisy shadow's
    /// pixels now sit either side of zero. Dividing by the local average there
    /// threw pixels to full white. At 1:1, with both noise controls up, a dark
    /// noisy frame must stay as dark as it was. (Sharpening is left out: it
    /// raises noise peaks by design.)
    #[test]
    fn noise_reduction_stays_calm_on_signed_shadows() {
        let Some(gpu) = gpu() else { return };
        // A deep shadow: 40 above black, noise ten times that.
        let mut raw = neutral_sensor(0.5);
        let size = 64u32;
        (raw.width, raw.height) = (size, size);
        raw.active_area = Rect {
            x: 0,
            y: 0,
            width: size,
            height: size,
        };
        raw.black = [300; 4];
        raw.white = [16383; 4];
        raw.wb_coeffs = [2.0, 1.0, 3.0, 1.0];
        raw.cfa = (0..size * size)
            .map(|i| {
                let (x, y) = (i % size, i / size);
                let hash = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 1024;
                (340 + (hash as i32 - 512) * 400 / 512).clamp(0, 16383) as u16
            })
            .collect();
        let mut stack = EditStack::default();
        stack.detail.noise = 0.8;
        stack.detail.color_noise = 0.8;
        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let pixels = pipeline
            .render_to_pixels(gpu, &session, &stack, 64, 64)
            .expect("render");
        let plain = pipeline
            .render_to_pixels(gpu, &session, &EditStack::default(), 64, 64)
            .expect("render");
        let brightest = |p: &[u8]| {
            p.chunks_exact(4)
                .map(|px| px[0].max(px[1]).max(px[2]))
                .max()
                .unwrap()
        };
        eprintln!(
            "brightest pixel: {} filtered, {} unfiltered",
            brightest(&pixels),
            brightest(&plain)
        );
        assert!(
            brightest(&pixels) <= brightest(&plain).saturating_add(24),
            "noise reduction threw a speck well above anything in the frame"
        );
    }

    /// The clipping fix must not bleach ordinary bright colours — only ones
    /// that actually reached the sensor ceiling.
    #[test]
    fn bright_but_unclipped_colour_keeps_its_hue() {
        let Some(gpu) = gpu() else { return };
        let mut raw = coloured_sensor([1.6, 0.8, 0.7]);
        raw.wb_coeffs = [1.83 * 1.6, 1.0 * 0.8, 2.79 * 0.7, 0.8];

        let mean = interior_mean(&render(gpu, &raw, &EditStack::default()), 16, 16);
        let max = mean.iter().cloned().fold(f64::MIN, f64::max);
        let min = mean.iter().cloned().fold(f64::MAX, f64::min);
        eprintln!("bright unclipped colour renders as {mean:?}");
        assert!(
            max - min > 10.0,
            "an unclipped colour was flattened to neutral ({mean:?}) — the highlight fix is \
             reaching too far down"
        );
    }

    /// The point curve has to reach actual pixels.
    ///
    /// It is composed into the tone table on the CPU and the shader never
    /// learns it exists, which is the design — but that also means a wiring
    /// mistake would be completely silent, with every curve rendering as the
    /// identity. This is the test that would notice.
    #[test]
    fn the_point_curve_changes_the_rendered_image() {
        let Some(gpu) = gpu() else { return };
        let raw = neutral_sensor(0.5);

        let flat = interior_mean(&render(gpu, &raw, &EditStack::default()), 16, 16);
        let darkened = EditStack {
            // Pulls the white point down, so the curve darkens across its
            // whole range. A midtone-only curve is a poor probe here: this
            // synthetic scene renders around 230, high enough that a curve
            // returning to 1.0 at the top barely moves it.
            curve: vec![
                crate::develop::CurvePoint::new(0.0, 0.0),
                crate::develop::CurvePoint::new(0.5, 0.25),
                crate::develop::CurvePoint::new(1.0, 0.5),
            ],
            ..Default::default()
        };
        let curved = interior_mean(&render(gpu, &raw, &darkened), 16, 16);
        eprintln!("neutral {flat:?} -> curve-darkened {curved:?}");

        assert!(
            curved[1] < flat[1] - 20.0,
            "the point curve did not darken the render: {flat:?} -> {curved:?}"
        );
        // A curve on the composite tone table must not tint anything: all three
        // channels pass through the same table, so they must move together.
        let spread = curved.iter().cloned().fold(f64::MIN, f64::max)
            - curved.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread < 3.0, "the curve tinted a neutral scene: {curved:?}");
    }

    /// Colour grading has to reach pixels, and reach the *right* ones.
    ///
    /// A single bright scene would only prove the wheels do something. Grading
    /// a bright frame with the shadow wheel and a dark frame with the same
    /// wheel shows the range split is actually selecting, which is the part
    /// that would still look plausible if it were wrong.
    #[test]
    fn colour_grading_tints_only_its_own_tonal_range() {
        let Some(gpu) = gpu() else { return };
        let warm = crate::develop::GradeRange {
            hue: 30.0,
            saturation: 1.0,
            luminance: 0.0,
        };
        let shadow_grade = EditStack {
            grading: crate::develop::ColorGrading {
                shadows: warm,
                ..Default::default()
            },
            ..Default::default()
        };

        // Exposure separates the two frames rather than sensor level, so both
        // go through an identical mosaic and only the tone position differs.
        let dark = EditStack {
            exposure: -3.0,
            ..Default::default()
        };
        let dark_graded = EditStack {
            exposure: -3.0,
            ..shadow_grade.clone()
        };

        let raw = neutral_sensor(0.5);
        // Red minus blue over the frame's own level. A plain R/B ratio blows up
        // in a dark frame, where blue floors and the guard against dividing by
        // zero sets the answer — reporting a meaningless number that happens to
        // pass. This stays comparable across exposures.
        let warmth = |s: &EditStack| {
            let m = interior_mean(&render(gpu, &raw, s), 16, 16);
            let level = (m[0] + m[1] + m[2]) / 3.0;
            (m[0] - m[2]) / level.max(1.0)
        };

        let bright = warmth(&shadow_grade) - warmth(&EditStack::default());
        let dark_shift = warmth(&dark_graded) - warmth(&dark);
        eprintln!(
            "shadow wheel warms a dark frame by {dark_shift:.3}, a bright one by {bright:.3}"
        );

        assert!(
            dark_shift > 0.3,
            "the shadow wheel did not warm a dark frame ({dark_shift:.3})"
        );
        assert!(
            bright < dark_shift * 0.4,
            "the shadow wheel reached the highlights: dark {dark_shift:.3}, bright {bright:.3}"
        );
    }

    /// A neutral grade must be a byte-level no-op — every unedited photo goes
    /// through the new sampling path.
    #[test]
    fn a_neutral_grade_leaves_the_render_untouched() {
        let Some(gpu) = gpu() else { return };
        let raw = neutral_sensor(0.5);
        let plain = render(gpu, &raw, &EditStack::default());
        let graded = render(
            gpu,
            &raw,
            &EditStack {
                grading: crate::develop::ColorGrading::default(),
                ..Default::default()
            },
        );
        assert_eq!(plain, graded);
    }

    /// A neutral edge must not pick up colour.
    ///
    /// Interpolating each channel on its own puts the three in slightly
    /// different places across an edge, and that disagreement *is* a hue — so a
    /// plain black-to-white boundary comes out fringed.
    ///
    /// Measured on the worst pixel rather than the mean: fringing is a thin
    /// line a couple of pixels wide, and an average over the frame hides it
    /// completely.
    ///
    /// The progression this test drove, all on the same synthetic step:
    ///
    /// | demosaic | worst fringe |
    /// |---|---|
    /// | bilinear | 255 |
    /// | Malvar-He-Cutler | 184 |
    /// | …bounded by its own samples | 97 |
    /// | …plus edge-directed green | 165 |
    /// | …plus colour-difference red and blue | **2** |
    ///
    /// Two of those are worth remembering. Plain Malvar-He-Cutler is *worse*
    /// than bilinear at a hard edge, because its gradient correction has
    /// negative lobes and overshoots. And edge-directed green on its own is
    /// worse again — an edge-aware green beside a red and blue still averaging
    /// straight through the edge disagree by more than three consistently wrong
    /// channels do. Only the pair together fixes it.
    #[test]
    fn a_neutral_edge_does_not_pick_up_colour() {
        let Some(gpu) = gpu() else { return };
        let raw = edge_sensor();
        let (w, h) = (raw.width, raw.height);
        let pixels = render(gpu, &raw, &EditStack::default());

        let mut worst = 0i32;
        let mut worst_at = (0u32, 0u32);
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                let i = ((y * w + x) * 4) as usize;
                let p = [pixels[i] as i32, pixels[i + 1] as i32, pixels[i + 2] as i32];
                let spread = p.iter().max().unwrap() - p.iter().min().unwrap();
                if spread > worst {
                    worst = spread;
                    worst_at = (x, y);
                }
            }
        }
        eprintln!("worst edge fringe: {worst} at {worst_at:?}");

        assert!(
            worst < 16,
            "a neutral edge fringed by {worst}/255 at {worst_at:?} — the demosaic is \
             inventing colour at edges"
        );
    }

    /// Cropping must actually select a region, not just resize the frame.
    ///
    /// Rendered from the edge scene, whose two halves differ, so a crop of the
    /// bright half has to come out uniformly bright. A crop that reported the
    /// right size while still sampling the whole image would pass a dimensions
    /// check and fail this.
    #[test]
    fn a_crop_selects_its_region() {
        let Some(gpu) = gpu() else { return };
        let raw = edge_sensor();
        let (w, h) = (raw.width, raw.height);

        let whole = interior_mean(&render(gpu, &raw, &EditStack::default()), w, h);

        // The right half is the bright side of `edge_sensor`.
        let bright = EditStack {
            geometry: crate::develop::Geometry {
                crop: crate::develop::CropRect {
                    x: 0.55,
                    y: 0.0,
                    width: 0.4,
                    height: 1.0,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let (cw, ch) = crate::develop::Geometry {
            crop: bright.geometry.crop,
            ..Default::default()
        }
        .output_size(w, h);
        assert_eq!((cw, ch), ((w as f32 * 0.4).round() as u32, h));

        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let pixels = pipeline
            .render_to_pixels(gpu, &session, &bright, cw, ch)
            .expect("render");
        let cropped = interior_mean(&pixels, cw, ch);

        eprintln!("whole frame {whole:?} -> crop of the bright half {cropped:?}");
        assert!(
            cropped[1] > whole[1] + 15.0,
            "cropping to the bright half did not brighten the frame: \
             {whole:?} -> {cropped:?}"
        );
    }

    /// A quarter turn has to swap the rendered dimensions, and the pixels have
    /// to follow. Reported size alone would be satisfied by doing nothing.
    #[test]
    fn a_user_quarter_turn_rotates_the_pixels() {
        let Some(gpu) = gpu() else { return };
        let raw = edge_sensor();
        let (w, h) = (raw.width, raw.height);

        let turned = EditStack {
            geometry: crate::develop::Geometry {
                quarter_turns: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(turned.geometry.output_size(w, h), (h, w));

        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let pixels = pipeline
            .render_to_pixels(gpu, &session, &turned, h, w)
            .expect("render");

        // The edge ran vertically; after a quarter turn it must run
        // horizontally, so the two halves differ top-to-bottom instead.
        let luma = |x: u32, y: u32| {
            let i = ((y * h + x) * 4) as usize;
            pixels[i] as f64 + pixels[i + 1] as f64 + pixels[i + 2] as f64
        };
        let top = luma(h / 2, 2);
        let bottom = luma(h / 2, w - 3);
        eprintln!("after a quarter turn: top {top:.0}, bottom {bottom:.0}");
        assert!(
            (top - bottom).abs() > 60.0,
            "the edge did not become horizontal: top {top:.0}, bottom {bottom:.0}"
        );
    }

    /// A mask's own clarity, whites and blacks act like the global ones
    /// where it covers, and a mask elsewhere leaves the pixels alone.
    #[test]
    fn a_mask_carries_clarity_and_endpoints() {
        let Some(gpu) = gpu() else { return };
        use crate::develop::mask::{LocalAdjustments, Mask, MaskShape};
        let raw = edge_sensor_of(256);
        let (w, h) = (raw.width, raw.height);
        let masked = |cx: f32, adjustments: LocalAdjustments| EditStack {
            masks: vec![Mask {
                adjustments,
                ..Mask::single(MaskShape::Radial {
                    cx,
                    cy: 0.5,
                    radius_x: 2.0,
                    radius_y: 2.0,
                    angle: 0.0,
                    feather: 0.01,
                })
            }],
            ..Default::default()
        };
        let edge = |stack: &EditStack| {
            let px = render(gpu, &raw, stack);
            let (mut lo, mut hi, mut sum, mut n) = (255.0f64, 0.0f64, 0.0f64, 0.0f64);
            for y in 2..h - 2 {
                for x in (w / 2 - 3)..(w / 2 + 3) {
                    let v = px[((y * w + x) * 4 + 1) as usize] as f64;
                    lo = lo.min(v);
                    hi = hi.max(v);
                    sum += v;
                    n += 1.0;
                }
            }
            (hi - lo, sum / n)
        };
        let adj = |f: fn(&mut LocalAdjustments)| {
            let mut a = LocalAdjustments::default();
            f(&mut a);
            a
        };

        let (plain, plain_mean) = edge(&EditStack::default());
        let (clear, _) = edge(&masked(0.5, adj(|a| a.clarity = 0.8)));
        let (soft, _) = edge(&masked(0.5, adj(|a| a.clarity = -0.8)));
        eprintln!(
            "masked edge contrast: plain {plain:.1}, +clarity {clear:.1}, -clarity {soft:.1}"
        );
        assert!(clear > plain + 2.0 && soft < plain - 2.0);

        let (_, whiter) = edge(&masked(0.5, adj(|a| a.whites = 0.8)));
        let (_, crushed) = edge(&masked(0.5, adj(|a| a.blacks = -0.8)));
        assert!(
            whiter > plain_mean + 2.0,
            "whites: {plain_mean:.1} -> {whiter:.1}"
        );
        assert!(
            crushed < plain_mean - 1.0,
            "blacks: {plain_mean:.1} -> {crushed:.1}"
        );

        // Centred far off the frame, the mask covers nothing here.
        let (away, away_mean) = edge(&masked(-5.0, adj(|a| a.clarity = 0.8)));
        assert!((away - plain).abs() < 0.5 && (away_mean - plain_mean).abs() < 0.5);
    }

    /// Clarity has to raise local contrast, and turn it back down again.
    ///
    /// Measured as the spread across an edge rather than a mean: local contrast
    /// leaves the frame average essentially untouched, which is the whole point
    /// of it, so any frame-wide statistic would call this a no-op.
    #[test]
    fn clarity_raises_and_lowers_local_contrast() {
        let Some(gpu) = gpu() else { return };
        // Large enough that a 2% radius is several pixels; see `edge_sensor_of`.
        let raw = edge_sensor_of(256);
        let (w, h) = (raw.width, raw.height);

        // Spread over the band straddling the edge, where local contrast acts.
        let contrast = |clarity: f32| {
            let stack = EditStack {
                detail: crate::develop::Detail {
                    clarity,
                    ..Default::default()
                },
                ..Default::default()
            };
            let px = render(gpu, &raw, &stack);
            let mut lo = 255.0f64;
            let mut hi = 0.0f64;
            for y in 2..h - 2 {
                for x in (w / 2 - 3)..(w / 2 + 3) {
                    let i = ((y * w + x) * 4) as usize;
                    let v = px[i + 1] as f64;
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            hi - lo
        };

        let (neutral, boosted, softened) = (contrast(0.0), contrast(0.8), contrast(-0.8));
        eprintln!(
            "edge contrast: neutral {neutral:.1}, +clarity {boosted:.1}, -clarity {softened:.1}"
        );

        assert!(
            boosted > neutral + 2.0,
            "clarity did not increase local contrast: {neutral:.1} -> {boosted:.1}"
        );
        assert!(
            softened < neutral - 2.0,
            "negative clarity did not soften: {neutral:.1} -> {softened:.1}"
        );
    }

    /// Neutral detail must be a byte-level no-op — the path every unedited
    /// photo takes through the two new texture fetches.
    #[test]
    fn neutral_detail_leaves_the_render_untouched() {
        let Some(gpu) = gpu() else { return };
        let raw = edge_sensor();
        let plain = render(gpu, &raw, &EditStack::default());
        let explicit = render(
            gpu,
            &raw,
            &EditStack {
                detail: crate::develop::Detail::default(),
                ..Default::default()
            },
        );
        assert_eq!(plain, explicit);
    }

    /// Zoom has to magnify, not merely crop the output.
    ///
    /// Rendered from the edge scene: at fit, the vertical edge sits at the
    /// middle of the frame and both halves are visible. Zoomed into the bright
    /// half at 1:1, the whole output must be bright — an implementation that
    /// resized the target without narrowing the sampled region would still
    /// show the edge, and would look plausible in a screenshot.
    #[test]
    fn zooming_magnifies_rather_than_resizing() {
        let Some(gpu) = gpu() else { return };
        let raw = edge_sensor_of(256);
        let (w, h) = (raw.width, raw.height);

        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");
        let stack = EditStack::default();

        let mean_of = |view: Option<&crate::develop::geometry::ViewPort>, tw, th| {
            let px = pipeline
                .render_view(gpu, &session, &stack, tw, th, view, false)
                .expect("render");
            interior_mean(&px, tw, th)
        };

        let fit = mean_of(None, w, h);

        // 64 output pixels at 1:1 is 64 of 256, centred three-quarters across —
        // entirely inside the bright half.
        let zoomed = crate::develop::geometry::ViewPort {
            zoom: 1.0,
            centre: [0.75, 0.5],
            width: 64,
            height: 64,
        };
        let bright = mean_of(Some(&zoomed), 64, 64);

        let dark_side = crate::develop::geometry::ViewPort {
            centre: [0.25, 0.5],
            ..zoomed
        };
        let dark = mean_of(Some(&dark_side), 64, 64);

        eprintln!("fit {fit:?}  zoom into bright {bright:?}  into dark {dark:?}");
        assert!(
            bright[1] > fit[1] + 20.0,
            "zooming into the bright half did not brighten: {fit:?} -> {bright:?}"
        );
        assert!(
            dark[1] < fit[1] - 20.0,
            "zooming into the dark half did not darken: {fit:?} -> {dark:?}"
        );
        // And each view is uniform, because it sits entirely on one side.
        for (name, m) in [("bright", bright), ("dark", dark)] {
            let spread = m.iter().cloned().fold(f64::MIN, f64::max)
                - m.iter().cloned().fold(f64::MAX, f64::min);
            assert!(spread < 6.0, "the {name} view is not uniform: {m:?}");
        }
    }

    /// A flat scene with reproducible noise on it.
    ///
    /// Noise reduction cannot be tested on a clean fixture, and sharpening
    /// cannot be told apart from it: both change a smooth gradient, but only
    /// one is supposed to leave the grain behind. Deterministic rather than
    /// random, so a failure is the same failure twice.
    fn noisy_sensor(size: u32, amplitude: i32) -> RawImage {
        let mut raw = edge_sensor_of(size);
        let (w, h) = (raw.width, raw.height);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                // A hash, not a PRNG: cheap, and uncorrelated between
                // neighbours, which is what makes it look like sensor noise
                // rather than a pattern the filters could exploit.
                let hash = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 1024;
                let jitter = (hash as i32 - 512) * amplitude / 512;
                raw.cfa[i] = (raw.cfa[i] as i32 + jitter).clamp(0, raw.white[1] as i32) as u16;
            }
        }
        raw
    }

    /// Roughness: how much neighbouring pixels disagree, which is what noise is
    /// and what noise reduction should lower.
    ///
    /// Restricted to a horizontal band, always: measuring the whole frame lets
    /// a hard edge dominate the number and hides what these filters do.
    fn roughness_in(pixels: &[u8], w: u32, h: u32, xs: std::ops::Range<u32>) -> f64 {
        let mut sum = 0.0;
        let mut n = 0u32;
        for y in 2..h - 2 {
            for x in xs.clone() {
                let at =
                    |dx: u32, dy: u32| pixels[(((y + dy) * w + x + dx) * 4 + 1) as usize] as f64;
                sum += (at(0, 0) - at(1, 0)).abs() + (at(0, 0) - at(0, 1)).abs();
                n += 1;
            }
        }
        sum / n.max(1) as f64
    }

    /// Noise reduction has to smooth the grain, and sharpening has to roughen
    /// it — measured at 1:1, which is the only place either is meaningful.
    #[test]
    fn sharpening_and_noise_reduction_move_roughness_the_right_way() {
        let Some(gpu) = gpu() else { return };
        let raw = noisy_sensor(192, 120);
        let (w, h) = (raw.width, raw.height);

        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");

        // Rendered at the source's own size so one output pixel is one source
        // pixel. At a proxy size the neighbourhood is already averaged away and
        // neither control can do anything.
        let measure = |detail: crate::develop::Detail| {
            let stack = EditStack {
                detail,
                ..Default::default()
            };
            let px = pipeline
                .render_to_pixels(gpu, &session, &stack, w, h)
                .expect("render");
            // The bright third: flat, well clear of the edge, and where the
            // added grain is a sane fraction of the signal. Measuring the whole
            // frame lets the hard edge dominate the number and hides what these
            // filters actually do — the first version of this test did exactly
            // that and reported noise reduction as a 7% effect.
            roughness_in(&px, w, h, 2 * w / 3..w - 2)
        };

        let plain = measure(crate::develop::Detail::default());
        let denoised = measure(crate::develop::Detail {
            noise: 0.9,
            ..Default::default()
        });
        let sharpened = measure(crate::develop::Detail {
            sharpen: 0.8,
            ..Default::default()
        });

        eprintln!("roughness: plain {plain:.2}, denoised {denoised:.2}, sharpened {sharpened:.2}");
        assert!(
            denoised < plain * 0.8,
            "noise reduction barely smoothed anything: {plain:.2} -> {denoised:.2}"
        );
        assert!(
            sharpened > plain * 1.15,
            "sharpening did not add local contrast: {plain:.2} -> {sharpened:.2}"
        );
    }

    /// Masking is what keeps sharpening off flat areas, where it has no detail
    /// to recover and only amplifies noise. Without it the control is actively
    /// harmful on sky and shadow.
    ///
    /// Measured on a **lightly** noisy scene and away from the edge. Masking
    /// separates flat from detailed by local contrast, so heavy grain reads as
    /// detail and correctly defeats it — which is exactly why the accepted
    /// order is to denoise first and sharpen second. A first version of this
    /// test used the same heavy noise as the one above and showed masking doing
    /// almost nothing; the control was fine and the test was wrong.
    #[test]
    fn masking_withholds_sharpening_from_flat_areas() {
        let Some(gpu) = gpu() else { return };
        let raw = noisy_sensor(192, 25);
        let (w, h) = (raw.width, raw.height);

        let pipeline = Pipeline::new(gpu);
        let session = pipeline.prepare(gpu, &raw).expect("prepare");

        let measure = |masking: f32| {
            let stack = EditStack {
                detail: crate::develop::Detail {
                    sharpen: 0.8,
                    sharpen_masking: masking,
                    ..Default::default()
                },
                ..Default::default()
            };
            // The bright third: flat, clear of the edge, and where the grain
            // is a sane fraction of the signal. The dark side of this scene is
            // an eighth as bright, so the same grain is proportionally huge
            // there and reads as detail rather than noise.
            roughness_in(
                &pipeline
                    .render_to_pixels(gpu, &session, &stack, w, h)
                    .expect("render"),
                w,
                h,
                2 * w / 3..w - 2,
            )
        };

        let (unmasked, masked) = (measure(0.0), measure(1.0));
        eprintln!("sharpening roughness: unmasked {unmasked:.2}, fully masked {masked:.2}");
        assert!(
            masked < unmasked * 0.9,
            "masking did not hold sharpening back on a flat, noisy scene: \
             {unmasked:.2} -> {masked:.2}"
        );
    }

    /// Neutral settings must be a byte-level no-op — the path every unedited
    /// photo takes through eight new texture fetches.
    #[test]
    fn neutral_sharpening_leaves_the_render_untouched() {
        let Some(gpu) = gpu() else { return };
        let raw = noisy_sensor(64, 80);
        let plain = render(gpu, &raw, &EditStack::default());
        let explicit = render(
            gpu,
            &raw,
            &EditStack {
                detail: crate::develop::Detail {
                    sharpen: 0.0,
                    noise: 0.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert_eq!(plain, explicit);
    }

    /// X-Trans must not fringe a neutral edge either.
    ///
    /// The same property the Bayer path is held to, on the layout that has no
    /// Bayer structure to exploit. Worth pinning separately: X-Trans reaches
    /// the shader through an entirely different function, so the Bayer test
    /// says nothing at all about it.
    #[test]
    fn an_x_trans_neutral_edge_does_not_pick_up_colour() {
        let Some(gpu) = gpu() else { return };
        let raw = xtrans_edge_sensor(96);
        let (w, h) = (raw.width, raw.height);
        let pixels = render(gpu, &raw, &EditStack::default());

        let mut worst = 0i32;
        for y in 4..h - 4 {
            for x in 4..w - 4 {
                let i = ((y * w + x) * 4) as usize;
                let p = [pixels[i] as i32, pixels[i + 1] as i32, pixels[i + 2] as i32];
                worst = worst.max(p.iter().max().unwrap() - p.iter().min().unwrap());
            }
        }
        eprintln!("worst X-Trans edge fringe: {worst}");
        // A plain weighted average per colour measured 195 here, and adding
        // colour-difference red and blue without making green edge-directed
        // left it there — green being wrong at the edge made every channel
        // wrong together. Both together give 2, the same as the Bayer path.
        assert!(
            worst < 16,
            "a neutral edge fringed by {worst}/255 on X-Trans — the demosaic is              inventing colour at edges"
        );
    }

    /// Vignetting correction has to lift the corners and leave the centre
    /// alone — that is what distinguishes it from an exposure change.
    #[test]
    fn vignetting_brightens_the_corners_and_not_the_centre() {
        let Some(gpu) = gpu() else { return };
        let raw = neutral_sensor(0.25);
        let (w, h) = (raw.width, raw.height);

        let sample = |stack: &EditStack, x: u32, y: u32| {
            let px = render(gpu, &raw, stack);
            px[((y * w + x) * 4 + 1) as usize] as f64
        };

        let plain = EditStack::default();
        let lifted = EditStack {
            lens: crate::develop::LensCorrection {
                vignetting: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let (cx, cy) = (w / 2, h / 2);
        let centre_before = sample(&plain, cx, cy);
        let centre_after = sample(&lifted, cx, cy);
        let corner_before = sample(&plain, 2, 2);
        let corner_after = sample(&lifted, 2, 2);

        eprintln!(
            "centre {centre_before} -> {centre_after}, corner {corner_before} -> {corner_after}"
        );
        assert!(
            corner_after > corner_before + 10.0,
            "the corner did not brighten: {corner_before} -> {corner_after}"
        );
        assert!(
            (centre_after - centre_before).abs() < 2.0,
            "the centre moved: {centre_before} -> {centre_after} — this is behaving like \
             an exposure change rather than a vignette"
        );
    }

    /// Distortion has to move the image radially, and the two directions have
    /// to move it opposite ways.
    #[test]
    fn distortion_moves_the_frame_radially() {
        let Some(gpu) = gpu() else { return };
        let raw = disc_sensor(128);
        let (w, h) = (raw.width, raw.height);

        // The disc's area. Reading further out shrinks what is visible, so
        // positive distortion — which corrects barrel — makes the disc smaller.
        let disc_area = |distortion: f32| {
            let stack = EditStack {
                lens: crate::develop::LensCorrection {
                    distortion,
                    ..Default::default()
                },
                ..Default::default()
            };
            let px = render(gpu, &raw, &stack);
            let mut bright = 0u32;
            for y in 2..h - 2 {
                for x in 2..w - 2 {
                    if px[((y * w + x) * 4 + 1) as usize] > 150 {
                        bright += 1;
                    }
                }
            }
            bright as f64
        };

        let (plain, pulled, pushed) = (disc_area(0.0), disc_area(1.0), disc_area(-1.0));
        eprintln!("disc area: plain {plain}, +distortion {pulled}, -distortion {pushed}");
        assert!(
            pulled < plain * 0.95,
            "positive distortion did not pull the frame in: {plain} -> {pulled}"
        );
        assert!(
            pushed > plain * 1.05,
            "negative distortion did not push it out: {plain} -> {pushed}"
        );
    }

    /// Neutral lens settings must be a byte-level no-op — three radial
    /// computations that every unedited photo passes through.
    #[test]
    fn neutral_lens_correction_leaves_the_render_untouched() {
        let Some(gpu) = gpu() else { return };
        let raw = edge_sensor_of(64);
        let plain = render(gpu, &raw, &EditStack::default());
        let explicit = render(
            gpu,
            &raw,
            &EditStack {
                lens: crate::develop::LensCorrection::default(),
                ..Default::default()
            },
        );
        assert_eq!(plain, explicit);
    }

    /// The proxy exists so slider changes stay cheap; it must not distort the
    /// image or upscale small ones.
    #[test]
    fn proxy_preserves_aspect_and_never_upscales() {
        assert_eq!(fit_within(6000, 4000, 1400), (1400, 933));
        assert_eq!(fit_within(4000, 6000, 1400), (933, 1400));
        assert_eq!(fit_within(800, 600, 1400), (800, 600), "must not upscale");
        assert_eq!(fit_within(1400, 1400, 1400), (1400, 1400), "exact fit");
        // A very wide crop must still produce at least one row.
        let (_, h) = fit_within(10000, 3, 1400);
        assert!(h >= 1, "height collapsed to zero");
    }

    /// The neutral test sensor, `width` × `height`: its 16 × 16 pattern tiled.
    fn wide_sensor(width: u32, height: u32) -> RawImage {
        let tile = neutral_sensor(0.4);
        let cfa = (0..height)
            .flat_map(|y| (0..width).map(move |x| (y % 16, x % 16)))
            .map(|(y, x)| tile.cfa[(y * 16 + x) as usize])
            .collect();
        RawImage {
            cfa,
            width,
            height,
            active_area: Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            ..tile
        }
    }

    /// A flat grey rendered photo, `width` × `height`.
    fn grey_rendered(width: u32, height: u32) -> crate::raster::Source {
        use crate::raster::{Pixels, RenderedImage, Source};
        Source::Rendered(RenderedImage::new(
            Pixels::Eight(image::ImageBuffer::from_pixel(
                width,
                height,
                image::Rgb([150; 3]),
            )),
            1,
            None,
        ))
    }

    /// Film grain is as rough across a 3:2 frame as down it. Grain on a
    /// framed-UV grid had cells the frame's aspect wide, half as wide again as
    /// tall, which read as a brick grid: across/down came out at 0.67.
    #[test]
    fn grain_is_square_on_a_wide_frame() {
        let Some(gpu) = gpu() else { return };
        let (w, h) = (1500, 1000);
        let pipeline = Pipeline::new(gpu);
        let session = pipeline
            .prepare_source(gpu, &grey_rendered(w, h))
            .expect("rendered");
        let stack = EditStack {
            effects: crate::develop::effects::Effects {
                grain: 1.0,
                // The coarsest grain: cells ~1.7 px, big enough to have a shape.
                grain_size: 3.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let px = pipeline
            .render_to_pixels(gpu, &session, &stack, w, h)
            .expect("render");
        let g = |x: u32, y: u32| px[((y * w + x) * 4 + 1) as usize] as f64;
        let (mut across, mut down) = (0.0, 0.0);
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                across += (g(x, y) - g(x + 1, y)).abs();
                down += (g(x, y) - g(x, y + 1)).abs();
            }
        }
        let ratio = across / down;
        eprintln!(
            "grain across/down {ratio:.3}, mean step {:.2}",
            down / f64::from((w - 1) * (h - 1))
        );
        assert!(down > 0.0, "no grain at all");
        assert!(
            (0.85..1.18).contains(&ratio),
            "grain is stretched: across/down {ratio:.3}"
        );
    }

    /// The whole of an export of `session` at its own size, as 8-bit RGBA.
    fn export_all(pipeline: &Pipeline, gpu: &Gpu, session: &DevelopSession) -> Vec<u8> {
        let stack = EditStack::default();
        let (w, h) = session.framed_size(&stack);
        match pipeline.render_for_export(gpu, session, &stack, w, h, color::Output::SRGB, false) {
            Ok(RenderedPixels::Eight(p)) => p,
            other => panic!("export: {:?}", other.map(|_| "sixteen-bit")),
        }
    }

    /// Past wgpu's default 8,192 px texture edge, where a Sony A7R
    /// IV/V, Nikon Z8/Z9 or Fuji GFX frame lands: the device asks for the
    /// adapter's own limit, so a raw and a rendered file that wide open and
    /// export at full size instead of panicking.
    #[test]
    fn a_photo_wider_than_8192_px_opens_and_exports() {
        let Some(gpu) = gpu() else { return };
        // A Sony A7R IV/V frame's width.
        let wide = 9504;
        assert!(gpu.max_edge() >= wide, "limit {}", gpu.max_edge());
        let pipeline = Pipeline::new(gpu);

        let raw = crate::raster::Source::Raw(wide_sensor(wide, 16));
        let session = pipeline.prepare_source(gpu, &raw).expect("raw");
        assert_eq!(session.stored_size(), (wide, 16));
        let pixels = export_all(&pipeline, gpu, &session);
        assert_eq!(pixels.len(), (wide * 16 * 4) as usize);

        let session = pipeline
            .prepare_source(gpu, &grey_rendered(wide, 8))
            .expect("rendered");
        assert_eq!(session.stored_size(), (wide, 8));
        let mean = interior_mean(&export_all(&pipeline, gpu, &session), wide, 8);
        for c in mean {
            assert!((c - 150.0).abs() <= 2.0, "came back {mean:?}");
        }
    }

    /// Wider still than the card can hold: a rendered panorama is
    /// developed at the largest whole fraction that fits, its tone intact; a
    /// raw, which cannot be scaled before demosaicing, is refused with a
    /// reason. Neither panics.
    #[test]
    fn a_photo_past_the_gpu_limit_is_scaled_or_refused() {
        let Some(gpu) = gpu() else { return };
        let pipeline = Pipeline::new(gpu);
        let past = gpu.max_edge() + 2;

        let session = pipeline
            .prepare_source(gpu, &grey_rendered(past, 12))
            .expect("rendered");
        assert_eq!(session.stored_size(), (past / 2, 6));
        let mean = interior_mean(&export_all(&pipeline, gpu, &session), past / 2, 6);
        for c in mean {
            assert!((c - 150.0).abs() <= 2.0, "came back {mean:?}");
        }

        let raw = crate::raster::Source::Raw(wide_sensor(past, 2));
        let refused = pipeline.prepare_source(gpu, &raw).err().expect("refused");
        assert!(refused.to_string().contains("at most"), "{refused:#}");

        // One pixel across at half size is no pixels at all.
        let strip = pipeline.prepare_source(gpu, &grey_rendered(1, past)).err();
        let strip = strip.expect("refused");
        assert!(strip.to_string().contains("too narrow"), "{strip:#}");
    }

    /// A GPU error inside `checked` comes back as an `Err`; outside one, the
    /// tests keep wgpu's panic, so this is what stands between a bad size and
    /// an aborted app.
    #[test]
    fn a_gpu_error_inside_checked_is_an_err() {
        let Some(gpu) = gpu() else { return };
        let too_big = gpu.max_edge() + 1;
        let got = gpu.checked("making a texture too big", || {
            Ok(gpu.create_render_target(too_big, 1))
        });
        let Err(e) = got else { panic!("no error") };
        assert!(e.to_string().contains("too big"), "{e:#}");
    }

    #[test]
    fn downscale_factor_is_the_smallest_that_fits() {
        assert_eq!(downscale_factor((9504, 6336), 16384), 1);
        assert_eq!(downscale_factor((16384, 2000), 16384), 1);
        assert_eq!(downscale_factor((16385, 2000), 16384), 2);
        assert_eq!(downscale_factor((2000, 40000), 16384), 3);
        assert_eq!(downscale_factor((0, 0), 16384), 1);
    }
}
