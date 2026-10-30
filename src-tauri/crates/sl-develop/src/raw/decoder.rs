//! The decode contract between raw files and the develop pipeline.
//!
//! A [`RawImage`] is deliberately **undeveloped**: sensor values still in their
//! colour-filter-array layout, plus the metadata needed to interpret them. No
//! demosaic, no white balance, no colour transform — those are pipeline stages
//! in WGSL, where they run on the GPU and can be re-run per slider tick.
//!
//! Keeping this a trait matters for more than tidiness. The decoder backend is
//! the one part of the app with a licence constraint attached to it, so the rest
//! of the codebase must never name a concrete decoder.

use anyhow::Result;
use std::path::Path;

/// Colour-filter-array layout, as a repeating tile of colour indices
/// (`0 = red, 1 = green, 2 = blue, 3 = second green`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfaPattern {
    /// Standard 2x2 Bayer tile, row-major from the top-left of the active area.
    Bayer([u8; 4]),
    /// Fujifilm X-Trans 6x6 tile. Its irregular layout is why X-Trans needs a
    /// separate demosaic path rather than a parameterised Bayer one.
    XTrans(Box<[u8; 36]>),
    /// No mosaic at all: every pixel carries red, green and blue, interleaved
    /// in [`RawImage::cfa`]. A *linear* DNG — iPhone ProRAW, Samsung Expert
    /// RAW, and what editors write for an enhanced, denoised or merged
    /// photo — is already demosaiced, and goes straight to the linear image.
    Linear,
}

impl CfaPattern {
    /// Colour index of the photosite at `(x, y)`, relative to the active area's
    /// top-left corner. For [`Self::Linear`], green: every pixel has one, and
    /// [`RawImage::at`] reads it.
    pub fn color_at(&self, x: u32, y: u32) -> u8 {
        match self {
            Self::Bayer(tile) => tile[((y % 2) * 2 + (x % 2)) as usize],
            Self::XTrans(tile) => tile[((y % 6) * 6 + (x % 6)) as usize],
            Self::Linear => 1,
        }
    }

    /// Tile edge length — 2 for Bayer, 6 for X-Trans, 1 for no mosaic.
    pub fn period(&self) -> u32 {
        match self {
            Self::Bayer(_) => 2,
            Self::XTrans(_) => 6,
            Self::Linear => 1,
        }
    }

    /// Values stored per pixel: three for [`Self::Linear`], one for a mosaic.
    pub fn samples(&self) -> u32 {
        match self {
            Self::Linear => 3,
            _ => 1,
        }
    }
}

/// Inverts a 3x3 matrix, or `None` if it is singular.
///
/// Needed because raw metadata and the render pipeline disagree about
/// direction: the DNG specification (and therefore dcraw and LibRaw) stores the
/// colour matrix as **XYZ → camera**, while a pipeline wants **camera → XYZ**.
/// Using one where the other is meant produces colours that look plausible
/// enough to ship and are entirely wrong.
pub fn invert3(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);

    if det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;

    let mut out = [[0.0f32; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            // Transposed cofactor — note the (c, r) indexing, which is the
            // adjugate rather than the cofactor matrix.
            let (r0, r1) = ((c + 1) % 3, (c + 2) % 3);
            let (c0, c1) = ((r + 1) % 3, (r + 2) % 3);
            *v = (m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]) * inv_det;
        }
    }
    Some(out)
}

/// The valid image region inside the sensor array. The margins outside it hold
/// masked calibration pixels, which is where the black level is measured — they
/// must be cropped before display but are not junk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A decoded raw frame.
#[derive(Debug, Clone)]
pub struct RawImage {
    /// One value per photosite, row-major over the **full** sensor array
    /// (`width * height`), still in CFA layout. Crop with [`Self::active_area`].
    /// Three per pixel, red, green, blue, for [`CfaPattern::Linear`].
    pub cfa: Vec<u16>,
    pub width: u32,
    pub height: u32,

    pub cfa_pattern: CfaPattern,

    /// Per-channel black point, indexed by colour like [`CfaPattern`]. Sensors
    /// encode "no light" as a positive value, and it differs per channel, so a
    /// single scalar would tint the shadows.
    pub black: [u16; 4],
    /// Saturation point per channel, indexed by colour like [`CfaPattern`].
    /// Anything at or above this is clipped highlight data and only highlight
    /// reconstruction can recover it.
    ///
    /// Where the *sensor* saturates, which is not always the format's ceiling:
    /// see `raw::levels`. Normalising by a level above the real one leaves
    /// blown highlights short of full scale, where nothing recognises them as
    /// blown.
    pub white: [u16; 4],

