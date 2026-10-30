// Pass B: linear camera RGB -> display-encoded output.
//
// Everything a user adjusts lives here. It is a render pass rather than a
// compute pass on purpose: the same shader has to draw into an offscreen
// texture for readback and export today, and straight into a window surface
// once the direct GPU presenter lands. Sampling the mosaic output through a
// filtered sampler also means the proxy downscale costs nothing extra.
//
// Stage order is the product. Exposure is a linear-light multiply and must
// happen before the colour transform; the display encode must be last.

struct Params {
    // Columns are vec4-aligned: WGSL gives mat3x3 a 16-byte column stride, so
    // the Rust side pads to match. Getting this wrong shears the colour matrix
    // and tints the whole image.
    cam_to_working: mat3x3<f32>,
    working_to_display: mat3x3<f32>,
    exposure: f32,
    // Domain of the tone table, in stops relative to middle grey, and the
    // middle-grey value itself. Passed in rather than duplicated as constants
    // here, so the shader and `tone.rs` cannot drift apart.
    tone_min_stops: f32,
    tone_max_stops: f32,
    tone_grey: f32,
    tone_lut_size: f32,
    mixer_lut_size: f32,
    grade_lut_size: f32,
    channel_lut_size: f32,
    /// EXIF orientation, 1..=8.
    orientation: f32,
    /// 0 = tone each channel independently, 1 = tone luminance and keep the
    /// colour ratios. See `apply_tone`.
    chroma_preserve: f32,
    /// Creative-look LUT: grid size per axis, and a 0..1 blend (0 = no LUT).
    /// These two scalars occupy exactly the implicit padding WGSL used to insert
    /// before `wb` (ten scalars padded 136->144 bytes), so the struct's size is
    /// unchanged — `wb` still lands at offset 144. The Rust side drops its old
    /// `[f32; 2]` pad for the same two scalars.
    lut_size: f32,
    lut_intensity: f32,
    wb: vec4<f32>,
    /// Output-to-oriented affine map, row-major as `[xx, xy, yx, yy]`.
    /// Composed on the CPU in `develop/geometry.rs`, where it is tested.
    crop_matrix: vec4<f32>,
    /// Centre of the crop in oriented coordinates, plus two bytes of padding
    /// this struct needs anyway.
    crop_centre: vec4<f32>,
    /// `[texture amount, clarity amount, texture lod, clarity lod]`.
    detail: vec4<f32>,
    /// `[sharpen amount, sharpen radius, sharpen masking, noise reduction]`.
    sharpen: vec4<f32>,
    /// `[distortion, vignetting, chromatic aberration, unused]`.
    lens: vec4<f32>,
    /// `[colour-noise amount, chroma-blur lod, luma-coarse lod, sharpen detail]`.
    nr: vec4<f32>,
    /// `[post-crop vignette, vignette midpoint, grain, grain size]`.
    effects: vec4<f32>,
    /// `[dehaze amount, atmospheric-light mip level, soft-proof flag,
    /// output transfer: 0 sRGB curve, 1 gamma 563/256 (Adobe RGB)]`.
    dehaze: vec4<f32>,
    /// `[halation amount, halation threshold, grain highlight roll-off,
    /// rendering intent: 0 relative colorimetric, 1 perceptual]`.
    film: vec4<f32>,
    /// `[vibrance, saturation, output luma red, output luma green]`. Global
    /// colour intensity, then the output space's luminance weights for
    /// `gamut_map` (blue is the remainder; they sum to one).
    color: vec4<f32>,
    /// `[perceptual mixer on, colour ranges read OkLCh, unused, unused]`, as 0
    /// or 1: how a version-4 edit reads hue. See `apply_perceptual_mixer`.
    hue: vec4<f32>,
}

@group(0) @binding(0) var linear_rgb: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var tone_lut: texture_2d<f32>;
@group(0) @binding(4) var mixer_lut: texture_2d<f32>;
@group(0) @binding(5) var grade_lut: texture_2d<f32>;

// Local-adjustment masks. Layout mirrors `develop::mask::MaskUniform`: a header
// `[mask_count, component_count, 0, 0]`, the drawn->framed UV mapping, then two
// vec4 per component (g0, g1) flattened across every mask, then three adjustment
// vec4 per mask. `g1.w` of a component packs `mask_index*8 + mode*2 + inverted`.
// Shape kinds: 0 radial, 1 linear, 2 brush, 3 luminance, 4 colour, 5 ai.
const MAX_MASKS: u32 = 8u;
const MAX_COMPONENTS: u32 = 16u;
struct Masks {
    header: vec4<f32>,
    view: vec4<f32>,
    comps: array<vec4<f32>, 32>, // MAX_COMPONENTS * 2
    adjs: array<vec4<f32>, 24>,  // MAX_MASKS * 3
}
@group(0) @binding(6) var<uniform> masks: Masks;
// One layer per brush mask, R8 coverage rasterised on the CPU from its strokes.
@group(0) @binding(7) var mask_coverage_tex: texture_2d_array<f32>;

// Spot / heal / clone patches. Layout mirrors `develop::spot::SpotUniform`: a
// header `[count, 0, 0, 0]`, the drawn->framed UV mapping, then two vec4 per
// spot — `[tx, ty, sx, sy]` and `[radius, feather, heal, 0]`.
const MAX_SPOTS: u32 = 40u;
struct Spots {
    header: vec4<f32>,
    view: vec4<f32>,
    data: array<vec4<f32>, 80>, // MAX_SPOTS * 2
}
@group(0) @binding(8) var<uniform> spots: Spots;
// Per-channel (R/G/B) point curves, RGBA per input value. Identity when unset.
@group(0) @binding(9) var channel_lut: texture_2d<f32>;
// Creative-look 3D LUT (a `.cube`), resampled to a fixed grid. Applied last, on
// the display-encoded image; a no-op whenever `params.lut_intensity` is 0.
// Interpolated manually (see `sample_lut`) with `textureLoad`, so the float
// format needs no hardware-filtering feature — the same tactic the 1D LUTs use.
@group(0) @binding(10) var lut_3d: texture_3d<f32>;

/// Trilinearly samples the look LUT at a display-referred colour. Grid coord is
/// `input * (size-1)`; the eight surrounding texels are loaded and mixed. Texel
/// order is red-fastest along x, matching how the cube is uploaded, and this
/// matches the CPU reference in `develop::lut::CubeLut::sample`.
fn sample_lut(rgb: vec3<f32>) -> vec3<f32> {
    let last = params.lut_size - 1.0;
    let c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * last;
    let i0 = floor(c);
    let f = c - i0;
    let i1 = min(i0 + vec3<f32>(1.0), vec3<f32>(last));
    let a = vec3<i32>(i0);
    let b = vec3<i32>(i1);
    let c000 = textureLoad(lut_3d, vec3<i32>(a.x, a.y, a.z), 0).rgb;
    let c100 = textureLoad(lut_3d, vec3<i32>(b.x, a.y, a.z), 0).rgb;
    let c010 = textureLoad(lut_3d, vec3<i32>(a.x, b.y, a.z), 0).rgb;
    let c110 = textureLoad(lut_3d, vec3<i32>(b.x, b.y, a.z), 0).rgb;
    let c001 = textureLoad(lut_3d, vec3<i32>(a.x, a.y, b.z), 0).rgb;
    let c101 = textureLoad(lut_3d, vec3<i32>(b.x, a.y, b.z), 0).rgb;
    let c011 = textureLoad(lut_3d, vec3<i32>(a.x, b.y, b.z), 0).rgb;
    let c111 = textureLoad(lut_3d, vec3<i32>(b.x, b.y, b.z), 0).rgb;
    let x00 = mix(c000, c100, f.x);
    let x10 = mix(c010, c110, f.x);
    let x01 = mix(c001, c101, f.x);
    let x11 = mix(c011, c111, f.x);
    return mix(mix(x00, x10, f.y), mix(x01, x11, f.y), f.z);
}

