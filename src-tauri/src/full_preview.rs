//! Full-resolution pixels for the Cull loupe and the Lightbox zoom.
//!
//! Both surfaces show the cached 1600px preview, so "1:1" there is one
//! *preview* pixel. On a 24MP frame that is a quarter of the real resolution:
//! fine for composition, not enough to tell two frames of a burst apart. That
//! is the hardest call a culler makes.
//!
//! The pixels to answer it are already in the file. A raw embeds a camera JPEG
//! at or near full resolution; ingest extracts it and immediately shrinks it to
//! 1600px for the cache. This extracts it again, **on demand, for the one photo
//! being zoomed**. It adds no schema, no disk, and no cost for anyone who never
//! zooms. A handful are kept in memory so paging back and forth through a burst
//! is not a file read each time.
//!
//! **Except where it isn't.** Some bodies embed only a preview-sized JPEG. The
//! Sony a6400 embeds 1616px, the same as the cache. For those, [`render`]
//! decodes the sensor data through the develop pipeline instead: slower, but
//! the only way to real pixels.
//!
//! The JPEG is served as-is rather than decoded and re-encoded. Re-encoding a
//! 24MP frame costs far more than reading it, and adds a generation of loss to
//! the very pixels being inspected for sharpness. Orientation is conveyed the
//! way the webview already honours: an EXIF Orientation tag, which Chromium
//! applies by default (`image-orientation: from-image`).

use anyhow::Result;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

/// How much bigger than the cached preview an embedded JPEG has to be to be
/// worth serving. A 1616px Sony preview is technically larger than 1600px and
/// shows nothing new.
const WORTHWHILE_EDGE: u32 = crate::raw::PREVIEW_EDGE * 5 / 4;

/// Returns the embedded JPEG, oriented, if it is genuinely bigger than the
/// cached preview. `None` means the file has nothing better to offer, and the
/// caller falls back to [`render`].
///
/// **Not every camera embeds a large preview.** Measured on the fixtures:
/// Nikon Z6 6048px, Fuji X-T3 4416, Canon R6 3408, Olympus E-M10 3200 — but
/// Panasonic G9 1920 and Sony a6400 only 1616, the same size as the cache.
pub fn load(path: &Path, orientation: u32) -> Result<Option<Vec<u8>>> {
    let jpeg = crate::scan::preview_of(path)?;
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&jpeg))
        .with_guessed_format()?
        .into_dimensions()?;
    if w.max(h) < WORTHWHILE_EDGE {
        return Ok(None);
    }
    let oriented = with_orientation(&jpeg, orientation);
    Ok(Some(if in_adobe_rgb(path, &jpeg) {
        with_profile(&oriented, &crate::export::icc::adobe_rgb_profile())
    } else {
        oriented
    }))
}

/// Whether a raw's preview is Adobe RGB without an embedded profile saying so
/// — a camera set to Adobe RGB — and needs one attached to show right.
///
/// Served untouched, the webview reads an untagged JPEG as sRGB, and the
/// zoomed frame came out flatter than the same frame unzoomed. Attaching the
/// profile rather than converting keeps the served bytes the camera's, and
/// lets a wide-gamut screen show the colour sRGB cannot hold. A rendered
/// photo's preview is converted to sRGB already (`raster::displayable`).
fn in_adobe_rgb(path: &Path, jpeg: &[u8]) -> bool {
    use crate::exif_read::ColourSpace;
    if crate::raster::is_raster(path) || has_profile(jpeg) {
        return false;
    }
    let space = crate::exif_read::read(jpeg)
        .colour_space
        .or_else(|| crate::scan::metadata_for(path).colour_space);
    space == Some(ColourSpace::AdobeRgb)
}

/// Whether a JPEG carries an ICC profile (an APP2 `ICC_PROFILE` segment).
fn has_profile(jpeg: &[u8]) -> bool {
    app_segments(jpeg)
        .iter()
        .any(|s| s[1] == 0xE2 && s.get(4..).is_some_and(|b| b.starts_with(b"ICC_PROFILE\0")))
}

/// The JPEG's leading APPn segments, whole (marker and length included).
fn app_segments(jpeg: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF && (0xE0..=0xEF).contains(&jpeg[at + 1]) {
        let len = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        if len < 2 || at + 2 + len > jpeg.len() {
            break;
        }
        out.push(&jpeg[at..at + 2 + len]);
        at += 2 + len;
    }
    out
}

