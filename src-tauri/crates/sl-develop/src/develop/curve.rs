//! The point curve: a user-drawn tone adjustment on top of the parametric one.
//!
//! Where `tone.rs` exposes five named controls that shape a filmic curve, this
//! is the escape hatch — drag any point anywhere. The two compose rather than
//! compete: the parametric curve maps scene light to display, and the point
//! curve then reshapes that display-referred result, which is the usual
//! order in raw editors and the reason a curve panel reads in familiar 0..1 terms
//! rather than in stops.
//!
//! **It costs nothing in the shader.** The parametric curve is already baked
//! into a lookup table; a point curve applied to that table's *values* is
//! another table of the same size. So the whole feature is a transformation of
//! `Vec<f32>` on the CPU — unit-testable, and the develop pass never learns it
//! exists.

use serde::{Deserialize, Serialize};

/// One control point, in display-referred 0..1 on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

impl CurvePoint {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Evaluates a point curve, or passes values through unchanged.
///
/// Construction does the work — sorting, deduplicating and solving for the
/// tangents — so evaluating each of the thousand-odd LUT entries afterwards is
/// a couple of multiplies.
pub struct Curve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    /// Slope at each control point, from `monotone_tangents`.
    ms: Vec<f32>,
}

impl Curve {
    /// Builds a curve from control points. Fewer than two usable points means
    /// the identity, so an empty stack costs nothing and cannot misbehave.
    ///
    /// Points arrive from a UI where the user can drag one past another, so
    /// they are sorted here rather than trusted. Two points at the same x have
    /// no defined slope between them; the later one wins, which is what
    /// dragging one control point onto another looks like.
    pub fn new(points: &[CurvePoint]) -> Self {
        let mut pts: Vec<CurvePoint> = points
            .iter()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .map(|p| CurvePoint::new(p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0)))
            .collect();
        pts.sort_by(|a, b| a.x.total_cmp(&b.x));
        pts.dedup_by(|a, b| (a.x - b.x).abs() < 1e-6);

        let xs: Vec<f32> = pts.iter().map(|p| p.x).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p.y).collect();
        let ms = monotone_tangents(&xs, &ys);
        Self { xs, ys, ms }
    }

    /// True when this curve leaves every value untouched, so callers can skip
    /// the work entirely.
    pub fn is_identity(&self) -> bool {
        self.xs.len() < 2
    }

    /// Evaluates the curve at `x`, holding the end values outside its span.
    ///
    /// Clamping rather than extrapolating: a cubic continued past its last
    /// control point leaves 0..1 almost immediately, and the result would be a
    /// curve that inverts in the deepest shadows for no reason the user could
    /// see.
    pub fn eval(&self, x: f32) -> f32 {
        if self.is_identity() {
            return x;
        }
        let last = self.xs.len() - 1;
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[last] {
            return self.ys[last];
        }

        // The LUT is walked in order, but binary search keeps `eval` honest as
        // a standalone function rather than one that only works in a loop.
        let i = match self.xs.binary_search_by(|p| p.total_cmp(&x)) {
            Ok(hit) => return self.ys[hit],
            Err(next) => next - 1,
        };

        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);

        // Cubic Hermite basis.
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;

        (h00 * self.ys[i] + h10 * h * self.ms[i] + h01 * self.ys[i + 1] + h11 * h * self.ms[i + 1])
            .clamp(0.0, 1.0)
    }

    /// Reshapes an already-baked tone table in place.
    ///
    /// The table holds display-referred values, which is exactly this curve's
    /// domain, so composition is a straight remap of each entry. Order matters:
    /// parametric first, then this.
    pub fn apply_to_lut(&self, lut: &mut [f32]) {
        if self.is_identity() {
            return;
        }
        for v in lut.iter_mut() {
            *v = self.eval(*v);
        }
    }
}

/// Entries in the per-channel curve LUT — indexed by a channel's 0..1 value, so
/// 256 is plenty and matches an 8-bit input's resolution.
pub const CHANNEL_LUT_SIZE: usize = 256;

/// Bakes three per-channel point curves into one RGBA LUT the shader samples per
/// channel: entry `i` (for value `v = i/(n-1)`) holds `[red(v), green(v),
/// blue(v), 0]`. An empty channel curve is the identity, so an untouched channel
/// passes straight through and the whole thing is a no-op when nothing is set.
pub fn channel_lut(
    red: &[CurvePoint],
    green: &[CurvePoint],
    blue: &[CurvePoint],
    size: usize,
) -> Vec<f32> {
    let (r, g, b) = (Curve::new(red), Curve::new(green), Curve::new(blue));
    let last = (size.max(2) - 1) as f32;
    let mut lut = vec![0f32; size * 4];
    for i in 0..size {
        let v = i as f32 / last;
        lut[i * 4] = r.eval(v);
        lut[i * 4 + 1] = g.eval(v);
        lut[i * 4 + 2] = b.eval(v);
    }
    lut
}

