//! The develop canvas's own Metal layer on macOS, beneath the webview.
//!
//! The Mac half of the arrangement `win32.rs` makes on Windows: a view of the
//! canvas's own, a sibling of the `WKWebView` ordered **below** it, so the page
//! can cut a transparent hole where the photo goes and the compositor shows the
//! layer through it. Clicks never reach it — the webview above covers the whole
//! window and takes every event, which is what keeps the crop handles, masks,
//! zoom and pan in React.
//!
//! **The view hosts a `CAMetalLayer` we create**, rather than letting wgpu make
//! one, for one reason: colour. An untagged Metal layer on a Display P3 panel
//! shows its sRGB values as if they were P3 — oversaturated, and different from
//! the readback path, which WebKit treats as sRGB. Tagging the layer sRGB makes
//! the system convert, so both paths show the same colours. (Tagging it P3, with
//! the shader writing P3, is how the preview could one day show the gamut a
//! Display P3 export already holds.)
//!
//! **Threads.** AppKit views may only be touched on the main thread. Creation is
//! dispatched there by the caller; moving and tearing down go through
//! [`on_main`], which runs inline on the main thread and otherwise queues the
//! work there *without waiting*. Never waiting is the point: the presenter is
//! called with the develop lock held, and a main thread blocked on that lock
//! would never run a synchronous dispatch — a deadlock.
//!
//! As in `win32.rs`, everything crosses the boundary as `isize`, so the
//! presenter holding it stays `Send`.

use std::ffi::c_void;

use anyhow::{bail, Result};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

// SAFETY: the encodings are the ones Core Graphics declares for these structs
// on a 64-bit Mac, where `CGFloat` is a double.
unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
// SAFETY: as above.
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
// SAFETY: as above.
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

/// `CGColorSpaceRef`'s pointee, for the encoding `setColorspace:` expects.
#[repr(C)]
struct CGColorSpace {
    _private: [u8; 0],
}
// SAFETY: an opaque Core Graphics type, encoded as the empty struct its header
// declares.
unsafe impl RefEncode for CGColorSpace {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColorSpace", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    static kCGColorSpaceSRGB: *const c_void;
    // Declared as `raster::imageio` declares it: one signature per symbol.
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *const c_void;
    fn CGColorSpaceRelease(space: *const c_void);
}

#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

/// Surfaces made and not yet torn down. Only the main thread touches it.
///
/// Turning direct rendering off and straight back on — React mounting
/// Develop twice in development does exactly that — creates the second
/// surface before the first one's queued teardown runs. That teardown must not
/// give the webview its background back while the second is still using it.
static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `NSWindowBelow`, as `addSubview:positioned:relativeTo:` takes it.
const NS_WINDOW_BELOW: isize = -1;

/// The pieces of one hosted layer. Pointers as integers, so this is `Send`.
#[derive(Clone, Copy, Debug)]
pub struct Native {
    /// The `NSView` hosting the layer. Retained here.
    view: isize,
    /// The `CAMetalLayer`. Retained here, and again by the view and by wgpu.
    layer: isize,
    /// The `WKWebView` the view sits beneath. Not retained: it lives as long
    /// as the window, which outlives any develop session.
    webview: isize,
}

fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass> {
    AnyClass::get(name).ok_or_else(|| anyhow::anyhow!("no {name:?} class"))
}

fn is_main_thread() -> bool {
    // SAFETY: no preconditions.
    unsafe { libc::pthread_main_np() != 0 }
}

/// Runs `work` on the main thread: now, if this is it, otherwise queued there
/// without waiting. See the module comment for why it never waits.
fn on_main(work: impl FnOnce() + Send + 'static) {
    if is_main_thread() {
        work();
        return;
    }
    extern "C" fn trampoline(context: *mut c_void) {
        // SAFETY: `context` is the box leaked below, taken back exactly once.
        let work = unsafe { Box::from_raw(context as *mut Box<dyn FnOnce() + Send>) };
        work();
    }
    let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(work));
    // SAFETY: the main queue is a process-lifetime static; the context is
    // reclaimed by the trampoline.
    unsafe {
        dispatch_async_f(
            &_dispatch_main_q as *const c_void,
            Box::into_raw(boxed) as *mut c_void,
            trampoline,
        )
    };
}