/// Coverage of one component (geometry `g0`, `g1`) at framed-image UV `p`,
/// inversion applied.
///
/// `rgb` is the pixel's developed colour, used only by the value shapes
/// (luminance and colour range); the spatial shapes ignore it. Inversion is the
/// low bit of the packed flags in `g1.w`.
fn component_coverage(g0: vec4<f32>, g1: vec4<f32>, p: vec2<f32>, rgb: vec3<f32>) -> f32 {
    var cov = 0.0;
    if g0.x > 4.5 {
        // AI coverage: a model-generated bitmap uploaded as a texture layer, same
        // as a brush — g0.y is the layer index.
        cov = textureSampleLevel(mask_coverage_tex, linear_sampler, p, i32(g0.y), 0.0).r;
    } else if g0.x > 3.5 {
        // Colour range: g0 = [kind, target_hue, range, feather], hue a fraction
        // of the wheel. Distance wraps; near-neutral pixels are gated out.
        // A version-4 edit reads OkLCh hue, and gates greys by chroma, which
        // a dark or pale colour of the same hue has little of; HSV saturation
        // called a dark brown as saturated as a vivid orange.
        var hue = 0.0;
        var sat_gate = 0.0;
        if params.hue.y > 0.5 {
            let lch = oklch(rgb);
            hue = lch.z / 360.0;
            sat_gate = smoothstep(0.015, 0.045, lch.y);
        } else {
            let hsv = rgb_to_hsv(rgb);
            hue = hsv.x / 360.0;
            sat_gate = smoothstep(0.05, 0.15, hsv.y);
        }
        var d = abs(hue - g0.y) % 1.0;
        if d > 0.5 {
            d = 1.0 - d;
        }
        let f = max(g0.w, 1e-4);
        let hue_cov = 1.0 - smoothstep(g0.z, g0.z + f, d);
        cov = clamp(hue_cov * sat_gate, 0.0, 1.0);
    } else if g0.x > 2.5 {
        // Luminance: g0 = [kind, low, high, feather]. Coverage from brightness,
        // not position — full through low..high, feathered on each side.
        let luma = dot(rgb, LUMA_REC2020);
        let low = g0.y;
        let high = g0.z;
        let f = max(g0.w, 1e-4);
        let lo = smoothstep(low - f, low, luma);
        let hi = 1.0 - smoothstep(high, high + f, luma);
        cov = clamp(lo * hi, 0.0, 1.0);
    } else if g0.x > 1.5 {
        // Brush: sample the pre-rasterised coverage layer. Explicit LOD, so it
        // is safe inside the mask loop's dynamic control flow.
        cov = textureSampleLevel(mask_coverage_tex, linear_sampler, p, i32(g0.y), 0.0).r;
    } else if g0.x < 0.5 {
        // Radial: g0 = [kind, cx, cy, feather], g1 = [rx, ry, angle, packed].
        let centre = vec2<f32>(g0.y, g0.z);
        let feather = clamp(g0.w, 0.0, 1.0);
        let rx = max(g1.x, 1e-4);
        let ry = max(g1.y, 1e-4);
        let a = g1.z;
        let s = sin(a);
        let co = cos(a);
        let d = p - centre;
        let ex = (d.x * co + d.y * s) / rx;
        let ey = (-d.x * s + d.y * co) / ry;
        let dist = sqrt(ex * ex + ey * ey);
        // Floor the inner edge below 1 so a zero feather is a near-hard edge
        // rather than a degenerate smoothstep.
        let inner = 1.0 - max(feather, 1e-3);
        cov = 1.0 - smoothstep(inner, 1.0, dist);
    } else {
        // Linear: g0 = [kind, x1, y1, 0], g1 = [x2, y2, 0, packed].
        let p1 = vec2<f32>(g0.y, g0.z);
        let axis = vec2<f32>(g1.x, g1.y) - p1;
        let len2 = dot(axis, axis);
        if len2 < 1e-8 {
            cov = 1.0;
        } else {
            let t = clamp(dot(p - p1, axis) / len2, 0.0, 1.0);
            cov = 1.0 - smoothstep(0.0, 1.0, t);
        }
    }
    // Inversion is the low bit of the packed flags.
    let packed = i32(round(g1.w));
    if (packed % 2) == 1 {
        cov = 1.0 - cov;
    }
    return cov;
}

/// Applies one mask's adjustments, weighted by coverage, in linear working
/// space (post-tone, display-referred). A local dodge/burn plus tone recovery
/// and white balance.
///
/// `adj0 = [exposure, contrast, highlights, shadows]`,
/// `adj1 = [temperature, tint, saturation, unused]`,
/// `adj2 = [whites, blacks, clarity, texture]`. `bands` are the pixel's
/// texture and clarity bands (see `detail_bands`), zero when no mask uses them.
fn apply_local(
    rgb0: vec3<f32>,
    cov: f32,
    adj0: vec4<f32>,
    adj1: vec4<f32>,
    adj2: vec4<f32>,
    bands: vec2<f32>,
) -> vec3<f32> {
    var rgb = rgb0;
    // Region weights come from the pixel's brightness *before* the local edit,
    // so highlights and shadows target the tones the user is looking at.
    let region = dot(rgb0, vec3<f32>(0.2627, 0.6780, 0.0593));

    if adj0.x != 0.0 {
        rgb = rgb * exp2(cov * adj0.x);
    }
    if adj0.y != 0.0 {
        let pivot = 0.18;
        rgb = (rgb - pivot) * (1.0 + cov * adj0.y) + pivot;
    }
    if adj0.z != 0.0 {
        let w = smoothstep(0.4, 1.0, region);
        rgb = rgb * (1.0 + cov * adj0.z * w * 0.6);
    }
    if adj0.w != 0.0 {
        let w = 1.0 - smoothstep(0.0, 0.5, region);
        rgb = rgb * (1.0 + cov * adj0.w * w * 0.6);
    }
    if adj1.x != 0.0 {
        let t = cov * adj1.x * 0.3;
        rgb.r = rgb.r * (1.0 + t);
        rgb.b = rgb.b * (1.0 - t);
    }
    if adj1.y != 0.0 {
        rgb.g = rgb.g * (1.0 - cov * adj1.y * 0.3);
    }
    if adj1.z != 0.0 {
        let luma = dot(rgb, vec3<f32>(0.2627, 0.6780, 0.0593));
        rgb = mix(vec3<f32>(luma), rgb, 1.0 + cov * adj1.z);
    }
    // Local contrast, as the global Clarity/Texture: a bounded multiplier on
    // the pixel's own bands, so it reads the same in a bright and a dark area.
    if adj2.z != 0.0 || adj2.w != 0.0 {
        let boost = clamp(1.0 + cov * (adj2.w * bands.x + adj2.z * bands.y), 0.25, 4.0);
        rgb = rgb * boost;
    }
    // Endpoints last, as in the global tone stage: positive opens, negative
    // clips. Same scales as `ToneParams::map_stops`.
    if adj2.x != 0.0 || adj2.y != 0.0 {
        let black_point = cov * adj2.y * -0.05;
        let white_point = 1.0 + cov * adj2.x * -0.4;
        rgb = (rgb - vec3<f32>(black_point)) / max(white_point - black_point, 1e-4);
    }
    return rgb;
}

fn rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let maxc = max(c.r, max(c.g, c.b));
    let minc = min(c.r, min(c.g, c.b));
    let delta = maxc - minc;

    var hue = 0.0;
    if delta > 1e-6 {
        if maxc == c.r {
            hue = 60.0 * (((c.g - c.b) / delta) % 6.0);
        } else if maxc == c.g {
            hue = 60.0 * (((c.b - c.r) / delta) + 2.0);
        } else {
            hue = 60.0 * (((c.r - c.g) / delta) + 4.0);
        }
    }
    if hue < 0.0 {
        hue = hue + 360.0;
    }
    let sat = select(0.0, delta / maxc, maxc > 1e-6);
    return vec3<f32>(hue, sat, maxc);
}

fn hsv_to_rgb(c: vec3<f32>) -> vec3<f32> {
    let h = c.x / 60.0;
    let s = clamp(c.y, 0.0, 1.0);
    let v = c.z;

    let i = floor(h);
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));

    let sector = i32(i) % 6;
    if sector == 0 { return vec3<f32>(v, t, p); }
    if sector == 1 { return vec3<f32>(q, v, p); }
    if sector == 2 { return vec3<f32>(p, v, t); }
    if sector == 3 { return vec3<f32>(p, q, v); }
    if sector == 4 { return vec3<f32>(t, p, v); }
    return vec3<f32>(v, p, q);
}

// Oklab, mirrored from `develop/oklab.rs` (a test there checks the numbers).
// Each matrix is written by its rows, so `v * M` applies it: a row vector
// times a matrix dots it with each column, and these columns are those rows.
const REC2020_TO_LMS_ROWS = mat3x3<f32>(
    vec3<f32>(0.616756, 0.360198, 0.023046),
    vec3<f32>(0.265133, 0.635839, 0.099028),
    vec3<f32>(0.100103, 0.203907, 0.695991),
);

const LMS_TO_REC2020_ROWS = mat3x3<f32>(
    vec3<f32>(2.139907, -1.246390, 0.106483),
    vec3<f32>(-0.884736, 2.163231, -0.278495),
    vec3<f32>(-0.048574, -0.454503, 1.503077),
);

const LMS_TO_LAB_ROWS = mat3x3<f32>(
    vec3<f32>(0.210454, 0.793618, -0.004072),
    vec3<f32>(1.977998, -2.428592, 0.450594),
    vec3<f32>(0.025904, 0.782772, -0.808676),
);

const LAB_TO_LMS_ROWS = mat3x3<f32>(
    vec3<f32>(1.000000, 0.396338, 0.215804),
    vec3<f32>(1.000000, -0.105561, -0.063854),
    vec3<f32>(1.000000, -0.089484, -1.291486),
);

