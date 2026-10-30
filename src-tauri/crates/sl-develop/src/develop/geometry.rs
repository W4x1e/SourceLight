//! Crop, straighten, rotation and mirroring.
//!
//! All of it is a coordinate transform on an already-demosaiced image, so it
//! costs nothing per interaction: dragging a crop handle re-runs the cheap
//! develop pass, never the mosaic. That is the same reason EXIF orientation
//! lives in the develop shader rather than being baked into the cached texture.
//!
//! **The transform is composed here, on the CPU, and handed to the shader as a
//! matrix.** Getting it wrong produces a plausible image that is subtly
//! mis-framed or sheared, which is exactly the kind of bug a screenshot does
//! not settle — so the composition is unit-tested and the shader only applies
//! what it is given.
//!
//! Coordinates are normalised 0..1 in the **oriented** frame, meaning after
//! EXIF rotation. That is the frame the user sees and drags in; the shader
//! applies EXIF afterwards, on its way back to sensor space.

use serde::{Deserialize, Serialize};

/// The kept rectangle, normalised against the oriented frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for CropRect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl CropRect {
    pub fn is_full_frame(&self) -> bool {
        *self == Self::default()
    }

    /// Clamped to the frame, and never collapsed to nothing.
    ///
    /// A crop arrives from a drag gesture, so it can be inverted, oversized or
    /// zero-width before it reaches here. A zero-area crop would ask for a
    /// zero-pixel render, which fails at texture creation rather than anywhere
    /// informative.
    pub fn sanitised(&self) -> Self {
        const MIN: f32 = 0.02;
        let width = self.width.abs().clamp(MIN, 1.0);
        let height = self.height.abs().clamp(MIN, 1.0);
        Self {
            x: self.x.clamp(0.0, 1.0 - width),
            y: self.y.clamp(0.0, 1.0 - height),
            width,
            height,
        }
    }
}

/// Everything that changes the frame rather than the colour.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct Geometry {
    pub crop: CropRect,
    /// Levelling angle in degrees, positive clockwise. Small by nature —
    /// anything past a few degrees is a rotation, not a straighten.
    pub straighten: f32,
    /// User rotation in 90-degree steps, on top of the camera's own EXIF
    /// orientation. Kept separate because EXIF describes how the camera was
    /// held and this describes what the photographer wants.
    pub quarter_turns: u32,
    /// Mirror horizontally, after rotation.
    pub flip: bool,
}

/// The affine map from output coordinates to oriented-frame coordinates.
///
/// `oriented = matrix * (uv - 0.5) + centre`, with `uv` normalised against the
/// output. Stored row-major as `[xx, xy, yx, yy]` because it crosses into a
/// shader uniform, where a `mat2x2` would carry surprising column padding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropTransform {
    pub matrix: [f32; 4],
    pub centre: [f32; 2],
}

/// Where the viewer is looking, and how closely.
///
/// **Not part of [`Geometry`], and not part of the edit stack.** Zoom and pan
/// are what the user is looking at, not what the photograph is — they must not
/// land in history, persist to the database, or travel with a preset. Keeping
/// them out of `EditStack` is what guarantees that.
///
/// This is the piece that makes sharpening and noise reduction judgeable: both
/// act per pixel and cannot be assessed at fit-to-window, which is why the two
/// are still unbuilt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewPort {
    /// Output pixels per image pixel. 1.0 is one-to-one; below that the render
    /// is downscaled as before.
    pub zoom: f32,
    /// Centre of the visible region, normalised against the cropped frame.
    pub centre: [f32; 2],
    /// The area being drawn into, in pixels.
    pub width: u32,
    pub height: u32,
}

impl ViewPort {
    /// Clamps the centre so the view cannot be panned off the photograph.
    ///
    /// When the visible region is wider than the image — zoomed out past fit on
    /// one axis — there is nothing to pan, so the centre is pinned to the
    /// middle rather than allowed to drift and leave the image hanging off an
    /// edge.
    /// Shrinks the drawing area to the size the photo occupies at this zoom.
    ///
    /// **This is what stops a zoomed view stretching.** The visible region is
    /// `area / zoom` image pixels per axis, and once that exceeds the frame
    /// there is no more photograph to show — [`Self::anchored`] then clamps that
    /// axis to the whole frame while the other still shows a slice, so the
    /// sampled region and the target disagree about aspect ratio and the render
    /// fills the target anyway. Reducing the target instead means both axes stay
    /// unclamped and the difference shows up as letterboxing, which is what
    /// fit-to-window has always done.
    pub fn fitted(&self, framed: (u32, u32)) -> Self {
        let zoom = self.zoom.max(1e-3);
        let occupies = |frame: u32| ((frame as f32 * zoom).round() as u32).max(1);
        Self {
            width: self.width.min(occupies(framed.0)).max(1),
            height: self.height.min(occupies(framed.1)).max(1),
            ..*self
        }
    }

