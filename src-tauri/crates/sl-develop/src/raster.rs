//! Photos that are not raw files: JPEG, PNG, TIFF and WebP.
//!
//! Phones, JPEG-only cameras, scanners, drones and other apps' exports all
//! write these, and until now the library walked past them — invisible to
//! search, cull, places and people, which is most of what "find any photo"
//! promises. Phones are also what carry GPS.
//!
//! Two things differ from a raw, and both are small:
//!
//! * **The preview is the file.** A raw embeds a camera JPEG; here the photo
//!   *is* the rendered image, so ingest and the loupe's 1:1 zoom read it
//!   directly (re-encoded only when the webview cannot show the format).
//! * **Develop starts after the demosaic.** The pixels are already RGB, so they
//!   are linearised, the neutral tone curve is undone, and the result is
//!   uploaded as the pipeline's linear image (`Pipeline::prepare_rendered`).
//!   At neutral settings the photo opens exactly as the file shows it, and
//!   every control from white balance on works as it does on a raw. What
//!   cannot come back is what the camera threw away: an 8-bit file has little
//!   headroom, and the Develop panel says so.
//!
//! **A RAW+JPEG pair stays one photo**, as before: a JPEG beside a raw of the
//! same name is the raw's companion, not a photo of its own.
//!
//! **Colour comes from the file's ICC profile** (`icc`), or failing one from
//! its EXIF, which is the only place a camera says it shot Adobe RGB
//! (`profile_of`). Most are sRGB and are shown untouched; an Adobe RGB export,
//! an Adobe RGB camera JPEG or a Display P3 phone photo is converted to sRGB
//! for the grid and the loupe, and opens in Develop with its own primaries, so
//! none looks washed out.

pub mod icc;
#[cfg(target_os = "macos")]
pub mod imageio;
#[cfg(windows)]
pub mod wic;

use anyhow::{Context, Result};
use image::{DynamicImage, ImageBuffer, Rgb};
use std::path::Path;

use crate::exif_read::ColourSpace;

/// Rendered formats the library catalogues, lowercase and without the dot.
pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "webp"];

/// HEIF: iPhones' HEIC, and Fujifilm's HIF. Read through the operating
/// system's codec, so only where there is one wired up: WIC on Windows,
/// ImageIO on a Mac.
pub const HEIF_EXTENSIONS: &[&str] = &["heic", "heif", "hif"];

pub fn is_raster_extension(ext: &str) -> bool {
    EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
        || (HEIF_READABLE && is_heif_extension(ext))
}

/// Whether this build has an operating-system HEIF decoder to ask.
const HEIF_READABLE: bool = cfg!(any(windows, target_os = "macos"));

fn is_heif_extension(ext: &str) -> bool {
    HEIF_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
}

fn is_heif(path: &Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str())
        .is_some_and(is_heif_extension)
}

/// Decoded 8-bit pixels and the ICC profile that came with them, if any.
pub type WithProfile = (ImageBuffer<Rgb<u8>, Vec<u8>>, Option<Vec<u8>>);

/// A HEIF photo's pixels, decoded by the operating system and already upright
/// (the codec applies the container's rotation itself, see `wic` and
/// `imageio`), with its colour profile if it carries one.
fn decode_heif(path: &Path) -> Result<WithProfile> {
    #[cfg(windows)]
    {
        wic::decode(path)
    }
    #[cfg(target_os = "macos")]
    {
        imageio::decode(path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    anyhow::bail!("HEIC is not read on this system yet: {}", path.display())
}

/// The most memory one decode may ask for its pixels. A file's header
/// says how big it is, and a 200-byte TIFF can claim 200,000 px square: the
/// allocation that asked for failed outright, and a failed allocation aborts
/// the app — on every launch, since the scan reaches the file again. 2 GiB is
/// a 25,000 px square at 8 bits, or 18,000 at 16: well past any camera.
pub const MAX_DECODE_BYTES: u64 = 2 << 30;

/// The longest side the decoders accept, for the same reason.
const MAX_DECODE_EDGE: u32 = 65_535;

/// The largest photo file read whole into memory. LibRaw refuses a bigger raw
/// only once all of it has been read, and a raster file this big would not
/// decode within [`MAX_DECODE_BYTES`] either.
pub const MAX_FILE_BYTES: u64 = i32::MAX as u64;

/// Reads a photo file whole, refusing one over [`MAX_FILE_BYTES`] before
/// reading any of it.
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    let read = || format!("cannot read {}", path.display());
    let len = std::fs::metadata(path).with_context(read)?.len();
    anyhow::ensure!(
        len <= MAX_FILE_BYTES,
        "{} is too large to open: {} MB",
        path.display(),
        len >> 20
    );
    std::fs::read(path).with_context(read)
}

/// Refuses an image whose pixels would need more than [`MAX_DECODE_BYTES`].
pub fn check_decode_size(path: &Path, bytes: u64) -> Result<()> {
    anyhow::ensure!(
        bytes <= MAX_DECODE_BYTES,
        "{} is too large to open: its pixels would take {} MB of memory",
        path.display(),
        bytes >> 20
    );
    Ok(())
}

/// A decoder for a format the image crate reads, with limits on what it may
/// allocate. Only `ImageReader::decode` applies the crate's own cap, and the
/// paths here decode through a decoder, so it is set here.
fn decoder<'a>(path: &Path, data: &'a [u8]) -> Result<impl image::ImageDecoder + 'a> {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits.max_image_width = Some(MAX_DECODE_EDGE);
    limits.max_image_height = Some(MAX_DECODE_EDGE);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format()?;
    reader.limits(limits);
    reader
        .into_decoder()
        .with_context(|| format!("cannot decode {}", path.display()))
}

