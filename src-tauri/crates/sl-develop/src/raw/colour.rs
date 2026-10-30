//! What a raw file's numbers mean as colour: the matrix from its camera RGB to
//! CIE XYZ, and the white its as-shot balance describes.
//!
//! Kept free of any decoder so the arithmetic can be tested on its own. The
//! LibRaw backend gathers what the file says and hands it here.
//!
//! **Two kinds of file, two methods.** A camera's own raw (ARW, NEF, CR3...)
//! comes with one matrix from LibRaw's table, measured under D65, and nothing
//! else; the pipeline white-balances it in camera space (see
//! `gpu::color::camera_to_working`). A DNG carries its own calibration, often
//! for two illuminants, sometimes with forward matrices and a per-unit
//! calibration, and the DNG specification says exactly how to combine them
//! (chapter 6, "Mapping Camera Color Space to CIE XYZ Space"). Reading only one
//! of its matrices — which is what this app used to do — renders a phone or
//! drone DNG shot under tungsten with the daylight matrix, and ignores the
//! forward matrix that most DNG writers include precisely for accuracy.

use crate::gpu::color::{self, Mat3};
use crate::raw::decoder::invert3;

/// CIE XYZ of D65, Y = 1.
pub const D65: [f32; 3] = [0.950_47, 1.0, 1.088_83];
/// CIE XYZ of D50, Y = 1: the ICC connection space, where DNG's forward
/// matrices land.
pub const D50: [f32; 3] = [0.964_22, 1.0, 0.825_21];

/// Bradford cone response.
const BRADFORD: Mat3 = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// Linear Bradford adaptation taking colours seen under the white `from` to
/// their appearance under `to` (both XYZ). Used by the DNG path, which the
/// specification defines in these terms, and nowhere else.
pub fn bradford(from: [f32; 3], to: [f32; 3]) -> Mat3 {
    let cone_from = color::apply(BRADFORD, from);
    let cone_to = color::apply(BRADFORD, to);
    let scale = [
        [cone_to[0] / cone_from[0], 0.0, 0.0],
        [0.0, cone_to[1] / cone_from[1], 0.0],
        [0.0, 0.0, cone_to[2] / cone_from[2]],
    ];
    let inverse = invert3(BRADFORD).expect("Bradford inverts");
    color::multiply(inverse, color::multiply(scale, BRADFORD))
}

fn xyz_to_xy(xyz: [f32; 3]) -> Option<[f32; 2]> {
    let sum = xyz[0] + xyz[1] + xyz[2];
    (sum.is_finite() && sum > 1e-9).then(|| [xyz[0] / sum, xyz[1] / sum])
}

fn xy_to_xyz(xy: [f32; 2]) -> [f32; 3] {
    let y = xy[1].max(1e-6);
    [xy[0] / y, 1.0, (1.0 - xy[0] - xy[1]) / y]
}

/// Robertson's isotemperature lines: reciprocal megakelvin, then the CIE 1960
/// (u, v) where each crosses the Planckian locus, and its slope. Robertson
/// (1968) as tabulated by Wyszecki & Stiles, with the known misprint at 325
/// corrected.
pub(crate) const ROBERTSON: [(f32, f32, f32, f32); 31] = [
    (0.0, 0.180_06, 0.263_52, -0.243_41),
    (10.0, 0.180_66, 0.265_89, -0.254_79),
    (20.0, 0.181_33, 0.268_46, -0.268_76),
    (30.0, 0.182_08, 0.271_19, -0.285_39),
    (40.0, 0.182_93, 0.274_07, -0.304_70),
    (50.0, 0.183_88, 0.277_09, -0.326_75),
    (60.0, 0.184_94, 0.280_21, -0.351_56),
    (70.0, 0.186_11, 0.283_42, -0.379_15),
    (80.0, 0.187_40, 0.286_68, -0.409_55),
    (90.0, 0.188_80, 0.289_97, -0.442_78),
    (100.0, 0.190_32, 0.293_26, -0.478_88),
    (125.0, 0.194_62, 0.301_41, -0.582_04),
    (150.0, 0.199_62, 0.309_21, -0.704_71),
    (175.0, 0.205_25, 0.316_47, -0.849_01),
    (200.0, 0.211_42, 0.323_12, -1.018_2),
    (225.0, 0.218_07, 0.329_09, -1.216_8),
    (250.0, 0.225_11, 0.334_39, -1.451_2),
    (275.0, 0.232_47, 0.339_04, -1.729_8),
    (300.0, 0.240_10, 0.343_08, -2.063_7),
    (325.0, 0.247_92, 0.346_55, -2.468_1),
    (350.0, 0.255_91, 0.349_51, -2.964_1),
    (375.0, 0.264_00, 0.352_00, -3.581_4),
    (400.0, 0.272_18, 0.354_07, -4.363_3),
    (425.0, 0.280_39, 0.355_77, -5.376_2),
    (450.0, 0.288_63, 0.357_14, -6.726_2),
    (475.0, 0.296_85, 0.358_23, -8.595_5),
    (500.0, 0.305_05, 0.359_07, -11.324),
    (525.0, 0.313_20, 0.359_68, -15.628),
    (550.0, 0.321_29, 0.360_11, -23.325),
    (575.0, 0.329_31, 0.360_38, -40.770),
    (600.0, 0.337_24, 0.360_51, -116.45),
];

