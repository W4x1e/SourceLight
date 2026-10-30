//! Rendering a delivered file.
//!
//! Export runs the **same develop shader** as the preview, at full sensor
//! resolution. That is the whole point: a preview is only honest if the file it
//! promises goes through the identical code path. Nothing here is allowed to
//! reinterpret the edit stack.
//!
//! Resizing happens *after* the render, on the CPU with a proper filter, rather
//! than by rendering straight to the target size. The develop pass samples with
//! a single bilinear tap, which is fine when the scale factor is near 1 but
//! aliases badly at export reductions — a 6000px frame going to 1080px would
//! sample one photosite in thirty and turn fine detail into noise.

use anyhow::{bail, Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod batch;
pub mod exif;
pub mod icc;
pub mod location;
pub mod xmp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Jpeg,
    Png,
    Tiff,
    WebP,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tif",
            Self::WebP => "webp",
        }
    }

    /// Whether `quality` means anything for this format.
    pub fn is_lossy(&self) -> bool {
        matches!(self, Self::Jpeg)
    }

    /// Whether this format can carry sixteen bits a channel.
    pub fn holds_sixteen_bits(&self) -> bool {
        matches!(self, Self::Png | Self::Tiff)
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }
}

/// Output sharpening, applied *after* the resize so it compensates for the
/// softening a downscale introduces — the point of a delivery-time sharpen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum SharpenLevel {
    #[default]
    Off,
    Low,
    Standard,
    High,
}

impl SharpenLevel {
    /// `(sigma, threshold)` for `image::imageops::unsharpen`; `None` skips the
    /// pass entirely so an "off" export is byte-for-byte the unsharpened render.
    /// A smaller sigma keeps the halo tight; a lower threshold sharpens into
    /// finer contrast. Restrained values — this is corrective, not a look.
    fn params(self) -> Option<(f32, i32)> {
        match self {
            Self::Off => None,
            Self::Low => Some((0.8, 4)),
            Self::Standard => Some((1.2, 2)),
            Self::High => Some((1.8, 1)),
        }
    }
}

/// One of nine placements for a watermark, matching the dialog's 3×3 grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum WatermarkAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    #[default]
    BottomRight,
}

/// An image (logo) watermark composited onto the delivered file. Text watermarks
/// are a separate follow-up (they need a font rasteriser).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Watermark {
    /// Absolute path to the logo image (PNG with alpha is the intended case).
    pub image_path: String,
    pub anchor: WatermarkAnchor,
    /// Overall opacity multiplier, 0..=1.
    pub opacity: f32,
    /// Logo width as a fraction of the exported image's width, 0..=1.
    pub scale: f32,
}

/// A text watermark. The font is a file the user chose at export time (there is
/// no bundled face), rendered white with a soft dark shadow for legibility over
/// an unknown photograph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TextWatermark {
    pub text: String,
    /// Absolute path to a `.ttf`/`.otf` font file.
    pub font_path: String,
    pub anchor: WatermarkAnchor,
    /// Overall opacity multiplier, 0..=1.
    pub opacity: f32,
    /// Cap height as a fraction of the image's longer edge, roughly 0.01..=0.2.
    pub size: f32,
}

/// The colour space an export is rendered into and tagged as.
///
/// Each is an output matrix, a transfer curve, and the profile that says so
/// — see [`crate::gpu::color::Output`]. Out-of-gamut colour is clipped per
/// channel — relative colorimetric in effect; there is no perceptual gamut
/// mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum ColorSpace {
    /// The web and most screens; what every export was before.
    #[default]
    Srgb,
    /// Wide gamut: Apple displays, most recent phones and laptops. Holds the
    /// saturated reds and greens sRGB clips.
    DisplayP3,
    /// The print and pre-press standard: wider than sRGB in the greens and
    /// cyans, with a pure 2.2 gamma.
    AdobeRgb,
}

impl ColorSpace {
    /// What the render encodes into: matrix and transfer curve.
    pub fn output(self) -> crate::gpu::color::Output {
        use crate::gpu::color::{self, Output, Transfer};
        match self {
            Self::Srgb => Output::SRGB,
            Self::DisplayP3 => Output::new(color::rec2020_to_display_p3(), Transfer::Srgb),
            Self::AdobeRgb => Output::new(color::rec2020_to_adobe_rgb(), Transfer::Gamma22),
        }
    }

    /// The ICC profile that says so.
    pub fn icc_profile(self) -> Vec<u8> {
        match self {
            Self::Srgb => icc::srgb_profile(),
            Self::DisplayP3 => icc::display_p3_profile(),
            Self::AdobeRgb => icc::adobe_rgb_profile(),
        }
    }
}

/// Bits a channel in the delivered file.
///
/// Sixteen is for a file that will be edited further: under a strong curve in
/// another editor, eight bits bands in a smooth sky where sixteen does not.
/// Only PNG and TIFF hold it; the others are always eight
/// ([`ExportOptions::sixteen_bit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum BitDepth {
    #[default]
    Eight,
    Sixteen,
}

