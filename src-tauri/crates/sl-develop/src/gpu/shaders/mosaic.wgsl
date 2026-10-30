// Pass A: colour-filter-array mosaic -> linear camera RGB.
//
// Runs once per photo, not per slider tick. Demosaicing is the expensive part
// of the pipeline, and nothing a user adjusts changes its result, so its output
// is cached and every interactive edit runs only the cheap develop pass.
//
// Three things happen here, in this order:
//   1. black/white level normalisation, per colour channel
//   2. the camera's *as-shot* white balance
//   3. demosaic
//
// White balance belongs before demosaic rather than after: interpolation
// assumes neighbouring photosites carry comparable magnitudes, and on an
// unbalanced sensor the red channel can sit at half the green, which biases
// every gradient the interpolation looks at. The user's own white-balance
// adjustment is a separate, post-demosaic multiply in the develop pass, so
// moving that slider does not re-run any of this.

struct Params {
    active_size: vec2<u32>,
    active_origin: vec2<u32>,
    // 2 for Bayer, 6 for X-Trans.
    period: u32,
    _pad0: u32,
    _pad1: vec2<u32>,
    // Indexed by colour, matching `tile`: 0 red, 1 green, 2 blue, 3 second green.
    black: vec4<f32>,
    white: vec4<f32>,
    wb: vec4<f32>,
    // Colour index per position in the repeating tile, packed 4 per vec4.
    // 36 entries covers X-Trans; Bayer uses the first 4.
    tile: array<vec4<u32>, 9>,
}

@group(0) @binding(0) var cfa: texture_2d<u32>;
@group(0) @binding(1) var out_linear: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: Params;
// Each pixel's chromaticity times how far to trust it, and that trust: what
// `highlights.wgsl` borrows colour from for a blown highlight.
@group(0) @binding(3) var out_chroma: texture_storage_2d<rgba16float, write>;

/// Colour index at a tile position, wrapping correctly for negative
/// coordinates so callers can probe neighbours at the image edge.
fn color_index(x: i32, y: i32) -> u32 {
    let p = i32(params.period);
    let mx = ((x % p) + p) % p;
    let my = ((y % p) + p) % p;
    let i = u32(my * p + mx);
    return params.tile[i / 4u][i % 4u];
}

/// Treats the second green as green. Sensors distinguish the two greens for
/// black-level and white-balance purposes, but they are the same colour to a
/// demosaic.
fn as_rgb(index: u32) -> u32 {
    if index == 3u {
        return 1u;
    }
    return index;
}

/// One photosite, normalised to 0..1 and white-balanced.
///
/// Coordinates are relative to the active area, and clamped to it — never to
/// the full sensor. The margins outside hold masked calibration pixels, and
/// letting those bleed into the image edge produces a dark border.
fn tap(x: i32, y: i32) -> f32 {
    let cx = clamp(x, 0, i32(params.active_size.x) - 1);
    let cy = clamp(y, 0, i32(params.active_size.y) - 1);

    let sensor = vec2<i32>(
        cx + i32(params.active_origin.x),
        cy + i32(params.active_origin.y),
    );
    let raw = f32(textureLoad(cfa, sensor, 0).r);

    let ci = color_index(cx, cy);
    let black = params.black[ci];
    let range = max(params.white[ci] - black, 1.0);
    // **Not clamped at black.** In a dark area the readings scatter either
    // side of it, and keeping only the upper half leaves a bias that white
    // balance then multiplies by 2 on red and 3 or 4 on blue: every noisy
    // shadow turned purple, and no averaging — the preview's downsample,
    // noise reduction — could take it out again. Signed, the scatter averages
    // to the scene; the display encode clips what is still below zero.
    return (raw - black) / range * params.wb[ci];
}

/// Keeps an interpolated value inside the range of the samples it came from.
///
/// The gradient correction has negative lobes, so at a hard edge it overshoots:
/// on a synthetic step it drove a channel past every neighbour, and the tone
/// curve magnified that into a 184/255 colour fringe — worse than the plain
/// bilinear it replaced. Bounding each estimate by the measured samples around
/// it keeps the sharpening where the signal is smooth and falls back to
/// something sane at an edge, which is where the correction stops being
/// trustworthy anyway.
fn bounded4(value: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    let lo = min(min(a, b), min(c, d));
    let hi = max(max(a, b), max(c, d));
    return clamp(value, lo, hi);
}

