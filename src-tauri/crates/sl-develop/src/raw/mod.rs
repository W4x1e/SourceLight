//! Raw file handling.
//!
//! Two paths, with very different costs. **Ingest** pulls the camera-rendered
//! JPEG preview out of the file — orders of magnitude cheaper than decoding
//! sensor data, and all the library grid needs. **Develop** decodes the sensor
//! itself via a [`decoder::RawDecoder`], and only when a photo is opened or
//! exported.
//!
//! The code below is the ingest path, and the TIFF reader in it is Sony-specific.
//! It stays as a zero-FFI fast path; broader format coverage comes from the
//! decoder backend.

pub mod colour;
pub mod decoder;
pub mod levels;
pub mod opcodes;

use anyhow::{anyhow, Result};
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::DynamicImage;
use std::path::{Path, PathBuf};

const THUMB_EDGE: u32 = 512;
pub const PREVIEW_EDGE: u32 = 1600;

/// Raw extensions the library will index, lowercase and without the dot.
///
/// One source of truth: the scanner uses this to decide what to walk, and the
/// LibRaw backend reports the same set.
///
/// **Verified against real files: ARW (Sony), DNG (DJI), CR3, NEF, RAF, ORF and
/// RW2.** The open question was never "can LibRaw read a NEF" but whether *our*
/// mapping survives that format's quirks — X-Trans tiles, per-channel black
/// levels, 12-bit white levels — and for those seven it does. The fixtures live
/// in `tests/fixtures/raw-samples.tsv` and are fetched by
/// `scripts/fetch-raw-fixtures.sh`; CI runs `raw_decode` against them.
///
/// The remaining entries — CR2, CRW, NRW, SRF, SR2, PEF, RWL, 3FR, FFF, IIQ,
/// SRW, X3F — are still unproven. Adding one is a line in the manifest. Until
/// then, be careful about advertising them: someone whose files decode to
/// garbage is more likely to give up on the app than to report a bug.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "arw", "srf", "sr2", // Sony
    "cr2", "cr3", "crw", // Canon
    "nef", "nrw", // Nikon
    "raf", // Fujifilm
    "orf", // Olympus / OM System
    "rw2", // Panasonic
    "pef", // Pentax
    "dng", // Adobe / Apple / Leica / Ricoh
    "rwl", // Leica
    "3fr", "fff", // Hasselblad
    "iiq", // Phase One
    "srw", // Samsung
    "x3f", // Sigma
];

pub fn is_supported_extension(ext: &str) -> bool {
    SUPPORTED_EXTENSIONS
        .iter()
        .any(|e| ext.eq_ignore_ascii_case(e))
}

/// Whether the file is a TIFF-based raw container.
///
/// Worth knowing because the IFD walk below only applies to those. ARW, CR2,
/// NEF, DNG, ORF, PEF and RW2 are all TIFF underneath; CR3 (ISO BMFF), RAF and
/// X3F are not, and must go to the decoder backend instead. Checking the magic
/// rather than letting `extract_preview` fall through matters: its brute-force
/// JPEG scan will happily return some small embedded thumbnail from a container
/// it does not understand, which looks like success and yields a poor preview.
pub fn is_tiff_container(data: &[u8]) -> bool {
    matches!(data.get(..2), Some(b"II") | Some(b"MM"))
}

/// Minimal TIFF reader. ARW is a TIFF container, so the previews live in the
/// IFD chain under the standard JPEGInterchangeFormat tags — or, in a DNG, as
/// the single strip of a reduced-resolution IFD.
struct Tiff<'a> {
    d: &'a [u8],
    le: bool,
}

const TAG_NEW_SUBFILE_TYPE: u16 = 0x00FE;
const TAG_COMPRESSION: u16 = 0x0103;
const TAG_PHOTOMETRIC: u16 = 0x0106;
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x0117;
const TAG_SUB_IFDS: u16 = 0x014A;
const TAG_JPEG_OFFSET: u16 = 0x0201;
const TAG_JPEG_LENGTH: u16 = 0x0202;
const TAG_EXIF_IFD: u16 = 0x8769;