/// Decodes an image in memory up to [`MAX_DECODE_BYTES`].
/// `image::load_from_memory` stops at the image crate's own 512 MiB, which a
/// 200 MP phone photo (599 MB of pixels) or a big panorama passes: it never
/// got into the catalogue, only into the scan's failed count.
pub fn load_from_memory(data: &[u8]) -> Result<DynamicImage> {
    load_within(data, MAX_DECODE_BYTES)
}

fn load_within(data: &[u8], max_alloc: u64) -> Result<DynamicImage> {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_alloc);
    limits.max_image_width = Some(MAX_DECODE_EDGE);
    limits.max_image_height = Some(MAX_DECODE_EDGE);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format()?;
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// The pixels of `decoder`, refused first if they would not fit the limit:
/// the step `ImageReader::decode` takes and `DynamicImage::from_decoder` does
/// not.
///
/// The limit covers the full-size RGB copy Develop makes too, which for a
/// greyscale file is three times the decoded size.
fn pixels_of(path: &Path, decoder: impl image::ImageDecoder) -> Result<DynamicImage> {
    let ((w, h), colour) = (decoder.dimensions(), decoder.color_type());
    let channel = u64::from(colour.bytes_per_pixel() / colour.channel_count());
    let rgb = u64::from(w) * u64::from(h) * 3 * channel;
    check_decode_size(path, decoder.total_bytes().max(rgb))?;
    DynamicImage::from_decoder(decoder).with_context(|| format!("cannot decode {}", path.display()))
}

/// The colour space a file's pixels are in, **only if it isn't sRGB**: `None`
/// is the common case, and it lets a JPEG be shown untouched.
///
/// An embedded profile says it best. Without one, the EXIF can still say Adobe
/// RGB — and does, on every JPEG a camera shoots in Adobe RGB, since cameras
/// do not embed profiles (`exif_read::ColourSpace`). Read as sRGB, those came
/// out visibly flat.
pub fn profile_of(icc: Option<Vec<u8>>, exif: Option<ColourSpace>) -> Option<icc::Profile> {
    match icc.and_then(|b| icc::Profile::parse(&b)) {
        Some(p) => (!p.is_srgb()).then_some(p),
        None => (exif == Some(ColourSpace::AdobeRgb)).then(icc::Profile::adobe_rgb),
    }
}

/// The colour space of a raw's embedded preview, when it isn't sRGB: the
/// preview's own profile or EXIF first (Fujifilm and Panasonic give theirs
/// EXIF), then what the raw itself says the camera was set to (`raw`).
///
/// A camera set to Adobe RGB encodes its raw's preview in Adobe RGB too, and
/// that preview is what the grid, the loupe and the lightbox show.
pub fn preview_profile(jpeg: &[u8], raw: Option<ColourSpace>) -> Option<icc::Profile> {
    use image::ImageDecoder;
    let icc = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(jpeg))
        .ok()
        .and_then(|mut d| d.icc_profile().ok().flatten());
    let own = crate::exif_read::read(jpeg).colour_space;
    profile_of(icc, own.or(raw))
}

/// The median green of a camera's JPEG in linear light, read through its
/// colour space (`preview_profile`) from a spread of about a quarter of a
/// million pixels. What a raw's default brightness is matched to
/// (`ToneParams::exposure_matching`). `None` for a thumbnail too small to
/// say, or one that will not decode.
pub fn median_green(jpeg: &[u8], space: Option<ColourSpace>) -> Option<f32> {
    let img = image::load_from_memory(jpeg).ok()?.to_rgb8();
    if img.width().max(img.height()) < 64 {
        return None;
    }
    let lut: [f32; 256] = match preview_profile(jpeg, space) {
        Some(p) => std::array::from_fn(|i| p.linear(1, i as f32 / 255.0)),
        None => srgb_lut8(),
    };
    let step = (img.pixels().len() / 250_000).max(1);
    let mut greens: Vec<f32> = img
        .pixels()
        .step_by(step)
        .map(|p| lut[p.0[1] as usize])
        .collect();
    let mid = greens.len() / 2;
    let (_, median, _) = greens.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    Some(*median)
}

