//! LibRaw decode backend.
//!
//! **We call `unpack()` and never `dcraw_process()`.** LibRaw is used purely as
//! a container parser and decompressor: it hands us the sensor values and the
//! metadata needed to interpret them, and every subsequent decision — demosaic,
//! white balance, colour transform, tone — belongs to our own WGSL pipeline.
//!
//! That boundary is not just architectural tidiness. LibRaw's better demosaic
//! algorithms (AMaZE, DCB and friends) ship as separately **GPL**-licensed
//! packs; never entering the processing path keeps them out of the build
//! entirely, which is what allows this application to stay MIT and sellable.
//! See `build.rs` for the matching dynamic-linking requirement.

use anyhow::{anyhow, bail, Context, Result};
use std::ffi::{c_void, CStr};
use std::path::Path;

use super::colour as raw_colour;
use super::decoder::{CfaPattern, RawDecoder, RawImage, Rect};

#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code
)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/libraw_bindings.rs"));

    // Hand-declared rather than generated: these signatures live in the C++
    // `libraw.h`, and pointing bindgen at that header pulls in the whole C++
    // class surface. They are short and have not changed in years — unlike the
    // struct layout above, which is exactly why that part is generated.
    extern "C" {
        pub fn libraw_init(flags: ::std::os::raw::c_uint) -> *mut libraw_data_t;
        pub fn libraw_close(lr: *mut libraw_data_t);
        pub fn libraw_open_buffer(
            lr: *mut libraw_data_t,
            buffer: *const ::std::os::raw::c_void,
            size: usize,
        ) -> ::std::os::raw::c_int;
        pub fn libraw_unpack(lr: *mut libraw_data_t) -> ::std::os::raw::c_int;
        pub fn libraw_unpack_thumb(lr: *mut libraw_data_t) -> ::std::os::raw::c_int;
        pub fn libraw_COLOR(
            lr: *mut libraw_data_t,
            row: ::std::os::raw::c_int,
            col: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int;
        pub fn libraw_strerror(code: ::std::os::raw::c_int) -> *const ::std::os::raw::c_char;
        pub fn libraw_version() -> *const ::std::os::raw::c_char;
        pub fn libraw_versionNumber() -> ::std::os::raw::c_int;
    }
}

/// Owns a `libraw_data_t` and guarantees `libraw_close` runs.
struct Handle(*mut sys::libraw_data_t);

impl Handle {
    fn new() -> Result<Self> {
        // SAFETY: no arguments to get wrong; a null result is checked below.
        let ptr = unsafe { sys::libraw_init(0) };
        if ptr.is_null() {
            bail!("libraw_init returned null");
        }
        Ok(Self(ptr))
    }

    fn data(&self) -> &sys::libraw_data_t {
        // SAFETY: non-null by construction, and LibRaw keeps the struct
        // allocated until `drop` closes it.
        unsafe { &*self.0 }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `libraw_init` and is closed only here.
        unsafe { sys::libraw_close(self.0) }
    }
}

/// A LibRaw call that failed, with LibRaw's own code — kept typed, rather than
/// flattened into text, so `errors::classify` can tell an unsupported camera
/// from a damaged file.
#[derive(Debug)]
pub struct LibRawError {
    pub code: i32,
    what: String,
    msg: String,
}

impl std::fmt::Display for LibRawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} failed: {} (LibRaw {})",
            self.what, self.msg, self.code
        )
    }
}

impl std::error::Error for LibRawError {}

fn check(code: i32, what: &str) -> Result<()> {
    if code == 0 {
        return Ok(());
    }
    // SAFETY: `libraw_strerror` returns a static NUL-terminated string for
    // any code, including ones it does not know.
    let msg = unsafe { CStr::from_ptr(sys::libraw_strerror(code)) }
        .to_string_lossy()
        .into_owned();
    Err(LibRawError {
        code,
        what: what.to_owned(),
        msg,
    }
    .into())
}

/// Whether `a` is a non-empty part of a `width` × `height` frame, in
/// arithmetic that cannot overflow.
fn active_area_fits(a: Rect, width: u32, height: u32) -> bool {
    a.width > 0
        && a.height > 0
        && u64::from(a.x) + u64::from(a.width) <= u64::from(width)
        && u64::from(a.y) + u64::from(a.height) <= u64::from(height)
}

fn c_str(bytes: &[::std::os::raw::c_char]) -> String {
    // SAFETY: `c_char` is `i8` or `u8`, so this is the same memory as bytes.
    let raw: &[u8] =
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u8, bytes.len()) };
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

type Mat3 = crate::gpu::color::Mat3;

/// The top-left 3x3 of a LibRaw matrix, or `None` when it is all zeros —
/// LibRaw's way of saying the file had no such matrix.
fn matrix3<const N: usize>(rows: &[[f32; N]]) -> Option<Mat3> {
    let mut out = [[0.0f32; 3]; 3];
    for (dst, src) in out.iter_mut().zip(rows) {
        dst.copy_from_slice(&src[..3]);
    }
    (out.iter().flatten().all(|v| v.is_finite()) && out.iter().flatten().any(|v| *v != 0.0))
        .then_some(out)
}

