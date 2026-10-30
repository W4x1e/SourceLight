//! Colour space matrices.
//!
//! The working space is **linear Rec.2020**: wide enough to hold any camera's
//! gamut without clipping saturated colours on the way through, and better
//! behaved than ProPhoto, whose primaries are imaginary.

pub type Mat3 = [[f32; 3]; 3];

/// CIE XYZ (D65) to linear Rec.2020.
pub const XYZ_TO_REC2020: Mat3 = [
    [1.7166512, -0.3556708, -0.2533663],
    [-0.6666844, 1.6164812, 0.0157685],
    [0.0176399, -0.0427706, 0.9421031],
];

/// Linear Rec.2020 to linear sRGB. Values outside the sRGB gamut come out
/// negative or above one; clipping is the display encode's job, not this one's.
/// **Mirrored as a constant in `develop.wgsl`**, where the look LUT uses it.
pub const REC2020_TO_SRGB: Mat3 = [
    [1.6605, -0.5876, -0.0728],
    [-0.1246, 1.1329, -0.0083],
    [-0.0182, -0.1006, 1.1187],
];

/// CIE XYZ (D65) to linear Display P3 — DCI-P3 primaries on a D65 white, the
/// wide-gamut space of Apple displays and most recent phones and laptops.
/// The published values, verbatim, even where f32 cannot hold every digit.
#[allow(clippy::excessive_precision)]
pub const XYZ_TO_DISPLAY_P3: Mat3 = [
    [2.4934969, -0.9313836, -0.4027108],
    [-0.8294890, 1.7626641, 0.0236247],
    [0.0358458, -0.0761724, 0.9568845],
];

/// Linear Rec.2020 to linear Display P3, for a wide-gamut export.
///
/// Composed rather than a hand-typed table, so it cannot drift from the two
/// matrices it is made of. P3 shares sRGB's transfer curve, which is why this
/// matrix is the only thing a P3 render changes.
pub fn rec2020_to_display_p3() -> Mat3 {
    let rec2020_to_xyz =
        crate::raw::decoder::invert3(XYZ_TO_REC2020).expect("Rec.2020 matrix is invertible");
    multiply(XYZ_TO_DISPLAY_P3, rec2020_to_xyz)
}

/// CIE XYZ (D65) to linear Adobe RGB (1998).
#[allow(clippy::excessive_precision)]
pub const XYZ_TO_ADOBE_RGB: Mat3 = [
    [2.0413690, -0.5649464, -0.3446944],
    [-0.9692660, 1.8760108, 0.0415560],
    [0.0134474, -0.1183897, 1.0154096],
];

/// Linear Rec.2020 to linear Adobe RGB (1998), composed like P3's.
pub fn rec2020_to_adobe_rgb() -> Mat3 {
    let rec2020_to_xyz =
        crate::raw::decoder::invert3(XYZ_TO_REC2020).expect("Rec.2020 matrix is invertible");
    multiply(XYZ_TO_ADOBE_RGB, rec2020_to_xyz)
}

/// How the develop pass encodes linear light for the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    /// The piecewise sRGB curve — sRGB and Display P3.
    Srgb,
    /// Pure gamma 563/256 (≈2.2) — Adobe RGB (1998).
    Gamma22,
}

/// What a render is encoded into: the working → output matrix and the
/// transfer curve. The two travel together because a matrix without its curve
/// is a different colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Output {
    pub working_to_output: Mat3,
    pub transfer: Transfer,
    /// The output space's luminance weights, red then green (blue is the
    /// remainder — they sum to one). `gamut_map` pulls an out-of-gamut colour
    /// toward its own luminance, and that has to be measured in the space it
    /// is mapping into, or the compression shifts brightness and hue.
    pub luma_rg: [f32; 2],
    /// Perceptual rendering intent rather than relative colorimetric — see
    /// `perceptual_scale` in the develop shader.
    pub perceptual: bool,
}

impl Output {
    /// The on-screen canvas, thumbnails, and every sRGB export. Rec.709's
    /// published weights, exactly — so sRGB renders are unchanged to the bit.
    pub const SRGB: Output = Output {
        working_to_output: REC2020_TO_SRGB,
        transfer: Transfer::Srgb,
        luma_rg: [0.2126, 0.7152],
        perceptual: false,
    };