/// Converts pixels in `profile` to 8-bit sRGB, for the grid and the loupe:
/// the webview takes an untagged image as sRGB.
///
/// Colours outside sRGB are pulled in at constant hue and luminance
/// (`gpu::color::gamut_map`, what Develop does), not clipped channel by
/// channel: a clipped Adobe RGB green turns yellower and a clipped blue turns
/// cyan, and those are the colours an Adobe RGB file holds beyond sRGB.
pub fn to_srgb(img: &DynamicImage, profile: &icc::Profile) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    use crate::gpu::color;
    use rayon::prelude::*;
    let m = color::multiply(
        crate::raw::decoder::invert3(SRGB_TO_XYZ).expect("sRGB inverts"),
        profile.to_xyz,
    );
    // Linear to encoded sRGB, fine enough that 8-bit output never steps.
    const STEPS: usize = 4096;
    let encode: Vec<u8> = (0..=STEPS)
        .map(|i| {
            let v = i as f32 / STEPS as f32;
            let e = if v <= 0.003_130_8 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            (e * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect();
    let finish = |lin: [f32; 3], out: &mut [u8]| {
        for (o, v) in out.iter_mut().zip(color::gamut_map(color::apply(m, lin))) {
            *o = encode[(v.clamp(0.0, 1.0) * STEPS as f32).round() as usize];
        }
    };
    let (w, h) = (img.width(), img.height());
    // In `usize`: a picture over 1.4 gigapixels passes `u32::MAX` bytes.
    let mut out = vec![0u8; w as usize * h as usize * 3];
    if matches!(
        img.color().bytes_per_pixel() / img.color().channel_count(),
        1
    ) {
        let lut = profile.lut8();
        let src = img.to_rgb8();
        out.par_chunks_mut(3)
            .zip(src.as_raw().par_chunks(3))
            .for_each(|(o, px)| finish(std::array::from_fn(|c| lut[c][px[c] as usize]), o));
    } else {
        let src = img.to_rgb16();
        out.par_chunks_mut(3)
            .zip(src.as_raw().par_chunks(3))
            .for_each(|(o, px)| {
                finish(
                    std::array::from_fn(|c| profile.linear(c, px[c] as f32 / 65535.0)),
                    o,
                )
            });
    }
    ImageBuffer::from_raw(w, h, out).expect("sized to fit")
}

pub fn is_raster(path: &Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str())
        .is_some_and(is_raster_extension)
}

/// Whether `data` is a JPEG, which the webview shows as-is.
fn is_jpeg(data: &[u8]) -> bool {
    data.starts_with(&[0xFF, 0xD8, 0xFF])
}

/// The photo as a JPEG the webview can show, at full resolution, in sRGB.
///
/// An sRGB JPEG is returned untouched: re-encoding would cost time and a
/// generation of loss on the pixels the loupe is judging. Everything else is
/// decoded, converted to sRGB if its profile says it isn't, and encoded once
/// at high quality.
pub fn displayable(path: &Path, data: &[u8]) -> Result<Vec<u8>> {
    displayable_within(path, data, None)
}

/// [`displayable`], fitted to `edge` on its long side when it is bigger.
///
/// For the scan, which only makes a preview of it. At full
/// size an iPhone photo, all Display P3, or an Adobe RGB export was converted
/// to sRGB pixel by pixel, encoded and decoded again whole, to be shrunk to
/// 1600 px at the end. Shrunk first, all of that is done on 2.5 MP. A JPEG
/// that's already sRGB still comes back as it is.
pub fn displayable_within(path: &Path, data: &[u8], edge: Option<u32>) -> Result<Vec<u8>> {
    use image::ImageDecoder;
    let (img, profile) = if is_heif(path) {
        let (rgb, icc) = decode_heif(path)?;
        (DynamicImage::ImageRgb8(rgb), profile_of(icc, None))
    } else {
        let mut decoder = decoder(path, data)?;
        let profile = profile_of(
            decoder.icc_profile().ok().flatten(),
            crate::exif_read::read(data).colour_space,
        );
        if profile.is_none() && is_jpeg(data) {
            return Ok(data.to_vec());
        }
        (pixels_of(path, decoder)?, profile)
    };
    let img = match edge {
        Some(edge) if img.width().max(img.height()) > edge => crate::raw::shrink(&img, edge),
        _ => img,
    };
    let rgb = match &profile {
        Some(p) => to_srgb(&img, p),
        None => img.to_rgb8(),
    };
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
        .encode_image(&rgb)
        .with_context(|| format!("cannot re-encode {}", path.display()))?;
    Ok(out.into_inner())
}

/// What the scan records for a rendered file beyond its EXIF: its pixel size,
/// and, for HEIF, an orientation that overrides the EXIF tag.
///
/// The size is the stored one, before any EXIF rotation: the same convention
/// as a raw's sensor size. Read from the header, so it costs no decode.
///
/// A HEIF comes out of the codec already upright, so its orientation is 1
/// whatever the EXIF says. The HEIF specification says the same (the
/// container's rotation is the one to apply), and applying the EXIF tag as
/// well would turn every portrait iPhone photo on its side.
pub fn probe(path: &Path, data: &[u8]) -> (Option<(u32, u32)>, Option<u32>) {
    if !is_heif(path) {
        return (dimensions(data), None);
    }
    #[cfg(windows)]
    let size = wic::size(path).ok();
    #[cfg(target_os = "macos")]
    let size = imageio::size(path).ok();
    #[cfg(not(any(windows, target_os = "macos")))]
    let size = None;
    (size, Some(1))
}

/// The stored pixel size from a file's header.
pub fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Decoded pixels, still in the file's own encoding, at the precision the
/// file has. Linearised on the way to the GPU.
pub enum Pixels {
    Eight(ImageBuffer<Rgb<u8>, Vec<u8>>),
    Sixteen(ImageBuffer<Rgb<u16>, Vec<u16>>),
}

/// A rendered photo, ready for the develop pipeline. Stored orientation, like
/// a raw: the develop pass rotates as it samples.
pub struct RenderedImage {
    pub pixels: Pixels,
    /// EXIF orientation, 1..=8.
    pub orientation: u8,
    /// The file's colour space; sRGB when it names none.
    pub profile: icc::Profile,
    /// Boxed: 3 KB inline would make every `Source` that size.
    lut8: Box<[[f32; 256]; 3]>,
}

impl RenderedImage {
    pub fn new(pixels: Pixels, orientation: u8, profile: Option<icc::Profile>) -> Self {
        let profile = profile.unwrap_or_else(icc::Profile::srgb);
        RenderedImage {
            pixels,
            orientation,
            lut8: Box::new(profile.lut8()),
            profile,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        match &self.pixels {
            Pixels::Eight(p) => p.dimensions(),
            Pixels::Sixteen(p) => p.dimensions(),
        }
    }

    /// Rows `y0..y0 + rows` as linear RGBA in the file's own primaries, one
    /// `f32` per channel.
    ///
    /// By band rather than whole so a 24MP frame never needs a 400 MB float
    /// copy in memory on its way to the GPU.
    pub fn linear_rows(&self, y0: u32, rows: u32, out: &mut Vec<f32>) {
        use rayon::prelude::*;
        let (w, _) = self.size();
        // In `usize`: a wide panorama's offset passes `u32::MAX`.
        let (w, y0, rows) = (w as usize, y0 as usize, rows as usize);
        let (start, len) = (y0 * w * 3, rows * w * 3);
        out.clear();
        out.resize(w * rows * 4, 1.0);
        let pixels = out.par_chunks_mut(4);
        let lut = &self.lut8;
        match &self.pixels {
            Pixels::Eight(p) => pixels
                .zip(p.as_raw()[start..start + len].par_chunks(3))
                .for_each(|(o, px)| {
                    for c in 0..3 {
                        o[c] = lut[c][px[c] as usize];
                    }
                }),
            Pixels::Sixteen(p) => pixels
                .zip(p.as_raw()[start..start + len].par_chunks(3))
                .for_each(|(o, px)| {
                    for c in 0..3 {
                        o[c] = self.profile.linear(c, px[c] as f32 / 65535.0);
                    }
                }),
        }
    }
}

/// The sRGB transfer curve, undone.
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// [`srgb_to_linear`] for every 8-bit value.
pub fn srgb_lut8() -> [f32; 256] {
    std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0))
}

