//! A native surface hosted inside the app window, beneath the webview.
//!
//! The portable half of the arrangement. Everything above this file deals in a
//! rectangle and a wgpu target; everything below it is Win32 (a child window)
//! or AppKit (an `NSView` hosting a `CAMetalLayer`). Keeping the split here is
//! what stops the render graph and the develop session from ever learning
//! which platform they are running on — the rule stated in this module's
//! parent.

#[cfg(not(target_os = "macos"))]
use anyhow::bail;
use anyhow::Result;

/// The platform's handle on the surface: a window handle on Windows, the
/// view, layer and webview on macOS. Integers, so it is `Send`.
#[cfg(windows)]
type Native = isize;
#[cfg(target_os = "macos")]
type Native = super::appkit::Native;
#[cfg(not(any(windows, target_os = "macos")))]
type Native = ();

/// A surface of its own for the develop canvas.
///
/// Holds the native handle as integers rather than platform pointer types,
/// which is what makes it `Send` — a `Presenter` must be, and the presenter
/// owns one of these.
pub struct HostedSurface {
    native: Native,
    /// Last size passed to [`Self::place`], in physical pixels. The swapchain
    /// has to match it, and asking the window costs a round trip to the thread
    /// that owns it.
    size: (u32, u32),
}

impl HostedSurface {
    /// Creates the surface beneath the webview of `window`.
    ///
    /// **Must run on the main thread** — call it from inside
    /// `WebviewWindow::with_webview`, which runs there and hands over
    /// `webview`. A window belongs to its creating thread on Windows, and
    /// AppKit views may only be touched on the main one.
    pub fn new(
        window: &tauri::WebviewWindow,
        webview: &tauri::webview::PlatformWebview,
    ) -> Result<Self> {
        let native = Self::create(window, webview)?;
        Ok(Self {
            native,
            size: (1, 1),
        })
    }

    /// Moves it to `(x, y)` at `width` x `height`, in physical pixels relative
    /// to the app window's client area. Returns whether the size changed, which
    /// is the caller's cue to reconfigure the swapchain.
    pub fn place(&mut self, x: i32, y: i32, width: u32, height: u32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        Self::move_to(self.native, x, y, width, height);
        let resized = self.size != (width, height);
        self.size = (width, height);
        resized
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// What the window manager thinks this surface is — visibility, geometry
    /// and stacking, read back rather than assumed.
    pub fn describe(&self) -> String {
        #[cfg(windows)]
        return super::win32::describe(self.native);
        #[cfg(target_os = "macos")]
        return super::appkit::describe(self.native);
        #[cfg(not(any(windows, target_os = "macos")))]
        return "no hosted surface on this platform".into();
    }

    #[cfg(windows)]
    fn create(
        window: &tauri::WebviewWindow,
        _webview: &tauri::webview::PlatformWebview,
    ) -> Result<Native> {
        // Via wgpu's re-export, so there is exactly one `raw-window-handle` in
        // play — Tauri and wgpu must agree on the version or the traits are
        // different types entirely.
        use wgpu::rwh::HasWindowHandle;
        let handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("cannot read the window handle: {e}"))?;
        match handle.as_raw() {
            wgpu::rwh::RawWindowHandle::Win32(h) => super::win32::create(h.hwnd.get()),
            other => bail!("the main window is not a Win32 window: {other:?}"),
        }
    }

    #[cfg(target_os = "macos")]
    fn create(
        _window: &tauri::WebviewWindow,
        webview: &tauri::webview::PlatformWebview,
    ) -> Result<Native> {
        super::appkit::create(webview.inner())
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn create(
        _window: &tauri::WebviewWindow,
        _webview: &tauri::webview::PlatformWebview,
    ) -> Result<Native> {
        // Deliberately an error rather than a silent no-op: a caller that got a
        // `HostedSurface` back would configure a swapchain against nothing and
        // report success while showing an empty canvas.
        bail!("direct rendering is not implemented on this platform")
    }

    fn move_to(native: Native, x: i32, y: i32, width: u32, height: u32) {
        #[cfg(windows)]
        super::win32::place(native, x, y, width, height);
        #[cfg(target_os = "macos")]
        super::appkit::place(native, x, y, width, height);
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = (native, x, y, width, height);
    }

    /// The wgpu target for this surface.
    ///
    /// # Safety
    ///
    /// The returned target borrows the native window (or layer) by handle, and
    /// wgpu cannot check that it outlives the surface built from it. The
    /// presenter owns this `HostedSurface` alongside that surface, so they are
    /// dropped together — that ordering is the guarantee. (On macOS wgpu also
    /// retains the layer itself, so the view going first would be survivable
    /// there; on Windows it would not.)
    pub unsafe fn target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
        #[cfg(windows)]
        return super::win32::surface_target(self.native);
        #[cfg(target_os = "macos")]
        return Ok(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
            super::appkit::layer(&self.native),
        ));
        #[cfg(not(any(windows, target_os = "macos")))]
        bail!("direct rendering is not implemented on this platform")
    }
}

impl Drop for HostedSurface {
    fn drop(&mut self) {
        #[cfg(windows)]
        super::win32::destroy(self.native);
        #[cfg(target_os = "macos")]
        super::appkit::destroy(self.native);
    }
}