/// OkLCh of linear Rec.2020: lightness, chroma, hue in degrees 0..360.
fn oklch(rgb: vec3<f32>) -> vec3<f32> {
    let lms = rgb * REC2020_TO_LMS_ROWS;
    // A signed cube root: a working-space colour outside the cone responses
    // still maps somewhere, and back again.
    let root = sign(lms) * pow(abs(lms), vec3<f32>(1.0 / 3.0));
    let lab = root * LMS_TO_LAB_ROWS;
    var hue = degrees(atan2(lab.z, lab.y));
    if hue < 0.0 {
        hue = hue + 360.0;
    }
    return vec3<f32>(lab.x, length(lab.yz), hue);
}

/// Linear Rec.2020 of OkLCh.
fn from_oklch(lch: vec3<f32>) -> vec3<f32> {
    let h = radians(lch.z);
    let lab = vec3<f32>(lch.x, lch.y * cos(h), lch.y * sin(h));
    let root = lab * LAB_TO_LMS_ROWS;
    return (root * root * root) * LMS_TO_REC2020_ROWS;
}

/// Rec.2020 luminance weights, matching the working space.
const LUMA_REC2020 = vec3<f32>(0.2627, 0.6780, 0.0593);

/// Scene-referred to display-referred, blending two ways of applying the curve.
///
/// **Per channel** is what film does: the dominant channel hits the shoulder
/// first and compresses hardest, so saturated colours desaturate as they
/// brighten. That highlight roll-off looks natural — but applied in a space as
/// wide as Rec.2020 it costs saturation everywhere, measurably so: rendering
/// this way alone came out 8% flatter than the camera's own JPEG.
///
/// **Ratio preserving** tones one norm of the triplet and scales all three
/// channels by the same factor, so the channel ratios — the hue — survive the
/// curve exactly. The norm is the **max channel**: it puts the brightest channel
/// on the curve (so highlights still roll off) while leaving nothing above the
/// toned max, which keeps this self-bounding. A luminance norm does not bound —
/// a channel brighter than the luminance is scaled past 1.0 and the later clamp
/// flattens it, which is why this path used to be disabled. Colours that leave
/// the display gamut are handled downstream by `gamut_map`, not by clipping.
///
/// Blending the two keeps most of the saturation and most of the roll-off; the
/// mix is `params.chroma_preserve` (see `tone.rs`).
fn apply_tone(rgb: vec3<f32>) -> vec3<f32> {
    let per_channel = vec3<f32>(
        tone_channel(rgb.r),
        tone_channel(rgb.g),
        tone_channel(rgb.b),
    );

    let m = max(rgb.r, max(rgb.g, rgb.b));
    if m <= 1e-6 {
        return per_channel;
    }
    let preserved = rgb * (tone_channel(m) / m);

    return mix(per_channel, preserved, params.chroma_preserve);
}

/// Per-hue-band hue rotation, saturation and luminance.
///
/// Sampled from a table the CPU baked, so the band weighting lives in Rust
/// where it is tested. Interpolated by hand and wrapped, because hue is
/// circular — a clamped lookup would flatten the adjustment across the red
/// seam where 359 degrees meets 1.
fn apply_mixer(rgb: vec3<f32>) -> vec3<f32> {
    var hsv = rgb_to_hsv(rgb);
    if hsv.y < 1e-4 {
        // Neutral pixels have no meaningful hue; leaving them alone stops the
        // mixer tinting greys according to sensor noise.
        return rgb;
    }

    let size = params.mixer_lut_size;
    let x = hsv.x / 360.0 * size;
    let i0 = i32(floor(x)) % i32(size);
    let i1 = (i0 + 1) % i32(size);
    let a = textureLoad(mixer_lut, vec2<i32>(i0, 0), 0).rgb;
    let b = textureLoad(mixer_lut, vec2<i32>(i1, 0), 0).rgb;
    let adj = mix(a, b, x - floor(x));

    hsv.x = (hsv.x + adj.x) % 360.0;
    if hsv.x < 0.0 {
        hsv.x = hsv.x + 360.0;
    }
    // Saturation scales multiplicatively so the control behaves the same on a
    // pastel and on a vivid colour.
    hsv.y = clamp(hsv.y * (1.0 + adj.y), 0.0, 1.0);
    hsv.z = max(hsv.z * (1.0 + adj.z * 0.5), 0.0);
    return hsv_to_rgb(hsv);
}