/// How colours the output space cannot hold are brought into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum RenderingIntent {
    /// Only out-of-gamut colours move, to the edge of the gamut, keeping their
    /// hue. Everything the output can hold is exact. What every export was.
    #[default]
    RelativeColorimetric,
    /// Colours near and past the edge are eased inward together, so saturated
    /// gradients keep their steps — at the cost of slightly muting the most
    /// saturated colours that did fit.
    Perceptual,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// 1..=100, lossy formats only.
    pub quality: u8,
    /// Longest edge in pixels. `None` exports at full sensor resolution.
    pub max_edge: Option<u32>,
    /// Delivery-time output sharpening, applied after any resize.
    #[serde(default)]
    pub sharpen: SharpenLevel,
    /// An optional image watermark, composited last.
    #[serde(default)]
    pub watermark: Option<Watermark>,
    /// An optional text watermark, drawn after the image one.
    #[serde(default)]
    pub text_watermark: Option<TextWatermark>,
    /// The colour space to render into and tag the file as. The pixels must
    /// have been rendered for it — see [`ColorSpace::output`].
    #[serde(default)]
    pub color_space: ColorSpace,
    /// How out-of-gamut colour is handled; relative colorimetric unless asked.
    #[serde(default)]
    pub intent: RenderingIntent,
    /// Leave EXIF and XMP out entirely — camera, lens, exposure, capture
    /// date, creator and copyright. For a file posted somewhere that should not
    /// learn any of it. The sRGB profile is kept either way: it describes the
    /// pixels, not the photographer.
    #[serde(default)]
    pub strip_metadata: bool,
    /// What the file says beyond the camera's record, when not stripped:
    /// location, names, the AI description and AI keywords, each its
    /// own choice.
    #[serde(default)]
    pub metadata: crate::xmp::MetadataChoices,
    /// Bits a channel, where the format holds sixteen.
    #[serde(default)]
    pub bit_depth: BitDepth,
}

impl ExportOptions {
    /// Whether this export renders and writes sixteen bits a channel: asked
    /// for, in a format that holds them.
    pub fn sixteen_bit(&self) -> bool {
        self.bit_depth == BitDepth::Sixteen && self.format.holds_sixteen_bits()
    }

    /// What the render encodes into: the colour space, under the chosen intent.
    pub fn output(&self) -> crate::gpu::color::Output {
        self.color_space
            .output()
            .with_intent(self.intent == RenderingIntent::Perceptual)
    }
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Jpeg,
            // High enough that JPEG artefacts are not the limiting factor, low
            // enough to avoid the steep size cost of the last few points.
            quality: 92,
            max_edge: None,
            sharpen: SharpenLevel::Off,
            watermark: None,
            text_watermark: None,
            color_space: ColorSpace::Srgb,
            intent: RenderingIntent::RelativeColorimetric,
            strip_metadata: false,
            metadata: crate::xmp::MetadataChoices::default(),
            bit_depth: BitDepth::Eight,
        }
    }
}

/// Roughly how many bytes exporting a `width` x `height` photo will write.
///
/// Only decides whether to warn that a batch may not fit on the drive, so
/// it leans high: a warning a little early costs a click, a silent full disk
/// costs the export. Every format here is RGB, eight bits a channel unless a
/// PNG or TIFF asks for sixteen, which doubles it; TIFF is written
/// uncompressed, so its figure is exact, and WebP is written *lossless*, so it
/// sizes like PNG rather than like JPEG. Cropping only makes the real file
/// smaller, so the uncropped size is used.
pub fn estimate_bytes(width: u32, height: u32, options: &ExportOptions) -> u64 {
    let long = width.max(height).max(1) as f64;
    let scale = options
        .max_edge
        .map_or(1.0, |edge| (edge as f64 / long).min(1.0));
    let pixels = width as f64 * height as f64 * scale * scale;
    let per_pixel = match options.format {
        ExportFormat::Tiff => 3.0,
        ExportFormat::Png => 2.5,
        ExportFormat::WebP => 2.2,
        ExportFormat::Jpeg => match options.quality {
            95.. => 1.0,
            90..=94 => 0.7,
            80..=89 => 0.45,
            _ => 0.3,
        },
    };
    let depth = if options.sixteen_bit() { 2.0 } else { 1.0 };
    // Headers, the ICC profile and metadata.
    (pixels * per_pixel * depth) as u64 + 64 * 1024
}

/// Rows of the image each core sharpens at a time; see [`unsharpen_in_bands`].
const SHARPEN_BAND: u32 = 256;

/// [`image::imageops::unsharpen`] across every core, a band of rows at a time,
/// with the same result to the byte.
///
/// An unsharp mask is a blur and a per-pixel difference, and the blur reaches
/// only a few rows either way (nine taps at High), clamping at the image's own
/// edges. So a band sharpened with a margin of rows either side comes out
/// exactly as it would inside the whole image. On one core it was 136–172 ms
/// of a 24 MP export, and with sharpening on the slowest step of a batch.
fn unsharpen_in_bands<T: Channel>(img: &Rgb<T>, sigma: f32, threshold: i32, band: u32) -> Rgb<T>
where
    image::Rgb<T>: image::Pixel<Subpixel = T>,
{
    use rayon::prelude::*;
    // Far more than the blur reaches, which costs a few rows a band.
    let margin = (sigma * 8.0).ceil() as u32 + 8;
    let (width, height) = img.dimensions();
    let band = band.max(1);
    let row = width as usize * 3;
    let mut out = vec![T::zeroed(); row * height as usize];
    out.par_chunks_mut(row * band as usize)
        .enumerate()
        .for_each(|(k, rows)| {
            let top = k as u32 * band;
            let bottom = top + (rows.len() / row) as u32;
            let from = top.saturating_sub(margin);
            let to = (bottom + margin).min(height);
            let view = image::imageops::crop_imm(img, 0, from, width, to - from);
            let sharp = image::imageops::unsharpen(&*view, sigma, threshold);
            let skip = (top - from) as usize * row;
            rows.copy_from_slice(&sharp.as_raw()[skip..skip + rows.len()]);
        });
    Rgb::<T>::from_raw(width, height, out).expect("the bands make up the image")
}

/// A channel of delivered pixels: eight bits, or sixteen.
trait Channel: image::Primitive + bytemuck::Pod + Send + Sync + 'static {
    /// RGB of this channel, as an encoder names it.
    const RGB: ExtendedColorType;
    /// Steps in one 8-bit level, which is what sharpening thresholds are in.
    const PER_LEVEL: i32;
    fn to_f32(self) -> f32;
    /// Rounded and clamped into the channel's range.
    fn from_f32(v: f32) -> Self;
    /// Full scale.
    fn max() -> f32;
}

