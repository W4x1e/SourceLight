//! Local contrast: clarity and texture.
//!
//! Both are the same operation at different scales — compare the image against
//! a blurred copy of itself and push the difference — and the scale is the
//! whole distinction. Clarity works on broad midtone structure and reads as
//! "punch"; texture works on fine detail and reads as skin, fabric or foliage
//! becoming crisper.
//!
//! **The blur comes from the mip chain**, which already exists for resampling.
//! A 120-pixel Gaussian in a fragment shader would need hundreds of taps; the
//! matching mip level is one, already filtered, and built once per photo.
//!
//! **Radii are fractions of the image, not pixel counts.** That is what keeps
//! the preview and the export agreeing: a radius fixed in output pixels would
//! target completely different scene detail at 1400px than at 6000px, so the
//! exported file would not match what was on screen. Expressed as a fraction,
//! the mip level is an absolute property of the photo and both paths sample the
//! same one.

use serde::{Deserialize, Serialize};

/// Blur radius for clarity, as a fraction of the longest edge.
///
/// Broad enough to shape the midtone structure of a scene rather than its
/// edges — around 120 pixels on a 24MP frame.
const CLARITY_RADIUS: f32 = 0.02;

/// The same for texture, roughly 18 pixels on the same frame: fine enough to
/// act on surface detail, coarse enough not to be pure noise.
const TEXTURE_RADIUS: f32 = 0.003;

/// Blur radius the **chroma** noise reduction pulls colour toward — colour
/// speckle is coarse, so this is a good deal wider than the luminance grain
/// (~36 pixels on a 24MP frame). Expressed as a fraction so the mip level is a
/// scene property and the preview matches the export, exactly like clarity.
const NOISE_CHROMA_RADIUS: f32 = 0.006;

/// The coarse scale of the **luminance** noise reduction — the low-frequency
/// blotch a small bilateral window cannot reach (~15 pixels). The fine scale is
/// the bilateral over the sharpening neighbourhood; this is the second band.
const NOISE_LUMA_RADIUS: f32 = 0.0025;

/// The atmospheric scale for **dehaze**: the broad neighbourhood whose average
/// stands in for the local airlight (~180 pixels on a 24MP frame). Broader than
/// clarity — haze is a low-frequency veil, so the estimate must average over
/// more than the structure being recovered or it would cancel it. A fraction of
/// the long edge, like every other radius, so the mip level is a scene property
/// and the preview matches the export.
const DEHAZE_RADIUS: f32 = 0.03;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct Detail {
    /// -1..=1. Broad local contrast.
    pub clarity: f32,
    /// -1..=1. Fine local contrast.
    pub texture: f32,

    /// 0..=1. Capture sharpening strength.
    ///
    /// **Judge this at 1:1 and nowhere else.** Sharpening acts on single
    /// pixels, so at fit-to-window — where each screen pixel already averages
    /// eighteen of them — it is invisible by construction. That is correct
    /// behaviour rather than a bug, and it is why this control could not
    /// honestly be built before the viewer could zoom.
    pub sharpen: f32,
    /// Radius in source pixels, roughly 0.5..=3. Below one pixel there is
    /// nothing to sharpen; far above it the halo becomes the visible effect.
    pub sharpen_radius: f32,
    /// 0..=1. Withholds sharpening from flat areas.
    ///
    /// Without it, sharpening amplifies sensor noise hardest exactly where
    /// there is no detail to recover — clear sky, shadow, skin. Other editors
    /// call the same control masking.
    pub sharpen_masking: f32,

    /// 0..=1. Sharpening *character*: 0 is a plain unsharp mask (broad, a little
    /// haloed), 1 is a deconvolution-style response that crisps the finest detail
    /// with less halo. The usual "Detail" slider under Sharpening. Zero keeps
    /// the unsharp mask exactly as it was.
    #[serde(default)]
    pub sharpen_detail: f32,

    /// 0..=1. Luminance noise reduction.
    pub noise: f32,

    /// 0..=1. Colour (chroma) noise reduction — removes the coloured speckle
    /// high-ISO shots pick up, independently of the luminance grain. Blurs
    /// chroma while keeping each pixel's luminance, so detail survives.
    #[serde(default)]
    pub color_noise: f32,

    /// -1..=1. Dehaze. Positive cuts atmospheric veil (expands local contrast
    /// and restores saturation, weighted by a dark-channel estimate of how hazy
    /// each region is); negative adds veil. Zero is an exact no-op.
    #[serde(default)]
    pub dehaze: f32,

    /// 0..=1. AI noise reduction's Amount: how much of NAFNet's result
    /// replaces the linear image, before anything else in the pipeline. Not a
    /// shader parameter: the model runs once per photo (`denoise.rs`), and
    /// Develop and export blend its result in. Zero is off.
    #[serde(default)]
    pub ai_denoise: f32,
}

