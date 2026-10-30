//! AI noise reduction: NAFNet over the linear image, blended back by
//! an Amount.
//!
//! The spikes (`docs/AI-MODEL-SPIKES.md`) settled the shape of it:
//!
//! - **NAFNet-SIDD-width64** (MIT), Qualcomm AI Hub's ONNX export converted
//!   to fp16 (published as a release of W4x1e/models). It
//!   clears the colour noise and blotches the classic noise reduction leaves,
//!   and keeps edges. It replaced SCUNet (Apache-2.0), which looked the same
//!   at half resolution and was 3–6× slower on every kind of hardware
//!   measured: a 256 px tile took 12.5 ms against 73 ms on an RTX 5070 Ti,
//!   855 ms against 3.5 s on integrated Radeon graphics, and 0.6 s against
//!   1.6 s on an 8-core CPU. Convolutions, which DirectML runs well, against
//!   memory-bound attention. fp16 halved the download (234 MB) and made
//!   it faster again on graphics hardware: 7.1 against 13.3 ms per tile on
//!   the RTX, 625 against 841 ms on the integrated Radeon. On a CPU alone it
//!   is slower (1.6 against 0.6 s per tile), which only a PC with no DirectX 12
//!   graphics at all ever falls back to.
//! - **Run on the linear image, before anything else.** Noise is removed
//!   before sharpening, clarity and texture can amplify it, as other raw
//!   developers' denoise does. The model was trained on display-referred photos,
//!   so the linear image is encoded with the sRGB curve on the way in and
//!   decoded on the way out, scaled so its brightest real values fit.
//! - **Tiled at a fixed 256 px**, the size the export is fixed at (and the
//!   size DirectML needs: it crashed on SCUNet's free dimensions); a 32 px
//!   tent window blends the overlaps without seams.
//! - **At half resolution, for colour and the coarse band only**
//!   ([`denoise_half_with`]), which is a quarter of the pixels. The half-size
//!   result supplies all of the colour and the luminance coarser than two
//!   pixels; the finest luminance band stays the original's. Side by side at
//!   1:1 on ISO 6400 frames that was cleaner in colour than a full-resolution
//!   run blended at 60%, with no bleeding at edges, and kept a fine, even
//!   grain the Luminance noise slider can take down. It matters more for
//!   NAFNet than for SCUNet: at full resolution NAFNet, trained on phone
//!   sensors (SIDD), paints colour patches into stone and hair. About 2 s for
//!   24 MP on an RTX 5070 Ti, 1–2 min on weak integrated graphics or a CPU.
//!
//! - **Where you are looking first, shown as it goes** ([`Steer`]). Tiles
//!   run nearest the centre of the view first, and a long run shows its result
//!   so far every few seconds (the rest still the original), so on a slow
//!   machine the part being judged at 1:1 is done in seconds, not minutes.
//!
//! The pure parts (tiling, the order, the transfer, resampling, the blend) are
//! here and tested; the session sits behind `--features ai`.

/// The fixed tile edge the model runs at.
pub const TILE: u32 = 256;
/// How much neighbouring tiles overlap, blended with a tent window.
pub const OVERLAP: u32 = 32;
/// The Amount a first run applies: all of it. The half-resolution method
/// already keeps the fine grain and texture, and was judged at full strength;
/// at the 60% a full-resolution run needed, 40% of the colour speckle came
/// back. The Amount is there to take it down.
pub const DEFAULT_AMOUNT: f32 = 1.0;

/// Where tiles start along one edge of `len` pixels: every `TILE - OVERLAP`,
/// with the last flush to the far edge so nothing is missed.
pub fn tile_origins(len: u32) -> Vec<u32> {
    if len <= TILE {
        return vec![0];
    }
    let step = TILE - OVERLAP;
    let mut out: Vec<u32> = (0..len - TILE).step_by(step as usize).collect();
    out.push(len - TILE);
    out.dedup();
    out
}

/// The blending weight at position `i` along a tile: ramps up over the
/// overlap at each end, so overlapping tiles fade into one another.
pub fn tent(i: u32) -> f32 {
    let from_edge = (i + 1).min(TILE - i) as f32;
    (from_edge / OVERLAP as f32).min(1.0)
}

/// A scale that brings the linear image's bright end to 1 before encoding:
/// the 99.9th percentile of its channel values, never below 1. A clipped
/// sensor channel or a white-balance multiplier can take linear values past
/// 1, and clamping them would flatten every highlight.
pub fn scale_for(rgba: &[f32]) -> f32 {
    let mut sample: Vec<f32> = rgba
        .chunks_exact(4)
        .step_by(97)
        .flat_map(|p| [p[0], p[1], p[2]])
        .filter(|v| v.is_finite())
        .collect();
    if sample.is_empty() {
        return 1.0;
    }
    let k = ((sample.len() - 1) as f32 * 0.999) as usize;
    let (_, v, _) = sample.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    v.max(1.0)
}