/// A version-4 edit's colour mixer: per-band hue, chroma and lightness in
/// OkLCh, from the table's second row, indexed by OkLCh hue.
///
/// Each moves one thing: hue turns the colour at its lightness and chroma,
/// saturation scales chroma at its lightness, and luminance moves lightness —
/// by the factor it scaled HSV value by, taken as light and so cube-rooted
/// here — without touching the colour. In HSV each of those moved the others;
/// desaturating a blue lifted it most of a stop.
///
/// Near the neutral axis hue is noise, so hue and lightness changes fade in
/// with chroma: a band cannot pick grey pixels out by the random hue their
/// noise gives them. A chroma pushed past what the working space holds is
/// walked back until the colour fits, keeping its hue and lightness.
fn apply_perceptual_mixer(rgb: vec3<f32>) -> vec3<f32> {
    if params.hue.x < 0.5 {
        return rgb;
    }
    var lch = oklch(rgb);
    let size = params.mixer_lut_size;
    let x = lch.z / 360.0 * size;
    let i0 = i32(floor(x)) % i32(size);
    let i1 = (i0 + 1) % i32(size);
    let a = textureLoad(mixer_lut, vec2<i32>(i0, 1), 0).rgb;
    let b = textureLoad(mixer_lut, vec2<i32>(i1, 1), 0).rgb;
    let adj = mix(a, b, x - floor(x));

    let hued = smoothstep(0.01, 0.04, lch.y);
    let light = max(1.0 + adj.z * 0.5 * hued, 0.0);
    lch.x = lch.x * pow(light, 1.0 / 3.0);
    lch.y = max(lch.y * (1.0 + adj.y), 0.0);
    lch.z = lch.z + adj.x * hued;

    var out = from_oklch(lch);
    if min(out.r, min(out.g, out.b)) < 0.0 {
        var lo = 0.0;
        var hi = lch.y;
        for (var i = 0; i < 12; i = i + 1) {
            let mid = 0.5 * (lo + hi);
            let c = from_oklch(vec3<f32>(lch.x, mid, lch.z));
            if min(c.r, min(c.g, c.b)) < 0.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        out = max(from_oklch(vec3<f32>(lch.x, lo, lch.z)), vec3<f32>(0.0));
    }
    return out;
}

/// Shadow/midtone/highlight colour wheels, as a multiplier looked up by
/// luminance.
///
/// The whole grade — which range owns this pixel, what colour that range asks
/// for, how the three blend — is baked into the table on the CPU, so nothing is
/// left here but working out how bright the pixel is. Multiplying rather than
/// adding keeps the tint proportionate: a flat colour offset would wash out
/// the darkest parts of the frame, where there is least signal to tint.
fn apply_grade(rgb: vec3<f32>) -> vec3<f32> {
    let luma = clamp(dot(rgb, LUMA_REC2020), 0.0, 1.0);

    let last = i32(params.grade_lut_size) - 1;
    let x = luma * f32(last);
    let i0 = i32(floor(x));
    let i1 = min(i0 + 1, last);

    let a = textureLoad(grade_lut, vec2<i32>(i0, 0), 0).rgb;
    let b = textureLoad(grade_lut, vec2<i32>(i1, 0), 0).rgb;
    return rgb * mix(a, b, x - floor(x));
}

/// Global vibrance and saturation, in the working space.
///
/// Both act on the pixel's chroma — its distance from the equal-luminance grey
/// — so neither shifts hue or brightness. Saturation scales that chroma by a
/// flat factor. Vibrance does too, but tapers the boost by how saturated the
/// pixel already is, so it lifts muted colour while leaving the vivid parts
/// (and, in a portrait, skin — which is rarely the most saturated thing in the
/// frame) closer to where they were. The two combine multiplicatively, matching
/// how the per-band mixer's saturation reads.
fn apply_saturation(rgb: vec3<f32>) -> vec3<f32> {
    let vib = params.color.x;
    let sat = params.color.y;
    if abs(vib) < 1e-4 && abs(sat) < 1e-4 {
        return rgb;
    }
    let luma = dot(rgb, LUMA_REC2020);
    let chroma = rgb - vec3<f32>(luma);

    var scale = 1.0 + sat;
    if abs(vib) > 1e-4 {
        // How saturated the pixel is now, 0 (grey) .. 1 (vivid), measured
        // relative to its own brightness so it reads the same in shadow and
        // highlight. The boost is weighted by what is *left* to give.
        let mx = max(rgb.r, max(rgb.g, rgb.b));
        let mn = min(rgb.r, min(rgb.g, rgb.b));
        let s = clamp((mx - mn) / max(mx, 1e-4), 0.0, 1.0);
        scale = scale * (1.0 + vib * (1.0 - s));
    }

    return vec3<f32>(luma) + chroma * max(scale, 0.0);
}

/// Scene-referred light to display-referred, through the baked tone table.
///
/// Indexed by stops rather than linear value: the table then has fine
/// resolution where the eye does, and covers highlights far above 1.0 without
/// needing an enormous number of entries.
/// Fetched with `textureLoad` and interpolated by hand rather than sampled.
/// The table is R32Float, which WebGPU does not guarantee is filterable, and
/// storing it as a filterable format would cost precision for no benefit —
/// the interpolation is three lines.
fn tone_channel(v: f32) -> f32 {
    let stops = log2(max(v, 1e-8) / params.tone_grey);
    let t = (stops - params.tone_min_stops) / (params.tone_max_stops - params.tone_min_stops);

    let last = i32(params.tone_lut_size) - 1;
    let x = clamp(t, 0.0, 1.0) * f32(last);
    let i0 = i32(floor(x));
    let i1 = min(i0 + 1, last);

    let a = textureLoad(tone_lut, vec2<i32>(i0, 0), 0).r;
    let b = textureLoad(tone_lut, vec2<i32>(i1, 0), 0).r;
    return mix(a, b, x - floor(x));
}

/// Per-channel (R/G/B) point curves. Each texel of `channel_lut` remaps one
/// input value: `.r` is red's output, `.g` green's, `.b` blue's, and each
/// channel indexes the table by its *own* value. Identity when the curves are
/// unset, so this is a no-op until the user drags a per-channel curve. Same
/// fetch-and-interpolate-by-hand pattern as `tone_channel`.
fn channel_curve(rgb: vec3<f32>) -> vec3<f32> {
    let last = i32(params.channel_lut_size) - 1;
    let x = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * f32(last);
    let i0 = vec3<i32>(floor(x));
    let i1 = min(i0 + vec3<i32>(1), vec3<i32>(last));
    let f = x - floor(x);

    let r = mix(
        textureLoad(channel_lut, vec2<i32>(i0.r, 0), 0).r,
        textureLoad(channel_lut, vec2<i32>(i1.r, 0), 0).r,
        f.r,
    );
    let g = mix(
        textureLoad(channel_lut, vec2<i32>(i0.g, 0), 0).g,
        textureLoad(channel_lut, vec2<i32>(i1.g, 0), 0).g,
        f.g,
    );
    let b = mix(
        textureLoad(channel_lut, vec2<i32>(i0.b, 0), 0).b,
        textureLoad(channel_lut, vec2<i32>(i1.b, 0), 0).b,
        f.b,
    );
    return vec3<f32>(r, g, b);
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

/// Maps an output coordinate back to the sensor, applying EXIF orientation.
///
/// Cameras record which way up they were held rather than rotating the sensor
/// data, so a portrait frame is stored landscape. Skipping this leaves every
/// rotated photo on its side in Develop while the library — which bakes the
/// rotation into its thumbnails — shows it upright.
///
/// Applied here rather than in the mosaic pass so that rotating costs nothing:
/// it is a coordinate transform on a cached texture, not a re-demosaic.
/// Output coordinates to oriented-frame coordinates: crop, straighten, user
/// rotation and mirroring, all in one affine map.
///
/// Applied *before* `orient`, because the crop is expressed in the frame the
/// user actually sees and drags in — the EXIF rotation is then undone on the
/// way back to sensor space.
///
/// Reading outside 0..1 is possible once a straighten angle is in play: the
/// corners of a rotated rectangle reach past the image. Those samples are
/// clamped to the edge by the sampler, which smears the border rather than
/// wrapping the opposite side in — the crop rectangle is meant to be pulled
/// inside the valid area, and the UI is what should enforce that.
fn crop(uv: vec2<f32>) -> vec2<f32> {
    let p = uv - vec2<f32>(0.5, 0.5);
    let m = params.crop_matrix;
    return vec2<f32>(
        m.x * p.x + m.y * p.y + params.crop_centre.x,
        m.z * p.x + m.w * p.y + params.crop_centre.y,
    );
}

fn orient(uv: vec2<f32>) -> vec2<f32> {
    let o = i32(params.orientation);
    if o == 2 { return vec2<f32>(1.0 - uv.x, uv.y); }        // mirror
    if o == 3 { return vec2<f32>(1.0 - uv.x, 1.0 - uv.y); }  // 180
    if o == 4 { return vec2<f32>(uv.x, 1.0 - uv.y); }        // flip
    if o == 5 { return vec2<f32>(uv.y, uv.x); }              // transpose
    if o == 6 { return vec2<f32>(uv.y, 1.0 - uv.x); }        // 90 clockwise
    if o == 7 { return vec2<f32>(1.0 - uv.y, 1.0 - uv.x); }  // transverse
    if o == 8 { return vec2<f32>(1.0 - uv.y, uv.x); }        // 90 anticlockwise
    return uv;
}

@vertex
fn vs(@builtin(vertex_index) idx: u32) -> VertexOut {
    // One oversized triangle rather than two triangles: no seam along the
    // diagonal, and one fewer vertex to invoke.
    let uv = vec2<f32>(f32((idx << 1u) & 2u), f32(idx & 2u));
    var out: VertexOut;
    out.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    // Flip Y: clip space runs bottom-up, texture space top-down.
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

/// Linear light to sRGB. The piecewise curve, not the 2.2 approximation —
/// the linear segment near black is what keeps deep shadows from crushing.
fn encode_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

/// sRGB to linear light: the inverse of `encode_srgb`.
fn decode_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

/// Linear Rec.2020 (the working space) to linear sRGB, column-major.
/// **Mirrors `gpu::color::REC2020_TO_SRGB`.**
const REC2020_TO_SRGB = mat3x3<f32>(
    vec3<f32>(1.6605, -0.1246, -0.0182),
    vec3<f32>(-0.5876, 1.1329, -0.1006),
    vec3<f32>(-0.0728, -0.0083, 1.1187),
);
const LUMA_SRGB = vec3<f32>(0.2126, 0.7152, 0.0722);

/// WGSL has no matrix inverse. Columns `a, b, c`: the inverse's rows are the
/// pairwise cross products over the determinant.
fn inverse3(m: mat3x3<f32>) -> mat3x3<f32> {
    let r0 = cross(m[1], m[2]);
    let r1 = cross(m[2], m[0]);
    let r2 = cross(m[0], m[1]);
    return transpose(mat3x3<f32>(r0, r1, r2)) * (1.0 / dot(m[0], r0));
}

/// Whether this render is encoded as sRGB — every preview, and most exports.
fn srgb_output() -> bool {
    let d = params.working_to_display;
    let e = abs(d[0] - REC2020_TO_SRGB[0]) + abs(d[1] - REC2020_TO_SRGB[1])
        + abs(d[2] - REC2020_TO_SRGB[2]);
    return params.dehaze.w < 0.5 && max(e.x, max(e.y, e.z)) < 1e-4;
}

/// The look LUT's result for a wide-gamut output, from the linear,
/// in-gamut output colour `lin`, encoded in the output's own transfer.
///
/// The LUT is shown the colour as sRGB (gamut-mapped into it), and what it
/// changes is carried back into the output space as a *difference*: an
/// identity LUT is still an exact no-op, and colour sRGB could not hold keeps
/// the saturation it had rather than being clipped to sRGB by any look.
fn look_through_srgb(lin: vec3<f32>) -> vec3<f32> {
    let working = inverse3(params.working_to_display) * lin;
    let srgb = clamp(gamut_map_in(REC2020_TO_SRGB * working, LUMA_SRGB), vec3<f32>(0.0), vec3<f32>(1.0));
    let change = decode_srgb(sample_lut(encode_srgb(srgb))) - srgb;
    let srgb_to_output = params.working_to_display * inverse3(REC2020_TO_SRGB);
    let result = clamp(lin + srgb_to_output * change, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(encode_srgb(result), encode_gamma22(result), params.dehaze.w > 0.5);
}

/// Linear light to Adobe RGB (1998): a pure power law, 563/256 ≈ 2.2, with no
/// linear toe — the standard says so, unlike sRGB.
fn encode_gamma22(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(256.0 / 563.0));
}

/// Fits a linear display-primary colour into [0,1] by desaturating it toward its
/// equal-luminance grey at constant hue, rather than clipping each channel
/// independently (which rotates the hue of any colour that leaves the gamut).
///
/// **Mirrors `gpu::color::gamut_map`, which is the tested reference — keep the
/// two identical.**
fn gamut_map(rgb: vec3<f32>) -> vec3<f32> {
    // The output space's own luminance, not always sRGB's: a P3 or Adobe RGB
    // render is compressed toward its own brightness.
    return gamut_map_in(rgb, vec3<f32>(params.color.z, params.color.w, 1.0 - params.color.z - params.color.w));
}

/// [`gamut_map`] into the space whose luminance weights are `luma`.
fn gamut_map_in(rgb: vec3<f32>, luma: vec3<f32>) -> vec3<f32> {
    let l = clamp(dot(rgb, luma), 0.0, 1.0);
    var s = 1.0;
    if rgb.r > 1.0 {
        s = min(s, (1.0 - l) / (rgb.r - l));
    } else if rgb.r < 0.0 {
        s = min(s, l / (l - rgb.r));
    }
    if rgb.g > 1.0 {
        s = min(s, (1.0 - l) / (rgb.g - l));
    } else if rgb.g < 0.0 {
        s = min(s, l / (l - rgb.g));
    }
    if rgb.b > 1.0 {
        s = min(s, (1.0 - l) / (rgb.b - l));
    } else if rgb.b < 0.0 {
        s = min(s, l / (l - rgb.b));
    }
    s = clamp(s, 0.0, 1.0);
    if params.film.w > 0.5 {
        s = min(s, perceptual_scale(rgb, l));
    }
    return vec3<f32>(l) + s * (rgb - vec3<f32>(l));
}

/// The perceptual intent's curve: below `PERCEPTUAL_THRESHOLD` of the way to the
/// gamut edge a colour is left alone; from there to `PERCEPTUAL_LIMIT` (about
/// where a Rec.2020 primary lands against sRGB) is eased into what is left.
/// `PERCEPTUAL_POWER` shapes the roll-off. **Mirrors `gpu::color`.**
const PERCEPTUAL_THRESHOLD: f32 = 0.7;
const PERCEPTUAL_LIMIT: f32 = 2.5;
const PERCEPTUAL_POWER: f32 = 1.2;

/// The perceptual rendering intent's chroma scale.
///
/// Relative colorimetric (the default) moves only the colours that are out of
/// gamut, straight to its edge — so every colour beyond the edge lands on the
/// same one, and a saturated gradient flattens into a band. Perceptual measures
/// how far each colour is towards the edge (`t`: 1 at the boundary, above 1
/// outside it) and compresses everything past the threshold into the room left,
/// so colours outside the gamut stay distinct from each other, at the price of
/// slightly muting the most saturated in-gamut ones. Hue and luminance are
/// untouched either way.
///
/// **Mirrors `gpu::color::perceptual_scale`, the tested reference.**
fn perceptual_scale(rgb: vec3<f32>, l: f32) -> f32 {
    // How far the furthest channel sits towards its limit, 0..1 inside the gamut.
    var t = 0.0;
    for (var i = 0; i < 3; i++) {
        let c = rgb[i];
        if c > l && l < 1.0 {
            t = max(t, (c - l) / (1.0 - l));
        } else if c < l && l > 0.0 {
            t = max(t, (l - c) / l);
        }
    }
    let thr = PERCEPTUAL_THRESHOLD;
    if t <= thr {
        return 1.0;
    }
    let p = PERCEPTUAL_POWER;
    // Chosen so the limit lands exactly on the gamut edge (ACES's reference
    // gamut compression uses the same parametric curve).
    let k = (PERCEPTUAL_LIMIT - thr) / pow(pow((1.0 - thr) / (PERCEPTUAL_LIMIT - thr), -p) - 1.0, 1.0 / p);
    let x = (t - thr) / k;
    let compressed = thr + k * x / pow(1.0 + pow(x, p), 1.0 / p);
    return min(compressed, 1.0) / t;
}

/// Soft-proof gamut check. When enabled (`params.dehaze.z`), a colour that falls
/// outside the output (sRGB) gamut — any display-primary channel below 0 or above
/// 1 *before* `gamut_map` compresses it — is painted a flat marker so the
/// photographer can see exactly what will not survive the output profile, the way
/// a gamut check in other raw developers does. A small epsilon keeps colours that sit on the
/// boundary from flickering as marked.
const GAMUT_EPS = 1e-3;
const GAMUT_WARN = vec3<f32>(1.0, 0.0, 1.0);
fn out_of_gamut(rgb: vec3<f32>) -> bool {
    return any(rgb < vec3<f32>(-GAMUT_EPS)) || any(rgb > vec3<f32>(1.0 + GAMUT_EPS));
}

/// Reads the mosaic with enough samples to actually cover the output pixel.
///
/// The preview draws roughly 1400px from a 6000px sensor, so one output pixel
/// spans about eighteen photosites. A single bilinear tap reads four of them
/// and ignores the rest: high-contrast edges alias, and single-pixel demosaic
/// fringing survives as isolated coloured dots rather than averaging into its
/// surroundings. Measured against a properly resampled render, that put 58%
/// more high-frequency energy in the preview than belongs there — detail the
/// pipeline invented.
///
/// Letting the mip chain handle it alone overcorrects. At a 6.7x reduction the
/// hardware blends the divide-by-4 and divide-by-8 levels and leans mostly on
/// the latter, which is far softer than needed: 37% *under* the reference.
///
/// So: four taps on a rotated grid spanning the pixel's own footprint, each
/// biased one level sharper than the hardware would choose. Four taps at half
/// the linear extent cover the same area as one tap at full extent, which is
/// what puts the sampling density back where it belongs. `dpdx`/`dpdy` give
/// the footprint directly, so this stays correct at any output size — the
/// export path renders full-size, where the derivatives collapse and the taps
/// converge on the same texel.
fn sample_linear(uv: vec2<f32>) -> vec3<f32> {
    let dx = dpdx(uv);
    let dy = dpdy(uv);

    // Rotated-grid offsets. A regular 2x2 grid aligns its samples with
    // horizontal and vertical edges, which is where aliasing is most visible
    // and most common in a photograph.
    let a = 0.125 * dx + 0.375 * dy;
    let b = 0.375 * dx - 0.125 * dy;

    var acc = textureSampleBias(linear_rgb, linear_sampler, uv + a, -2.0).rgb;
    acc += textureSampleBias(linear_rgb, linear_sampler, uv + b, -2.0).rgb;
    acc += textureSampleBias(linear_rgb, linear_sampler, uv - a, -2.0).rgb;
    acc += textureSampleBias(linear_rgb, linear_sampler, uv - b, -2.0).rgb;
    return acc * 0.25;
}

/// Dehaze: remove (or add) atmospheric veil.
///
/// The dark-channel prior made cheap. Haze is an additive, low-saturation veil
/// of atmospheric light `A`, and the recovery is the standard `J = (I - A)/t + A`
/// — an expansion of contrast about the airlight, stronger where transmission
/// `t` is lower. Two approximations keep it to one texture fetch and no reduction
/// pass:
///   * `A` is the **broad local average** (a coarse mip), the atmosphere over
///     this region rather than a single global constant. That makes it adaptive
///     — a scene half-clear, half-hazy is handled correctly.
///   * transmission is estimated from how **grey and bright** that local average
///     is (`dark/bright`, the dark-channel prior): ~1 over hazy sky, ~0 over a
///     dark or saturated subject, so clear regions are left alone.
///
/// Positive amount lowers `t` (cuts veil, expands contrast, restores the colour
/// the haze washed out); negative raises it (adds veil). Zero returns the input
/// untouched, so the control is a genuine no-op at rest. Runs in scene-referred
/// linear light beside clarity, where the mip chain lives and the additive-veil
/// model is physically meaningful.
const DEHAZE_OMEGA: f32 = 0.9;
fn apply_dehaze(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let amount = params.dehaze.x;
    if abs(amount) < 1e-6 {
        return rgb;
    }
    let veil = textureSampleLevel(linear_rgb, linear_sampler, uv, params.dehaze.y).rgb;
    let dark = min(veil.r, min(veil.g, veil.b));
    let bright = max(veil.r, max(veil.g, veil.b)) + 1e-4;
    // Haze weight: how grey-and-bright the local atmosphere is. The dark-channel
    // prior — a hazy patch has an elevated darkest channel, a clear one does not.
    let haze = clamp(dark / bright, 0.0, 1.0);
    // Transmission, bounded away from zero so the recovery cannot blow up, and
    // above one so a negative amount can thicken the veil without inverting it.
    let t = clamp(1.0 - DEHAZE_OMEGA * amount * haze, 0.1, 3.0);
    return (rgb - veil) / t + veil;
}

/// Clarity and texture: local contrast at two scales.
///
/// Halation: a warm bloom spilling out of the brightest parts of the frame.
///
/// Takes a wide mip of the *source* — the same chain clarity and noise
/// reduction already build, so this is one extra fetch, not a separate blur
/// pass — keeps only what is above the threshold, and adds it back tinted
/// toward red. The tint is the physical part: the halo around a bright window
/// on film is orange-red because red light penetrates the emulsion furthest
/// before it reflects off the base.
///
/// `HALATION_LOD` is a fixed mip rather than a pixel radius so the bloom is the
/// same *scene* size in a fit preview and a full-size export — the same reason
/// grain is generated on source pixels rather than output pixels.
const HALATION_LOD: f32 = 5.0;
const HALATION_TINT: vec3<f32> = vec3<f32>(1.0, 0.42, 0.22);

fn apply_halation(rgb: vec3<f32>, source: vec2<f32>) -> vec3<f32> {
    let amount = params.film.x;
    if (amount <= 1e-4) {
        return rgb;
    }
    let blurred = textureSampleLevel(linear_rgb, linear_sampler, source, HALATION_LOD).rgb;
    let bright = max(dot(blurred, LUMA_REC2020) - params.film.y, 0.0);
    // Squared, so the bloom falls off fast: a highlight twice over the
    // threshold blooms four times as hard, which is how it reads on film and
    // keeps the effect off everything that is merely light.
    return rgb + HALATION_TINT * (bright * bright) * amount * 2.0;
}

/// A lattice point's random value in -0.5..0.5. An integer hash (PCG's output
/// permutation): `fract(sin(...))` runs out of f32 precision at grain-lattice
/// coordinates and leaves faint repeating stripes.
fn grain_hash(p: vec2<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 1597334677u) ^ (bitcast<u32>(p.y) * 3812015801u);
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0 - 0.5;
}

/// Value noise: the four surrounding lattice values blended with a smoothstep,
/// so a grain is a soft blob rather than a hard-edged cell.
fn grain_value(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = grain_hash(i);
    let b = grain_hash(i + vec2<i32>(1, 0));
    let c = grain_hash(i + vec2<i32>(0, 1));
    let d = grain_hash(i + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

/// Film grain at lattice coordinate `p`: a fine octave plus a coarser one on a
/// rotated, offset lattice, which clumps the grains the way an emulsion's do
/// and hides the square lattice's axis-aligned look. Scaled to the spread of
/// the uniform -0.5..0.5 noise it replaced (`GRAIN_NORM`, measured over a
/// 6000x4000 frame), so a saved grain amount looks as strong as it did.
const GRAIN_NORM: f32 = 1.3468;
fn film_grain(p: vec2<f32>) -> f32 {
    let r = mat2x2<f32>(0.6749, 0.7379, -0.7379, 0.6749); // 0.83 rad
    let coarse = grain_value((r * p) * 0.5 + vec2<f32>(17.31, 41.07));
    return (0.8 * grain_value(p) + 0.6 * coarse) * GRAIN_NORM;
}

/// Both compare the image against a blurred copy and push the difference. The
/// blurs come from the mip chain that already exists for resampling — a
/// 120-pixel blur would otherwise cost hundreds of taps, and here it is one
/// already-filtered fetch.
///
/// Split into two **bands** rather than two independent unsharp masks. Texture
/// gets what lies between the sharp image and the fine blur; clarity gets what
/// lies between the fine and coarse blurs. Without the split, turning both up
/// would apply the fine detail twice, and the two sliders would fight.
///
/// Applied as a ratio on linear light, so the adjustment scales with the pixel
/// and leaves hue alone — an additive version washes out shadows, where there
/// is least signal to push.
/// The texture and clarity bands at `uv` — local contrast at a fine and a
/// coarse scale, relative to the local average — read from the linear image
/// the way `apply_detail` reads them. Masks use these for local clarity and
/// texture, after the global stage has moved on.
fn detail_bands(uv: vec2<f32>) -> vec2<f32> {
    let sharp = textureSampleLevel(linear_rgb, linear_sampler, uv, 0.0).rgb;
    let fine = textureSampleLevel(linear_rgb, linear_sampler, uv, params.detail.z).rgb;
    let coarse = textureSampleLevel(linear_rgb, linear_sampler, uv, params.detail.w).rgb;
    let l_sharp = dot(sharp, LUMA_REC2020);
    let l_fine = dot(fine, LUMA_REC2020);
    let l_coarse = dot(coarse, LUMA_REC2020);
    return vec2<f32>(
        (l_sharp - l_fine) / max(l_fine, 1e-4),
        (l_fine - l_coarse) / max(l_coarse, 1e-4),
    );
}

fn apply_detail(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let amounts = params.detail.xy;
    if abs(amounts.x) < 1e-4 && abs(amounts.y) < 1e-4 {
        return rgb;
    }

    let fine = textureSampleLevel(linear_rgb, linear_sampler, uv, params.detail.z).rgb;
    let coarse = textureSampleLevel(linear_rgb, linear_sampler, uv, params.detail.w).rgb;

    let l_sharp = dot(rgb, LUMA_REC2020);
    let l_fine = dot(fine, LUMA_REC2020);
    let l_coarse = dot(coarse, LUMA_REC2020);

    // Relative to the local average rather than absolute, which is what makes
    // one setting behave the same on a bright wall and in a shadow.
    let texture_band = (l_sharp - l_fine) / max(l_fine, 1e-4);
    let clarity_band = (l_fine - l_coarse) / max(l_coarse, 1e-4);

    // Bounded: an unclamped ratio explodes where the local average approaches
    // zero, and a single dark pixel would bloom into a white speck.
    let boost = clamp(
        1.0 + amounts.x * texture_band + amounts.y * clarity_band,
        0.25,
        4.0,
    );
    return rgb * boost;
}

/// Noise reduction (luminance + chroma) and capture sharpening.
///
/// **All three are per-pixel and only mean anything near 1:1.** At fit-to-window
/// each output pixel already averages many source pixels, so a one-pixel radius
/// is below what the render can represent. That is correct — and why they could
/// not honestly be built before the viewer could zoom.
///
/// Order is not optional: **denoise, then sharpen** — sharpen first and it
/// amplifies the grain, after which no smoothing separates them again. Luminance
/// NR is two-scale: a bilateral over the shared eight-neighbour gather removes
/// fine grain edge-preservingly, then a wider mip blur pulls out the
/// low-frequency blotch a small window cannot reach, in flat areas only. Chroma
/// NR is separate and works from the mip chain alone.
///
/// `lod` is passed in so every gathered tap reads the base sample's mip level;
/// the coarse bands use their own render-size-corrected levels (`params.nr`) so
/// the preview and the export blur the same scene scale.
fn apply_pixel_detail(rgb: vec3<f32>, uv: vec2<f32>, lod: f32) -> vec3<f32> {
    let noise = params.sharpen.w;
    let amount = params.sharpen.x;
    let cnoise = params.nr.x;
    if noise < 1e-4 && amount < 1e-4 && cnoise < 1e-4 {
        return rgb;
    }

    let centre_luma = dot(rgb, LUMA_REC2020);
    var out = rgb;

    // The eight-neighbour gather is only needed by the luminance bilateral and
    // the sharpen; chroma NR works from the mip chain alone, so skip it when the
    // only active control is colour noise.
    if noise >= 1e-4 || amount >= 1e-4 {
        let texel = 1.0 / vec2<f32>(textureDimensions(linear_rgb, 0));
        let step = texel * max(params.sharpen.y, 0.25);
        var neighbours = array<vec3<f32>, 8>(
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(-step.x, -step.y), lod).rgb,
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(0.0, -step.y), lod).rgb,
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(step.x, -step.y), lod).rgb,
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(-step.x, 0.0), lod).rgb,
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(step.x, 0.0), lod).rgb,
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(-step.x, step.y), lod).rgb,
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(0.0, step.y), lod).rgb,
            textureSampleLevel(linear_rgb, linear_sampler, uv + vec2(step.x, step.y), lod).rgb,
        );

        var mean = vec3<f32>(0.0);
        for (var i = 0; i < 8; i++) {
            mean += neighbours[i];
        }
        mean /= 8.0;
        let local_contrast = abs(centre_luma - dot(mean, LUMA_REC2020)) / max(centre_luma, 0.01);

        if noise >= 1e-4 {
            // Fine band: a bilateral average. Neighbours count in proportion to
            // how similar they are, so grain in a flat area averages away while
            // an edge (neighbours differ a lot) is left alone. The window widens
            // as the control rises — a stronger setting accepts larger
            // differences as noise — and scales with brightness because sensor
            // noise does.
            let width = mix(0.01, 0.22, noise) * max(centre_luma, 0.02);
            var sum = rgb;
            var weight = 1.0;
            for (var i = 0; i < 8; i++) {
                let l = dot(neighbours[i], LUMA_REC2020);
                let w = exp(-((l - centre_luma) * (l - centre_luma)) / (width * width));
                sum += neighbours[i] * w;
                weight += w;
            }
            out = mix(rgb, sum / weight, noise);

            // Coarse band: the low-frequency luminance blotch the small window
            // cannot reach. Pull the luma toward a wider mip blur, but only in
            // flat areas — an edge or a smooth gradient keeps its own brightness
            // (there the blur equals the pixel, so this is a no-op). Chroma is
            // untouched here; only luminance is matched.
            //
            // The blur must be genuinely coarser than the *current* view, or at a
            // proxy scale it collapses onto the base sampling level and does
            // nothing — so it is at least 1.5 mip levels above what this pixel is
            // already sampled at, and no finer than the scene-referred level.
            let coarse_lod = max(params.nr.z, lod + 1.5);
            let low = textureSampleLevel(linear_rgb, linear_sampler, uv, coarse_lod).rgb;
            let flat = 1.0 - smoothstep(0.02, 0.12, local_contrast);
            let l_low = dot(low, LUMA_REC2020);
            let l_out = dot(out, LUMA_REC2020);
            // Shifted rather than scaled to the blur's luma. In a flat area the
            // two agree; in a noisy shadow, where a pixel's luma sits either
            // side of zero, dividing by it flung pixels to extremes and left a
            // colour cast behind.
            out = out + vec3<f32>((l_low - l_out) * flat * noise * 0.7);
        }

        if amount >= 1e-4 {
            // Unsharp mask against the neighbourhood mean. Masking withholds it
            // from flat areas, where it would only amplify sensor noise.
            let mask = mix(1.0, smoothstep(0.0, 0.06, local_contrast), params.sharpen.z);
            let fine = out - mean;
            var hp = fine;

            // Deconvolution character (the "Detail" control). A plain unsharp
            // mask corrects a Gaussian blur to first order and leaves a halo; a
            // second van Cittert iteration flattens the inverse response, boosting
            // the finest band and subtracting the mid band so fine detail crisps
            // up with less halo. The iteration needs G(G(I)) — approximated by one
            // coarser mip level — so it costs a single extra tap and only when the
            // control is up. `hp = fine + detail*(fine - mid)` is that step with
            // the relaxation folded into `detail`; at 0 it is exactly the unsharp
            // mask above, byte-for-byte.
            let detail = params.nr.w;
            if detail >= 1e-4 {
                let blur2 = textureSampleLevel(linear_rgb, linear_sampler, uv, lod + 1.0).rgb;
                let mid = mean - blur2;
                hp = fine + detail * (fine - mid);
            }
            out = out + hp * amount * 2.0 * mask;
        }
    }

    if cnoise >= 1e-4 {
        // Chroma noise: colour speckle is coarse and photographs have few real
        // chroma edges, so replacing each pixel's chroma with a wide mip blur
        // removes the blotches with little visible loss — as long as luminance
        // is kept. Scale the blurred colour back to the pixel's own luma, so
        // detail and tone survive and only the hue/saturation are smoothed.
        let blur = textureSampleLevel(linear_rgb, linear_sampler, uv, params.nr.y).rgb;
        let lc = dot(out, LUMA_REC2020);
        let lb = dot(blur, LUMA_REC2020);
        // Scaled where the surroundings are lit, which keeps hue right at an
        // edge between light and dark. In a deep shadow the surroundings'
        // luma is itself noise about zero, and dividing by it threw pixels to
        // full white; there the colour is shifted to the pixel's luma instead.
        // Both put the pixel at its own luma.
        let scaled = blur * (lc / max(lb, 1e-4));
        let shifted = blur + vec3<f32>(lc - lb);
        let lit = smoothstep(0.002, 0.02, lb);
        out = mix(out, mix(shifted, scaled, lit), cnoise);
    }

    // Not clamped at zero: this is camera light, before the colour transform,
    // and a shadow's noise averages to the scene only if both sides of zero
    // survive (see `tap` in mosaic.wgsl). The display encode clips.
    return out;
}