/// The JPEG with `icc` attached as its colour profile, after its other APPn
/// segments. Unchanged if the profile would not fit one segment, or the input
/// is not a JPEG.
fn with_profile(jpeg: &[u8], icc: &[u8]) -> Vec<u8> {
    const HEADER: &[u8] = b"ICC_PROFILE\0\x01\x01";
    let len = 2 + HEADER.len() + icc.len();
    if jpeg.len() < 4 || jpeg[..2] != [0xFF, 0xD8] || len > u16::MAX as usize {
        return jpeg.to_vec();
    }
    let at = 2 + app_segments(jpeg).iter().map(|s| s.len()).sum::<usize>();
    let mut out = Vec::with_capacity(jpeg.len() + len + 2);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xE2]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(HEADER);
    out.extend_from_slice(icc);
    out.extend_from_slice(&jpeg[at..]);
    out
}

/// The fallback for a raw whose embedded preview is small: decode the sensor
/// data and render it at full resolution through the develop pipeline, the
/// same call an export makes.
///
/// **At default settings, not the saved edits.** The loupe is judging the
/// capture — is the eye sharp — and the image it swaps out is the camera's
/// uncropped preview. Rendering the saved crop would move the frame under the
/// cursor at the moment of zooming in.
///
/// About a second on a 24MP frame, which is why it is only reached on zoom,
/// only for these cameras, and cached. The result is upright, so no
/// orientation tag is needed.
#[cfg(feature = "libraw")]
pub fn render(
    gpu: &crate::gpu::Gpu,
    pipeline: &crate::gpu::pipeline::Pipeline,
    decoder: &dyn crate::raw::decoder::RawDecoder,
    path: &Path,
) -> Result<Vec<u8>> {
    let stack = &crate::develop::EditStack::default();
    let source = crate::raster::load(path, decoder)?;
    let session = pipeline.prepare_source(gpu, &source)?;
    let (width, height) = session.framed_size(stack);
    let pixels = pipeline.render_to_pixels(gpu, &session, stack, width, height)?;
    crate::export::encode(
        &pixels,
        width,
        height,
        &crate::export::ExportOptions {
            format: crate::export::ExportFormat::Jpeg,
            // High enough that JPEG artefacts are not what a culler ends up
            // judging, low enough to keep a 24MP frame to a few megabytes.
            quality: 90,
            ..Default::default()
        },
        // A zoom preview, not a delivered file: no metadata to carry.
        None,
    )
}

/// A minimal EXIF block: one IFD, one entry, Orientation.
fn orientation_exif(orientation: u16) -> Vec<u8> {
    let mut b = b"Exif\0\0II*\0".to_vec();
    b.extend_from_slice(&8u32.to_le_bytes()); // IFD0 at 8
    b.extend_from_slice(&1u16.to_le_bytes()); // one entry
    b.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
    b.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    b.extend_from_slice(&1u32.to_le_bytes()); // count
    b.extend_from_slice(&orientation.to_le_bytes());
    b.extend_from_slice(&[0, 0]); // pad the value to 4 bytes
    b.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    b
}

/// Replaces whatever EXIF the JPEG carries with one that says `orientation`.
///
/// Replaces rather than adds. An embedded preview occasionally carries its own
/// EXIF, and if its Orientation disagreed with the raw's (which is what ingest
/// used for the cached preview), zooming in would rotate the photo. Returns the
/// input unchanged if it does not parse as a JPEG.
pub fn with_orientation(jpeg: &[u8], orientation: u32) -> Vec<u8> {
    let orientation = if (1..=8).contains(&orientation) {
        orientation as u16
    } else {
        1
    };
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return jpeg.to_vec();
    }

    let exif = orientation_exif(orientation);
    let mut out = Vec::with_capacity(jpeg.len() + exif.len() + 4);
    out.extend_from_slice(&jpeg[..2]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&exif);

    // Copy the APPn segments across, dropping any existing EXIF.
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF && (0xE0..=0xEF).contains(&jpeg[at + 1]) {
        let len = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        if len < 2 || at + 2 + len > jpeg.len() {
            return jpeg.to_vec();
        }
        let segment = &jpeg[at..at + 2 + len];
        let is_exif = jpeg[at + 1] == 0xE1 && segment[4..].starts_with(b"Exif\0\0");
        if !is_exif {
            out.extend_from_slice(segment);
        }
        at += 2 + len;
    }
    out.extend_from_slice(&jpeg[at..]);
    out
}

