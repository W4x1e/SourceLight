//! The white-balance eyedropper: the balance that turns a picked point grey.

use anyhow::{Context, Result};

use crate::develop::EditStack;
use crate::gpu::pipeline::{DevelopSession, Pipeline};
use crate::gpu::Gpu;

/// The balance that turns the point at framed-image UV (`x`, `y`) grey.
///
/// Measured on a small render that keeps the stack's framing (crop, rotation,
/// lens correction, so the point lands where it was clicked) and leaves out
/// everything creative: the eyedropper balances the scene, not a split-tone
/// or a LUT laid over it.
///
/// **Its own exposure, not the photo's.** Balance and the colour matrix are
/// linear and the tone curve keeps greys grey, so whether the point is neutral
/// does not depend on exposure; but its *measurement* does, since on the
/// curve's shoulder warmth and tint look alike. So the probe first brings the
/// point to a mid-tone. A bright white wall is then as good a pick as a grey
/// card, which is what a user expects.
pub fn neutral_point(
    gpu: &Gpu,
    pipeline: &Pipeline,
    session: &DevelopSession,
    stack: &EditStack,
    x: f32,
    y: f32,
) -> Result<crate::develop::auto::AutoWb> {
    let mut probe = EditStack {
        // The edit's version, so the sliders being solved for mean what they
        // will mean on it.
        version: stack.version,
        geometry: stack.geometry,
        lens: stack.lens,
        exposure: stack.exposure,
        ..Default::default()
    };
    let (w, h) = session.framed_proxy(&probe, 400);
    let (px, py) = (
        (x.clamp(0.0, 1.0) * (w - 1) as f32).round() as i64,
        (y.clamp(0.0, 1.0) * (h - 1) as f32).round() as i64,
    );
    let sample = |probe: &EditStack| -> Result<[f32; 3]> {
        let pixels = pipeline.render_to_pixels(gpu, session, probe, w, h)?;
        // A 5x5 patch, so one noisy pixel doesn't set the balance.
        let mut sum = [0f32; 3];
        let mut n = 0.0;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let (sx, sy) = (px + dx, py + dy);
                if sx < 0 || sy < 0 || sx >= w as i64 || sy >= h as i64 {
                    continue;
                }
                let i = ((sy as usize) * (w as usize) + sx as usize) * 4;
                for c in 0..3 {
                    sum[c] += pixels[i + c] as f32 / 255.0;
                }
                n += 1.0;
            }
        }
        Ok(sum.map(|v| v / n))
    };

    // Bring the point to a mid-tone. Display light is roughly a power of
    // scene light, so a few proportional steps settle it.
    const MID: f32 = 0.4;
    for _ in 0..4 {
        let rgb = sample(&probe)?;
        let peak = rgb.iter().copied().fold(0.0f32, f32::max);
        if peak < 0.005 {
            anyhow::bail!("That spot is too dark to read. Pick something grey and well lit.");
        }
        let off = (MID / peak).log2();
        if off.abs() < 0.1 {
            break;
        }
        probe.exposure = (probe.exposure + 1.5 * off).clamp(-8.0, 8.0);
    }

    // Back from the screen towards the camera's own channels, where the
    // sliders act: undo the display encoding, the output matrix, the neutral
    // tone curve and the camera matrix. The inverses need not be exact, only
    // steady: grey maps to grey through every one of them, so where this reads
    // neutral, the screen does too.
    use crate::gpu::color;
    let display_to_working =
        crate::raw::decoder::invert3(color::REC2020_TO_SRGB).context("output matrix")?;
    let working_to_camera =
        crate::raw::decoder::invert3(session.cam_to_working()).context("camera matrix")?;
    let to_camera = |rgb: [f32; 3]| {
        let lin = rgb.map(crate::raster::srgb_to_linear);
        let toned = color::apply(display_to_working, lin);
        let scene = toned.map(|v| crate::develop::tone::ToneParams::neutral_inverse(v.max(0.0)));
        color::apply(working_to_camera, scene).map(|v| v.max(1e-5))
    };
    crate::develop::auto::neutralise(|warmth, tint| {
        probe.warmth = warmth;
        probe.tint = tint;
        Ok(to_camera(sample(&probe)?))
    })
}