/// Linear sRGB (D65) to CIE XYZ: what a raw's camera matrix is to its sensor.
pub const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175],
    [0.019_333_9, 0.119_192, 0.950_304_1],
];

/// Decodes a rendered photo for Develop.
pub fn decode(path: &Path) -> Result<RenderedImage> {
    use image::ImageDecoder;
    if is_heif(path) {
        let (rgb, icc) = decode_heif(path)?;
        let profile = icc.and_then(|b| icc::Profile::parse(&b));
        // Upright already: see `probe`.
        return Ok(RenderedImage::new(Pixels::Eight(rgb), 1, profile));
    }
    let data = read_file(path)?;
    let exif = crate::exif_read::read(&data);
    let orientation = exif.orientation.clamp(1, 8) as u8;
    let mut decoder = decoder(path, &data)?;
    let profile = profile_of(decoder.icc_profile().ok().flatten(), exif.colour_space);
    let img = pixels_of(path, decoder)?;
    let pixels = match img {
        DynamicImage::ImageRgb8(_)
        | DynamicImage::ImageRgba8(_)
        | DynamicImage::ImageLuma8(_)
        | DynamicImage::ImageLumaA8(_) => Pixels::Eight(img.into_rgb8()),
        // 16-bit TIFF and PNG keep their precision, which is the headroom a
        // scan or an exported master has over a JPEG.
        other => Pixels::Sixteen(other.into_rgb16()),
    };
    Ok(RenderedImage::new(pixels, orientation, profile))
}

/// What Develop opens: sensor data to demosaic, or pixels already rendered.
pub enum Source {
    Raw(crate::raw::decoder::RawImage),
    Rendered(RenderedImage),
}