/// Linear to the model's display-referred input.
pub fn encode(linear: f32, scale: f32) -> f32 {
    let v = (linear / scale).clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// The model's output back to linear.
pub fn decode(encoded: f32, scale: f32) -> f32 {
    crate::raster::srgb_to_linear(encoded.clamp(0.0, 1.0)) * scale
}

/// `original` and `denoised` mixed by `amount` (0 keeps the original), as
/// RGBA half floats, alpha from the original. In parallel: it runs whenever
/// the Amount is committed, over 96 million values for a 24 MP photo.
pub fn blend(original: &[u16], denoised: &[u16], amount: f32) -> Vec<u16> {
    use half::f16;
    use rayon::prelude::*;
    let a = amount.clamp(0.0, 1.0);
    if a <= 0.0 {
        return original.to_vec();
    }
    let mut out = original.to_vec();
    out.par_chunks_mut(4)
        .zip(denoised.par_chunks(4))
        .for_each(|(o, d)| {
            for c in 0..3 {
                let (ov, dv) = (f16::from_bits(o[c]).to_f32(), f16::from_bits(d[c]).to_f32());
                o[c] = f16::from_f32(ov + (dv - ov) * a).to_bits();
            }
        });
    out
}

/// Half floats to `f32`s.
pub fn to_f32(rgba: &[u16]) -> Vec<f32> {
    rgba.iter()
        .map(|&b| half::f16::from_bits(b).to_f32())
        .collect()
}

/// `f32`s to half floats.
pub fn to_f16(rgba: &[f32]) -> Vec<u16> {
    rgba.iter()
        .map(|&v| half::f16::from_f32(v).to_bits())
        .collect()
}

/// How a run is steered from the viewer: which part of the photo to do first,
/// and a look at the result so far.
#[derive(Default)]
pub struct Steer<'a> {
    /// Where to start, as fractions of the image's width and height: the
    /// tiles nearest it run first. The centre when `None`.
    pub focus: Option<(f32, f32)>,
    /// Shown the whole image so far, denoised where the model has been and the
    /// original elsewhere, every few seconds of a long run ([`preview_due`]).
    /// Called on a thread of its own, so the model does not wait for it.
    pub preview: Option<&'a mut (dyn FnMut(Vec<f32>) + Send)>,
}

/// The tiles of a `width` × `height` image, as their top-left corners,
/// nearest `focus` (in pixels) first.
pub fn tile_order(width: u32, height: u32, focus: (f32, f32)) -> Vec<(u32, u32)> {
    let half = TILE as f32 / 2.0;
    let xs = tile_origins(width);
    let mut tiles: Vec<(u32, u32)> = tile_origins(height)
        .into_iter()
        .flat_map(|y| xs.iter().map(move |&x| (x, y)))
        .collect();
    let distance = |&(x, y): &(u32, u32)| {
        let (dx, dy) = (x as f32 + half - focus.0, y as f32 + half - focus.1);
        dx * dx + dy * dy
    };
    tiles.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    tiles
}

/// Whether to show the result so far, `elapsed` seconds into a run with
/// `done` of `total` tiles finished and `since_last` seconds after the last
/// look. Never for a run that will be over in a few seconds anyway: a look
/// costs a fraction of a second, which a fast machine would only feel as
/// slowness. Otherwise every few seconds, and more rarely the longer the run.
pub fn preview_due(elapsed: f32, since_last: f32, done: usize, total: usize) -> bool {
    if done == 0 || done >= total {
        return false;
    }
    let expected = elapsed / done as f32 * total as f32;
    expected >= 8.0 && since_last >= (expected / 12.0).max(2.5)
}

/// The image put back together from the tiles' weighted sums, with the
/// original wherever no tile has reached yet.
fn assemble(rgba: &[f32], acc: &[f32], weight: &[f32], scale: f32) -> Vec<f32> {
    use rayon::prelude::*;
    let mut result = rgba.to_vec();
    result
        .par_chunks_mut(4)
        .zip(acc.par_chunks(3))
        .zip(weight.par_iter())
        .for_each(|((px, sum), &wgt)| {
            if wgt > 0.0 {
                for c in 0..3 {
                    px[c] = decode(sum[c] / wgt, scale);
                }
            }
        });
    result
}

