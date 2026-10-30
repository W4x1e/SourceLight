//! Keeping local edits on the content when the framing changes.
//!
//! Masks and heal spots are stored in **framed-output UV**: 0..1 across the
//! cropped, turned and straightened frame, the space the user places them in
//! and the shader evaluates them in. So a re-crop, a 4:5 aspect, a straighten
//! or a quarter turn left every spot and mask at the same place *in the frame*
//! while the photo moved under it: the dust came back, a clone landed on the
//! background, and the face lift moved off the face.
//!
//! When the framing changes, [`reframe`] carries every local edit from the old
//! frame to the new one through the image itself. From framed UV to the
//! oriented image is an affine map ([`Geometry::transform`]), so old framed →
//! image → new framed is one affine map `A·p + b`:
//!
//! - **Points** (a radial's centre, a gradient's ends, brush dabs, a spot's
//!   target and source) go through it exactly.
//! - **A radial** is an ellipse in UV, and an affine map takes an ellipse to an
//!   ellipse: its shape matrix `E` becomes `A·E·Aᵀ`, decomposed back into radii
//!   and an angle. Its feather, a fraction of the radius, is unchanged.
//! - **Brush dabs** are round on screen (radius in frame widths, aspect
//!   corrected). Framed pixels are a rotated, uniformly scaled copy of image
//!   pixels, so a dab stays round and its radius scales by the ratio of image
//!   pixels per frame width, old to new.
//! - **A spot** is a circle in plain UV; across a change of aspect that is an
//!   ellipse, which a spot cannot be, so its radius takes the map's area scale.
//! - **An AI mask** is a bitmap over the frame: resampled through the map,
//!   with coverage outside the old frame zero.
//! - Value masks (luminance, colour range) have no geometry.
//!
//! Lens corrections are left out of the map, as in `stored_uv`: they move a
//! pixel by a fraction of a percent near the corners.

use crate::develop::geometry::Geometry;
use crate::develop::mask::{self, MaskShape};
use crate::develop::stack::EditStack;

/// Framed UV → oriented-image UV for one geometry: `M·(p − ½) + c`.
#[derive(Clone, Copy)]
struct Frame {
    m: [f32; 4],
    c: [f32; 2],
}

impl Frame {
    fn of(g: &Geometry, width: u32, height: u32) -> Self {
        let t = g.transform(width, height);
        Frame {
            m: t.matrix,
            c: t.centre,
        }
    }
    fn image_at(&self, p: (f32, f32)) -> (f32, f32) {
        let (x, y) = (p.0 - 0.5, p.1 - 0.5);
        (
            self.m[0] * x + self.m[1] * y + self.c[0],
            self.m[2] * x + self.m[3] * y + self.c[1],
        )
    }
    fn framed_at(&self, q: (f32, f32)) -> (f32, f32) {
        let inv = invert(self.m);
        let (x, y) = (q.0 - self.c[0], q.1 - self.c[1]);
        (inv[0] * x + inv[1] * y + 0.5, inv[2] * x + inv[3] * y + 0.5)
    }
    /// Image pixels per framed-UV unit along the frame's width.
    fn width_px(&self, width: u32, height: u32) -> f32 {
        (self.m[0] * width as f32).hypot(self.m[2] * height as f32)
    }
}

fn invert(m: [f32; 4]) -> [f32; 4] {
    let det = m[0] * m[3] - m[1] * m[2];
    let d = if det.abs() < 1e-12 { 1e-12 } else { det };
    [m[3] / d, -m[1] / d, -m[2] / d, m[0] / d]
}

fn mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
    ]
}

/// Moves `stack`'s masks and spots, placed in the frame `from` gave, to where
/// the same content is in the frame `stack.geometry` gives. `width`/`height`
/// are the oriented image in pixels (the session's). A no-op when the framing
/// did not change.
pub fn reframe(stack: &mut EditStack, from: &Geometry, width: u32, height: u32) {
    if *from == stack.geometry {
        return;
    }
    let old = Frame::of(from, width, height);
    let new = Frame::of(&stack.geometry, width, height);
    let point = |p: (f32, f32)| new.framed_at(old.image_at(p));
    // The linear part, old framed UV → new framed UV.
    let a = mul(invert(new.m), old.m);
    let area = (a[0] * a[3] - a[1] * a[2]).abs().sqrt();
    let width_ratio = old.width_px(width, height) / new.width_px(width, height).max(1e-6);

    for s in &mut stack.spots {
        (s.target_x, s.target_y) = point((s.target_x, s.target_y));
        (s.source_x, s.source_y) = point((s.source_x, s.source_y));
        s.radius *= area;
    }
    for m in &mut stack.masks {
        for c in &mut m.components {
            match &mut c.shape {
                MaskShape::Radial {
                    cx,
                    cy,
                    radius_x,
                    radius_y,
                    angle,
                    ..
                } => {
                    (*cx, *cy) = point((*cx, *cy));
                    (*radius_x, *radius_y, *angle) = ellipse(a, *radius_x, *radius_y, *angle);
                }
                MaskShape::Linear { x1, y1, x2, y2 } => {
                    (*x1, *y1) = point((*x1, *y1));
                    (*x2, *y2) = point((*x2, *y2));
                }
                MaskShape::Brush { strokes } => {
                    for s in strokes.iter_mut() {
                        (s.x, s.y) = point((s.x, s.y));
                        s.radius *= width_ratio;
                    }
                }
                MaskShape::Ai {
                    width: w,
                    height: h,
                    rle,
                    ..
                } => {
                    *rle = resample_ai(rle, *w, *h, &old, &new);
                }
                MaskShape::Luminance { .. } | MaskShape::ColorRange { .. } => {}
            }
        }
    }
}

