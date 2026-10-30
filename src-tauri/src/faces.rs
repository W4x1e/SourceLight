//! On-device face analysis for culling (phase 2a): find faces, and per face
//! score focus and whether the eyes are open — the signals that decide which
//! frame of a group of people survives a cull.
//!
//! Split like the tagger (`tagger/builtin.rs`): the pure geometry and
//! aggregation live here and are always compiled and unit-tested; the ONNX
//! sessions sit behind `--features ai`. The models are **YuNet** (detection and
//! five landmarks, MIT) and, for the eyes, MediaPipe's **face mesh** and
//! **blendshape** models (Apache-2.0): 478 landmarks, then 52 expression scores
//! among which `eyeBlink` is kept apart from `eyeSquint`, so a smile no longer
//! reads as a blink. The AI installer carries them (`ai_models::BUNDLED`).
//!
//! Detection results are stored per face (box + landmarks), not only aggregated,
//! because a later people-grouping phase (2b) re-uses the boxes to embed each
//! face rather than re-detecting.

/// One detected face, in the pixel coordinates of the image the detector ran on.
/// The five landmarks are YuNet's, in order: right eye, left eye, nose tip,
/// right mouth corner, left mouth corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Detector confidence, 0..1.
    pub score: f32,
    /// Five `(x, y)` landmarks, image pixels.
    pub landmarks: [[f32; 2]; 5],
    /// Focus of the face crop (same variance-of-Laplacian metric as
    /// `quality::sharpness`, on the face region). Higher is sharper. Filled in
    /// after detection; 0 until then.
    pub sharpness: f32,
    /// Eyes-open score, 0 (shut) … 1 (open), below 0.5 meaning shut (see
    /// [`eye_open_from`]). `None` is "can't tell": a profile, sunglasses, a
    /// face too small to read, or not scored at all. Never read as shut.
    pub eye_open: Option<f32>,
    /// Whether the eyes were read with the face mesh, so a pass that could not
    /// (models missing) is picked up by the next one.
    pub eyes_read: bool,
}

impl Face {
    /// A face's box as `[x, y, w, h]`, for NMS.
    pub fn bbox(&self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }

    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }
}

/// Per-photo face signals — the cached aggregates the cull UI reads without a
/// join to the `faces` table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceSummary {
    pub count: usize,
    /// Focus of the primary (largest) face — the relevant sharpness when the
    /// subject is a person.
    pub primary_sharpness: Option<f32>,
    /// Lowest eye-open across the faces whose eyes could be read, so a single
    /// blink in a group pulls it down. `None` when none could be.
    pub min_eye_open: Option<f32>,
}

/// Reduce a photo's faces to the signals stored on its row. The primary face is
/// the largest by area (the subject); the eye score is the worst across all
/// faces so any blink is caught.
pub fn summarize(faces: &[Face]) -> FaceSummary {
    let primary = faces.iter().max_by(|a, b| a.area().total_cmp(&b.area()));
    let min_eye_open = faces
        .iter()
        .filter_map(|f| f.eye_open)
        .fold(None, |acc, e| Some(acc.map_or(e, |a: f32| a.min(e))));
    FaceSummary {
        count: faces.len(),
        primary_sharpness: primary.map(|f| f.sharpness),
        min_eye_open,
    }
}

/// Intersection-over-union of two `[x, y, w, h]` boxes.
pub fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ax2 = a[0] + a[2];
    let ay2 = a[1] + a[3];
    let bx2 = b[0] + b[2];
    let by2 = b[1] + b[3];
    let ix = (ax2.min(bx2) - a[0].max(b[0])).max(0.0);
    let iy = (ay2.min(by2) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Greedy non-max suppression: keep the highest-scoring box, drop everything
/// overlapping it beyond `iou_thresh`, repeat. Returns the surviving indices,
/// best first.
pub fn nms(boxes: &[[f32; 4]], scores: &[f32], iou_thresh: f32) -> Vec<usize> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&i, &j| scores[j].total_cmp(&scores[i]));
    let mut keep = Vec::new();
    let mut dropped = vec![false; boxes.len()];
    for idx in 0..order.len() {
        let i = order[idx];
        if dropped[i] {
            continue;
        }
        keep.push(i);
        for &j in &order[idx + 1..] {
            if !dropped[j] && iou(boxes[i], boxes[j]) > iou_thresh {
                dropped[j] = true;
            }
        }
    }
    keep
}

// ------------------------------------------------------------------------
// Eyes: MediaPipe face mesh, then its blendshapes.
// ------------------------------------------------------------------------

/// The face mesh model's square input edge.
pub const MESH_EDGE: u32 = 256;

/// The 146 mesh landmarks the blendshape model reads, in its order (from
/// MediaPipe's `face_blendshapes_graph.cc`).
pub const BLENDSHAPE_SUBSET: [usize; 146] = [
    0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 17, 21, 33, 37, 39, 40, 46, 52, 53, 54, 55, 58, 61, 63, 65,
    66, 67, 70, 78, 80, 81, 82, 84, 87, 88, 91, 93, 95, 103, 105, 107, 109, 127, 132, 133, 136,
    144, 145, 146, 148, 149, 150, 152, 153, 154, 155, 157, 158, 159, 160, 161, 162, 163, 168, 172,
    173, 176, 178, 181, 185, 191, 195, 197, 234, 246, 249, 251, 263, 267, 269, 270, 276, 282, 283,
    284, 285, 288, 291, 293, 295, 296, 297, 300, 308, 310, 311, 312, 314, 317, 318, 321, 323, 324,
    332, 334, 336, 338, 356, 361, 362, 365, 373, 374, 375, 377, 378, 379, 380, 381, 382, 384, 385,
    386, 387, 388, 389, 390, 397, 398, 400, 402, 405, 409, 415, 454, 466, 468, 469, 470, 471, 472,
    473, 474, 475, 476, 477,
];

/// `eyeBlinkLeft` and `eyeBlinkRight` among the 52 blendshapes.
pub const BLINK_LEFT: usize = 9;
pub const BLINK_RIGHT: usize = 10;

/// The square the mesh model looks at: centre, side and roll, in image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Roi {
    pub cx: f32,
    pub cy: f32,
    pub side: f32,
    /// Roll in radians: positive turns the crop clockwise on screen.
    pub angle: f32,
}

