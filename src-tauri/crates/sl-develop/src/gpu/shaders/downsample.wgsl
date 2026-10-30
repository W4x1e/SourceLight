// Halves the linear image, one mip level per dispatch.
//
// The develop pass draws a proxy — around 1400px from a 6000px sensor — by
// sampling the mosaic with a single bilinear tap. That tap covers four
// photosites out of the roughly eighteen each output pixel actually spans, so
// everything in between is simply skipped: high-contrast edges alias, and
// single-pixel demosaic fringing survives as isolated coloured dots instead of
// averaging away into its surroundings.
//
// A mip chain fixes both, because the hardware then picks a level whose texels
// already cover the output pixel. It is built once per photo alongside the
// mosaic, so no part of this cost lands on a slider drag.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_size = textureDimensions(dst);
    if gid.x >= out_size.x || gid.y >= out_size.y {
        return;
    }

    // A plain 2x2 box average. Odd dimensions halve down to a level that is
    // one texel short, so the read is clamped rather than wrapped — sampling
    // past the edge would fold the opposite side of the photo into its border.
    let limit = vec2<i32>(textureDimensions(src)) - vec2<i32>(1, 1);
    let base = vec2<i32>(i32(gid.x) * 2, i32(gid.y) * 2);

    var sum = vec4<f32>(0.0);
    for (var dy = 0; dy < 2; dy++) {
        for (var dx = 0; dx < 2; dx++) {
            sum += textureLoad(src, min(base + vec2<i32>(dx, dy), limit), 0);
        }
    }

    textureStore(dst, vec2<i32>(i32(gid.x), i32(gid.y)), sum * 0.25);
}