/// Opens `path` for Develop by whichever route its format needs.
pub fn load(path: &Path, decoder: &dyn crate::raw::decoder::RawDecoder) -> Result<Source> {
    if is_raster(path) {
        decode(path).map(Source::Rendered)
    } else {
        decoder.decode(path).map(Source::Raw)
    }
}

/// Adobe RGB to 8-bit sRGB by the two specifications' own published
/// matrices, independent of anything read from a profile: the reference the
/// colour tests hold the app to.
#[cfg(any(test, feature = "test-helpers"))]
pub fn adobe_to_srgb_by_the_book(rgb: [u8; 3]) -> [f32; 3] {
    let lin = rgb.map(|v| (v as f32 / 255.0).powf(563.0 / 256.0));
    let adobe_to_xyz = [
        [0.576_67, 0.185_56, 0.188_23],
        [0.297_34, 0.627_36, 0.075_29],
        [0.027_03, 0.070_69, 0.991_34],
    ];
    let xyz_to_srgb = [
        [3.240_6, -1.537_2, -0.498_6],
        [-0.968_9, 1.875_8, 0.041_5],
        [0.055_7, -0.204, 1.057],
    ];
    let srgb = crate::gpu::color::apply(xyz_to_srgb, crate::gpu::color::apply(adobe_to_xyz, lin));
    srgb.map(|v| {
        let v = v.clamp(0.0, 1.0);
        let e = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        e * 255.0
    })
}

/// `image::imageops::resize(img, width, height, FilterType::Triangle)`, to
/// the bit, several times faster.
///
/// The automatic analysis passes resize every preview this way, SigLIP 2's to
/// 224 px and the face detector's to 640, and on a GPU that was most of each
/// photo's time: 15 of 23 ms for the search index on an RTX 5070 Ti, about 20
/// of the face pass's 27. The weights, and the order each output's products
/// are added in, are the image crate's; what differs is that the weights are
/// worked out once per row and column, each pass reads its rows in order, and
/// rounding to a byte is two instructions rather than a call into the C
/// library (`mask::round_to_byte`). `resize_triangle_matches_the_image_crate`
/// holds it to that.
pub fn resize_triangle(
    img: &ImageBuffer<Rgb<u8>, Vec<u8>>,
    width: u32,
    height: u32,
) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return ImageBuffer::new(width, height);
    }
    if (width, height) == (w, h) {
        return img.clone();
    }
    let src = img.as_raw();
    let row = w as usize * 3;
    // Down the columns first, into floats, as the image crate does.
    let mut mid = vec![0f32; row * height as usize];
    for ((left, weights), out) in triangle_weights(h, height)
        .into_iter()
        .zip(mid.chunks_exact_mut(row))
    {
        for (i, weight) in weights.into_iter().enumerate() {
            let line = &src[(left + i) * row..][..row];
            for (o, &v) in out.iter_mut().zip(line) {
                *o += v as f32 * weight;
            }
        }
    }
    // Then along the rows, rounded to bytes.
    let columns = triangle_weights(w, width);
    let mut out = Vec::with_capacity(width as usize * height as usize * 3);
    for line in mid.chunks_exact(row) {
        for (left, weights) in &columns {
            let mut t = [0f32; 3];
            for (i, weight) in weights.iter().enumerate() {
                let p = &line[(left + i) * 3..][..3];
                t[0] += p[0] * weight;
                t[1] += p[1] * weight;
                t[2] += p[2] * weight;
            }
            out.extend(t.map(crate::develop::mask::round_to_byte));
        }
    }
    ImageBuffer::from_raw(width, height, out).expect("sized to fit")
}