/// What a file says about its colour, gathered from LibRaw into plain values
/// so [`resolve_colour`] can be tested without a file.
#[derive(Debug, Clone, Default)]
struct ColourFacts {
    /// The file's own calibration, for a DNG that has one.
    dng: Option<raw_colour::DngColour>,
    /// LibRaw's table entry for the body: XYZ → camera, D65 — Adobe's
    /// published matrix for almost every camera LibRaw knows.
    table: Option<Mat3>,
    /// A matrix the camera itself recorded, camera → linear sRGB, for white-
    /// balanced data (Olympus and Pentax write one).
    camera_to_srgb: Option<Mat3>,
    /// As-shot multipliers, LibRaw's `cam_mul`: R, G, B, G2.
    cam_mul: [f32; 4],
    /// The camera applied its white balance to the data already (multiple
    /// exposures, Nikon sRAW and a few others); applying it again doubles it.
    balance_applied: bool,
    /// A monochrome sensor: no colour filter, so no colour to transform.
    monochrome: bool,
}

/// Which source a frame's colour matrix came from, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatrixSource {
    Dng,
    Table,
    Camera,
    Generic,
    Monochrome,
}

#[derive(Debug, Clone, PartialEq)]
struct ResolvedColour {
    cam_to_xyz: Mat3,
    /// As-shot multipliers, R, G, B, G2, always usable.
    wb: [f32; 4],
    source: MatrixSource,
}

/// Multipliers that can actually be applied: finite and positive on red,
/// green and blue. `cam_mul` is zero, or -1, or garbage on files that record
/// no balance, and taken literally that removes a channel from the photo.
fn usable(cam_mul: [f32; 4]) -> bool {
    cam_mul[..3].iter().all(|v| v.is_finite() && *v > 0.0)
}

/// The camera neutral (green = 1) behind a set of multipliers.
fn neutral_of(cam_mul: [f32; 4]) -> [f32; 3] {
    [cam_mul[1] / cam_mul[0], 1.0, cam_mul[1] / cam_mul[2]]
}

/// Multipliers for a camera neutral, the second green filled in.
fn multipliers(neutral: [f32; 3]) -> [f32; 4] {
    [1.0 / neutral[0], 1.0, 1.0 / neutral[2], 1.0]
}

/// The colour matrix and white balance a frame is rendered with.
///
/// **The matrix**, in order of preference:
///
/// 1. A DNG's own calibration, combined the way the DNG specification says
///    (`raw::colour`) — interpolated for the scene's white, through its
///    forward matrix, per-unit calibration and analog balance. A DNG from a
///    body LibRaw also knows gets its table matrix too, and used to take it
///    over the file's own; the file describes itself better.
/// 2. LibRaw's table, Adobe's D65 matrix for the body.
/// 3. The camera's own matrix, if it recorded one.
/// 4. A generic sensor ([`raw_colour::GENERIC_XYZ_TO_CAMERA`]). A body newer
///    than this build's LibRaw used to fail to open at all.
///
/// **The white balance** is the camera's as shot; where the file has none,
/// the daylight balance the matrix implies (dcraw's fallback), rather than
/// multipliers of zero; and ones where the camera already applied it.
///
/// The second green is always filled in. Cameras that treat both greens
/// identically leave `cam_mul[3]` at zero, and taken literally that multiplies
/// half the green photosites by nothing — a green checkerboard across the
/// whole frame, a striking artefact with a very unobvious cause.
fn resolve_colour(facts: &ColourFacts) -> ResolvedColour {
    use crate::gpu::color;
    use crate::raw::decoder::invert3;

    let as_shot = usable(facts.cam_mul).then(|| {
        let mut wb = facts.cam_mul;
        if !(wb[3].is_finite() && wb[3] > 0.0) {
            wb[3] = wb[1];
        }
        wb
    });
    let balanced = |wb: [f32; 4]| {
        if facts.balance_applied {
            [1.0; 4]
        } else {
            wb
        }
    };

    if facts.monochrome {
        return ResolvedColour {
            cam_to_xyz: crate::raster::SRGB_TO_XYZ,
            wb: [1.0; 4],
            source: MatrixSource::Monochrome,
        };
    }

    if let Some(dng) = &facts.dng {
        let d65 = [0.312_7, 0.329_0];
        let neutral = as_shot.map(neutral_of).or_else(|| dng.xy_to_neutral(d65));
        if let Some(neutral) = neutral {
            if let Some(cam_to_xyz) = dng.camera_to_xyz(neutral) {
                return ResolvedColour {
                    cam_to_xyz,
                    wb: balanced(as_shot.unwrap_or_else(|| multipliers(neutral))),
                    source: MatrixSource::Dng,
                };
            }
        }
    }

    let (cam_to_xyz, source) = facts
        .table
        .and_then(|m| invert3(m).map(|inv| (inv, MatrixSource::Table)))
        .or_else(|| {
            facts.camera_to_srgb.map(|m| {
                (
                    color::multiply(crate::raster::SRGB_TO_XYZ, m),
                    MatrixSource::Camera,
                )
            })
        })
        .unwrap_or_else(|| {
            (
                invert3(raw_colour::GENERIC_XYZ_TO_CAMERA).expect("the generic matrix inverts"),
                MatrixSource::Generic,
            )
        });
    let wb = as_shot.unwrap_or_else(|| {
        let neutral = invert3(cam_to_xyz)
            .and_then(raw_colour::daylight_neutral)
            .unwrap_or([1.0; 3]);
        multipliers(neutral)
    });
    ResolvedColour {
        cam_to_xyz,
        wb: balanced(wb),
        source,
    }
}