/// A DNG preview, by the IFD that holds it: reduced resolution (subfile type
/// 1), JPEG-compressed (6 old-style, 7 new), and a picture — RGB or YCbCr —
/// rather than sensor data. The raw itself can be JPEG-compressed too, as
/// lossless tiles that start as any JPEG does; its photometric (colour filter
/// array or linear raw) is what tells them apart.
fn is_dng_preview(
    subfile: Option<u32>,
    compression: Option<u32>,
    photometric: Option<u32>,
) -> bool {
    subfile == Some(1) && matches!(compression, Some(6 | 7)) && matches!(photometric, Some(2 | 6))
}

impl<'a> Tiff<'a> {
    fn new(d: &'a [u8]) -> Option<(Self, usize)> {
        if d.len() < 8 {
            return None;
        }
        let le = match &d[0..2] {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let t = Tiff { d, le };
        if t.u16(2)? != 42 {
            return None;
        }
        let ifd0 = t.u32(4)? as usize;
        Some((t, ifd0))
    }

    fn u16(&self, o: usize) -> Option<u16> {
        let b = self.d.get(o..o + 2)?;
        Some(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32(&self, o: usize) -> Option<u32> {
        let b = self.d.get(o..o + 4)?;
        Some(if self.le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    /// The value of the single-valued entry at `e`, SHORT or LONG. A SHORT
    /// sits in the first two bytes of the value field, which is the low half
    /// of a little-endian LONG but the high half of a big-endian one.
    fn scalar(&self, e: usize) -> Option<u32> {
        match self.u16(e + 2)? {
            3 => self.u16(e + 8).map(u32::from),
            4 => self.u32(e + 8),
            _ => None,
        }
    }

    /// Whether `n` bytes at `o` are inside a file of `file_len` bytes but past
    /// the ones in hand — bytes the walk needs that a head alone lacks.
    fn past_head(&self, o: usize, n: usize, file_len: usize) -> bool {
        o.saturating_add(n) > self.d.len() && o < file_len
    }

    /// Collects every (offset, length) JPEG preview pair reachable from `start`,
    /// following the IFD chain plus SubIFDs and the Exif IFD, in a file of
    /// `file_len` bytes of which `d` may be only the start.
    ///
    /// `None` when the walk reaches an IFD past what is in hand: it cannot say
    /// which previews there are, and the whole file is needed after all. With
    /// the whole file in hand, it is always `Some`.
    fn collect_previews(&self, start: usize, file_len: usize) -> Option<Vec<(usize, usize)>> {
        let mut found = Vec::new();
        let mut queue = vec![start];
        let mut visited = Vec::new();
        // Bounded so a malformed/looping file can never hang the scan.
        const MAX_IFDS: usize = 64;
        let mut budget = MAX_IFDS;

        while let Some(ifd) = queue.pop() {
            if budget == 0 || ifd == 0 || visited.contains(&ifd) {
                continue;
            }
            budget -= 1;
            visited.push(ifd);

            if self.past_head(ifd, 2, file_len) {
                return None;
            }
            let Some(count) = self.u16(ifd) else { continue };
            // The entries and the pointer to the next IFD.
            if self.past_head(ifd, 2 + count as usize * 12 + 4, file_len) {
                return None;
            }
            let mut jpeg_off: Option<usize> = None;
            let mut jpeg_len: Option<usize> = None;
            let (mut subfile, mut compression, mut photometric) = (None, None, None);
            let (mut strip_off, mut strip_len) = (None, None);

            for i in 0..count as usize {
                let e = ifd + 2 + i * 12;
                let (Some(tag), Some(cnt), Some(val)) =
                    (self.u16(e), self.u32(e + 4), self.u32(e + 8))
                else {
                    continue;
                };

                match tag {
                    TAG_JPEG_OFFSET => jpeg_off = Some(val as usize),
                    TAG_JPEG_LENGTH => jpeg_len = Some(val as usize),
                    TAG_NEW_SUBFILE_TYPE => subfile = self.scalar(e),
                    TAG_COMPRESSION => compression = self.scalar(e),
                    TAG_PHOTOMETRIC => photometric = self.scalar(e),
                    // One strip: a preview in several would need stitching,
                    // and no DNG writer does that.
                    TAG_STRIP_OFFSETS if cnt == 1 => strip_off = self.scalar(e),
                    TAG_STRIP_BYTE_COUNTS if cnt == 1 => strip_len = self.scalar(e),
                    TAG_EXIF_IFD => queue.push(val as usize),
                    TAG_SUB_IFDS => {
                        if cnt == 1 {
                            queue.push(val as usize);
                        } else {
                            // More than one SubIFD: `val` points at an array of offsets.
                            if self.past_head(val as usize, cnt as usize * 4, file_len) {
                                return None;
                            }
                            // No more than can be visited: a count from the
                            // file can be four billion.
                            for k in 0..(cnt as usize).min(MAX_IFDS) {
                                if let Some(o) = self.u32(val as usize + k * 4) {
                                    queue.push(o as usize);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            if let (Some(o), Some(l)) = (jpeg_off, jpeg_len) {
                if l > 0 && o.saturating_add(l) <= file_len {
                    found.push((o, l));
                }
            }
            if is_dng_preview(subfile, compression, photometric) {
                if let (Some(o), Some(l)) = (strip_off, strip_len) {
                    let (o, l) = (o as usize, l as usize);
                    if l > 0 && o.saturating_add(l) <= file_len {
                        found.push((o, l));
                    }
                }
            }

            // Follow the chain to the next IFD.
            if let Some(next) = self.u32(ifd + 2 + count as usize * 12) {
                queue.push(next as usize);
            }
        }

        // Largest preview = closest to full resolution.
        found.sort_by_key(|(_, l)| std::cmp::Reverse(*l));
        Some(found)
    }
}

/// Where a TIFF-based raw's embedded JPEG previews are, largest first, found
/// from the start of the file alone: `head` is its first bytes, `file_len` its
/// whole length.
///
/// The IFDs that say where the previews are sit near the start; the previews
/// may not, so a scan reads just the one it wants rather than the whole raw —
/// the preview of a 48 MB NEF ends 4.6 MB in, an ARW's 0.4 MB in. `None` when
/// the head does not hold the answer: no TIFF IFD chain (RW2 and ORF have
/// their own magic), or one that leads past the head. Then [`extract_preview`]
/// on the whole file is the way.
pub fn preview_ranges(head: &[u8], file_len: usize) -> Option<Vec<(usize, usize)>> {
    let (tiff, ifd0) = Tiff::new(head)?;
    tiff.collect_previews(ifd0, file_len)
}

/// Whether `b` starts as a JPEG does: start-of-image, then another marker.
pub fn looks_like_jpeg(b: &[u8]) -> bool {
    b.len() > 4 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF
}

/// Last-resort scan for the largest embedded JPEG, used if the IFD walk finds
/// nothing (e.g. an unusual ARW variant).
fn scan_for_largest_jpeg(d: &[u8]) -> Option<&[u8]> {
    let mut best: Option<&[u8]> = None;
    let mut i = 0usize;
    while i + 3 < d.len() {
        if d[i] == 0xFF && d[i + 1] == 0xD8 && d[i + 2] == 0xFF {
            // Find EOI after this SOI.
            let mut j = i + 2;
            while j + 1 < d.len() {
                if d[j] == 0xFF && d[j + 1] == 0xD9 {
                    let slice = &d[i..j + 2];
                    if best.is_none_or(|b| slice.len() > b.len()) {
                        best = Some(slice);
                    }
                    break;
                }
                j += 1;
            }
            i = j.max(i + 2);
        } else {
            i += 1;
        }
    }
    best
}

/// Pulls the largest embedded JPEG preview out of an ARW.
pub fn extract_preview(data: &[u8]) -> Result<&[u8]> {
    if let Some(candidates) = preview_ranges(data, data.len()) {
        for (o, l) in candidates {
            let slice = &data[o..o + l];
            if looks_like_jpeg(slice) {
                return Ok(slice);
            }
        }
    }
    scan_for_largest_jpeg(data).ok_or_else(|| anyhow!("no embedded JPEG preview found"))
}

/// Applies the EXIF orientation so the image is displayed upright. A value
/// outside 1..=8 leaves it as stored.
fn apply_orientation(mut img: DynamicImage, orientation: u32) -> DynamicImage {
    if let Some(o) = u8::try_from(orientation)
        .ok()
        .and_then(Orientation::from_exif)
    {
        img.apply_orientation(o);
    }
    img
}

pub struct Derivatives {
    pub thumb_path: PathBuf,
    pub preview_path: PathBuf,
    /// The embedded preview's size, upright.
    pub width: u32,
    pub height: u32,
    /// The grid thumbnail, upright — kept so focus and brightness can be
    /// scored without decoding the preview a second time.
    pub thumbnail: DynamicImage,
}

/// `img` fitted to `edge` on its long side, as previews are made.
///
/// A big frame shrinks by an integer area average, not `resize`, which
/// filtered it through a float RGBA copy: 100 MB at 24 MP on every import
/// thread. From twice `edge` up the two agree to about a
/// level, and the average took a Z6's 6048 px frame to 1600 in 95 ms against
/// 199. Below that the average comes out crisper than the filter, so a frame
/// that's only a little bigger keeps the filter.
pub fn shrink(img: &DynamicImage, edge: u32) -> DynamicImage {
    if img.width().max(img.height()) >= 2 * edge {
        img.thumbnail(edge, edge)
    } else {
        img.resize(edge, edge, FilterType::Triangle)
    }
}

/// Writes a grid thumbnail and a lightbox-sized preview into the cache dir.
/// `key` must be stable and filesystem-safe (we use the photo's content hash).
///
/// `profile` is the preview's colour space when it is not sRGB — a camera set
/// to Adobe RGB encodes its raw's preview that way — and the derivatives are
/// converted to sRGB from it, since the webview takes an untagged image as
/// sRGB. See `raster::preview_profile`.
pub fn write_derivatives(
    jpeg: &[u8],
    orientation: u32,
    cache_dir: &Path,
    key: &str,
    profile: Option<&crate::raster::icc::Profile>,
) -> Result<Derivatives> {
    // Up to the app's own limit, not the image crate's 512 MiB.
    let img = crate::raster::load_from_memory(jpeg)?;
    let (width, height) = match orientation {
        // A quarter turn swaps the sides.
        5..=8 => (img.height(), img.width()),
        _ => (img.width(), img.height()),
    };

    // One pass over the camera's image, and it is turned upright only once it
    // is small. Turning first copied the whole of it — a full-size JPEG from
    // a Nikon, Fujifilm or Canon is 12 to 24 MP — and the thumbnail was then
    // shrunk from that full image a second time. The shrink does not care
    // which way up the pixels are.
    //
    let preview = shrink(&img, PREVIEW_EDGE);
    // The camera's image is done with: freed before the encodes below.
    drop(img);
    // Converted once it is small: the colour conversion is per pixel.
    let preview = match profile {
        Some(p) => DynamicImage::ImageRgb8(crate::raster::to_srgb(&preview, p)),
        None => preview,
    };
    let preview = apply_orientation(preview, orientation);
    let thumbnail = preview.resize(THUMB_EDGE, THUMB_EDGE, FilterType::Triangle);

    std::fs::create_dir_all(cache_dir)?;
    let thumb_path = cache_dir.join(format!("{key}_t.jpg"));
    let preview_path = cache_dir.join(format!("{key}_p.jpg"));
    if !thumb_path.exists() {
        thumbnail.to_rgb8().save(&thumb_path)?;
    }
    if !preview_path.exists() {
        preview.to_rgb8().save(&preview_path)?;
    }

    Ok(Derivatives {
        thumb_path,
        preview_path,
        width,
        height,
        thumbnail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian TIFF whose IFD0, at `ifd`, points at a JPEG of `len`
    /// bytes at `at`.
    fn tiff(ifd: usize, at: usize, len: usize) -> Vec<u8> {
        let mut d = vec![0u8; at + len];
        d[..4].copy_from_slice(b"II*\0");
        d[4..8].copy_from_slice(&(ifd as u32).to_le_bytes());
        d[ifd..ifd + 2].copy_from_slice(&2u16.to_le_bytes());
        for (i, (tag, val)) in [(TAG_JPEG_OFFSET, at), (TAG_JPEG_LENGTH, len)]
            .into_iter()
            .enumerate()
        {
            let e = ifd + 2 + i * 12;
            d[e..e + 2].copy_from_slice(&tag.to_le_bytes());
            d[e + 2..e + 4].copy_from_slice(&4u16.to_le_bytes());
            d[e + 4..e + 8].copy_from_slice(&1u32.to_le_bytes());
            d[e + 8..e + 12].copy_from_slice(&(val as u32).to_le_bytes());
        }
        d[at..at + 4].copy_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0]);
        d
    }

    /// The IFDs are in the head and the preview is past it: the head says
    /// where, so only that range need be read.
    #[test]
    fn the_head_says_where_a_preview_past_it_is() {
        let d = tiff(8, 4096, 64);
        assert_eq!(preview_ranges(&d[..512], d.len()), Some(vec![(4096, 64)]));
        // The same answer the whole file gives.
        assert_eq!(extract_preview(&d).unwrap(), &d[4096..4096 + 64]);
    }

    /// An IFD past the head, or one that runs over its end, cannot be read
    /// from it: the answer is "read the whole file", never a guess.
    #[test]
    fn an_ifd_past_the_head_asks_for_the_whole_file() {
        let d = tiff(1000, 4096, 64);
        assert_eq!(preview_ranges(&d[..512], d.len()), None);
        assert_eq!(preview_ranges(&d[..1010], d.len()), None, "entries cut off");
        assert!(preview_ranges(&d, d.len()).is_some());
    }

    /// A preview said to run past the end of the file is not one.
    #[test]
    fn a_preview_past_the_end_of_the_file_is_ignored() {
        let d = tiff(8, 4096, 64);
        assert_eq!(preview_ranges(&d[..512], 4096 + 32), Some(vec![]));
    }

    /// A big-endian TIFF with one IFD at 8 of `(tag, type, value)` entries,
    /// each with a count of 1, and a JPEG start at every offset in `jpegs`.
    fn big_endian(entries: &[(u16, u16, u32)], jpegs: &[usize], len: usize) -> Vec<u8> {
        let mut d = vec![0u8; len];
        d[..4].copy_from_slice(b"MM\0*");
        d[4..8].copy_from_slice(&8u32.to_be_bytes());
        d[8..10].copy_from_slice(&(entries.len() as u16).to_be_bytes());
        for (i, &(tag, kind, val)) in entries.iter().enumerate() {
            let e = 10 + i * 12;
            d[e..e + 2].copy_from_slice(&tag.to_be_bytes());
            d[e + 2..e + 4].copy_from_slice(&kind.to_be_bytes());
            d[e + 4..e + 8].copy_from_slice(&1u32.to_be_bytes());
            match kind {
                3 => d[e + 8..e + 10].copy_from_slice(&(val as u16).to_be_bytes()),
                _ => d[e + 8..e + 12].copy_from_slice(&val.to_be_bytes()),
            }
        }
        for &at in jpegs {
            d[at..at + 4].copy_from_slice(&[0xFF, 0xD8, 0xFF, 0xDB]);
        }
        d
    }

    /// A DNG keeps its preview as the one strip of a reduced-resolution IFD,
    /// with no JPEG-thumbnail tags at all: a Galaxy S23's, whose only JPEG
    /// otherwise is its lossless raw data, came out as that.
    #[test]
    fn a_dng_preview_is_found_by_its_ifd() {
        let preview = |subfile: u32, photometric: u32| {
            big_endian(
                &[
                    (TAG_NEW_SUBFILE_TYPE, 4, subfile),
                    (TAG_COMPRESSION, 3, 7),
                    (TAG_PHOTOMETRIC, 3, photometric),
                    (TAG_STRIP_OFFSETS, 4, 1024),
                    (TAG_STRIP_BYTE_COUNTS, 4, 512),
                ],
                &[1024],
                2048,
            )
        };
        let d = preview(1, 6);
        assert_eq!(preview_ranges(&d, d.len()), Some(vec![(1024, 512)]));
        assert_eq!(extract_preview(&d).unwrap(), &d[1024..1536]);
        // The same shape holding the sensor data — full resolution, or a
        // colour filter array (32803) or linear raw (34892) — is not one.
        for (subfile, photometric) in [(0, 6), (1, 32803), (1, 34892)] {
            let d = preview(subfile, photometric);
            assert_eq!(preview_ranges(&d, d.len()), Some(vec![]));
        }
    }

    /// A SubIFD array the file says has four billion entries, pointing past
    /// its end, is read no further than the walk could visit: it used to be
    /// counted through entry by entry, hanging the scan.
    #[test]
    fn a_huge_subifd_count_does_not_hang_the_walk() {
        let mut d = big_endian(&[(TAG_SUB_IFDS, 4, 1 << 20)], &[], 64);
        // The one entry's count.
        d[14..18].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(preview_ranges(&d, d.len()), Some(vec![]));
    }

    /// Each EXIF orientation turns the stored pixels the way the
    /// specification says, and a value outside 1..=8 leaves them as stored.
    #[test]
    fn orientation_turns_the_stored_pixels_upright() {
        let (w, h) = (3u32, 2u32);
        // A different value at every position, so a wrong turn shows.
        let stored = image::GrayImage::from_fn(w, h, |x, y| image::Luma([(y * w + x) as u8]));
        // Where the stored pixel at (x, y) is shown.
        let shown_at = |o: u32, x: u32, y: u32| match o {
            2 => (w - 1 - x, y),
            3 => (w - 1 - x, h - 1 - y),
            4 => (x, h - 1 - y),
            5 => (y, x),
            6 => (h - 1 - y, x),
            7 => (h - 1 - y, w - 1 - x),
            8 => (y, w - 1 - x),
            _ => (x, y),
        };
        // 262 is 6 in its low byte, and must not be read as 6.
        for o in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 262] {
            let shown = apply_orientation(DynamicImage::ImageLuma8(stored.clone()), o).to_luma8();
            let size = if (5..=8).contains(&o) { (h, w) } else { (w, h) };
            assert_eq!(shown.dimensions(), size, "orientation {o}");
            for (x, y, p) in stored.enumerate_pixels() {
                let (sx, sy) = shown_at(o, x, y);
                assert_eq!(
                    shown.get_pixel(sx, sy),
                    p,
                    "orientation {o}, stored ({x}, {y})"
                );
            }
        }
    }

    /// Both shrinks: a frame twice the preview's size or more takes
    /// the area average, a smaller one the filter, and either way the files
    /// come out at the preview's and thumbnail's sizes, turned upright.
    #[test]
    fn derivatives_come_out_upright_at_their_sizes_from_any_frame() {
        let dir = crate::test_support::Scratch::new("raw-derivatives");
        for (w, h) in [(3400, 2000), (2000, 1200)] {
            let img = image::RgbImage::from_fn(w, h, |x, y| {
                image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
            });
            let mut jpeg = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(
                    &mut std::io::Cursor::new(&mut jpeg),
                    image::ImageFormat::Jpeg,
                )
                .unwrap();
            // Orientation 6: a quarter turn, so the sides swap.
            let d = write_derivatives(&jpeg, 6, &dir, &format!("k{w}"), None).unwrap();
            assert_eq!((d.width, d.height), (h, w));
            let preview = image::open(&d.preview_path).unwrap();
            let long = PREVIEW_EDGE;
            let short = (long as f64 * h as f64 / w as f64).round() as u32;
            assert_eq!(
                (preview.width(), preview.height()),
                (short, long),
                "{w}x{h}"
            );
            let thumb = image::open(&d.thumb_path).unwrap();
            assert_eq!(thumb.height(), THUMB_EDGE, "{w}x{h}");
            assert!(thumb.width() < THUMB_EDGE);
        }
    }
}