impl Roi {
    /// The first look: half again the detector's box, upright.
    pub fn around_box(b: [f32; 4]) -> Roi {
        Roi {
            cx: b[0] + b[2] / 2.0,
            cy: b[1] + b[3] / 2.0,
            side: b[2].max(b[3]) * 1.5,
            angle: 0.0,
        }
    }

    /// The second look, as MediaPipe's tracking takes it: centred on the mesh
    /// the first look found, the outer eye corners (33, 263) level, half again
    /// the mesh's span. On a tilted head this is what puts the eyelids where
    /// the blendshape model expects them.
    pub fn around_mesh(pts: &[[f32; 2]]) -> Roi {
        let (r, l) = (pts[33], pts[263]);
        let n = pts.len().max(1) as f32;
        let (sx, sy) = pts
            .iter()
            .fold((0.0, 0.0), |(a, b), p| (a + p[0], b + p[1]));
        let span = |k: usize| {
            let (lo, hi) = pts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                (lo.min(p[k]), hi.max(p[k]))
            });
            hi - lo
        };
        Roi {
            cx: sx / n,
            cy: sy / n,
            side: span(0).max(span(1)) * 1.5,
            angle: (l[1] - r[1]).atan2(l[0] - r[0]),
        }
    }

    /// Where the crop's pixel `(u, v)` (0..[`MESH_EDGE`]) lies in the image.
    pub fn to_image(&self, u: f32, v: f32) -> [f32; 2] {
        let half = MESH_EDGE as f32 / 2.0;
        let s = self.side / MESH_EDGE as f32;
        let (sn, cs) = self.angle.sin_cos();
        let (du, dv) = ((u - half) * s, (v - half) * s);
        [self.cx + cs * du - sn * dv, self.cy + sn * du + cs * dv]
    }

    /// The crop as the mesh model's input: planar RGB in 0..1, bilinear, black
    /// outside the image (as MediaPipe pads).
    pub fn tensor(&self, rgb: &image::RgbImage) -> Vec<f32> {
        let n = MESH_EDGE;
        let plane = (n * n) as usize;
        let mut out = vec![0.0f32; 3 * plane];
        let (w, h) = (rgb.width(), rgb.height());
        if w == 0 || h == 0 {
            return out;
        }
        for v in 0..n {
            for u in 0..n {
                let [x, y] = self.to_image(u as f32 + 0.5, v as f32 + 0.5);
                let (x, y) = (x - 0.5, y - 0.5);
                if x < -0.5 || y < -0.5 || x > w as f32 - 0.5 || y > h as f32 - 0.5 {
                    continue;
                }
                let px = sample_bilinear(rgb, x, y, w, h);
                let i = (v * n + u) as usize;
                out[i] = px[0] / 255.0;
                out[plane + i] = px[1] / 255.0;
                out[2 * plane + i] = px[2] / 255.0;
            }
        }
        out
    }
}

/// The mesh's 478 `(x, y, z)` crop-pixel landmarks, as image pixels.
pub fn mesh_to_image(raw: &[f32], roi: &Roi) -> Vec<[f32; 2]> {
    raw.chunks_exact(3)
        .take(478)
        .map(|p| roi.to_image(p[0], p[1]))
        .collect()
}

/// How open the eyes are by shape alone: the lid gap over the eye's width,
/// the smaller of the two eyes. About 0.25–0.35 for an open eye, under 0.1
/// shut or looking right down.
pub fn eye_aspect(pts: &[[f32; 2]]) -> f32 {
    let d = |a: usize, b: usize| (pts[a][0] - pts[b][0]).hypot(pts[a][1] - pts[b][1]);
    let right = d(159, 145) / d(33, 133).max(1e-3);
    let left = d(386, 374) / d(362, 263).max(1e-3);
    right.min(left)
}

/// What the mesh and blendshapes say about one face's eyes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyeReading {
    /// The mesh model's confidence that a face is there at all, 0..1.
    pub presence: f32,
    /// `eyeBlink`, the smaller of the two eyes: a blink closes both, and a
    /// single high reading is more often a pair of sunglasses or a turned face.
    pub blink: f32,
    /// [`eye_aspect`].
    pub aspect: f32,
}

/// Below this presence the mesh has not found a face: a profile, the back of
/// a head, a false detection. Measured: every one of those read under 0.2.
const MIN_PRESENCE: f32 = 0.9;
/// Below this width (preview pixels) an eye is a few pixels tall and the
/// readings drift toward "shut" whatever the eye does.
pub const MIN_EYE_FACE_PX: f32 = 64.0;

/// The eye-open score stored for a face, or `None` for can't tell.
///
/// Calibrated on the owner's real candid photos and on portraits with eyes
/// plainly shut (Wikimedia Commons), 2026-09-28:
///
/// - Open eyes on readable faces, smiling or not, read `blink` 0.04–0.47; not
///   one of ~50 went over. OCEC, before, read most of them shut.
/// - Shut eyes read 0.46–0.77, a lower ceiling than the model's reputation.
///   The lid gap separates what the score alone does not: shut eyes measure
///   0.03–0.10, open ones 0.13 and up.
/// - Looking down sits between (blink 0.43–0.52, gap 0.06–0.11). Some of it
///   reads shut. The lids *are* down, which a culler wants to know anyway.
///
/// So: shut when both eyes' blink is at least 0.6, or at least 0.45 with the
/// lids nearly touching. The score keeps the old meaning (under 0.5 is shut),
/// so the badge's threshold is unchanged.
pub fn eye_open_from(r: &EyeReading, face_w: f32) -> Option<f32> {
    if r.presence < MIN_PRESENCE || face_w < MIN_EYE_FACE_PX {
        return None;
    }
    let shut = r.blink >= 0.6 || (r.aspect <= 0.10 && r.blink >= 0.45);
    let open = (1.0 - r.blink).clamp(0.0, 1.0);
    Some(if shut { open.min(0.45) } else { open.max(0.55) })
}

/// One raw YuNet detection in the detector's own (letterboxed) input pixels,
/// before it is scaled back onto the original image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawDet {
    /// `[x, y, w, h]` in detector-input pixels.
    pub bbox: [f32; 4],
    pub score: f32,
    pub landmarks: [[f32; 2]; 5],
}

