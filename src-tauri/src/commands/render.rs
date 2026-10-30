//! Develop's preview: frames served on `develop://`, and direct rendering
//! to the window.

use tauri::{AppHandle, Manager, State};

use super::common::{err, with_develop};
use crate::{develop, present, sync, AppState};

/// Turns the direct surface on or off: the develop preview drawn straight into
/// a layer beneath the webview, instead of read back and handed over as an
/// image. Returns the GPU and path in use.
///
/// **Reversible on purpose.** The photo, its edits and its history all survive
/// the switch — only the display path changes — and the `develop://` images
/// keep working either way, so switching back always restores a working
/// preview without a restart. Turning it off drops the presenter, which tears
/// the surface down with it.
///
/// `async` for a specific reason: it has to wait for the main thread to create
/// the surface, and a synchronous Tauri command may already *be* on the main
/// thread, where that would deadlock. An async command runs on the runtime's
/// pool, so the main thread is always someone else.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_direct_rendering(app: AppHandle, enabled: bool) -> Result<String, String> {
    let state = app.state::<AppState>();
    if !enabled {
        // Taken out under the lock, torn down outside it: dropping it
        // waits on the GPU and hides its window, and a main-thread command
        // waiting for the lock meanwhile could never answer that.
        let old = with_develop(&state, |dev| Ok(dev.take_direct()))?;
        drop(old);
        return with_develop(&state, |dev| Ok(dev.describe()));
    }
    if with_develop(&state, |dev| Ok(dev.has_direct()))? {
        return with_develop(&state, |dev| Ok(dev.describe()));
    }

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "no main window".to_string())?;

    // A Win32 window belongs to the thread that created it, and AppKit views
    // may only be touched on the main thread; `with_webview` runs there and
    // hands over the platform's webview. Everything afterwards — moving the
    // surface, tearing it down — goes through entry points safe from any
    // thread.
    let (tx, rx) = std::sync::mpsc::channel();
    let owner = window.clone();
    window
        .with_webview(move |webview| {
            let made = present::host::HostedSurface::new(&owner, &webview).map_err(err);
            let _ = tx.send(made);
        })
        .map_err(err)?;
    let host = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "the main thread did not create the render surface".to_string())??;

    with_develop(&state, |dev| {
        let backend = dev.gpu().info.backend;
        let presenter = present::surface::SurfacePresenter::new(dev.gpu(), dev.pipeline(), host)
            .map_err(|e| format!("{e:#} [{backend:?} backend]"))?;
        dev.set_direct(Some(Box::new(presenter)));
        Ok(dev.describe())
    })
}

/// Fills the render surface with flat magenta and reports what it did.
///
/// A blank canvas has two causes that look the same from the outside — the
/// surface is not being composited, or it is and the photo did not draw — and
/// they need opposite fixes. This draws a colour no photograph contains,
/// through no part of the render graph, so one look settles it.
#[tauri::command]
#[specta::specta]
pub(crate) fn probe_direct(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<String, String> {
    let state = app.state::<AppState>();
    let (width, height) = (
        width.clamp(1, MAX_RENDER_EDGE),
        height.clamp(1, MAX_RENDER_EDGE),
    );
    with_develop(&state, |dev| {
        dev.place_target(x, y, width, height).map_err(err)?;
        dev.probe_target().map_err(err)
    })
}

/// `stack` (`None` is the current one) with `show` as the mask to tint; as
/// it was when there is none to show.
fn show_mask(
    dev: &develop::DevelopState,
    stack: Option<develop::EditStack>,
    show: Option<u32>,
) -> Result<Option<develop::EditStack>, String> {
    let Some(show) = show else {
        return Ok(stack);
    };
    let mut stack = match stack {
        Some(s) => s,
        None => dev.current_stack().map_err(err)?,
    };
    stack.show_mask = Some(show);
    Ok(Some(stack))
}

/// The edit stack a preview shows: the current one, or — for the before view
/// and the crop tool — a variant of it. `None` is the current stack.
///
/// "Before" resets the creative edits but keeps the *framing* — crop, rotation,
/// straighten, flip and lens correction — so the before/after split lines up
/// instead of comparing a cropped frame against the whole uncropped original.
/// "Uncropped" is the edit with its crop released, for the crop tool to drag
/// the edges back out into — still turned, flipped and straightened,
/// so those controls show what they do while the tool is open. Before keeps the edit's version too: an edit made
/// before photos opened at the camera's brightness compares against the
/// photo as it opened then.
fn preview_stack(
    dev: &develop::DevelopState,
    before: bool,
    uncropped: bool,
    draft: bool,
) -> Result<Option<develop::EditStack>, String> {
    // The edit being dragged, when the URL asks for it and one is held.
    let draft = draft.then(|| dev.draft().cloned()).flatten();
    if before {
        let current = dev.current_stack().map_err(err)?;
        return Ok(Some(develop::EditStack {
            version: current.version,
            geometry: current.geometry,
            lens: current.lens,
            ..develop::EditStack::default()
        }));
    }
    if uncropped {
        let mut framed = match draft {
            Some(draft) => draft,
            None => dev.current_stack().map_err(err)?,
        };
        framed.geometry = develop::geometry::Geometry {
            crop: Default::default(),
            ..framed.geometry
        };
        return Ok(Some(framed));
    }
    Ok(draft)
}

/// Holds the edit being dragged for the `develop://` preview to draw,
/// and returns the number the preview URL asks for it by; `None` drops it.
///
/// Off the main thread: a drag calls it once per frame the preview can keep
/// up with, and it waits on the develop lock behind any render in flight.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn set_develop_draft(
    state: State<'_, AppState>,
    draft: Option<develop::EditStack>,
) -> Result<u32, String> {
    with_develop(&state, |dev| Ok(dev.set_draft(draft)))
}

