//! Golden-image comparison for the render pipeline.
//!
//! Every shader stage gets a committed reference PNG. CI re-renders it and
//! fails if the result moves. This is the only thing standing between a tweak
//! to one shader and a silent change to the colour of everybody's exports, so
//! it is deliberately strict.
//!
//! **On "perceptual".** This compares channel values numerically rather than
//! using a real perceptual metric (CIEDE2000, SSIM). That is on purpose: a
//! perceptual metric is specifically designed to *ignore* small uniform colour
//! shifts, which in a develop pipeline are exactly the regressions worth
//! catching. Two plain numbers — worst single channel, and mean across the
//! image — catch both localised breakage and global drift, with no tuning
//! constants to quietly rot.
//!
//! Tolerance is not zero because GPU vendors round differently; see the
//! `k/255` note in `create_render_target`'s tests for how sharp that edge is.

use image::{ImageBuffer, Rgba, RgbaImage};
use std::path::{Path, PathBuf};

/// Set to any value to rewrite reference images from the current render and
/// pass. Review the resulting diff before committing — blessing is how an
/// unnoticed regression becomes the new expected output.
const BLESS_VAR: &str = "SOURCELIGHT_BLESS_GOLDEN";

pub struct Golden {
    name: String,
    max_channel_delta: u8,
    max_mean_delta: f64,
}

#[derive(Debug, PartialEq)]
struct Stats {
    max_channel: u8,
    mean: f64,
}

/// Worst single-channel difference, and the mean absolute difference across
/// every channel of every pixel.
fn compare(a: &[u8], b: &[u8]) -> Stats {
    let mut max_channel = 0u8;
    let mut total = 0u64;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x.abs_diff(*y);
        max_channel = max_channel.max(d);
        total += d as u64;
    }
    Stats {
        max_channel,
        mean: total as f64 / a.len() as f64,
    }
}

impl Golden {
    /// `name` becomes the reference filename, so keep it stable and unique.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            // Tight enough that a real pipeline change trips it, loose enough
            // to absorb the last-bit rounding differences between vendors.
            max_channel_delta: 2,
            max_mean_delta: 0.5,
        }
    }

    pub fn tolerance(mut self, max_channel_delta: u8, max_mean_delta: f64) -> Self {
        self.max_channel_delta = max_channel_delta;
        self.max_mean_delta = max_mean_delta;
        self
    }

    fn reference_path(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(format!("{}.png", self.name))
    }

    fn failure_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/golden-failures")
    }

    /// Panics with a readable report unless `actual` matches the reference.
    pub fn assert_matches(&self, actual: &[u8], width: u32, height: u32) {
        assert_eq!(
            actual.len(),
            (width * height * 4) as usize,
            "{}: buffer is not {width}x{height} RGBA",
            self.name
        );
        let actual_img: RgbaImage = ImageBuffer::from_raw(width, height, actual.to_vec())
            .expect("buffer size already checked");

        let reference_path = self.reference_path();
        let blessing = std::env::var(BLESS_VAR).is_ok();

        if blessing || !reference_path.exists() {
            if !blessing {
                panic!(
                    "{}: no reference image at {}.\n\
                     Render it once with {BLESS_VAR}=1, inspect the result, then commit it.",
                    self.name,
                    reference_path.display()
                );
            }
            std::fs::create_dir_all(reference_path.parent().unwrap()).unwrap();
            actual_img.save(&reference_path).unwrap();
            eprintln!("blessed {}", reference_path.display());
            return;
        }

        let reference = image::open(&reference_path)
            .unwrap_or_else(|e| panic!("{}: cannot read reference: {e}", self.name))
            .to_rgba8();

        assert_eq!(
            reference.dimensions(),
            (width, height),
            "{}: reference is {:?}, render is {width}x{height}",
            self.name,
            reference.dimensions()
        );

        let stats = compare(actual, reference.as_raw());
        if stats.max_channel <= self.max_channel_delta && stats.mean <= self.max_mean_delta {
            return;
        }

        // Write the render and an amplified diff so the failure can actually be
        // looked at rather than just read about.
        let dir = Self::failure_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let actual_out = dir.join(format!("{}.actual.png", self.name));
        let diff_out = dir.join(format!("{}.diff.png", self.name));
        actual_img.save(&actual_out).unwrap();

        // Normalised so the worst pixel is full brightness. The magnitude is
        // already in the message below; this image only has to answer "where",
        // and a fixed multiplier renders small-but-real deltas near-black.
        let scale = 255.0 / stats.max_channel.max(1) as f32;
        let diff: RgbaImage = ImageBuffer::from_fn(width, height, |x, y| {
            let a = actual_img.get_pixel(x, y).0;
            let b = reference.get_pixel(x, y).0;
            let amp = |i: usize| (a[i].abs_diff(b[i]) as f32 * scale).min(255.0) as u8;
            Rgba([amp(0), amp(1), amp(2), 255])
        });
        diff.save(&diff_out).unwrap();

        panic!(
            "{}: render differs from reference.\n\
             worst channel {} (limit {}), mean {:.4} (limit {})\n\
             reference:  {}\n\
             render:     {}\n\
             diff (x{scale:.0}): {}",
            self.name,
            stats.max_channel,
            self.max_channel_delta,
            stats.mean,
            self.max_mean_delta,
            reference_path.display(),
            actual_out.display(),
            diff_out.display(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_buffers_have_no_difference() {
        let a = [10u8, 20, 30, 255];
        assert_eq!(
            compare(&a, &a),
            Stats {
                max_channel: 0,
                mean: 0.0
            }
        );
    }

    /// A single badly-wrong pixel must not be averaged away by a large,
    /// otherwise-correct image — that is the failure mode a mean-only check has.
    #[test]
    fn one_bad_channel_survives_averaging() {
        // A 32x32 image's worth of channels, one of them badly wrong.
        let mut a = vec![0u8; 32 * 32 * 4];
        let b = vec![0u8; 32 * 32 * 4];
        a[7] = 200;

        let stats = compare(&a, &b);
        assert_eq!(stats.max_channel, 200);
        assert!(
            stats.mean < 0.05,
            "mean is {:.4} — averaging alone would wave this through",
            stats.mean
        );
    }
}