/// Fritsch–Carlson tangents: the slopes that make a cubic Hermite spline
/// monotonic wherever the data is.
///
/// **This is the whole reason the curve is not a plain Catmull–Rom spline.** An
/// unconstrained cubic through control points overshoots near a sharp change —
/// so dragging a point up to brighten the midtones would dip the curve *below*
/// its neighbours on the way there. On a tone curve that reads as inverted
/// tones and banding in a smooth gradient, the same failure the parametric
/// contrast control has to avoid.
///
/// The construction: start with the average of the neighbouring secants, then
/// wherever a segment is flat force both its tangents to zero, and elsewhere
/// pull the tangent pair back inside a circle of radius 3. That last bound is
/// the Fritsch–Carlson condition, and it is what guarantees no overshoot.
fn monotone_tangents(xs: &[f32], ys: &[f32]) -> Vec<f32> {
    let n = xs.len();
    if n < 2 {
        return vec![0.0; n];
    }

    let secants: Vec<f32> = (0..n - 1)
        .map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]))
        .collect();

    let mut m = Vec::with_capacity(n);
    m.push(secants[0]);
    for i in 1..n - 1 {
        // A sign change means a genuine local extremum; a zero tangent there
        // keeps the curve from swinging through it.
        if secants[i - 1] * secants[i] <= 0.0 {
            m.push(0.0);
        } else {
            m.push((secants[i - 1] + secants[i]) / 2.0);
        }
    }
    m.push(secants[n - 2]);

    for i in 0..n - 1 {
        if secants[i].abs() < 1e-9 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let alpha = m[i] / secants[i];
        let beta = m[i + 1] / secants[i];
        let norm = alpha * alpha + beta * beta;
        if norm > 9.0 {
            let tau = 3.0 / norm.sqrt();
            m[i] = tau * alpha * secants[i];
            m[i + 1] = tau * beta * secants[i];
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_lut_is_identity_when_empty_and_remaps_when_set() {
        // No curves: every entry passes its own value through per channel.
        let id = channel_lut(&[], &[], &[], CHANNEL_LUT_SIZE);
        let last = (CHANNEL_LUT_SIZE - 1) as f32;
        for i in 0..CHANNEL_LUT_SIZE {
            let v = i as f32 / last;
            assert!((id[i * 4] - v).abs() < 1e-4, "red identity at {i}");
            assert!((id[i * 4 + 1] - v).abs() < 1e-4, "green identity");
            assert!((id[i * 4 + 2] - v).abs() < 1e-4, "blue identity");
        }
        // A red curve that lifts the midtones changes only the red channel.
        let reds = [
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.5, 0.7),
            CurvePoint::new(1.0, 1.0),
        ];
        let lut = channel_lut(&reds, &[], &[], CHANNEL_LUT_SIZE);
        let mid = CHANNEL_LUT_SIZE / 2;
        assert!(
            lut[mid * 4] > 0.6,
            "red midtone lifted, got {}",
            lut[mid * 4]
        );
        assert!((lut[mid * 4 + 1] - 0.5).abs() < 0.02, "green untouched");
        assert!((lut[mid * 4 + 2] - 0.5).abs() < 0.02, "blue untouched");
        // Endpoints are pinned.
        assert!(lut[0] < 1e-3 && (lut[(CHANNEL_LUT_SIZE - 1) * 4] - 1.0).abs() < 1e-3);
    }

    /// A curve along the diagonal must change nothing, and an empty one must
    /// not be treated as a curve at all. Both matter because the panel starts
    /// in one of these states on every photo — if either tinted the image, the
    /// curve would be applying itself before anyone touched it.
    #[test]
    fn an_untouched_curve_is_the_identity() {
        assert!(Curve::new(&[]).is_identity());
        assert!(Curve::new(&[CurvePoint::new(0.5, 0.5)]).is_identity());

        let diagonal = Curve::new(&[CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)]);
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            assert!(
                (diagonal.eval(x) - x).abs() < 1e-5,
                "diagonal moved {x} to {}",
                diagonal.eval(x)
            );
        }
    }

    /// Endpoints are the user's black and white points; the curve must pass
    /// exactly through them rather than near them.
    #[test]
    fn the_curve_honours_its_control_points() {
        let pts = [
            CurvePoint::new(0.0, 0.05),
            CurvePoint::new(0.25, 0.15),
            CurvePoint::new(0.75, 0.9),
            CurvePoint::new(1.0, 0.95),
        ];
        let c = Curve::new(&pts);
        for p in pts {
            assert!(
                (c.eval(p.x) - p.y).abs() < 1e-4,
                "control point ({}, {}) evaluated to {}",
                p.x,
                p.y,
                c.eval(p.x)
            );
        }
    }

    /// The property the spline choice exists for.
    ///
    /// An unconstrained cubic through these points overshoots below zero
    /// between the two flat shadow points and above the plateau after the
    /// steep rise. Either would show as inverted tones in a gradient.
    #[test]
    fn a_steep_curve_never_doubles_back() {
        let c = Curve::new(&[
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.4, 0.02),
            CurvePoint::new(0.5, 0.9),
            CurvePoint::new(0.6, 0.92),
            CurvePoint::new(1.0, 1.0),
        ]);

        let mut prev = c.eval(0.0);
        for i in 1..=2000 {
            let x = i as f32 / 2000.0;
            let y = c.eval(x);
            assert!(
                y >= prev - 1e-5,
                "curve fell from {prev} to {y} at x={x} — a monotone spline must not overshoot"
            );
            assert!((0.0..=1.0).contains(&y), "curve left 0..1 at x={x}: {y}");
            prev = y;
        }
    }

    /// Dragging a midtone point upward has to brighten the midtones, and a
    /// classic S has to brighten highlights while darkening shadows. Obvious,
    /// and precisely what a sign error would break.
    #[test]
    fn the_curve_moves_tones_in_the_direction_it_is_dragged() {
        let lifted = Curve::new(&[
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.5, 0.65),
            CurvePoint::new(1.0, 1.0),
        ]);
        assert!(lifted.eval(0.5) > 0.6, "midtone lift did not brighten");

        let s = Curve::new(&[
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.25, 0.15),
            CurvePoint::new(0.75, 0.85),
            CurvePoint::new(1.0, 1.0),
        ]);
        assert!(s.eval(0.25) < 0.25, "S-curve did not deepen the shadows");
        assert!(s.eval(0.75) > 0.75, "S-curve did not lift the highlights");
    }

    /// Points arrive from a draggable UI, so nothing about their order or
    /// spacing can be assumed. Garbage in must still produce a usable curve
    /// rather than a panic or a NaN that blanks the whole image.
    #[test]
    fn unordered_and_degenerate_points_still_produce_a_usable_curve() {
        let c = Curve::new(&[
            CurvePoint::new(1.0, 1.0),
            CurvePoint::new(0.5, 0.4),
            CurvePoint::new(0.0, 0.0),
            // Duplicated x, and values outside the panel's range.
            CurvePoint::new(0.5, 0.6),
            CurvePoint::new(2.0, -1.0),
            CurvePoint::new(f32::NAN, 0.5),
        ]);
        for i in 0..=100 {
            let y = c.eval(i as f32 / 100.0);
            assert!(
                y.is_finite() && (0.0..=1.0).contains(&y),
                "eval produced {y}"
            );
        }
    }

    /// **The anti-drift guard shared with the TypeScript spline.**
    ///
    /// `src/lib/curve.ts` reimplements this maths so the panel can draw the
    /// curve without an IPC round trip per mouse move, and the risk is that one
    /// side changes and the other silently keeps drawing the old shape. These
    /// exact numbers are asserted verbatim in `src/lib/curve.test.ts`; if either
    /// implementation changes, one of the two tests fails and the divergence is
    /// caught rather than shipped. The curve is a symmetric S through
    /// (0.25, 0.15) and (0.75, 0.85).
    #[test]
    fn the_ts_and_rust_splines_agree() {
        let c = Curve::new(&[
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.25, 0.15),
            CurvePoint::new(0.75, 0.85),
            CurvePoint::new(1.0, 1.0),
        ]);
        let expected = [
            (0.10, 0.050400),
            (0.25, 0.150000),
            (0.40, 0.343200),
            (0.50, 0.500000),
            (0.60, 0.656800),
            (0.75, 0.850000),
            (0.90, 0.949600),
        ];
        for (x, y) in expected {
            assert!(
                (c.eval(x) - y).abs() < 1e-5,
                "spline drifted at x={x}: {} vs committed {y}",
                c.eval(x)
            );
        }
    }

    /// Composition is the point of the design: the curve reshapes the baked
    /// parametric table rather than living in the shader.
    #[test]
    fn applying_to_a_lut_remaps_its_values() {
        let mut lut = vec![0.0, 0.25, 0.5, 0.75, 1.0];
        // Invert-ish: a monotone *decreasing* curve is still monotone, and the
        // spline must handle it, because nothing stops a user drawing one.
        let c = Curve::new(&[CurvePoint::new(0.0, 1.0), CurvePoint::new(1.0, 0.0)]);
        c.apply_to_lut(&mut lut);
        assert_eq!(lut, vec![1.0, 0.75, 0.5, 0.25, 0.0]);

        // An identity curve must leave the table byte-identical, not merely
        // close — this is the no-edit path every photo takes.
        let mut untouched = vec![0.1, 0.37, 0.9];
        let before = untouched.clone();
        Curve::new(&[]).apply_to_lut(&mut untouched);
        assert_eq!(untouched, before);
    }
}