/// Green at any photosite: measured where green was sampled, estimated along
/// the local edge direction everywhere else.
///
/// Green is reconstructed first and on its own because it is sampled at twice
/// the density of red and blue, so it carries the luminance detail the other
/// two are then hung off.
///
/// **Interpolating along an edge rather than across it is the point.** At a
/// vertical edge the horizontal neighbours straddle it and their average
/// describes nothing in the scene, while the vertical pair sit on one side and
/// are both right. Comparing the two gradients and favouring the smoother
/// direction costs almost nothing, since the taps are already loaded.
fn green_at(x: i32, y: i32) -> f32 {
    if as_rgb(color_index(x, y)) == 1u {
        return tap(x, y);
    }

    let center = tap(x, y);
    let n = tap(x, y - 1);
    let s = tap(x, y + 1);
    let e = tap(x + 1, y);
    let w = tap(x - 1, y);
    let nn = tap(x, y - 2);
    let ss = tap(x, y + 2);
    let ee = tap(x + 2, y);
    let ww = tap(x - 2, y);

    // Second differences as well as first: a fine texture can show only a small
    // first difference across an edge it should still not interpolate through.
    let grad_h = abs(e - w) + abs(2.0 * center - ee - ww);
    let grad_v = abs(n - s) + abs(2.0 * center - nn - ss);

    // Each direction's estimate is the one-dimensional Malvar-He-Cutler
    // correction: the average of that pair, adjusted by this photosite's own
    // curvature along the same axis.
    let green_h = 0.5 * (e + w) + 0.25 * (2.0 * center - ee - ww);
    let green_v = 0.5 * (n + s) + 0.25 * (2.0 * center - nn - ss);

    // Blended by relative gradient rather than picked outright. A hard choice
    // flips direction between neighbouring pixels wherever the two gradients
    // are close, stamping its own speckle across smooth areas.
    let total = grad_h + grad_v;
    let toward_h = select(0.5, grad_v / total, total > 1e-6);
    return bounded4(mix(green_v, green_h, toward_h), n, s, e, w);
}

/// Edge-directed demosaic for a 2x2 Bayer mosaic.
///
/// Bilinear interpolation produces coloured fringes along any sharp edge, and
/// the reason is structural rather than a matter of precision: each channel is
/// interpolated in isolation, so at an edge the three land in slightly
/// different places and that disagreement *is* a hue. A neutral black-to-white
/// edge comes out with colour on it — measured at the worst possible 255/255 on
/// a synthetic step.
///
/// Two ideas fix it, and **they only work together**:
///
/// 1. Green is interpolated along the edge rather than through it (`green_at`).
/// 2. Red and blue are interpolated as *differences* from green, never
///    directly.
///
/// The second is what makes the first pay. A colour difference stays smooth
/// where brightness does not — across a neutral edge `R - G` sits near zero on
/// both sides while R and G each leap — so interpolating the difference and
/// adding this pixel's own green carries the edge across from green intact and
/// the three channels move together. Doing only the first measured *worse*
/// than doing neither, 165 against 97, because an edge-aware green beside a
/// red and blue still averaging straight through the edge disagree by more
/// than three consistently-wrong channels do.
///
/// This was previously deferred as invisible at proxy scale, which was wrong
/// twice over: the proxy point-sampled rather than averaged, so per-pixel
/// demosaic errors survived as isolated dots, and the cost falls in the cached
/// per-photo pass rather than on any interaction.
fn demosaic_bayer(x: i32, y: i32, center: f32, c: u32) -> vec3<f32> {
    let g = green_at(x, y);

    if c == 1u {
        // On a green photosite. Red and blue each lie along one axis; which is
        // which depends on the mosaic phase, so ask the tile rather than
        // assuming RGGB.
        let along_row = 0.5
            * ((tap(x - 1, y) - green_at(x - 1, y)) + (tap(x + 1, y) - green_at(x + 1, y)));
        let along_col = 0.5
            * ((tap(x, y - 1) - green_at(x, y - 1)) + (tap(x, y + 1) - green_at(x, y + 1)));

        if as_rgb(color_index(x - 1, y)) == 0u {
            return vec3<f32>(g + along_row, g, g + along_col);
        }
        return vec3<f32>(g + along_col, g, g + along_row);
    }

    // On a red or blue photosite the opposite colour sits on the diagonals.
    let diagonal = 0.25
        * ((tap(x - 1, y - 1) - green_at(x - 1, y - 1))
            + (tap(x + 1, y - 1) - green_at(x + 1, y - 1))
            + (tap(x - 1, y + 1) - green_at(x - 1, y + 1))
            + (tap(x + 1, y + 1) - green_at(x + 1, y + 1)));

    if c == 0u {
        return vec3<f32>(center, g, g + diagonal);
    }
    return vec3<f32>(g + diagonal, g, center);
}

