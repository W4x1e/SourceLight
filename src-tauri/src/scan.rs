//! Library scanning: walk the root for raw files, extract metadata and
//! derivatives, and upsert into the index. Originals are never modified or moved.

use anyhow::Result;
use rayon::prelude::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::{db, exif_read, raw};

/// Files are read and decoded in bounded batches so peak memory stays flat
/// regardless of library size — a 5k-photo scan would otherwise try to hold
/// thousands of ~25MB raws at once.
const BATCH: usize = 24;

pub struct IngestedPhoto {
    pub path: String,
    pub filename: String,
    pub file_hash: String,
    pub capture_date: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i64>,
    pub aperture: Option<f64>,
    pub shutter: Option<String>,
    pub focal_length: Option<f64>,
    pub width: i64,
    pub height: i64,
    pub orientation: i64,
    pub thumb_path: Option<String>,
    pub preview_path: Option<String>,
    /// Focus/sharpness score of the embedded preview (see `quality.rs`). Computed
    /// here because the preview is already decoded for the derivatives.
    pub sharpness: Option<f32>,
    /// Mean brightness of the same preview, 0 black to 1 white.
    pub brightness: Option<f32>,
    /// Where it was taken, from EXIF or LibRaw.
    pub gps: Option<(f64, f64)>,
    /// The file's size and modified time as the scan saw them, so the next
    /// scan can pass it over on a `stat` (see [`db::FileStamp`]).
    pub stamp: Option<db::FileStamp>,
}

/// Every photo under `root`: raws, and rendered files other than the
/// JPEG half of a RAW+JPEG pair.
pub fn find_photo_files(root: &Path) -> Vec<PathBuf> {
    let files = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_photo_file(e.path()))
        .map(|e| e.into_path())
        .collect();
    without_raw_partners(files)
}

/// Whether `path` names a photo the app reads: a raw, or a rendered file.
pub fn is_photo_file(path: &Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| raw::is_supported_extension(x) || crate::raster::is_raster_extension(x))
}

/// Drops each rendered file that has a raw of the same name beside it.
///
/// A camera shooting RAW+JPEG writes both, and they are one photograph: the
/// JPEG travels with the raw as its companion (`shell::companions`) and was
/// never a photo of its own. Cataloguing it would put every such frame in the
/// grid twice.
pub(crate) fn without_raw_partners(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let key = |p: &Path| {
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_lowercase());
        (p.parent().map(Path::to_path_buf), stem)
    };
    let raws: std::collections::HashSet<_> = files
        .iter()
        .filter(|p| !crate::raster::is_raster(p))
        .map(|p| key(p))
        .collect();
    files
        .into_iter()
        .filter(|p| !crate::raster::is_raster(p) || !raws.contains(&key(p)))
        .collect()
}

const HEAD_BYTES: usize = 65536;

/// Cheap content-change signal: size + mtime + a prefix of the file. Hashing
/// 5k raws in full would cost gigabytes of I/O for no practical benefit.
///
/// Deliberately readable from just the file head so a re-scan can decide
/// "already indexed" after a 64KB read instead of decoding a 25MB raw.
///
/// The hash is the catalogue's lasting name for a file — "already imported",
/// relinking a moved folder, derivative names — so it is pinned: it was
/// std's `DefaultHasher`, whose algorithm std promises nothing about across
/// releases, and a toolchain that changed it would have silently lost every
/// match. [`stable_hash`] is the same SipHash-1-3, keys zero, over the same
/// bytes `DefaultHasher` was fed, so every stored value still matches.
pub(crate) fn quick_hash(path: &Path) -> Result<String> {
    Ok(quick_hashes(path, &[0])?.remove(0))
}

/// [`quick_hash`] as it would be with the file's modified time moved by each
/// of `shifts` seconds — the head read once for all of them.
///
/// For a card: FAT keeps local time with no zone, and Windows turns it
/// into UTC with the PC's *current* zone and daylight saving. After a clock
/// change, or a laptop's zone change, the same card file reports a time an
/// hour or more off, and an imported file would look new.
pub(crate) fn quick_hashes(path: &Path, shifts: &[i64]) -> Result<Vec<String>> {
    use std::io::Read;

    let md = std::fs::metadata(path)?;
    let modified = md
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    // The whole head, however many reads it takes: one `read` may return
    // less (a network or cloud volume), and a shorter head is another hash.
    let mut head = Vec::with_capacity(HEAD_BYTES);
    std::fs::File::open(path)?
        .take(HEAD_BYTES as u64)
        .read_to_end(&mut head)?;
    let n = head.len();

    let mut out = Vec::with_capacity(shifts.len());
    for &shift in shifts {
        let mut bytes: Vec<u8> = Vec::with_capacity(HEAD_BYTES + 24);
        bytes.extend_from_slice(&md.len().to_le_bytes());
        if let Some(secs) = modified {
            let secs = secs.saturating_add_signed(shift);
            bytes.extend_from_slice(&secs.to_le_bytes());
        }
        // A slice hashed as std hashes one: its length, then its bytes.
        bytes.extend_from_slice(&(n as u64).to_le_bytes());
        bytes.extend_from_slice(&head[..n]);
        out.push(format!("{:016x}", stable_hash(&bytes)));
    }
    Ok(out)
}

/// SipHash-1-3 with zero keys: what std's `DefaultHasher::new()` computes
/// today, written out so it can never change under us.
pub(crate) fn stable_hash(msg: &[u8]) -> u64 {
    let (mut v0, mut v1, mut v2, mut v3): (u64, u64, u64, u64) = (
        0x736f_6d65_7073_6575,
        0x646f_7261_6e64_6f6d,
        0x6c79_6765_6e65_7261,
        0x7465_6462_7974_6573,
    );
    let round = |v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64| {
        *v0 = v0.wrapping_add(*v1);
        *v1 = v1.rotate_left(13);
        *v1 ^= *v0;
        *v0 = v0.rotate_left(32);
        *v2 = v2.wrapping_add(*v3);
        *v3 = v3.rotate_left(16);
        *v3 ^= *v2;
        *v0 = v0.wrapping_add(*v3);
        *v3 = v3.rotate_left(21);
        *v3 ^= *v0;
        *v2 = v2.wrapping_add(*v1);
        *v1 = v1.rotate_left(17);
        *v1 ^= *v2;
        *v2 = v2.rotate_left(32);
    };
    let mut chunks = msg.chunks_exact(8);
    for chunk in &mut chunks {
        let m = u64::from_le_bytes(chunk.try_into().expect("eight bytes"));
        v3 ^= m;
        round(&mut v0, &mut v1, &mut v2, &mut v3);
        v0 ^= m;
    }
    let mut last = (msg.len() as u64 & 0xff) << 56;
    for (i, &b) in chunks.remainder().iter().enumerate() {
        last |= u64::from(b) << (8 * i);
    }
    v3 ^= last;
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    v0 ^= last;
    v2 ^= 0xff;
    for _ in 0..3 {
        round(&mut v0, &mut v1, &mut v2, &mut v3);
    }
    v0 ^ v1 ^ v2 ^ v3
}

