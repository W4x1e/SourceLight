//! What the warmth and tint sliders do: say what light the photo was taken
//! under, and adapt it from that light, the same way on every camera.
//!
//! They used to scale the camera's own red and blue channels (a version-2
//! stack still does, `EditStack::wb_scale`). A camera's channels are its own:
//! what a boost to red does to a picture depends on that sensor's filters and
//! matrix, so one warmth landed tens of mireds apart from body to body, and
//! on some it tinted as well as warmed (`tests/white_balance.rs`). Gains
//! solved through each camera's matrix narrow that, to 11 mireds on the
//! fixtures, but leave each sensor's space doing the adapting, and a Leica's
//! still tints.
//!
//! So the move is made after the camera's matrix, in the working space, where
//! every camera's colours already agree: warmth moves the light's colour
//! temperature, in mireds — the unit in which equal steps look equal at any
//! temperature — tint moves it across the Planckian locus, in Δuv, and the
//! picture is adapted from that light to daylight by the Bradford transform,
//! the chromatic adaptation the DNG and ICC specifications use. One setting is
//! one matrix, whatever the camera.
//!
//! Photographers' "warmer" means the light was bluer than the camera took it
//! for, and adapting from it warms the picture; so positive warmth lowers the
//! light's mireds, and positive tint (magenta) makes it greener.

use crate::gpu::color::{self, Mat3};
use crate::raw::colour::{bradford, D65, ROBERTSON};
use crate::raw::decoder::invert3;

/// Mireds the light moves per unit of warmth, at first. Set so that small
/// moves warm the calibration body (the a6400) as fast as the
/// channel-scaling sliders did, so a feel learned on them carries over.
pub const MIREDS_PER_WARMTH: f32 = 175.0;

/// How far the light goes at the cool end, warmth -1: 200 mireds warmer than
/// daylight, about 2800 K, which is a tungsten bulb's.
pub const WARMEST_LIGHT_MIREDS: f32 = 200.0;

/// Δuv the light moves per unit of tint, positive towards green. Set, as
/// warmth's is, against the channel-scaling sliders on the calibration body;
/// the range spans what fluorescent and LED lights sit off the locus.
pub const DUV_PER_TINT: f32 = 0.013;

/// The mireds a warmth moves the light by, from D65 at `d65` mireds.
///
/// `MIREDS_PER_WARMTH` a unit at the start, bending so that each end of the
/// slider is an end of what light does. Mireds are lopsided about daylight:
/// a light can be at most 154 mireds bluer (the top of the temperature scale,
/// where warmth +1 lands) and far more than that warmer (warmth -1 stops at
/// `WARMEST_LIGHT_MIREDS`). Straight lines would leave a dead stretch at one
/// end or a short reach at the other.
fn mireds_for(warmth: f32, d65: f32) -> f32 {
    let k = MIREDS_PER_WARMTH;
    let w = warmth.abs();
    if warmth >= 0.0 {
        -(k * w - (k - d65) * w * w)
    } else {
        k * w + (WARMEST_LIGHT_MIREDS - k) * w * w
    }
}

/// CIE 1960 (u, v) of an XYZ colour.
fn uv(xyz: [f32; 3]) -> [f32; 2] {
    let d = xyz[0] + 15.0 * xyz[1] + 3.0 * xyz[2];
    [4.0 * xyz[0] / d, 6.0 * xyz[1] / d]
}