/// Correlated colour temperature of a chromaticity, in kelvin, by
/// Robertson's method: find the two isotemperature lines the point falls
/// between and interpolate. Clamped to the table's range, 1667 K to infinity
/// (reported as 50 000 K).
pub fn temperature(xy: [f32; 2]) -> f32 {
    let denom = 1.5 - xy[0] + 6.0 * xy[1];
    let (u, v) = (2.0 * xy[0] / denom, 3.0 * xy[1] / denom);
    let mut last_dt = 0.0f32;
    for i in 1..ROBERTSON.len() {
        let (_, tu, tv, slope) = ROBERTSON[i];
        let len = (1.0 + slope * slope).sqrt();
        let (du, dv) = (1.0 / len, slope / len);
        let mut dt = -(u - tu) * dv + (v - tv) * du;
        if dt <= 0.0 || i == ROBERTSON.len() - 1 {
            dt = (-dt).max(0.0);
            let f = if i == 1 { 0.0 } else { dt / (last_dt + dt) };
            let mired = ROBERTSON[i - 1].0 * f + ROBERTSON[i].0 * (1.0 - f);
            return (1.0e6 / mired.max(20.0)).min(50_000.0);
        }
        last_dt = dt;
    }
    6500.0
}

/// The colour temperature an EXIF `LightSource` code stands for, as the DNG
/// SDK assigns them. `None` for unknown, "other" and anything unlisted — a
/// matrix whose illuminant is unknown cannot be placed for interpolation.
pub fn illuminant_temperature(code: u16) -> Option<f32> {
    Some(match code {
        17 | 3 => 2850.0,              // Standard light A, tungsten
        24 => 3200.0,                  // ISO studio tungsten
        23 => 5000.0,                  // D50
        20 | 1 | 9 | 4 | 18 => 5500.0, // D55, daylight, fine weather, flash, B
        21 | 19 | 10 => 6500.0,        // D65, C, cloudy
        22 | 11 => 7500.0,             // D75, shade
        12 => 6400.0,                  // daylight fluorescent, 5700-7100 K
        13 => 5050.0,                  // day white fluorescent, 4600-5500 K
        14 | 2 => 4150.0,              // cool white fluorescent, 3800-4500 K
        15 => 3525.0,                  // white fluorescent, 3250-3800 K
        16 => 2925.0,                  // warm white fluorescent, 2600-3250 K
        _ => return None,
    })
}

/// One calibration a DNG carries: the matrices measured under one illuminant.
#[derive(Debug, Clone, PartialEq)]
pub struct DngCalibration {
    /// EXIF `LightSource` code of the calibration illuminant.
    pub illuminant: u16,
    /// `ColorMatrixN`: XYZ to *reference* camera RGB.
    pub color_matrix: Mat3,
    /// `ForwardMatrixN`: white-balanced reference camera RGB to XYZ (D50).
    pub forward_matrix: Option<Mat3>,
    /// `CameraCalibrationN`: reference camera to this particular unit.
    pub camera_calibration: Option<Mat3>,
}

/// Everything a DNG says about its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct DngColour {
    /// One or two calibrations, in file order.
    pub calibrations: Vec<DngCalibration>,
    /// `AnalogBalance`, the gain the camera applied before digitising.
    pub analog_balance: [f32; 3],
}

fn diagonal(d: [f32; 3]) -> Mat3 {
    [[d[0], 0.0, 0.0], [0.0, d[1], 0.0], [0.0, 0.0, d[2]]]
}