/// A DNG's calibration as LibRaw parsed it, or `None` for a file that is not
/// a DNG or carries no colour matrix.
///
/// LibRaw keeps a per-field "parsed" mask only on its private per-IFD copy,
/// so presence is read from the values: an absent matrix is all zeros, an
/// absent illuminant 0 or 0xFFFF.
fn dng_colour(dng_version: u32, color: &sys::libraw_colordata_t) -> Option<raw_colour::DngColour> {
    if dng_version == 0 {
        return None;
    }
    let calibrations: Vec<raw_colour::DngCalibration> = color
        .dng_color
        .iter()
        .filter_map(|c| {
            Some(raw_colour::DngCalibration {
                illuminant: c.illuminant,
                color_matrix: matrix3(&c.colormatrix)?,
                forward_matrix: matrix3(&c.forwardmatrix),
                camera_calibration: matrix3(&c.calibration),
            })
        })
        .collect();
    if calibrations.is_empty() {
        return None;
    }
    let ab = color.dng_levels.analogbalance;
    let analog_balance = std::array::from_fn(|i| {
        if ab[i].is_finite() && ab[i] > 0.0 {
            ab[i]
        } else {
            1.0
        }
    });
    Some(raw_colour::DngColour {
        calibrations,
        analog_balance,
    })
}

/// The white level the metadata states, per colour, before the frame's own
/// data has its say (`raw::levels`).
///
/// LibRaw's `maximum` is the format's ceiling. Canon alone records where its
/// sensor really saturates, as the specular white level (LibRaw's
/// `linear_max`): 14008 on an EOS R6 whose files run to 16383, and as low as
/// 13036 on an R7. LibRaw's own processing hides the gap by lowering the
/// level to the brightest value in each frame, which this pipeline does not
/// do — so a Canon's blown highlights sat at 0.85 of full scale, below where
/// the mosaic pass recognises them, and came out pink. Other makers' figures
/// in the same field are not a clip level (Nikon's is LibRaw's own guess at
/// `maximum / 1.07`, Sony's sits a thousand below where its data clips), so
/// only Canon's is believed.
fn nominal_white(
    idata: &sys::libraw_iparams_t,
    color: &sys::libraw_colordata_t,
    black: [u16; 4],
) -> [u16; 4] {
    let maximum = color.maximum.min(u16::MAX as u32) as u16;
    let canon = idata.dng_version == 0
        && c_str(&idata.make)
            .trim()
            .to_ascii_uppercase()
            .starts_with("CANON");
    std::array::from_fn(|c| {
        let stated = color.linear_max[c];
        let floor = black[c] as u32 + (maximum.saturating_sub(black[c]) as u32) / 2;
        // A DNG may give each sample its own `WhiteLevel`; LibRaw's `maximum`
        // is only the first.
        let dng = color.dng_levels.dng_whitelevel[c.min(2)];
        if canon && stated > floor && stated <= maximum as u32 {
            stated as u16
        } else if idata.dng_version != 0 && dng > black[c] as u32 && dng <= u16::MAX as u32 {
            dng as u16
        } else {
            maximum
        }
    })
}

/// Applies a DNG's lens-shading corrections — its gain maps and radial
/// vignette, from `OpcodeList2` and `OpcodeList3` — to the decoded values
/// (`raw::opcodes`). A phone's DNG leaves them to the reader, and without
/// them its corners come out dark and tinted.
fn shading_corrections(
    color: &sys::libraw_colordata_t,
    cfa: &mut [u16],
    width: u32,
    active: Rect,
    pattern: &CfaPattern,
    black: [u16; 4],
    white: [u16; 4],
) {
    use super::opcodes::{self, Stage, Target};
    let samples = pattern.samples();
    let linear = matches!(pattern, CfaPattern::Linear);
    let colour = |row: u32, col: u32, sample: u32| -> u32 {
        if linear {
            return sample;
        }
        match pattern.color_at(col, row) {
            3 => 1, // The second green is green.
            c => c as u32,
        }
    };
    // `rawopcodes[1]` is OpcodeList2, `[2]` OpcodeList3.
    for (list, stage) in [(1usize, Stage::Raw), (2, Stage::Demosaiced)] {
        let op = &color.dng_levels.rawopcodes[list];
        if op.len == 0 || op.data.is_null() {
            continue;
        }
        // SAFETY: LibRaw keeps each opcode list as `len` bytes at a non-null
        // `data`, owned by the handle `color` is borrowed from.
        let bytes = unsafe { std::slice::from_raw_parts(op.data as *const u8, op.len as usize) };
        let ops = opcodes::parse(bytes);
        opcodes::apply(
            &ops,
            stage,
            &mut Target {
                data: cfa,
                stride: (width * samples) as usize,
                origin: (active.y, active.x),
                size: (active.height, active.width),
                samples,
                colour: &colour,
                black,
                white,
            },
        );
    }
}

/// The level each channel saturates at: the stated one, moved to where the
/// frame's own data shows clipping if it shows any (`raw::levels`), and never
/// at or below black.
fn white_levels(
    cfa: &[u16],
    width: u32,
    active: Rect,
    pattern: &CfaPattern,
    black: [u16; 4],
    nominal: [u16; 4],
) -> [u16; 4] {
    let found = super::levels::clip_levels(cfa, width, active, pattern, black, nominal);
    std::array::from_fn(|c| found[c].max(black[c].saturating_add(1)))
}

