//! Readback presenter: render, copy to the CPU, wrap, hand to the webview.
//!
//! Slower than drawing to a surface — a full round trip through system memory —
//! but it works everywhere, needs no window compositing, and the frame can be
//! written to a file when a render looks wrong. It stays in the codebase after
//! the direct presenter lands, as the fallback and the debugging path.
//!
//! **Frames go out as uncompressed BMP, not JPEG.** Measured at 1400x935:
//! JPEG 32.1ms and 187KB against BMP 0.88ms and 5113KB. The encode was about
//! nine tenths of every interaction, and trading 5MB of in-process memcpy for
//! 31ms of CPU is not a close call. It also makes the preview *lossless*, which
//! matters more than it sounds: JPEG artefacts at quality 90 are easy to
//! mistake for sensor noise, and the sharpening and noise controls are judged
//! at 1:1 on exactly that detail.

use anyhow::Result;

use super::{Frame, Presented, Presenter};

#[derive(Default)]
pub struct ReadbackPresenter;

impl ReadbackPresenter {
    pub fn new() -> Self {
        Self
    }
}

/// Wraps the rendered pixels in a BMP header, without compressing them.
///
/// **The encode was the frame.** Measured on a 5070 Ti at 1400x935: GPU render
/// 0.3ms, readback 5ms, JPEG encode 49ms — the encode was roughly nine tenths of
/// every interaction. `image`'s JPEG encoder runs at about 26 MP/s against
/// libjpeg-turbo's 200, so the fix is not a faster JPEG but no JPEG at all.
///
/// A frame costs about 5MB this way instead of 200KB. That is the right trade
/// here and nowhere else: the custom URI scheme hands bytes to the webview
/// in-process, so the "transfer" is a memcpy, and the browser decodes a BMP by
/// reading it. Nothing crosses a network or touches a disk.
///
/// Written top-down with a negative height, so the rows go out in the order
/// they were rendered. A bottom-up BMP is the format's default and would need
/// the whole buffer reversed — which is exactly the sort of per-frame copy this
/// exists to remove.
pub fn encode_bmp(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() < expected {
        anyhow::bail!(
            "expected {expected} bytes for {width}x{height} RGBA, got {}",
            rgba.len()
        );
    }

    const FILE_HEADER: usize = 14;
    const INFO_HEADER: usize = 40;
    let pixels = expected;

    let mut out = Vec::with_capacity(FILE_HEADER + INFO_HEADER + pixels);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((FILE_HEADER + INFO_HEADER + pixels) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&((FILE_HEADER + INFO_HEADER) as u32).to_le_bytes());

    out.extend_from_slice(&(INFO_HEADER as u32).to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    // Negative: top-down. See above.
    out.extend_from_slice(&(-(height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB, uncompressed
    out.extend_from_slice(&(pixels as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi, horizontal
    out.extend_from_slice(&2835i32.to_le_bytes()); // and vertical
    out.extend_from_slice(&0u32.to_le_bytes()); // palette entries
    out.extend_from_slice(&0u32.to_le_bytes()); // important colours

    // BMP stores BGRA. 32 bits per pixel means rows are already 4-byte aligned,
    // so there is no padding to work out — the reason for choosing 32bpp over
    // the smaller 24.
    out.extend(
        rgba[..expected]
            .chunks_exact(4)
            .flat_map(|p| [p[2], p[1], p[0], 255]),
    );
    Ok(out)
}

impl Presenter for ReadbackPresenter {
    fn present(&mut self, frame: Frame<'_>) -> Result<Presented> {
        let pixels = frame.pipeline.render_view(
            frame.gpu,
            frame.session,
            frame.stack,
            frame.width,
            frame.height,
            frame.view.as_ref(),
            frame.soft_proof,
        )?;
        let bytes = encode_bmp(&pixels, frame.width, frame.height)?;
        Ok(Presented::Image {
            bytes,
            content_type: "image/bmp",
            width: frame.width,
            height: frame.height,
        })
    }

    fn describe(&self) -> &'static str {
        "readback (render -> CPU -> BMP)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                px.extend_from_slice(&[
                    (x * 255 / width.max(1)) as u8,
                    (y * 255 / height.max(1)) as u8,
                    128,
                    255,
                ]);
            }
        }
        px
    }

    /// The header has to be one a decoder accepts, and the size has to be
    /// exactly header plus pixels — a BMP claiming the wrong length either
    /// truncates or trails garbage.
    #[test]
    fn the_header_describes_the_frame() {
        let (w, h) = (64u32, 48u32);
        let bmp = encode_bmp(&gradient(w, h), w, h).unwrap();

        assert_eq!(&bmp[..2], b"BM", "not a BMP");
        assert_eq!(bmp.len(), 54 + (w * h * 4) as usize);
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize,
            bmp.len(),
            "the declared file size disagrees with the actual one"
        );
        // Negative height means top-down; a positive one would show the photo
        // upside down.
        assert!(
            i32::from_le_bytes(bmp[22..26].try_into().unwrap()) < 0,
            "rows are bottom-up"
        );
    }

    /// **Lossless, and that is half the point.** Every pixel has to survive
    /// exactly: the sharpening and noise controls are judged at 1:1 on
    /// single-pixel detail, and JPEG artefacts there are indistinguishable
    /// from the sensor noise the controls exist to manage.
    #[test]
    fn every_pixel_survives_the_round_trip() {
        let (w, h) = (37u32, 21u32);
        let source = gradient(w, h);
        let bmp = encode_bmp(&source, w, h).unwrap();

        let decoded = image::load_from_memory(&bmp).expect("re-decodes").to_rgb8();
        assert_eq!((decoded.width(), decoded.height()), (w, h));

        for (i, px) in decoded.pixels().enumerate() {
            let want = &source[i * 4..i * 4 + 3];
            assert_eq!(
                px.0,
                [want[0], want[1], want[2]],
                "pixel {i} came back changed"
            );
        }
    }

    /// A short buffer is a bug elsewhere, and writing a header over it would
    /// produce a file that decodes into garbage rather than failing.
    #[test]
    fn a_short_buffer_is_refused() {
        assert!(encode_bmp(&[0; 16], 64, 48).is_err());
    }
}