    /// As-shot white balance multipliers, indexed by colour like [`CfaPattern`].
    /// Always usable: a decoder fills in daylight when the file records no
    /// balance, and ones when the camera already applied it to the data.
    pub wb_coeffs: [f32; 4],
    /// Camera native RGB to CIE XYZ (D65). The first transform after demosaic,
    /// and the reason two cameras of the same scene agree on colour.
    ///
    /// Note the direction. Raw metadata stores the *inverse* of this — the DNG
    /// spec defines `ColorMatrix` as XYZ → camera — so decoders must invert
    /// before filling this in.
    ///
    /// Only the matrix's action on *white-balanced* camera RGB matters: the
    /// pipeline normalises it so balanced white is white
    /// (`gpu::color::camera_to_working`), which makes it indifferent to a
    /// diagonal scaling on the camera side. That is what lets a DNG's forward
    /// matrix, defined on balanced data, travel in the same field.
    pub cam_to_xyz: [[f32; 3]; 3],
    /// Stops to add to the exposure before anything the user sets: a DNG's
    /// `BaselineExposure`, which is how a phone or a converter says its raw
    /// is meant to be shown brighter than the data alone suggests. Zero for a
    /// camera's own raw, whose placement the tone curve is calibrated to.
    pub baseline_exposure: f32,
    /// The median green of the camera's own JPEG, the preview this file
    /// embeds, in linear display light: what the photographer saw on the
    /// camera. The pipeline opens the frame at that brightness
    /// (`ToneParams::exposure_matching`). `None` without a usable preview;
    /// then `baseline_exposure` stands in.
    pub camera_median: Option<f32>,

    pub active_area: Rect,
    /// EXIF orientation, 1..=8.
    pub orientation: u8,

    /// Camera that produced the file, for lens-correction and profile lookup.
    pub make: String,
    pub model: String,
}

impl RawImage {
    /// Value at a sensor coordinate, of the colour `cfa_pattern.color_at` names
    /// there, or `None` outside the array.
    pub fn at(&self, x: u32, y: u32) -> Option<u16> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let samples = self.cfa_pattern.samples();
        let offset = match self.cfa_pattern {
            CfaPattern::Linear => 1,
            _ => 0,
        };
        self.cfa
            .get(((y * self.width + x) * samples + offset) as usize)
            .copied()
    }

    /// Cheap sanity check for decoder output. A decoder that silently returns a
    /// short buffer or a zero white level produces a black or wildly clipped
    /// image much later, in the GPU pipeline, where the cause is far harder to see.
    pub fn validate(&self) -> Result<()> {
        let expected =
            (self.width as usize) * (self.height as usize) * self.cfa_pattern.samples() as usize;
        anyhow::ensure!(
            self.cfa.len() == expected,
            "CFA buffer is {} values, expected {expected} for {}x{}",
            self.cfa.len(),
            self.width,
            self.height
        );
        anyhow::ensure!(self.width > 0 && self.height > 0, "zero-sized raw image");
        anyhow::ensure!(self.white.iter().all(|&w| w > 0), "white level is zero");
        anyhow::ensure!(
            self.black.iter().zip(self.white).all(|(&b, w)| b < w),
            "black level {:?} is not below white level {:?}",
            self.black,
            self.white
        );
        anyhow::ensure!(
            self.wb_coeffs[..3]
                .iter()
                .all(|w| w.is_finite() && *w > 0.0),
            "white balance {:?} is not usable",
            self.wb_coeffs
        );

        let a = self.active_area;
        anyhow::ensure!(
            a.width > 0
                && a.height > 0
                && u64::from(a.x) + u64::from(a.width) <= u64::from(self.width)
                && u64::from(a.y) + u64::from(a.height) <= u64::from(self.height),
            "active area {a:?} does not fit inside {}x{}",
            self.width,
            self.height
        );
        anyhow::ensure!(
            (1..=8).contains(&self.orientation),
            "orientation {} is outside 1..=8",
            self.orientation
        );
        Ok(())
    }
}

/// A raw format backend.
///
/// `Send + Sync` so a batch export can decode across a rayon pool.
pub trait RawDecoder: Send + Sync {
    /// Full sensor decode. Expensive — used when opening a photo in the editor
    /// or exporting, never during a library scan.
    fn decode(&self, path: &Path) -> Result<RawImage>;

    /// The largest embedded preview JPEG, returned as encoded bytes.
    ///
    /// This is what a library scan uses: every raw carries a camera-rendered
    /// preview, and pulling it out is orders of magnitude cheaper than decoding
    /// the sensor data.
    fn embedded_preview(&self, path: &Path) -> Result<Vec<u8>>;

