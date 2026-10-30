// Pass A2: highlights blown in one channel, rebuilt from the colour around
// them.
//
// Where a photosite clips, the sensor stops saying how bright that channel
// was; the others, if they did not clip, still say something. The sunlit side
// of an orange, skin in hard light, a yellow petal: each clips in its
// strongest channel first, and with that channel stuck while the others keep
// climbing, its hue drifts towards them — an orange goes yellow.
//
// So the lost channel is borrowed from where it was measured. The mosaic pass
// writes each trustworthy pixel's chromaticity — bright, and nowhere near a
// clipped photosite — and a mip chain of that spreads it: the first level
// with any weight at a clipped pixel holds the colour of its nearest measured
// surroundings. The clipped channel is rebuilt to that colour's proportion of
// the two measured ones, and never below where it clipped; the measured
// channels stay as measured.
//
// Only where exactly one channel clipped, because only there can a borrowed
// colour be checked: its ratio between the two measured channels has to be
// the pixel's own. A white cloud against blue sky borrows blue, and its red
// and blue say it is not; with two channels clipped there is one number to
// check against, and a Leica's blown sky borrowed blue unopposed and lost its
// clouds. Anything that cannot be checked, or fails, gets what every clipped
// pixel got before this pass existed: white if near neutral, left alone if
// not (`before`) — and so does anything near neutral, which that whitens.

struct Params {
    // Where each channel clips, after white balance: the balance's own gains,
    // since the mosaic pass normalises the white level to 1 before them.
    ceiling: vec4<f32>,
    // [levels in the chroma chain, unused, unused, unused].
    levels: vec4<f32>,
}

@group(0) @binding(0) var demosaiced: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;
@group(0) @binding(2) var chroma_sampler: sampler;
@group(0) @binding(3) var out_linear: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var<uniform> params: Params;

/// How clipped each channel is, 0..1. **The same ramp as `mosaic.wgsl`'s**,
/// which decides which pixels lend their colour.
fn clipped(rgb: vec3<f32>) -> vec3<f32> {
    return smoothstep(vec3<f32>(0.90), vec3<f32>(0.98), rgb / max(params.ceiling.rgb, vec3<f32>(1e-6)));
}

/// What a blown highlight got before colour was rebuilt, and still gets
/// wherever it cannot be: green saturating on something near neutral goes
/// white. Green is the first channel to run out on a bright neutral, and past
/// that point white balance paints it magenta; near neutral rather than
/// always, so a saturated green is not bleached. Mirrored for linear DNGs by
/// `linear_pixel` in `gpu/pipeline.rs`.
fn before(rgb: vec3<f32>) -> vec3<f32> {
    return mix(rgb, vec3<f32>(max(rgb.r, max(rgb.g, rgb.b))), whitening(rgb));
}

/// How far `before` takes a pixel to white, 0..1.
fn whitening(rgb: vec3<f32>) -> f32 {
    let saturated = smoothstep(0.88, 1.0, rgb.g / max(params.ceiling.g, 1e-6));
    let neutralish = smoothstep(0.5, 0.8, min(rgb.r, rgb.b) / max(rgb.g, 1e-6));
    return saturated * neutralish;
}

/// The two measured channels' ratio, and the lost one's value against their
/// sum, for the pixel and for a colour: `(ratio, share)`.
fn split(v: vec3<f32>, lost: u32) -> vec2<f32> {
    var a = v.g;
    var b = v.b;
    var c = v.r;
    if lost == 1u {
        a = v.r;
        c = v.g;
    } else if lost == 2u {
        a = v.r;
        b = v.g;
        c = v.b;
    }
    return vec2<f32>(a / max(b, 1e-6), c / max(a + b, 1e-6));
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(out_linear);
    if gid.x >= size.x || gid.y >= size.y {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let rgb = textureLoad(demosaiced, p, 0).rgb;
    let fallback = before(rgb);
    let clip = clipped(rgb);

    // Exactly one channel clipped, the others clear of it.
    let hard = clip > vec3<f32>(0.5);
    let count = u32(hard.r) + u32(hard.g) + u32(hard.b);
    if count != 1u {
        textureStore(out_linear, p, vec4<f32>(fallback, 1.0));
        return;
    }
    var lost = 0u;
    if hard.g {
        lost = 1u;
    } else if hard.b {
        lost = 2u;
    }

    // The nearest measured colour: the finest level that has any weight.
    // Level 0 is this pixel's own, and it is clipped.
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(size);
    var k = vec3<f32>(0.0);
    var found = false;
    for (var level = 1.0; level < params.levels.x; level = level + 1.0) {
        let s = textureSampleLevel(chroma, chroma_sampler, uv, level);
        if s.w > 0.005 {
            k = max(s.rgb / s.w, vec3<f32>(0.0));
            found = true;
            break;
        }
    }
    if !found {
        textureStore(out_linear, p, vec4<f32>(fallback, 1.0));
        return;
    }

    let own = split(rgb, lost);
    let borrowed = split(k, lost);
    // Within 2% the colour is the pixel's; past 6% it is someone else's.
    let mismatch = abs(log(max(own.x, 1e-6) / max(borrowed.x, 1e-6)));
    let agreement = 1.0 - smoothstep(0.02, 0.06, mismatch);

    // The lost channel at the borrowed colour's share of the measured two,
    // never below where it clipped.
    var colour = rgb;
    let measured = rgb.r + rgb.g + rgb.b - rgb[lost];
    colour[lost] = max(borrowed.y * measured, rgb[lost]);

    // Fully trusted only once the channel is well past clipping and the
    // measured ones are well clear of it, so nothing steps at either edge.
    var others = clip;
    others[lost] = 0.0;
    //
    // And not where `before` would whiten it. Near neutral, a blown area
    // whitening towards its core reads as light; rebuilt, the rim kept its
    // faint tint around a core that has none to borrow (all three channels
    // clipped), and on an out-of-focus sky through leaves that was a
    // cream-coloured halo. So rebuilding is for the colours `before` leaves
    // to drift: the saturated ones, and those that clip in red or blue.
    let trust = agreement * smoothstep(0.5, 1.0, clip[lost])
        * (1.0 - smoothstep(0.0, 0.5, max(others.r, max(others.g, others.b))))
        * (1.0 - whitening(rgb));
    textureStore(out_linear, p, vec4<f32>(mix(fallback, colour, trust), 1.0));
}