/// How much of a raw is read for its metadata and to find its preview. A
/// TIFF-based raw's IFDs and EXIF sit in its first few kilobytes; one whose
/// IFDs reach further is read whole (see [`raw::preview_ranges`]).
const RAW_HEAD: u64 = 1 << 20;

/// What a scan reads of a photo file.
struct Source {
    /// The whole file, or for a raw only its first [`RAW_HEAD`] bytes when
    /// they are all the metadata readers can use.
    data: Vec<u8>,
    /// The camera's embedded JPEG, read on its own where the head says it is.
    preview: Option<Vec<u8>>,
}

/// Reads what ingest needs of `path`, and no more.
///
/// A raw used to be read whole — 25 MB for an ARW, 48 for a NEF, 56 for a RAF
/// — for the few megabytes at most a scan uses: the metadata at the start and
/// the embedded preview, which ends 0.4 MB into an ARW and 4.6 MB into a NEF.
/// From a card or an external drive, reading the rest was most of a scan.
fn read_source(path: &Path) -> Result<Source> {
    use std::io::{Read, Seek, SeekFrom};

    let whole = |path: &Path| -> Result<Source> {
        Ok(Source {
            data: crate::raster::read_file(path)?,
            preview: None,
        })
    };
    if crate::raster::is_raster(path) {
        return whole(path);
    }
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = Vec::new();
    (&mut file).take(RAW_HEAD).read_to_end(&mut head)?;
    if head.len() as u64 >= len || !raw::is_tiff_container(&head) {
        // Either the whole file already, or CR3, RAF, X3F: the decoder reads
        // those by path, and the EXIF reader only understands TIFF.
        return Ok(Source {
            data: head,
            preview: None,
        });
    }
    // RW2 and ORF have magic of their own, and a raw whose IFDs reach past
    // the head cannot say from it where its previews are: both need the
    // whole file, as before.
    let Some(ranges) = raw::preview_ranges(&head, len as usize) else {
        return whole(path);
    };
    for (offset, length) in ranges {
        let bytes = match head.get(offset..offset + length) {
            Some(inside) => inside.to_vec(),
            None => {
                let mut outside = vec![0; length];
                file.seek(SeekFrom::Start(offset as u64))?;
                file.read_exact(&mut outside)?;
                outside
            }
        };
        if raw::looks_like_jpeg(&bytes) {
            return Ok(Source {
                data: head,
                preview: Some(bytes),
            });
        }
    }
    // No IFD led to a JPEG: the last-resort search needs the whole file.
    whole(path)
}

/// The camera's embedded preview of `path` (or the photo itself, for a
/// rendered one), reading no more of the file than it takes.
pub fn preview_of(path: &Path) -> Result<Vec<u8>> {
    let source = read_source(path)?;
    match source.preview {
        Some(preview) => Ok(preview),
        None => embedded_preview(path, &source.data),
    }
}

/// Whether the previews a scan made of `path` before the camera's colour
/// space was read (migration 28) came out in the wrong colours: a camera
/// JPEG or raw preview in Adobe RGB, which says so only in its EXIF or maker
/// notes. `Some(space)` when they need remaking, with the space to remake
/// them from.
///
/// Reads the start of the file, where a TIFF-based raw or a JPEG keeps its
/// EXIF and a RAF or RW2 its preview's, and asks LibRaw only when that says
/// nothing (CR3, ORF) — so checking a library costs a fraction of scanning it.
pub fn stale_colours(path: &Path) -> Result<Option<exif_read::ColourSpace>> {
    use exif_read::ColourSpace;
    use std::io::Read;
    // As much as reading locations takes: a TIFF-based raw's EXIF and maker
    // notes sit well inside it, and a partial read is still read.
    const HEAD: u64 = 512 * 1024;
    let mut head = Vec::new();
    std::fs::File::open(path)?
        .take(HEAD)
        .read_to_end(&mut head)?;
    let adobe = |space: Option<ColourSpace>| {
        (space == Some(ColourSpace::AdobeRgb)).then_some(ColourSpace::AdobeRgb)
    };
    // A profile, if the file or its preview has one, was honoured all along.
    // A phone's Display P3 photo — and the preview in its DNG — says
    // "uncalibrated" in its EXIF, as a camera set to Adobe RGB does; the
    // profile is what tells them apart. Cameras embed none.
    if head.windows(12).any(|w| w == b"ICC_PROFILE\0") {
        return Ok(None);
    }
    if crate::raster::is_raster(path) {
        return Ok(adobe(exif_read::read_colour_space_head(&head)));
    }
    let space = exif_read::read_colour_space_head(&head)
        .or_else(|| exif_read::embedded_colour_space(&head));
    #[cfg(feature = "libraw")]
    let space = space.or_else(|| {
        raw::libraw::LibRawDecoder::new()
            .metadata(path)
            .ok()
            .and_then(|m| m.colour_space)
    });
    Ok(adobe(space))
}

/// Makes `path`'s grid thumbnail and preview again, named from `key`, in the
/// colour space `space` the camera recorded.
pub fn remake_previews(
    path: &Path,
    orientation: u32,
    cache_dir: &Path,
    key: &str,
    space: Option<exif_read::ColourSpace>,
) -> Result<raw::Derivatives> {
    let preview = preview_of(path)?;
    // A rendered photo's preview comes out of `raster::displayable` in sRGB.
    let profile = (!crate::raster::is_raster(path))
        .then(|| crate::raster::preview_profile(&preview, space))
        .flatten();
    raw::write_derivatives(&preview, orientation, cache_dir, key, profile.as_ref())
}

