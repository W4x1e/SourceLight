//! Stand-in thumbnails for the grid spike (docs/MOBILE.md, "Spikes first").
//!
//! The question the spike answers is whether a phone's webview scrolls a
//! 20,000-tile grid smoothly. What makes that hard is decoding and drawing
//! JPEGs while flinging, not where they came from, so these are made here
//! rather than fetched from a desktop: 48 distinct images the size of the
//! desktop's real thumbnails (512 px on the long edge, about 30 KB, measured on
//! a real library), served round-robin under 20,000 different URLs so the
//! webview decodes each tile as a separate image, as it will for real ones.

use std::sync::OnceLock;

use image::codecs::jpeg::JpegEncoder;
use image::{Rgb, RgbImage};
use tauri::http::{Request, Response};

const POOL: usize = 48;

fn pool() -> &'static [Vec<u8>] {
    static POOL_JPEGS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    POOL_JPEGS.get_or_init(|| (0..POOL).map(make).collect())
}

/// One stand-in: a colour wash with fine noise, so it compresses like a
/// photo rather than to a few hundred bytes. Every fifth one is upright.
fn make(i: usize) -> Vec<u8> {
    let (w, h) = if i % 5 == 4 { (341, 512) } else { (512, 341) };
    let hue = i as f32 / POOL as f32;
    let mut seed = 0x9e37_79b9_u32.wrapping_mul(i as u32 + 1);
    let img = RgbImage::from_fn(w, h, |x, y| {
        // xorshift: cheap, deterministic grain.
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let grain = (seed % 48) as f32 - 24.0;
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let base = |phase: f32| {
            let v = (std::f32::consts::TAU * (hue + phase) + fx * 2.0 - fy).sin() * 0.5 + 0.5;
            (40.0 + v * 170.0 + grain).clamp(0.0, 255.0) as u8
        };
        Rgb([base(0.0), base(0.33), base(0.66)])
    });
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 80)
        .encode_image(&img)
        .expect("encoding to memory cannot fail");
    out
}

/// `thumb://localhost/<n>`: stand-in thumbnail `n`.
pub fn serve(request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let n: usize = request
        .uri()
        .path()
        .trim_start_matches('/')
        .parse()
        .unwrap_or(0);
    Response::builder()
        .header("Content-Type", "image/jpeg")
        .header("Cache-Control", "max-age=31536000, immutable")
        .body(pool()[n % POOL].clone())
        .expect("a static response builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stand_ins_are_the_size_of_real_thumbnails() {
        let sizes: Vec<usize> = pool().iter().map(Vec::len).collect();
        let mean = sizes.iter().sum::<usize>() / sizes.len();
        assert!((15_000..60_000).contains(&mean), "mean {mean} bytes");
        let first = image::load_from_memory(&pool()[0]).unwrap();
        assert_eq!((first.width(), first.height()), (512, 341));
        let upright = image::load_from_memory(&pool()[4]).unwrap();
        assert_eq!((upright.width(), upright.height()), (341, 512));
    }

    #[test]
    fn any_number_serves_an_image() {
        for path in ["/0", "/19999", "/oops", "/"] {
            let req = Request::builder()
                .uri(format!("thumb://localhost{path}"))
                .body(Vec::new())
                .unwrap();
            let res = serve(&req);
            assert_eq!(res.status(), 200);
            assert!(res.body().starts_with(&[0xFF, 0xD8]), "{path}");
        }
    }
}