/// Sets `drawsBackground` on the webview. Off, the page's transparent parts
/// show what is beneath the webview; on is WebKit's default.
///
/// Key-value coding rather than a public setter, because macOS's `WKWebView`
/// has none — it is the same switch Tauri's own transparent windows use.
///
/// # Safety
///
/// `webview` must be a live `WKWebView`, and this called on the main thread.
unsafe fn set_draws_background(webview: *mut AnyObject, draws: bool) {
    let Ok(number) = class(c"NSNumber") else {
        return;
    };
    let Ok(string) = class(c"NSString") else {
        return;
    };
    let value: *mut AnyObject = msg_send![number, numberWithBool: Bool::new(draws)];
    let key: *mut AnyObject = msg_send![string, stringWithUTF8String: c"drawsBackground".as_ptr()];
    if !value.is_null() && !key.is_null() {
        let _: () = msg_send![webview, setValue: value, forKey: key];
    }
}

/// Creates the view and its layer beneath `webview` (a `WKWebView *`).
/// **Must be called on the main thread.**
pub fn create(webview: *mut c_void) -> Result<Native> {
    if !is_main_thread() {
        bail!("the render layer has to be made on the main thread");
    }
    let webview = webview as *mut AnyObject;
    if webview.is_null() {
        bail!("the window has no webview");
    }
    // SAFETY: each message is sent to a live object of the class that declares
    // it, with arguments of the declared types; the two objects created here
    // are owned by the returned `Native` and released in `destroy`.
    unsafe {
        let superview: *mut AnyObject = msg_send![webview, superview];
        let window: *mut AnyObject = msg_send![webview, window];
        if superview.is_null() || window.is_null() {
            bail!("the webview is not in a window yet");
        }
        let scale: f64 = msg_send![window, backingScaleFactor];

        let layer: *mut AnyObject = msg_send![class(c"CAMetalLayer")?, new];
        if layer.is_null() {
            bail!("could not create a Metal layer");
        }
        let _: () = msg_send![layer, setOpaque: Bool::YES];
        let _: () = msg_send![layer, setContentsScale: scale];
        let srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        if !srgb.is_null() {
            let _: () = msg_send![layer, setColorspace: srgb as *mut CGColorSpace];
            CGColorSpaceRelease(srgb);
        }

        let view: *mut AnyObject = msg_send![class(c"NSView")?, alloc];
        let view: *mut AnyObject = msg_send![view, initWithFrame: CGRect::default()];
        if view.is_null() {
            let _: () = msg_send![layer, release];
            bail!("could not create the render view");
        }
        // Layer-hosting: the layer is set before `wantsLayer`, so AppKit uses
        // ours instead of making its own and never draws into it.
        let _: () = msg_send![view, setLayer: layer];
        let _: () = msg_send![view, setWantsLayer: Bool::YES];
        let _: () = msg_send![
            superview,
            addSubview: view,
            positioned: NS_WINDOW_BELOW,
            relativeTo: webview
        ];
        set_draws_background(webview, false);
        LIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(Native {
            view: view as isize,
            layer: layer as isize,
            webview: webview as isize,
        })
    }
}

/// The layer, for wgpu to build a swapchain on.
pub fn layer(native: &Native) -> *mut c_void {
    native.layer as *mut c_void
}