/// Decode one YuNet stride head into candidate detections. The four slices are
/// the model's `cls_*`, `obj_*`, `bbox_*` and `kps_*` outputs for this stride,
/// anchor-major (one value per anchor for `cls`/`obj`, four for `bbox`, ten for
/// `kps`); `cols` is the feature-map width (input / stride). YuNet's confidence
/// is `sqrt(cls · obj)`; anchors below `score_thresh` are dropped. Boxes come
/// back as `[x, y, w, h]` in the detector's input pixels.
pub fn decode_stride(
    stride: u32,
    cols: u32,
    cls: &[f32],
    obj: &[f32],
    bbox: &[f32],
    kps: &[f32],
    score_thresh: f32,
) -> Vec<RawDet> {
    let s = stride as f32;
    let cols = cols.max(1);
    let mut out = Vec::new();
    for idx in 0..cls.len() {
        let cl = cls[idx].clamp(0.0, 1.0);
        let ob = obj.get(idx).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let score = (cl * ob).sqrt();
        if score < score_thresh {
            continue;
        }
        // The prior sits at the top-left corner of cell (col, row); the bbox
        // regression gives a centre offset in cells and log-space size.
        let c = (idx as u32 % cols) as f32;
        let r = (idx as u32 / cols) as f32;
        let cx = (c + bbox[idx * 4]) * s;
        let cy = (r + bbox[idx * 4 + 1]) * s;
        let w = bbox[idx * 4 + 2].exp() * s;
        let h = bbox[idx * 4 + 3].exp() * s;
        let mut landmarks = [[0.0f32; 2]; 5];
        for (k, lm) in landmarks.iter_mut().enumerate() {
            *lm = [
                (c + kps[idx * 10 + 2 * k]) * s,
                (r + kps[idx * 10 + 2 * k + 1]) * s,
            ];
        }
        out.push(RawDet {
            bbox: [cx - w / 2.0, cy - h / 2.0, w, h],
            score,
            landmarks,
        });
    }
    out
}

// ------------------------------------------------------------------------
// Face portraits for the People page (pure, tested).
// ------------------------------------------------------------------------

/// Edge of a cached face portrait, in pixels: twice a People tile's 162, so a
/// high-density screen shows it sharp too.
pub const PORTRAIT_EDGE: u32 = 324;

/// How much of the portrait the face box fills; the rest is the hair,
/// shoulders and air that make it read as a person rather than a swatch.
const PORTRAIT_FILL: f32 = 0.6;

/// The square to cut for a face's portrait, as `(left, top, side)` in image
/// pixels: centred on the face, `1 / PORTRAIT_FILL` times its larger edge,
/// and slid (not shrunk) to stay inside the image. Shrinks only when the image
/// itself is smaller than the square.
pub fn portrait_square(face: [f32; 4], img_w: u32, img_h: u32) -> (u32, u32, u32) {
    let [x, y, w, h] = face;
    let side = (w.max(h) / PORTRAIT_FILL).round().max(1.0) as u32;
    let side = side.min(img_w).min(img_h).max(1);
    let cx = x + w / 2.0;
    // A touch above centre: the eyes, not the chin, belong in the middle.
    let cy = y + h * 0.45;
    let left = (cx - side as f32 / 2.0)
        .round()
        .clamp(0.0, (img_w - side) as f32) as u32;
    let top = (cy - side as f32 / 2.0)
        .round()
        .clamp(0.0, (img_h - side) as f32) as u32;
    (left, top, side)
}

/// A face's portrait, [`PORTRAIT_EDGE`] square, from the image its box was
/// measured on. Resampled with Lanczos: a small face scaled up that way stays
/// far crisper than the browser's bilinear stretch of a cropped thumbnail,
/// which is what the People page used to show.
pub fn portrait(image: &image::RgbImage, face: [f32; 4]) -> image::RgbImage {
    let (left, top, side) = portrait_square(face, image.width(), image.height());
    let crop = image::imageops::crop_imm(image, left, top, side, side).to_image();
    image::imageops::resize(
        &crop,
        PORTRAIT_EDGE,
        PORTRAIT_EDGE,
        image::imageops::FilterType::Lanczos3,
    )
}

// ------------------------------------------------------------------------
// Face alignment for the embedder (pure, tested).
// ------------------------------------------------------------------------

