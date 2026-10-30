//! Local adjustments: masks that confine an edit to part of the frame.
//!
//! The moat of a real raw editor. A mask assigns every output pixel a coverage
//! in 0..1, and blends the mask's [`LocalAdjustments`] by it. Most of a mask is
//! a handful of numbers that serialise into the [`crate::develop::EditStack`]
//! like any other control and cost nothing to store or undo.
//!
//! A mask is a **group of components**, each combined into the mask's coverage
//! by [`MaskMode`] (add / subtract / intersect) — the model other editors use,
//! so "a radial gradient minus a brushed-out patch" or "a colour range
//! intersected with a luminance range" is one mask with one set of adjustments.
//!
//! Each component is one [`MaskShape`]: a radial ellipse, a linear gradient, a
//! painted brush, or a value mask (luminance or colour range, whose coverage
//! comes from a pixel's brightness or hue rather than its position).
//!
//! **Coverage is defined in the framed output's 0..1 UV**, the space the user
//! places the mask in. A circle there is an ellipse on a non-square sensor,
//! which is exactly what the overlay draws, so the two always agree.

use serde::{Deserialize, Serialize};

/// The most masks a single photo may carry. The shader iterates a fixed-size
/// uniform array, so this is a hard cap rather than a soft one; eight is more
/// local adjustments than any sane edit uses and keeps the uniform small.
pub const MAX_MASKS: usize = 8;

/// The most mask components in total, across every mask on a photo. Also the
/// depth of the brush coverage texture array, so a brush component's layer index
/// is always in range.
pub const MAX_COMPONENTS: usize = 16;

/// The adjustments a mask applies where it covers, each nominally -1..=1 with 0
/// neutral: a local dodge/burn, tone and colour, and the endpoints
/// (whites, blacks) and local contrast (clarity, texture).
///
/// Not here, deliberately: sharpening and noise reduction act per pixel and
/// only mean anything at 1:1, and dehaze estimates the haze from the whole
/// frame, so none of the three has a sensible "just here".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalAdjustments {
    /// Local exposure in stops, applied as a linear multiply weighted by
    /// coverage — the dodge/burn control.
    pub exposure: f32,
    pub contrast: f32,
    /// Recover or lift the bright and dark ends *within* the mask, weighted by
    /// how bright each pixel already is.
    pub highlights: f32,
    pub shadows: f32,
    /// Local white balance: warm/cool and green/magenta. Approximate — applied
    /// post-tone rather than in camera space — but that is where a local WB
    /// belongs, since it is correcting the developed image, not the sensor.
    pub temperature: f32,
    pub tint: f32,
    pub saturation: f32,
    /// Move the white and black points within the mask, signs as the global
    /// controls: positive opens, negative clips.
    pub whites: f32,
    pub blacks: f32,
    /// Local contrast at two scales, as the global Clarity and Texture.
    pub clarity: f32,
    pub texture: f32,
}

impl Default for LocalAdjustments {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            temperature: 0.0,
            tint: 0.0,
            saturation: 0.0,
            whites: 0.0,
            blacks: 0.0,
            clarity: 0.0,
            texture: 0.0,
        }
    }
}

impl LocalAdjustments {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }
}

/// One dab of a brush stroke, in framed-output UV. A stroke is a list of these,
/// laid down as the pointer moves; the mask keeps them so a brush is
/// re-rasterisable and serialises like any other control.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub x: f32,
    pub y: f32,
    /// Radius as a fraction of the frame *width*, so the brush is round on
    /// screen once the aspect is corrected.
    pub radius: f32,
    /// Edge softness, 0 (fully soft) .. 1 (hard).
    pub hardness: f32,
    /// An erase dab removes coverage instead of adding it.
    #[serde(default)]
    pub erase: bool,
}

/// The geometry of a mask component, in framed-output UV (0..1 on both axes).
///
/// Not `Copy`: the brush variant carries its strokes. The analytic variants are
/// still trivial; only `pack`/`coverage` matching had to move to borrows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MaskShape {
    /// An ellipse: full coverage inside, fading to zero across a feather band at
    /// the edge. `angle` rotates the ellipse, in radians.
    Radial {
        cx: f32,
        cy: f32,
        radius_x: f32,
        radius_y: f32,
        angle: f32,
        /// Fraction of the radius over which coverage fades, 0..1.
        feather: f32,
    },
    /// A gradient: full coverage on the `(x1, y1)` line, fading to zero at the
    /// `(x2, y2)` line, and zero beyond it.
    Linear { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// A painted mask: coverage accumulated from brush dabs. Rendered by
    /// rasterising the strokes into a coverage texture (see [`rasterize`]) that
    /// the shader samples, rather than evaluated analytically.
    Brush { strokes: Vec<Stroke> },
    /// A value mask: coverage comes from a pixel's *brightness*, not its
    /// position — full inside `low..high`, fading to zero across a `feather`
    /// band on each side. Luminance is the developed (post-grade) Rec.2020 luma
    /// in 0..1, so the range reads the way it looks on screen. Has no geometry
    /// to drag; the panel drives it with range sliders.
    Luminance { low: f32, high: f32, feather: f32 },
    /// A value mask that selects by *hue*: full where the pixel's hue is within
    /// `range` of `hue` (both a fraction of the colour wheel, 0..1), fading over
    /// `feather`. Near-neutral pixels are excluded, since a grey has no
    /// meaningful hue. No geometry to drag.
    ColorRange { hue: f32, range: f32, feather: f32 },
    /// An AI-generated coverage mask (subject, sky, …). The model output is a
    /// grayscale coverage bitmap; it is stored **RLE-compressed and base64'd**
    /// in `rle` at `width`×`height` (row-major R8), because unlike a brush it
    /// cannot be re-derived without re-running the model. Rendered exactly like a
    /// brush — decoded to a coverage-texture layer the shader samples — so it
    /// drops into the component/combine system for free. `label` names it in the
    /// UI ("Subject", "Sky").
    Ai {
        label: String,
        width: u32,
        height: u32,
        rle: String,
    },
}

/// How a component folds into the coverage accumulated by the components before
/// it. The first component in a mask starts the accumulation, so its mode is
/// ignored. Order-dependent, as in other editors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    /// Union — a pixel covered by this component *or* the accumulation.
    #[default]
    Add,
    /// Difference — remove this component's coverage from the accumulation.
    Subtract,
    /// Intersection — keep only where this component *and* the accumulation
    /// overlap.
    Intersect,
}

/// One piece of a mask: a shape, whether it is inverted, and how it combines
/// with the pieces before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MaskComponent {
    pub shape: MaskShape,
    /// Swaps covered for uncovered, so the same ellipse can carve out its centre
    /// instead of selecting it.
    #[serde(default)]
    pub inverted: bool,
    #[serde(default)]
    pub mode: MaskMode,
}