/// Denoises a linear RGBA image by running `model` over it tile by tile.
///
/// `model` takes one tile, planar RGB in 0..1 (`3 * TILE * TILE` values), and
/// returns the same shape denoised. Split out from the session so the tiling,
/// padding and blending are tested with a stand-in model.
pub fn denoise_with(
    rgba: &[f32],
    width: u32,
    height: u32,
    model: impl FnMut(&[f32]) -> anyhow::Result<Vec<f32>>,
    progress: impl FnMut(usize, usize),
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Vec<f32>> {
    denoise_steered(
        rgba,
        width,
        height,
        model,
        progress,
        cancelled,
        Steer::default(),
    )
}

/// [`denoise_with`], steered: tiles nearest the focus first, and a look at the
/// result so far now and then.
pub fn denoise_steered(
    rgba: &[f32],
    width: u32,
    height: u32,
    mut model: impl FnMut(&[f32]) -> anyhow::Result<Vec<f32>>,
    mut progress: impl FnMut(usize, usize),
    cancelled: impl Fn() -> bool,
    steer: Steer,
) -> anyhow::Result<Vec<f32>> {
    let (w, h) = (width as usize, height as usize);
    anyhow::ensure!(
        rgba.len() == w * h * 4,
        "image is {} values, expected {}",
        rgba.len(),
        w * h * 4
    );
    let scale = scale_for(rgba);
    // Encoded once, in parallel, rather than per tile (with every overlap
    // encoded twice or four times) on one thread.
    let encoded: Vec<f32> = {
        use rayon::prelude::*;
        rgba.par_chunks(4)
            .flat_map_iter(|p| {
                [
                    encode(p[0], scale),
                    encode(p[1], scale),
                    encode(p[2], scale),
                ]
            })
            .collect()
    };
    let focus = steer
        .focus
        .map(|(fx, fy)| (fx * width as f32, fy * height as f32))
        .unwrap_or((width as f32 / 2.0, height as f32 / 2.0));
    let tiles = tile_order(width, height, focus);
    let total = tiles.len();
    let t = TILE as usize;
    let plane = t * t;
    let mut acc = vec![0.0f32; w * h * 3];
    let mut weight = vec![0.0f32; w * h];
    let mut done = 0;
    let started = std::time::Instant::now();
    let mut last_look = started;
    let look = std::sync::Mutex::new(steer.preview);
    let wants_looks = crate::sync::lock_recover(&look).is_some();
    std::thread::scope(|scope| -> anyhow::Result<()> {
        // A look is put together and shown on a thread of its own: at 24 MP
        // that is over a second of resampling, blending and upload, and done
        // here it held the model up for every one (a 10 s run took 15). One
        // at a time; the scope waits for the last, so none can arrive after
        // the result or after Cancel has put the original back.
        let mut showing: Option<std::thread::ScopedJoinHandle<'_, ()>> = None;
        for &(x0, y0) in &tiles {
            anyhow::ensure!(!cancelled(), "cancelled");
            // Read the tile, clamping at the edges for an image smaller than
            // a tile (edge pixels repeat rather than padding with black).
            let mut input = vec![0.0f32; 3 * plane];
            for ty in 0..t {
                let sy = (y0 as usize + ty).min(h - 1);
                for tx in 0..t {
                    let sx = (x0 as usize + tx).min(w - 1);
                    let p = (sy * w + sx) * 3;
                    for c in 0..3 {
                        input[c * plane + ty * t + tx] = encoded[p + c];
                    }
                }
            }
            let out = model(&input)?;
            anyhow::ensure!(
                out.len() == 3 * plane,
                "the model returned {} values",
                out.len()
            );
            for ty in 0..t {
                let sy = y0 as usize + ty;
                if sy >= h {
                    break;
                }
                for tx in 0..t {
                    let sx = x0 as usize + tx;
                    if sx >= w {
                        break;
                    }
                    let wgt = tent(tx as u32) * tent(ty as u32);
                    let i = sy * w + sx;
                    weight[i] += wgt;
                    for c in 0..3 {
                        acc[i * 3 + c] += out[c * plane + ty * t + tx] * wgt;
                    }
                }
            }
            done += 1;
            progress(done, total);
            let now = std::time::Instant::now();
            let idle = showing.as_ref().is_none_or(|h| h.is_finished());
            if wants_looks
                && idle
                && preview_due(
                    (now - started).as_secs_f32(),
                    (now - last_look).as_secs_f32(),
                    done,
                    total,
                )
            {
                let (acc_now, weight_now) = (acc.clone(), weight.clone());
                let look = &look;
                showing = Some(scope.spawn(move || {
                    let part = assemble(rgba, &acc_now, &weight_now, scale);
                    if let Some(show) = crate::sync::lock_recover(look).as_mut() {
                        (*show)(part);
                    }
                }));
                last_look = now;
            }
        }
        Ok(())
    })?;
    // Every pixel has a weight by now: the tiles cover the image.
    Ok(assemble(rgba, &acc, &weight, scale))
}

/// A 2×2 box average, the last row or column dropped from an odd size.
pub fn half_size(rgba: &[f32], width: u32, height: u32) -> (Vec<f32>, u32, u32) {
    half_size_by(|i| rgba[i], width, height)
}

/// [`half_size`] of half floats, read as it goes: converting the whole
/// photo to `f32`s first held 384 MB more at 24 MP for the length of a run.
pub fn half_size_f16(rgba: &[u16], width: u32, height: u32) -> (Vec<f32>, u32, u32) {
    half_size_by(|i| half::f16::from_bits(rgba[i]).to_f32(), width, height)
}

fn half_size_by(
    value: impl Fn(usize) -> f32 + Sync,
    width: u32,
    height: u32,
) -> (Vec<f32>, u32, u32) {
    use rayon::prelude::*;
    let (w, hw, hh) = (
        width as usize,
        (width / 2).max(1) as usize,
        (height / 2).max(1) as usize,
    );
    let (w1, h1) = (
        (width as usize).saturating_sub(1),
        (height as usize).saturating_sub(1),
    );
    let mut out = vec![0.0f32; hw * hh * 4];
    out.par_chunks_mut(hw * 4).enumerate().for_each(|(y, row)| {
        for x in 0..hw {
            for c in 0..4 {
                let p = |dx: usize, dy: usize| {
                    value(((2 * y + dy).min(h1) * w + (2 * x + dx).min(w1)) * 4 + c)
                };
                row[x * 4 + c] = (p(0, 0) + p(1, 0) + p(0, 1) + p(1, 1)) * 0.25;
            }
        }
    });
    (out, hw as u32, hh as u32)
}

/// Bilinear from a [`half_size`] image back to `width` × `height`, sampling
/// at pixel centres so nothing shifts.
pub fn upsample(half: &[f32], hw: u32, hh: u32, width: u32, height: u32) -> Vec<f32> {
    use rayon::prelude::*;
    let (hw, hh, w) = (hw as usize, hh as usize, width as usize);
    let mut out = vec![0.0f32; w * height as usize * 4];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hh - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(hh - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(hw - 1);
            for c in 0..4 {
                let s = |xx: usize, yy: usize| half[(yy * hw + xx) * 4 + c];
                let top = s(x0, y0) + (s(x1, y0) - s(x0, y0)) * tx;
                let bottom = s(x0, y1) + (s(x1, y1) - s(x0, y1)) * tx;
                row[x * 4 + c] = top + (bottom - top) * ty;
            }
        }
    });
    out
}