/// An ellipse `(rx, ry, angle)` in UV under the linear map `a`: the shape
/// matrix `R·diag(rx², ry²)·Rᵀ` becomes `A·E·Aᵀ`, and its eigenvectors are the
/// new axes.
fn ellipse(a: [f32; 4], rx: f32, ry: f32, angle: f32) -> (f32, f32, f32) {
    let (s, c) = angle.sin_cos();
    let (rx2, ry2) = (rx * rx, ry * ry);
    // E = rx²·u·uᵀ + ry²·v·vᵀ, u = (c, s), v = (−s, c).
    let e = [
        rx2 * c * c + ry2 * s * s,
        (rx2 - ry2) * c * s,
        (rx2 - ry2) * c * s,
        rx2 * s * s + ry2 * c * c,
    ];
    let at = [a[0], a[2], a[1], a[3]];
    let e2 = mul(mul(a, e), at);
    let (p, b, q) = (e2[0], 0.5 * (e2[1] + e2[2]), e2[3]);
    let theta = 0.5 * (2.0 * b).atan2(p - q);
    let (st, ct) = theta.sin_cos();
    let along = p * ct * ct + 2.0 * b * st * ct + q * st * st;
    let across = p + q - along;
    (along.max(0.0).sqrt(), across.max(0.0).sqrt(), theta)
}