    /// The drawn-quad → framed-UV mapping `(extent, centre)`, for placing masks
    /// in the frame independently of the zoom. `framed = centre + (uv - 0.5) *
    /// extent`. This is [`Self::anchored`] under a public name — masks need the
    /// same clamped mapping the crop transform uses.
    pub fn framed_mapping(&self, framed: (u32, u32)) -> ([f32; 2], [f32; 2]) {
        self.anchored(framed)
    }

    fn anchored(&self, framed: (u32, u32)) -> ([f32; 2], [f32; 2]) {
        let zoom = self.zoom.max(1e-3);
        let extent = [
            (self.width as f32 / zoom / framed.0.max(1) as f32).min(1.0),
            (self.height as f32 / zoom / framed.1.max(1) as f32).min(1.0),
        ];
        let centre = [
            self.centre[0].clamp(extent[0] / 2.0, 1.0 - extent[0] / 2.0),
            self.centre[1].clamp(extent[1] / 2.0, 1.0 - extent[1] / 2.0),
        ];
        (extent, centre)
    }
}

impl CropTransform {
    /// Narrows this transform to the region a viewport is looking at.
    ///
    /// Both maps are affine, so they compose into one rather than becoming a
    /// second stage in the shader. Substituting
    /// `framed = centre + (uv - 0.5) * extent` into
    /// `oriented = matrix * (framed - 0.5) + crop_centre` gives a matrix scaled
    /// by the extent and a centre offset by where the viewport is looking —
    /// which is why zooming costs the develop pass exactly nothing.
    pub fn through(&self, view: &ViewPort, framed: (u32, u32)) -> CropTransform {
        let (extent, centre) = view.anchored(framed);
        let m = self.matrix;
        CropTransform {
            matrix: [
                m[0] * extent[0],
                m[1] * extent[1],
                m[2] * extent[0],
                m[3] * extent[1],
            ],
            centre: [
                m[0] * (centre[0] - 0.5) + m[1] * (centre[1] - 0.5) + self.centre[0],
                m[2] * (centre[0] - 0.5) + m[3] * (centre[1] - 0.5) + self.centre[1],
            ],
        }
    }
}

impl Geometry {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Quarter turns reduced to 0..=3.
    pub fn turns(&self) -> u32 {
        self.quarter_turns % 4
    }