/// Squared distance from the optical centre, normalised so a corner is 1.
///
/// Measured in *pixels* rather than normalised coordinates: on a 3:2 frame the
/// two axes are not the same length, so a circle in uv space is an ellipse on
/// the sensor. A lens does not know about aspect ratios.
fn radius2(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(linear_rgb, 0));
    let d = (uv - vec2<f32>(0.5)) * size;
    let corner = 0.5 * size;
    return dot(d, d) / max(dot(corner, corner), 1e-6);
}

/// Distortion, applied to the sampling position.
///
/// The optical centre is the *sensor's* centre, so this belongs after `orient`
/// and `crop` have mapped back to sensor space — correcting about the centre of
/// a crop would bend the picture around the wrong point.
fn lens_distort(uv: vec2<f32>, r2: f32) -> vec2<f32> {
    let k = params.lens.x;
    if abs(k) < 1e-6 {
        return uv;
    }
    let size = vec2<f32>(textureDimensions(linear_rgb, 0));
    let d = (uv - vec2<f32>(0.5)) * size;
    // Reading from further out pulls the corners in, which is what corrects
    // barrel distortion — hence sampling at a *larger* radius for positive k.
    return vec2<f32>(0.5) + d * (1.0 + k * r2) / size;
}

/// Lateral chromatic aberration: red and blue land at slightly different
/// magnifications, so correcting it means scaling them apart again.
///
/// Three separate samples, and only when the control is off zero — it triples
/// the cost of the read, and every photo that does not need it should not pay.
fn sample_corrected(uv: vec2<f32>, r2: f32) -> vec3<f32> {
    let ca = params.lens.z;
    let base = lens_distort(uv, r2);
    if abs(ca) < 1e-6 {
        return sample_linear(base);
    }

    let size = vec2<f32>(textureDimensions(linear_rgb, 0));
    let d = (base - vec2<f32>(0.5)) * size;
    let red = vec2<f32>(0.5) + d * (1.0 + ca) / size;
    let blue = vec2<f32>(0.5) + d * (1.0 - ca) / size;
    return vec3<f32>(
        sample_linear(red).r,
        sample_linear(base).g,
        sample_linear(blue).b,
    );
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    let source = orient(crop(in.uv));
    let r2 = radius2(source);
    var rgb = sample_corrected(source, r2);

    // Spot removal: for a pixel under a spot, replace it with a patch sampled
    // from the spot's source location. Done first, on the sampled mosaic value,
    // so the rest of develop processes the repaired pixel like any other.
    let scount = u32(spots.header.x);
    if scount > 0u {
        let framed = spots.view.zw + (in.uv - vec2<f32>(0.5)) * spots.view.xy;
        var best = 0.0;
        var off = vec2<f32>(0.0);
        var heal = 0.0;
        for (var i = 0u; i < scount; i = i + 1u) {
            let a = spots.data[i * 2u];       // [tx, ty, sx, sy]
            let b = spots.data[i * 2u + 1u];  // [radius, feather, heal, 0]
            let d = distance(framed, vec2<f32>(a.x, a.y));
            let r = max(b.x, 1e-4);
            let inner = r * (1.0 - clamp(b.y, 0.0, 1.0));
            let cov = 1.0 - smoothstep(inner, r, d);
            if cov > best {
                best = cov;
                off = vec2<f32>(a.z - a.x, a.w - a.y); // source - target, framed UV
                heal = b.z;
            }
        }
        if best > 0.0 {
            // Framed-UV offset back to drawn-quad UV (÷ the zoom extent), then to
            // sensor space through the same crop/orient the base sample used.
            let src_in_uv = in.uv + off / max(spots.view.xy, vec2<f32>(1e-4));
            let src_sensor = orient(crop(src_in_uv));
            var fill = sample_corrected(src_sensor, radius2(src_sensor));
            if heal > 0.5 {
                // Heal: keep the source's detail but the target's local tone.
                // Swap low frequencies via the mosaic mip chain — a cheap
                // gradient-domain approximation.
                let src_low = textureSampleLevel(linear_rgb, linear_sampler, src_sensor, 5.0).rgb;
                let tgt_low = textureSampleLevel(linear_rgb, linear_sampler, source, 5.0).rgb;
                fill = max(fill - src_low + tgt_low, vec3<f32>(0.0));
            }
            rgb = mix(rgb, fill, best);
        }
    }

    // Vignetting, before anything else touches the values: it is a statement
    // about how much light the lens delivered, so it belongs with the exposure
    // rather than after the tone curve has already reshaped it.
    let vignette = params.lens.y;
    if abs(vignette) > 1e-6 {
        rgb = rgb * (1.0 + vignette * r2);
    }

    // How many source pixels this output pixel spans, as a mip level. Computed
    // once at top level, where derivatives are valid, and handed to the
    // per-pixel filters so their taps match the base sample's scale.
    let size = vec2<f32>(textureDimensions(linear_rgb, 0));
    let footprint = max(length(dpdx(source) * size), length(dpdy(source) * size));
    let lod = max(log2(max(footprint, 1e-6)), 0.0);

    rgb = apply_pixel_detail(rgb, source, lod);

    // 0. Dehaze, before clarity: it is the broadest veil-removal, and estimating
    //    the atmospheric light from the un-sharpened scene keeps the estimate
    //    stable. Scene-referred, where the additive-haze model holds.
    rgb = apply_dehaze(rgb, source);

    // 0b. Local contrast, before anything else touches the values: it is a
    //    statement about the scene's structure, and running it after the tone
    //    curve would make its strength depend on the exposure slider.
    rgb = apply_detail(rgb, source);

    // 0c. Halation, while the frame is still scene-referred and additive.
    //
    //    On film, light that punches through the emulsion reflects off the base
    //    and re-exposes the grains around it — red furthest, because red
    //    penetrates deepest. It happens at capture, so it belongs here, ahead of
    //    tone and grade: bloom added after the curve would sit on top of a
    //    finished image instead of being shaped by it, which is exactly what
    //    makes a screen-blend "glow" filter look pasted on.
    //
    //    The wide blur is one mip fetch. The chain already exists for clarity
    //    and noise reduction, so the whole effect costs a single extra sample.
    rgb = apply_halation(rgb, source);

    // 1. User white balance, in camera space while the channels still
    //    correspond to the sensor's own primaries.
    rgb = rgb * params.wb.rgb;

    // 2. Exposure, in linear light, where a stop is a factor of two.
    rgb = rgb * params.exposure;

    // 3. Camera primaries into the working space (linear Rec.2020). Still
    //    scene-referred: values well above 1.0 are normal here.
    rgb = params.cam_to_working * rgb;

    // 4. Tone. This is the step that turns open-ended scene light into an
    //    image a display can show, and the reason an untoned raw looks flat.
    //    Applied per channel in the working space, which is what gives the
    //    film-like warm shift in the highlights rather than a flat grey clip.
    rgb = apply_tone(rgb);

    // 4b. Per-channel (R/G/B) point curves, on the same display-referred values
    //     the composite tone curve produces — editors keep them in the Tone
    //     Curve panel for exactly this reason. Identity until the user drags one.
    rgb = channel_curve(rgb);

    // 5. Colour mixer, on display-referred values. After tone rather than
    //    before, so a saturation change behaves the way it looks on screen
    //    instead of being reshaped by the curve afterwards.
    rgb = apply_mixer(rgb);
    rgb = apply_perceptual_mixer(rgb);

    // 6. Colour grading. After the mixer, so the wheels tint what the mixer
    //    produced rather than being reshaped by it — and still in the working
    //    space, where the luminance weights the range split uses are correct.
    rgb = apply_grade(rgb);

    // 6a. Global vibrance and saturation, on the graded result. A finishing
    //     colour-intensity control, in the working space alongside the mixer
    //     and grade so its luminance weights are the ones those stages use.
    rgb = apply_saturation(rgb);

    // 6b. Local adjustments. Each mask's coverage is evaluated in framed-image
    //     UV (the drawn UV mapped back through the zoom), and its adjustments
    //     are blended by that coverage — a dodge/burn on the developed image.
    let mask_count = u32(masks.header.x);
    let comp_count = u32(masks.header.y);
    // The mask whose coverage is shown, tinted, while masking: every
    // kind, combined as the mask combines them. Zero is none.
    var show_cov = 0.0;
    if mask_count > 0u {
        let framed = masks.view.zw + (in.uv - vec2<f32>(0.5)) * masks.view.xy;
        // First combine each mask's components into its coverage, then apply
        // that mask's adjustments. Coverage is evaluated against the developed
        // colour *before* any mask is applied, so a value (luminance/colour)
        // component reads the image the way the user sees it.
        var mcov = array<f32, 8>();   // MAX_MASKS
        var seen = array<f32, 8>();
        for (var c = 0u; c < comp_count; c = c + 1u) {
            let g0 = masks.comps[c * 2u];
            let g1 = masks.comps[c * 2u + 1u];
            let cc = component_coverage(g0, g1, framed, rgb);
            let packed = i32(round(g1.w));
            let mode = (packed / 2) % 4;   // 0 add, 1 subtract, 2 intersect
            let mi = u32(packed / 8);
            if seen[mi] < 0.5 {
                mcov[mi] = cc;   // first component starts the accumulation
                seen[mi] = 1.0;
            } else if mode == 0 {
                mcov[mi] = max(mcov[mi], cc);
            } else if mode == 1 {
                mcov[mi] = mcov[mi] * (1.0 - cc);
            } else {
                mcov[mi] = min(mcov[mi], cc);
            }
        }
        let shown = i32(round(masks.header.z)) - 1;
        if shown >= 0 && u32(shown) < mask_count {
            show_cov = mcov[u32(shown)];
        }
        // The detail bands are three texture reads, so only when some mask
        // asks for local clarity or texture.
        var bands = vec2<f32>(0.0);
        for (var m = 0u; m < mask_count; m = m + 1u) {
            let a2 = masks.adjs[m * 3u + 2u];
            if a2.z != 0.0 || a2.w != 0.0 {
                bands = detail_bands(source);
                break;
            }
        }
        for (var m = 0u; m < mask_count; m = m + 1u) {
            let cov = mcov[m];
            if cov > 0.0 {
                rgb = apply_local(
                    rgb,
                    cov,
                    masks.adjs[m * 3u],
                    masks.adjs[m * 3u + 1u],
                    masks.adjs[m * 3u + 2u],
                    bands,
                );
            }
        }
    }

    // 7. Working space to the display's primaries.
    rgb = params.working_to_display * rgb;

    // 7-proof. Soft-proof gamut check: note whether this colour is outside the
    //     output gamut *now*, before the gamut map pulls it back in, so the
    //     overlay marks exactly what would otherwise be compressed. Painted in at
    //     the very end so the marker is never itself reshaped by later stages.
    let marked = params.dehaze.z > 0.5 && out_of_gamut(rgb);

    // 7a. Gamut map: a colour the sensor caught outside the display's gamut is
    //     desaturated toward its equal-luminance grey at constant hue, rather
    //     than clipped per channel (which would rotate its hue). Done before the
    //     finishing effects, which add achromatic texture the final clamp mops up.
    rgb = gamut_map(rgb);

    // 7b. Finishing effects on the framed, display-referred image: a post-crop
    //     vignette about the *cropped* frame's centre (distinct from the lens
    //     vignette earlier, which is about the sensor's) and film grain, which
    //     is faded out of the highlights the way an emulsion's is. Both stay
    //     pinned to the photo under a zoom: the vignette on the framed UV (via
    //     the mask view mapping), the grain on source pixels.
    let eff = params.effects;
    if abs(eff.x) > 1e-4 || eff.z > 1e-4 {
        let framed = masks.view.zw + (in.uv - vec2<f32>(0.5)) * masks.view.xy;
        if abs(eff.x) > 1e-4 {
            // Elliptical falloff fitting the frame: ~0 at centre, ~1 at a corner.
            let r = length(framed - vec2<f32>(0.5)) * 1.41421356;
            let falloff = smoothstep(clamp(eff.y, 0.0, 0.98), 1.0, r);
            rgb = rgb * (1.0 + eff.x * falloff);
        }
        if eff.z > 1e-4 {
            // On the source texture's own pixels, scaled to a fixed density
            // along its long edge: square grains (framed UV stretched them
            // with the frame's aspect), pinned to the photo under a zoom, and
            // the same scene size in a preview and an export.
            let dims = vec2<f32>(textureDimensions(linear_rgb, 0));
            let gp = orient(crop(in.uv)) * dims
                * (2600.0 / (max(dims.x, dims.y) * max(eff.w, 0.1)));
            let n = film_grain(gp);
            // Fade the grain out of the highlights. Uniform noise is the
            // giveaway of fake grain: on film the highlights are where the
            // emulsion is most exposed and reads smoothest, so grain belongs in
            // the mid-tones and shadows. `params.film.z` at 0 restores the old
            // flat behaviour.
            let l = clamp(dot(rgb, LUMA_REC2020), 0.0, 1.0);
            let mask = mix(1.0, 1.0 - smoothstep(0.45, 1.0, l), clamp(params.film.z, 0.0, 1.0));
            rgb = rgb + n * eff.z * 0.12 * mask;
        }
    }

    // 8. Clamp, then encode. Clipping happens here and nowhere earlier, so
    //    every preceding stage keeps its out-of-range highlights.
    let clamped = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    var out = select(encode_srgb(clamped), encode_gamma22(clamped), params.dehaze.w > 0.5);

    // 9. Creative "look" LUT — the final stage, applied to the sRGB-encoded
    //    image (the space `.cube` looks are authored for), blended by intensity.
    //    Skipped entirely at intensity 0. A wide-gamut output hands the LUT an
    //    sRGB rendition instead of its own encoding, which the LUT would read
    //    as the wrong colours.
    if params.lut_intensity > 0.0 {
        var looked: vec3<f32>;
        if srgb_output() {
            looked = sample_lut(out);
        } else {
            looked = look_through_srgb(clamped);
        }
        out = mix(out, looked, params.lut_intensity);
    }

    // The shown mask's coverage, over everything but the gamut marker.
    if show_cov > 0.0 {
        out = mix(out, vec3<f32>(0.95, 0.2, 0.2), 0.5 * clamp(show_cov, 0.0, 1.0));
    }
    // Soft-proof marker last of all, so nothing tints it.
    if marked {
        out = GAMUT_WARN;
    }
    return vec4<f32>(out, 1.0);
}