/// An AI mask's bitmap, resampled from the old frame to the new.
fn resample_ai(rle_b64: &str, w: u32, h: u32, old: &Frame, new: &Frame) -> String {
    use base64::Engine;
    let src = mask::decode_ai_sized(rle_b64, w, h);
    let (wu, hu) = (w as usize, h as usize);
    if src.len() < wu * hu || wu == 0 || hu == 0 {
        return rle_b64.to_string();
    }
    let sample = |u: f32, v: f32| -> f32 {
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return 0.0;
        }
        // Bilinear, at pixel centres.
        let x = (u * w as f32 - 0.5).clamp(0.0, (wu - 1) as f32);
        let y = (v * h as f32 - 0.5).clamp(0.0, (hu - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(wu - 1), (y0 + 1).min(hu - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |xx: usize, yy: usize| src[yy * wu + xx] as f32;
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    };
    let mut out = vec![0u8; wu * hu];
    for j in 0..hu {
        for i in 0..wu {
            let p = ((i as f32 + 0.5) / w as f32, (j as f32 + 0.5) / h as f32);
            let (u, v) = old.framed_at(new.image_at(p));
            out[j * wu + i] = sample(u, v).round().clamp(0.0, 255.0) as u8;
        }
    }
    base64::engine::general_purpose::STANDARD.encode(mask::rle_encode(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::geometry::CropRect;
    use crate::develop::mask::{Mask, MaskComponent, Stroke};
    use crate::develop::spot::Spot;

    const W: u32 = 6000;
    const H: u32 = 4000;

    fn cropped(x: f32, y: f32, w: f32, h: f32) -> Geometry {
        Geometry {
            crop: CropRect {
                x,
                y,
                width: w,
                height: h,
            },
            ..Default::default()
        }
    }

    /// Where a framed point is in the image, for a geometry.
    fn image_of(g: &Geometry, p: (f32, f32)) -> (f32, f32) {
        Frame::of(g, W, H).image_at(p)
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    fn with(spot: Spot, shape: MaskShape) -> EditStack {
        EditStack {
            spots: vec![spot],
            masks: vec![Mask {
                components: vec![MaskComponent {
                    shape,
                    inverted: false,
                    mode: Default::default(),
                }],
                adjustments: Default::default(),
                name: None,
            }],
            ..Default::default()
        }
    }

    fn spot(x: f32, y: f32, r: f32) -> Spot {
        Spot {
            target_x: x,
            target_y: y,
            source_x: x + 0.05,
            source_y: y,
            radius: r,
            feather: 0.5,
            heal: true,
        }
    }

    /// A spot and a radial on a subject, then a
    /// tighter crop — both still on the subject.
    #[test]
    fn a_recrop_keeps_a_spot_and_a_radial_on_the_subject() {
        let before = Geometry::default();
        let subject = image_of(&before, (0.6, 0.4));
        let mut stack = with(
            spot(0.6, 0.4, 0.02),
            MaskShape::Radial {
                cx: 0.6,
                cy: 0.4,
                radius_x: 0.1,
                radius_y: 0.1,
                angle: 0.0,
                feather: 0.3,
            },
        );
        stack.geometry = cropped(0.3, 0.2, 0.5, 0.5);
        reframe(&mut stack, &before, W, H);

        let s = &stack.spots[0];
        assert!(close(
            image_of(&stack.geometry, (s.target_x, s.target_y)),
            subject
        ));
        // Half the crop each way: twice the size in the frame.
        assert!((s.radius - 0.04).abs() < 1e-4, "{}", s.radius);
        let MaskShape::Radial {
            cx,
            cy,
            radius_x,
            radius_y,
            ..
        } = stack.masks[0].components[0].shape
        else {
            panic!()
        };
        assert!(close(image_of(&stack.geometry, (cx, cy)), subject));
        assert!(
            (radius_x - 0.2).abs() < 1e-3 && (radius_y - 0.2).abs() < 1e-3,
            "{radius_x} {radius_y}"
        );
    }

    /// A straighten and a quarter turn: the points stay on the content, and a
    /// brush dab stays the same size in image pixels.
    #[test]
    fn a_turn_and_a_straighten_keep_brush_dabs_on_the_content() {
        let before = cropped(0.1, 0.1, 0.8, 0.8);
        let dab = (0.3, 0.7);
        let target = image_of(&before, dab);
        let mut stack = with(
            spot(0.5, 0.5, 0.01),
            MaskShape::Brush {
                strokes: vec![Stroke {
                    x: dab.0,
                    y: dab.1,
                    radius: 0.02,
                    hardness: 0.5,
                    erase: false,
                }],
            },
        );
        let mut after = before;
        after.straighten = 7.5;
        after.quarter_turns = 1;
        stack.geometry = after;
        reframe(&mut stack, &before, W, H);

        let MaskShape::Brush { strokes } = &stack.masks[0].components[0].shape else {
            panic!()
        };
        assert!(close(
            image_of(&after, (strokes[0].x, strokes[0].y)),
            target
        ));
        let px = |g: &Geometry, r: f32| r * Frame::of(g, W, H).width_px(W, H);
        assert!((px(&after, strokes[0].radius) - px(&before, 0.02)).abs() < 0.5);
    }

    /// Reframing there and back is the identity: nothing drifts.
    #[test]
    fn there_and_back_puts_everything_where_it_was() {
        let a = cropped(0.0, 0.0, 1.0, 1.0);
        let mut b = cropped(0.2, 0.1, 0.6, 0.7);
        b.straighten = -4.0;
        b.flip = true;
        let original = with(
            spot(0.4, 0.6, 0.03),
            MaskShape::Radial {
                cx: 0.4,
                cy: 0.5,
                radius_x: 0.2,
                radius_y: 0.08,
                angle: 0.6,
                feather: 0.4,
            },
        );
        let mut stack = original.clone();
        stack.geometry = b;
        reframe(&mut stack, &a, W, H);
        stack.geometry = a;
        reframe(&mut stack, &b, W, H);

        let (s0, s1) = (&original.spots[0], &stack.spots[0]);
        assert!(close(
            (s0.target_x, s0.target_y),
            (s1.target_x, s1.target_y)
        ));
        let pick = |st: &EditStack| match st.masks[0].components[0].shape {
            MaskShape::Radial {
                cx,
                cy,
                radius_x,
                radius_y,
                ..
            } => (cx, cy, radius_x, radius_y),
            _ => panic!(),
        };
        let (o, n) = (pick(&original), pick(&stack));
        assert!((o.0 - n.0).abs() < 1e-4 && (o.1 - n.1).abs() < 1e-4);
        assert!(
            (o.2 - n.2).abs() < 1e-3 && (o.3 - n.3).abs() < 1e-3,
            "{o:?} {n:?}"
        );
    }

    #[test]
    fn an_ai_mask_is_resampled_onto_the_new_frame() {
        use base64::Engine;
        // Covered on the right half of the frame.
        let (w, h) = (40u32, 20u32);
        let mut buf = vec![0u8; (w * h) as usize];
        for j in 0..h {
            for i in w / 2..w {
                buf[(j * w + i) as usize] = 255;
            }
        }
        let rle = base64::engine::general_purpose::STANDARD.encode(mask::rle_encode(&buf));
        let before = Geometry::default();
        let mut stack = with(
            spot(0.5, 0.5, 0.01),
            MaskShape::Ai {
                label: "Subject".into(),
                width: w,
                height: h,
                rle,
            },
        );
        // The right half alone: now covered everywhere.
        stack.geometry = cropped(0.5, 0.0, 0.5, 1.0);
        reframe(&mut stack, &before, W, H);
        let MaskShape::Ai { rle, .. } = &stack.masks[0].components[0].shape else {
            panic!()
        };
        let out = mask::decode_ai_sized(rle, w, h);
        let covered = out.iter().filter(|&&v| v > 200).count();
        assert!(covered as f32 > 0.95 * (w * h) as f32, "{covered}");
    }
}