/// A file's size and modified time in whole seconds — the same two
/// [`quick_hash`] folds in, read without opening the file. `None` when there
/// is no usable time to compare.
fn file_stamp(path: &Path) -> Option<db::FileStamp> {
    let md = std::fs::metadata(path).ok()?;
    let secs = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((i64::try_from(md.len()).ok()?, i64::try_from(secs).ok()?))
}

/// Whether the file at `path`, catalogued as `known` (its hash and stamp) and
/// now stamped `now`, changed only its modified time: the same size, and the
/// hash it has with the time it had then is the one the catalogue holds. A
/// FAT drive's times move with the computer's time zone and daylight saving,
/// and a copy or a sync may not keep them; the quick hash folds the
/// time in, so either looked like the file edited in place.
fn only_time_moved(
    path: &Path,
    known: Option<&(String, Option<db::FileStamp>)>,
    now: db::FileStamp,
) -> bool {
    let Some((hash, Some((size, then)))) = known else {
        return false;
    };
    *size == now.0
        && *then != now.1
        && quick_hashes(path, &[then - now.1]).is_ok_and(|h| h.first() == Some(hash))
}

/// The camera-rendered preview, by whichever route the container allows.
///
/// TIFF-based raws (ARW, CR2, NEF, DNG, ORF, PEF, RW2) go through the in-process
/// IFD walk: no FFI, and the file is already in memory. The rest — CR3's ISO
/// BMFF, Fuji's RAF, Sigma's X3F — need the decoder backend, which is why the
/// fast path is gated on the container magic rather than simply tried first.
pub fn embedded_preview(path: &Path, data: &[u8]) -> Result<Vec<u8>> {
    // A rendered photo is its own preview. Checked first: a TIFF scan has the
    // same magic as a raw, and no camera JPEG inside it to find.
    if crate::raster::is_raster(path) {
        return crate::raster::displayable(path, data);
    }
    if raw::is_tiff_container(data) {
        if let Ok(preview) = raw::extract_preview(data) {
            return Ok(preview.to_vec());
        }
    }

    #[cfg(feature = "libraw")]
    {
        use raw::decoder::RawDecoder;
        raw::libraw::LibRawDecoder::new().embedded_preview(path)
    }

    #[cfg(not(feature = "libraw"))]
    anyhow::bail!(
        "no embedded preview in {} (raw decoding is not compiled in)",
        path.display()
    )
}

/// Fills the gaps a TIFF EXIF reader leaves.
///
/// `exif_read` is built on a TIFF reader and says so; ARW and NEF are TIFF
/// containers and come back complete. CR3 is ISO-BMFF, and RAF, ORF and RW2 are
/// not plain TIFF either, so for those it returns an empty struct — and this
/// used to be the only source, which meant a Canon, Fujifilm, Olympus or
/// Panasonic photo imported with no capture date at all. No date means invisible
/// to every date filter and missing from trip clustering, so the Places
/// workspace was simply empty for most of the market.
///
/// Field by field rather than all-or-nothing: EXIF wins wherever it produced
/// something, because it is the file's own account of itself and LibRaw's
/// normalisation occasionally rounds. LibRaw only fills what is still missing.
///
/// Dimensions are the exception, and deliberately. EXIF's width/height come
/// from whichever IFD the reader landed on, which for a NEF is the **embedded
/// thumbnail** — 160x120 rather than the sensor's 6048x4024. A wrong number
/// that looks plausible is worse than a missing one, so an implausibly small
/// EXIF size is discarded in favour of LibRaw's.
/// The metadata ingest would record for `path`, without ingesting it.
///
/// Exists so the format tests can assert on what a photo *would* import as
/// rather than on whichever reader happened to answer — what matters is that
/// the photo ends up findable, not who supplied the date.
pub fn metadata_for(path: &Path) -> exif_read::ExifData {
    match read_source(path) {
        Ok(source) => read_metadata(path, &source.data),
        Err(_) => exif_read::ExifData::default(),
    }
}

/// The metadata of a file already in memory, by whichever route its format
/// needs.
fn read_metadata(path: &Path, data: &[u8]) -> exif_read::ExifData {
    let meta = exif_read::read(data);
    if !crate::raster::is_raster(path) {
        return enrich_metadata(path, meta);
    }
    // A rendered file's EXIF, when it has any, is complete — LibRaw has
    // nothing to add. Its size is the header's: that is the image itself,
    // where an editor's EXIF can still describe the frame before a crop, and a
    // PNG or a scan often has no EXIF at all.
    let mut meta = meta;
    let (size, orientation) = crate::raster::probe(path, data);
    if let Some((w, h)) = size {
        meta.width = Some(i64::from(w));
        meta.height = Some(i64::from(h));
    }
    if let Some(o) = orientation {
        meta.orientation = o;
    }
    meta
}

fn enrich_metadata(path: &Path, meta: exif_read::ExifData) -> exif_read::ExifData {
    #[cfg(not(feature = "libraw"))]
    {
        let _ = path;
        meta
    }
    #[cfg(feature = "libraw")]
    {
        // Nothing to add, and opening the file again is not free.
        let dimensions_look_wrong = meta.width.is_some_and(|w| w < MIN_PLAUSIBLE_EDGE);
        // A missing position alone does not justify reopening the file: most
        // photos simply have none, and the EXIF would have shown it. A missing
        // colour space does: a Pentax records Adobe RGB only in its maker
        // note, which LibRaw reads and the EXIF reader does not, and its
        // previews come out flat without it.
        if meta.capture_date.is_some()
            && meta.camera.is_some()
            && meta.iso.is_some()
            && meta.colour_space.is_some()
            && !dimensions_look_wrong
        {
            return meta;
        }

        let Ok(extra) = crate::raw::libraw::LibRawDecoder::new().metadata(path) else {
            // A file LibRaw cannot open is not a new problem here — the preview
            // extraction below will fail on it and report properly.
            return meta;
        };

        let mut meta = meta;
        meta.capture_date = meta.capture_date.or(extra.capture_date);
        meta.camera = meta.camera.or(extra.camera);
        meta.lens = meta.lens.or(extra.lens);
        meta.iso = meta.iso.or(extra.iso);
        meta.aperture = meta.aperture.or(extra.aperture);
        meta.shutter = meta.shutter.or(extra.shutter);
        meta.focal_length = meta.focal_length.or(extra.focal_length);
        meta.gps = meta.gps.or(extra.gps);
        meta.colour_space = meta.colour_space.or(extra.colour_space);
        if dimensions_look_wrong || meta.width.is_none() {
            meta.width = extra.width.or(meta.width);
            meta.height = extra.height.or(meta.height);
        }
        meta
    }
}