/// The most brush dabs one component may hold.
pub const MAX_STROKES: usize = 100_000;

/// Puts a stack's masks within what can be rendered, wherever a stack
/// comes from — the page, the catalogue, a preset: dabs near the frame with a
/// sane radius and no more than [`MAX_STROKES`] of them; an AI mask's size
/// within [`MAX_AI_PIXELS`]. A stack is saved, so one malformed dab hung, or
/// one malformed AI mask panicked, every render of the photo for good.
pub fn sanitise(masks: &mut [Mask]) {
    for mask in masks {
        for comp in &mut mask.components {
            match &mut comp.shape {
                MaskShape::Brush { strokes } => {
                    strokes.truncate(MAX_STROKES);
                    strokes.retain(|s| {
                        s.x.is_finite()
                            && s.y.is_finite()
                            && s.radius.is_finite()
                            && s.hardness.is_finite()
                    });
                    for s in strokes {
                        s.x = s.x.clamp(-1.0, 2.0);
                        s.y = s.y.clamp(-1.0, 2.0);
                        s.radius = s.radius.clamp(0.0, 1.0);
                        s.hardness = s.hardness.clamp(0.0, 1.0);
                    }
                }
                MaskShape::Ai {
                    width, height, rle, ..
                } => {
                    let fits = (*width as usize)
                        .checked_mul(*height as usize)
                        .is_some_and(|n| n <= MAX_AI_PIXELS);
                    if !fits {
                        *width = 0;
                        *height = 0;
                        rle.clear();
                    }
                }
                _ => {}
            }
        }
    }
}

impl MaskComponent {
    /// A plain additive component around `shape`.
    pub fn new(shape: MaskShape) -> Self {
        Self {
            shape,
            inverted: false,
            mode: MaskMode::Add,
        }
    }

    /// Spatial coverage (radial / linear / brush) at output UV `(u, v)`,
    /// inversion applied. Value shapes report full spatial coverage — see
    /// [`MaskShape::coverage`].
    pub fn spatial_coverage(&self, u: f32, v: f32) -> f32 {
        let c = self.shape.coverage(u, v);
        if self.inverted {
            1.0 - c
        } else {
            c
        }
    }

    /// Coverage of a value (luminance) component at brightness `luma`, inversion
    /// applied. `None` for non-luminance shapes.
    pub fn value_coverage(&self, luma: f32) -> Option<f32> {
        self.shape
            .value_coverage(luma)
            .map(|c| if self.inverted { 1.0 - c } else { c })
    }

    /// Coverage of a colour-range component at `(hue, sat)`, inversion applied.
    /// `None` for every other shape.
    pub fn hue_coverage(&self, hue: f32, sat: f32) -> Option<f32> {
        self.shape
            .hue_coverage(hue, sat)
            .map(|c| if self.inverted { 1.0 - c } else { c })
    }
}

/// One local adjustment: a group of components deciding *where* it applies, and
/// the [`LocalAdjustments`] deciding *what* it does there.
///
/// `Deserialize` is hand-written (via [`MaskCompat`]) rather than derived with
/// `#[serde(from)]`, because `from` makes `specta` reflect the *source* type —
/// here the clean `{components, adjustments}` struct is what the generated TS
/// should mirror, while the legacy `{shape, inverted}` form still loads.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Mask {
    pub components: Vec<MaskComponent>,
    #[serde(default)]
    pub adjustments: LocalAdjustments,
    /// Optional user-facing name. `None` falls back to a derived label (the AI
    /// subject's own label, or "Radial 1" &c.) in the UI. Metadata only — it
    /// does not affect the render, but it persists with the stack.
    #[serde(default)]
    pub name: Option<String>,
}

impl<'de> Deserialize<'de> for Mask {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(MaskCompat::deserialize(d)?.into())
    }
}

/// Accepts both the current `{components}` form and the legacy single-shape
/// `{shape, inverted}` form, folding the latter into one component.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaskCompat {
    #[serde(default)]
    components: Vec<MaskComponent>,
    #[serde(default)]
    shape: Option<MaskShape>,
    #[serde(default)]
    inverted: bool,
    #[serde(default)]
    adjustments: LocalAdjustments,
    #[serde(default)]
    name: Option<String>,
}

impl From<MaskCompat> for Mask {
    fn from(c: MaskCompat) -> Self {
        let mut components = c.components;
        if components.is_empty() {
            if let Some(shape) = c.shape {
                components.push(MaskComponent {
                    shape,
                    inverted: c.inverted,
                    mode: MaskMode::Add,
                });
            }
        }
        Mask {
            components,
            adjustments: c.adjustments,
            name: c.name,
        }
    }
}

impl Mask {
    /// A one-component mask around `shape` — the common case, and what the UI
    /// creates before any Add/Subtract is layered on.
    pub fn single(shape: MaskShape) -> Self {
        Mask {
            components: vec![MaskComponent::new(shape)],
            adjustments: LocalAdjustments::default(),
            name: None,
        }
    }

    /// Combined coverage from a per-component coverage function (which must
    /// already apply the component's inversion). The first component starts the
    /// accumulation; each later one folds in by its [`MaskMode`].
    pub fn combined<F: Fn(&MaskComponent) -> f32>(&self, cov: F) -> f32 {
        let mut acc = 0.0f32;
        for (i, c) in self.components.iter().enumerate() {
            let cc = cov(c);
            acc = if i == 0 { cc } else { combine(acc, cc, c.mode) };
        }
        acc
    }

    /// Spatial-only combined coverage at `(u, v)` — the value shapes report full
    /// spatial coverage, so this is exact only for spatial masks.
    pub fn coverage(&self, u: f32, v: f32) -> f32 {
        self.combined(|c| c.spatial_coverage(u, v))
    }
}

/// Folds one component's coverage into the accumulation. Mirrors the shader.
fn combine(acc: f32, cc: f32, mode: MaskMode) -> f32 {
    match mode {
        MaskMode::Add => acc.max(cc),
        MaskMode::Subtract => acc * (1.0 - cc),
        MaskMode::Intersect => acc.min(cc),
    }
}

