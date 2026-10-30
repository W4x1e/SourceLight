//! The GPU test screen (docs/MOBILE.md section 4, step 1): the desktop's
//! Develop pipeline run on this phone, timed, with sliders on the result.
//! Temporary, until editing on the phone is built.

use std::sync::Mutex;

use sl_gpu_test::bench::{Live, Report};
use tauri::http::Response;
use tauri::{Manager, State};

use crate::cache::Kind;
use crate::link::Link;

/// The first photo of the last test, still on the GPU for the sliders.
#[derive(Default)]
pub struct GpuTest {
    live: Mutex<Option<Live>>,
}

/// Runs the test on photo `id`'s preview, or on a made-up picture when there
/// is none, then on the same picture enlarged to 12 MP. Frames are rendered
/// at the size the photo fills a screen of `width` × `height` device pixels.
#[tauri::command]
pub async fn gpu_test(
    app: tauri::AppHandle,
    link: State<'_, Link>,
    test: State<'_, GpuTest>,
    id: Option<i64>,
    width: u32,
    height: u32,
) -> Result<Report, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("gpu-test");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let photo = dir.join("photo.jpg");
    let large = dir.join("12mp.jpg");
    let preview = match id {
        Some(id) => link.image(Kind::Preview, id).await,
        None => None,
    };
    let label = match &preview {
        Some(jpeg) => {
            std::fs::write(&photo, jpeg).map_err(|e| e.to_string())?;
            "Preview"
        }
        None => {
            test_card(&photo).map_err(|e| e.to_string())?;
            "Test card"
        }
    };
    // Dropped first: one photo on the GPU at a time.
    *test.live.lock().unwrap() = None;
    let screen = (width.clamp(64, 8192), height.clamp(64, 8192));
    let (report, live) = tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = sl_gpu_test::bench::enlarge(&photo, &large, 4000) {
            let report = Report {
                error: Some(format!("making the 12 MP picture: {e:#}")),
                ..Report::default()
            };
            return (report, None);
        }
        let out = sl_gpu_test::bench::run(&[(label, &photo), ("12 MP", &large)], screen);
        let _ = std::fs::remove_file(&photo);
        let _ = std::fs::remove_file(&large);
        out
    })
    .await
    .map_err(|e| e.to_string())?;
    *test.live.lock().unwrap() = live;
    Ok(report)
}

/// A 1600 × 1067 picture with smooth gradients and colour, for a phone that
/// has no library yet.
fn test_card(to: &std::path::Path) -> anyhow::Result<()> {
    let (w, h) = (1600u32, 1067u32);
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
        let band = (u * 6.0) as u32;
        let [r, g, b] = match band {
            0 => [1.0, u * 6.0, 0.0],
            1 => [2.0 - u * 6.0, 1.0, 0.0],
            2 => [0.0, 1.0, u * 6.0 - 2.0],
            3 => [0.0, 4.0 - u * 6.0, 1.0],
            4 => [u * 6.0 - 4.0, 0.0, 1.0],
            _ => [1.0, 0.0, 6.0 - u * 6.0],
        };
        // Full colour at the top, fading through grey to black at the bottom.
        let grey = 0.5;
        let mix = |c: f32| ((c * (1.0 - v) + grey * v) * (1.0 - v * 0.6) * 255.0) as u8;
        image::Rgb([mix(r), mix(g), mix(b)])
    });
    img.save_with_format(to, image::ImageFormat::Jpeg)?;
    Ok(())
}

/// `gputest://localhost/<jpeg|rgba>_<exposure>_<warmth>`: one frame for the
/// sliders, with what it cost in headers.
pub fn serve(app: &tauri::AppHandle, path: &str) -> Response<Vec<u8>> {
    let mut parts = path.trim_start_matches('/').split('_');
    let as_jpeg = parts.next() == Some("jpeg");
    let exposure: f32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let warmth: f32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let test = app.state::<GpuTest>();
    let live = test.live.lock().unwrap();
    let frame = match live.as_ref() {
        Some(live) => live.frame(exposure, warmth, as_jpeg),
        None => Err(anyhow::anyhow!("run the test first")),
    };
    let builder = Response::builder()
        // The page is on another origin, and reads the timings.
        .header("Access-Control-Allow-Origin", "*")
        .header(
            "Access-Control-Expose-Headers",
            "X-Width, X-Height, X-Render-Ms, X-Encode-Ms",
        )
        .header("Cache-Control", "no-store");
    match frame {
        Ok(f) => builder
            .header(
                "Content-Type",
                if as_jpeg {
                    "image/jpeg"
                } else {
                    "application/octet-stream"
                },
            )
            .header("X-Width", f.width.to_string())
            .header("X-Height", f.height.to_string())
            .header("X-Render-Ms", format!("{:.1}", f.render_ms))
            .header("X-Encode-Ms", format!("{:.1}", f.encode_ms))
            .body(f.bytes),
        Err(e) => builder.status(500).body(format!("{e:#}").into_bytes()),
    }
    .expect("a response builds")
}