/// Below this, an EXIF "image size" is a thumbnail rather than a photograph.
///
/// No camera worth cataloguing produces a 500px frame, and the embedded
/// thumbnails that get mistaken for one are 160px.
#[cfg(feature = "libraw")]
const MIN_PLAUSIBLE_EDGE: i64 = 512;

/// [`ingest_one`], with a panic in it caught and returned as that file's
/// error: one malformed file used to take the rest of its folder's
/// scan down with it.
fn ingest_guarded(
    path: &Path,
    cache_dir: &Path,
    hash: String,
    stamp: Option<db::FileStamp>,
) -> Result<IngestedPhoto> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ingest_one(path, cache_dir, hash, stamp)
    }))
    .unwrap_or_else(|panic| {
        let why = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".into());
        Err(anyhow::anyhow!("reading it crashed: {why}"))
    })
}

fn ingest_one(
    path: &Path,
    cache_dir: &Path,
    hash: String,
    stamp: Option<db::FileStamp>,
) -> Result<IngestedPhoto> {
    let Source { data, preview } = read_source(path)?;
    let meta = read_metadata(path, &data);
    let preview = match preview {
        Some(preview) => preview,
        // A rendered photo's preview is made at the size it's kept at, not
        // converted and encoded whole only to be shrunk.
        None if crate::raster::is_raster(path) => {
            crate::raster::displayable_within(path, &data, Some(raw::PREVIEW_EDGE))?
        }
        None => embedded_preview(path, &data)?,
    };
    // The file is done with: for a raster it's all of it, hundreds of MB for
    // a 16-bit TIFF, held through the derivatives for nothing.
    drop(data);
    // A rendered photo's preview is already sRGB (`raster::displayable`); a
    // raw's is in whatever the camera was set to.
    let profile = (!crate::raster::is_raster(path))
        .then(|| crate::raster::preview_profile(&preview, meta.colour_space))
        .flatten();
    let derived = raw::write_derivatives(
        &preview,
        meta.orientation,
        cache_dir,
        &hash,
        profile.as_ref(),
    )?;
    // Focus and brightness from the thumbnail just made — one classical-CV
    // pass, no model, and no second decode of the preview. The thumbnail is
    // the working size the scores are measured at anyway, and it is what the
    // backfill job scores a photo from too (the cached preview, shrunk).
    let scores = Some(crate::quality::scores(&derived.thumbnail));

    Ok(IngestedPhoto {
        path: path.to_string_lossy().to_string(),
        filename: path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default(),
        file_hash: hash,
        capture_date: meta.capture_date,
        camera: meta.camera,
        lens: meta.lens,
        iso: meta.iso,
        aperture: meta.aperture,
        shutter: meta.shutter,
        focal_length: meta.focal_length,
        // Prefer the real capture size; fall back to the preview's if EXIF is silent.
        width: meta.width.unwrap_or(derived.width as i64),
        height: meta.height.unwrap_or(derived.height as i64),
        orientation: meta.orientation as i64,
        thumb_path: Some(derived.thumb_path.to_string_lossy().to_string()),
        preview_path: Some(derived.preview_path.to_string_lossy().to_string()),
        sharpness: scores.map(|s| s.sharpness),
        brightness: scores.map(|s| s.brightness),
        gps: meta.gps,
        stamp,
    })
}

pub struct ScanCounts {
    pub added: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Files found at a new path and reunited with their old row — a folder
    /// renamed or moved within the library.
    pub relinked: usize,
    /// Photos under the root whose file the scan did not find.
    pub missing: usize,
    /// Why the first photo that could not be read failed, named. Worth
    /// showing: failures tend to share a cause (HEIC without Windows' codec),
    /// and a bare count gives no way to fix it.
    pub first_failure: Option<String>,
}