    /// Output size in pixels for an oriented frame of `width` x `height`.
    ///
    /// An odd number of quarter turns swaps the axes; the straighten angle does
    /// not change the size, because the crop rectangle is defined in the
    /// straightened frame and keeping the canvas fixed is what lets a straighten
    /// be adjusted without the framing jumping around underneath it.
    pub fn output_size(&self, width: u32, height: u32) -> (u32, u32) {
        let crop = self.crop.sanitised();
        let w = ((width as f32 * crop.width).round() as u32).max(1);
        let h = ((height as f32 * crop.height).round() as u32).max(1);
        if self.turns() % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Builds the output-to-oriented transform.
    ///
    /// `width` and `height` are the oriented frame in pixels, needed because a
    /// rotation in normalised coordinates shears unless the aspect ratio is
    /// undone first and reapplied after.
    pub fn transform(&self, width: u32, height: u32) -> CropTransform {
        let crop = self.crop.sanitised();
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);

        // Output extent in oriented-frame *pixels*, before rotation. An odd
        // quarter turn means the output's x runs along the crop's y.
        let (ex, ey) = if self.turns() % 2 == 1 {
            (crop.height * h, crop.width * w)
        } else {
            (crop.width * w, crop.height * h)
        };

        // Rotation from output space back into the oriented frame: the user's
        // quarter turns and the straighten angle are the same kind of thing, so
        // they compose into one angle.
        //
        // Negated because this maps output *back* to source. Both terms carry
        // the same sign so the two controls turn the image the same way — with
        // them opposed, a quarter turn went anticlockwise while straighten went
        // clockwise, which reads as two unrelated controls.
        let angle = -(self.turns() as f32 * 90.0 + self.straighten).to_radians();
        let (sin, cos) = angle.sin_cos();

        // Columns are where the output's x and y axes land, in pixels.
        let fx = if self.flip { -1.0 } else { 1.0 };
        let col_x = [cos * ex * fx, sin * ex * fx];
        let col_y = [-sin * ey, cos * ey];

        // Back to normalised oriented coordinates, which is where the aspect
        // correction is undone.
        let (m0, m1) = (col_x[0] / w, col_y[0] / w);
        let (m2, m3) = (col_x[1] / h, col_y[1] / h);
        let (cx, cy) = (crop.x + crop.width / 2.0, crop.y + crop.height / 2.0);

        // **Auto-crop on straighten.** A rotation samples the source at the
        // rotated corners of the output rectangle; once straightened, those
        // corners fall outside [0,1] and the sampler clamps, smearing the edge
        // pixels into the corners. Shrinking the sampled region about the crop
        // centre — a slight zoom in — keeps every sample inside the frame, which
        // is the "minor crop" a straighten is expected to make. `s` is the
        // largest scale that fits, so it is exactly 1 at zero and at any quarter
        // turn (there the rectangle already fits) and only bites for the
        // in-between angles a straighten produces.
        let reach_x = 0.5 * (m0.abs() + m1.abs());
        let reach_y = 0.5 * (m2.abs() + m3.abs());
        let avail_x = cx.min(1.0 - cx).max(1e-4);
        let avail_y = cy.min(1.0 - cy).max(1e-4);
        let s = 1.0_f32
            .min(avail_x / reach_x.max(1e-6))
            .min(avail_y / reach_y.max(1e-6));

        CropTransform {
            matrix: [m0 * s, m1 * s, m2 * s, m3 * s],
            centre: [cx, cy],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The transform, pinned for one geometry with every part in play, and
    /// pinned to the same numbers in `src/lib/cropView.test.ts`, which mirrors
    /// it for the crop tool. A change here must change the mirror.
    #[test]
    fn the_transform_is_what_the_crop_tools_mirror_expects() {
        let g = Geometry {
            crop: CropRect {
                x: 0.25,
                y: 0.25,
                width: 0.4,
                height: 0.45,
            },
            straighten: -6.0,
            quarter_turns: 1,
            flip: true,
        };
        let t = g.transform(6000, 4000);
        let want = [-0.031_358_525_f32, 0.397_808_76, 0.447_534_86, 0.062_717_05];
        for (got, want) in t.matrix.iter().zip(want) {
            assert!((got - want).abs() < 1e-5, "{:?}", t.matrix);
        }
        assert_eq!(t.centre, [0.45, 0.475]);
    }

    /// **The stretch guard.**
    ///
    /// A zoomed view samples a region of the photograph and fills the target
    /// with it, so the moment the two disagree about aspect ratio the picture is
    /// silently distorted — and `anchored` clamps each axis *independently*,
    /// which is precisely how they come to disagree. Reported as "the image gets
    /// stretched when zooming": a portrait frame in a landscape canvas clamps
    /// the wide axis to the whole frame while the tall one still shows a slice.
    ///
    /// The property is scale-free on purpose. Asserting particular extents would
    /// re-encode the arithmetic; asserting that sampled shape equals drawn shape
    /// is the thing a viewer actually sees.
    #[test]
    fn a_zoomed_view_samples_the_same_shape_it_draws() {
        // Portrait sensor, landscape canvas — the reported case.
        let framed = (4024u32, 6024u32);
        for zoom in [0.05, 0.1743, 0.26, 0.5, 1.0, 3.0, 8.0] {
            let view = ViewPort {
                zoom,
                centre: [0.5, 0.5],
                width: 1850,
                height: 1050,
            }
            .fitted(framed);
            let (extent, _) = view.anchored(framed);

            let sampled = (extent[0] * framed.0 as f32) / (extent[1] * framed.1 as f32);
            let drawn = view.width as f32 / view.height as f32;
            assert!(
                (sampled / drawn - 1.0).abs() < 0.01,
                "at {zoom}x it samples {sampled:.4} and draws {drawn:.4}",
            );
        }
    }

    /// Zoomed in far enough to cover the canvas, nothing is given back: the
    /// whole area is photograph. A `fitted` that shrank here would letterbox a
    /// view that has no need of it and waste the pixels.
    #[test]
    fn a_view_larger_than_the_canvas_still_fills_it() {
        let asked = ViewPort {
            zoom: 1.0,
            centre: [0.5, 0.5],
            width: 1850,
            height: 1050,
        };
        let view = asked.fitted((4024, 6024));
        assert_eq!((view.width, view.height), (asked.width, asked.height));
    }

    /// Zoomed out far enough that the photo is smaller than the canvas, the
    /// target collapses onto the photo's own shape — which is what turns the
    /// difference into letterboxing instead of a stretch.
    #[test]
    fn a_view_smaller_than_the_canvas_matches_the_photo() {
        let view = ViewPort {
            zoom: 0.1,
            centre: [0.5, 0.5],
            width: 1850,
            height: 1050,
        }
        .fitted((4024, 6024));
        assert_eq!((view.width, view.height), (402, 602));
    }

    /// Applies a transform the way the shader does, for testing.
    fn map(t: &CropTransform, u: f32, v: f32) -> (f32, f32) {
        let (p, q) = (u - 0.5, v - 0.5);
        (
            t.matrix[0] * p + t.matrix[1] * q + t.centre[0],
            t.matrix[2] * p + t.matrix[3] * q + t.centre[1],
        )
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    /// **The straighten auto-crop guard.**
    ///
    /// A straightened full-frame crop must not sample outside the source — doing
    /// so clamps the edge and smears it into the corners, which is exactly what
    /// was reported. Every output corner must land inside [0,1]², and at zero
    /// straighten the transform must be untouched (no unwanted zoom).
    #[test]
    fn straighten_never_samples_past_the_frame() {
        let g = Geometry {
            straighten: 8.0,
            ..Geometry::default()
        };
        let t = g.transform(6000, 4000);
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)] {
            let (x, y) = map(&t, u, v);
            assert!(
                (-1e-4..=1.0 + 1e-4).contains(&x) && (-1e-4..=1.0 + 1e-4).contains(&y),
                "straightened corner ({u},{v}) sampled ({x:.4},{y:.4}) outside the frame",
            );
        }

        // Zero straighten (and no other geometry) is still the identity — the
        // auto-crop must not zoom an unrotated photo.
        let none = Geometry::default().transform(6000, 4000);
        assert!(close(map(&none, 0.0, 0.0), (0.0, 0.0)));
        assert!(close(map(&none, 1.0, 1.0), (1.0, 1.0)));
    }

    /// An untouched geometry must map every output pixel to itself. Every
    /// unedited photo takes this path, so a transform that was subtly off would
    /// shift the whole library by a fraction of a pixel and soften it.
    #[test]
    fn the_default_geometry_is_the_identity() {
        let g = Geometry::default();
        assert!(g.is_identity());
        assert_eq!(g.output_size(6000, 4000), (6000, 4000));

        let t = g.transform(6000, 4000);
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.5, 0.5), (0.25, 0.75)] {
            assert!(
                close(map(&t, u, v), (u, v)),
                "({u}, {v}) mapped to {:?}",
                map(&t, u, v)
            );
        }
    }

    /// The corners of the output have to land on the corners of the crop, or
    /// the preview shows a different rectangle than the one being dragged.
    #[test]
    fn the_output_corners_land_on_the_crop_corners() {
        let g = Geometry {
            crop: CropRect {
                x: 0.25,
                y: 0.1,
                width: 0.5,
                height: 0.4,
            },
            ..Default::default()
        };
        let t = g.transform(1000, 1000);
        assert!(close(map(&t, 0.0, 0.0), (0.25, 0.1)));
        assert!(close(map(&t, 1.0, 1.0), (0.75, 0.5)));
        assert!(close(map(&t, 0.5, 0.5), (0.5, 0.3)));
        assert_eq!(g.output_size(1000, 1000), (500, 400));
    }

    /// A quarter turn swaps the reported dimensions and moves the corners
    /// around, rather than merely relabelling the axes.
    #[test]
    fn a_quarter_turn_rotates_the_frame() {
        let g = Geometry {
            quarter_turns: 1,
            ..Default::default()
        };
        assert_eq!(g.output_size(6000, 4000), (4000, 6000));

        let t = g.transform(1000, 1000);
        // Turning the image one quarter clockwise means the output's top-left
        // reads from the source's bottom-left.
        assert!(
            close(map(&t, 0.0, 0.0), (0.0, 1.0)),
            "top-left mapped to {:?}",
            map(&t, 0.0, 0.0)
        );
        assert!(close(map(&t, 1.0, 0.0), (0.0, 0.0)));
        assert!(close(map(&t, 1.0, 1.0), (1.0, 0.0)));
    }

    /// Four quarter turns is a full circle and must be exactly the identity —
    /// not merely close, since it is reachable by clicking rotate four times.
    #[test]
    fn four_quarter_turns_return_to_the_start() {
        let g = Geometry {
            quarter_turns: 4,
            ..Default::default()
        };
        assert_eq!(g.turns(), 0);
        let t = g.transform(3000, 2000);
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.3, 0.8)] {
            assert!(close(map(&t, u, v), (u, v)));
        }
    }

    /// Straightening must rotate rather than shear.
    ///
    /// This is the failure a non-square frame invites: rotating normalised
    /// coordinates directly scales x and y differently, so the image skews and
    /// the horizon still is not level. Checked by confirming two perpendicular
    /// output edges stay perpendicular *in pixels* after mapping.
    #[test]
    fn straightening_rotates_without_shearing() {
        let g = Geometry {
            straighten: 10.0,
            ..Default::default()
        };
        let (w, h) = (6000.0f32, 4000.0f32);
        let t = g.transform(w as u32, h as u32);

        let origin = map(&t, 0.5, 0.5);
        let along_x = map(&t, 1.0, 0.5);
        let along_y = map(&t, 0.5, 1.0);

        // Back into pixels before measuring the angle between them.
        let ax = ((along_x.0 - origin.0) * w, (along_x.1 - origin.1) * h);
        let ay = ((along_y.0 - origin.0) * w, (along_y.1 - origin.1) * h);

        let dot = ax.0 * ay.0 + ax.1 * ay.1;
        let scale = (ax.0.hypot(ax.1)) * (ay.0.hypot(ay.1));
        assert!(
            (dot / scale).abs() < 1e-4,
            "output axes are no longer perpendicular: cos = {}",
            dot / scale
        );
    }

    /// Positive straighten has to turn the image the way the control says, or
    /// the slider fights the user. Positive is clockwise, the usual convention.
    #[test]
    fn straighten_turns_the_expected_way() {
        let g = Geometry {
            straighten: 10.0,
            ..Default::default()
        };
        let t = g.transform(1000, 1000);
        // Rotating the *image* clockwise means sampling along a line that runs
        // upward to the right, so a step right in the output moves up in the source.
        let origin = map(&t, 0.5, 0.5);
        let right = map(&t, 1.0, 0.5);
        assert!(
            right.1 < origin.1,
            "positive straighten turned the wrong way: {origin:?} -> {right:?}"
        );
    }

    /// Flipping mirrors horizontally and must not move anything vertically.
    #[test]
    fn flipping_mirrors_only_the_horizontal() {
        let g = Geometry {
            flip: true,
            ..Default::default()
        };
        let t = g.transform(1000, 1000);
        assert!(close(map(&t, 0.0, 0.25), (1.0, 0.25)));
        assert!(close(map(&t, 1.0, 0.75), (0.0, 0.75)));
    }

    /// A crop from a drag gesture can be inverted, oversized or empty. None of
    /// those may reach the renderer, where a zero-pixel target fails at texture
    /// creation rather than anywhere that explains itself.
    #[test]
    fn a_degenerate_crop_is_repaired_rather_than_rendered() {
        let cases = [
            CropRect {
                x: 0.8,
                y: 0.8,
                width: 0.5,
                height: 0.5,
            },
            CropRect {
                x: -0.5,
                y: -0.5,
                width: 2.0,
                height: 2.0,
            },
            CropRect {
                x: 0.5,
                y: 0.5,
                width: 0.0,
                height: 0.0,
            },
            CropRect {
                x: 0.5,
                y: 0.5,
                width: -0.3,
                height: -0.3,
            },
        ];
        for c in cases {
            let s = c.sanitised();
            assert!(s.width > 0.0 && s.height > 0.0, "{c:?} -> {s:?}");
            assert!(s.x >= 0.0 && s.y >= 0.0, "{c:?} -> {s:?}");
            assert!(s.x + s.width <= 1.0001, "{c:?} -> {s:?}");
            assert!(s.y + s.height <= 1.0001, "{c:?} -> {s:?}");

            let g = Geometry {
                crop: c,
                ..Default::default()
            };
            let (w, h) = g.output_size(6000, 4000);
            assert!(w > 0 && h > 0, "{c:?} produced a {w}x{h} render");
        }
    }

    /// At 1:1 on a viewport the size of the whole image, zooming must change
    /// nothing. This is the case that catches an off-by-one in the extent.
    #[test]
    fn a_viewport_matching_the_image_is_the_identity() {
        let g = Geometry::default();
        let t = g.transform(1000, 800);
        let view = ViewPort {
            zoom: 1.0,
            centre: [0.5, 0.5],
            width: 1000,
            height: 800,
        };
        let z = t.through(&view, (1000, 800));
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.7)] {
            assert!(
                close(map(&z, u, v), (u, v)),
                "({u}, {v}) mapped to {:?}",
                map(&z, u, v)
            );
        }
    }

    /// Zooming to 1:1 on a small viewport shows exactly that many image pixels,
    /// centred where asked. Getting the extent wrong here shows the wrong
    /// magnification, which is precisely what makes sharpening unjudgeable.
    #[test]
    fn zooming_shows_one_image_pixel_per_output_pixel() {
        let t = Geometry::default().transform(1000, 1000);
        let view = ViewPort {
            zoom: 1.0,
            centre: [0.5, 0.5],
            width: 200,
            height: 200,
        };
        let z = t.through(&view, (1000, 1000));

        // 200 output pixels at 1:1 covers 200 of 1000, so 0.2 of the frame.
        let left = map(&z, 0.0, 0.5).0;
        let right = map(&z, 1.0, 0.5).0;
        assert!(
            ((right - left) - 0.2).abs() < 1e-4,
            "covered {} of the frame, expected 0.2",
            right - left
        );
        assert!(close(map(&z, 0.5, 0.5), (0.5, 0.5)), "centre drifted");
    }

    /// Panning must move the view, and must stop at the edge of the photograph
    /// rather than sliding off into clamped border pixels.
    #[test]
    fn panning_moves_the_view_but_stops_at_the_edges() {
        let t = Geometry::default().transform(1000, 1000);
        let at = |cx: f32| {
            let view = ViewPort {
                zoom: 2.0,
                centre: [cx, 0.5],
                width: 500,
                height: 500,
            };
            map(&t.through(&view, (1000, 1000)), 0.5, 0.5).0
        };

        assert!(at(0.7) > at(0.5), "panning right did not move the view");

        // 500 pixels at 2x covers 250 of 1000, a quarter of the frame, so the
        // centre cannot go below 0.125 without showing beyond the edge.
        assert!(
            (at(-5.0) - 0.125).abs() < 1e-4,
            "panning past the left edge landed at {}",
            at(-5.0)
        );
        assert!((at(5.0) - 0.875).abs() < 1e-4, "past the right edge");
    }

    /// Zoomed out far enough that the whole frame fits, there is nothing to pan
    /// and the view must stay centred rather than drifting.
    #[test]
    fn a_view_wider_than_the_image_stays_centred() {
        let t = Geometry::default().transform(1000, 1000);
        let view = ViewPort {
            zoom: 0.1,
            centre: [0.9, 0.1],
            width: 500,
            height: 500,
        };
        assert!(close(
            map(&t.through(&view, (1000, 1000)), 0.5, 0.5),
            (0.5, 0.5)
        ));
    }

    /// Zoom composes with a crop rather than ignoring it: at 1:1 inside a
    /// cropped frame the view must stay within the crop.
    #[test]
    fn zooming_stays_inside_the_crop() {
        let g = Geometry {
            crop: CropRect {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            ..Default::default()
        };
        let framed = g.output_size(1000, 1000);
        let view = ViewPort {
            zoom: 1.0,
            centre: [0.5, 0.5],
            width: 100,
            height: 100,
        };
        let z = g.transform(1000, 1000).through(&view, framed);

        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.5, 0.5)] {
            let (x, y) = map(&z, u, v);
            assert!(
                (0.25..=0.75).contains(&x) && (0.25..=0.75).contains(&y),
                "({u}, {v}) escaped the crop to ({x}, {y})"
            );
        }
    }

    /// Crop and rotation together: the output size must reflect both, and a
    /// rotated crop must still read from inside the crop rectangle.
    #[test]
    fn a_rotated_crop_stays_inside_its_rectangle() {
        let g = Geometry {
            crop: CropRect {
                x: 0.1,
                y: 0.2,
                width: 0.4,
                height: 0.6,
            },
            quarter_turns: 1,
            ..Default::default()
        };
        // 0.6 of the height becomes the output's width.
        assert_eq!(g.output_size(1000, 1000), (600, 400));

        let t = g.transform(1000, 1000);
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)] {
            let (x, y) = map(&t, u, v);
            assert!(
                (0.099..=0.501).contains(&x) && (0.199..=0.801).contains(&y),
                "({u}, {v}) mapped outside the crop to ({x}, {y})"
            );
        }
    }
}