/// Distance-weighted average over a neighbourhood, one colour at a time.
///
/// Used for X-Trans, whose 6x6 tile has no regular axis structure to exploit.
/// Approximate and slower than a dedicated X-Trans algorithm, but it is
/// correct for any layout — and there is no Fuji sample to verify a better one
/// against yet.
/// Green at any photosite on a non-Bayer mosaic.
///
/// A radius of one is enough on X-Trans: the layout guarantees a green in every
/// row and every column, and its red and blue sites are isolated, so no pixel
/// is ever more than one step from a green.
fn green_generic(x: i32, y: i32) -> f32 {
    if as_rgb(color_index(x, y)) == 1u {
        return tap(x, y);
    }

    // Along the row and along the column, separately. X-Trans guarantees a
    // green in every row and every column, so both estimates always exist.
    var row_sum = 0.0;
    var row_weight = 0.0;
    var row_lo = 1e30;
    var row_hi = -1e30;
    var col_sum = 0.0;
    var col_weight = 0.0;
    var col_lo = 1e30;
    var col_hi = -1e30;

    for (var d = -2; d <= 2; d++) {
        if d == 0 {
            continue;
        }
        let w = 1.0 / f32(abs(d));

        if as_rgb(color_index(x + d, y)) == 1u {
            let v = tap(x + d, y);
            row_sum += v * w;
            row_weight += w;
            row_lo = min(row_lo, v);
            row_hi = max(row_hi, v);
        }
        if as_rgb(color_index(x, y + d)) == 1u {
            let v = tap(x, y + d);
            col_sum += v * w;
            col_weight += w;
            col_lo = min(col_lo, v);
            col_hi = max(col_hi, v);
        }
    }

    // **Interpolate along the edge, not across it.** The spread among the
    // greens on each axis says which direction crosses an edge: the axis whose
    // samples disagree least is the one lying along it. Averaging both
    // directions blindly is what produced a 195/255 fringe on a neutral step —
    // red and blue were already following green, so green being wrong at the
    // edge made every channel wrong together.
    let row_ok = row_weight > 1e-6;
    let col_ok = col_weight > 1e-6;
    if !row_ok {
        return col_sum / max(col_weight, 1e-6);
    }
    if !col_ok {
        return row_sum / row_weight;
    }

    let row_span = row_hi - row_lo;
    let col_span = col_hi - col_lo;
    let total = row_span + col_span;
    // Weighted towards the calmer axis rather than picked outright: a hard
    // choice flips between neighbouring pixels wherever the two are close and
    // stamps its own speckle across smooth areas.
    let toward_row = select(0.5, col_span / total, total > 1e-6);
    return mix(col_sum / col_weight, row_sum / row_weight, toward_row);
}