/// The camera's own JPEG of the frame, as encoded bytes.
fn thumbnail_jpeg(handle: &Handle) -> Result<Vec<u8>> {
    check(
        // SAFETY: a live handle with a file open in it.
        unsafe { sys::libraw_unpack_thumb(handle.0) },
        "unpack_thumb",
    )?;
    let thumb = &handle.data().thumbnail;
    if thumb.thumb.is_null() || thumb.tlength == 0 {
        bail!("no embedded preview");
    }
    // tformat 1 is LIBRAW_THUMBNAIL_JPEG. Bitmap thumbnails exist but are
    // rare and tiny; the ingest path wants encoded JPEG bytes.
    if thumb.tformat != sys::LibRaw_thumbnail_formats_LIBRAW_THUMBNAIL_JPEG {
        bail!("a non-JPEG preview (format {})", thumb.tformat);
    }
    // SAFETY: after a successful unpack, `thumb` is `tlength` bytes LibRaw
    // allocated and keeps until the handle closes; they are copied out here.
    Ok(
        unsafe { std::slice::from_raw_parts(thumb.thumb as *const u8, thumb.tlength as usize) }
            .to_vec(),
    )
}

/// A DNG's `BaselineExposure`, in stops, when it has one. LibRaw leaves -999
/// when the tag is absent; anything outside a few stops is not a real value.
fn baseline_exposure(dng_version: u32, color: &sys::libraw_colordata_t) -> f32 {
    let be = color.dng_levels.baseline_exposure;
    if dng_version != 0 && be.is_finite() && be.abs() <= 6.0 {
        be
    } else {
        0.0
    }
}

/// LibRaw reports rotation as dcraw's `flip` code, not an EXIF value.
fn exif_orientation(flip: i32) -> u8 {
    match flip {
        0 => 1,
        3 => 3, // 180°
        5 => 8, // 90° counter-clockwise
        6 => 6, // 90° clockwise
        _ => 1,
    }
}

pub struct LibRawDecoder;

impl LibRawDecoder {
    pub fn new() -> Self {
        Self
    }