/// The ArcFace 5-point template — canonical landmark positions (left eye, right
/// eye, nose, left mouth corner, right mouth corner) on a 112×112 crop. Face
/// embedders (AuraFace / ArcFace / MobileFaceNet) all expect a face warped onto it.
pub const ARCFACE_TEMPLATE: [[f32; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];

/// Aligned-crop edge the embedder consumes.
pub const ALIGN_EDGE: u32 = 112;

/// YuNet's landmark order matches the ArcFace template index-for-index by image
/// position: YuNet's landmark 0 ("the person's right eye") sits at image-left,
/// exactly where the template's first point (38, 51) is — so the correspondence
/// is the identity, no reordering needed.
fn to_template_order(lm: &[[f32; 2]; 5]) -> [[f32; 2]; 5] {
    *lm
}

/// Solve a 4×4 linear system by Gaussian elimination with partial pivoting.
#[allow(clippy::needless_range_loop)]
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> [f64; 4] {
    for i in 0..4 {
        let mut piv = i;
        for r in i + 1..4 {
            if a[r][i].abs() > a[piv][i].abs() {
                piv = r;
            }
        }
        a.swap(i, piv);
        b.swap(i, piv);
        let d = a[i][i];
        if d.abs() < 1e-12 {
            continue;
        }
        for k in i..4 {
            a[i][k] /= d;
        }
        b[i] /= d;
        for r in 0..4 {
            if r != i {
                let f = a[r][i];
                for k in i..4 {
                    a[r][k] -= f * a[i][k];
                }
                b[r] -= f * b[i];
            }
        }
    }
    b
}

/// Least-squares similarity transform (uniform scale + rotation + translation)
/// mapping `src` points onto `dst`, as a 2×3 affine `[[a, -b, tx], [b, a, ty]]`
/// (so `u = a·x − b·y + tx`, `v = b·x + a·y + ty`). Over-determined by the five
/// landmarks, solved via the normal equations.
pub fn similarity_transform(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> [[f32; 3]; 2] {
    let mut ata = [[0.0f64; 4]; 4];
    let mut atb = [0.0f64; 4];
    for i in 0..5 {
        let (x, y) = (src[i][0] as f64, src[i][1] as f64);
        for (row, t) in [
            ([x, -y, 1.0, 0.0], dst[i][0] as f64),
            ([y, x, 0.0, 1.0], dst[i][1] as f64),
        ] {
            for j in 0..4 {
                atb[j] += row[j] * t;
                for k in 0..4 {
                    ata[j][k] += row[j] * row[k];
                }
            }
        }
    }
    let p = solve4(ata, atb);
    let (a, b, tx, ty) = (p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32);
    [[a, -b, tx], [b, a, ty]]
}

/// Invert a 2×3 affine transform.
fn invert_affine(m: [[f32; 3]; 2]) -> [[f32; 3]; 2] {
    let (a, c, e) = (m[0][0], m[0][1], m[0][2]);
    let (b, d, f) = (m[1][0], m[1][1], m[1][2]);
    let det = a * d - b * c;
    let inv = if det.abs() < 1e-12 { 0.0 } else { 1.0 / det };
    [
        [d * inv, -c * inv, (c * f - d * e) * inv],
        [-b * inv, a * inv, (b * e - a * f) * inv],
    ]
}

/// Bilinear RGB sample at a fractional pixel, edges clamped.
fn sample_bilinear(rgb: &image::RgbImage, x: f32, y: f32, w: u32, h: u32) -> [f32; 3] {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |xx: u32, yy: u32| {
        let p = rgb.get_pixel(xx, yy).0;
        [p[0] as f32, p[1] as f32, p[2] as f32]
    };
    let (p00, p10, p01, p11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let mut out = [0.0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let top = p00[c] * (1.0 - fx) + p10[c] * fx;
        let bot = p01[c] * (1.0 - fx) + p11[c] * fx;
        *o = top * (1.0 - fy) + bot * fy;
    }
    out
}

/// Warp a face onto the 112×112 ArcFace template from its five landmarks and emit
/// the planar NCHW RGB tensor the embedder consumes, normalised `(px−127.5)/127.5`
/// (InsightFace's ArcFace convention, which AuraFace follows).
/// Alignment (rather than a plain box crop) is what makes the embedding robust to
/// pose and framing, so the same person matches across shots.
pub fn align_112(rgb: &image::RgbImage, landmarks: &[[f32; 2]; 5]) -> Vec<f32> {
    let n = ALIGN_EDGE;
    let plane = (n * n) as usize;
    let (w, h) = (rgb.width(), rgb.height());
    if w == 0 || h == 0 {
        return vec![0.0; 3 * plane];
    }
    let inv = invert_affine(similarity_transform(
        &to_template_order(landmarks),
        &ARCFACE_TEMPLATE,
    ));
    let mut out = vec![0f32; 3 * plane];
    for oy in 0..n {
        for ox in 0..n {
            let sx = inv[0][0] * ox as f32 + inv[0][1] * oy as f32 + inv[0][2];
            let sy = inv[1][0] * ox as f32 + inv[1][1] * oy as f32 + inv[1][2];
            let rgbv = sample_bilinear(rgb, sx, sy, w, h);
            let i = (oy * n + ox) as usize;
            out[i] = (rgbv[0] - 127.5) / 127.5;
            out[plane + i] = (rgbv[1] - 127.5) / 127.5;
            out[2 * plane + i] = (rgbv[2] - 127.5) / 127.5;
        }
    }
    out
}

// ------------------------------------------------------------------------
// ONNX sessions — only with `--features ai`.
// ------------------------------------------------------------------------
#[cfg(feature = "ai")]
pub use imp::FaceEngine;

#[cfg(feature = "ai")]
mod imp {
    use super::*;
    use anyhow::{Context, Result};
    use std::path::Path;

    /// YuNet's fixed square input edge; the image is letterboxed into it.
    const YUNET_INPUT: u32 = 640;
    /// Minimum `sqrt(cls·obj)` confidence to keep a detection. Real frontal faces
    /// score ~0.9; texture false-positives (validated on nature frames) stay
    /// below ~0.35, so this leaves clear margin while still catching softer faces.
    const YUNET_CONF: f32 = 0.7;
    /// IoU above which the lower-scoring of two overlapping faces is suppressed.
    const YUNET_NMS_IOU: f32 = 0.3;
    /// The face mesh and blendshape models, under `<models>/faces`.
    pub const MESH_FILE: &str = "mesh.onnx";
    pub const BLENDSHAPES_FILE: &str = "blendshapes.onnx";

    /// The people-grouping embedder: AuraFace v1 (fal, Apache-2.0), an
    /// ArcFace ResNet100 trained on commercially licensed data. It replaced
    /// FaceX nano, which separated people barely better than chance on a real
    /// library (AUC 0.69 against 0.90). See `docs/AI-MODEL-SPIKES.md`.
    pub const EMBEDDER_FILE: &str = "auraface.onnx";

    /// YuNet, the eye models and the optional AuraFace embedder, loaded once
    /// and reused.
    pub struct FaceEngine {
        yunet: std::sync::Mutex<ort::session::Session>,
        /// The face mesh and its blendshapes; `None` for a models folder from
        /// before they were added, whose faces then wait to be read.
        eyes: Option<(
            std::sync::Mutex<ort::session::Session>,
            std::sync::Mutex<ort::session::Session>,
        )>,
        /// The people-grouping embedder — optional so a library that only culls
        /// (a "Face tools" download from before the embedder was added, or
        /// one still carrying FaceX) still loads and culls.
        embedder: Option<std::sync::Mutex<ort::session::Session>>,
    }

    impl FaceEngine {
        /// Loads `faces/yunet.onnx`, the eye models and [`EMBEDDER_FILE`] where
        /// present, from the models dir. Loading proves the files parse;
        /// running is [`Self::detect`] / [`Self::embed`].
        pub fn load(dir: &Path) -> Result<Self> {
            crate::develop::ai::ensure_ort_dylib();
            let yunet = build_session(&dir.join("yunet.onnx"))?;
            let (mesh, blend) = (dir.join(MESH_FILE), dir.join(BLENDSHAPES_FILE));
            let eyes = if mesh.exists() && blend.exists() {
                Some((
                    std::sync::Mutex::new(build_session(&mesh)?),
                    std::sync::Mutex::new(build_session(&blend)?),
                ))
            } else {
                None
            };
            let embedder_path = dir.join(EMBEDDER_FILE);
            let embedder = if embedder_path.exists() {
                Some(std::sync::Mutex::new(build_session(&embedder_path)?))
            } else {
                None
            };
            Ok(Self {
                yunet: std::sync::Mutex::new(yunet),
                eyes,
                embedder,
            })
        }

        /// Whether the people-grouping embedder is available.
        pub fn can_embed(&self) -> bool {
            self.embedder.is_some()
        }

        /// Whether the eye models are there to read eyes with.
        pub fn can_read_eyes(&self) -> bool {
            self.eyes.is_some()
        }

        /// Reads the eyes of faces already found in `image`, by their
        /// boxes: the backfill for faces scored before the face mesh, and the
        /// same path a fresh detection takes. One entry per box, `None` where
        /// the eyes can't be read.
        pub fn read_eyes(&self, image: &[u8], boxes: &[[f32; 4]]) -> Result<Vec<Option<f32>>> {
            let rgb = image::load_from_memory(image)
                .context("decoding image for reading eyes")?
                .to_rgb8();
            boxes.iter().map(|b| self.eyes_of(&rgb, *b)).collect()
        }

        /// One face's eye-open score: the mesh on the box, then again on the
        /// crop it implies (level, centred), then the blendshapes.
        fn eyes_of(&self, rgb: &image::RgbImage, b: [f32; 4]) -> Result<Option<f32>> {
            let Some((mesh, blend)) = &self.eyes else {
                return Ok(None);
            };
            let run_mesh = |roi: &Roi| -> Result<(Vec<[f32; 2]>, f32)> {
                let mut sess = crate::sync::lock_recover(mesh);
                let name = sess
                    .inputs
                    .first()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "input_12".to_string());
                let e = MESH_EDGE as i64;
                let tensor = ort::value::Value::from_array(([1_i64, 3, e, e], roi.tensor(rgb)))?;
                let outputs = sess.run(ort::inputs![name.as_str() => tensor])?;
                let pts = mesh_to_image(&tensor_vec(&outputs, "Identity")?, roi);
                let logit = tensor_vec(&outputs, "Identity_1")?
                    .first()
                    .copied()
                    .unwrap_or(-10.0);
                Ok((pts, 1.0 / (1.0 + (-logit).exp())))
            };
            let (first, _) = run_mesh(&Roi::around_box(b))?;
            if first.len() < 478 {
                return Ok(None);
            }
            // Twice more, each look centred and levelled on the last: from a
            // loose first box one refinement can still sit off the face, and a
            // detector's box is looser on a close portrait than on a candid.
            let (second, _) = run_mesh(&Roi::around_mesh(&first))?;
            if second.len() < 478 {
                return Ok(None);
            }
            let (pts, presence) = run_mesh(&Roi::around_mesh(&second))?;
            if pts.len() < 478 {
                return Ok(None);
            }
            let subset: Vec<f32> = BLENDSHAPE_SUBSET
                .iter()
                .flat_map(|&i| [pts[i][0], pts[i][1]])
                .collect();
            let shapes = {
                let mut sess = crate::sync::lock_recover(blend);
                let name = sess
                    .inputs
                    .first()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "input_points".to_string());
                let tensor = ort::value::Value::from_array(([1_i64, 146, 2], subset))?;
                let outputs = sess.run(ort::inputs![name.as_str() => tensor])?;
                let (_shape, data) = outputs[0].try_extract_tensor::<f32>()?;
                data.to_vec()
            };
            if shapes.len() <= BLINK_RIGHT {
                return Ok(None);
            }
            let reading = EyeReading {
                presence,
                blink: shapes[BLINK_LEFT].min(shapes[BLINK_RIGHT]),
                aspect: eye_aspect(&pts),
            };
            Ok(eye_open_from(&reading, b[2]))
        }

        /// A face's identity embedding (512-d, L2-normalised): align to the
        /// template from the landmarks, run AuraFace. Cosine similarity of two
        /// of these says whether they're the same person. Errors if the
        /// embedder wasn't downloaded.
        pub fn embed(&self, image: &[u8], landmarks: &[[f32; 2]; 5]) -> Result<Vec<f32>> {
            Ok(self
                .embed_all(image, std::slice::from_ref(landmarks))?
                .remove(0))
        }

        /// [`Self::embed`] for every face in one photo, decoding it once.
        pub fn embed_all(
            &self,
            image: &[u8],
            landmarks: &[[[f32; 2]; 5]],
        ) -> Result<Vec<Vec<f32>>> {
            let rgb = image::load_from_memory(image)
                .context("decoding image for face embedding")?
                .to_rgb8();
            self.embed_all_rgb(&rgb, landmarks)
        }

        /// [`Self::embed_all`] on an image already decoded — the one
        /// [`Self::detect_rgb`] just found the faces in.
        pub fn embed_all_rgb(
            &self,
            rgb: &image::RgbImage,
            landmarks: &[[[f32; 2]; 5]],
        ) -> Result<Vec<Vec<f32>>> {
            let embedder = self.embedder.as_ref().with_context(|| {
                format!("the face embedder ({EMBEDDER_FILE}) is not downloaded")
            })?;
            landmarks
                .iter()
                .map(|lm| self.embed_aligned(rgb, lm, embedder))
                .collect()
        }

        /// One aligned face, embedded as the average of itself and its mirror
        /// image: the standard test-time trick, and worth it here because
        /// candid faces are rarely square to the camera.
        fn embed_aligned(
            &self,
            rgb: &image::RgbImage,
            landmarks: &[[f32; 2]; 5],
            embedder: &std::sync::Mutex<ort::session::Session>,
        ) -> Result<Vec<f32>> {
            let face = align_112(rgb, landmarks);
            let e = ALIGN_EDGE as usize;
            let mirrored: Vec<f32> = face
                .chunks_exact(e)
                .flat_map(|row| row.iter().rev().copied())
                .collect();
            let mut sess = crate::sync::lock_recover(embedder);
            let name = sess
                .inputs
                .first()
                .map(|i| i.name.clone())
                .unwrap_or_else(|| "data".to_string());
            // One image per run, not the pair as a batch of two. DirectML
            // rejects a batch of two at AuraFace's final BatchNormalization
            // ("The parameter is incorrect"). That failed every embed on
            // Windows GPUs, and People came up empty with no reason given.
            let mut run = |pixels: Vec<f32>| -> Result<Vec<f32>> {
                let tensor =
                    ort::value::Value::from_array(([1_i64, 3, e as i64, e as i64], pixels))?;
                let outputs = sess.run(ort::inputs![name.as_str() => tensor])?;
                let (_shape, data) = outputs[0].try_extract_tensor::<f32>()?;
                Ok(data.to_vec())
            };
            let mut a = run(face)?;
            let mut b = run(mirrored)?;
            crate::people::l2_normalize(&mut a);
            crate::people::l2_normalize(&mut b);
            let mut v: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x + y).collect();
            crate::people::l2_normalize(&mut v);
            Ok(v)
        }

        /// [`Self::detect_rgb`] on an encoded image.
        pub fn detect(&self, image: &[u8]) -> Result<Vec<Face>> {
            let rgb = image::load_from_memory(image)
                .context("decoding image for face detection")?
                .to_rgb8();
            self.detect_rgb(&rgb)
        }

        /// Detect faces in a decoded image and, per face, score focus and
        /// eye-openness. Boxes and landmarks are in the image's pixels.
        pub fn detect_rgb(&self, rgb: &image::RgbImage) -> Result<Vec<Face>> {
            let (img_w, img_h) = (rgb.width(), rgb.height());

            // --- YuNet: letterbox → run → decode every stride head. ---
            let (input, scale) = preprocess_yunet(rgb);
            let raw: Vec<RawDet> = {
                let mut yunet = crate::sync::lock_recover(&self.yunet);
                let name = yunet
                    .inputs
                    .first()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "input".to_string());
                let tensor = ort::value::Value::from_array((
                    [1_i64, 3, YUNET_INPUT as i64, YUNET_INPUT as i64],
                    input,
                ))?;
                let outputs = yunet.run(ort::inputs![name.as_str() => tensor])?;
                let mut dets = Vec::new();
                for &stride in &[8u32, 16, 32] {
                    let cols = YUNET_INPUT / stride;
                    let cls = tensor_vec(&outputs, &format!("cls_{stride}"))?;
                    let obj = tensor_vec(&outputs, &format!("obj_{stride}"))?;
                    let bbox = tensor_vec(&outputs, &format!("bbox_{stride}"))?;
                    let kps = tensor_vec(&outputs, &format!("kps_{stride}"))?;
                    dets.extend(decode_stride(
                        stride, cols, &cls, &obj, &bbox, &kps, YUNET_CONF,
                    ));
                }
                dets
            };

            // Scale detector-space boxes/landmarks back onto the image (the
            // letterbox pads bottom-right, so the origin is shared — just divide
            // by the fit scale) and clamp inside the frame.
            let clamp = move |v: f32, hi: u32| v.clamp(0.0, hi as f32);
            let faces: Vec<Face> = raw
                .into_iter()
                .map(|d| {
                    let x = clamp(d.bbox[0] / scale, img_w);
                    let y = clamp(d.bbox[1] / scale, img_h);
                    let x2 = clamp((d.bbox[0] + d.bbox[2]) / scale, img_w);
                    let y2 = clamp((d.bbox[1] + d.bbox[3]) / scale, img_h);
                    let mut lm = [[0.0f32; 2]; 5];
                    for (k, p) in lm.iter_mut().enumerate() {
                        *p = [
                            clamp(d.landmarks[k][0] / scale, img_w),
                            clamp(d.landmarks[k][1] / scale, img_h),
                        ];
                    }
                    Face {
                        x,
                        y,
                        w: (x2 - x).max(0.0),
                        h: (y2 - y).max(0.0),
                        score: d.score,
                        landmarks: lm,
                        sharpness: 0.0,
                        eye_open: None,
                        eyes_read: false,
                    }
                })
                .collect();

            // --- NMS, then per-face focus + eye scoring. ---
            let boxes: Vec<[f32; 4]> = faces.iter().map(|f| f.bbox()).collect();
            let scores: Vec<f32> = faces.iter().map(|f| f.score).collect();
            let mut kept: Vec<Face> = nms(&boxes, &scores, YUNET_NMS_IOU)
                .into_iter()
                .map(|i| faces[i])
                .collect();

            for f in &mut kept {
                f.sharpness = face_sharpness(rgb, f);
                // A face whose eyes fail to read keeps "can't tell" and is
                // tried again by the next pass, rather than sinking the photo.
                match self.eyes_of(rgb, f.bbox()) {
                    Ok(e) => {
                        f.eye_open = e;
                        f.eyes_read = self.can_read_eyes();
                    }
                    Err(e) => eprintln!("faces: cannot read eyes: {e:#}"),
                }
            }
            Ok(kept)
        }
    }

    /// Letterbox an image into YuNet's square input: scale to fit (preserving
    /// aspect), place at the top-left of a zero-padded canvas, and emit the planar
    /// NCHW **BGR**, 0–255 tensor YuNet expects (OpenCV convention — no mean/scale
    /// normalisation). Returns the tensor and the fit scale for mapping back.
    fn preprocess_yunet(rgb: &image::RgbImage) -> (Vec<f32>, f32) {
        let (w, h) = (rgb.width().max(1), rgb.height().max(1));
        let t = YUNET_INPUT;
        let scale = (t as f32 / w as f32).min(t as f32 / h as f32);
        let nw = ((w as f32 * scale).round() as u32).clamp(1, t);
        let nh = ((h as f32 * scale).round() as u32).clamp(1, t);
        // The image crate's Triangle filter, to the bit, a fraction of the
        // time: most of the face pass on a GPU (`raster::resize_triangle`).
        let resized = crate::raster::resize_triangle(rgb, nw, nh);
        let plane = (t * t) as usize;
        let mut input = vec![0f32; 3 * plane];
        for y in 0..nh {
            for x in 0..nw {
                let p = resized.get_pixel(x, y).0; // RGB
                let i = (y * t + x) as usize;
                input[i] = p[2] as f32; // B
                input[plane + i] = p[1] as f32; // G
                input[2 * plane + i] = p[0] as f32; // R
            }
        }
        (input, scale)
    }

    /// Copy a named model output into an owned `Vec<f32>`.
    fn tensor_vec(outputs: &ort::session::SessionOutputs, name: &str) -> Result<Vec<f32>> {
        let (_shape, data) = outputs[name].try_extract_tensor::<f32>()?;
        Ok(data.to_vec())
    }

    /// Focus of a face: crop the box and reuse the library's variance-of-Laplacian
    /// sharpness metric on it. 0 for a degenerate box.
    fn face_sharpness(rgb: &image::RgbImage, f: &Face) -> f32 {
        let (x, y) = (f.x.max(0.0) as u32, f.y.max(0.0) as u32);
        let (w, h) = (f.w as u32, f.h as u32);
        if w < 3 || h < 3 {
            return 0.0;
        }
        let face = image::imageops::crop_imm(rgb, x, y, w, h).to_image();
        crate::quality::sharpness(&image::DynamicImage::ImageRgb8(face))
    }

    /// Every face model is automatic analysis's: below normal priority on the
    /// CPU (`develop::ai::cpu_threads`).
    fn build_session(path: &Path) -> Result<ort::session::Session> {
        crate::develop::ai::commit_on_gpu_or_cpu(
            path,
            "faces",
            crate::develop::ai::Workload::Background,
        )
    }
}

