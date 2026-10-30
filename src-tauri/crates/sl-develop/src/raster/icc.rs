//! What colour a rendered photo's numbers mean, from its ICC profile.
//!
//! A JPEG's (200, 60, 30) is only a colour once you know the space. Most files
//! are sRGB and say so or say nothing, but an editor's export is often Adobe
//! RGB, an iPhone shoots Display P3, and treating either as sRGB shows it
//! washed out: the grid, the loupe and Develop all lose saturation the file
//! actually has.
//!
//! This reads **matrix/shaper** profiles, which is what those are: three
//! primaries (`rXYZ`, `gXYZ`, `bXYZ`) and a tone curve per channel (`rTRC`...),
//! as `curv` (a gamma or a table) or `para` (the parametric families). Table-
//! based (`A2B0`) profiles are rare for photographs and are read as sRGB.
//!
//! The primaries in a profile are adapted to the ICC's D50 connection space;
//! Bradford takes them back to D65, where sRGB and the develop pipeline live.

use crate::gpu::color::{self, Mat3};

/// One channel's tone curve: encoded value (0..=1) to linear light.
#[derive(Debug, Clone, PartialEq)]
enum Curve {
    Gamma(f32),
    Table(Vec<f32>),
    /// ICC `para` function type 0-4, parameters `[g, a, b, c, d, e, f]`.
    Parametric(u16, [f32; 7]),
}

impl Curve {
    fn eval(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Curve::Gamma(g) => x.powf(*g),
            Curve::Table(t) => {
                let pos = x * (t.len() - 1) as f32;
                let i = (pos as usize).min(t.len() - 2);
                let f = pos - i as f32;
                t[i] + (t[i + 1] - t[i]) * f
            }
            Curve::Parametric(kind, [g, a, b, c, d, e, f]) => match kind {
                0 => x.powf(*g),
                1 => {
                    if x >= -b / a {
                        (a * x + b).powf(*g)
                    } else {
                        0.0
                    }
                }
                2 => {
                    if x >= -b / a {
                        (a * x + b).powf(*g) + c
                    } else {
                        *c
                    }
                }
                3 => {
                    if x >= *d {
                        (a * x + b).powf(*g)
                    } else {
                        c * x
                    }
                }
                _ => {
                    if x >= *d {
                        (a * x + b).powf(*g) + e
                    } else {
                        c * x + f
                    }
                }
            },
        }
    }
}

/// A photo's colour space: its primaries and its tone curves.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Linear RGB to CIE XYZ, D65.
    pub to_xyz: Mat3,
    curves: [Curve; 3],
}

/// Bradford chromatic adaptation, D50 to D65.
const D50_TO_D65: Mat3 = [
    [0.955_576_6, -0.023_039_3, 0.063_163_6],
    [-0.028_289_5, 1.009_941_6, 0.021_007_7],
    [0.012_298_2, -0.020_483, 1.329_909_8],
];

/// The sRGB curve in ICC's parametric form (type 3).
const SRGB_CURVE: [f32; 7] = [
    2.4,
    1.0 / 1.055,
    0.055 / 1.055,
    1.0 / 12.92,
    0.04045,
    0.0,
    0.0,
];

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn s15f16(b: &[u8], at: usize) -> Option<f32> {
    Some(be_u32(b, at)? as i32 as f32 / 65536.0)
}

impl Profile {
    /// sRGB, which is also what a file without a profile is taken to be.
    pub fn srgb() -> Profile {
        let curve = Curve::Parametric(3, SRGB_CURVE);
        Profile {
            to_xyz: crate::raster::SRGB_TO_XYZ,
            curves: [curve.clone(), curve.clone(), curve],
        }
    }

    /// Adobe RGB (1998), by its specification: what a camera set to Adobe
    /// RGB encodes its JPEGs and raw previews in, without embedding a profile
    /// to say so (see `exif_read::ColourSpace`).
    pub fn adobe_rgb() -> Profile {
        let curve = Curve::Gamma(563.0 / 256.0);
        Profile {
            to_xyz: [
                [0.576_730_9, 0.185_554, 0.188_185_2],
                [0.297_376_9, 0.627_349_1, 0.075_274_1],
                [0.027_034_3, 0.070_687_2, 0.991_108_5],
            ],
            curves: [curve.clone(), curve.clone(), curve],
        }
    }