impl Channel for u8 {
    const RGB: ExtendedColorType = ExtendedColorType::Rgb8;
    const PER_LEVEL: i32 = 1;
    fn to_f32(self) -> f32 {
        self as f32
    }
    fn from_f32(v: f32) -> Self {
        v.round().clamp(0.0, 255.0) as u8
    }
    fn max() -> f32 {
        255.0
    }
}

impl Channel for u16 {
    const RGB: ExtendedColorType = ExtendedColorType::Rgb16;
    const PER_LEVEL: i32 = 257;
    fn to_f32(self) -> f32 {
        self as f32
    }
    fn from_f32(v: f32) -> Self {
        v.round().clamp(0.0, 65535.0) as u16
    }
    fn max() -> f32 {
        65535.0
    }
}

/// An RGB image of `T` channels.
type Rgb<T> = image::ImageBuffer<image::Rgb<T>, Vec<T>>;

/// A render as the develop pass read it back: tightly packed RGBA, eight or
/// sixteen bits a channel.
pub use crate::gpu::pipeline::RenderedPixels as Pixels;

/// [`encode`] or [`encode16`], whichever the render is.
pub fn encode_pixels(
    pixels: &Pixels,
    width: u32,
    height: u32,
    options: &ExportOptions,
    metadata: Option<&exif::ExportMetadata>,
) -> Result<Vec<u8>> {
    match pixels {
        Pixels::Eight(rgba) => encode(rgba, width, height, options, metadata),
        Pixels::Sixteen(rgba) => encode16(rgba, width, height, options, metadata),
    }
}

/// [`encode`], from a sixteen-bit render into a sixteen-bit file. Only for a
/// format that holds sixteen bits ([`ExportFormat::holds_sixteen_bits`]).
pub fn encode16(
    rgba: &[u16],
    width: u32,
    height: u32,
    options: &ExportOptions,
    metadata: Option<&exif::ExportMetadata>,
) -> Result<Vec<u8>> {
    if !options.format.holds_sixteen_bits() {
        bail!(
            "a {:?} file cannot hold sixteen bits a channel",
            options.format
        );
    }
    finish(rgb_of(rgba, width, height)?, options, metadata)
}

/// Drops alpha up front. The develop pass always writes 1.0, and carrying it
/// into a photograph only bloats formats that support it. A row a task: 24 MP
/// took 29 ms on one core.
fn rgb_of<T: Channel>(rgba: &[T], width: u32, height: u32) -> Result<Rgb<T>>
where
    image::Rgb<T>: image::Pixel<Subpixel = T>,
{
    use rayon::prelude::*;
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        bail!(
            "expected {expected} values for {width}x{height} RGBA, got {}",
            rgba.len()
        );
    }
    let mut raw = vec![T::zeroed(); expected / 4 * 3];
    raw.par_chunks_mut(width as usize * 3)
        .zip(rgba.par_chunks(width as usize * 4))
        .for_each(|(to, from)| {
            for (to, from) in to.chunks_exact_mut(3).zip(from.chunks_exact(4)) {
                to.copy_from_slice(&from[..3]);
            }
        });
    Rgb::<T>::from_raw(width, height, raw).context("sizing the export image")
}

/// Encodes a full-resolution RGBA render into a deliverable file.
///
/// `width`/`height` describe `rgba`, which is expected tightly packed.
/// `metadata` becomes the file's EXIF block, unless the options strip it — see
/// [`exif`]. TIFF is the exception: the `image` crate's TIFF encoder has no way
/// to add EXIF, so a TIFF export carries its sRGB profile and nothing else.
pub fn encode(
    rgba: &[u8],
    width: u32,
    height: u32,
    options: &ExportOptions,
    metadata: Option<&exif::ExportMetadata>,
) -> Result<Vec<u8>> {
    finish(rgb_of(rgba, width, height)?, options, metadata)
}

