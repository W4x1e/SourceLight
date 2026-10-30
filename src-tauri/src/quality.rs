//! On-device image-quality scoring for culling — focus/sharpness, and how bright
//! the frame is overall.
//!
//! No model and no `ort` dependency: this is classical computer vision (the
//! variance of a Laplacian), so it runs in **every** build, not just
//! `--features ai`. The score is a **relative focus-energy** metric — higher is
//! sharper — computed on a fixed working size so the same photo scores the same
//! at any resolution. It is meant for ranking *within* a set (best of a burst)
//! and for a per-set "soft" threshold, not as an absolute physical unit.
//!
//! Brightness, by contrast, *is* absolute: the mean of the camera's own
//! rendering, 0 black to 1 white. The cull uses it only at the extremes
//! — a lens-cap frame, a flash that didn't fire, a frame blown to white — where
//! no judgement about taste is involved.

use anyhow::{Context, Result};
use image::{imageops::FilterType, GenericImageView, GrayImage};

/// Longest edge the score is computed at. Fixing the working size is what makes
/// the metric resolution-stable: a full-res frame and its proxy score alike.
const WORK_EDGE: u32 = 512;

/// Variance of the Laplacian of `luma` — the standard focus measure. A sharp
/// image has strong second derivatives (edges), hence a high variance; a blurred
/// or flat one has little. Computed on values normalised to `[0, 1]` so the
/// number is independent of bit depth. Returns 0 for a degenerate (tiny) image.
pub fn variance_of_laplacian(luma: &GrayImage) -> f32 {
    let (w, h) = luma.dimensions();
    if w < 3 || h < 3 {
        return 0.0;
    }
    let at = |x: u32, y: u32| luma.get_pixel(x, y)[0] as f64 / 255.0;

    // One pass over the interior, accumulating the Laplacian's mean and
    // mean-of-squares so the variance falls out at the end.
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut n = 0u64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let lap = at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1) - 4.0 * at(x, y);
            sum += lap;
            sum_sq += lap * lap;
            n += 1;
        }
    }
    let inv = 1.0 / n as f64;
    let mean = sum * inv;
    ((sum_sq * inv) - mean * mean).max(0.0) as f32
}

/// Focus/sharpness score of an already-decoded image: downscale to a fixed
/// working size (for resolution-stability), convert to grey, measure. Higher is
/// sharper.
pub fn sharpness(img: &image::DynamicImage) -> f32 {
    variance_of_laplacian(&working_luma(img))
}

/// The image at the fixed working size, as grey.
fn working_luma(img: &image::DynamicImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let scaled = if w.max(h) > WORK_EDGE {
        img.resize(WORK_EDGE, WORK_EDGE, FilterType::Triangle)
    } else {
        img.clone()
    };
    scaled.to_luma8()
}

/// Mean brightness of grey values, 0 (black) to 1 (white). 0 for an empty image.
pub fn mean_brightness(luma: &GrayImage) -> f32 {
    let n = luma.pixels().len();
    if n == 0 {
        return 0.0;
    }
    let sum: u64 = luma.pixels().map(|p| p[0] as u64).sum();
    (sum as f64 / (n as f64 * 255.0)) as f32
}

/// What the quality pass measures about one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scores {
    /// See [`sharpness`]: relative, higher is sharper.
    pub sharpness: f32,
    /// See [`mean_brightness`]: absolute, 0 black to 1 white.
    pub brightness: f32,
}

/// Both scores of an image already decoded — the scan's grid thumbnail, in
/// practice, which is already at the working size — downscaled once for the
/// two.
pub fn scores(img: &image::DynamicImage) -> Scores {
    let luma = working_luma(img);
    Scores {
        sharpness: variance_of_laplacian(&luma),
        brightness: mean_brightness(&luma),
    }
}

/// [`scores`] straight from encoded image bytes — a cached preview, for a
/// photo scanned before scores were kept.
pub fn scores_of_bytes(bytes: &[u8]) -> Result<Scores> {
    let img = image::load_from_memory(bytes).context("decoding image for quality scoring")?;
    Ok(scores(&img))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GrayImage, Luma};

    /// A hard vertical edge: the left half black, the right half white. Lots of
    /// high-frequency energy at the seam.
    fn edge(size: u32) -> DynamicImage {
        let mut img = GrayImage::new(size, size);
        for y in 0..size {
            for x in 0..size {
                let v = if x < size / 2 { 0 } else { 255 };
                img.put_pixel(x, y, Luma([v]));
            }
        }
        DynamicImage::ImageLuma8(img)
    }

    fn flat(size: u32, level: u8) -> DynamicImage {
        DynamicImage::ImageLuma8(GrayImage::from_pixel(size, size, Luma([level])))
    }

    #[test]
    fn a_sharp_edge_scores_higher_than_its_blurred_copy() {
        let sharp = edge(256);
        let blurred = DynamicImage::ImageLuma8(image::imageops::blur(&sharp.to_luma8(), 4.0));
        let s = sharpness(&sharp);
        let b = sharpness(&blurred);
        assert!(s > b * 2.0, "sharp {s} should clearly beat blurred {b}");
    }

    #[test]
    fn a_flat_field_scores_about_zero() {
        assert!(sharpness(&flat(256, 128)) < 1e-4);
    }

    #[test]
    fn brightness_reads_black_grey_and_white() {
        assert_eq!(mean_brightness(&flat(64, 0).to_luma8()), 0.0);
        assert!((mean_brightness(&flat(64, 128).to_luma8()) - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(mean_brightness(&flat(64, 255).to_luma8()), 1.0);
        // Half black, half white.
        assert!((mean_brightness(&edge(64).to_luma8()) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn scores_come_from_one_decode_of_the_bytes() {
        let mut png = Vec::new();
        edge(256)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let s = scores_of_bytes(&png).unwrap();
        assert_eq!(s.sharpness, sharpness(&edge(256)));
        assert!((s.brightness - 0.5).abs() < 0.01, "{s:?}");
        assert!(scores_of_bytes(b"not an image").is_err());
    }

    #[test]
    fn the_score_is_stable_across_resolution() {
        // The same picture at two sizes must land close. Both are larger than the
        // fixed work edge, so both are normalised down to it before scoring —
        // which is the whole point of `WORK_EDGE`, and matches reality, where
        // camera previews are always well above 512px. Nearest-upscaling the base
        // adds no real detail, so the two should agree within a modest tolerance.
        let base = edge(768);
        let big = base.resize_exact(1536, 1536, FilterType::Nearest);
        let a = sharpness(&base);
        let b = sharpness(&big);
        let ratio = a / b;
        assert!(
            (0.6..1.7).contains(&ratio),
            "resolution changed the score too much: {a} vs {b} (ratio {ratio})"
        );
    }
}