/// Puts the full-size photo back together: colour and coarse luminance from
/// the denoised half-size result, the finest luminance band from the
/// original (what the half-size copy could not hold). All three are the same
/// size; `low` is the original taken to half size and back.
pub fn recombine(original: &[f32], low: &[f32], denoised: &[f32]) -> Vec<f32> {
    use rayon::prelude::*;
    let luma = |p: &[f32]| (p[0] + p[1] + p[2]) / 3.0;
    original
        .par_chunks(4)
        .zip(low.par_chunks(4))
        .zip(denoised.par_chunks(4))
        .flat_map_iter(|((o, lo), d)| {
            // Added to all three channels, it changes luminance and not colour.
            let fine = luma(o) - luma(lo);
            [d[0] + fine, d[1] + fine, d[2] + fine, o[3]]
        })
        .collect()
}

/// [`recombine`] of [`upsample`]d half-size images, in one pass: the
/// original taken down and back up (`low`), and the denoised result brought
/// up, are worked out pixel by pixel instead of held as two more full-size
/// copies, and the result is written once, as `T`. Exactly what the three
/// steps gave.
fn recombine_half<T: Copy + Default + Send>(
    original: impl Fn(usize) -> f32 + Sync,
    small: &[f32],
    denoised: &[f32],
    (hw, hh): (u32, u32),
    (width, height): (u32, u32),
    store: impl Fn(f32) -> T + Sync,
) -> Vec<T> {
    use rayon::prelude::*;
    let (hw, hh, w) = (hw as usize, hh as usize, width as usize);
    let mut out = vec![T::default(); w * height as usize * 4];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hh - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(hh - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(hw - 1);
            // As `upsample` samples, channel by channel.
            let up = |half: &[f32], c: usize| {
                let s = |xx: usize, yy: usize| half[(yy * hw + xx) * 4 + c];
                let top = s(x0, y0) + (s(x1, y0) - s(x0, y0)) * tx;
                let bottom = s(x0, y1) + (s(x1, y1) - s(x0, y1)) * tx;
                top + (bottom - top) * ty
            };
            let at = (y * w + x) * 4;
            let o = [
                original(at),
                original(at + 1),
                original(at + 2),
                original(at + 3),
            ];
            let lo = [up(small, 0), up(small, 1), up(small, 2)];
            let d = [up(denoised, 0), up(denoised, 1), up(denoised, 2)];
            // As `recombine`: the finest luminance band from the original.
            let fine = (o[0] + o[1] + o[2]) / 3.0 - (lo[0] + lo[1] + lo[2]) / 3.0;
            let px = &mut row[x * 4..x * 4 + 4];
            px[0] = store(d[0] + fine);
            px[1] = store(d[1] + fine);
            px[2] = store(d[2] + fine);
            px[3] = store(o[3]);
        }
    });
    out
}

