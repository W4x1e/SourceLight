//! Oklab and its polar form, OkLCh: the space the colour mixer and colour-range
//! masks read hue in.
//!
//! They used HSV on the working space. HSV's hue is an angle between the
//! primaries of whatever RGB it is taken in, and Rec.2020's are far from the
//! colours people name: its "yellow" sector holds most of what an sRGB screen
//! calls orange, and a sixty-degree step is a small change of colour in one
//! place and a large one in another. Its saturation and value are no better —
//! desaturating a blue in HSV lifts it most of a stop, since value is the
//! largest channel and blue is dark. Oklab (Björn Ottosson, 2020) is built so
//! that equal steps look equal: lightness, chroma and hue move independently,
//! and its hue angles fall where the named colours do.
//!
//! **Mirrored in `develop.wgsl`**, which does the per-pixel work; the matrices
//! here are the source, and a test checks the shader carries the same numbers.

use crate::gpu::color::{self, Mat3};

/// Linear Rec.2020 (D65) to Oklab's cone-like LMS: Ottosson's M1 composed with
/// Rec.2020 to XYZ, each row then scaled to sum to exactly one (a change of
/// under 5e-4), so the working space's white is (1, 1, 1) and every grey has
/// no chroma and so no hue.
pub const REC2020_TO_LMS: Mat3 = [
    [0.616756, 0.360198, 0.023046],
    [0.265133, 0.635839, 0.099028],
    [0.100103, 0.203907, 0.695991],
];

/// Oklab's LMS back to linear Rec.2020: the inverse of [`REC2020_TO_LMS`].
pub const LMS_TO_REC2020: Mat3 = [
    [2.139907, -1.24639, 0.106483],
    [-0.884736, 2.163231, -0.278495],
    [-0.048574, -0.454503, 1.503077],
];

/// Cube-rooted LMS to Oklab (L, a, b): Ottosson's M2.
pub const LMS_TO_LAB: Mat3 = [
    [0.210454, 0.793618, -0.004072],
    [1.977998, -2.428592, 0.450594],
    [0.025904, 0.782772, -0.808676],
];

/// Oklab (L, a, b) to cube-rooted LMS: the inverse of [`LMS_TO_LAB`].
pub const LAB_TO_LMS: Mat3 = [
    [1.000000, 0.396338, 0.215804],
    [1.000000, -0.105561, -0.063854],
    [1.000000, -0.089484, -1.291486],
];

/// Oklab of linear Rec.2020.
pub fn lab(rgb: [f32; 3]) -> [f32; 3] {
    let lms = color::apply(REC2020_TO_LMS, rgb).map(f32::cbrt);
    color::apply(LMS_TO_LAB, lms)
}

/// Linear Rec.2020 of Oklab.
pub fn rgb(lab: [f32; 3]) -> [f32; 3] {
    let lms = color::apply(LAB_TO_LMS, lab).map(|v| v * v * v);
    color::apply(LMS_TO_REC2020, lms)
}

/// OkLCh of linear Rec.2020: lightness, chroma, and hue in degrees 0..360.
pub fn lch(rgb: [f32; 3]) -> [f32; 3] {
    let [l, a, b] = lab(rgb);
    [l, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0)]
}

/// Linear Rec.2020 of OkLCh.
pub fn from_lch(lch: [f32; 3]) -> [f32; 3] {
    let [l, c, h] = lch;
    let h = h.to_radians();
    rgb([l, c * h.cos(), c * h.sin()])
}

/// The OkLCh hue of a display colour given as sRGB 0..1: how the mixer places
/// the colours its bands are named after.
pub fn srgb_hue(srgb: [f32; 3]) -> f32 {
    let linear = srgb.map(crate::raster::srgb_to_linear);
    let rec2020 = color::apply(
        crate::raw::decoder::invert3(color::REC2020_TO_SRGB).expect("the output matrix inverts"),
        linear,
    );
    lch(rec2020)[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::decoder::invert3;

    /// Ottosson's M1, XYZ (D65) to LMS.
    const XYZ_TO_LMS: Mat3 = [
        [0.818_933, 0.361_866_74, -0.128_859_71],
        [0.032_984_544, 0.929_311_9, 0.036_145_64],
        [0.048_200_3, 0.264_366_27, 0.633_851_7],
    ];

    fn close(a: Mat3, b: Mat3, tol: f32) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() < tol)
    }

    /// The composed matrices are what they say: M1 after Rec.2020 to XYZ, and
    /// each inverse the inverse of its pair.
    #[test]
    fn the_matrices_compose_and_invert() {
        let to_xyz = invert3(color::XYZ_TO_REC2020).unwrap();
        assert!(close(
            REC2020_TO_LMS,
            color::multiply(XYZ_TO_LMS, to_xyz),
            5e-4
        ));
        for row in REC2020_TO_LMS {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 2e-6, "{row:?}");
        }
        assert!(close(
            LMS_TO_REC2020,
            invert3(REC2020_TO_LMS).unwrap(),
            5e-5
        ));
        assert!(close(LAB_TO_LMS, invert3(LMS_TO_LAB).unwrap(), 5e-5));
    }

    /// White is L 1, no chroma; the round trip is exact to float precision.
    #[test]
    fn white_is_white_and_round_trips() {
        let [l, c, _] = lch([1.0; 3]);
        assert!((l - 1.0).abs() < 1e-4 && c < 2e-5, "{l} {c}");
        for rgb_in in [[0.2, 0.5, 0.1], [0.9, 0.05, 0.3], [0.01, 0.02, 0.4]] {
            let back = from_lch(lch(rgb_in));
            for (a, b) in rgb_in.iter().zip(back) {
                assert!((a - b).abs() < 1e-4, "{rgb_in:?} -> {back:?}");
            }
        }
    }

    /// Ottosson's published hues for the sRGB primaries, which is the test
    /// that the whole chain, working space included, is Oklab.
    #[test]
    fn srgb_primaries_land_where_oklab_puts_them() {
        for (srgb, hue) in [
            ([1.0, 0.0, 0.0], 29.2),
            ([0.0, 1.0, 0.0], 142.5),
            ([0.0, 0.0, 1.0], 264.1),
        ] {
            let got = srgb_hue(srgb);
            assert!((got - hue).abs() < 0.5, "{srgb:?} at {got}, expected {hue}");
        }
    }

    /// The shader carries the same matrices.
    #[test]
    fn the_shader_mirrors_the_matrices() {
        let shader = include_str!("../gpu/shaders/develop.wgsl");
        for m in [REC2020_TO_LMS, LMS_TO_REC2020, LMS_TO_LAB, LAB_TO_LMS] {
            for v in m.iter().flatten() {
                let text = format!("{v:.6}");
                assert!(shader.contains(&text), "develop.wgsl lacks {text}");
            }
        }
    }
}