/// The image crate's Triangle weights from `src` positions to `dst`: for
/// each destination position, the first source position it reads and the
/// normalised weight of each from there.
fn triangle_weights(src: u32, dst: u32) -> Vec<(usize, Vec<f32>)> {
    let ratio = src as f32 / dst as f32;
    let sratio = if ratio < 1.0 { 1.0 } else { ratio };
    // The filter's support, 1, scaled.
    let support = sratio;
    (0..dst)
        .map(|o| {
            let centre = (o as f32 + 0.5) * ratio;
            let left = ((centre - support).floor() as i64).clamp(0, src as i64 - 1);
            let right = ((centre + support).ceil() as i64).clamp(left + 1, src as i64);
            let centre = centre - 0.5;
            let mut weights: Vec<f32> = (left..right)
                .map(|i| {
                    let x = (i as f32 - centre) / sratio;
                    if x.abs() < 1.0 {
                        1.0 - x.abs()
                    } else {
                        0.0
                    }
                })
                .collect();
            let mut sum = 0.0;
            for w in &weights {
                sum += w;
            }
            for w in &mut weights {
                *w /= sum;
            }
            (left as usize, weights)
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::test_helpers::camera_jpeg;
    use super::*;

    /// The decode for previews takes the limit it is given: the
    /// image crate's own default of 512 MiB turned away 200 MP photos.
    #[test]
    fn decoding_for_previews_uses_the_apps_own_limit() {
        let img = image::RgbImage::from_pixel(400, 300, image::Rgb([90, 120, 150]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let needs = 400 * 300 * 3;
        assert!(
            load_within(&png, needs - 1).is_err(),
            "refused below what it needs"
        );
        assert_eq!(load_within(&png, needs).unwrap().width(), 400);
        const { assert!(MAX_DECODE_BYTES > 600 << 20, "room for a 200 MP photo") };
        assert_eq!(load_from_memory(&png).unwrap().height(), 300);
    }

    fn tagged(format: image::ImageFormat, rgb: [u8; 3], icc: &[u8]) -> Vec<u8> {
        use image::ImageEncoder;
        let img = ImageBuffer::from_pixel(32, 32, Rgb(rgb));
        let mut out = Vec::new();
        match format {
            image::ImageFormat::Png => {
                let mut e = image::codecs::png::PngEncoder::new(&mut out);
                e.set_icc_profile(icc.to_vec()).unwrap();
                e.write_image(img.as_raw(), 32, 32, image::ExtendedColorType::Rgb8)
                    .unwrap();
            }
            _ => {
                let mut e = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95);
                e.set_icc_profile(icc.to_vec()).unwrap();
                e.write_image(img.as_raw(), 32, 32, image::ExtendedColorType::Rgb8)
                    .unwrap();
            }
        }
        out
    }

    /// An Adobe RGB export shows in the grid as its own colours (converted to
    /// sRGB), not the washed-out reading of its numbers as sRGB.
    #[test]
    fn an_adobe_rgb_photo_is_shown_in_srgb() {
        let adobe = icc::build_for_test(
            icc::ADOBE_RGB_D50,
            &icc::gamma_curve_for_test(563.0 / 256.0),
        );
        for rgb in [[100u8, 150, 200], [200, 60, 30], [128, 128, 128]] {
            let png = tagged(image::ImageFormat::Png, rgb, &adobe);
            let shown = image::load_from_memory(&displayable(Path::new("x.png"), &png).unwrap())
                .unwrap()
                .to_rgb8();
            let got = shown.get_pixel(16, 16).0;
            let want = adobe_to_srgb_by_the_book(rgb);
            for c in 0..3 {
                assert!(
                    (got[c] as f32 - want[c]).abs() <= 2.5,
                    "Adobe RGB {rgb:?} showed as {got:?}, the specifications say {want:?}"
                );
            }
        }
    }

    /// A camera's Adobe RGB JPEG carries no profile, only its EXIF saying so.
    /// It is shown in its own colours, like a tagged one, and opens in
    /// Develop with Adobe RGB's primaries.
    #[test]
    fn an_adobe_rgb_camera_jpeg_is_shown_in_its_colours() {
        for rgb in [[100u8, 150, 200], [200, 60, 30]] {
            let jpeg = camera_jpeg(rgb, 0xFFFF, b"R03");
            let shown = image::load_from_memory(&displayable(Path::new("x.jpg"), &jpeg).unwrap())
                .unwrap()
                .to_rgb8();
            let got = shown.get_pixel(16, 16).0;
            let want = adobe_to_srgb_by_the_book(rgb);
            for c in 0..3 {
                assert!(
                    (got[c] as f32 - want[c]).abs() <= 3.0,
                    "Adobe RGB {rgb:?} showed as {got:?}, the specifications say {want:?}"
                );
            }
        }
        // The same numbers in an sRGB camera file are shown as they are.
        let srgb = camera_jpeg([200, 60, 30], 1, b"R98");
        assert_eq!(displayable(Path::new("x.jpg"), &srgb).unwrap(), srgb);
    }

    /// Colour beyond sRGB is pulled in at constant hue, not clipped channel by
    /// channel — which turns a vivid Adobe RGB green yellower.
    #[test]
    fn colour_beyond_srgb_keeps_its_hue() {
        use crate::gpu::color;
        let adobe = icc::Profile::adobe_rgb();
        let src = [40u8, 210, 70];
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(2, 2, Rgb(src)));
        let got = to_srgb(&img, &adobe).get_pixel(0, 0).0;

        let m = color::multiply(
            crate::raw::decoder::invert3(SRGB_TO_XYZ).unwrap(),
            adobe.to_xyz,
        );
        let want = color::apply(
            m,
            std::array::from_fn(|c| adobe.linear(c, src[c] as f32 / 255.0)),
        );
        assert!(
            want.iter().any(|v| *v < 0.0),
            "the test colour is inside sRGB: {want:?}"
        );
        let got = got.map(|v| srgb_to_linear(v as f32 / 255.0));
        let l = (want[0] * 0.2126 + want[1] * 0.7152 + want[2] * 0.0722).clamp(0.0, 1.0);
        let scale: Vec<f32> = (0..3).map(|c| (got[c] - l) / (want[c] - l)).collect();
        for s in &scale {
            assert!(
                (s - scale[0]).abs() < 0.03,
                "hue moved: {got:?} is not on the line from grey to {want:?} ({scale:?})"
            );
        }
    }

    /// A JPEG that declares sRGB is still served byte for byte.
    #[test]
    fn an_srgb_tagged_jpeg_is_left_alone() {
        let srgb_d50 = [
            [0.436_066, 0.222_488, 0.013_916],
            [0.385_147, 0.716_873, 0.097_076],
            [0.143_066, 0.060_608, 0.714_096],
        ];
        let mut para = b"para\0\0\0\0".to_vec();
        para.extend_from_slice(&3u16.to_be_bytes());
        para.extend_from_slice(&[0, 0]);
        for v in [2.4f32, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
            para.extend_from_slice(&((v * 65536.0).round() as i32).to_be_bytes());
        }
        let jpeg = tagged(
            image::ImageFormat::Jpeg,
            [200, 60, 30],
            &icc::build_for_test(srgb_d50, &para),
        );
        assert_eq!(displayable(Path::new("x.jpg"), &jpeg).unwrap(), jpeg);

        // An Adobe RGB JPEG is not: it is converted.
        let adobe = icc::build_for_test(icc::ADOBE_RGB_D50, &icc::gamma_curve_for_test(2.2));
        let jpeg = tagged(image::ImageFormat::Jpeg, [200, 60, 30], &adobe);
        assert_ne!(displayable(Path::new("x.jpg"), &jpeg).unwrap(), jpeg);
    }

    #[test]
    fn heif_is_catalogued_where_the_system_can_read_it() {
        assert_eq!(is_raster_extension("HEIC"), HEIF_READABLE);
        assert_eq!(is_raster_extension("hif"), HEIF_READABLE);
    }

    #[test]
    fn knows_rendered_formats_from_raws() {
        for p in [
            "a/IMG_0001.JPG",
            "b.jpeg",
            "scan.TIF",
            "c.tiff",
            "d.png",
            "e.webp",
        ] {
            assert!(is_raster(Path::new(p)), "{p}");
        }
        for p in ["DSC0001.ARW", "x.dng", "notes.txt", "noext"] {
            assert!(!is_raster(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn the_transfer_curve_pins_black_white_and_mid_grey() {
        let lut = srgb_lut8();
        assert_eq!(lut[0], 0.0);
        assert!((lut[255] - 1.0).abs() < 1e-6);
        // sRGB 118 is the familiar 18% grey.
        assert!((lut[118] - 0.18).abs() < 0.005, "{}", lut[118]);
    }

    #[test]
    fn srgb_white_is_d65() {
        let white: Vec<f32> = SRGB_TO_XYZ.iter().map(|r| r.iter().sum()).collect();
        assert!((white[0] - 0.9505).abs() < 1e-3);
        assert!((white[1] - 1.0).abs() < 1e-3);
        assert!((white[2] - 1.089).abs() < 1e-3);
    }

    #[test]
    fn a_jpeg_is_shown_as_is_and_a_png_is_re_encoded() {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(8, 4, Rgb([200, 100, 50])));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();
        let shown = displayable(Path::new("x.png"), &png).unwrap();
        assert!(is_jpeg(&shown));
        assert_eq!(dimensions(&shown), Some((8, 4)));

        let same = displayable(Path::new("x.jpg"), &shown).unwrap();
        assert_eq!(same, shown, "a JPEG is served untouched");
        assert_eq!(dimensions(&png), Some((8, 4)));
    }

    /// The scan's preview of a big rendered photo: shrunk before it's
    /// converted, in the same colours as the whole one converted.
    #[test]
    fn a_big_photo_is_shown_within_the_edge_in_the_same_colours() {
        use image::ImageEncoder;
        let adobe = icc::build_for_test(
            icc::ADOBE_RGB_D50,
            &icc::gamma_curve_for_test(563.0 / 256.0),
        );
        let (w, h) = (4000, 2000);
        let img = ImageBuffer::from_pixel(w, h, Rgb([200u8, 60, 30]));
        let mut png = Vec::new();
        let mut e = image::codecs::png::PngEncoder::new(&mut png);
        e.set_icc_profile(adobe.to_vec()).unwrap();
        e.write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .unwrap();

        let whole = displayable(Path::new("x.png"), &png).unwrap();
        let fitted = displayable_within(Path::new("x.png"), &png, Some(1600)).unwrap();
        assert_eq!(dimensions(&whole), Some((w, h)));
        assert_eq!(dimensions(&fitted), Some((1600, 800)));
        let a = image::load_from_memory(&whole).unwrap().to_rgb8();
        let b = image::load_from_memory(&fitted).unwrap().to_rgb8();
        let (pa, pb) = (a.get_pixel(2000, 1000).0, b.get_pixel(800, 400).0);
        for c in 0..3 {
            assert!(pa[c].abs_diff(pb[c]) <= 2, "{pa:?} against {pb:?}");
        }

        // A JPEG that's already sRGB is still handed back as it is.
        let mut jpeg = Vec::new();
        DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        assert_eq!(
            displayable_within(Path::new("x.jpg"), &jpeg, Some(1600)).unwrap(),
            jpeg
        );
    }

    #[test]
    fn linear_rows_cover_a_band() {
        let img = RenderedImage::new(
            Pixels::Eight(ImageBuffer::from_fn(3, 4, |x, y| {
                Rgb([(x * 100) as u8, (y * 60) as u8, 255])
            })),
            1,
            None,
        );
        let lut = srgb_lut8();
        let mut band = Vec::new();
        img.linear_rows(1, 2, &mut band);
        assert_eq!(band.len(), 3 * 2 * 4);
        // Row 1, column 2: (200, 60, 255).
        assert_eq!(&band[8..12], &[lut[200], lut[60], 1.0, 1.0]);
        // Row 2 starts the second half.
        assert_eq!(band[12 + 1], lut[120]);
    }

    /// A tiny TIFF that claims to be 200,000 px square is refused with a
    /// reason, not handed to the allocator — which aborted the app.
    #[test]
    fn a_tiff_claiming_absurd_dimensions_is_refused() {
        // 120 GB worth of claimed pixels.
        let path = Path::new("bomb.tif");
        let err = displayable(path, &claimed_tiff(200_000, 200_000, 3)).expect_err("refused");
        let why = format!("{err:#}");
        assert!(why.contains("bomb.tif"), "{why}");
    }

    /// A greyscale frame within the limit whose RGB copy is not is refused
    /// before either is made.
    #[test]
    fn a_grey_tiff_whose_rgb_copy_would_not_fit_is_refused() {
        let err = displayable(Path::new("grey.tif"), &claimed_tiff(40_000, 40_000, 1))
            .expect_err("refused");
        let why = format!("{err:#}");
        assert!(why.contains("too large to open"), "{why}");
    }

    /// A little-endian TIFF with one IFD: `width` × `height` pixels of
    /// `samples` 8-bit channels, in one strip at offset 8 that holds none of
    /// them.
    fn claimed_tiff(width: u32, height: u32, samples: u32) -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend_from_slice(&8u32.to_le_bytes());
        let photometric = if samples == 1 { 1 } else { 2 };
        let entries: [(u16, u16, u32, u32); 8] = [
            (256, 4, 1, width),       // ImageWidth
            (257, 4, 1, height),      // ImageLength
            (258, 3, 1, 8),           // BitsPerSample
            (259, 3, 1, 1),           // Compression: none
            (262, 3, 1, photometric), // Grey or RGB
            (273, 4, 1, 8),           // StripOffsets
            (277, 3, 1, samples),     // SamplesPerPixel
            (279, 4, 1, 16),          // StripByteCounts
        ];
        t.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            t.extend_from_slice(&tag.to_le_bytes());
            t.extend_from_slice(&kind.to_le_bytes());
            t.extend_from_slice(&count.to_le_bytes());
            t.extend_from_slice(&value.to_le_bytes());
        }
        t.extend_from_slice(&0u32.to_le_bytes());
        t
    }

    #[test]
    fn resize_triangle_matches_the_image_crate() {
        let mut seed = 5u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        };
        for ((w, h), (nw, nh)) in [
            ((1600, 1067), (224, 224)),
            ((1067, 1600), (427, 640)),
            ((640, 427), (640, 427)),
            ((37, 5), (3, 91)),
            ((5, 3), (17, 11)),
            ((1, 1), (3, 3)),
        ] {
            let img = ImageBuffer::from_fn(w, h, |_, _| Rgb([next(), next(), next()]));
            assert_eq!(
                resize_triangle(&img, nw, nh),
                image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle),
                "{w}x{h} -> {nw}x{nh}"
            );
        }
        let empty = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(0, 0);
        assert_eq!(resize_triangle(&empty, 4, 4).dimensions(), (4, 4));
    }
}

/// What tests need to make photos that are not raw, the desktop's as well as
/// this crate's (`--features test-helpers`, which the desktop's tests enable).
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers {
    use image::{ImageBuffer, Rgb};

    /// A JPEG with EXIF but no profile, as a camera writes it.
    pub fn camera_jpeg(rgb: [u8; 3], space: u16, interop: &[u8]) -> Vec<u8> {
        use exif::{experimental::Writer, Field, In, Tag, Value};
        let fields = [
            Field {
                tag: Tag::Make,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![b"Canon".to_vec()]),
            },
            Field {
                tag: Tag::ColorSpace,
                ifd_num: In::PRIMARY,
                value: Value::Short(vec![space]),
            },
            Field {
                tag: Tag::InteroperabilityIndex,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![interop.to_vec()]),
            },
        ];
        let mut w = Writer::new();
        for f in &fields {
            w.push_field(f);
        }
        let mut tiff = std::io::Cursor::new(Vec::new());
        w.write(&mut tiff, false).unwrap();
        let tiff = tiff.into_inner();

        let img = ImageBuffer::from_pixel(32, 32, Rgb(rgb));
        let mut plain = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 100)
            .encode_image(&img)
            .unwrap();
        let mut jpeg = plain[..2].to_vec();
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&plain[2..]);
        jpeg
    }
}