/// Everything after the render, at either depth: resize, sharpen, watermark,
/// then the file.
fn finish<T: Channel>(
    mut rgb: Rgb<T>,
    options: &ExportOptions,
    metadata: Option<&exif::ExportMetadata>,
) -> Result<Vec<u8>>
where
    image::Rgb<T>: image::Pixel<Subpixel = T>,
{
    if let Some(max_edge) = options.max_edge {
        let (width, height) = rgb.dimensions();
        let (w, h) = crate::gpu::pipeline::fit_within(width, height, max_edge);
        if (w, h) != (width, height) {
            // Lanczos3 rather than the Triangle filter used for library
            // thumbnails: this is a delivered file, and the cost is paid once.
            rgb = image::imageops::resize(&rgb, w, h, FilterType::Lanczos3);
        }
    }

    // Sharpen last, on the final delivery pixels — an unsharp mask over whatever
    // the resize produced. Skipped entirely when Off.
    if let Some((sigma, threshold)) = options.sharpen.params() {
        rgb = unsharpen_in_bands(&rgb, sigma, threshold * T::PER_LEVEL, SHARPEN_BAND);
    }

    // Watermark after sharpening so the logo itself isn't sharpened.
    if let Some(wm) = &options.watermark {
        apply_watermark(&mut rgb, wm).context("compositing the watermark")?;
    }
    if let Some(tw) = &options.text_watermark {
        apply_text_watermark(&mut rgb, tw).context("drawing the text watermark")?;
    }

    let (w, h) = (rgb.width(), rgb.height());
    let mut out = Vec::new();

    // Everything this pipeline writes is sRGB. Saying so is the difference
    // between a colour-managed reader knowing what the numbers mean and having
    // to assume — see `icc`.
    let profile = options.color_space.icc_profile();
    let chosen;
    let metadata = match metadata.filter(|_| !options.strip_metadata) {
        Some(m) => {
            let mut m = m.clone();
            m.apply(&options.metadata);
            chosen = m;
            Some(&chosen)
        }
        None => None,
    };
    let exif_block = metadata.map(exif::build);

    match options.format {
        ExportFormat::Jpeg => {
            let mut encoder =
                JpegEncoder::new_with_quality(&mut out, options.quality.clamp(1, 100));
            encoder
                .set_icc_profile(profile)
                .context("attaching the sRGB profile to the JPEG")?;
            if let Some(block) = &exif_block {
                encoder
                    .set_exif_metadata(block.clone())
                    .context("attaching EXIF to the JPEG")?;
            }
            encoder
                .write_image(bytemuck::cast_slice(rgb.as_raw()), w, h, T::RGB)
                .context("encoding JPEG")?;
            // IPTC Core creator and copyright, for the tools that read XMP
            // rather than EXIF — see `xmp`.
            if let Some(packet) = metadata.and_then(xmp::packet) {
                out = xmp::insert_into_jpeg(out, &packet);
            }
        }
        ExportFormat::Png => {
            let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
            encoder
                .set_icc_profile(profile)
                .context("attaching the sRGB profile to the PNG")?;
            if let Some(block) = &exif_block {
                encoder
                    .set_exif_metadata(block.clone())
                    .context("attaching EXIF to the PNG")?;
            }
            encoder
                .write_image(bytemuck::cast_slice(rgb.as_raw()), w, h, T::RGB)
                .context("encoding PNG")?;
        }
        // TIFF carries the profile in IFD tag 34675 (`ICCProfile`); WebP in an
        // `ICCP` chunk, which the encoder promotes to the extended (VP8X)
        // container for. Both are attached exactly like JPEG/PNG above, so every
        // delivered format is now a *tagged* sRGB file rather than one a
        // colour-managed reader has to assume.
        ExportFormat::Tiff => {
            // TIFF needs a seekable writer for its offset table.
            let mut cursor = std::io::Cursor::new(Vec::new());
            let mut encoder = image::codecs::tiff::TiffEncoder::new(&mut cursor);
            encoder
                .set_icc_profile(profile)
                .context("attaching the sRGB profile to the TIFF")?;
            encoder
                .write_image(bytemuck::cast_slice(rgb.as_raw()), w, h, T::RGB)
                .context("encoding TIFF")?;
            out = cursor.into_inner();
        }
        ExportFormat::WebP => {
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
            encoder
                .set_icc_profile(profile)
                .context("attaching the sRGB profile to the WebP")?;
            if let Some(block) = &exif_block {
                encoder
                    .set_exif_metadata(block.clone())
                    .context("attaching EXIF to the WebP")?;
            }
            encoder
                .write_image(bytemuck::cast_slice(rgb.as_raw()), w, h, T::RGB)
                .context("encoding WebP")?;
        }
    }

    Ok(out)
}

/// Where the top-left of the watermark goes, given the two sizes and a margin.
fn anchor_offset(
    anchor: WatermarkAnchor,
    bw: i64,
    bh: i64,
    lw: i64,
    lh: i64,
    margin: i64,
) -> (i64, i64) {
    use WatermarkAnchor::*;
    let x = match anchor {
        TopLeft | MiddleLeft | BottomLeft => margin,
        TopCenter | Center | BottomCenter => (bw - lw) / 2,
        TopRight | MiddleRight | BottomRight => bw - lw - margin,
    };
    let y = match anchor {
        TopLeft | TopCenter | TopRight => margin,
        MiddleLeft | Center | MiddleRight => (bh - lh) / 2,
        BottomLeft | BottomCenter | BottomRight => bh - lh - margin,
    };
    (x, y)
}

/// Loads, scales, and alpha-composites a logo over the delivery pixels. Blends
/// per pixel because the base is opaque RGB — there is no alpha channel to lean
/// on. A missing or unreadable file is an error, so a mistyped path fails loudly
/// rather than silently shipping unmarked files.
fn apply_watermark<T: Channel>(base: &mut Rgb<T>, wm: &Watermark) -> Result<()>
where
    image::Rgb<T>: image::Pixel<Subpixel = T>,
{
    let opacity = wm.opacity.clamp(0.0, 1.0);
    let scale = wm.scale.clamp(0.0, 1.0);
    if opacity <= 0.0 || scale <= 0.0 {
        return Ok(());
    }

    let logo = image::open(&wm.image_path)
        .with_context(|| format!("opening watermark {}", wm.image_path))?
        .to_rgba8();
    if logo.width() == 0 || logo.height() == 0 {
        bail!("watermark image is empty");
    }

    let (bw, bh) = (base.width(), base.height());
    let target_w = (((bw as f32) * scale).round() as u32).max(1);
    let s = target_w as f32 / logo.width() as f32;
    let target_h = (((logo.height() as f32) * s).round() as u32).max(1);
    let logo = image::imageops::resize(&logo, target_w, target_h, FilterType::Lanczos3);

    // A small breathing margin, tied to the frame so it looks the same at any size.
    let margin = ((bw.min(bh) as f32) * 0.03).round() as i64;
    let (ox, oy) = anchor_offset(
        wm.anchor,
        bw as i64,
        bh as i64,
        target_w as i64,
        target_h as i64,
        margin,
    );

    composite(base, &logo, (ox, oy), opacity);
    Ok(())
}