/// XYZ, with Y = 1, of a CIE 1960 (u, v).
fn xyz(uv: [f32; 2]) -> [f32; 3] {
    let d = 2.0 * uv[0] - 8.0 * uv[1] + 4.0;
    let (x, y) = (3.0 * uv[0] / d, 2.0 * uv[1] / d);
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Where the Planckian locus is at `mireds`, and the unit direction across it
/// towards green: Robertson's table, whose isotemperature lines cross the
/// locus at right angles, interpolated.
fn locus(mireds: f32) -> ([f32; 2], [f32; 2]) {
    let m = mireds.clamp(0.0, ROBERTSON[ROBERTSON.len() - 1].0);
    let i = ROBERTSON
        .iter()
        .position(|r| r.0 >= m)
        .unwrap_or(ROBERTSON.len() - 1)
        .max(1);
    let (a, b) = (ROBERTSON[i - 1], ROBERTSON[i]);
    let f = (m - a.0) / (b.0 - a.0);
    let lerp = |p: f32, q: f32| p + f * (q - p);
    // An isotemperature line runs along (1, slope); green is the end with
    // more v, which is the negated one for these negative slopes.
    let across = |slope: f32| {
        let len = (1.0 + slope * slope).sqrt();
        [-1.0 / len, -slope / len]
    };
    let (ga, gb) = (across(a.3), across(b.3));
    let g = [lerp(ga[0], gb[0]), lerp(ga[1], gb[1])];
    let len = g[0].hypot(g[1]);
    ([lerp(a.1, b.1), lerp(a.2, b.2)], [g[0] / len, g[1] / len])
}

/// Where a light or colour sits: its temperature in mireds, and its Δuv from
/// the locus, positive towards green.
pub fn placement(xyz: [f32; 3]) -> (f32, f32) {
    let sum = xyz[0] + xyz[1] + xyz[2];
    let mireds = 1.0e6 / crate::raw::colour::temperature([xyz[0] / sum, xyz[1] / sum]);
    let (on, green) = locus(mireds);
    let at = uv(xyz);
    let duv = (at[0] - on[0]) * green[0] + (at[1] - on[1]) * green[1];
    (mireds, duv)
}

/// The light a photo is balanced for, moved from as shot: its XYZ, Y = 1.
///
/// As-shot balance takes the photo's light to the working space's white,
/// D65, so the move is made from there: along the locus by `mireds`, keeping
/// D65's own small distance from it, and across it by `duv`.
pub fn light(mireds: f32, duv: f32) -> [f32; 3] {
    let (from, off) = placement(D65);
    let (on, green) = locus(from + mireds);
    let duv = off + duv;
    xyz([on[0] + duv * green[0], on[1] + duv * green[1]])
}

/// The light `warmth` and `tint` say the photo was taken under.
pub fn light_for(warmth: f32, tint: f32) -> [f32; 3] {
    let (d65, _) = placement(D65);
    light(
        mireds_for(warmth.clamp(-1.0, 1.0), d65),
        tint.clamp(-1.0, 1.0) * DUV_PER_TINT,
    )
}

/// The adaptation `warmth` and `tint` make, in the working space (linear
/// Rec.2020): from the light they describe to daylight, by Bradford. Neutrals
/// under that light come out neutral and keep their brightness, since both
/// whites are taken at Y = 1; nothing at all moves at zero.
pub fn adaptation(warmth: f32, tint: f32) -> Mat3 {
    const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    if warmth == 0.0 && tint == 0.0 {
        return IDENTITY;
    }
    let Some(working_to_xyz) = invert3(color::XYZ_TO_REC2020) else {
        return IDENTITY;
    };
    // To the same arithmetic's daylight, so a move of nothing is nothing
    // whatever the rounding in the round trip through (u, v).
    let adapt = bradford(light_for(warmth, tint), light(0.0, 0.0));
    color::multiply(
        color::XYZ_TO_REC2020,
        color::multiply(adapt, working_to_xyz),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    /// What the adaptation makes of the working space's white: the grey that
    /// was neutral as shot.
    fn grey(warmth: f32, tint: f32) -> [f32; 3] {
        color::apply(adaptation(warmth, tint), [1.0; 3])
    }

    #[test]
    fn no_move_is_no_change() {
        assert_eq!(adaptation(0.0, 0.0), IDENTITY);
        let tiny = grey(1e-4, 0.0);
        assert!(tiny.iter().all(|g| (g - 1.0).abs() < 1e-3), "{tiny:?}");
    }

    /// Warmer lifts red against blue; magenta lifts both against green.
    #[test]
    fn the_sliders_move_the_way_they_say() {
        let warm = grey(0.5, 0.0);
        assert!(warm[0] > warm[1] && warm[1] > warm[2], "{warm:?}");
        let cool = grey(-0.5, 0.0);
        assert!(cool[0] < cool[1] && cool[1] < cool[2], "{cool:?}");
        let magenta = grey(0.0, 0.5);
        assert!(magenta[1] < (magenta[0] * magenta[2]).sqrt(), "{magenta:?}");
        let green = grey(0.0, -0.5);
        assert!(green[1] > (green[0] * green[2]).sqrt(), "{green:?}");
    }

    /// The light moves along the locus: warmth changes its temperature and
    /// not how far it sits from the locus, which is what tint is for.
    #[test]
    fn warmth_follows_the_locus() {
        let (d65, d65_duv) = placement(D65);
        assert!(
            (1.0e6 / d65 - 6504.0).abs() < 30.0,
            "D65 at {} K",
            1.0e6 / d65
        );
        for mireds in [-100.0, -40.0, 40.0, 100.0] {
            let (at, duv) = placement(light(mireds, 0.0));
            assert!(
                (at - (d65 + mireds)).abs() < 1.0,
                "{mireds} mireds: moved to {at}, expected {}",
                d65 + mireds
            );
            assert!(
                (duv - d65_duv).abs() < 5e-4,
                "{mireds}: Δuv {duv} vs D65's {d65_duv}"
            );
        }
        // And tint moves only the Δuv.
        let (at, duv) = placement(light(0.0, 0.01));
        assert!((at - d65).abs() < 1.0 && (duv - d65_duv - 0.01).abs() < 5e-4);
    }

    /// A neutral lit by the light the sliders describe comes out neutral, at
    /// the brightness it had: that is what they mean.
    #[test]
    fn the_described_light_comes_out_white() {
        for (w, t) in [(0.4, -0.3), (-0.8, 0.5), (1.0, 0.0), (-1.0, -1.0)] {
            let white = color::apply(color::XYZ_TO_REC2020, light(0.0, 0.0));
            let lit = color::apply(color::XYZ_TO_REC2020, light_for(w, t));
            let lit: [f32; 3] = std::array::from_fn(|c| lit[c] / white[c]);
            let out = color::apply(adaptation(w, t), lit);
            for c in out {
                assert!((c - 1.0).abs() < 2e-3, "({w}, {t}): {out:?}");
            }
        }
    }

    /// Warmth warms the grey, and through the middle of its range hardly
    /// tints it: 0.001 Δuv at ±0.3. Further out Bradford takes it a little
    /// green of the locus (0.004 at +1) — the same on every camera, which is
    /// the point. Past about -0.5 the grey is bluer than the top of the
    /// temperature scale, where Δuv stops meaning anything.
    #[test]
    fn warmth_warms_without_tinting() {
        let to_xyz = invert3(color::XYZ_TO_REC2020).unwrap();
        let at = |w: f32| placement(color::apply(to_xyz, grey(w, 0.0)));
        let (m0, d0) = at(0.0);
        for w in [-0.3, -0.15, 0.15, 0.3] {
            let (m, d) = at(w);
            assert!(
                (m - m0) * w > 0.0,
                "warmth {w} moved the grey {} mireds",
                m - m0
            );
            assert!(
                (d - d0).abs() < 1.5e-3,
                "warmth {w} tinted the grey by {}",
                d - d0
            );
        }
        let warmer: Vec<f32> = (0..=10).map(|i| at(i as f32 / 10.0).0).collect();
        assert!(warmer.windows(2).all(|p| p[1] > p[0]), "{warmer:?}");
    }

    /// The a6400's matrix, XYZ to camera, from its DNG profile; the body the
    /// slider scales are set against.
    #[allow(clippy::approx_constant)]
    fn a6400() -> Mat3 {
        let xyz_to_camera = [
            [0.5991, -0.1456, -0.0217],
            [-0.4342, 1.1914, 0.2728],
            [-0.0580, 0.1418, 0.6089],
        ];
        color::camera_to_working(invert3(xyz_to_camera).unwrap()).unwrap()
    }

    /// How fast each slider moves the as-shot grey in the camera's own
    /// channels at the start, which is where the old sliders acted: warmth in
    /// ln(red / blue), tint in ln(green / the mean of the other two).
    fn slopes(camera: Mat3) -> (f32, f32) {
        let to_camera = invert3(camera).unwrap();
        let seen = |w: f32, t: f32| color::apply(to_camera, grey(w, t)).map(f32::ln);
        let e = 0.01;
        let (wp, wm) = (seen(e, 0.0), seen(-e, 0.0));
        let (tp, tm) = (seen(0.0, e), seen(0.0, -e));
        let warm = ((wp[0] - wp[2]) - (wm[0] - wm[2])) / (2.0 * e);
        let tint = ((tp[1] - (tp[0] + tp[2]) / 2.0) - (tm[1] - (tm[0] + tm[2]) / 2.0)) / (2.0 * e);
        (warm, tint)
    }

    /// Small moves on the a6400 go as fast as the channel-scaling sliders'
    /// did: ln(red / blue) by 1 a unit of warmth, green against the others by
    /// 0.1 a unit of tint.
    #[test]
    fn the_sliders_start_as_fast_as_they_did() {
        let (warm, tint) = slopes(a6400());
        assert!(
            (warm - 1.0).abs() < 0.05,
            "warmth moves ln(R/B) by {warm} a unit"
        );
        assert!(
            (tint + 0.1).abs() < 0.01,
            "tint moves ln(G/RB) by {tint} a unit"
        );
    }

    /// Each end of the warmth slider is an end of what light does, and the
    /// way there only ever goes one way.
    #[test]
    fn warmth_reaches_both_ends_of_the_scale() {
        let (d65, _) = placement(D65);
        assert!(
            (mireds_for(1.0, d65) + d65).abs() < 1e-3,
            "+1 is the top of the scale"
        );
        assert_eq!(mireds_for(-1.0, d65), WARMEST_LIGHT_MIREDS);
        let steps: Vec<f32> = (-20..=20)
            .map(|i| mireds_for(i as f32 / 20.0, d65))
            .collect();
        assert!(steps.windows(2).all(|p| p[1] < p[0]), "{steps:?}");
        // Red against blue keeps moving right to each end.
        let ratio = |w: f32| {
            let g = grey(w, 0.0);
            g[0] / g[2]
        };
        assert!(ratio(1.0) > ratio(0.9) && ratio(-1.0) < ratio(-0.9));
    }
}