// ------------------------------------------------------------------------
// Stub for a build without `--features ai`, so the type always exists.
// ------------------------------------------------------------------------
#[cfg(not(feature = "ai"))]
pub use stub::FaceEngine;

#[cfg(not(feature = "ai"))]
mod stub {
    use anyhow::{bail, Result};
    use std::path::Path;

    pub struct FaceEngine;

    impl FaceEngine {
        pub fn load(_dir: &Path) -> Result<Self> {
            bail!("this build was compiled without AI — rebuild with `--features ai`")
        }

        pub fn detect(&self, _image: &[u8]) -> Result<Vec<super::Face>> {
            bail!("this build was compiled without AI")
        }

        pub fn detect_rgb(&self, _rgb: &image::RgbImage) -> Result<Vec<super::Face>> {
            bail!("this build was compiled without AI")
        }

        pub fn embed_all_rgb(
            &self,
            _rgb: &image::RgbImage,
            _landmarks: &[[[f32; 2]; 5]],
        ) -> Result<Vec<Vec<f32>>> {
            bail!("this build was compiled without AI")
        }

        pub fn can_embed(&self) -> bool {
            false
        }

        pub fn can_read_eyes(&self) -> bool {
            false
        }

        pub fn read_eyes(&self, _image: &[u8], _boxes: &[[f32; 4]]) -> Result<Vec<Option<f32>>> {
            bail!("this build was compiled without AI")
        }