fn blend(a: &Mat3, b: &Mat3, weight_a: f32) -> Mat3 {
    let mut out = *a;
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a[r][c] * weight_a + b[r][c] * (1.0 - weight_a);
        }
    }
    out
}

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The calibration at one colour temperature: interpolated when there are two.
struct AtTemperature {
    /// `AB * CC * CM`: XYZ to this unit's camera RGB.
    xyz_to_camera: Mat3,
    /// `AB * CC`: reference camera to this unit's camera RGB.
    ab_cc: Mat3,
    forward: Option<Mat3>,
}

impl DngColour {
    /// The weight of the first calibration at `temperature`, per the DNG
    /// specification: linear in inverse temperature between the two
    /// calibration illuminants, and clamped outside them. `None` when there is
    /// nothing to interpolate between.
    fn first_weight(&self, temperature: f32) -> Option<f32> {
        let [a, b] = [self.calibrations.first()?, self.calibrations.get(1)?];
        let (ta, tb) = (
            illuminant_temperature(a.illuminant)?,
            illuminant_temperature(b.illuminant)?,
        );
        if (ta - tb).abs() < 1.0 {
            return None;
        }
        let inverse = |t: f32| 1.0 / t.max(1.0);
        let w = (inverse(temperature) - inverse(tb)) / (inverse(ta) - inverse(tb));
        Some(w.clamp(0.0, 1.0))
    }

    /// The one calibration to use when the two cannot be interpolated: the one
    /// whose illuminant is nearest daylight, or the first.
    fn single(&self) -> &DngCalibration {
        self.calibrations
            .iter()
            .min_by_key(|c| {
                illuminant_temperature(c.illuminant)
                    .map(|t| (1.0e6 / t - 1.0e6 / 6500.0).abs() as i64)
                    .unwrap_or(i64::MAX)
            })
            .unwrap_or(&self.calibrations[0])
    }

    fn at(&self, temperature: f32) -> AtTemperature {
        let ab = diagonal(self.analog_balance);
        let resolve = |c: &DngCalibration| {
            let cc = c.camera_calibration.unwrap_or(IDENTITY);
            (c.color_matrix, cc, c.forward_matrix)
        };
        let (cm, cc, forward) = match self.first_weight(temperature) {
            Some(w) => {
                let (a, b) = (&self.calibrations[0], &self.calibrations[1]);
                let ((cm_a, cc_a, fm_a), (cm_b, cc_b, fm_b)) = (resolve(a), resolve(b));
                let forward = match (fm_a, fm_b) {
                    (Some(x), Some(y)) => Some(blend(&x, &y, w)),
                    // One forward matrix alone is not interpolable; the
                    // specification then uses whichever is present.
                    (x, y) => x.or(y),
                };
                (blend(&cm_a, &cm_b, w), blend(&cc_a, &cc_b, w), forward)
            }
            None => resolve(self.single()),
        };
        let ab_cc = color::multiply(ab, cc);
        AtTemperature {
            xyz_to_camera: color::multiply(ab_cc, cm),
            ab_cc,
            forward,
        }
    }

    /// The white chromaticity a camera neutral describes. Iterative, because
    /// the matrix that converts it depends on the temperature it converts to:
    /// the DNG SDK's own method, starting from D50.
    pub fn neutral_to_xy(&self, neutral: [f32; 3]) -> [f32; 2] {
        let mut xy = xyz_to_xy(D50).expect("D50 has a chromaticity");
        for pass in 0..30 {
            let m = self.at(temperature(xy)).xyz_to_camera;
            let Some(next) = invert3(m).and_then(|inv| xyz_to_xy(color::apply(inv, neutral)))
            else {
                return xy;
            };
            if (next[0] - xy[0]).abs() + (next[1] - xy[1]).abs() < 1e-7 {
                return next;
            }
            // Oscillating near the end: settle between the last two.
            xy = if pass == 29 {
                [(xy[0] + next[0]) * 0.5, (xy[1] + next[1]) * 0.5]
            } else {
                next
            };
        }
        xy
    }

    /// The camera neutral a white chromaticity would produce, green = 1.
    pub fn xy_to_neutral(&self, xy: [f32; 2]) -> Option<[f32; 3]> {
        let n = color::apply(self.at(temperature(xy)).xyz_to_camera, xy_to_xyz(xy));
        (n.iter().all(|v| v.is_finite() && *v > 0.0) && n[1] > 0.0)
            .then(|| [n[0] / n[1], 1.0, n[2] / n[1]])
    }