/// Lays an 8-bit RGBA `layer` over `base` with its top-left at `at`, at
/// `opacity`, whatever depth `base` is. Parts off the frame are dropped.
fn composite<T: Channel>(base: &mut Rgb<T>, layer: &image::RgbaImage, at: (i64, i64), opacity: f32)
where
    image::Rgb<T>: image::Pixel<Subpixel = T>,
{
    let (bw, bh) = (base.width() as i64, base.height() as i64);
    let scale = T::max() / 255.0;
    for (lx, ly, px) in layer.enumerate_pixels() {
        let a = (px[3] as f32 / 255.0) * opacity;
        if a <= 0.0 {
            continue;
        }
        let (bx, by) = (at.0 + lx as i64, at.1 + ly as i64);
        if bx < 0 || by < 0 || bx >= bw || by >= bh {
            continue;
        }
        let under = base.get_pixel_mut(bx as u32, by as u32);
        for c in 0..3 {
            let blended = under[c].to_f32() * (1.0 - a) + (px[c] as f32 * scale) * a;
            under[c] = T::from_f32(blended);
        }
    }
}

/// Draws a text watermark using a user-chosen font. Renders onto a transparent
/// layer (with a soft dark shadow behind white text, so it survives over a light
/// photo) and composites that with the requested opacity.
fn apply_text_watermark<T: Channel>(base: &mut Rgb<T>, tw: &TextWatermark) -> Result<()>
where
    image::Rgb<T>: image::Pixel<Subpixel = T>,
{
    use ab_glyph::{FontVec, PxScale};

    let text = tw.text.trim();
    let opacity = tw.opacity.clamp(0.0, 1.0);
    if text.is_empty() || opacity <= 0.0 {
        return Ok(());
    }

    let bytes =
        std::fs::read(&tw.font_path).with_context(|| format!("reading font {}", tw.font_path))?;
    let font =
        FontVec::try_from_vec(bytes).map_err(|_| anyhow::anyhow!("not a usable font file"))?;

    let (bw, bh) = (base.width(), base.height());
    let px = ((bw.max(bh) as f32) * tw.size.clamp(0.005, 0.3))
        .round()
        .max(8.0);
    let scale = PxScale::from(px);

    let (text_w, text_h) = imageproc::drawing::text_size(scale, &font, text);

    let shadow = (px * 0.06).round().max(1.0) as i32;
    let pad = shadow + 2;
    let layer_w = (text_w as i32 + 2 * pad).max(1) as u32;
    let layer_h = (text_h as i32 + 2 * pad).max(1) as u32;
    let mut layer = image::RgbaImage::new(layer_w, layer_h);
    imageproc::drawing::draw_text_mut(
        &mut layer,
        image::Rgba([0, 0, 0, 150]),
        pad + shadow,
        pad + shadow,
        scale,
        &font,
        text,
    );
    imageproc::drawing::draw_text_mut(
        &mut layer,
        image::Rgba([255, 255, 255, 255]),
        pad,
        pad,
        scale,
        &font,
        text,
    );

    let margin = ((bw.min(bh) as f32) * 0.03).round() as i64;
    let (ox, oy) = anchor_offset(
        tw.anchor,
        bw as i64,
        bh as i64,
        layer_w as i64,
        layer_h as i64,
        margin,
    );

    composite(base, &layer, (ox, oy), opacity);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    /// Sharpening in bands must be sharpening, byte for byte, at every level
    /// and whatever the band size — including bands smaller than the margin
    /// and one larger than the image.
    #[test]
    fn sharpening_in_bands_matches_sharpening_whole() {
        let mut seed = 7u32;
        let img = RgbImage::from_fn(301, 211, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let b = seed.to_le_bytes();
            image::Rgb([b[1], b[2], b[3]])
        });
        for level in [
            SharpenLevel::Low,
            SharpenLevel::Standard,
            SharpenLevel::High,
        ] {
            let (sigma, threshold) = level.params().unwrap();
            let whole = image::imageops::unsharpen(&img, sigma, threshold);
            for band in [1, 7, 64, 500] {
                assert!(
                    unsharpen_in_bands(&img, sigma, threshold, band) == whole,
                    "{level:?} in bands of {band} differs"
                );
            }
        }
        // And at sixteen bits, where the threshold is in the same 8-bit levels.
        let deep: Rgb<u16> = image::ImageBuffer::from_fn(301, 211, |x, y| {
            image::Rgb(img.get_pixel(x, y).0.map(|v| v as u16 * 257))
        });
        let (sigma, threshold) = SharpenLevel::Standard.params().unwrap();
        let whole = image::imageops::unsharpen(&deep, sigma, threshold * u16::PER_LEVEL);
        assert!(unsharpen_in_bands(&deep, sigma, threshold * u16::PER_LEVEL, 7) == whole);
    }

    /// A sixteen-bit PNG or TIFF keeps every one of its bits: a ramp in steps
    /// of 64 comes back exactly, which eight bits could not hold.
    #[test]
    fn sixteen_bits_survive_into_png_and_tiff() {
        let (w, h) = (1024u32, 2u32);
        let rgba: Vec<u16> = (0..h)
            .flat_map(|_| {
                (0..w).flat_map(|x| {
                    let v = (x * 64) as u16;
                    [v, 65535 - v, v / 2, 65535]
                })
            })
            .collect();
        for format in [ExportFormat::Png, ExportFormat::Tiff] {
            let options = ExportOptions {
                format,
                bit_depth: BitDepth::Sixteen,
                ..Default::default()
            };
            assert!(options.sixteen_bit());
            let bytes = encode16(&rgba, w, h, &options, None).unwrap();
            let img = image::load_from_memory(&bytes).unwrap();
            assert_eq!(img.color(), image::ColorType::Rgb16, "{format:?}");
            let back = img.to_rgb16();
            for x in 0..w {
                let v = (x * 64) as u16;
                assert_eq!(
                    back.get_pixel(x, 1).0,
                    [v, 65535 - v, v / 2],
                    "{format:?} at {x}"
                );
            }
        }
        // A format that cannot hold them is refused, not quietly cut to eight;
        // and asked for there, sixteen bits is not what is rendered.
        let jpeg = ExportOptions {
            format: ExportFormat::Jpeg,
            bit_depth: BitDepth::Sixteen,
            ..Default::default()
        };
        assert!(!jpeg.sixteen_bit());
        assert!(encode16(&rgba, w, h, &jpeg, None).is_err());
    }

    /// A watermark lands the same at sixteen bits as at eight: the deep
    /// result, taken back to eight bits, is the shallow one.
    #[test]
    fn a_watermark_composites_the_same_at_either_depth() {
        let layer = image::RgbaImage::from_fn(40, 30, |x, y| {
            image::Rgba([(x * 6) as u8, 200, (y * 8) as u8, (x * 6 + y) as u8])
        });
        let mut eight = RgbImage::from_pixel(60, 50, image::Rgb([90, 140, 30]));
        let mut sixteen: Rgb<u16> =
            image::ImageBuffer::from_pixel(60, 50, image::Rgb([90 * 257, 140 * 257, 30 * 257]));
        composite(&mut eight, &layer, (-5, 12), 0.7);
        composite(&mut sixteen, &layer, (-5, 12), 0.7);
        for (a, b) in eight.pixels().zip(sixteen.pixels()) {
            for c in 0..3 {
                let back = b[c] as f32 / 257.0;
                assert!((a[c] as f32 - back).abs() <= 0.5 + 1e-3, "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn sixteen_bits_double_the_estimate() {
        let png = |bit_depth| ExportOptions {
            format: ExportFormat::Png,
            bit_depth,
            ..Default::default()
        };
        let (eight, sixteen) = (
            estimate_bytes(6000, 4000, &png(BitDepth::Eight)),
            estimate_bytes(6000, 4000, &png(BitDepth::Sixteen)),
        );
        assert!(sixteen > eight * 19 / 10, "{eight} -> {sixteen}");
    }

    #[test]
    fn export_size_estimates_scale_with_format_quality_and_size() {
        let opts = |format, quality, max_edge| ExportOptions {
            format,
            quality,
            max_edge,
            ..Default::default()
        };
        let (w, h) = (6000, 4000);
        let tiff = estimate_bytes(w, h, &opts(ExportFormat::Tiff, 92, None));
        // Uncompressed 8-bit RGB: three bytes a pixel, plus a little.
        assert!((72_000_000..72_200_000).contains(&tiff), "{tiff}");

        let jpeg_hi = estimate_bytes(w, h, &opts(ExportFormat::Jpeg, 98, None));
        let jpeg_lo = estimate_bytes(w, h, &opts(ExportFormat::Jpeg, 60, None));
        assert!(jpeg_lo < jpeg_hi && jpeg_hi < tiff);

        // A 2048 px long edge keeps (2048/6000)^2 of the 24 MP: 2.8 MP, at three
        // bytes each plus the 64 KB allowance — about 8.45 MB.
        let resized = estimate_bytes(w, h, &opts(ExportFormat::Tiff, 92, Some(2048)));
        assert!((8_400_000..8_500_000).contains(&resized), "{resized}");
        // A limit larger than the photo does not enlarge it.
        assert_eq!(
            estimate_bytes(w, h, &opts(ExportFormat::Tiff, 92, Some(9000))),
            tiff
        );
    }

    /// A text watermark with an unreadable font path fails loudly rather than
    /// silently shipping unmarked files.
    #[test]
    fn a_missing_font_is_an_error() {
        let mut base = RgbImage::from_pixel(60, 60, image::Rgb([10, 10, 10]));
        let got = apply_text_watermark(
            &mut base,
            &TextWatermark {
                text: "© James".into(),
                font_path: "/no/such/font.ttf".into(),
                anchor: WatermarkAnchor::BottomRight,
                opacity: 1.0,
                size: 0.1,
            },
        );
        assert!(got.is_err());
    }

    /// A logo lands at its anchor, blends over the base, and leaves the opposite
    /// corner untouched.
    #[test]
    fn a_watermark_lands_at_its_anchor() {
        let mut base = RgbImage::from_pixel(200, 100, image::Rgb([128, 128, 128]));

        // A solid, fully-opaque red logo written to a temp file — apply_watermark
        // loads by path, exactly as a real export would.
        let dir = crate::test_support::Scratch::new("watermark");
        let path = dir.join("logo.png");
        image::RgbaImage::from_pixel(20, 20, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();

        apply_watermark(
            &mut base,
            &Watermark {
                image_path: path.to_string_lossy().into_owned(),
                anchor: WatermarkAnchor::BottomRight,
                opacity: 1.0,
                scale: 0.1, // 10% of 200px = a 20px-wide logo, matching the source
            },
        )
        .unwrap();

        // The far corner is untouched grey…
        assert_eq!(base.get_pixel(0, 0), &image::Rgb([128, 128, 128]));
        // …and a pixel inside the bottom-right logo box is now red.
        let px = base.get_pixel(185, 85);
        assert!(
            px[0] > 200 && px[1] < 60 && px[2] < 60,
            "expected red-ish at the anchor, got {px:?}"
        );
    }

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                px.extend_from_slice(&[
                    (x * 255 / width.max(1)) as u8,
                    (y * 255 / height.max(1)) as u8,
                    96,
                    255,
                ]);
            }
        }
        px
    }

    /// The profile has to survive encoding and come back out intact.
    ///
    /// Building valid ICC bytes and *attaching* them are separate problems, and
    /// the unit tests in `icc` only cover the first. Reading it back through a
    /// real decoder is the only thing that proves a colour-managed reader will
    /// find it — an encoder that silently dropped the profile would pass every
    /// other test here, since the pixels are unaffected either way.
    #[test]
    fn a_display_p3_export_is_tagged_display_p3() {
        use image::ImageDecoder;
        let (w, h) = (16u32, 12u32);
        let bytes = encode(
            &gradient(w, h),
            w,
            h,
            &ExportOptions {
                color_space: ColorSpace::DisplayP3,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let mut decoder =
            image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let found = decoder.icc_profile().unwrap().expect("a profile");
        assert_eq!(found, icc::display_p3_profile());
        assert_ne!(found, icc::srgb_profile());
    }

    #[test]
    fn an_exported_file_carries_a_readable_srgb_profile() {
        use image::ImageDecoder;

        let (w, h) = (32u32, 24u32);
        let expected = icc::srgb_profile();

        // Every delivered format, not just the two that always worked: TIFF and
        // WebP now tag too, and each decoder reads the profile back out of its
        // own container (JPEG APP2, PNG iCCP, TIFF tag 34675, WebP ICCP).
        for format in [
            ExportFormat::Jpeg,
            ExportFormat::Png,
            ExportFormat::Tiff,
            ExportFormat::WebP,
        ] {
            let bytes = encode(
                &gradient(w, h),
                w,
                h,
                &ExportOptions {
                    format,
                    quality: 90,
                    max_edge: None,
                    ..Default::default()
                },
                None,
            )
            .unwrap();

            let cursor = std::io::Cursor::new(bytes.as_slice());
            let found = match format {
                ExportFormat::Jpeg => image::codecs::jpeg::JpegDecoder::new(cursor)
                    .unwrap()
                    .icc_profile()
                    .unwrap(),
                ExportFormat::Png => image::codecs::png::PngDecoder::new(cursor)
                    .unwrap()
                    .icc_profile()
                    .unwrap(),
                ExportFormat::Tiff => image::codecs::tiff::TiffDecoder::new(cursor)
                    .unwrap()
                    .icc_profile()
                    .unwrap(),
                ExportFormat::WebP => image::codecs::webp::WebPDecoder::new(cursor)
                    .unwrap()
                    .icc_profile()
                    .unwrap(),
            };

            let profile =
                found.unwrap_or_else(|| panic!("{format:?} came out with no ICC profile attached"));
            assert_eq!(
                profile, expected,
                "{format:?} altered the profile on the way through"
            );
        }
    }

    #[test]
    fn every_format_produces_a_decodable_file_of_the_right_size() {
        let (w, h) = (64u32, 40u32);
        let px = gradient(w, h);

        for format in [
            ExportFormat::Jpeg,
            ExportFormat::Png,
            ExportFormat::Tiff,
            ExportFormat::WebP,
        ] {
            let bytes = encode(
                &px,
                w,
                h,
                &ExportOptions {
                    format,
                    ..Default::default()
                },
                None,
            )
            .unwrap_or_else(|e| panic!("{format:?} failed: {e}"));

            let decoded = image::load_from_memory(&bytes)
                .unwrap_or_else(|e| panic!("{format:?} did not re-decode: {e}"));
            assert_eq!(
                (decoded.width(), decoded.height()),
                (w, h),
                "{format:?} changed the dimensions"
            );
        }
    }

    #[test]
    fn resizing_preserves_aspect_and_never_upscales() {
        let (w, h) = (100u32, 50u32);
        let px = gradient(w, h);

        let smaller = encode(
            &px,
            w,
            h,
            &ExportOptions {
                format: ExportFormat::Png,
                max_edge: Some(40),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let decoded = image::load_from_memory(&smaller).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (40, 20));

        let bigger = encode(
            &px,
            w,
            h,
            &ExportOptions {
                format: ExportFormat::Png,
                max_edge: Some(500),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let decoded = image::load_from_memory(&bigger).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (w, h),
            "a larger limit must not upscale"
        );
    }

    /// Lossless formats have to be exactly lossless — a resize or a stray
    /// colour conversion in the path would show up here.
    #[test]
    fn png_round_trips_pixels_exactly() {
        let (w, h) = (16u32, 16u32);
        let px = gradient(w, h);
        let bytes = encode(
            &px,
            w,
            h,
            &ExportOptions {
                format: ExportFormat::Png,
                ..Default::default()
            },
            None,
        )
        .unwrap();

        let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                assert_eq!(
                    decoded.get_pixel(x, y).0,
                    [px[i], px[i + 1], px[i + 2]],
                    "pixel ({x},{y}) changed"
                );
            }
        }
    }

    #[test]
    fn quality_affects_lossy_size_only() {
        let (w, h) = (128u32, 128u32);
        let px = gradient(w, h);
        let at = |format, quality| {
            encode(
                &px,
                w,
                h,
                &ExportOptions {
                    format,
                    quality,
                    max_edge: None,
                    ..Default::default()
                },
                None,
            )
            .unwrap()
            .len()
        };

        assert!(at(ExportFormat::Jpeg, 40) < at(ExportFormat::Jpeg, 95));
        assert_eq!(
            at(ExportFormat::Png, 40),
            at(ExportFormat::Png, 95),
            "quality should not affect a lossless format"
        );
    }

    fn read_exif(bytes: &[u8]) -> Option<::exif::Exif> {
        ::exif::Reader::new()
            .read_from_container(&mut std::io::Cursor::new(bytes))
            .ok()
    }

    #[test]
    fn metadata_lands_in_every_format_that_can_carry_it() {
        let (w, h) = (48, 32);
        let meta = exif::ExportMetadata {
            camera: Some("Sony ILCE-6400".into()),
            copyright: Some("© 2026 Jane Doe".into()),
            ..Default::default()
        };
        for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::WebP] {
            let options = ExportOptions {
                format,
                ..Default::default()
            };
            let bytes = encode(&gradient(w, h), w, h, &options, Some(&meta)).unwrap();
            let found =
                read_exif(&bytes).unwrap_or_else(|| panic!("{format:?} export should carry EXIF"));
            let model = found
                .get_field(::exif::Tag::Model, ::exif::In::PRIMARY)
                .unwrap_or_else(|| panic!("{format:?} lost the camera"));
            assert!(
                model.display_value().to_string().contains("ILCE-6400"),
                "{format:?}: {}",
                model.display_value()
            );
            // Still a valid image, and still tagged sRGB, with EXIF alongside.
            image::load_from_memory(&bytes).unwrap();
        }
    }

    #[test]
    fn a_jpeg_also_carries_creator_and_copyright_as_xmp() {
        let (w, h) = (48, 32);
        let meta = exif::ExportMetadata {
            creator: Some("Jane Doe".into()),
            copyright: Some("© 2026 Jane Doe".into()),
            ..Default::default()
        };
        let bytes = encode(
            &gradient(w, h),
            w,
            h,
            &ExportOptions::default(),
            Some(&meta),
        )
        .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("http://ns.adobe.com/xap/1.0/"),
            "no XMP segment"
        );
        assert!(text.contains("<rdf:li>Jane Doe</rdf:li>"));
        assert!(text.contains("© 2026 Jane Doe"));
        // Inserting a segment must leave both the image and the EXIF readable.
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), w);
        assert!(read_exif(&bytes).is_some());

        let stripped = encode(
            &gradient(w, h),
            w,
            h,
            &ExportOptions {
                strip_metadata: true,
                ..Default::default()
            },
            Some(&meta),
        )
        .unwrap();
        assert!(!String::from_utf8_lossy(&stripped).contains("xap/1.0"));
    }

    /// Location, names, the AI description and AI keywords are each their
    /// own choice; the rest of the record stays whatever they say.
    #[test]
    fn each_metadata_choice_leaves_out_only_its_own() {
        use crate::xmp::MetadataChoices;
        let (w, h) = (48, 32);
        let meta = exif::ExportMetadata {
            camera: Some("Sony ILCE-6400".into()),
            keywords: vec!["castle".into(), "beer".into()],
            ai_keywords: vec!["beer".into()],
            people: vec!["Ada Lovelace".into()],
            caption: Some("A sign reading 14 Elm Road".into()),
            caption_is_ai: true,
            location: Some(location::PhotoLocation {
                position: Some((55.9533, -3.1883)),
                name: Some("Edinburgh, United Kingdom".into()),
                city: Some("Edinburgh".into()),
                country: Some("United Kingdom".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let export = |metadata: MetadataChoices| {
            let options = ExportOptions {
                metadata,
                ..Default::default()
            };
            encode(&gradient(w, h), w, h, &options, Some(&meta)).unwrap()
        };
        let gps = |bytes: &[u8]| {
            read_exif(bytes)
                .unwrap()
                .get_field(::exif::Tag::GPSLatitude, ::exif::In::PRIMARY)
                .is_some()
        };

        let all = export(MetadataChoices::ALL);
        assert!(gps(&all));
        let text = String::from_utf8_lossy(&all);
        assert!(text.contains("<photoshop:City>Edinburgh</photoshop:City>"));
        assert!(text.contains("<rdf:li>Ada Lovelace</rdf:li>"));
        assert!(text.contains("<rdf:li>beer</rdf:li>"));
        assert!(text.contains("14 Elm Road"));
        assert!(
            text.contains(
                "<photoshop:CaptionWriter>AI (on-device model)</photoshop:CaptionWriter>"
            ),
            "the model's caption says whose it is"
        );

        // A client delivery: where, but not who, nor the model's words.
        let delivered = export(MetadataChoices::default());
        assert!(gps(&delivered));
        let text = String::from_utf8_lossy(&delivered);
        assert!(!text.contains("Ada Lovelace"), "a name left in");
        assert!(!text.contains("14 Elm Road"), "the model's caption left in");
        assert!(
            !text.contains("<rdf:li>beer</rdf:li>"),
            "an AI keyword left in"
        );
        assert!(
            text.contains("<rdf:li>castle</rdf:li>"),
            "the user's keyword went"
        );

        // No location takes the model's caption too: it names the setting.
        let unlocated = export(MetadataChoices {
            location: false,
            ..MetadataChoices::ALL
        });
        assert!(!gps(&unlocated), "GPS left in despite No location");
        let text = String::from_utf8_lossy(&unlocated);
        assert!(!text.contains("Edinburgh"), "the town left in");
        assert!(
            !text.contains("14 Elm Road"),
            "the caption naming the place left in"
        );
        assert!(
            text.contains("Ada Lovelace"),
            "names are their own choice now"
        );
        let model = read_exif(&unlocated)
            .unwrap()
            .get_field(::exif::Tag::Model, ::exif::In::PRIMARY)
            .map(|f| f.display_value().to_string());
        assert!(
            model.is_some_and(|m| m.contains("ILCE-6400")),
            "the rest went too"
        );
    }

    #[test]
    fn stripping_leaves_no_exif_behind() {
        let (w, h) = (48, 32);
        let meta = exif::ExportMetadata {
            creator: Some("Jane Doe".into()),
            ..Default::default()
        };
        for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::WebP] {
            let options = ExportOptions {
                format,
                strip_metadata: true,
                ..Default::default()
            };
            let bytes = encode(&gradient(w, h), w, h, &options, Some(&meta)).unwrap();
            assert!(
                read_exif(&bytes).is_none(),
                "{format:?} kept EXIF despite strip_metadata"
            );
        }
    }

    #[test]
    fn a_mismatched_buffer_is_rejected() {
        let err = encode(&[0; 10], 64, 40, &ExportOptions::default(), None).unwrap_err();
        assert!(err.to_string().contains("expected"));
    }

    #[test]
    fn format_is_inferred_from_the_chosen_filename() {
        assert_eq!(
            ExportFormat::from_path(Path::new("a/b/photo.JPEG")),
            Some(ExportFormat::Jpeg)
        );
        assert_eq!(
            ExportFormat::from_path(Path::new("photo.tif")),
            Some(ExportFormat::Tiff)
        );
        assert_eq!(ExportFormat::from_path(Path::new("photo.arw")), None);
        assert_eq!(ExportFormat::from_path(Path::new("photo")), None);
    }
}