/// [`denoise_with`] on a half-size copy, recombined at full size (see the
/// module notes for why). A photo too small to halve is denoised as it is.
pub fn denoise_half_with(
    rgba: &[f32],
    width: u32,
    height: u32,
    model: impl FnMut(&[f32]) -> anyhow::Result<Vec<f32>>,
    progress: impl FnMut(usize, usize),
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Vec<f32>> {
    denoise_half_steered(
        rgba,
        width,
        height,
        model,
        progress,
        cancelled,
        Steer::default(),
    )
}

/// [`denoise_half_with`], steered. The focus is a fraction of the image, so
/// it means the same at half size; a look is recombined at full size.
pub fn denoise_half_steered(
    rgba: &[f32],
    width: u32,
    height: u32,
    model: impl FnMut(&[f32]) -> anyhow::Result<Vec<f32>>,
    progress: impl FnMut(usize, usize),
    cancelled: impl Fn() -> bool,
    steer: Steer,
) -> anyhow::Result<Vec<f32>> {
    if width < 2 * TILE || height < 2 * TILE {
        return denoise_steered(rgba, width, height, model, progress, cancelled, steer);
    }
    let (small, hw, hh) = half_size(rgba, width, height);
    let Steer { focus, mut preview } = steer;
    let wants_looks = preview.is_some();
    let mut look_full = |part: Vec<f32>| {
        if let Some(look) = preview.as_mut() {
            look(recombine_half(
                |i| rgba[i],
                &small,
                &part,
                (hw, hh),
                (width, height),
                |v| v,
            ));
        }
    };
    let small_denoised = denoise_steered(
        &small,
        hw,
        hh,
        model,
        progress,
        cancelled,
        Steer {
            focus,
            preview: if wants_looks {
                Some(&mut look_full)
            } else {
                None
            },
        },
    )?;
    Ok(recombine_half(
        |i| rgba[i],
        &small,
        &small_denoised,
        (hw, hh),
        (width, height),
        |v| v,
    ))
}

/// [`denoise_half_steered`] of half floats to half floats, with no full-size
/// `f32` copy held at any point. By the buffers' sizes, about 0.6 GB at
/// its peak for a 24 MP photo, where converting, halving, bringing back up and
/// recombining as separate full-size steps held about 2 GB. A look is still a full-size
/// `f32` image, made in one pass.
pub fn denoise_half_steered_f16(
    rgba: &[u16],
    width: u32,
    height: u32,
    model: impl FnMut(&[f32]) -> anyhow::Result<Vec<f32>>,
    progress: impl FnMut(usize, usize),
    cancelled: impl Fn() -> bool,
    steer: Steer,
) -> anyhow::Result<Vec<u16>> {
    let f16_at = |i: usize| half::f16::from_bits(rgba[i]).to_f32();
    if width < 2 * TILE || height < 2 * TILE {
        let out = denoise_steered(
            &to_f32(rgba),
            width,
            height,
            model,
            progress,
            cancelled,
            steer,
        )?;
        return Ok(to_f16(&out));
    }
    let (small, hw, hh) = half_size_f16(rgba, width, height);
    let Steer { focus, mut preview } = steer;
    let wants_looks = preview.is_some();
    let mut look_full = |part: Vec<f32>| {
        if let Some(look) = preview.as_mut() {
            look(recombine_half(
                f16_at,
                &small,
                &part,
                (hw, hh),
                (width, height),
                |v| v,
            ));
        }
    };
    let small_denoised = denoise_steered(
        &small,
        hw,
        hh,
        model,
        progress,
        cancelled,
        Steer {
            focus,
            preview: if wants_looks {
                Some(&mut look_full)
            } else {
                None
            },
        },
    )?;
    Ok(recombine_half(
        f16_at,
        &small,
        &small_denoised,
        (hw, hh),
        (width, height),
        |v| half::f16::from_f32(v).to_bits(),
    ))
}

#[cfg(feature = "ai")]
pub use imp::DenoiseEngine;

#[cfg(feature = "ai")]
mod imp {
    use super::*;
    use anyhow::{Context, Result};
    use std::path::Path;

    /// The model file, under `<models>/denoise`.
    pub const MODEL_FILE: &str = "nafnet_denoise_fp16.onnx";

    /// NAFNet, loaded once.
    pub struct DenoiseEngine {
        session: std::sync::Mutex<ort::session::Session>,
    }

    impl DenoiseEngine {
        /// Loads the model from the models folder's `denoise` directory. Its
        /// input is fixed at one 256 px tile; memory patterns are off, as
        /// DirectML requires.
        pub fn load(models_dir: &Path) -> Result<Self> {
            crate::develop::ai::ensure_ort_dylib();
            let path = models_dir.join("denoise").join(MODEL_FILE);
            anyhow::ensure!(
                path.exists(),
                "AI noise reduction isn't downloaded yet: get it in Settings → On-device models"
            );
            let session = crate::develop::ai::commit_on_gpu_or_cpu_with(
                &path,
                "denoise",
                crate::develop::ai::Workload::Interactive,
                |b| b.with_memory_pattern(false),
            )
            .with_context(|| format!("loading {}", path.display()))?;
            Ok(Self {
                session: std::sync::Mutex::new(session),
            })
        }

        /// Denoises a linear RGBA half-float image; see [`denoise_half_with`].
        pub fn denoise(
            &self,
            rgba: &[u16],
            width: u32,
            height: u32,
            progress: impl FnMut(usize, usize),
            cancelled: impl Fn() -> bool,
        ) -> Result<Vec<u16>> {
            self.denoise_steered(rgba, width, height, Steer::default(), progress, cancelled)
        }

        /// [`Self::denoise`], steered from the viewer ([`Steer`]).
        pub fn denoise_steered(
            &self,
            rgba: &[u16],
            width: u32,
            height: u32,
            steer: Steer,
            progress: impl FnMut(usize, usize),
            cancelled: impl Fn() -> bool,
        ) -> Result<Vec<u16>> {
            let mut sess = crate::sync::lock_recover(&self.session);
            let name = sess
                .inputs
                .first()
                .map(|i| i.name.clone())
                .unwrap_or_else(|| "image".to_string());
            let t = TILE as i64;
            denoise_half_steered_f16(
                rgba,
                width,
                height,
                |tile| {
                    let tensor = ort::value::Value::from_array(([1_i64, 3, t, t], tile.to_vec()))?;
                    let outputs = sess.run(ort::inputs![name.as_str() => tensor])?;
                    let (_shape, data) = outputs[0].try_extract_tensor::<f32>()?;
                    Ok(data.to_vec())
                },
                progress,
                cancelled,
                steer,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test image: smooth, with some fine detail and noise in it.
    fn sample_image(w: u32, h: u32) -> Vec<f32> {
        let mut seed = 7u32;
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = (seed >> 8) as f32 / (1u32 << 24) as f32 * 0.05;
                let base = 0.3 + 0.2 * (x / 37.0).sin() * (y / 23.0).cos();
                [base + n, base * 0.8 + n, base * 0.6 + n, 1.0]
            })
            .collect()
    }

    /// The fused finish gives exactly what the three full-size steps did,
    /// so the memory saving changes no pixel.
    #[test]
    fn the_one_pass_finish_matches_the_three_steps() {
        let (w, h) = (101, 67);
        let rgba = sample_image(w, h);
        let (small, hw, hh) = half_size(&rgba, w, h);
        let denoised: Vec<f32> = small.iter().map(|v| v * 0.9 + 0.01).collect();
        let three = recombine(
            &rgba,
            &upsample(&small, hw, hh, w, h),
            &upsample(&denoised, hw, hh, w, h),
        );
        let one = recombine_half(|i| rgba[i], &small, &denoised, (hw, hh), (w, h), |v| v);
        assert_eq!(one, three);
    }

    /// Half floats in and out give what the `f32` path gives, to half-float
    /// precision.
    #[test]
    fn the_half_float_path_matches_the_f32_one() {
        let (w, h) = (2 * TILE + 13, 2 * TILE + 5);
        let rgba = to_f16(&sample_image(w, h));
        let model = |tile: &[f32]| Ok(tile.iter().map(|v| v * 0.95).collect());
        let wide = denoise_half_with(&to_f32(&rgba), w, h, model, |_, _| {}, || false).unwrap();
        let half =
            denoise_half_steered_f16(&rgba, w, h, model, |_, _| {}, || false, Steer::default())
                .unwrap();
        assert_eq!(half, to_f16(&wide));
    }

    #[test]
    fn tiles_cover_the_edge_and_overlap() {
        assert_eq!(tile_origins(100), vec![0], "one tile for a small image");
        assert_eq!(tile_origins(256), vec![0]);
        let xs = tile_origins(1000);
        assert_eq!(xs[0], 0);
        assert_eq!(*xs.last().unwrap(), 1000 - TILE, "the last tile is flush");
        assert!(xs.windows(2).all(|w| w[1] - w[0] <= TILE - OVERLAP));
    }

    #[test]
    fn the_tent_ramps_over_the_overlap_only() {
        assert!(tent(0) < 0.05);
        assert_eq!(tent(OVERLAP), 1.0);
        assert_eq!(tent(TILE / 2), 1.0);
        assert!(tent(TILE - 1) < 0.05);
    }

    #[test]
    fn encoding_round_trips_and_keeps_highlights() {
        for v in [0.0f32, 0.001, 0.18, 0.5, 1.0] {
            assert!((decode(encode(v, 1.0), 1.0) - v).abs() < 1e-4, "{v}");
        }
        // A value past 1 survives when the scale makes room for it.
        assert!((decode(encode(1.8, 2.0), 2.0) - 1.8).abs() < 1e-3);
        let mut img = vec![0.2f32; 4 * 1000];
        img[4 * 500] = 3.0;
        assert_eq!(scale_for(&img), 1.0, "one hot pixel does not set the scale");
    }

    #[test]
    fn blending_by_amount() {
        let o = to_f16(&[0.2, 0.4, 0.6, 1.0]);
        let d = to_f16(&[0.4, 0.4, 0.2, 0.0]);
        assert_eq!(blend(&o, &d, 0.0), o, "none is the original");
        let half = to_f32(&blend(&o, &d, 0.5));
        assert!((half[0] - 0.3).abs() < 1e-3 && (half[2] - 0.4).abs() < 1e-3);
        assert_eq!(half[3], 1.0, "alpha from the original");
    }

    /// With a model that changes nothing, the tiled result is the image back:
    /// the tiling, the transfer and the blend add nothing of their own.
    #[test]
    fn an_identity_model_returns_the_image() {
        let (w, h) = (300u32, 170u32);
        let img: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = (i % 97) as f32 / 97.0;
                [v, v * 0.5, 1.0 - v, 1.0]
            })
            .collect();
        let mut tiles = 0;
        let got = denoise_with(&img, w, h, |t| Ok(t.to_vec()), |_, n| tiles = n, || false).unwrap();
        assert_eq!(tiles, tile_origins(w).len() * tile_origins(h).len());
        let worst = img
            .iter()
            .zip(&got)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "worst difference {worst}");
    }

    /// Halving and scaling back leave a smooth image where it was: the
    /// resampling adds no shift or tint of its own, odd sizes included.
    #[test]
    fn resampling_round_trips_a_smooth_image() {
        for (w, h) in [(40u32, 30u32), (41, 31)] {
            let img: Vec<f32> = (0..w * h)
                .flat_map(|i| {
                    let (x, y) = ((i % w) as f32, (i / w) as f32);
                    [0.2 + 0.01 * x, 0.5 - 0.005 * y, 0.3, 1.0]
                })
                .collect();
            let (small, hw, hh) = half_size(&img, w, h);
            assert_eq!((hw, hh), (w / 2, h / 2));
            let back = upsample(&small, hw, hh, w, h);
            assert_eq!(back.len(), img.len());
            // Away from the edges, where bilinear clamps.
            for y in 2..h - 2 {
                for x in 2..w - 3 {
                    let i = ((y * w + x) * 4) as usize;
                    for c in 0..3 {
                        assert!((back[i + c] - img[i + c]).abs() < 2e-3, "({x},{y}) {c}");
                    }
                }
            }
        }
    }

    /// With a model that changes nothing, the half-resolution path keeps the
    /// photo's luminance exactly and its colour smoothed only by the resampling:
    /// the recombination itself invents nothing.
    #[test]
    fn the_half_resolution_path_keeps_luminance() {
        let (w, h) = (600u32, 540u32);
        let img: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                // A fine luminance checker over a smooth colour gradient.
                let fine = if (x + y) % 2 == 0 { 0.05 } else { -0.05 };
                let g = x as f32 / w as f32;
                [0.3 + g * 0.2 + fine, 0.4 + fine, 0.5 - g * 0.2 + fine, 1.0]
            })
            .collect();
        let got = denoise_half_with(&img, w, h, |t| Ok(t.to_vec()), |_, _| {}, || false).unwrap();
        let luma = |p: &[f32]| (p[0] + p[1] + p[2]) / 3.0;
        let mut worst_luma = 0.0f32;
        let mut worst_colour = 0.0f32;
        for y in 4..h - 4 {
            for x in 4..w - 4 {
                let i = ((y * w + x) * 4) as usize;
                let (a, b) = (&img[i..i + 4], &got[i..i + 4]);
                worst_luma = worst_luma.max((luma(a) - luma(b)).abs());
                for c in 0..3 {
                    worst_colour = worst_colour.max(((a[c] - luma(a)) - (b[c] - luma(b))).abs());
                }
            }
        }
        assert!(worst_luma < 2e-3, "luminance moved {worst_luma}");
        assert!(
            worst_colour < 2e-3,
            "a smooth colour gradient moved {worst_colour}"
        );
    }

    /// Only a quarter of the pixels go through the model.
    #[test]
    fn the_model_sees_a_quarter_of_the_pixels() {
        let (w, h) = (1200u32, 800u32);
        let img = vec![0.5f32; (w * h * 4) as usize];
        let (mut full, mut half) = (0, 0);
        denoise_with(&img, w, h, |t| Ok(t.to_vec()), |_, n| full = n, || false).unwrap();
        denoise_half_with(&img, w, h, |t| Ok(t.to_vec()), |_, n| half = n, || false).unwrap();
        assert_eq!(full, tile_origins(w).len() * tile_origins(h).len());
        assert_eq!(half, tile_origins(w / 2).len() * tile_origins(h / 2).len());
        assert!(half * 3 < full, "{half} tiles against {full}");
    }

    /// The tile under the focus runs first, and every tile runs once.
    #[test]
    fn tiles_run_nearest_the_focus_first() {
        let (w, h) = (1500u32, 1000u32);
        let order = tile_order(w, h, (1400.0, 900.0));
        let first = order[0];
        assert!(
            first.0 <= 1400 && 1400 < first.0 + TILE && first.1 <= 900 && 900 < first.1 + TILE,
            "{first:?}"
        );
        let mut all = order.clone();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), order.len());
        assert_eq!(order.len(), tile_origins(w).len() * tile_origins(h).len());
        // The far corner comes last.
        assert_eq!(order.last(), Some(&(0, 0)));
    }

    #[test]
    fn looks_are_for_long_runs_only() {
        // 10 tiles of 100 in 1 s: a 10 s run, looked at every 2.5 s.
        assert!(!preview_due(1.0, 1.0, 10, 100));
        assert!(preview_due(2.6, 2.6, 26, 100));
        // A 5 s run is never interrupted.
        assert!(!preview_due(3.0, 3.0, 60, 100));
        // A 20-minute run looks every 100 s, not every 2.5 s.
        assert!(!preview_due(60.0, 30.0, 5, 100));
        assert!(preview_due(120.0, 101.0, 10, 100));
        assert!(!preview_due(5.0, 5.0, 0, 100));
        assert!(!preview_due(5.0, 5.0, 100, 100));
    }

    /// A look shows the model's result where a tile has run and the original
    /// where none has yet.
    #[test]
    fn a_look_is_the_result_so_far() {
        let img = vec![0.2f32, 0.3, 0.4, 1.0, 0.2, 0.3, 0.4, 1.0];
        // Pixel 0 reached (encoded 0.5 summed at weight 2), pixel 1 not.
        let acc = vec![1.0f32, 1.0, 1.0, 0.0, 0.0, 0.0];
        let weight = vec![2.0f32, 0.0];
        let part = assemble(&img, &acc, &weight, 1.0);
        assert!(
            (part[0] - decode(0.5, 1.0)).abs() < 1e-6,
            "reached: the model's value"
        );
        assert_eq!(&part[4..8], &img[4..8], "not reached: the original");
        assert_eq!(part[3], 1.0, "alpha kept");
    }

    /// Steering changes the order, not the result.
    #[test]
    fn steering_changes_nothing_about_the_result() {
        let (w, h) = (1200u32, 900u32);
        let img: Vec<f32> = (0..w * h)
            .flat_map(|i| [(i % 13) as f32 / 13.0, 0.4, 0.6, 1.0])
            .collect();
        let dark = |t: &[f32]| Ok(t.iter().map(|v| v * 0.5).collect());
        let mut look = |_: Vec<f32>| {};
        let steered = denoise_half_steered(
            &img,
            w,
            h,
            dark,
            |_, _| {},
            || false,
            Steer {
                focus: Some((0.9, 0.1)),
                preview: Some(&mut look),
            },
        )
        .unwrap();
        let plain = denoise_half_with(&img, w, h, dark, |_, _| {}, || false).unwrap();
        let worst = steered
            .iter()
            .zip(&plain)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        // Overlaps add up in another order: float rounding only.
        assert!(worst < 1e-5, "{worst}");
    }

    #[test]
    fn cancelling_stops_it() {
        let img = vec![0.5f32; 4 * 300 * 300];
        assert!(denoise_with(&img, 300, 300, |t| Ok(t.to_vec()), |_, _| {}, || true).is_err());
    }
}