    /// Camera RGB **after the as-shot white balance** to CIE XYZ (D65), for a
    /// frame whose camera neutral is `neutral`.
    ///
    /// Balanced input is what the mosaic pass produces: it divides by the
    /// neutral before demosaicing, so a grey subject arrives as equal
    /// channels. Per the specification, that goes to XYZ through the forward
    /// matrix when there is one; otherwise through the inverse colour matrix,
    /// then a Bradford adaptation from the scene's white. Either way white
    /// lands on D65 — the pipeline's own normalisation then only rescales.
    pub fn camera_to_xyz(&self, neutral: [f32; 3]) -> Option<Mat3> {
        let xy = self.neutral_to_xy(neutral);
        let at = self.at(temperature(xy));
        let unbalance = diagonal(neutral);
        let m = match at.forward {
            Some(fm) => {
                // CameraToXYZ_D50 = FM * D * Inverse(AB * CC), where D makes
                // the neutral, taken back to the reference camera, unity.
                let to_reference = invert3(at.ab_cc)?;
                let reference_neutral = color::apply(to_reference, neutral);
                if reference_neutral
                    .iter()
                    .any(|v| !v.is_finite() || *v <= 0.0)
                {
                    return None;
                }
                let d = diagonal(reference_neutral.map(|v| 1.0 / v));
                let d50 = color::multiply(fm, color::multiply(d, to_reference));
                color::multiply(bradford(D50, D65), d50)
            }
            None => {
                let camera_to_xyz = invert3(at.xyz_to_camera)?;
                color::multiply(bradford(xy_to_xyz(xy), D65), camera_to_xyz)
            }
        };
        let m = color::multiply(m, unbalance);
        m.iter().flatten().all(|v| v.is_finite()).then_some(m)
    }
}

/// The camera neutral daylight produces through an XYZ → camera matrix, green
/// = 1: the white balance to fall back on when a file records none.
pub fn daylight_neutral(xyz_to_camera: Mat3) -> Option<[f32; 3]> {
    let n = color::apply(xyz_to_camera, D65);
    (n.iter().all(|v| v.is_finite() && *v > 0.0)).then(|| [n[0] / n[1], 1.0, n[2] / n[1]])
}