/// The last few full-resolution JPEGs served, most recent last.
///
/// Small on purpose. Each entry is the camera's full-size JPEG, typically
/// 5–15 MB, and the access pattern is a culler stepping back and forth across
/// a burst of a few frames, not browsing the library.
///
/// Keyed by the photo and its content's hash: ids are reused after
/// photos are removed, and a file can be saved over in place, and either way
/// the old photo's frame was shown for the new one.
pub struct Cache {
    entries: VecDeque<(CacheKey, Arc<Option<Vec<u8>>>)>,
    capacity: usize,
}

/// A photo id and its file's quick hash.
pub type CacheKey = (i32, String);

impl Cache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Also remembers a `None`, so a file with no large preview is not
    /// re-read on every zoom.
    pub fn get(&mut self, key: &CacheKey) -> Option<Arc<Option<Vec<u8>>>> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(i)?;
        let value = Arc::clone(&entry.1);
        self.entries.push_back(entry);
        Some(value)
    }

    pub fn put(&mut self, key: CacheKey, value: Arc<Option<Vec<u8>>>) {
        self.entries.retain(|(k, _)| *k != key);
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([120, 60, 30]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut out)
            .encode_image(&img)
            .unwrap();
        out
    }

    fn orientation_of(bytes: &[u8]) -> Option<u32> {
        let exif = ::exif::Reader::new()
            .read_from_container(&mut std::io::Cursor::new(bytes))
            .ok()?;
        exif.get_field(::exif::Tag::Orientation, ::exif::In::PRIMARY)?
            .value
            .get_uint(0)
    }

    #[test]
    fn the_raws_orientation_is_what_the_webview_sees() {
        for o in 1..=8 {
            let out = with_orientation(&jpeg(40, 20), o);
            assert_eq!(orientation_of(&out), Some(o));
            // Still a JPEG that decodes to the same pixels' dimensions.
            let img = image::load_from_memory(&out).unwrap();
            assert_eq!((img.width(), img.height()), (40, 20));
        }
    }

    #[test]
    fn an_existing_exif_is_replaced_not_stacked() {
        let first = with_orientation(&jpeg(16, 16), 3);
        let second = with_orientation(&first, 6);
        assert_eq!(orientation_of(&second), Some(6));
        let exif_segments = second.windows(6).filter(|w| w == b"Exif\0\0").count();
        assert_eq!(exif_segments, 1);
    }

    #[test]
    fn a_nonsense_orientation_means_upright() {
        assert_eq!(orientation_of(&with_orientation(&jpeg(8, 8), 0)), Some(1));
        assert_eq!(orientation_of(&with_orientation(&jpeg(8, 8), 42)), Some(1));
    }

    #[test]
    fn a_non_jpeg_passes_through() {
        assert_eq!(with_orientation(b"not a jpeg", 6), b"not a jpeg");
        assert_eq!(with_profile(b"not a jpeg", b"icc"), b"not a jpeg");
    }

    /// An attached profile is one any reader finds: the image crate's JPEG
    /// decoder here, WebKit and Chromium in the app. The pixels and the
    /// orientation are untouched.
    #[test]
    fn an_attached_profile_is_read_back() {
        use image::ImageDecoder;
        let icc = crate::export::icc::adobe_rgb_profile();
        let plain = with_orientation(&jpeg(40, 20), 6);
        assert!(!has_profile(&plain));
        let tagged = with_profile(&plain, &icc);
        assert!(has_profile(&tagged));
        let mut decoder =
            image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&tagged)).unwrap();
        assert_eq!(decoder.icc_profile().unwrap(), Some(icc.clone()));
        assert_eq!(orientation_of(&tagged), Some(6));
        let (a, b) = (
            image::load_from_memory(&plain).unwrap().to_rgb8(),
            image::load_from_memory(&tagged).unwrap().to_rgb8(),
        );
        assert_eq!(a, b, "attaching a profile changed the pixels");
        // And it parses as the Adobe RGB it claims to be.
        let parsed = crate::raster::icc::Profile::parse(&icc).unwrap();
        assert!(!parsed.is_srgb());
    }

    #[test]
    fn the_cache_keeps_the_most_recent() {
        let k = |id: i32| (id, format!("h{id}"));
        let mut c = Cache::new(2);
        c.put(k(1), Arc::new(Some(vec![1])));
        c.put(k(2), Arc::new(Some(vec![2])));
        // Touching 1 makes 2 the oldest, so 3 evicts 2.
        assert!(c.get(&k(1)).is_some());
        c.put(k(3), Arc::new(None));
        assert!(c.get(&k(2)).is_none());
        assert!(c.get(&k(1)).is_some());
        assert_eq!(*c.get(&k(3)).unwrap(), None);
        // The same id with other content is another entry.
        assert!(c.get(&(1, "other".into())).is_none());
    }
}