/// Demosaic for any mosaic that is not a 2x2 Bayer — in practice X-Trans.
///
/// Built on the same two ideas as the Bayer path, for the same reason:
/// reconstruct green first, then carry red and blue as **differences** from it.
/// Interpolating the three channels independently — which is what a plain
/// weighted average per colour does — puts them in slightly different places
/// across an edge, and that disagreement is a coloured fringe.
///
/// X-Trans suits this better than Bayer does. Twenty of its thirty-six sites
/// are green, so the channel carrying the luminance detail is densely sampled
/// and the sparse red and blue only have to supply the colour, which varies
/// slowly. That is the whole design intent of the layout.
///
/// Still not Markesteijn: this does not pick an interpolation direction per
/// pixel. Worth revisiting, but it needs a Fuji frame with known-hard content
/// to judge against rather than a guess about what looks better.
fn demosaic_generic(x: i32, y: i32) -> vec3<f32> {
    let g = green_generic(x, y);

    // Red and blue sit within two steps on every X-Trans phase, so a 5x5
    // neighbourhood always finds several of each.
    var sums = vec2<f32>(0.0);
    var weights = vec2<f32>(0.0);
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let c = as_rgb(color_index(x + dx, y + dy));
            if c == 1u {
                continue;
            }
            let w = 1.0 / (1.0 + f32(dx * dx + dy * dy));
            // The colour *difference*, which stays smooth across an edge where
            // brightness does not.
            let d = (tap(x + dx, y + dy) - green_generic(x + dx, y + dy)) * w;
            if c == 0u {
                sums.x += d;
                weights.x += w;
            } else {
                sums.y += d;
                weights.y += w;
            }
        }
    }

    let diff = sums / max(weights, vec2<f32>(1e-6));
    return vec3<f32>(g + diff.x, g, g + diff.y);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.active_size.x || gid.y >= params.active_size.y {
        return;
    }
    let x = i32(gid.x);
    let y = i32(gid.y);

    var rgb: vec3<f32>;
    if params.period == 2u {
        rgb = demosaic_bayer(x, y, tap(x, y), as_rgb(color_index(x, y)));
    } else {
        rgb = demosaic_generic(x, y);
    }

    // Blown highlights are rebuilt in the next pass (`highlights.wgsl`),
    // from the colour of the unclipped pixels around them; what this pass
    // leaves at a clipped photosite is the colour white balance made of a
    // channel stuck at its ceiling, which is the classic pink highlight.
    //
    // What it gives that pass is each pixel's colour, and how much to trust
    // it: not at all where any photosite the demosaic read was clipping —
    // a pixel at the edge of a blown area is interpolated from clipped and
    // unclipped sites alike, and its colour is neither — and only once the
    // pixel is bright. What surrounds a highlight is what it fades into, the
    // bright edge of the same thing; a darker neighbour is more often the
    // background — the blue sky beside a white cloud — and a dark pixel's
    // colour is mostly noise. **The clip ramp matches `highlights.wgsl`'s.**
    let ceiling = max(params.wb.rgb, vec3<f32>(1e-6));
    let clip = smoothstep(vec3<f32>(0.90), vec3<f32>(0.98), rgb / ceiling);
    let bright = clamp(max(rgb.r / ceiling.r, max(rgb.g / ceiling.g, rgb.b / ceiling.b)), 0.0, 1.0);
    let positive = max(rgb, vec3<f32>(0.0));
    let total = positive.r + positive.g + positive.b;
    let lit = smoothstep(0.25, 0.75, bright);
    var trust = 0.0;
    // Most of a frame is too dark to lend colour, and the neighbourhood is
    // twenty-five reads a pixel: asked only of pixels that could lend it,
    // which took what this adds to a 24 MP open from 110 ms to about 25.
    if lit > 0.0 && total > 1e-4 {
        var near = max(clip.r, max(clip.g, clip.b));
        for (var dy = -2; dy <= 2; dy++) {
            for (var dx = -2; dx <= 2; dx++) {
                let site = tap(x + dx, y + dy) / max(params.wb[color_index(x + dx, y + dy)], 1e-6);
                near = max(near, smoothstep(0.90, 0.98, site));
            }
        }
        trust = (1.0 - near) * lit * lit;
    }
    let colour = positive / max(total, 1e-4);
    textureStore(out_chroma, vec2<i32>(x, y), vec4<f32>(colour * trust, trust));

    // Deliberately unclamped above 1.0: white balance pushes the
    // non-green channels past full scale, and clipping here would destroy
    // headroom the tone curve's shoulder still needs. f16 has the range.
    textureStore(out_linear, vec2<i32>(x, y), vec4<f32>(rgb, 1.0));
}