/// A typical modern Bayer sensor's XYZ → camera matrix (D65), for a camera
/// neither LibRaw's table nor the file itself describes — one newer than the
/// LibRaw in the build, most often.
///
/// The mean of LibRaw's matrices for the six fixture bodies (Canon R6,
/// Fujifilm X-T3, Nikon Z 6, Olympus E-M10 III, Panasonic G9, Sony a6400),
/// each scaled so its green channel's response to daylight is 1. Measured on
/// sky, foliage, skin and three saturated patches rendered through each
/// body's own matrix and through this one: mostly within 10/255, at worst
/// 44/255 (a yellow, on the Canon). Treating camera RGB as sRGB, which is
/// dcraw's fallback, is off by 25 to 95. Approximate, but a photo that opens
/// slightly off beats one that does not open at all — which is what an
/// absent matrix used to mean here.
pub const GENERIC_XYZ_TO_CAMERA: Mat3 = [
    [0.8647, -0.2976, -0.0785],
    [-0.3901, 1.1321, 0.2192],
    [-0.0757, 0.1592, 0.5533],
];

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    /// Published chromaticities and their accepted colour temperatures.
    #[test]
    fn robertson_places_the_standard_illuminants() {
        for (xy, want, tol) in [
            ([0.312_7, 0.329_0], 6504.0, 15.0), // D65
            ([0.345_7, 0.358_5], 5003.0, 15.0), // D50
            ([0.447_6, 0.407_5], 2856.0, 15.0), // A
            ([0.299_0, 0.315_0], 7504.0, 30.0), // D75
        ] {
            let t = temperature(xy);
            assert!(close(t, want, tol), "{xy:?}: {t} K, expected {want} K");
        }
    }

    #[test]
    fn bradford_maps_one_white_onto_the_other() {
        let a = xy_to_xyz([0.447_6, 0.407_5]);
        let out = color::apply(bradford(a, D65), a);
        for (o, w) in out.iter().zip(D65) {
            assert!(close(*o, w, 1e-4), "{out:?}");
        }
        // And back: the adaptation is invertible and neutral-preserving.
        let identity = color::multiply(bradford(D65, a), bradford(a, D65));
        for (r, row) in identity.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!(close(*v, if r == c { 1.0 } else { 0.0 }, 1e-4));
            }
        }
    }

    /// The published ILCE-6400 matrix (D65), as a stand-in for a DNG's
    /// daylight calibration.
    const D65_CM: Mat3 = [
        [0.7657, -0.2847, -0.0607],
        [-0.4083, 1.1966, 0.2389],
        [-0.0684, 0.1418, 0.5844],
    ];
    /// A tungsten calibration for the same sensor: redder and less blue
    /// sensitive, as a StdA matrix is.
    const A_CM: Mat3 = [
        [0.8100, -0.3500, -0.0400],
        [-0.3900, 1.1600, 0.2700],
        [-0.0300, 0.1100, 0.7200],
    ];

    fn two_matrix_dng() -> DngColour {
        DngColour {
            calibrations: vec![
                DngCalibration {
                    illuminant: 17,
                    color_matrix: A_CM,
                    forward_matrix: None,
                    camera_calibration: None,
                },
                DngCalibration {
                    illuminant: 21,
                    color_matrix: D65_CM,
                    forward_matrix: None,
                    camera_calibration: None,
                },
            ],
            analog_balance: [1.0; 3],
        }
    }

    /// Whatever the illuminant, the as-shot neutral must come out as D65
    /// white: that is what white balance means.
    #[test]
    fn the_neutral_lands_on_white_at_any_temperature() {
        let dng = two_matrix_dng();
        for xy in [[0.312_7, 0.329_0], [0.447_6, 0.407_5], [0.38, 0.38]] {
            let neutral = dng.xy_to_neutral(xy).unwrap();
            let m = dng.camera_to_xyz(neutral).unwrap();
            // Balanced camera white is (1, 1, 1).
            let white = color::apply(m, [1.0, 1.0, 1.0]);
            let scale = white[1];
            for (o, w) in white.iter().zip(D65) {
                assert!(close(o / scale, w, 2e-3), "{xy:?} -> {white:?}");
            }
        }
    }

    /// The iteration recovers the white a neutral came from.
    #[test]
    fn a_neutral_round_trips_to_its_chromaticity() {
        let dng = two_matrix_dng();
        for xy in [[0.312_7, 0.329_0], [0.447_6, 0.407_5], [0.40, 0.39]] {
            let back = dng.neutral_to_xy(dng.xy_to_neutral(xy).unwrap());
            assert!(
                close(back[0], xy[0], 2e-4) && close(back[1], xy[1], 2e-4),
                "{xy:?} came back as {back:?}"
            );
        }
    }

    /// Under daylight the D65 calibration is the one used; under tungsten the
    /// StdA one; between them, a blend — the whole point of carrying two.
    #[test]
    fn interpolation_follows_the_illuminant() {
        let dng = two_matrix_dng();
        assert_eq!(dng.first_weight(6500.0), Some(0.0));
        assert_eq!(dng.first_weight(2850.0), Some(1.0));
        assert_eq!(dng.first_weight(9000.0), Some(0.0), "clamped past D65");
        let mid = dng.first_weight(4000.0).unwrap();
        assert!(mid > 0.2 && mid < 0.8, "{mid}");
    }

    /// With only a daylight matrix this reduces to the inverse matrix with a
    /// Bradford adaptation — and a saturated colour under daylight survives
    /// it, as it survives the camera-space method the other raws use.
    #[test]
    fn a_single_daylight_matrix_keeps_saturated_colour() {
        let dng = DngColour {
            calibrations: vec![DngCalibration {
                illuminant: 21,
                color_matrix: D65_CM,
                forward_matrix: None,
                camera_calibration: None,
            }],
            analog_balance: [1.0; 3],
        };
        let neutral = daylight_neutral(D65_CM).unwrap();
        let m = dng.camera_to_xyz(neutral).unwrap();
        let sky = [0.18, 0.21, 0.40]; // XYZ of a clear sky, roughly
        let camera = color::apply(D65_CM, sky);
        let balanced = [
            camera[0] / neutral[0],
            camera[1] / neutral[1],
            camera[2] / neutral[2],
        ];
        let out = color::apply(m, balanced);
        let k = out[1] / sky[1];
        for (o, s) in out.iter().zip(sky) {
            assert!(close(o / k, s, 2e-3), "{out:?} vs {sky:?}");
        }
    }

    /// A forward matrix maps balanced camera white to D50 by definition; the
    /// result must still land on D65, and must be the forward matrix's own
    /// mapping rather than the colour matrix's.
    #[test]
    fn a_forward_matrix_is_used_when_present() {
        // Built so it maps (1, 1, 1) to D50, as the specification requires.
        let fm_raw = [[0.60, 0.25, 0.10], [0.25, 0.70, 0.05], [0.05, 0.05, 0.70]];
        let sums: Vec<f32> = fm_raw.iter().map(|r| r.iter().sum()).collect();
        let fm = std::array::from_fn(|r| fm_raw[r].map(|v| v / sums[r] * D50[r]));
        let dng = DngColour {
            calibrations: vec![DngCalibration {
                illuminant: 21,
                color_matrix: D65_CM,
                forward_matrix: Some(fm),
                camera_calibration: None,
            }],
            analog_balance: [1.0; 3],
        };
        let neutral = daylight_neutral(D65_CM).unwrap();
        let m = dng.camera_to_xyz(neutral).unwrap();
        let white = color::apply(m, [1.0, 1.0, 1.0]);
        for (o, w) in white.iter().zip(D65) {
            assert!(close(*o / white[1], w, 2e-3), "{white:?}");
        }
        // Pure balanced red goes where Bradford(D50 -> D65) * FM sends it.
        let want = color::apply(color::multiply(bradford(D50, D65), fm), [1.0, 0.0, 0.0]);
        let got = color::apply(m, [1.0, 0.0, 0.0]);
        for (g, w) in got.iter().zip(want) {
            assert!(close(*g, w, 1e-3), "{got:?} vs {want:?}");
        }
    }

    /// A camera calibration and analog balance describe *this* unit against
    /// the reference; a frame from a unit whose red runs 10% hot must render
    /// the same as the reference unit's frame of the same scene.
    #[test]
    fn camera_calibration_and_analog_balance_are_undone() {
        let reference = DngColour {
            calibrations: vec![DngCalibration {
                illuminant: 21,
                color_matrix: D65_CM,
                forward_matrix: None,
                camera_calibration: None,
            }],
            analog_balance: [1.0; 3],
        };
        let hot = DngColour {
            calibrations: vec![DngCalibration {
                illuminant: 21,
                color_matrix: D65_CM,
                forward_matrix: None,
                camera_calibration: Some([[1.1, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            }],
            analog_balance: [1.0, 1.0, 0.9],
        };
        let scene = [0.3, 0.2, 0.15];
        let render = |dng: &DngColour, gain: [f32; 3]| {
            let raw = color::apply(D65_CM, scene);
            let raw = [raw[0] * gain[0], raw[1] * gain[1], raw[2] * gain[2]];
            let neutral = {
                let n = color::apply(D65_CM, D65);
                [n[0] * gain[0], n[1] * gain[1], n[2] * gain[2]]
            };
            let neutral = [neutral[0] / neutral[1], 1.0, neutral[2] / neutral[1]];
            let m = dng.camera_to_xyz(neutral).unwrap();
            let balanced = [
                raw[0] / neutral[0],
                raw[1] / neutral[1],
                raw[2] / neutral[2],
            ];
            let out = color::apply(m, balanced);
            let white = color::apply(m, [1.0; 3]);
            out.map(|v| v / white[1])
        };
        let a = render(&reference, [1.0; 3]);
        let b = render(&hot, [1.1, 1.0, 0.9]);
        for (x, y) in a.iter().zip(b) {
            assert!(close(*x, y, 1e-3), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn daylight_neutral_is_the_cameras_response_to_d65() {
        let n = daylight_neutral(D65_CM).unwrap();
        let direct = color::apply(D65_CM, D65);
        assert!(close(n[0], direct[0] / direct[1], 1e-6));
        assert!(close(n[2], direct[2] / direct[1], 1e-6));
        assert_eq!(n[1], 1.0);
    }

    #[test]
    fn the_generic_matrix_is_a_plausible_camera() {
        let n = daylight_neutral(GENERIC_XYZ_TO_CAMERA).expect("positive daylight response");
        // Daylight white balance for a Bayer sensor: red and blue need more
        // gain than green, never less.
        assert!(n[0] < 1.0 && n[2] < 1.0, "{n:?}");
        assert!(invert3(GENERIC_XYZ_TO_CAMERA).is_some());
    }

    #[test]
    fn unknown_illuminants_have_no_temperature() {
        assert_eq!(illuminant_temperature(0), None);
        assert_eq!(illuminant_temperature(255), None);
        assert_eq!(illuminant_temperature(0xffff), None);
        assert_eq!(illuminant_temperature(21), Some(6500.0));
    }
}