/// Mip levels to sample for the two blurs, and the finest level the current
/// render can actually resolve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailLods {
    pub texture: f32,
    pub clarity: f32,
    /// Mip level for the chroma-noise blur.
    pub noise_chroma: f32,
    /// Mip level for the coarse band of the luminance-noise reduction.
    pub noise_luma: f32,
    /// Mip level for the dehaze atmospheric-light estimate.
    pub dehaze: f32,
    /// `log2` of how many source pixels each output pixel spans.
    ///
    /// Anything finer than this is not present in the rendered image, so both
    /// radii are held at or above it. Without that floor, a proxy render would
    /// ask for detail at a level it is already downsampling past and the band
    /// would come back empty — or worse, negative, inverting the control.
    pub floor: f32,
}

impl Default for Detail {
    /// Everything off, but with a *usable* sharpening radius rather than zero.
    ///
    /// A radius of zero describes no neighbourhood at all, so the slider would
    /// start at a setting that cannot do anything even after the amount is
    /// raised. One source pixel is the sensible starting point and the one
    /// other editors open on.
    fn default() -> Self {
        Self {
            clarity: 0.0,
            texture: 0.0,
            sharpen: 0.0,
            sharpen_radius: 1.0,
            sharpen_masking: 0.0,
            sharpen_detail: 0.0,
            noise: 0.0,
            color_noise: 0.0,
            dehaze: 0.0,
            ai_denoise: 0.0,
        }
    }
}

impl Detail {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    /// Chooses mip levels for a photo of `long_edge` source pixels being
    /// rendered `output_long_edge` wide.
    pub fn lods(long_edge: u32, output_long_edge: u32) -> DetailLods {
        let long = long_edge.max(1) as f32;
        let out = output_long_edge.max(1) as f32;
        let floor = (long / out).max(1.0).log2();

        let level = |fraction: f32| (fraction * long).max(1.0).log2().max(floor);
        DetailLods {
            texture: level(TEXTURE_RADIUS),
            clarity: level(CLARITY_RADIUS),
            noise_chroma: level(NOISE_CHROMA_RADIUS),
            noise_luma: level(NOISE_LUMA_RADIUS),
            dehaze: level(DEHAZE_RADIUS),
            floor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The radii have to describe the same scene detail whatever size the
    /// render is, or the preview and the exported file disagree.
    #[test]
    fn the_radii_do_not_depend_on_the_render_size() {
        let full = Detail::lods(6000, 6000);
        let proxy = Detail::lods(6000, 1400);

        assert!(
            (full.clarity - proxy.clarity).abs() < 1e-5,
            "clarity moved between render sizes: {} vs {}",
            full.clarity,
            proxy.clarity
        );
    }

    /// Clarity is the broader of the two, by construction. Getting these the
    /// wrong way round would swap what the two sliders do.
    #[test]
    fn clarity_is_broader_than_texture() {
        let l = Detail::lods(6000, 6000);
        assert!(l.clarity > l.texture, "{l:?}");
        // And both describe a real blur rather than the source itself.
        assert!(l.texture > 0.0, "{l:?}");
    }

    /// A render too small to resolve the fine radius has it held at the level
    /// the render actually reaches.
    ///
    /// Without the floor the shader would difference two mip levels in the
    /// wrong order and the texture slider would invert — a fault that looks
    /// like a sign error somewhere else entirely.
    ///
    /// Worth knowing where the floor actually bites: at the normal 1400px
    /// proxy it does **not**. The fine radius is ~18 source pixels, which is
    /// still 4 pixels wide once downscaled, so texture is visible while
    /// editing rather than only at 1:1. It only binds on genuinely small
    /// renders, like the export dialog's thumbnails.
    #[test]
    fn a_render_too_small_to_resolve_the_fine_radius_clamps_it() {
        let proxy = Detail::lods(6000, 1400);
        assert!(
            proxy.texture > proxy.floor,
            "the ordinary proxy should not be clamping texture: {proxy:?}"
        );

        let tiny = Detail::lods(6000, 200);
        assert_eq!(
            tiny.texture, tiny.floor,
            "a tiny render should pin texture to its floor: {tiny:?}"
        );
        assert!(tiny.texture > proxy.texture, "{tiny:?} vs {proxy:?}");
    }

    /// A tiny image must not produce a negative or NaN level. Reachable through
    /// the export dialog's small-preview sizes.
    #[test]
    fn small_images_still_produce_usable_levels() {
        for (long, out) in [(1u32, 1u32), (16, 16), (64, 4000), (4000, 1)] {
            let l = Detail::lods(long, out);
            for v in [l.texture, l.clarity, l.floor] {
                assert!(v.is_finite() && v >= 0.0, "{long}x{out} produced {l:?}");
            }
            assert!(l.clarity >= l.texture, "{long}x{out} produced {l:?}");
        }
    }
}