/// The largest render the page may ask for, on either edge. The page only ever
/// sends sane sizes, but a size that arrives over IPC or a URL is clamped so a
/// crafted request can never ask the GPU for an enormous allocation.
const MAX_RENDER_EDGE: u32 = 8192;

/// One frame for the direct surface: where the canvas is, what it shows.
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectFrame {
    /// The canvas, in physical pixels relative to the webview's top left.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Output pixels per image pixel; `None` is fit-to-window.
    pub zoom: Option<f32>,
    /// Centre of the visible region, normalised against the framed photo.
    pub cx: f32,
    pub cy: f32,
    /// The unedited version, as the before view shows it.
    pub before: bool,
    /// The edit without its crop, for the crop tool.
    pub uncropped: bool,
    /// Mark colours the output profile cannot hold.
    pub soft_proof: bool,
    /// The split view: the unedited photo on the left half, the edit on the
    /// right.
    pub split: bool,
    /// The mask whose coverage to tint, while masking.
    #[serde(default)]
    pub show_mask: Option<u32>,
    /// The edit as the page has it mid-drag, to show without recording it —
    /// a slider commits to history only on release, and until now nothing
    /// was drawn before it did. `None` shows the current edit.
    pub draft: Option<develop::EditStack>,
}

/// Draws the open photo into the direct surface, positioned at the canvas.
///
/// Only meaningful with direct rendering on. The rectangle is in *physical*
/// pixels relative to the webview's top left: the page reports CSS pixels, and
/// on a scaled display the two differ by the device pixel ratio — getting that
/// wrong draws the photo at a fraction of the canvas, in a corner. `zoom`
/// `None` is fit-to-window.
///
/// Synchronous, so it runs on the main thread: moving the surface is a window
/// operation, and done there it lands in the same pass as the frame drawn into
/// it rather than a frame later.
#[tauri::command]
#[specta::specta]
pub(crate) fn render_direct(app: AppHandle, frame: DirectFrame) -> Result<(), String> {
    let started = std::time::Instant::now();
    let state = app.state::<AppState>();
    let width = frame.width.clamp(1, MAX_RENDER_EDGE);
    let height = frame.height.clamp(1, MAX_RENDER_EDGE);
    let view = frame
        .zoom
        .filter(|z| z.is_finite() && *z > 0.0)
        .map(|zoom| develop::geometry::ViewPort {
            zoom: zoom.min(64.0),
            centre: [frame.cx, frame.cy],
            width,
            height,
        });

    let drawn = with_develop(&state, |dev| {
        dev.place_target(frame.x, frame.y, width, height)
            .map_err(err)?;
        // The before view is the unedited photo, whatever is being dragged.
        let stack = match frame.draft {
            Some(draft) if !frame.before => {
                let mut draft = draft;
                if frame.uncropped {
                    // The crop released, the turn and straighten kept.
                    draft.geometry = develop::geometry::Geometry {
                        crop: Default::default(),
                        ..draft.geometry
                    };
                }
                Some(draft)
            }
            _ => preview_stack(dev, frame.before, frame.uncropped, false)?,
        };
        let stack = show_mask(
            dev,
            stack,
            if frame.before { None } else { frame.show_mask },
        )?;
        // The split's before keeps the framing of the edit beside it, as the
        // `develop://` before does, so the halves line up.
        let before = if frame.split && !frame.before {
            let shown = match &stack {
                Some(s) => s.clone(),
                None => dev.current_stack().map_err(err)?,
            };
            Some(develop::EditStack {
                version: shown.version,
                geometry: shown.geometry,
                lens: shown.lens,
                ..develop::EditStack::default()
            })
        } else {
            None
        };
        dev.render_direct(
            (width, height),
            stack.as_ref(),
            view,
            frame.soft_proof && !frame.before,
            before.as_ref(),
            frame.before,
        )
        .map_err(err)
    });
    trace_frame("direct", started);
    drawn
}