impl MaskShape {
    /// Coverage in 0..1 at output UV `(u, v)`, before inversion.
    ///
    /// The same maths the shader runs, kept here so it can be tested on the CPU
    /// without a GPU — the shader version must mirror it exactly.
    pub fn coverage(&self, u: f32, v: f32) -> f32 {
        match self {
            MaskShape::Radial {
                cx,
                cy,
                radius_x,
                radius_y,
                angle,
                feather,
            } => {
                let (sin, cos) = angle.sin_cos();
                let (dx, dy) = (u - cx, v - cy);
                // Into the ellipse's own axes, then normalised by its radii.
                let rx = (dx * cos + dy * sin) / radius_x.max(1e-4);
                let ry = (-dx * sin + dy * cos) / radius_y.max(1e-4);
                let d = (rx * rx + ry * ry).sqrt();
                let inner = 1.0 - feather.clamp(0.0, 1.0);
                1.0 - smoothstep(inner, 1.0, d)
            }
            MaskShape::Linear { x1, y1, x2, y2 } => {
                let (ax, ay) = (x2 - x1, y2 - y1);
                let len2 = ax * ax + ay * ay;
                if len2 < 1e-8 {
                    return 1.0;
                }
                let t = ((u - x1) * ax + (v - y1) * ay) / len2;
                1.0 - smoothstep(0.0, 1.0, t.clamp(0.0, 1.0))
            }
            // Square aspect here (the shader samples the aspect-correct texture
            // instead); this path is for tests and never renders.
            MaskShape::Brush { strokes } => brush_coverage(strokes, u, v, 1.0),
            // Value masks have no spatial coverage — they select by pixel value,
            // so they impose no positional restriction. `value_coverage` /
            // `hue_coverage` are the real evaluations; this keeps the spatial
            // path total.
            MaskShape::Luminance { .. } | MaskShape::ColorRange { .. } => 1.0,
            // AI coverage is a bitmap sampled from a texture on the GPU; this CPU
            // path (used only by combine tests, never to render) treats it as
            // full, exactly as the brush path is a square-aspect approximation.
            MaskShape::Ai { .. } => 1.0,
        }
    }

    /// Coverage of a value-based mask at brightness `luma` (developed Rec.2020
    /// luma, 0..1). `None` for spatial and hue-based shapes. The same maths the
    /// shader's luminance branch runs.
    pub fn value_coverage(&self, luma: f32) -> Option<f32> {
        match self {
            MaskShape::Luminance { low, high, feather } => {
                Some(luminance_band(*low, *high, *feather, luma))
            }
            _ => None,
        }
    }

    /// Coverage of a colour-range mask for a pixel with the given `hue` and
    /// `sat` (both 0..1). `None` for every other shape. Mirrors the shader.
    pub fn hue_coverage(&self, hue: f32, sat: f32) -> Option<f32> {
        match self {
            MaskShape::ColorRange {
                hue: target,
                range,
                feather,
            } => Some(color_band(*target, *range, *feather, hue, sat)),
            _ => None,
        }
    }
}

/// Soft band over luminance: rises across `feather` up to `low`, full through
/// `low..high`, falls across `feather` past `high`.
fn luminance_band(low: f32, high: f32, feather: f32, luma: f32) -> f32 {
    let f = feather.max(1e-4);
    let lo = smoothstep(low - f, low, luma);
    let hi = 1.0 - smoothstep(high, high + f, luma);
    (lo * hi).clamp(0.0, 1.0)
}

/// Coverage from hue proximity, gated by saturation. `target`/`hue` are
/// fractions of the colour wheel (0..1); distance wraps because hue is circular.
/// Near-neutral pixels are pushed to zero, since a grey has no meaningful hue —
/// the same reason the mixer leaves low-saturation pixels alone.
fn color_band(target: f32, range: f32, feather: f32, hue: f32, sat: f32) -> f32 {
    let mut d = (hue - target).abs() % 1.0;
    if d > 0.5 {
        d = 1.0 - d;
    }
    let f = feather.max(1e-4);
    let hue_cov = 1.0 - smoothstep(range, range + f, d);
    let sat_gate = smoothstep(0.05, 0.15, sat);
    (hue_cov * sat_gate).clamp(0.0, 1.0)
}

/// Accumulated coverage of a brush's dabs at `(u, v)`.
///
/// `aspect` is the frame's height/width, applied to the vertical distance so a
/// dab is round on screen rather than in UV. Paint dabs build coverage up
/// (`cov + c(1-cov)`); erase dabs take it away (`cov(1-c)`), in stroke order.
fn brush_coverage(strokes: &[Stroke], u: f32, v: f32, aspect: f32) -> f32 {
    let mut cov = 0.0f32;
    for s in strokes {
        let dx = u - s.x;
        let dy = (v - s.y) * aspect;
        let d = (dx * dx + dy * dy).sqrt();
        let r = s.radius.max(1e-4);
        let inner = r * s.hardness.clamp(0.0, 1.0);
        let c = 1.0 - smoothstep(inner, r, d);
        if s.erase {
            cov *= 1.0 - c;
        } else {
            cov += c * (1.0 - cov);
        }
    }
    cov
}

/// Rasterises brush strokes into a `w`×`h` single-channel coverage buffer (row
/// major, 0..255). The buffer is uploaded as one layer of the mask coverage
/// texture; the shader samples it for brush masks instead of computing coverage.
///
/// Incremental — each dab touches only its own bounding box in stroke order, so
/// cost is the summed dab area, not `pixels × strokes`. That is what keeps
/// re-rasterising affordable when a mask carries hundreds of dabs. The result is
/// identical to accumulating every stroke at every pixel (`brush_coverage`),
/// which the tests check.
pub fn rasterize(strokes: &[Stroke], w: u32, h: u32) -> Vec<u8> {
    let aspect = h as f32 / w as f32;
    let mut cov = vec![0f32; (w * h) as usize];
    for s in strokes {
        let r = s.radius.max(1e-4);
        let inner = r * s.hardness.clamp(0.0, 1.0);
        // Bounding box: radius r in u (width) units; in v it reaches r/aspect
        // because the vertical distance is scaled by the aspect.
        let x0 = (((s.x - r) * w as f32).floor() as i32).max(0) as u32;
        // Clamped both ways: a dab wholly left of or above the frame
        // gave a negative bound, cast to about four billion, and the loop
        // below spun for minutes on every render of the photo.
        let x1 = (((s.x + r) * w as f32).ceil() as i32).clamp(0, w as i32) as u32;
        let ry_uv = r / aspect.max(1e-4);
        let y0 = (((s.y - ry_uv) * h as f32).floor() as i32).max(0) as u32;
        let y1 = (((s.y + ry_uv) * h as f32).ceil() as i32).clamp(0, h as i32) as u32;
        for py in y0..y1 {
            let v = (py as f32 + 0.5) / h as f32;
            let dy = (v - s.y) * aspect;
            for px in x0..x1 {
                let u = (px as f32 + 0.5) / w as f32;
                let dx = u - s.x;
                let d = (dx * dx + dy * dy).sqrt();
                let c = 1.0 - smoothstep(inner, r, d);
                if c <= 0.0 {
                    continue;
                }
                let p = &mut cov[(py * w + px) as usize];
                if s.erase {
                    *p *= 1.0 - c;
                } else {
                    *p += c * (1.0 - *p);
                }
            }
        }
    }
    cov.iter()
        .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

/// Matches WGSL `smoothstep`, so the CPU coverage and the shader agree.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < 1e-6 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The mask set packed for the shader.
///
/// A separate uniform from `DevelopParams` on purpose: that struct's byte layout
/// is painstakingly padded to match WGSL, and threading a variable-length mask
/// array through it would be a standing hazard. Its own binding keeps the two
/// independent.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaskUniform {
    /// `[mask_count, component_count, 0, 0]`. The mask *visualization* is a DOM
    /// overlay the frontend draws from the same geometry, so no "show mask" flag
    /// is needed here — the shader only ever applies coverage, never tints it.
    header: [f32; 4],
    /// Maps the drawn-quad UV to the framed-image UV masks are defined in:
    /// `[extent_x, extent_y, centre_x, centre_y]`, so `framed = centre + (uv -
    /// 0.5) * extent`. Identity `[1, 1, 0.5, 0.5]` at fit-to-window; a zoomed
    /// view narrows the extent so a mask stays pinned to the same part of the
    /// photo instead of sliding with the viewport.
    view: [f32; 4],
    /// Two `vec4`s per component (geometry `g0`, `g1`), flattened across every
    /// mask in the order `pack`/`brush_layers` walk them. `g1.w` packs
    /// `mask_index*8 + mode*2 + inverted`, so the shader knows which mask a
    /// component belongs to and how it combines.
    comps: [[f32; 4]; MAX_COMPONENTS * 2],
    /// Two `vec4`s per mask: the two local-adjustment vectors, indexed by the
    /// component's mask index.
    adjs: [[f32; 4]; MAX_MASKS * 3],
}