    /// LibRaw's own version string, e.g. `0.22.2-Release`.
    pub fn version() -> String {
        // SAFETY: LibRaw returns a static NUL-terminated string.
        unsafe { CStr::from_ptr(sys::libraw_version()) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn version_number() -> i32 {
        // SAFETY: takes no arguments and returns a constant.
        unsafe { sys::libraw_versionNumber() }
    }

    /// Opens a file into a handle with the raw data unpacked.
    ///
    /// Reads the whole file and uses `libraw_open_buffer` rather than
    /// `libraw_open_file`: the file-path entry point takes a narrow `char*`,
    /// which mangles non-ASCII paths on Windows. A buffer sidesteps path
    /// encoding entirely.
    fn open(path: &Path) -> Result<(Handle, Vec<u8>)> {
        let bytes = crate::raster::read_file(path)?;
        let handle = Handle::new()?;
        // SAFETY: the pointer and length describe `bytes`, and every caller
        // keeps `bytes`, returned beside the handle, for as long as it uses it.
        let code = unsafe {
            sys::libraw_open_buffer(handle.0, bytes.as_ptr() as *const c_void, bytes.len())
        };
        check(code, "open")?;
        // `bytes` is returned because LibRaw borrows the buffer rather than
        // copying it — dropping it here would leave the handle pointing at
        // freed memory.
        Ok((handle, bytes))
    }

    /// Camera metadata, for the formats a TIFF EXIF reader cannot see.
    ///
    /// `exif_read` handles TIFF-container raws directly and is much cheaper, so
    /// it stays the first choice. But CR3 is ISO-BMFF, and RAF, ORF and RW2 are
    /// not plain TIFF either, so for those it returns nothing at all — and
    /// ingest had no second source, which is how Canon, Fujifilm, Olympus and
    /// Panasonic files came to import with no date, no camera and no exposure.
    ///
    /// **No `libraw_unpack` here.** LibRaw fills `idata`, `sizes`, `other` and
    /// `lens` during `open_buffer`; unpacking decodes the sensor, which is
    /// thousands of times more work than reading a timestamp and is exactly
    /// what makes this affordable on the ingest path.
    pub fn metadata(&self, path: &Path) -> Result<RawMetadata> {
        let (handle, _buffer) = Self::open(path)?;
        let d = handle.data();
        let (idata, sizes, other, lens) = (&d.idata, &d.sizes, &d.other, &d.lens);

        // LibRaw leaves numeric fields at zero when the file did not say, so
        // zero means absent rather than "measured as nought" for every one of
        // these — an ISO of 0 or a 0mm lens is not a photograph.
        let positive = |v: f32| (v > 0.0).then_some(v);

        Ok(RawMetadata {
            capture_date: (other.timestamp > 0).then(|| format_timestamp(other.timestamp)),
            camera: crate::exif_read::camera_name(c_str(&idata.make), c_str(&idata.model)),
            lens: {
                let l = c_str(&lens.Lens).trim().to_string();
                (!l.is_empty()).then_some(l)
            },
            iso: positive(other.iso_speed).map(|v| v as i64),
            aperture: positive(other.aperture).map(|v| v as f64),
            // LibRaw reports shutter in seconds; the UI wants the photographic
            // form, where anything under a second is a reciprocal.
            shutter: positive(other.shutter).map(format_shutter),
            focal_length: positive(other.focal_len).map(|v| v as f64),
            // `sizes.width`/`height` are the visible frame after the sensor's
            // masked borders are removed, which is the number a photographer
            // recognises as the size of their picture. `raw_width` includes the
            // borders and would be a slightly larger, unfamiliar figure.
            width: (sizes.width > 0).then_some(sizes.width as i64),
            height: (sizes.height > 0).then_some(sizes.height as i64),
            // LibRaw parses the GPS block of the formats whose EXIF the
            // in-process reader cannot see (CR3, RAF), and flags it.
            gps: {
                let g = &other.parsed_gps;
                let lat = crate::geo::from_dms(
                    g.latitude[0] as f64,
                    g.latitude[1] as f64,
                    g.latitude[2] as f64,
                    g.latref as u8 == b'S',
                );
                let lon = crate::geo::from_dms(
                    g.longitude[0] as f64,
                    g.longitude[1] as f64,
                    g.longitude[2] as f64,
                    g.longref as u8 == b'W',
                );
                (g.gpsparsed != 0 && crate::geo::plausible(lat, lon)).then_some((lat, lon))
            },
            colour_space: preview_colour_space(
                d.makernotes.common.ColorSpace as u32,
                d.color.ExifColorSpace as u32,
            ),
        })
    }
}

/// The colour space the camera was set to, from LibRaw's reading of the maker
/// notes (Canon, Nikon, Olympus, Pentax, Samsung, Sony) and failing that of the
/// EXIF: what its embedded preview is encoded in. This is where a CR3 or ORF
/// says it, which no EXIF reader of ours can see.
fn preview_colour_space(makernote: u32, exif: u32) -> Option<crate::exif_read::ColourSpace> {
    use crate::exif_read::ColourSpace;
    // `enum LibRaw_colorspace` in libraw_const.h, which the bindings do not
    // generate: NotFound 0, sRGB 1, AdobeRGB 2, then wide gamut, ProPhoto and
    // others no camera offers for its previews.
    let read = |cs: u32| match cs {
        1 => Some(ColourSpace::Srgb),
        2 => Some(ColourSpace::AdobeRgb),
        _ => None,
    };
    read(makernote).or_else(|| read(exif))
}

/// What [`LibRawDecoder::metadata`] could recover. Every field is optional
/// because every field is genuinely absent from some real file.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RawMetadata {
    pub capture_date: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i64>,
    pub aperture: Option<f64>,
    pub shutter: Option<String>,
    pub focal_length: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub gps: Option<(f64, f64)>,
    pub colour_space: Option<crate::exif_read::ColourSpace>,
}

/// Unix seconds to the sortable `YYYY-MM-DD HH:MM:SS` the catalogue stores.
///
/// Local time, not UTC: the stored string is what the photographer sees as the
/// time they took the picture, and it is compared against date filters they
/// type in their own terms. Matches what `exif_read` produces from EXIF, which
/// is already local — mixing the two would put a photo in the wrong day.
fn format_timestamp(secs: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(secs, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => String::new(),
    }
}

/// Seconds to the photographic form: `1/250`, or `2.5"` for a long exposure.
fn format_shutter(secs: f32) -> String {
    if secs >= 1.0 {
        format!("{secs:.1}\"")
    } else {
        format!("1/{}", (1.0 / secs).round() as i64)
    }
}

impl Default for LibRawDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RawDecoder for LibRawDecoder {
    fn decode(&self, path: &Path) -> Result<RawImage> {
        let (handle, _buffer) = Self::open(path)?;
        // SAFETY: a live handle, and `_buffer` holds the file it reads.
        check(unsafe { sys::libraw_unpack(handle.0) }, "unpack")?;

        // The camera's own picture of the frame, whose brightness the pipeline
        // opens it at. Measured on a thread of its own while the sensor data
        // is prepared below: decoding a full-size preview costs about as much.
        let space = {
            let d = handle.data();
            preview_colour_space(
                d.makernotes.common.ColorSpace as u32,
                d.color.ExifColorSpace as u32,
            )
        };
        let camera = thumbnail_jpeg(&handle)
            .ok()
            .map(|jpeg| std::thread::spawn(move || crate::raster::median_green(&jpeg, space)));

        let d = handle.data();
        let sizes = &d.sizes;
        let color = &d.color;
        let idata = &d.idata;

        let width = sizes.raw_width as u32;
        let height = sizes.raw_height as u32;

        // Rows are `raw_pitch` bytes apart, which is not always the row's own
        // length: a decoder may pad them. Copying as if tightly packed would
        // shear the frame a little more on every row.
        let rows = |src: *const u16, stride: usize, take: usize| -> Result<Vec<u16>> {
            let row = width as usize * stride;
            let pitch = sizes.raw_pitch as usize / 2;
            // A pitch shorter than a row would read past the buffer below.
            anyhow::ensure!(
                row > 0 && pitch >= row,
                "LibRaw put rows {} bytes apart for {width} pixels in {}",
                sizes.raw_pitch,
                path.display()
            );
            let len = pitch
                .checked_mul(height as usize)
                .context("the frame is too large")?;
            // SAFETY: `src` is a non-null image LibRaw unpacked into `handle`,
            // which outlives this slice. LibRaw indexes it as `raw_height` rows
            // `raw_pitch` bytes apart itself, so it holds at least `len` values.
            let all = unsafe { std::slice::from_raw_parts(src, len) };
            Ok(all
                .chunks_exact(pitch)
                .flat_map(|line| {
                    line[..row]
                        .chunks_exact(stride)
                        .flat_map(move |px| &px[..take])
                })
                .copied()
                .collect())
        };
        let three_colour = idata.colors == 3 && idata.filters == 0;
        let (cfa, linear) = if !d.rawdata.raw_image.is_null() {
            (rows(d.rawdata.raw_image, 1, 1)?, false)
        } else if three_colour && !d.rawdata.color4_image.is_null() {
            // A *linear* DNG: already demosaiced, and handed over four values
            // a pixel, of which the fourth is unused.
            (rows(d.rawdata.color4_image as *const u16, 4, 3)?, true)
        } else if three_colour && !d.rawdata.color3_image.is_null() {
            (rows(d.rawdata.color3_image as *const u16, 3, 3)?, true)
        } else {
            // Foveon, and the odd sensor LibRaw hands over some other way.
            bail!(
                "{} has image data this app cannot read yet (neither a colour \
                 mosaic nor a linear DNG)",
                path.display()
            );
        };

        // A monochrome sensor (Leica's Monochroms, Pentax's K-3 III
        // Monochrome, Phase One's Achromatic backs): no colour filter, one
        // colour. Presented as a mosaic of nothing but green, which the Bayer
        // demosaic passes straight through as grey.
        let monochrome = idata.colors == 1;
        if idata.colors > 3 && c_str(&idata.cdesc) != "RGBG" {
            // Old cyan/magenta/yellow/green sensors: four genuinely different
            // colours, which a three-colour demosaic would misread entirely.
            bail!(
                "{} uses a {} colour filter, which is not supported",
                path.display(),
                c_str(&idata.cdesc)
            );
        }

        // `filters == 9` is LibRaw's marker for the X-Trans 6x6 mosaic.
        // `libraw_COLOR` answers for either layout, so the tile is sampled the
        // same way and only its size differs.
        let tile_colour = |y: i32, x: i32| -> Result<u8> {
            // SAFETY: a read-only query on the live handle, so `d`, borrowed
            // from it above, stays valid.
            let c = unsafe { sys::libraw_COLOR(handle.0, y, x) };
            // Everything downstream indexes four colours by this value.
            u8::try_from(c).ok().filter(|&c| c < 4).with_context(|| {
                format!("LibRaw gave colour {c} in the mosaic of {}", path.display())
            })
        };
        let cfa_pattern = if linear {
            CfaPattern::Linear
        } else if monochrome {
            CfaPattern::Bayer([1; 4])
        } else if idata.filters == 9 {
            let mut tile = [0u8; 36];
            for y in 0..6 {
                for x in 0..6 {
                    tile[y * 6 + x] = tile_colour(y as i32, x as i32)?;
                }
            }
            CfaPattern::XTrans(Box::new(tile))
        } else {
            let mut tile = [0u8; 4];
            for y in 0..2 {
                for x in 0..2 {
                    tile[y * 2 + x] = tile_colour(y as i32, x as i32)?;
                }
            }
            CfaPattern::Bayer(tile)
        };

        // Effective black is the global level plus the per-channel offset.
        // Sensors do not sit at zero in the dark, and the offset differs per
        // channel — collapsing it to one scalar tints the shadows.
        let level = |offset: u32| color.black.saturating_add(offset).min(u16::MAX as u32) as u16;
        let mut black = [0u16; 4];
        for (i, b) in black.iter_mut().enumerate() {
            *b = level(color.cblack[i]);
        }

        // **Some cameras keep the black level in a pattern grid instead.**
        //
        // `cblack[4]` and `cblack[5]` give the grid's dimensions and the values
        // follow from `cblack[6]`. LibRaw normally folds this into the
        // per-channel figures for you — but it does so inside `adjust_bl()`,
        // which runs as part of `dcraw_process()`, and this pipeline
        // deliberately never calls that. So it has to be done here.
        //
        // Missing it is not subtle. This Fuji reports `black` and every
        // `cblack[0..3]` as zero while holding 1022 in the grid, so every
        // photosite came out 1022 too high: blacks lifted to a foggy grey, and
        // a heavy magenta cast, because the white-balance multipliers scaled an
        // offset that should not have been there. The image was structurally
        // perfect and completely the wrong colour.
        let (grid_h, grid_w) = (color.cblack[4] as usize, color.cblack[5] as usize);
        if grid_h > 0 && grid_w > 0 && 6 + grid_h * grid_w <= color.cblack.len() {
            // Averaged per colour rather than applied per position: the black
            // array the shader takes is indexed by colour, and every grid this
            // has been seen on is uniform anyway. A genuinely varying grid would
            // need the pattern uploaded, which no sample so far calls for.
            // In `u64`: the grid's values come from the file, and their sum
            // can pass `u32::MAX`.
            let mut sums = [0u64; 4];
            let mut counts = [0u64; 4];
            for gy in 0..grid_h {
                for gx in 0..grid_w {
                    let colour = cfa_pattern.color_at(gx as u32, gy as u32) as usize;
                    sums[colour] += u64::from(color.cblack[6 + gy * grid_w + gx]);
                    counts[colour] += 1;
                }
            }
            // A colour the grid never lands on falls back to the grid's overall
            // mean rather than keeping zero. X-Trans never uses the second
            // green, so index 3 legitimately gets no cells — but leaving *any*
            // colour at zero when the rest subtract a thousand would put bright
            // speckles across the frame if a layout ever did use it.
            let total = sums.iter().sum::<u64>() / (grid_h * grid_w).max(1) as u64;
            for (i, b) in black.iter_mut().enumerate() {
                let offset = sums[i].checked_div(counts[i]).unwrap_or(total);
                *b = (*b as u64 + offset).min(u16::MAX as u64) as u16;
            }
        }

        if monochrome {
            // One colour, whose own offset LibRaw files under index 0; a grid,
            // if there was one, went to index 1 above, the colour the pattern
            // here claims for every photosite. Both, for everyone.
            let grid = black[1].saturating_sub(level(color.cblack[1]));
            black = [level(color.cblack[0]).saturating_add(grid); 4];
        }

        let active_area = Rect {
            x: sizes.left_margin as u32,
            y: sizes.top_margin as u32,
            width: sizes.width as u32,
            height: sizes.height as u32,
        };
        // Checked before anything below indexes the sensor buffer with it:
        // `validate` checks it too, but after shading and white levels
        // have sliced by it, so a file whose active area runs past its stored
        // frame panicked in Develop instead of being refused.
        anyhow::ensure!(
            active_area_fits(active_area, width, height),
            "LibRaw produced an inconsistent frame for {}: active area {active_area:?} \
             does not fit inside {width}x{height}",
            path.display()
        );
        let nominal = nominal_white(idata, color, black);
        let mut cfa = cfa;
        if idata.dng_version != 0 {
            shading_corrections(
                color,
                &mut cfa,
                width,
                active_area,
                &cfa_pattern,
                black,
                nominal,
            );
        }
        let white = white_levels(&cfa, width, active_area, &cfa_pattern, black, nominal);

        let colour = resolve_colour(&ColourFacts {
            dng: dng_colour(idata.dng_version, color),
            // `cam_xyz` follows the DNG convention despite its name: XYZ →
            // camera, the opposite of what the pipeline wants — the resolver
            // inverts it.
            table: matrix3(&color.cam_xyz),
            // LibRaw's own sanity check before it will use one.
            camera_to_srgb: matrix3(&color.cmatrix).filter(|m| m[0][0] > 0.125),
            cam_mul: color.cam_mul,
            balance_applied: color.as_shot_wb_applied != 0,
            monochrome,
        });
        if matches!(colour.source, MatrixSource::Generic | MatrixSource::Camera) {
            eprintln!(
                "{} {}: not in LibRaw {}'s camera table; colour from {}",
                c_str(&idata.make).trim(),
                c_str(&idata.model).trim(),
                Self::version(),
                if colour.source == MatrixSource::Camera {
                    "the camera's own matrix"
                } else {
                    "a generic sensor matrix and may be a little off"
                }
            );
        }

        let image = RawImage {
            cfa,
            width,
            height,
            cfa_pattern,
            black,
            white,
            wb_coeffs: colour.wb,
            cam_to_xyz: colour.cam_to_xyz,
            baseline_exposure: baseline_exposure(idata.dng_version, color),
            camera_median: camera.and_then(|t| t.join().ok().flatten()),
            active_area,
            orientation: exif_orientation(sizes.flip),
            make: c_str(&idata.make),
            model: c_str(&idata.model),
        };

        image.validate().with_context(|| {
            format!(
                "LibRaw produced an inconsistent frame for {}",
                path.display()
            )
        })?;
        Ok(image)
    }