/// Scans `root`, calling `on_progress(done, total, label)` as it goes.
pub fn scan_library<F>(
    conn: &mut Connection,
    root: &Path,
    cache_dir: &Path,
    mut on_progress: F,
) -> Result<ScanCounts>
where
    F: FnMut(usize, usize, &str),
{
    let files = find_photo_files(root);
    let total = files.len();
    let seen: std::collections::HashSet<String> = files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let mut counts = ScanCounts {
        added: 0,
        skipped: 0,
        failed: 0,
        first_failure: None,
        relinked: 0,
        missing: 0,
    };

    on_progress(0, total, "Reading files");

    // What the catalogue already has, by path, read once rather than asked of
    // it photo by photo.
    let known = db::known_files(conn)?;
    // Files earlier scans couldn't read.
    let attempts = db::scan_attempts(conn)?;

    let mut done = 0usize;
    for chunk in files.chunks(BATCH) {
        // Cheap pass first: drop anything already indexed, so a re-scan never
        // pays for a full read and JPEG decode — and, when its size and time
        // are as the last scan saw them, never opens it at all. Every launch
        // rescans every folder; opening and hashing the head of each photo
        // was a seek a photo on a spinning drive.
        let mut todo: Vec<(PathBuf, String, Option<db::FileStamp>)> = Vec::new();
        // Files that were being read, with others, when the app went down
        // once: each is read again on its own, so a second crash can only
        // be the one file that caused it.
        let mut alone: Vec<(PathBuf, String, Option<db::FileStamp>)> = Vec::new();
        let mut stamps: Vec<(String, db::FileStamp)> = Vec::new();
        // Known files whose modified time moved and nothing else.
        let mut restamped: Vec<(String, String, db::FileStamp)> = Vec::new();
        for path in chunk {
            done += 1;
            let path_str = path.to_string_lossy();
            let stamp = file_stamp(path);
            let row = known.get(path_str.as_ref());
            if stamp.is_some() && row.is_some_and(|(_, seen)| *seen == stamp) {
                counts.skipped += 1;
                continue;
            }
            // A file that failed before, unchanged since, fails again: skipped,
            // not retried on every launch. Named once in the log each scan.
            let tried = attempts
                .get(path_str.as_ref())
                .filter(|a| Some(a.stamp) == stamp);
            if let Some(a) = tried.filter(|a| a.reason.is_some() || a.attempts >= 2) {
                let why = a
                    .reason
                    .as_deref()
                    .unwrap_or("the app stopped while reading it");
                eprintln!(
                    "skipping {}: it couldn't be read before ({why})",
                    path.display()
                );
                counts.skipped += 1;
                continue;
            }
            let suspect = tried.is_some();
            match quick_hash(path) {
                Ok(hash) => {
                    // Known, unless its previews went and a restore asked
                    // for them again (`db::REDERIVE`).
                    let rederive = row.is_some_and(|(_, seen)| {
                        seen.is_some_and(|(size, _)| size == db::REDERIVE)
                    });
                    if !rederive && row.is_some_and(|(known_hash, _)| *known_hash == hash) {
                        counts.skipped += 1;
                        // Known by its hash — a photo scanned before stamps
                        // were kept. Stamped now, so the next scan needs no
                        // more than a `stat`.
                        if let Some(stamp) = stamp {
                            stamps.push((path_str.to_string(), stamp));
                        }
                    } else if let Some(stamp) =
                        stamp.filter(|&s| !rederive && only_time_moved(path, row, s))
                    {
                        // The same content, its time moved: not an edit, so
                        // nothing worked out from it is forgotten. The
                        // hash and stamp follow the file.
                        counts.skipped += 1;
                        restamped.push((path_str.to_string(), hash, stamp));
                    } else if let Some(id) =
                        db::relink_candidate(conn, &path_str, &hash).unwrap_or(None)
                    {
                        // The same file at a new path: bring its row — and all
                        // the work on it — along, instead of indexing it as a
                        // stranger. No decode needed; the derivatives
                        // are keyed by the hash, which has not changed.
                        let name = path
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        match db::relink(conn, id, &path_str, &name) {
                            Ok(()) => {
                                counts.relinked += 1;
                                if let Some(stamp) = stamp {
                                    stamps.push((path_str.to_string(), stamp));
                                }
                            }
                            Err(e) => {
                                eprintln!("relink failed {}: {e}", path.display());
                                counts.failed += 1;
                            }
                        }
                    } else if suspect {
                        alone.push((path.clone(), hash, stamp));
                    } else {
                        todo.push((path.clone(), hash, stamp));
                    }
                }
                Err(e) => {
                    eprintln!("hash failed {}: {e}", path.display());
                    counts.failed += 1;
                }
            }
        }

        // A stamp that fails to store costs the next scan a hash, nothing more.
        if let Err(e) = db::stamp_files(conn, &stamps) {
            eprintln!("could not record file stamps: {e:#}");
        }
        // Likewise: the next scan finds the time moved again, nothing more.
        if let Err(e) = db::restamp_files(conn, &restamped) {
            eprintln!("could not record moved file times: {e:#}");
        }

        // Decode the new ones in parallel, each read counted as begun first,
        // and a panic in one costing that file, not the scan...
        let begin =
            |files: &[(PathBuf, String, Option<db::FileStamp>)]| -> Vec<(String, db::FileStamp)> {
                files
                    .iter()
                    .filter_map(|(p, _, s)| s.map(|s| (p.to_string_lossy().to_string(), s)))
                    .collect()
            };
        if let Err(e) = db::begin_scan_attempts(conn, &begin(&todo)) {
            eprintln!("could not record the files being read: {e:#}");
        }
        let mut results: Vec<(PathBuf, Result<IngestedPhoto>)> = todo
            .into_par_iter()
            .map(|(p, hash, stamp)| {
                let out = ingest_guarded(&p, cache_dir, hash, stamp);
                (p, out)
            })
            .collect();
        for file in alone {
            if let Err(e) = db::begin_scan_attempts(conn, &begin(std::slice::from_ref(&file))) {
                eprintln!("could not record the file being read: {e:#}");
            }
            let (p, hash, stamp) = file;
            let out = ingest_guarded(&p, cache_dir, hash, stamp);
            results.push((p, out));
        }

        let ended: Vec<(String, Option<String>)> = results
            .iter()
            .map(|(p, res)| {
                (
                    p.to_string_lossy().to_string(),
                    res.as_ref().err().map(|e| format!("{e:#}")),
                )
            })
            .collect();
        if let Err(e) = db::end_scan_attempts(conn, &ended) {
            eprintln!("could not record how reading went: {e:#}");
        }

        // ...but write serially: one SQLite connection, one writer.
        for (path, res) in results {
            match res {
                Ok(photo) => match db::upsert_photo(conn, &photo) {
                    Ok(_) => counts.added += 1,
                    Err(e) => {
                        eprintln!("index failed {}: {e}", photo.path);
                        counts.failed += 1;
                    }
                },
                Err(e) => {
                    eprintln!("ingest failed {}: {e}", path.display());
                    counts.failed += 1;
                    if counts.first_failure.is_none() {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        counts.first_failure = Some(format!("{name}: {e:#}"));
                    }
                }
            }
        }

        on_progress(done, total, "Indexing");
    }

    // Last, so a relinked row is already at its new path and is not counted.
    counts.missing = db::mark_missing(conn, root, &seen)?;

    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn scratch(name: &str) -> crate::test_support::Scratch {
        crate::test_support::Scratch::new(&format!("scan-{name}"))
    }

    /// A stand-in raw. Nothing here decodes it: relinking and missing-marking
    /// both happen on the quick hash, before any decode.
    fn fake_raw(path: &Path) -> String {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            b"stand-in raw bytes; the quick hash reads only the head",
        )
        .unwrap();
        quick_hash(path).unwrap()
    }

    fn path_of(conn: &Connection, id: i32) -> String {
        conn.query_row("SELECT path FROM photos WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn missing(conn: &Connection, id: i32) -> bool {
        conn.query_row(
            "SELECT missing FROM photos WHERE id = ?1",
            params![id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            != 0
    }

    /// A RAW+JPEG pair is one photo, as it always was; a JPEG on its own, a
    /// PNG or a TIFF is a photo now.
    #[test]
    fn a_jpeg_beside_its_raw_is_a_companion_and_one_alone_is_a_photo() {
        let files = [
            "a/DSC0001.ARW",
            "a/DSC0001.JPG",
            "a/DSC0002.jpg",
            "b/DSC0001.JPG",
            "b/scan.tif",
            "a/notes.txt",
        ]
        .map(PathBuf::from)
        .to_vec();
        let mut kept: Vec<String> = without_raw_partners(files)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        kept.sort();
        assert_eq!(
            kept,
            [
                "a/DSC0001.ARW",
                "a/DSC0002.jpg",
                "a/notes.txt",
                "b/DSC0001.JPG",
                "b/scan.tif"
            ],
            "only the JPEG beside a raw of its own name goes"
        );
    }

    /// Real files end to end: a JPEG with EXIF, a PNG with none, and a 16-bit
    /// TIFF, which has a raw's magic and must not be mistaken for one.
    #[test]
    fn rendered_photos_scan_into_the_catalogue() {
        use image::{ImageBuffer, Rgb};
        let root = scratch("rendered");
        let photos = root.join("phone");
        std::fs::create_dir_all(&photos).unwrap();
        let eight = ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x * 4) as u8, (y * 5) as u8, 90u8]));
        eight.save(photos.join("IMG_0001.jpg")).unwrap();
        eight.save(photos.join("screenshot.png")).unwrap();
        let sixteen: ImageBuffer<Rgb<u16>, Vec<u16>> =
            ImageBuffer::from_fn(40, 30, |x, _| Rgb([(x * 1600) as u16, 30_000, 5_000]));
        sixteen.save(photos.join("scan.tif")).unwrap();

        let mut conn = db::open_memory().unwrap();
        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        assert_eq!((counts.added, counts.failed), (3, 0));

        let size = |name: &str| -> (i64, i64) {
            conn.query_row(
                "SELECT width, height FROM photos WHERE filename = ?1",
                params![name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(size("IMG_0001.jpg"), (64, 48));
        assert_eq!(size("screenshot.png"), (64, 48));
        assert_eq!(size("scan.tif"), (40, 30));
        let previews: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM photos WHERE thumb_path IS NOT NULL AND preview_path IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(previews, 3);
    }

    /// A camera's Adobe RGB JPEG catalogued before its colour space was read
    /// has flat previews; the backfill finds it from the start of the file and
    /// remakes them in its own colours. An sRGB one is left alone.
    #[test]
    fn previews_made_in_the_wrong_colours_are_found_and_remade() {
        let root = scratch("colours");
        let rgb = [200u8, 60, 30];
        let adobe = root.join("_MG_0001.JPG");
        let srgb = root.join("IMG_0002.JPG");
        std::fs::write(
            &adobe,
            crate::raster::test_helpers::camera_jpeg(rgb, 0xFFFF, b"R03"),
        )
        .unwrap();
        std::fs::write(
            &srgb,
            crate::raster::test_helpers::camera_jpeg(rgb, 1, b"R98"),
        )
        .unwrap();

        assert_eq!(
            stale_colours(&adobe).unwrap(),
            Some(exif_read::ColourSpace::AdobeRgb)
        );
        assert_eq!(stale_colours(&srgb).unwrap(), None);

        let cache = root.join("cache");
        let made =
            remake_previews(&adobe, 1, &cache, "k-c", stale_colours(&adobe).unwrap()).unwrap();
        let thumb = image::open(&made.thumb_path).unwrap().to_rgb8();
        let got = thumb.get_pixel(thumb.width() / 2, thumb.height() / 2).0;
        let want = crate::raster::adobe_to_srgb_by_the_book(rgb);
        for c in 0..3 {
            assert!(
                (got[c] as f32 - want[c]).abs() <= 4.0,
                "remade thumbnail shows {got:?}, Adobe RGB {rgb:?} is {want:?}"
            );
        }
    }

    /// An iPhone's HEIC scans in upright, through the system's own
    /// codec: WIC on Windows, ImageIO on a Mac.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_heic_scans_in_upright() {
        use image::{ImageBuffer, Rgb};
        let root = scratch("heic");
        let photos = root.join("iphone");
        std::fs::create_dir_all(&photos).unwrap();
        let img = ImageBuffer::from_pixel(64, 32, Rgb([120u8, 140, 160]));
        // Rotated a quarter in the container, as a portrait shot is.
        let heic = photos.join("IMG_0001.HEIC");
        #[cfg(windows)]
        let written = crate::raster::wic::encode_for_test(&img, 1, None, &heic).unwrap();
        #[cfg(target_os = "macos")]
        let written = crate::raster::imageio::encode_for_test(&img, 6, None, &heic).unwrap();
        if !written {
            eprintln!("skipping: no HEIC encoder on this machine");
            return;
        }
        std::fs::write(photos.join("IMG_0002.HEIC"), b"truncated").unwrap();

        let mut conn = db::open_memory().unwrap();
        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        assert_eq!((counts.added, counts.failed), (1, 1));
        let reason = counts.first_failure.unwrap();
        assert!(reason.starts_with("IMG_0002.HEIC: "), "{reason}");
        // The codec is installed here: a damaged file must not be blamed on it.
        assert!(!reason.contains("Microsoft Store"), "{reason}");

        let (w, h, o): (i64, i64, i64) = conn
            .query_row(
                "SELECT width, height, orientation FROM photos WHERE filename = 'IMG_0001.HEIC'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((w, h, o), (32, 64, 1));
    }

    /// The commonest thing anyone does to a photo folder: rename it in
    /// Explorer. The work on every photo in it must come along.
    #[test]
    fn a_renamed_folder_keeps_every_photos_work() {
        let root = scratch("renamed");
        let before = root.join("Karoo").join("DSC00001.ARW");
        let hash = fake_raw(&before);

        let mut conn = db::open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, rating, flag)
             VALUES (1, ?1, 'DSC00001.ARW', ?2, '2026-01-01', 5, 1)",
            params![before.to_string_lossy(), hash],
        )
        .unwrap();
        let copy = db::create_variant(&mut conn, 1, Some("Mono")).unwrap();

        std::fs::rename(root.join("Karoo"), root.join("Karoo trip")).unwrap();
        let after = root.join("Karoo trip").join("DSC00001.ARW");

        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        assert_eq!(counts.relinked, 1);
        assert_eq!((counts.added, counts.failed, counts.missing), (0, 0, 0));

        // The same row, now at the new path, with its rating and flag…
        assert_eq!(path_of(&conn, 1), after.to_string_lossy());
        let (rating, flag): (i64, i64) = conn
            .query_row("SELECT rating, flag FROM photos WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((rating, flag), (5, 1));
        // …and its virtual copy, which shares the file, followed it.
        assert_eq!(path_of(&conn, copy), after.to_string_lossy());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "no new rows for the moved file");
    }

    /// The pinned hash is the one every catalogue already holds: the
    /// values below are what std's `DefaultHasher` gave for these inputs when
    /// the hash was pinned, so a stored `file_hash` still matches.
    #[test]
    fn the_quick_hash_is_pinned() {
        // Taken from std's DefaultHasher on 2026-10-01, which this matched
        // byte for byte, including over the values quick_hash feeds it.
        let long: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        assert_eq!(stable_hash(b""), 0xd1fb_a762_150c_532c);
        assert_eq!(stable_hash(b"northlight"), 0x98c6_b0a3_d529_eea9);
        assert_eq!(stable_hash(&long[..300]), 0x4a3e_e92c_f03a_1ab4);
        assert_eq!(stable_hash(&long), 0x498f_889a_c054_d2d8);
    }

    /// A photo catalogued before stamps were kept is known by its hash on the
    /// next scan and stamped; after that, a scan knows it by its size and time
    /// alone — it does not even open it, so an unreadable file still counts
    /// as known rather than failing.
    #[cfg(unix)]
    #[test]
    fn an_unchanged_file_is_known_without_being_opened() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("stamped");
        let file = root.join("DSC00004.ARW");
        let hash = fake_raw(&file);
        let mut conn = db::open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, ?1, 'DSC00004.ARW', ?2, '2026-01-01')",
            params![file.to_string_lossy(), hash],
        )
        .unwrap();

        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        assert_eq!((counts.skipped, counts.failed), (1, 0));
        let stamp: (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT file_size, file_mtime FROM photos WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            stamp,
            file_stamp(&file).map(|(s, t)| (Some(s), Some(t))).unwrap()
        );

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {});
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let counts = counts.unwrap();
        assert_eq!(
            (counts.skipped, counts.failed),
            (1, 0),
            "an unchanged file was opened"
        );
    }

    /// A file rewritten in place — the same size, a new time — is not taken
    /// for the one the catalogue knows: it is read, and found to be new.
    #[test]
    fn a_file_rewritten_in_place_is_read_again() {
        let root = scratch("rewritten");
        let file = root.join("DSC00005.ARW");
        let hash = fake_raw(&file);
        let mut conn = db::open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, ?1, 'DSC00005.ARW', ?2, '2026-01-01')",
            params![file.to_string_lossy(), hash],
        )
        .unwrap();
        scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();

        let size = std::fs::metadata(&file).unwrap().len() as usize;
        std::fs::write(&file, vec![b'x'; size]).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();

        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        // Not skipped: read and sent to be indexed, where a stand-in raw
        // cannot be decoded.
        assert_eq!((counts.skipped, counts.failed), (0, 1));
    }

    /// A file whose time moved and nothing else — a FAT drive after a change
    /// of daylight saving, a copy that didn't keep times — is not an
    /// edit: it keeps everything worked out from it, and its hash and stamp
    /// follow it.
    #[test]
    fn a_file_whose_time_moved_keeps_its_work() {
        let root = scratch("time-moved");
        let file = root.join("DSC00006.ARW");
        let hash = fake_raw(&file);
        let mut conn = db::open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, caption)
             VALUES (1, ?1, 'DSC00006.ARW', ?2, '2026-01-01', 'a fox by a wall')",
            params![file.to_string_lossy(), hash],
        )
        .unwrap();
        scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();

        let an_hour_on = std::fs::metadata(&file).unwrap().modified().unwrap()
            + std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(an_hour_on)
            .unwrap();

        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        // Skipped, not read: a stand-in raw sent to be indexed fails.
        assert_eq!((counts.skipped, counts.failed), (1, 0));
        let (caption, stored): (String, String) = conn
            .query_row(
                "SELECT caption, file_hash FROM photos WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(caption, "a fox by a wall");
        assert_eq!(stored, quick_hash(&file).unwrap());
        // And the next scan knows it by its stamp again.
        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        assert_eq!((counts.skipped, counts.failed), (1, 0));
    }

    /// A file that is not where it was — its drive unplugged, say — is marked
    /// missing, never deleted, and unmarked when it is found again.
    #[test]
    fn a_missing_file_is_marked_not_deleted_and_comes_back() {
        let root = scratch("missing");
        let file = root.join("DSC00002.ARW");
        let hash = fake_raw(&file);

        let mut conn = db::open_memory().unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, rating)
             VALUES (1, ?1, 'DSC00002.ARW', ?2, '2026-01-01', 4)",
            params![file.to_string_lossy(), hash],
        )
        .unwrap();

        // Moved out of the library entirely — not a rename the scan can see.
        let elsewhere = scratch("missing-elsewhere");
        let away = elsewhere.join("DSC00002.ARW");
        std::fs::rename(&file, &away).unwrap();
        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        assert_eq!(counts.missing, 1);
        assert!(missing(&conn, 1));

        // Back again (the drive reconnected): found, unmarked, and its rating
        // was never at risk.
        std::fs::rename(&away, &file).unwrap();
        let counts = scan_library(&mut conn, &root, &root.join("cache"), |_, _, _| {}).unwrap();
        assert_eq!((counts.missing, counts.skipped), (0, 1));
        assert!(!missing(&conn, 1));
        let rating: i64 = conn
            .query_row("SELECT rating FROM photos WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rating, 4);
    }

    /// Two identical files both gone and one reappearing is ambiguous — the
    /// same card imported twice — and guessing would swap their work. It is
    /// left for the user rather than relinked to either.
    #[test]
    fn an_ambiguous_match_is_not_relinked() {
        let root = scratch("ambiguous");
        let a = root.join("a").join("DSC00003.ARW");
        let b = root.join("b").join("DSC00003.ARW");
        let hash = fake_raw(&a);
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();

        let conn = db::open_memory().unwrap();
        for (id, p) in [(1, &a), (2, &b)] {
            conn.execute(
                "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (?1, ?2, 'DSC00003.ARW', ?3, '2026-01-01')",
                params![id, p.to_string_lossy(), hash],
            )
            .unwrap();
        }
        let elsewhere = root.join("c").join("DSC00003.ARW");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        std::fs::rename(&a, &elsewhere).unwrap();

        assert_eq!(
            db::relink_candidate(&conn, &elsewhere.to_string_lossy(), &hash).unwrap(),
            None
        );
    }

    /// A raw read in part — its head, and its preview on its own — must come
    /// to exactly what reading the whole file did: the same preview, byte for
    /// byte, and the same metadata, for every camera in the fixtures.
    #[test]
    fn a_raw_read_in_part_matches_one_read_whole() {
        use crate::test_support::{files_in, raw_dir, skip, Needs};
        let Some(dir) = raw_dir() else {
            skip(Needs::Raw, "set SOURCELIGHT_TEST_RAW to a folder of raws");
            return;
        };
        let files = files_in(&dir, raw::is_supported_extension);
        assert!(!files.is_empty(), "no raws in {}", dir.display());
        let mut in_part = 0;
        for path in files {
            let whole = std::fs::read(&path).unwrap();
            let source = read_source(&path).unwrap();
            let name = path.display();
            assert_eq!(
                format!("{:?}", read_metadata(&path, &source.data)),
                format!("{:?}", read_metadata(&path, &whole)),
                "{name}: metadata"
            );
            let preview = match &source.preview {
                Some(p) => p.clone(),
                None => embedded_preview(&path, &source.data).unwrap(),
            };
            assert!(
                preview == embedded_preview(&path, &whole).unwrap(),
                "{name}: preview"
            );
            if source.data.len() < whole.len() {
                in_part += 1;
            }
        }
        // Not a vacuous pass: the TIFF-based raws, at least, were read in part.
        assert!(in_part > 0, "every raw was read whole");
    }

    /// A file that fails is remembered and skipped while it is unchanged,
    /// so a malformed file doesn't fail every launch; a changed one
    /// is tried again.
    #[test]
    fn a_file_that_fails_is_skipped_until_it_changes() {
        let root = scratch("fails-once");
        let bad = root.join("broken.jpg");
        std::fs::write(&bad, b"\xFF\xD8\xFF not really a jpeg").unwrap();
        let cache = scratch("fails-once-cache");
        let mut conn = db::open_memory().unwrap();

        let first = scan_library(&mut conn, &root, &cache, |_, _, _| {}).unwrap();
        assert_eq!((first.added, first.failed), (0, 1));
        let tried = db::scan_attempts(&conn).unwrap();
        let attempt = &tried[bad.to_string_lossy().as_ref()];
        assert!(attempt.reason.is_some(), "{attempt:?}");

        let second = scan_library(&mut conn, &root, &cache, |_, _, _| {}).unwrap();
        assert_eq!((second.failed, second.skipped), (0, 1), "not tried again");

        // Fixed: a real JPEG in its place is read, and forgotten as a failure.
        image::RgbImage::from_pixel(8, 8, image::Rgb([90, 90, 90]))
            .save(&bad)
            .unwrap();
        let third = scan_library(&mut conn, &root, &cache, |_, _, _| {}).unwrap();
        assert_eq!(third.added, 1);
        assert!(db::scan_attempts(&conn).unwrap().is_empty());
    }

    /// A file that was being read when the app went down once is read again
    /// on its own; twice, and it is skipped.
    #[test]
    fn a_file_read_when_the_app_went_down_is_isolated_then_skipped() {
        let root = scratch("went-down");
        let file = root.join("IMG_0001.jpg");
        image::RgbImage::from_pixel(8, 8, image::Rgb([90, 90, 90]))
            .save(&file)
            .unwrap();
        let cache = scratch("went-down-cache");
        let mut conn = db::open_memory().unwrap();
        let stamp = file_stamp(&file).unwrap();
        let key = file.to_string_lossy().to_string();

        // Begun and never ended: what a crash mid-read leaves.
        db::begin_scan_attempts(&mut conn, &[(key.clone(), stamp)]).unwrap();
        let next = scan_library(&mut conn, &root, &cache, |_, _, _| {}).unwrap();
        assert_eq!(next.added, 1, "tried again, alone, and fine");

        let mut conn = db::open_memory().unwrap();
        db::begin_scan_attempts(&mut conn, &[(key.clone(), stamp)]).unwrap();
        db::begin_scan_attempts(&mut conn, &[(key, stamp)]).unwrap();
        let next = scan_library(&mut conn, &root, &cache, |_, _, _| {}).unwrap();
        assert_eq!((next.added, next.skipped), (0, 1), "down twice: skipped");
    }

    /// However the page spells a library folder — as the picker gave it,
    /// with a separator, or as stored — a scan walks the stored form, so the
    /// second scan finds nothing new and nothing missing, and every photo is
    /// filed.
    ///
    /// Runs against a real drive root when `SOURCELIGHT_TEST_DRIVE_ROOT`
    /// names one, such as a `subst X: <scratch folder>` drive, which is the
    /// case the bug was in; otherwise through an ordinary folder.
    #[test]
    fn a_library_folder_scans_the_same_however_it_is_spelled() {
        use image::{ImageBuffer, Rgb};
        let scratch_root = scratch("spelled");
        let picked = std::env::var("SOURCELIGHT_TEST_DRIVE_ROOT")
            .unwrap_or_else(|_| format!("{}{}", scratch_root.display(), std::path::MAIN_SEPARATOR));
        let base = PathBuf::from(&picked);
        let photos = base.join("Photos");
        std::fs::create_dir_all(&photos).unwrap();
        let img = ImageBuffer::from_fn(32, 24, |x, y| Rgb([(x * 8) as u8, (y * 9) as u8, 70u8]));
        img.save(photos.join("IMG_0001.jpg")).unwrap();
        img.save(base.join("IMG_0002.jpg")).unwrap();
        let cache = scratch("spelled-cache");

        let mut conn = db::open_memory().unwrap();
        let trimmed = picked.trim_end_matches(['\\', '/']).to_string();
        let mut added = Vec::new();
        for spelling in [picked.clone(), trimmed.clone(), picked.clone()] {
            let (id, stored) = db::folder_for_scan(&mut conn, &spelling).unwrap();
            let counts = scan_library(&mut conn, Path::new(&stored), &cache, |_, _, _| {}).unwrap();
            db::assign_folder(&conn, id, &stored).unwrap();
            assert_eq!(counts.missing, 0, "scanning {spelling:?} as {stored:?}");
            added.push(counts.added);
        }
        assert_eq!(added, vec![2, 0, 0]);
        let (rows, unfiled): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(folder_id IS NULL), 0) FROM photos",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((rows, unfiled), (2, 0));
        if std::env::var("SOURCELIGHT_TEST_DRIVE_ROOT").is_ok() {
            let _ = std::fs::remove_dir_all(&photos);
            let _ = std::fs::remove_file(base.join("IMG_0002.jpg"));
        }
    }
}