        pub fn embed(&self, _image: &[u8], _landmarks: &[[f32; 2]; 5]) -> Result<Vec<f32>> {
            bail!("this build was compiled without AI")
        }

        pub fn embed_all(
            &self,
            _image: &[u8],
            _landmarks: &[[[f32; 2]; 5]],
        ) -> Result<Vec<Vec<f32>>> {
            bail!("this build was compiled without AI")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(x: f32, y: f32, w: f32, h: f32, sharp: f32, eye: f32) -> Face {
        Face {
            x,
            y,
            w,
            h,
            score: 0.9,
            landmarks: [[0.0, 0.0]; 5],
            sharpness: sharp,
            eye_open: Some(eye),
            eyes_read: true,
        }
    }

    #[test]
    fn summarize_takes_the_largest_face_and_the_worst_eyes() {
        let faces = [
            face(0.0, 0.0, 10.0, 10.0, 5.0, 0.9), // small, sharp, eyes open
            face(0.0, 0.0, 40.0, 40.0, 2.0, 0.1), // large (primary), soft, blinking
        ];
        let s = summarize(&faces);
        assert_eq!(s.count, 2);
        assert_eq!(s.primary_sharpness, Some(2.0), "primary = largest by area");
        assert_eq!(s.min_eye_open, Some(0.1), "worst eye across the group");
    }

    #[test]
    fn summarize_of_no_faces_is_empty() {
        let s = summarize(&[]);
        assert_eq!(s.count, 0);
        assert_eq!(s.primary_sharpness, None);
        assert_eq!(s.min_eye_open, None);
    }

    #[test]
    fn iou_is_one_for_identical_and_zero_for_disjoint() {
        assert!((iou([0.0, 0.0, 10.0, 10.0], [0.0, 0.0, 10.0, 10.0]) - 1.0).abs() < 1e-6);
        assert_eq!(iou([0.0, 0.0, 10.0, 10.0], [20.0, 20.0, 5.0, 5.0]), 0.0);
        // Half-overlap along x: intersection 50, union 150 → 1/3.
        assert!((iou([0.0, 0.0, 10.0, 10.0], [5.0, 0.0, 10.0, 10.0]) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn nms_keeps_the_best_and_drops_its_overlaps() {
        let boxes = [
            [0.0, 0.0, 10.0, 10.0],   // A
            [1.0, 1.0, 10.0, 10.0],   // near-duplicate of A
            [50.0, 50.0, 10.0, 10.0], // separate
        ];
        let scores = [0.9, 0.8, 0.7];
        let keep = nms(&boxes, &scores, 0.5);
        assert_eq!(
            keep,
            vec![0, 2],
            "the lower-scoring overlap of A is dropped"
        );
    }

    /// Faces whose eyes can't be read don't count toward the photo's worst.
    #[test]
    fn summarize_ignores_unreadable_eyes() {
        let mut unreadable = face(0.0, 0.0, 50.0, 50.0, 1.0, 0.0);
        unreadable.eye_open = None;
        let s = summarize(&[unreadable, face(0.0, 0.0, 10.0, 10.0, 1.0, 0.8)]);
        assert_eq!(s.min_eye_open, Some(0.8));
        assert_eq!(summarize(&[unreadable]).min_eye_open, None);
    }

    #[test]
    fn a_roi_maps_its_centre_and_turns_with_its_angle() {
        let roi = Roi {
            cx: 500.0,
            cy: 300.0,
            side: 512.0,
            angle: 0.0,
        };
        assert_eq!(roi.to_image(128.0, 128.0), [500.0, 300.0]);
        assert_eq!(
            roi.to_image(256.0, 128.0),
            [756.0, 300.0],
            "two pixels a crop pixel"
        );
        // A quarter turn: the crop's right runs down the image.
        let turned = Roi {
            angle: std::f32::consts::FRAC_PI_2,
            ..roi
        };
        let [x, y] = turned.to_image(256.0, 128.0);
        assert!(
            (x - 500.0).abs() < 1e-3 && (y - 556.0).abs() < 1e-3,
            "{x} {y}"
        );

        // Level eye corners, centred on the points, half again their span.
        let mut pts = vec![[100.0f32, 100.0]; 478];
        pts[33] = [80.0, 100.0];
        pts[263] = [120.0, 110.0];
        let m = Roi::around_mesh(&pts);
        assert!((m.angle - (10.0f32).atan2(40.0)).abs() < 1e-5);
        assert!((m.side - 60.0).abs() < 1e-3);
    }

    #[test]
    fn a_roi_crop_is_black_outside_the_image() {
        let img = image::RgbImage::from_pixel(64, 64, image::Rgb([255, 128, 0]));
        let t = Roi {
            cx: 0.0,
            cy: 32.0,
            side: 128.0,
            angle: 0.0,
        }
        .tensor(&img);
        let plane = (MESH_EDGE * MESH_EDGE) as usize;
        let at = |u: u32, v: u32| t[(v * MESH_EDGE + u) as usize];
        assert_eq!(at(10, 128), 0.0, "left of the image");
        assert!((at(200, 128) - 1.0).abs() < 1e-6, "inside, red");
        assert!((t[plane + (128 * MESH_EDGE + 200) as usize] - 128.0 / 255.0).abs() < 1e-6);
    }

    /// The rule, on readings measured in the spike.
    #[test]
    fn eyes_read_open_shut_or_cant_tell() {
        let r = |presence, blink, aspect| EyeReading {
            presence,
            blink,
            aspect,
        };
        let shut = |v: Option<f32>| v.is_some_and(|v| v < 0.5);
        let open = |v: Option<f32>| v.is_some_and(|v| v >= 0.5);
        // A big smile with open eyes: OCEC read this shut.
        assert!(open(eye_open_from(&r(1.0, 0.24, 0.25), 125.0)));
        // Plainly shut, portrait sized.
        assert!(shut(eye_open_from(&r(1.0, 0.68, 0.03), 469.0)));
        assert!(
            shut(eye_open_from(&r(1.0, 0.46, 0.09), 447.0)),
            "low blink, lids touching"
        );
        // Looking down with the lids only partly lowered stays open.
        assert!(open(eye_open_from(&r(1.0, 0.43, 0.07), 88.0)));
        // Sunglasses: one eye reads high, the pair doesn't.
        assert!(open(eye_open_from(&r(1.0, 0.38, 0.12), 70.0)));
        // Can't tell: no face found by the mesh, or too small to read.
        assert_eq!(eye_open_from(&r(0.0, 0.95, 0.02), 200.0), None);
        assert_eq!(eye_open_from(&r(1.0, 0.78, 0.05), 48.0), None);
    }

    #[test]
    fn similarity_transform_recovers_a_known_similarity() {
        // Map the template through a known similarity (scale+rotation+shift) to
        // make source points; the solver should recover the inverse, mapping the
        // source back onto the template.
        let m = [[0.5f32, -0.2, 10.0], [0.2, 0.5, 5.0]];
        let apply = |t: [[f32; 3]; 2], p: [f32; 2]| {
            [
                t[0][0] * p[0] + t[0][1] * p[1] + t[0][2],
                t[1][0] * p[0] + t[1][1] * p[1] + t[1][2],
            ]
        };
        let src: [[f32; 2]; 5] = ARCFACE_TEMPLATE.map(|p| apply(m, p));
        let back = similarity_transform(&src, &ARCFACE_TEMPLATE);
        for i in 0..5 {
            let got = apply(back, src[i]);
            assert!(
                (got[0] - ARCFACE_TEMPLATE[i][0]).abs() < 1e-2
                    && (got[1] - ARCFACE_TEMPLATE[i][1]).abs() < 1e-2,
                "point {i}: {got:?} vs {:?}",
                ARCFACE_TEMPLATE[i]
            );
        }
    }

    #[test]
    fn decode_stride_reads_one_anchor_back() {
        // A 1×1 grid at stride 8, one confident anchor whose box is centred in the
        // cell (offset 0.5) with log-size 0 (→ one cell = 8px), landmarks at the
        // same centre.
        let dets = decode_stride(
            8,
            1,
            &[1.0],                // cls
            &[1.0],                // obj
            &[0.5, 0.5, 0.0, 0.0], // bbox: centre offset, exp(0)*8 = 8px
            &[0.5f32; 10],         // kps: all at cell centre
            0.6,
        );
        assert_eq!(dets.len(), 1);
        let d = dets[0];
        assert!((d.score - 1.0).abs() < 1e-6);
        // centre (4,4), size 8 → top-left (0,0), 8×8.
        assert!(d
            .bbox
            .iter()
            .zip([0.0, 0.0, 8.0, 8.0])
            .all(|(a, b)| (a - b).abs() < 1e-4));
        assert!(d
            .landmarks
            .iter()
            .all(|p| (p[0] - 4.0).abs() < 1e-4 && (p[1] - 4.0).abs() < 1e-4));

        // A weak anchor is dropped by the score threshold.
        assert!(decode_stride(8, 1, &[0.1], &[0.1], &[0.0; 4], &[0.0; 10], 0.6).is_empty());
    }

    #[test]
    fn a_portrait_frames_the_face_and_stays_inside_the_image() {
        // A 100px face in the middle of a 1600x1067 preview.
        let (left, top, side) = portrait_square([750.0, 480.0, 100.0, 100.0], 1600, 1067);
        assert_eq!(side, 167, "the face fills 60% of the portrait");
        assert!(
            left <= 750 && left + side >= 850,
            "the face is inside horizontally"
        );
        assert!(top <= 480 && top + side >= 580, "and vertically");

        // Against the top-left corner: slid inside, not shrunk.
        let (left, top, side) = portrait_square([0.0, 0.0, 100.0, 100.0], 1600, 1067);
        assert_eq!((left, top, side), (0, 0, 167));

        // A face bigger than the image allows: the square shrinks to fit.
        let (_, _, side) = portrait_square([0.0, 0.0, 900.0, 900.0], 1600, 1067);
        assert_eq!(side, 1067);

        let img = image::RgbImage::new(1600, 1067);
        let p = portrait(&img, [750.0, 480.0, 100.0, 100.0]);
        assert_eq!((p.width(), p.height()), (PORTRAIT_EDGE, PORTRAIT_EDGE));
    }
}