/// The drawn-quad → framed-UV mapping at fit-to-window: the identity.
pub const IDENTITY_VIEW: [f32; 4] = [1.0, 1.0, 0.5, 0.5];

/// Kind discriminants shared with the shader.
const KIND_RADIAL: f32 = 0.0;
const KIND_LINEAR: f32 = 1.0;
const KIND_BRUSH: f32 = 2.0;
const KIND_LUMINANCE: f32 = 3.0;
const KIND_COLOR: f32 = 4.0;
const KIND_AI: f32 = 5.0;

/// Packs up to [`MAX_MASKS`] masks and [`MAX_COMPONENTS`] components in total;
/// extras are dropped rather than silently wrapping. `view` maps the drawn UV to
/// framed UV — see [`MaskUniform::view`].
pub fn pack(masks: &[Mask], view: [f32; 4]) -> MaskUniform {
    pack_showing(masks, view, None)
}

/// [`pack`], with `show` naming the mask whose coverage the render tints:
/// the header's third slot, one-based so zero is none.
pub fn pack_showing(masks: &[Mask], view: [f32; 4], show: Option<u32>) -> MaskUniform {
    let mut out = MaskUniform {
        header: [0.0; 4],
        view,
        comps: [[0.0; 4]; MAX_COMPONENTS * 2],
        adjs: [[0.0; 4]; MAX_MASKS * 3],
    };
    let mask_count = masks.len().min(MAX_MASKS);
    let mut ci = 0usize;
    // Brush and AI components each claim the next layer of the coverage texture
    // array; the pipeline uploads their coverage in this same order (see
    // `coverage_layers`), so the layer index here and the texture layer there
    // always agree.
    let mut coverage_layer = 0.0f32;
    for (mi, m) in masks.iter().take(MAX_MASKS).enumerate() {
        let a = m.adjustments;
        out.adjs[mi * 3] = [a.exposure, a.contrast, a.highlights, a.shadows];
        out.adjs[mi * 3 + 1] = [a.temperature, a.tint, a.saturation, 0.0];
        out.adjs[mi * 3 + 2] = [a.whites, a.blacks, a.clarity, a.texture];
        for comp in &m.components {
            if ci >= MAX_COMPONENTS {
                break;
            }
            let inv = if comp.inverted { 1.0 } else { 0.0 };
            let mode = match comp.mode {
                MaskMode::Add => 0.0,
                MaskMode::Subtract => 1.0,
                MaskMode::Intersect => 2.0,
            };
            let packed = mi as f32 * 8.0 + mode * 2.0 + inv;
            let (g0, g1) = match &comp.shape {
                MaskShape::Radial {
                    cx,
                    cy,
                    radius_x,
                    radius_y,
                    angle,
                    feather,
                } => (
                    [KIND_RADIAL, *cx, *cy, *feather],
                    [*radius_x, *radius_y, *angle, packed],
                ),
                MaskShape::Linear { x1, y1, x2, y2 } => {
                    ([KIND_LINEAR, *x1, *y1, 0.0], [*x2, *y2, 0.0, packed])
                }
                MaskShape::Brush { .. } => {
                    let layer = coverage_layer;
                    coverage_layer += 1.0;
                    ([KIND_BRUSH, layer, 0.0, 0.0], [0.0, 0.0, 0.0, packed])
                }
                MaskShape::Luminance { low, high, feather } => (
                    [KIND_LUMINANCE, *low, *high, *feather],
                    [0.0, 0.0, 0.0, packed],
                ),
                MaskShape::ColorRange {
                    hue,
                    range,
                    feather,
                } => (
                    [KIND_COLOR, *hue, *range, *feather],
                    [0.0, 0.0, 0.0, packed],
                ),
                MaskShape::Ai { .. } => {
                    let layer = coverage_layer;
                    coverage_layer += 1.0;
                    ([KIND_AI, layer, 0.0, 0.0], [0.0, 0.0, 0.0, packed])
                }
            };
            out.comps[ci * 2] = g0;
            out.comps[ci * 2 + 1] = g1;
            ci += 1;
        }
    }
    out.header = [
        mask_count as f32,
        ci as f32,
        show.map_or(0.0, |i| i as f32 + 1.0),
        0.0,
    ];
    out
}

/// Where a coverage-texture layer's pixels come from — a rasterised brush, or a
/// decoded AI bitmap to be resized to the layer's size.
pub enum CoverageSource<'a> {
    Brush(&'a [Stroke]),
    /// Decoded R8 coverage at its own `width`×`height`, to be resized on upload.
    Ai {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

/// Coverage components (brush + AI) paired with the texture-array layer they were
/// assigned, **in the same order `pack` assigns layers** — so the uniform's layer
/// index and the uploaded texture layer always agree. The pipeline turns each
/// source into an R8 buffer and uploads it.
pub fn coverage_layers(masks: &[Mask]) -> Vec<(u32, CoverageSource<'_>)> {
    let mut ci = 0usize;
    let mut layer = 0u32;
    let mut out = Vec::new();
    for m in masks.iter().take(MAX_MASKS) {
        for comp in &m.components {
            if ci >= MAX_COMPONENTS {
                return out;
            }
            match &comp.shape {
                MaskShape::Brush { strokes } => {
                    out.push((layer, CoverageSource::Brush(strokes.as_slice())));
                    layer += 1;
                }
                MaskShape::Ai {
                    width, height, rle, ..
                } => {
                    out.push((
                        layer,
                        CoverageSource::Ai {
                            data: decode_ai_sized(rle, *width, *height),
                            width: *width,
                            height: *height,
                        },
                    ));
                    layer += 1;
                }
                _ => {}
            }
            ci += 1;
        }
    }
    out
}

/// Run-length encodes an 8-bit coverage buffer: pairs of `[run(1..=255), value]`.
/// Coverage is mostly long runs of 0 and 255 with thin soft edges, so this is a
/// large win over raw bytes and keeps an AI mask small in the stack JSON.
pub fn rle_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let v = data[i];
        let mut run = 1u32;
        while i + (run as usize) < data.len() && data[i + run as usize] == v && run < 255 {
            run += 1;
        }
        out.push(run as u8);
        out.push(v);
        i += run as usize;
    }
    out
}