/// With `SOURCELIGHT_TRACE_FRAMES` set, prints how long each Develop frame
/// took on the Rust side, and how long since the one before — the way to see
/// what a release build keeps up with, which has no other logging. Read once.
pub(crate) fn trace_frame(what: &'static str, started: std::time::Instant) {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static LAST: std::sync::Mutex<Vec<(&str, std::time::Instant)>> =
        std::sync::Mutex::new(Vec::new());
    if !*TRACE.get_or_init(|| std::env::var_os("SOURCELIGHT_TRACE_FRAMES").is_some()) {
        return;
    }
    let now = std::time::Instant::now();
    let gap = {
        let mut last = sync::lock_recover(&LAST);
        match last.iter_mut().find(|(kind, _)| *kind == what) {
            Some((_, at)) => Some(now.duration_since(std::mem::replace(at, now))),
            None => {
                last.push((what, now));
                None
            }
        }
    };
    eprintln!(
        "frame {what}: {:.2} ms, {} since the last",
        started.elapsed().as_secs_f64() * 1000.0,
        gap.map_or("first".to_string(), |g| format!(
            "{:.1} ms",
            g.as_secs_f64() * 1000.0
        )),
    );
}

/// Serves the current develop preview over a `develop://` URI scheme.
///
/// The same trick as `thumb://`, for the same reason: a custom scheme moves
/// binary image data into the webview with no base64 inflation, and the browser
/// decodes and displays it natively. The frontend varies the `v` query
/// parameter so each edit is a distinct resource rather than a cache hit; the
/// value itself is not read here, since the handler always renders the current
/// state.
///
/// `?before=1` renders the stack's defaults instead, which is how the
/// before/after comparison works without disturbing history.
#[cfg(feature = "libraw")]
pub(crate) fn serve_develop(
    state: &AppState,
    uri: &tauri::http::Uri,
) -> tauri::http::Response<Vec<u8>> {
    // **A panic here must not escape.** This used to run inside a WebView2 COM
    // callback, where a Rust panic cannot unwind — it became
    // `panic_cannot_unwind` and took the whole app down with no usable error.
    // On the blocking pool it would instead drop the responder unanswered and
    // leave the preview waiting forever. wgpu panics on validation failures,
    // so anything touching the GPU from here is one mistake away from either.
    // Catching turns the worst case into a broken preview.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        serve_develop_inner(state, uri)
    }))
    .unwrap_or_else(|_| {
        tauri::http::Response::builder()
            .status(500)
            .header("content-type", "text/plain")
            .body(b"the renderer panicked; see the log".to_vec())
            .unwrap_or_default()
    })
}