    /// Lowercase extensions this backend handles, without the dot.
    fn extensions(&self) -> &'static [&'static str];

    /// Human-readable backend identity, including the underlying library
    /// version — worth surfacing in diagnostics, since decode differences
    /// between versions are otherwise invisible.
    fn describe(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rggb() -> CfaPattern {
        CfaPattern::Bayer([0, 1, 3, 2])
    }

    #[test]
    fn bayer_tile_repeats_across_the_sensor() {
        let p = rggb();
        // Top-left 2x2 is the tile itself.
        assert_eq!((p.color_at(0, 0), p.color_at(1, 0)), (0, 1));
        assert_eq!((p.color_at(0, 1), p.color_at(1, 1)), (3, 2));
        // And it tiles.
        assert_eq!(p.color_at(2, 2), p.color_at(0, 0));
        assert_eq!(p.color_at(101, 51), p.color_at(1, 1));
        assert_eq!(p.period(), 2);
    }

    #[test]
    fn xtrans_uses_a_six_by_six_tile() {
        let mut tile = [1u8; 36];
        tile[0] = 0;
        tile[35] = 2;
        let p = CfaPattern::XTrans(Box::new(tile));
        assert_eq!(p.color_at(0, 0), 0);
        assert_eq!(p.color_at(5, 5), 2);
        assert_eq!(p.color_at(6, 6), 0, "tile must repeat every 6 photosites");
        assert_eq!(p.period(), 6);
    }

    fn valid_image() -> RawImage {
        RawImage {
            cfa: vec![100; 16],
            width: 4,
            height: 4,
            cfa_pattern: rggb(),
            black: [16, 16, 16, 16],
            white: [16383; 4],
            wb_coeffs: [2.0, 1.0, 1.5, 1.0],
            cam_to_xyz: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            baseline_exposure: 0.0,
            camera_median: None,
            active_area: Rect {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
            orientation: 1,
            make: "Test".into(),
            model: "Cam".into(),
        }
    }

    #[test]
    fn a_well_formed_image_validates() {
        valid_image().validate().unwrap();
    }

    /// The failure this exists to catch: a decoder returning fewer values than
    /// the dimensions promise. Downstream that reads as a torn or black frame.
    #[test]
    fn a_short_cfa_buffer_is_rejected() {
        let mut img = valid_image();
        img.cfa.pop();
        let err = img.validate().unwrap_err().to_string();
        assert!(err.contains("expected 16"), "unexpected error: {err}");
    }

    #[test]
    fn black_at_or_above_white_is_rejected() {
        let mut img = valid_image();
        img.black[2] = img.white[2];
        assert!(img.validate().is_err());
    }

    /// A zero multiplier removes a channel from the photo — a black or
    /// single-colour frame. Decoders replace a missing balance, so reaching
    /// the pipeline with one is a decoder bug.
    #[test]
    fn an_unusable_white_balance_is_rejected() {
        let mut img = valid_image();
        img.wb_coeffs = [0.0, 1.0, 1.5, 1.0];
        assert!(img.validate().is_err());
        img.wb_coeffs = [2.0, f32::NAN, 1.5, 1.0];
        assert!(img.validate().is_err());
    }

    #[test]
    fn an_active_area_outside_the_sensor_is_rejected() {
        let mut img = valid_image();
        img.active_area.width = 5;
        assert!(img.validate().is_err());
    }

    fn multiply(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
        let mut out = [[0.0f32; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                out[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum();
            }
        }
        out
    }

    fn assert_identity(m: [[f32; 3]; 3]) {
        for (r, row) in m.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                let expected = if r == c { 1.0 } else { 0.0 };
                assert!(
                    (v - expected).abs() < 1e-4,
                    "element ({r},{c}) is {v}, expected {expected}\n{m:?}"
                );
            }
        }
    }

    #[test]
    fn inverting_a_matrix_round_trips_to_identity() {
        let m = [[2.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 8.0]];
        let inv = invert3(m).unwrap();
        assert_identity(multiply(m, inv));
        assert_identity(multiply(inv, m));
    }

    /// A real published matrix, not a contrived one: the Sony ILCE-6400
    /// ColorMatrix2 as it appears in raw metadata (XYZ -> camera). Diagonal
    /// test matrices invert correctly even with transposition bugs, so they
    /// would not catch the mistake this guards against.
    #[test]
    fn a_real_camera_matrix_inverts_cleanly() {
        let xyz_to_cam = [
            [0.6961, -0.1682, -0.0562],
            [-0.4443, 1.2049, 0.2762],
            [-0.0709, 0.1550, 0.5809],
        ];
        let cam_to_xyz = invert3(xyz_to_cam).unwrap();
        assert_identity(multiply(xyz_to_cam, cam_to_xyz));

        // Sanity on the result itself: a camera's red primary must carry more
        // X than Z, which is only true if the inverse came out the right way
        // round rather than transposed.
        assert!(
            cam_to_xyz[0][0] > cam_to_xyz[2][0],
            "camera red maps to more Z than X — matrix looks transposed: {cam_to_xyz:?}"
        );
    }

    #[test]
    fn a_singular_matrix_has_no_inverse() {
        // Second row is twice the first, so the rows are linearly dependent.
        let m = [[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [1.0, 0.0, 1.0]];
        assert!(invert3(m).is_none());
    }

    #[test]
    fn sensor_reads_are_bounds_checked() {
        let img = valid_image();
        assert_eq!(img.at(3, 3), Some(100));
        assert_eq!(img.at(4, 0), None);
        assert_eq!(img.at(0, 4), None);
    }
}