    /// Reads a matrix/shaper RGB profile; `None` for anything else.
    pub fn parse(icc: &[u8]) -> Option<Profile> {
        if icc.get(36..40)? != b"acsp" || icc.get(16..20)? != b"RGB " {
            return None;
        }
        let count = be_u32(icc, 128)? as usize;
        let tag = |sig: &[u8; 4]| -> Option<&[u8]> {
            (0..count.min(256)).find_map(|i| {
                let at = 132 + i * 12;
                (icc.get(at..at + 4)? == sig).then_some(())?;
                let (off, len) = (be_u32(icc, at + 4)? as usize, be_u32(icc, at + 8)? as usize);
                icc.get(off..off.checked_add(len)?)
            })
        };
        let xyz = |sig: &[u8; 4]| -> Option<[f32; 3]> {
            let t = tag(sig)?;
            (t.get(0..4)? == b"XYZ ").then_some(())?;
            Some([s15f16(t, 8)?, s15f16(t, 12)?, s15f16(t, 16)?])
        };
        let (r, g, b) = (xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?);
        // Columns are the primaries.
        let d50 = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
        let curves = [
            curve(tag(b"rTRC")?)?,
            curve(tag(b"gTRC")?)?,
            curve(tag(b"bTRC")?)?,
        ];
        // A curve that brightens mid-grey is backwards: some software writes
        // "sRGB" profiles with the encoding table where the decoding one
        // belongs. Honoured, it turns a photo's shadows nearly white, while the
        // author saw it as plain sRGB; so such a profile is not believed.
        if curves.iter().any(|c| c.eval(0.5) >= 0.5) {
            return None;
        }
        Some(Profile {
            to_xyz: color::multiply(D50_TO_D65, d50),
            curves,
        })
    }

    /// Close enough to sRGB that treating it as sRGB changes nothing visible:
    /// the usual case, and the one that lets a JPEG be shown untouched.
    pub fn is_srgb(&self) -> bool {
        let srgb = Profile::srgb();
        let primaries = self
            .to_xyz
            .iter()
            .flatten()
            .zip(srgb.to_xyz.iter().flatten())
            .all(|(a, b)| (a - b).abs() < 0.003);
        primaries
            && (0..3).all(|c| {
                (1..16).all(|i| {
                    let x = i as f32 / 16.0;
                    (self.linear(c, x) - srgb.linear(c, x)).abs() < 0.004
                })
            })
    }

    /// Channel `c`'s encoded value as linear light.
    pub fn linear(&self, c: usize, encoded: f32) -> f32 {
        self.curves[c].eval(encoded)
    }

    /// [`Self::linear`] for every 8-bit value of each channel.
    pub fn lut8(&self) -> [[f32; 256]; 3] {
        std::array::from_fn(|c| std::array::from_fn(|i| self.linear(c, i as f32 / 255.0)))
    }
}

fn curve(t: &[u8]) -> Option<Curve> {
    match t.get(0..4)? {
        b"curv" => {
            let n = be_u32(t, 8)? as usize;
            match n {
                0 => Some(Curve::Gamma(1.0)),
                1 => Some(Curve::Gamma(be_u16(t, 12)? as f32 / 256.0)),
                _ => {
                    let table: Option<Vec<f32>> = (0..n)
                        .map(|i| Some(be_u16(t, 12 + 2 * i)? as f32 / 65535.0))
                        .collect();
                    Some(Curve::Table(table?))
                }
            }
        }
        b"para" => {
            let kind = be_u16(t, 8)?;
            let n = [1, 3, 4, 5, 7].get(kind as usize).copied()?;
            let mut p = [0.0f32; 7];
            for (i, v) in p.iter_mut().enumerate().take(n) {
                *v = s15f16(t, 12 + 4 * i)?;
            }
            Some(Curve::Parametric(kind, p))
        }
        _ => None,
    }
}

/// Builds a minimal matrix/shaper profile: for tests, which cannot ship real
/// profiles from other people's software.
#[cfg(test)]
pub fn build_for_test(primaries_d50: [[f32; 3]; 3], trc: &[u8]) -> Vec<u8> {
    let fixed = |v: f32| ((v * 65536.0).round() as i32).to_be_bytes();
    let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (sig, p) in [b"rXYZ", b"gXYZ", b"bXYZ"].iter().zip(primaries_d50) {
        let mut t = b"XYZ \0\0\0\0".to_vec();
        for v in p {
            t.extend_from_slice(&fixed(v));
        }
        tags.push((**sig, t));
    }
    for sig in [b"rTRC", b"gTRC", b"bTRC"] {
        tags.push((*sig, trc.to_vec()));
    }
    let mut out = vec![0u8; 128];
    out[16..20].copy_from_slice(b"RGB ");
    out[36..40].copy_from_slice(b"acsp");
    out.extend_from_slice(&(tags.len() as u32).to_be_bytes());
    let mut offset = 128 + 4 + 12 * tags.len();
    let mut body = Vec::new();
    for (sig, data) in &tags {
        out.extend_from_slice(sig);
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        offset += data.len();
    }
    out.extend_from_slice(&body);
    let len = out.len() as u32;
    out[0..4].copy_from_slice(&len.to_be_bytes());
    out
}

