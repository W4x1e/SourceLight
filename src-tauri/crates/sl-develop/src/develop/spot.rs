//! Spot removal: clone and heal patches for dust and blemishes.
//!
//! Table stakes for any raw editor — every sensor has dust, every portrait has a
//! blemish. A spot copies pixels from a *source* location over a *target*
//! circle. Two modes:
//!
//! - **Clone** copies the source directly.
//! - **Heal** copies the source's *detail* but keeps the target's local
//!   brightness and colour, so the patch blends into its surroundings seamlessly.
//!   Done by subtracting the source's low-frequency and adding the target's —
//!   a cheap gradient-domain approximation using the mosaic mip chain.
//!
//! Like masks, spots are a handful of numbers in framed-output UV that serialise
//! into the [`crate::develop::EditStack`], cost nothing to store or undo, and are
//! evaluated analytically in the shader (redirecting where a pixel samples from)
//! rather than baked into a texture.

use serde::{Deserialize, Serialize};

/// The most spots a single photo may carry. The shader iterates a fixed-size
/// uniform array; a sensor with more than this many visible dust marks wants
/// cleaning, not software.
pub const MAX_SPOTS: usize = 40;

/// One clone/heal patch, all coordinates in framed-output UV (0..1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Spot {
    /// Centre of the area being repaired.
    pub target_x: f32,
    pub target_y: f32,
    /// Centre of the area being copied from.
    pub source_x: f32,
    pub source_y: f32,
    /// Radius of the patch, a fraction of the frame width.
    pub radius: f32,
    /// Edge softness, 0 (hard) .. 1 (fully soft). The fraction of the radius over
    /// which the patch fades into the original image.
    #[serde(default)]
    pub feather: f32,
    /// Heal (match surrounding tone) when true, plain clone when false.
    #[serde(default)]
    pub heal: bool,
}

impl Spot {
    /// Coverage in 0..1 at framed UV `(u, v)` — full at the target centre,
    /// fading to zero across the feather band at the radius. Mirrors the shader.
    pub fn coverage(&self, u: f32, v: f32) -> f32 {
        let d = ((u - self.target_x).powi(2) + (v - self.target_y).powi(2)).sqrt();
        let r = self.radius.max(1e-4);
        let inner = r * (1.0 - self.feather.clamp(0.0, 1.0));
        1.0 - smoothstep(inner, r, d)
    }
}

/// Matches WGSL `smoothstep`, so CPU and shader agree.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < 1e-6 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The spot set packed for the shader. Its own binding, for the same reason the
/// masks have one — `DevelopParams`' byte layout is padding-sensitive.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpotUniform {
    /// `[count, 0, 0, 0]`.
    header: [f32; 4],
    /// Drawn-quad UV → framed-image UV mapping, exactly as the masks use it:
    /// `[extent_x, extent_y, centre_x, centre_y]`. Identity at fit-to-window.
    view: [f32; 4],
    /// Two `vec4`s per spot: `[target_x, target_y, source_x, source_y]` and
    /// `[radius, feather, heal, 0]`.
    spots: [[f32; 4]; MAX_SPOTS * 2],
}

/// Packs up to [`MAX_SPOTS`] spots; extras are dropped. `view` maps the drawn UV
/// to framed UV — the same value the masks are packed with.
pub fn pack(spots: &[Spot], view: [f32; 4]) -> SpotUniform {
    let mut out = SpotUniform {
        header: [0.0; 4],
        view,
        spots: [[0.0; 4]; MAX_SPOTS * 2],
    };
    let count = spots.len().min(MAX_SPOTS);
    out.header = [count as f32, 0.0, 0.0, 0.0];
    for (i, s) in spots.iter().take(MAX_SPOTS).enumerate() {
        out.spots[i * 2] = [s.target_x, s.target_y, s.source_x, s.source_y];
        out.spots[i * 2 + 1] = [s.radius, s.feather, if s.heal { 1.0 } else { 0.0 }, 0.0];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot() -> Spot {
        Spot {
            target_x: 0.5,
            target_y: 0.5,
            source_x: 0.3,
            source_y: 0.3,
            radius: 0.1,
            feather: 0.3,
            heal: true,
        }
    }

    #[test]
    fn coverage_is_full_at_the_target_and_zero_outside() {
        let s = spot();
        assert!(
            (s.coverage(0.5, 0.5) - 1.0).abs() < 1e-4,
            "target centre is full"
        );
        assert!(s.coverage(0.9, 0.9) < 1e-4, "well outside is empty");
        // On the feather band, strictly partial.
        let edge = s.coverage(0.5 + 0.1 * 0.9, 0.5);
        assert!(
            edge > 0.0 && edge < 1.0,
            "the feather band should be partial, got {edge}"
        );
    }

    #[test]
    fn packing_reports_the_count_and_lays_out_the_fields() {
        let u = pack(&[spot()], crate::develop::mask::IDENTITY_VIEW);
        assert_eq!(u.header[0], 1.0, "count");
        assert_eq!(u.spots[0], [0.5, 0.5, 0.3, 0.3], "target then source");
        assert_eq!(
            u.spots[1],
            [0.1, 0.3, 1.0, 0.0],
            "radius, feather, heal flag"
        );
    }

    #[test]
    fn a_plain_clone_spot_reports_heal_zero() {
        let mut s = spot();
        s.heal = false;
        let u = pack(&[s], crate::develop::mask::IDENTITY_VIEW);
        assert_eq!(u.spots[1][2], 0.0, "clone spot has heal flag 0");
    }

    #[test]
    fn packing_caps_at_the_maximum() {
        let many = vec![spot(); MAX_SPOTS + 5];
        assert_eq!(
            pack(&many, crate::develop::mask::IDENTITY_VIEW).header[0],
            MAX_SPOTS as f32
        );
    }
}