/// Moves and sizes the view to `(x, y, width, height)` in physical pixels
/// relative to the webview's top left — the rectangle the page measured.
pub fn place(native: Native, x: i32, y: i32, width: u32, height: u32) {
    on_main(move || {
        // SAFETY: main thread; the objects are alive until `destroy`, which
        // is queued behind this on the same thread.
        unsafe {
            let view = native.view as *mut AnyObject;
            let layer = native.layer as *mut AnyObject;
            let webview = native.webview as *mut AnyObject;
            let superview: *mut AnyObject = msg_send![view, superview];
            let window: *mut AnyObject = msg_send![webview, window];
            if superview.is_null() || window.is_null() {
                return;
            }
            // The page measures in CSS pixels times `devicePixelRatio`, which
            // is the window's backing scale; the view is laid out in points.
            let scale: f64 = msg_send![window, backingScaleFactor];
            let mut rect = CGRect {
                origin: CGPoint {
                    x: x as f64 / scale,
                    y: y as f64 / scale,
                },
                size: CGSize {
                    width: width.max(1) as f64 / scale,
                    height: height.max(1) as f64 / scale,
                },
            };
            // The page's origin is its top left. A web view is flipped, so its
            // own coordinates already agree — but only rely on that if it says
            // so; AppKit's default is bottom-left.
            let flipped: Bool = msg_send![webview, isFlipped];
            if !flipped.as_bool() {
                let bounds: CGRect = msg_send![webview, bounds];
                rect.origin.y = bounds.size.height - rect.origin.y - rect.size.height;
            }
            let rect: CGRect = msg_send![webview, convertRect: rect, toView: superview];

            // No implicit animation: a layer's frame change animates over a
            // quarter of a second by default, which would make the photo
            // swim after every panel resize.
            let transaction = class(c"CATransaction");
            if let Ok(t) = transaction {
                let _: () = msg_send![t, begin];
                let _: () = msg_send![t, setDisableActions: Bool::YES];
            }
            let _: () = msg_send![view, setFrame: rect];
            let _: () = msg_send![layer, setContentsScale: scale];
            if let Ok(t) = transaction {
                let _: () = msg_send![t, commit];
            }
        }
    });
}

/// Removes the view, releases what `create` made, and gives the webview its
/// background back.
pub fn destroy(native: Native) {
    on_main(move || {
        // SAFETY: main thread; the view and layer were retained by `create`
        // and are released here once. wgpu holds its own reference to the
        // layer, so a surface still being dropped elsewhere stays valid.
        unsafe {
            let view = native.view as *mut AnyObject;
            let layer = native.layer as *mut AnyObject;
            let _: () = msg_send![view, removeFromSuperview];
            let _: () = msg_send![view, release];
            let _: () = msg_send![layer, release];
            if LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) == 1 {
                set_draws_background(native.webview as *mut AnyObject, true);
            }
        }
    });
}

/// Where AppKit has the view, for the diagnostic probe.
pub fn describe(native: Native) -> String {
    if !is_main_thread() {
        return format!(
            "layer view {:#x} (not on the main thread to read it)",
            native.view
        );
    }
    // SAFETY: main thread, read-only queries on live objects.
    unsafe {
        let view = native.view as *mut AnyObject;
        let frame: CGRect = msg_send![view, frame];
        let hidden: Bool = msg_send![view, isHidden];
        let superview: *mut AnyObject = msg_send![view, superview];
        let order = if superview.is_null() {
            "not in a window".to_string()
        } else {
            let subviews: *mut AnyObject = msg_send![superview, subviews];
            let below: usize = msg_send![subviews, indexOfObject: view];
            let webview: usize =
                msg_send![subviews, indexOfObject: native.webview as *mut AnyObject];
            if below < webview {
                "beneath the webview".to_string()
            } else {
                "ABOVE the webview".to_string()
            }
        };
        format!(
            "layer view at {:.0},{:.0} {:.0}x{:.0} pt, {}, {order}",
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
            if hidden.as_bool() {
                "HIDDEN"
            } else {
                "visible"
            },
        )
    }
}