    /// An output with its luminance weights derived from its matrix.
    pub fn new(working_to_output: Mat3, transfer: Transfer) -> Output {
        Output {
            working_to_output,
            transfer,
            luma_rg: luma_weights(working_to_output),
            perceptual: false,
        }
    }

    /// The same output under a different rendering intent.
    pub fn with_intent(self, perceptual: bool) -> Output {
        Output { perceptual, ..self }
    }
}

/// The luminance weights of the space `working_to_output` maps into: the Y
/// row of that space's own RGB → XYZ matrix, found as Rec.2020 → XYZ after
/// undoing the output matrix.
pub fn luma_weights(working_to_output: Mat3) -> [f32; 2] {
    let rec2020_to_xyz =
        crate::raw::decoder::invert3(XYZ_TO_REC2020).expect("Rec.2020 matrix is invertible");
    let output_to_working =
        crate::raw::decoder::invert3(working_to_output).expect("output matrix is invertible");
    let output_to_xyz = multiply(rec2020_to_xyz, output_to_working);
    [output_to_xyz[1][0], output_to_xyz[1][1]]
}

pub fn multiply(a: Mat3, b: Mat3) -> Mat3 {
    let mut out = [[0.0f32; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

pub fn apply(m: Mat3, v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Camera RGB to the linear Rec.2020 working space.
///
/// **The white adaptation has to happen in camera space, not on the output.**
/// The obvious construction — compose `XYZ_TO_REC2020` with the camera matrix
/// and then scale each row so white maps to white — puts that diagonal in the
/// *destination* space, and it is wrong everywhere except on neutrals.
///
/// It is wrong by a lot. On the a6400 the green row came out
/// `[-0.300, 2.179, -0.879]`, because the row it divides by sums to 0.63 and
/// inflates every coefficient by 1.6x. Pushing a clear sky through it collapsed
/// green from 0.17 above red to 0.01 above — which is a blue sky rendering
/// violet, and exactly what it looked like on screen. Neutrals stayed correct
/// throughout, which is why every other test in the suite passed: the
/// normalisation forces white to be right by construction and only saturated
/// colours reveal the damage.
///
/// Normalising the *other* direction fixes it. Scaling the rows of
/// working→camera so Rec.2020 white lands on camera `(1,1,1)`, then inverting,
/// puts the diagonal in camera space where the sensor's own imbalance actually
/// lives — a von Kries adaptation rather than a fudge applied after the fact.
/// The same sky then round-trips exactly, and the green row becomes a sane
/// `[-0.071, 1.466, -0.395]`. This is what dcraw does in `cam_xyz_coeff`.
pub fn camera_to_working(cam_to_xyz: Mat3) -> Option<Mat3> {
    let xyz_to_cam = crate::raw::decoder::invert3(cam_to_xyz)?;
    let rec2020_to_xyz = crate::raw::decoder::invert3(XYZ_TO_REC2020)?;
    let working_to_cam = multiply(xyz_to_cam, rec2020_to_xyz);
    crate::raw::decoder::invert3(normalise_white(working_to_cam))
}

/// Scales each row so it sums to one, i.e. so `(1,1,1)` maps to `(1,1,1)`.
///
/// After white balance a neutral subject reads as equal camera RGB and has to
/// stay neutral through the colour transform; camera matrices are not
/// normalised for that.
///
/// **Which matrix this is applied to decides whether it is correct** — see
/// [`camera_to_working`]. On a camera→display matrix it adapts in the wrong
/// space and rotates every saturated hue.
pub fn normalise_white(m: Mat3) -> Mat3 {
    let mut out = m;
    for row in out.iter_mut() {
        let sum: f32 = row.iter().sum();
        if sum.abs() > 1e-9 {
            for v in row.iter_mut() {
                *v /= sum;
            }
        }
    }
    out
}

/// Rec.709 luminance weights, for the linear-sRGB display primaries the gamut
/// map works in.
pub const LUMA_SRGB: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Fits a linear display-primary colour into the `[0, 1]` display gamut by
/// desaturating it toward the **equal-luminance grey** — at constant hue and
/// luminance — until every channel is representable. A colour already in gamut
/// is returned unchanged.
///
/// This replaces a per-channel clamp. Clamping each channel independently
/// rotates the hue of any colour that leaves the gamut: a clipped saturated blue
/// skews cyan, a clipped red skews orange. Pulling toward the grey of the same
/// luminance keeps the hue and only sheds saturation — the behaviour a
/// filmic-style pipeline uses, and what makes an out-of-sRGB colour degrade
/// gracefully instead of shifting.
///
/// **Mirrored in `gpu/shaders/develop.wgsl::gamut_map`; the two must stay
/// identical. This is the tested reference for that shader code.**
pub fn gamut_map(rgb: [f32; 3]) -> [f32; 3] {
    let l = srgb_luma(rgb);
    desaturate(rgb, l, relative_scale(rgb, l))
}

/// [`gamut_map`] under the perceptual rendering intent: the smaller of the two
/// scales, so the result is always in gamut however degenerate `l` is.
pub fn gamut_map_perceptual(rgb: [f32; 3]) -> [f32; 3] {
    let l = srgb_luma(rgb);
    desaturate(rgb, l, relative_scale(rgb, l).min(perceptual_scale(rgb, l)))
}

fn srgb_luma(rgb: [f32; 3]) -> f32 {
    (rgb[0] * LUMA_SRGB[0] + rgb[1] * LUMA_SRGB[1] + rgb[2] * LUMA_SRGB[2]).clamp(0.0, 1.0)
}

fn desaturate(rgb: [f32; 3], l: f32, s: f32) -> [f32; 3] {
    [
        l + s * (rgb[0] - l),
        l + s * (rgb[1] - l),
        l + s * (rgb[2] - l),
    ]
}

/// Relative colorimetric's scale: the largest fraction of the saturation that
/// still fits, 1 for a colour already in gamut.
fn relative_scale(rgb: [f32; 3], l: f32) -> f32 {
    // The largest fraction of the original saturation that still fits. A channel
    // above white binds against 1 (its excess is `c - l`, its allowance `1 - l`);
    // one below black binds against 0. Divisors are strictly positive: `c > 1 >= l`
    // gives `c - l > 0`, and `c < 0 <= l` gives `l - c > 0`.
    let mut s = 1.0f32;
    for &c in &rgb {
        if c > 1.0 {
            s = s.min((1.0 - l) / (c - l));
        } else if c < 0.0 {
            s = s.min(l / (l - c));
        }
    }
    s.clamp(0.0, 1.0)
}

/// Where the perceptual intent starts compressing: the fraction of the way to
/// the gamut edge below which a colour is left exactly as it is.
pub const PERCEPTUAL_THRESHOLD: f32 = 0.7;
/// How far outside the gamut the perceptual intent still has room for — about
/// where a Rec.2020 primary lands against sRGB. Anything beyond it meets the
/// edge, as it would under relative colorimetric.
pub const PERCEPTUAL_LIMIT: f32 = 2.5;
/// Shape of the perceptual roll-off (1 is the gentlest knee, higher is sharper).
pub const PERCEPTUAL_POWER: f32 = 1.2;

/// The perceptual rendering intent's saturation scale for `rgb` about its grey
/// `l`.
///
/// Relative colorimetric moves only the colours that are out of gamut, each
/// straight to the edge, so every colour past the edge in a given direction
/// lands on the same one and a saturated gradient flattens into a band. This
/// measures how far a colour is towards the edge (`t`: 1 on it, above 1
/// outside) and compresses everything from `PERCEPTUAL_THRESHOLD` to
/// `PERCEPTUAL_LIMIT` into the room between the threshold and the edge, so
/// colours outside the gamut stay distinct — at the price of muting the most
/// saturated in-gamut ones a little. The curve is the parametric one ACES's
/// reference gamut compression uses.
///
/// **Mirrored in `gpu/shaders/develop.wgsl::perceptual_scale`.**
pub fn perceptual_scale(rgb: [f32; 3], l: f32) -> f32 {
    let mut t = 0.0f32;
    for &c in &rgb {
        if c > l && l < 1.0 {
            t = t.max((c - l) / (1.0 - l));
        } else if c < l && l > 0.0 {
            t = t.max((l - c) / l);
        }
    }
    let thr = PERCEPTUAL_THRESHOLD;
    if t <= thr {
        return 1.0;
    }
    let p = PERCEPTUAL_POWER;
    // Chosen so that the limit lands exactly on the edge.
    let k = (PERCEPTUAL_LIMIT - thr)
        / (((1.0 - thr) / (PERCEPTUAL_LIMIT - thr)).powf(-p) - 1.0).powf(1.0 / p);
    let x = (t - thr) / k;
    let compressed = thr + k * x / (1.0 + x.powf(p)).powf(1.0 / p);
    compressed.min(1.0) / t
}

/// Repacks a row-major matrix into the column-major, vec4-strided layout WGSL
/// uses for `mat3x3<f32>`.
///
/// Two separate traps in one function. WGSL matrices are **column-major**, so
/// uploading rows silently transposes the transform; and each column is padded
/// to 16 bytes, so a tight `[[f32; 3]; 3]` upload shears it. Both produce a
/// picture that renders fine and is the wrong colour.
pub fn to_wgsl(m: Mat3) -> [[f32; 4]; 3] {
    let mut out = [[0.0f32; 4]; 3];
    for (c, col) in out.iter_mut().enumerate() {
        for r in 0..3 {
            col[r] = m[r][c];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < tol)
    }

    /// The derived weights reproduce the published ones: Rec.709 for sRGB
    /// (the constant the shader used to hard-code), and P3's and Adobe RGB's.
    #[test]
    fn luma_weights_are_each_spaces_own() {
        let near =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 2e-3 && (a[1] - b[1]).abs() < 2e-3;
        assert!(near(luma_weights(REC2020_TO_SRGB), [0.2126, 0.7152]));
        assert!(near(
            luma_weights(rec2020_to_display_p3()),
            [0.2290, 0.6917]
        ));
        assert!(near(luma_weights(rec2020_to_adobe_rgb()), [0.2974, 0.6273]));
    }

    /// White stays white into P3, and P3 holds more than sRGB: pure Rec.2020
    /// green is outside both, but much less far outside P3.
    #[test]
    fn display_p3_keeps_white_and_holds_more_than_srgb() {
        let m = rec2020_to_display_p3();
        assert!(close(apply(m, [1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 2e-3));
        let saturated = [0.0, 1.0, 0.0];
        let in_p3 = apply(m, saturated);
        let in_srgb = apply(REC2020_TO_SRGB, saturated);
        let spill = |v: [f32; 3]| v.iter().map(|c| (-c).max(c - 1.0).max(0.0)).sum::<f32>();
        assert!(spill(in_p3) < spill(in_srgb), "{in_p3:?} vs {in_srgb:?}");
    }

    #[test]
    fn rec2020_white_is_srgb_white() {
        let white = apply(REC2020_TO_SRGB, [1.0, 1.0, 1.0]);
        assert!(close(white, [1.0, 1.0, 1.0], 0.01), "got {white:?}");
    }

    /// D65 white in XYZ must land on neutral Rec.2020.
    #[test]
    fn d65_maps_to_neutral() {
        let d65 = [0.95047, 1.0, 1.08883];
        let rgb = apply(XYZ_TO_REC2020, d65);
        assert!(close(rgb, [1.0, 1.0, 1.0], 0.01), "got {rgb:?}");
    }

    #[test]
    fn normalisation_makes_neutral_stay_neutral() {
        // A deliberately unbalanced matrix.
        let m = [[0.8, 0.1, 0.05], [0.2, 0.9, 0.1], [0.0, 0.05, 1.4]];
        let out = apply(normalise_white(m), [1.0, 1.0, 1.0]);
        assert!(close(out, [1.0, 1.0, 1.0], 1e-5), "got {out:?}");
    }

    /// Pins the column-major repack. A row-major upload would put `m[0][1]`
    /// where `m[1][0]` belongs, which is exactly the transposition bug.
    #[test]
    fn wgsl_layout_is_column_major_and_padded() {
        let m = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        let packed = to_wgsl(m);

        assert_eq!(packed[0], [1.0, 4.0, 7.0, 0.0], "first column");
        assert_eq!(packed[1], [2.0, 5.0, 8.0, 0.0], "second column");
        assert_eq!(packed[2], [3.0, 6.0, 9.0, 0.0], "third column");
    }

    #[test]
    fn multiplication_composes_in_the_expected_order() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(multiply(XYZ_TO_REC2020, identity), XYZ_TO_REC2020);
        assert_eq!(multiply(identity, XYZ_TO_REC2020), XYZ_TO_REC2020);
    }

    /// A colour put in must come back out.
    ///
    /// This is the test the suite was missing. Every colour check here and in
    /// the integration tests looked at neutrals or at frame-wide averages, and
    /// the white normalisation forces neutrals to be right *by construction* —
    /// so a matrix that mangled every saturated colour still passed everything.
    /// A clear sky lost 94% of its green-over-red and rendered violet.
    ///
    /// Working backwards from a known sRGB colour to the camera RGB that would
    /// record it, then forwards through the real transform, is what exposes it.
    #[test]
    fn a_saturated_colour_survives_the_camera_transform() {
        // LibRaw's matrix for the Sony ILCE-6400.
        let cam_to_xyz: Mat3 = [
            [1.4960729, 0.35472122, 0.010384526],
            [0.49973437, 0.996734, -0.35555422],
            [0.053848483, -0.20033188, 1.7986445],
        ];

        let xyz_to_srgb = multiply(REC2020_TO_SRGB, XYZ_TO_REC2020);
        let srgb_to_xyz = crate::raw::decoder::invert3(xyz_to_srgb).unwrap();
        let xyz_to_cam = crate::raw::decoder::invert3(cam_to_xyz).unwrap();
        let m = camera_to_working(cam_to_xyz).expect("matrix should invert");

        // What the camera records for a neutral, so the same white balance the
        // mosaic pass applies can be undone here.
        let white = apply(xyz_to_cam, apply(srgb_to_xyz, [1.0, 1.0, 1.0]));

        // A clear sky, deep foliage and skin: the three that show a hue error
        // first, and none of them neutral.
        for target in [
            [0.28f32, 0.45, 0.85],
            [0.10, 0.35, 0.08],
            [0.75, 0.45, 0.35],
        ] {
            let cam = apply(xyz_to_cam, apply(srgb_to_xyz, target));
            let balanced = [cam[0] / white[0], cam[1] / white[1], cam[2] / white[2]];
            let out = apply(REC2020_TO_SRGB, apply(m, balanced));

            for (a, b) in out.iter().zip(target) {
                assert!(
                    (a - b).abs() < 0.005,
                    "{target:?} came back as {out:?} — the camera transform is rotating hues"
                );
            }
        }
    }

    /// The specific construction that was wrong, kept as a guard.
    ///
    /// Normalising the camera→working direction leaves coefficients that are
    /// wildly out of range; a healthy green row sits near `[-0.1, 1.5, -0.4]`.
    #[test]
    fn the_working_transform_has_sane_coefficients() {
        let cam_to_xyz: Mat3 = [
            [1.4960729, 0.35472122, 0.010384526],
            [0.49973437, 0.996734, -0.35555422],
            [0.053848483, -0.20033188, 1.7986445],
        ];
        let m = camera_to_working(cam_to_xyz).unwrap();
        for row in m {
            let sum: f32 = row.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "white is not preserved: {row:?}");
            for v in row {
                assert!(
                    v.abs() < 2.0,
                    "coefficient {v} is out of range in {m:?} — the adaptation is being \
                     applied in the wrong space"
                );
            }
        }
    }

    #[test]
    fn the_perceptual_intent_keeps_out_of_gamut_colours_apart() {
        // One red, at two saturations past the sRGB edge: same grey (0.3) and
        // same hue, `d` being a direction with no luminance of its own.
        // Relative colorimetric lands both on the one point of the edge —
        // the banding. Perceptual keeps them apart, ordered, and inside.
        let d = [1.0, -0.25, -0.468];
        let at = |k: f32| [0.3 + k * d[0], 0.3 + k * d[1], 0.3 + k * d[2]];
        let (a, b) = (at(0.9), at(1.2));
        let (ra, rb) = (gamut_map(a), gamut_map(b));
        let (pa, pb) = (gamut_map_perceptual(a), gamut_map_perceptual(b));
        let chroma = |c: [f32; 3]| {
            let l = srgb_luma(c);
            c.iter().map(|x| (x - l).abs()).fold(0.0f32, f32::max)
        };
        let inside = |c: [f32; 3]| c.iter().all(|&x| x > 1e-3 && x < 1.0 - 1e-3);
        assert!(
            close(ra, rb, 1e-4),
            "relative collapses them: {ra:?} vs {rb:?}"
        );
        assert!(
            inside(pa) && inside(pb),
            "perceptual stays inside: {pa:?} {pb:?}"
        );
        assert!(chroma(pb) > chroma(pa) + 0.01, "{pa:?} vs {pb:?}");
    }

    #[test]
    fn the_perceptual_intent_leaves_neutrals_and_mild_colours_alone() {
        for c in [
            [0.5, 0.5, 0.5],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.5, 0.4, 0.4],
        ] {
            assert!(close(gamut_map_perceptual(c), c, 1e-6), "{c:?} moved");
        }
    }

    #[test]
    fn the_perceptual_curve_is_continuous_and_monotonic() {
        // `t * scale` is where a colour ends up relative to the edge: it must rise
        // with `t`, start where the threshold is, and reach the edge at the limit.
        let l = 0.5;
        let place = |t: f32| t * perceptual_scale([l + t * (1.0 - l), l, l], l);
        let mut last = 0.0;
        for i in 1..=400 {
            let t = i as f32 / 100.0;
            let at = place(t);
            assert!(
                at >= last - 1e-6 && at <= 1.0 + 1e-5,
                "t {t}: {at} after {last}"
            );
            last = at;
        }
        assert!((place(PERCEPTUAL_THRESHOLD) - PERCEPTUAL_THRESHOLD).abs() < 1e-5);
        assert!((place(PERCEPTUAL_LIMIT) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn an_in_gamut_colour_passes_the_gamut_map_unchanged() {
        for c in [
            [0.6, 0.3, 0.72],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.5, 0.5, 0.5],
        ] {
            assert!(
                close(gamut_map(c), c, 1e-6),
                "in-gamut colour {c:?} was altered"
            );
        }
    }

    /// The point of the map: an out-of-gamut colour is brought into range while
    /// its hue is held — the failure a per-channel clamp has.
    #[test]
    fn an_out_of_gamut_colour_fits_the_display_and_keeps_its_hue() {
        // Above white on red, below black on blue: outside sRGB on two channels.
        let c = [1.4f32, 0.5, -0.2];
        let out = gamut_map(c);

        // Every channel is now representable.
        for v in out {
            assert!(
                (-1e-5..=1.0 + 1e-5).contains(&v),
                "channel {v} still out of gamut in {out:?}"
            );
        }

        // Hue held: `out` lies on the line from `c` through the equal-luminance
        // grey, so `(out - l)` is one positive scalar multiple of `(c - l)`.
        let l = (c[0] * LUMA_SRGB[0] + c[1] * LUMA_SRGB[1] + c[2] * LUMA_SRGB[2]).clamp(0.0, 1.0);
        let s = (out[0] - l) / (c[0] - l);
        assert!(
            s > 0.0 && s <= 1.0,
            "desaturation factor {s} is not in (0, 1]"
        );
        for i in 0..3 {
            if (c[i] - l).abs() > 1e-4 {
                assert!(
                    ((out[i] - l) / (c[i] - l) - s).abs() < 1e-4,
                    "channel {i} moved off the constant-hue line — the map is shifting hue"
                );
            }
        }
    }

    /// A colour far outside the gamut still resolves to a valid, finite result
    /// rather than blowing up on the division.
    #[test]
    fn a_wildly_out_of_gamut_colour_is_still_finite_and_in_range() {
        for c in [[5.0f32, -3.0, 0.2], [-1.0, 2.0, -0.5], [10.0, 10.0, -10.0]] {
            let out = gamut_map(c);
            for v in out {
                assert!(
                    v.is_finite() && (-1e-4..=1.0 + 1e-4).contains(&v),
                    "{c:?} -> {out:?}"
                );
            }
        }
    }
}