#[cfg(feature = "libraw")]
fn serve_develop_inner(state: &AppState, uri: &tauri::http::Uri) -> tauri::http::Response<Vec<u8>> {
    let reply = |status: u16, body: &str| {
        tauri::http::Response::builder()
            .status(status)
            .header("content-type", "text/plain")
            .body(body.as_bytes().to_vec())
            .unwrap_or_default()
    };

    // Render sizes come straight off the URL. The frontend only ever sends sane
    // values, but the scheme handler must not trust that: every size is clamped
    // to `MAX_RENDER_EDGE`.
    let query = uri.query().unwrap_or_default();
    let before = query.split('&').any(|p| p == "before=1");
    let max_edge = query
        .split('&')
        .find_map(|p| p.strip_prefix("edge="))
        .and_then(|v| v.parse::<u32>().ok())
        .map(|e| e.min(MAX_RENDER_EDGE));

    // While the crop rectangle is being dragged the preview has to show the
    // *whole* frame — otherwise the photo outside the crop is already gone and
    // there is no way to see what enlarging it would bring back.
    let uncropped = query.split('&').any(|p| p == "uncropped=1");

    // Soft-proof (gamut check): paint colours the output profile can't hold.
    let soft_proof = query.split('&').any(|p| p == "proof=1");

    // The edit being dragged, rather than the committed one.
    let draft = query.split('&').any(|p| p.starts_with("draft="));

    let number = |key: &str| {
        query
            .split('&')
            .find_map(|p| p.strip_prefix(key))
            .and_then(|v| v.parse::<f32>().ok())
    };
    // A zoom means the UI is asking for a specific magnification into a
    // specific area, so it supplies the target size too. Without all of it the
    // request is a plain fit-to-window.
    let view = match (number("zoom="), number("vw="), number("vh=")) {
        (Some(zoom), Some(w), Some(h)) if zoom > 0.0 && w >= 1.0 && h >= 1.0 => {
            Some(develop::geometry::ViewPort {
                zoom,
                centre: [number("cx=").unwrap_or(0.5), number("cy=").unwrap_or(0.5)],
                width: (w as u32).min(MAX_RENDER_EDGE),
                height: (h as u32).min(MAX_RENDER_EDGE),
            })
        }
        _ => None,
    };

    // The mask whose coverage to tint.
    let shown_mask = query
        .split('&')
        .find_map(|p| p.strip_prefix("mask="))
        .and_then(|v| v.parse::<u32>().ok());

    let rendered = with_develop(state, |dev| {
        let stack = preview_stack(dev, before, uncropped, draft)?;
        let stack = show_mask(dev, stack, if before { None } else { shown_mask })?;
        match (before, stack) {
            // Over the photo as decoded, not the AI-denoised image.
            (true, Some(baseline)) => dev.render_before(max_edge, &baseline, view, soft_proof),
            (_, stack) => dev.render(max_edge, stack.as_ref(), view, soft_proof),
        }
        .map_err(err)
    });

    match rendered {
        Ok(presented) => match presented.bytes() {
            // The presenter declares its own format — it is the only thing that
            // knows what it encoded.
            Some(bytes) => tauri::http::Response::builder()
                .status(200)
                .header(
                    "content-type",
                    presented
                        .content_type()
                        .unwrap_or("application/octet-stream"),
                )
                // Never cached: the URL varies per edit, and a stale frame here
                // would look exactly like a broken slider.
                .header("cache-control", "no-store")
                .body(bytes.to_vec())
                .unwrap_or_default(),
            // A direct-surface presenter has already drawn; there is no image
            // to serve and the UI should not have asked.
            None => reply(204, "rendered directly to a surface"),
        },
        Err(e) => reply(500, &e),
    }
}

#[cfg(all(test, feature = "libraw"))]
mod tests {
    use super::*;
    use crate::raw;

    /// The `develop://` preview draws the edit being dragged when its URL
    /// asks for it and one is held, and the committed edit otherwise:
    /// the readback preview, Windows' default, used to show only the latter,
    /// so a slider redrew the photo on release. A real raw, as the app opens it.
    #[cfg(feature = "libraw")]
    #[test]
    fn the_preview_draws_the_draft_while_one_is_held() {
        use crate::raw::decoder::RawDecoder;
        let Some(dir) = crate::test_support::raw_dir() else {
            crate::test_support::skip(
                crate::test_support::Needs::Raw,
                "set SOURCELIGHT_TEST_RAW to a folder of raw files",
            );
            return;
        };
        let decoder = raw::libraw::LibRawDecoder::new();
        let exts = decoder.extensions();
        let Some(path) = crate::test_support::files_in(&dir, |e| exts.contains(&e))
            .into_iter()
            .next()
        else {
            return;
        };
        if crate::gpu::test_support::hardware_gpu().is_none() {
            return;
        }
        let mut dev =
            develop::DevelopState::new(Box::new(present::readback::ReadbackPresenter::new()))
                .unwrap();
        dev.open(1, &path, &decoder, None).unwrap();
        let mean = |dev: &mut develop::DevelopState, draft: bool| -> f64 {
            let stack = preview_stack(dev, false, false, draft).unwrap();
            let shown = dev.render(Some(256), stack.as_ref(), None, false).unwrap();
            let bytes = shown.bytes().expect("readback yields bytes");
            bytes.iter().map(|&b| b as f64).sum::<f64>() / bytes.len() as f64
        };
        let committed = mean(&mut dev, false);
        assert_eq!(
            mean(&mut dev, true),
            committed,
            "no draft held: the committed edit"
        );

        let brighter = develop::EditStack {
            exposure: 1.5,
            ..dev.current_stack().unwrap()
        };
        dev.set_draft(Some(brighter));
        let dragged = mean(&mut dev, true);
        assert!(dragged > committed + 10.0, "{dragged} vs {committed}");
        assert_eq!(
            mean(&mut dev, false),
            committed,
            "only when the URL asks for it"
        );

        dev.open(1, &path, &decoder, None).unwrap();
        assert!(dev.draft().is_none(), "dropped when a photo opens");
    }
}