    fn embedded_preview(&self, path: &Path) -> Result<Vec<u8>> {
        let (handle, _buffer) = Self::open(path)?;
        thumbnail_jpeg(&handle).map_err(|e| anyhow!("{} has {e}", path.display()))
    }

    fn extensions(&self) -> &'static [&'static str] {
        super::SUPPORTED_EXTENSIONS
    }

    fn describe(&self) -> String {
        format!("LibRaw {}", Self::version())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the DLL actually loaded and we are talking to the version we
    /// vendored. If the loader picked up a different LibRaw from PATH, this is
    /// where it shows up.
    #[test]
    fn links_against_the_vendored_libraw() {
        // The version recorded beside the DLL, which is what
        // `scripts/check-libraw.sh` compares with upstream. So the record has
        // to describe the DLL that actually loads, not just the one in the tree.
        let record = include_str!("../../vendor/libraw/VERSION");
        let recorded = record
            .lines()
            .find_map(|l| l.strip_prefix("version="))
            .expect("version= in vendor/libraw/VERSION")
            .trim();
        let numeric = recorded.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let version = LibRawDecoder::version();
        assert!(
            version == numeric || version.starts_with(&format!("{numeric}-")),
            "loaded LibRaw {version}, but vendor/libraw/VERSION says {recorded} — \
             is a different DLL on the load path?"
        );
        assert!(LibRawDecoder::version_number() > 0);
    }

    /// LibRaw's ILCE-6400 table entry: XYZ → camera.
    const A6400: Mat3 = [
        [0.7657, -0.2847, -0.0607],
        [-0.4083, 1.1966, 0.2389],
        [-0.0684, 0.1418, 0.5844],
    ];

    fn table_camera(cam_mul: [f32; 4]) -> ColourFacts {
        ColourFacts {
            table: Some(A6400),
            cam_mul,
            ..Default::default()
        }
    }

    /// Observed on a DJI DNG, which reports `[2.01, 1.00, 1.72, 0.00]`.
    #[test]
    fn a_zero_second_green_inherits_the_first() {
        let r = resolve_colour(&table_camera([2.01, 1.0, 1.72, 0.0]));
        assert_eq!(r.wb, [2.01, 1.0, 1.72, 1.0]);
    }

    #[test]
    fn a_populated_second_green_is_left_alone() {
        let wb = [1872.0, 1024.0, 2860.0, 1024.0];
        assert_eq!(resolve_colour(&table_camera(wb)).wb, wb);
    }

    /// A body in LibRaw's table renders exactly as it always has: the table
    /// matrix, inverted, and the as-shot multipliers.
    #[test]
    fn a_table_camera_keeps_its_matrix_and_balance() {
        let wb = [2816.0, 1024.0, 1736.0, 1024.0];
        let r = resolve_colour(&table_camera(wb));
        assert_eq!(r.source, MatrixSource::Table);
        assert_eq!(r.cam_to_xyz, crate::raw::decoder::invert3(A6400).unwrap());
        assert_eq!(r.wb, wb);
    }

    /// A file with no usable balance used to reach the pipeline with a zero
    /// multiplier — a channel removed from the photo. Daylight, as the matrix
    /// predicts it, is what dcraw falls back on too.
    #[test]
    fn a_missing_balance_falls_back_to_daylight() {
        for cam_mul in [[0.0; 4], [-1.0, 1.0, 1.0, 0.0], [f32::NAN, 1.0, 2.0, 1.0]] {
            let r = resolve_colour(&table_camera(cam_mul));
            let daylight = raw_colour::daylight_neutral(A6400).unwrap();
            assert!(
                (r.wb[0] - 1.0 / daylight[0]).abs() < 1e-5
                    && (r.wb[2] - 1.0 / daylight[2]).abs() < 1e-5
                    && r.wb[1] == 1.0
                    && r.wb[3] == 1.0,
                "{cam_mul:?} -> {:?}",
                r.wb
            );
        }
    }

    /// Multiple exposures and a few other modes store data the camera has
    /// already balanced; balancing it again doubles the cast.
    #[test]
    fn a_balance_the_camera_applied_is_not_applied_again() {
        let mut facts = table_camera([2816.0, 1024.0, 1736.0, 1024.0]);
        facts.balance_applied = true;
        assert_eq!(resolve_colour(&facts).wb, [1.0; 4]);
    }

    /// A body newer than the build's LibRaw has no table entry. It used to
    /// fail to open; now it opens with the camera's own matrix, or a generic
    /// one.
    #[test]
    fn a_camera_missing_from_the_table_still_opens() {
        let generic = resolve_colour(&ColourFacts {
            cam_mul: [2.0, 1.0, 1.5, 0.0],
            ..Default::default()
        });
        assert_eq!(generic.source, MatrixSource::Generic);
        assert!(crate::gpu::color::camera_to_working(generic.cam_to_xyz).is_some());

        let own = [[1.7, -0.6, -0.1], [-0.25, 1.45, -0.2], [0.05, -0.5, 1.45]];
        let camera = resolve_colour(&ColourFacts {
            camera_to_srgb: Some(own),
            cam_mul: [2.0, 1.0, 1.5, 0.0],
            ..Default::default()
        });
        assert_eq!(camera.source, MatrixSource::Camera);
    }

    /// A DNG's own calibration wins over LibRaw's table entry for the same
    /// body: the file describes itself.
    #[test]
    fn a_dng_uses_its_own_calibration() {
        let dng = raw_colour::DngColour {
            calibrations: vec![raw_colour::DngCalibration {
                illuminant: 21,
                color_matrix: A6400,
                forward_matrix: None,
                camera_calibration: None,
            }],
            analog_balance: [1.0; 3],
        };
        let mut facts = table_camera([2.0, 1.0, 1.5, 0.0]);
        facts.dng = Some(dng);
        let r = resolve_colour(&facts);
        assert_eq!(r.source, MatrixSource::Dng);
        assert_eq!(r.wb, [2.0, 1.0, 1.5, 1.0]);
        // Balanced white lands on D65 white.
        let white = crate::gpu::color::apply(r.cam_to_xyz, [1.0; 3]);
        for (w, d) in white.iter().zip(raw_colour::D65) {
            assert!((w / white[1] - d).abs() < 2e-3, "{white:?}");
        }
    }

    #[test]
    fn a_monochrome_sensor_is_neutral() {
        let r = resolve_colour(&ColourFacts {
            monochrome: true,
            ..Default::default()
        });
        assert_eq!(r.wb, [1.0; 4]);
        assert_eq!(r.source, MatrixSource::Monochrome);
    }

    #[test]
    fn dcraw_flip_maps_onto_exif_orientation() {
        assert_eq!(exif_orientation(0), 1);
        assert_eq!(exif_orientation(3), 3);
        assert_eq!(exif_orientation(5), 8);
        assert_eq!(exif_orientation(6), 6);
        // Anything unexpected must degrade to "upright", not to an invalid value.
        assert_eq!(exif_orientation(99), 1);
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let err = LibRawDecoder::new()
            .decode(Path::new("does-not-exist.arw"))
            .unwrap_err();
        assert!(err.to_string().contains("cannot read"));
    }

    /// Garbage must be rejected by LibRaw rather than producing a frame.
    #[test]
    fn a_non_raw_file_is_rejected() {
        let dir = crate::test_support::Scratch::new("libraw");
        let path = dir.join("not-a-raw.arw");
        std::fs::write(&path, b"this is definitely not a raw file").unwrap();

        assert!(LibRawDecoder::new().decode(&path).is_err());
    }
}