/// A `curv` tag holding one gamma.
#[cfg(test)]
pub fn gamma_curve_for_test(gamma: f32) -> Vec<u8> {
    let mut t = b"curv\0\0\0\0".to_vec();
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&((gamma * 256.0).round() as u16).to_be_bytes());
    t
}

/// Adobe RGB (1998)'s primaries as its own ICC profile states them (D50).
#[cfg(test)]
pub const ADOBE_RGB_D50: [[f32; 3]; 3] = [
    [0.609_741, 0.311_111, 0.019_470],
    [0.205_276, 0.625_671, 0.060_867],
    [0.149_185, 0.063_217, 0.744_568],
];

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Mat3, b: Mat3, tol: f32) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() < tol)
    }

    #[test]
    fn adobe_rgb_reads_as_its_published_matrix() {
        let icc = build_for_test(ADOBE_RGB_D50, &gamma_curve_for_test(563.0 / 256.0));
        let p = Profile::parse(&icc).expect("parses");
        // The Adobe RGB (1998) specification's own RGB to XYZ (D65).
        let published = [
            [0.576_67, 0.185_56, 0.188_23],
            [0.297_34, 0.627_36, 0.075_29],
            [0.027_03, 0.070_69, 0.991_34],
        ];
        assert!(near(p.to_xyz, published, 0.002), "{:?}", p.to_xyz);
        assert!(!p.is_srgb());
        assert!((p.linear(1, 0.5) - 0.5f32.powf(2.199_218_8)).abs() < 1e-5);
    }

    #[test]
    fn an_srgb_profile_is_recognised_in_either_curve_form() {
        // sRGB's primaries as an ICC profile states them (D50).
        let d50 = [
            [0.436_066, 0.222_488, 0.013_916],
            [0.385_147, 0.716_873, 0.097_076],
            [0.143_066, 0.060_608, 0.714_096],
        ];
        // v4 style: the parametric curve.
        let mut para = b"para\0\0\0\0".to_vec();
        para.extend_from_slice(&3u16.to_be_bytes());
        para.extend_from_slice(&[0, 0]);
        for v in &SRGB_CURVE[..5] {
            para.extend_from_slice(&((v * 65536.0).round() as i32).to_be_bytes());
        }
        let v4 = Profile::parse(&build_for_test(d50, &para)).unwrap();
        assert!(v4.is_srgb(), "{:?}", v4.to_xyz);

        // v2 style: a 1024-entry table.
        let mut curv = b"curv\0\0\0\0".to_vec();
        curv.extend_from_slice(&1024u32.to_be_bytes());
        let srgb = Profile::srgb();
        for i in 0..1024 {
            let v = srgb.linear(0, i as f32 / 1023.0);
            curv.extend_from_slice(&((v * 65535.0).round() as u16).to_be_bytes());
        }
        assert!(Profile::parse(&build_for_test(d50, &curv))
            .unwrap()
            .is_srgb());
    }

    /// Found on a real JPEG: sRGB primaries with the sRGB curve stored the
    /// wrong way round. Believing it turned dark greens nearly white.
    #[test]
    fn a_backwards_curve_is_not_believed() {
        let mut curv = b"curv\0\0\0\0".to_vec();
        curv.extend_from_slice(&1024u32.to_be_bytes());
        for i in 0..1024 {
            let x = i as f32 / 1023.0;
            let encoded = if x <= 0.003_130_8 {
                x * 12.92
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            };
            curv.extend_from_slice(&((encoded * 65535.0).round() as u16).to_be_bytes());
        }
        assert_eq!(Profile::parse(&build_for_test(ADOBE_RGB_D50, &curv)), None);
    }

    #[test]
    fn not_a_matrix_profile_is_none() {
        assert_eq!(Profile::parse(b"nonsense"), None);
        let mut cmyk = build_for_test(ADOBE_RGB_D50, &gamma_curve_for_test(2.2));
        cmyk[16..20].copy_from_slice(b"CMYK");
        assert_eq!(Profile::parse(&cmyk), None);
    }
}