/// Inverse of [`rle_encode`].
pub fn rle_decode(rle: &[u8]) -> Vec<u8> {
    rle_decode_at_most(rle, usize::MAX)
}

/// [`rle_decode`], stopping at `limit` bytes: a few hundred bytes of
/// RLE could otherwise ask for gigabytes.
fn rle_decode_at_most(rle: &[u8], limit: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < rle.len() && out.len() < limit {
        let run = (rle[i] as usize).min(limit - out.len());
        let v = rle[i + 1];
        out.extend(std::iter::repeat_n(v, run));
        i += 2;
    }
    out
}

/// The most pixels an AI mask's coverage may have: the models make a
/// few hundred thousand; a stack claiming more is malformed.
pub const MAX_AI_PIXELS: usize = 4096 * 4096;

/// An AI mask's coverage, decoded only if it is the size it says: at
/// most [`MAX_AI_PIXELS`], and exactly `width` x `height`. Anything else is
/// empty, which renders as no coverage rather than a panic or a hang.
pub fn decode_ai_sized(rle_b64: &str, width: u32, height: u32) -> Vec<u8> {
    use base64::Engine;
    let Some(want) = (width as usize).checked_mul(height as usize) else {
        return Vec::new();
    };
    if want == 0 || want > MAX_AI_PIXELS {
        return Vec::new();
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(rle_b64) else {
        return Vec::new();
    };
    // One past the size, so data that runs on is caught, not cut short.
    let out = rle_decode_at_most(&bytes, want + 1);
    if out.len() == want {
        out
    } else {
        Vec::new()
    }
}

/// Decodes an AI mask's stored coverage (base64 of RLE) back to an R8 buffer.
/// A malformed string yields an empty buffer rather than panicking — a corrupt
/// mask should render as nothing, not crash the develop pass.
pub fn decode_ai(rle_b64: &str) -> Vec<u8> {
    use base64::Engine;
    match base64::engine::general_purpose::STANDARD.decode(rle_b64) {
        Ok(bytes) => rle_decode(&bytes),
        Err(_) => Vec::new(),
    }
}

/// Encodes an R8 coverage buffer to the stored form (RLE then base64).
pub fn encode_ai(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(rle_encode(data))
}

/// Bilinearly resizes a single-channel R8 buffer from `sw`×`sh` to `dw`×`dh`.
/// Used to fit a stored AI coverage to the coverage-texture size at upload; a
/// mismatched or empty source yields a zero (empty) coverage.
pub fn resize_coverage(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (dw, dh) = (dw.max(1), dh.max(1));
    // In usize, checked: 65536 x 65536 wrapped a u32 product to 0,
    // matched an empty buffer, and the sampling below indexed it.
    let fits = (sw as usize)
        .checked_mul(sh as usize)
        .is_some_and(|n| n > 0 && n == src.len());
    if !fits {
        return vec![0u8; dw as usize * dh as usize];
    }
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    // Each column's and each row's source pair and weight are worked out
    // once, not for every pixel. The arithmetic per pixel is the same, so
    // are the bytes, and it is several times quicker: Select Subject resizes
    // a megapixel matte at every click, and its input three times.
    let xs = bilinear_taps(sw, dw);
    let sw = sw as usize;
    let mut out = Vec::with_capacity(dw as usize * dh as usize);
    for (y0, y1, ty) in bilinear_taps(sh, dh) {
        let (r0, r1) = (&src[y0 * sw..][..sw], &src[y1 * sw..][..sw]);
        for &(x0, x1, tx) in &xs {
            let top = r0[x0] as f32 * (1.0 - tx) + r0[x1] as f32 * tx;
            let bot = r1[x0] as f32 * (1.0 - tx) + r1[x1] as f32 * tx;
            out.push(round_to_byte(top * (1.0 - ty) + bot * ty));
        }
    }
    out
}

/// `x.round().clamp(0.0, 255.0) as u8` for an `x` that is not negative, to
/// the bit, without the call: x86-64's baseline has no rounding instruction,
/// so `f32::round` is a call into the C library, and at a few per pixel it
/// was most of a megapixel resize. Truncation is an instruction, and the
/// fraction left after it is exact, so rounding half away from zero is one
/// comparison.
#[inline]
pub fn round_to_byte(x: f32) -> u8 {
    let whole = x as i32;
    let rounded = whole + (x - whole as f32 >= 0.5) as i32;
    rounded.clamp(0, 255) as u8
}

/// Bilinear resampling's taps along one axis, from `src` positions to `dst`:
/// for each destination position, the source positions either side of its
/// centre, clamped to the edge, and the weight of the second.
pub fn bilinear_taps(src: u32, dst: u32) -> Vec<(usize, usize, f32)> {
    let edge = |i: i32| i.clamp(0, src as i32 - 1) as usize;
    (0..dst)
        .map(|o| {
            // The destination pixel's centre, back in source space.
            let f = ((o as f32 + 0.5) * src as f32 / dst as f32) - 0.5;
            let i0 = f.floor() as i32;
            (edge(i0), edge(i0 + 1), f - i0 as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radial() -> MaskShape {
        MaskShape::Radial {
            cx: 0.5,
            cy: 0.5,
            radius_x: 0.25,
            radius_y: 0.25,
            angle: 0.0,
            feather: 0.5,
        }
    }

    #[test]
    fn a_radial_is_full_at_its_centre_and_zero_outside() {
        let s = radial();
        assert!(
            (s.coverage(0.5, 0.5) - 1.0).abs() < 1e-4,
            "centre should be full"
        );
        assert!(s.coverage(0.9, 0.5) < 1e-4, "well outside should be zero");
        // On the feather band, strictly between.
        let edge = s.coverage(0.5 + 0.25 * 0.9, 0.5);
        assert!(
            edge > 0.0 && edge < 1.0,
            "the feather band should be partial, got {edge}"
        );
    }

    #[test]
    fn inversion_swaps_covered_for_uncovered() {
        let c = MaskComponent {
            shape: radial(),
            inverted: true,
            mode: MaskMode::Add,
        };
        assert!(
            c.spatial_coverage(0.5, 0.5) < 1e-4,
            "inverted centre should be empty"
        );
        assert!(
            (c.spatial_coverage(0.9, 0.5) - 1.0).abs() < 1e-4,
            "inverted outside should be full"
        );
    }

    #[test]
    fn a_brush_dab_paints_coverage_and_erase_takes_it_away() {
        let painted = MaskShape::Brush {
            strokes: vec![Stroke {
                x: 0.5,
                y: 0.5,
                radius: 0.2,
                hardness: 0.5,
                erase: false,
            }],
        };
        assert!(
            painted.coverage(0.5, 0.5) > 0.99,
            "under the dab should be full"
        );
        assert!(
            painted.coverage(0.9, 0.5) < 1e-4,
            "far from the dab should be empty"
        );

        // An erase dab over the same spot removes what the paint dab laid down.
        let erased = MaskShape::Brush {
            strokes: vec![
                Stroke {
                    x: 0.5,
                    y: 0.5,
                    radius: 0.2,
                    hardness: 0.5,
                    erase: false,
                },
                Stroke {
                    x: 0.5,
                    y: 0.5,
                    radius: 0.2,
                    hardness: 0.5,
                    erase: true,
                },
            ],
        };
        assert!(
            erased.coverage(0.5, 0.5) < 1e-4,
            "erase should clear the centre"
        );
    }

    #[test]
    fn rasterise_matches_the_coverage_it_samples() {
        let strokes = [Stroke {
            x: 0.5,
            y: 0.5,
            radius: 0.3,
            hardness: 0.4,
            erase: false,
        }];
        let (w, h) = (32u32, 32u32);
        let buf = rasterize(&strokes, w, h);
        assert_eq!(buf.len(), (w * h) as usize);
        // Centre texel is (near) full, a corner is empty.
        let centre = buf[(h / 2 * w + w / 2) as usize];
        assert!(
            centre > 250,
            "centre coverage should be near full, got {centre}"
        );
        assert_eq!(buf[0], 0, "the corner is outside every dab");
    }

    #[test]
    fn coverage_layers_are_assigned_in_component_order() {
        let brush = |x: f32| {
            Mask::single(MaskShape::Brush {
                strokes: vec![Stroke {
                    x,
                    y: 0.5,
                    radius: 0.1,
                    hardness: 0.5,
                    erase: false,
                }],
            })
        };
        let ai = Mask::single(MaskShape::Ai {
            label: "Subject".into(),
            width: 2,
            height: 2,
            rle: encode_ai(&[0, 255, 255, 0]),
        });
        // A radial between coverage components must not consume a layer; brush and
        // AI share the layer counter in component order.
        let masks = vec![brush(0.2), Mask::single(radial()), ai, brush(0.8)];
        let layers = coverage_layers(&masks);
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].0, 0, "first brush is layer 0");
        assert_eq!(layers[1].0, 1, "the AI mask is layer 1, not the radial");
        assert_eq!(layers[2].0, 2, "second brush is layer 2");
        assert!(
            matches!(layers[1].1, CoverageSource::Ai { .. }),
            "layer 1 is the AI source"
        );
    }

    #[test]
    fn rle_round_trips_and_ai_encodes_through_base64() {
        // A run-heavy buffer, like a real coverage mask.
        let data: Vec<u8> = std::iter::repeat_n(0u8, 300)
            .chain(std::iter::repeat_n(255u8, 200))
            .chain([10, 20, 30])
            .collect();
        assert_eq!(rle_decode(&rle_encode(&data)), data, "RLE round-trips");
        assert_eq!(decode_ai(&encode_ai(&data)), data, "base64+RLE round-trips");
        // Runs longer than 255 split correctly.
        let long = vec![7u8; 600];
        assert_eq!(rle_decode(&rle_encode(&long)), long, "runs over 255 split");
        // A corrupt string decodes to empty rather than panicking.
        assert!(decode_ai("not base64!!!").is_empty());
    }

    /// Malformed mask data renders as nothing, quickly, instead of hanging
    /// or panicking.
    #[test]
    fn malformed_masks_neither_hang_nor_panic() {
        // A dab wholly off the left of the frame.
        let off = Stroke {
            x: -1.0,
            y: 0.5,
            radius: 0.02,
            hardness: 0.5,
            erase: false,
        };
        let started = std::time::Instant::now();
        let cov = rasterize(&[off], 512, 341);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(cov.iter().all(|&c| c == 0));

        // An AI mask claiming 65536 x 65536 with nothing behind it.
        assert!(decode_ai_sized("", 65536, 65536).is_empty());
        assert_eq!(resize_coverage(&[], 65536, 65536, 4, 4), vec![0u8; 16]);
        // RLE asking for far more than its size.
        let greedy = encode_ai(&[255u8; 64]);
        assert!(
            decode_ai_sized(&greedy, 2, 2).is_empty(),
            "64 bytes for a 2x2 mask"
        );
        assert_eq!(
            decode_ai_sized(&encode_ai(&[1, 2, 3, 4]), 2, 2),
            vec![1, 2, 3, 4]
        );

        let mut masks = vec![Mask::single(MaskShape::Ai {
            label: String::new(),
            width: 65536,
            height: 65536,
            rle: String::new(),
        })];
        sanitise(&mut masks);
        match &masks[0].components[0].shape {
            MaskShape::Ai { width, height, .. } => assert_eq!((*width, *height), (0, 0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn resize_coverage_scales_and_guards() {
        // Identity size returns the source untouched.
        let src = vec![0, 255, 255, 0];
        assert_eq!(resize_coverage(&src, 2, 2, 2, 2), src);
        // Upscaling keeps the corners and interpolates between.
        let up = resize_coverage(&src, 2, 2, 4, 4);
        assert_eq!(up.len(), 16);
        assert_eq!(up[0], 0, "top-left corner preserved");
        // A malformed source yields an empty (zero) coverage of the target size.
        assert_eq!(resize_coverage(&[1, 2, 3], 2, 2, 3, 3), vec![0u8; 9]);
    }

    /// The resize as it was written before its taps were shared, pixel by
    /// pixel: what the faster one must match byte for byte.
    fn resize_pixel_by_pixel(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
        let mut out = vec![0u8; dw as usize * dh as usize];
        let sample = |x: i32, y: i32| -> f32 {
            let x = x.clamp(0, sw as i32 - 1) as usize;
            let y = y.clamp(0, sh as i32 - 1) as usize;
            src[y * sw as usize + x] as f32
        };
        for oy in 0..dh {
            let fy = ((oy as f32 + 0.5) * sh as f32 / dh as f32) - 0.5;
            let y0 = fy.floor() as i32;
            let ty = fy - y0 as f32;
            for ox in 0..dw {
                let fx = ((ox as f32 + 0.5) * sw as f32 / dw as f32) - 0.5;
                let x0 = fx.floor() as i32;
                let tx = fx - x0 as f32;
                let top = sample(x0, y0) * (1.0 - tx) + sample(x0 + 1, y0) * tx;
                let bot = sample(x0, y0 + 1) * (1.0 - tx) + sample(x0 + 1, y0 + 1) * tx;
                out[(oy * dw + ox) as usize] =
                    (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
            }
        }
        out
    }

    #[test]
    fn rounding_to_a_byte_is_f32_round() {
        let mut x = 0.0f32;
        while x < 256.5 {
            assert_eq!(round_to_byte(x), x.round().clamp(0.0, 255.0) as u8, "{x}");
            // Every representable value near each half, and a sweep between.
            let half = x.floor() + 0.5;
            for v in [
                half,
                f32::from_bits(half.to_bits() - 1),
                f32::from_bits(half.to_bits() + 1),
            ] {
                assert_eq!(round_to_byte(v), v.round().clamp(0.0, 255.0) as u8, "{v}");
            }
            x += 0.013;
        }
    }

    #[test]
    fn the_shared_taps_resize_exactly_as_pixel_by_pixel_did() {
        let mut seed = 7u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        for (sw, sh, dw, dh) in [
            (1024, 683, 1024, 1024),
            (1024, 1024, 1024, 683),
            (37, 5, 3, 91),
            (1, 1, 7, 7),
            (640, 427, 256, 256),
        ] {
            let src: Vec<u8> = (0..sw * sh).map(|_| (next() >> 24) as u8).collect();
            assert_eq!(
                resize_coverage(&src, sw, sh, dw, dh),
                resize_pixel_by_pixel(&src, sw, sh, dw, dh),
                "{sw}x{sh} -> {dw}x{dh}"
            );
        }
    }

    #[test]
    fn an_ai_mask_packs_as_a_coverage_layer() {
        let m = Mask::single(MaskShape::Ai {
            label: "Sky".into(),
            width: 2,
            height: 2,
            rle: encode_ai(&[255, 255, 0, 0]),
        });
        let u = pack(std::slice::from_ref(&m), IDENTITY_VIEW);
        assert_eq!(u.comps[0][0], KIND_AI, "kind is AI");
        assert_eq!(u.comps[0][1], 0.0, "layer 0");
    }

    #[test]
    fn a_linear_gradient_runs_from_full_to_zero() {
        let s = MaskShape::Linear {
            x1: 0.0,
            y1: 0.5,
            x2: 1.0,
            y2: 0.5,
        };
        assert!(
            (s.coverage(0.0, 0.5) - 1.0).abs() < 1e-4,
            "start line is full"
        );
        assert!(s.coverage(1.0, 0.5) < 1e-4, "end line is empty");
        assert!(s.coverage(2.0, 0.5) < 1e-4, "beyond the end stays empty");
        let mid = s.coverage(0.5, 0.5);
        assert!(
            (mid - 0.5).abs() < 1e-4,
            "the midpoint should be half, got {mid}"
        );
    }

    #[test]
    fn a_luminance_mask_selects_a_brightness_band() {
        let s = MaskShape::Luminance {
            low: 0.3,
            high: 0.7,
            feather: 0.1,
        };
        // Value-based: spatial coverage imposes no restriction.
        assert_eq!(
            s.coverage(0.1, 0.9),
            1.0,
            "spatial coverage is unrestricted"
        );
        // Inside the band is full; well outside is empty.
        assert!(
            (s.value_coverage(0.5).unwrap() - 1.0).abs() < 1e-4,
            "mid-band is full"
        );
        assert!(s.value_coverage(0.0).unwrap() < 1e-4, "black is outside");
        assert!(s.value_coverage(1.0).unwrap() < 1e-4, "white is outside");
        // On the feather skirt, strictly partial.
        let edge = s.value_coverage(0.3 - 0.05).unwrap();
        assert!(
            edge > 0.0 && edge < 1.0,
            "the feather band should be partial, got {edge}"
        );
        // Spatial shapes report no value coverage.
        assert!(
            radial().value_coverage(0.5).is_none(),
            "a radial has no value coverage"
        );
    }

    #[test]
    fn a_luminance_component_inverts() {
        let c = MaskComponent {
            shape: MaskShape::Luminance {
                low: 0.3,
                high: 0.7,
                feather: 0.1,
            },
            inverted: true,
            mode: MaskMode::Add,
        };
        assert!(
            c.value_coverage(0.5).unwrap() < 1e-4,
            "inverted mid-band is empty"
        );
        assert!(
            (c.value_coverage(0.0).unwrap() - 1.0).abs() < 1e-4,
            "inverted black is full"
        );
    }

    #[test]
    fn a_luminance_mask_packs_its_range() {
        let m = Mask::single(MaskShape::Luminance {
            low: 0.25,
            high: 0.75,
            feather: 0.2,
        });
        let u = pack(&[m], IDENTITY_VIEW);
        assert_eq!(
            u.comps[0],
            [KIND_LUMINANCE, 0.25, 0.75, 0.2],
            "geometry vec4"
        );
        // A luminance mask claims no coverage layer.
        assert!(
            coverage_layers(&[Mask::single(MaskShape::Luminance {
                low: 0.0,
                high: 1.0,
                feather: 0.1
            })])
            .is_empty(),
            "a luminance mask is not a coverage layer",
        );
    }

    #[test]
    fn a_colour_mask_selects_a_hue_and_ignores_neutrals() {
        // Target orange-ish at hue 0.08 (~29 degrees), tolerance 0.05.
        let s = MaskShape::ColorRange {
            hue: 0.08,
            range: 0.05,
            feather: 0.05,
        };
        assert_eq!(
            s.coverage(0.1, 0.9),
            1.0,
            "spatial coverage is unrestricted"
        );
        // On-hue and saturated is full; the opposite hue is empty.
        assert!(
            (s.hue_coverage(0.08, 0.8).unwrap() - 1.0).abs() < 1e-4,
            "on-hue is full"
        );
        assert!(
            s.hue_coverage(0.58, 0.8).unwrap() < 1e-4,
            "the far hue is empty"
        );
        // A near-neutral pixel is excluded whatever its nominal hue.
        assert!(
            s.hue_coverage(0.08, 0.0).unwrap() < 1e-4,
            "a grey has no hue to select"
        );
        // Hue distance wraps: a target near 1.0 still catches a hue near 0.0.
        let wrap = MaskShape::ColorRange {
            hue: 0.98,
            range: 0.05,
            feather: 0.02,
        };
        assert!(
            wrap.hue_coverage(0.01, 0.8).unwrap() > 0.9,
            "distance wraps across the seam"
        );
        // Spatial shapes report no hue coverage.
        assert!(
            radial().hue_coverage(0.5, 0.8).is_none(),
            "a radial has no hue coverage"
        );
    }

    #[test]
    fn a_colour_mask_packs_its_range_and_inverts() {
        let m = Mask {
            components: vec![MaskComponent {
                shape: MaskShape::ColorRange {
                    hue: 0.33,
                    range: 0.1,
                    feather: 0.05,
                },
                inverted: true,
                mode: MaskMode::Add,
            }],
            adjustments: LocalAdjustments::default(),
            name: None,
        };
        let u = pack(std::slice::from_ref(&m), IDENTITY_VIEW);
        assert_eq!(u.comps[0], [KIND_COLOR, 0.33, 0.1, 0.05], "geometry vec4");
        // mask 0, add (mode 0), inverted -> 0*8 + 0*2 + 1 = 1.
        assert_eq!(u.comps[1][3], 1.0, "packed flags in g1.w");
        assert!(
            m.components[0].hue_coverage(0.33, 0.8).unwrap() < 1e-4,
            "inverted on-hue is empty"
        );
        assert!(coverage_layers(std::slice::from_ref(&m)).is_empty());
    }

    #[test]
    fn components_combine_by_mode() {
        let dot = |cx: f32, mode: MaskMode| MaskComponent {
            shape: MaskShape::Radial {
                cx,
                cy: 0.5,
                radius_x: 0.2,
                radius_y: 0.2,
                angle: 0.0,
                feather: 0.1,
            },
            inverted: false,
            mode,
        };
        let left = dot(0.35, MaskMode::Add);
        let mask = |m: MaskMode| Mask {
            components: vec![left.clone(), dot(0.65, m)],
            adjustments: LocalAdjustments::default(),
            name: None,
        };
        // Add unions: both isolated centres are covered.
        let add = mask(MaskMode::Add);
        assert!(
            add.coverage(0.35, 0.5) > 0.9 && add.coverage(0.65, 0.5) > 0.9,
            "add unions"
        );
        // Subtract removes the right dot: left kept, right carved out.
        let sub = mask(MaskMode::Subtract);
        assert!(sub.coverage(0.35, 0.5) > 0.9, "left kept");
        assert!(sub.coverage(0.65, 0.5) < 0.1, "right subtracted");
        // Intersect keeps only the overlap; these two dots do not overlap.
        let inter = mask(MaskMode::Intersect);
        assert!(
            inter.coverage(0.35, 0.5) < 0.1 && inter.coverage(0.65, 0.5) < 0.1,
            "no overlap is empty"
        );
        // The first component's mode is ignored — it starts the accumulation.
        let solo = Mask {
            components: vec![dot(0.65, MaskMode::Subtract)],
            adjustments: LocalAdjustments::default(),
            name: None,
        };
        assert!(
            solo.coverage(0.65, 0.5) > 0.9,
            "first component sets coverage regardless of its mode"
        );
    }

    #[test]
    fn packing_flattens_components_with_mask_index_and_mode() {
        let m = Mask {
            components: vec![
                MaskComponent::new(radial()),
                MaskComponent {
                    shape: MaskShape::Linear {
                        x1: 0.2,
                        y1: 0.5,
                        x2: 0.8,
                        y2: 0.5,
                    },
                    inverted: true,
                    mode: MaskMode::Subtract,
                },
            ],
            adjustments: LocalAdjustments::default(),
            name: None,
        };
        let u = pack(&[m], IDENTITY_VIEW);
        assert_eq!(u.header[0], 1.0, "one mask");
        assert_eq!(u.header[1], 2.0, "two components");
        assert_eq!(
            u.comps[1][3], 0.0,
            "first component: mask 0, add, not inverted"
        );
        // Second component: mask 0, subtract (mode 1), inverted -> 0 + 2 + 1 = 3.
        assert_eq!(u.comps[3][3], 3.0, "mask_index*8 + mode*2 + inverted");
    }

    #[test]
    fn a_legacy_single_shape_mask_still_loads() {
        // The pre-combine-ops serialised form: a bare shape + inverted flag.
        let json = r#"{"shape":{"type":"radial","cx":0.5,"cy":0.5,"radiusX":0.25,"radiusY":0.25,"angle":0.0,"feather":0.5},"inverted":true,"adjustments":{"exposure":0.5,"contrast":0,"highlights":0,"shadows":0,"temperature":0,"tint":0,"saturation":0}}"#;
        let m: Mask = serde_json::from_str(json).expect("legacy mask deserialises");
        assert_eq!(m.components.len(), 1, "folded into one component");
        assert!(m.components[0].inverted, "inversion carried over");
        assert_eq!(m.components[0].mode, MaskMode::Add);
        assert_eq!(m.adjustments.exposure, 0.5);
        // The current form round-trips.
        let round: Mask = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(round, m);
    }

    #[test]
    fn packing_reports_the_count_and_places_adjustments() {
        let m = Mask {
            components: vec![MaskComponent::new(radial())],
            adjustments: LocalAdjustments {
                exposure: 0.5,
                ..Default::default()
            },
            name: None,
        };
        let u = pack(&[m], IDENTITY_VIEW);
        assert_eq!(u.header[0], 1.0, "mask count");
        assert_eq!(u.header[1], 1.0, "component count");
        assert_eq!(
            u.adjs[0][0], 0.5,
            "exposure lands in the first adjustment vec4"
        );
    }

    /// Whites, blacks, clarity and texture ride in the third vec4.
    #[test]
    fn the_endpoint_and_contrast_adjustments_are_packed() {
        let m = Mask {
            components: vec![MaskComponent::new(radial())],
            adjustments: LocalAdjustments {
                whites: 0.1,
                blacks: 0.2,
                clarity: 0.3,
                texture: 0.4,
                ..Default::default()
            },
            name: None,
        };
        let u = pack(&[m.clone(), m], IDENTITY_VIEW);
        assert_eq!(u.adjs[2], [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(u.adjs[5], [0.1, 0.2, 0.3, 0.4], "the second mask's");
        // A stack saved before these existed still loads, neutral.
        let old: LocalAdjustments = serde_json::from_str(r#"{"exposure":0.5}"#).unwrap();
        assert_eq!((old.whites, old.clarity), (0.0, 0.0));
    }

    #[test]
    fn packing_caps_at_the_maximum() {
        let many = vec![Mask::single(radial()); MAX_MASKS + 4];
        assert_eq!(pack(&many, IDENTITY_VIEW).header[0], MAX_MASKS as f32);
    }

    #[test]
    fn packing_caps_total_components() {
        // One mask carrying more components than the cap allows.
        let m = Mask {
            components: vec![MaskComponent::new(radial()); MAX_COMPONENTS + 4],
            adjustments: LocalAdjustments::default(),
            name: None,
        };
        assert_eq!(
            pack(&[m], IDENTITY_VIEW).header[1],
            MAX_COMPONENTS as f32,
            "component count is capped"
        );
    }
}
