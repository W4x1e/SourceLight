//! The develop canvas's own window, sitting beneath the webview.
//!
//! **Why a separate window at all.** The first attempt put the swapchain on the
//! Tauri window itself, which renders nothing: Windows clips a parent's drawing
//! to exclude its child windows, and WebView2 in window-hosting mode *is* a
//! child covering the whole client area. The parent's pixels in that region are
//! never composited. So the surface needs a window of its own, a sibling of the
//! webview, ordered underneath it — then the two are composited by the window
//! manager and a transparent webview lets the photo through.
//!
//! Everything here is raw handles and integers rather than a struct holding a
//! `HWND`, for two reasons: `HWND` is a raw pointer and therefore not `Send`,
//! and a `Presenter` must be — and keeping the boundary at `isize` means the
//! portable half in `host.rs` needs no `unsafe impl`.
//!
//! **Thread affinity is the trap.** A window belongs to the thread that created
//! it. `SetWindowPos` from another thread is fine as long as that thread pumps
//! messages, which the main thread does; `DestroyWindow` from another thread
//! silently fails. So creation is dispatched to the main thread by the caller,
//! and teardown goes through `PostMessageW`, which is explicitly thread-safe.

use std::num::NonZeroIsize;
use std::sync::Once;

use anyhow::{bail, Result};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindow, GetWindowRect,
    IsWindowVisible, PostMessageW, RegisterClassW, SetWindowPos, GW_HWNDPREV, HTTRANSPARENT,
    HWND_BOTTOM, SWP_ASYNCWINDOWPOS, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, WM_CLOSE, WM_ERASEBKGND, WM_NCHITTEST, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_EX_NOPARENTNOTIFY, WS_EX_TRANSPARENT, WS_VISIBLE,
};

/// Class name, as a NUL-terminated wide string.
fn class_name() -> Vec<u16> {
    "PhotoOrganiserDevelopSurface"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

/// The child window's message handler.
///
/// # Safety
///
/// Called only by USER32, with a window of this class and its message.
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        // The swapchain writes every pixel of this window, so letting USER32
        // erase first buys a flash of background on every move and resize.
        // Saying "already erased" is what makes the child flicker-free.
        WM_ERASEBKGND => 1,
        // Clicks belong to the webview above: the crop handles, zoom and pan
        // are all React. A window that swallowed them would make the photo
        // area inert without any visible reason why.
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        // Teardown arrives as a posted message because the presenter is
        // dropped on whichever thread turned direct rendering off, and
        // `DestroyWindow` only works from the owning thread.
        WM_CLOSE => {
            // SAFETY: on the window's own thread, which is where USER32 calls
            // this, with the handle it was given.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        // SAFETY: passes USER32's own arguments straight back to it.
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

/// Creates the child window. **Must be called on the thread owning `parent`.**
///
/// Returns the new window's handle as an `isize`.
pub fn create(parent: isize) -> Result<isize> {
    let Some(parent) = NonZeroIsize::new(parent) else {
        bail!("the main window has no Win32 handle");
    };
    let name = class_name();

    // SAFETY: every call below is a documented USER32 entry point given
    // well-formed arguments; the pointers are either null or into `name`,
    // which outlives the call.
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());

        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let class = WNDCLASSW {
                // No CS_HREDRAW/CS_VREDRAW: those invalidate the whole window
                // on every resize, and there is nothing to repaint — the next
                // present replaces the contents wholesale.
                style: 0,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                // Null, not a stock brush. A background brush would paint over
                // the presented frame between draws.
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: name.as_ptr(),
            };
            RegisterClassW(&class);
        });

        let hwnd = CreateWindowExW(
            // TRANSPARENT keeps it out of hit testing entirely, belt to the
            // WM_NCHITTEST braces; NOPARENTNOTIFY spares the Tauri window a
            // stream of notifications about a child it does not manage.
            WS_EX_NOPARENTNOTIFY | WS_EX_TRANSPARENT,
            name.as_ptr(),
            std::ptr::null(),
            // CLIPSIBLINGS is the one that matters: without it this window and
            // the webview would each draw over the other's region and which
            // one you saw would depend on paint order.
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            parent.get() as HWND,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            bail!("could not create the render window");
        }

        // Bottom of the sibling order, so the webview is composited over it.
        // The reverse shows a photo sitting on top of the panels.
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        Ok(hwnd as isize)
    }
}

/// Moves and resizes the child, in physical pixels relative to the parent.
pub fn place(hwnd: isize, x: i32, y: i32, width: u32, height: u32) {
    let Some(hwnd) = NonZeroIsize::new(hwnd) else {
        return;
    };
    // SAFETY: `SetWindowPos` is documented as callable from any thread; the
    // owning thread is the main one and pumps messages, so this does not hang.
    unsafe {
        SetWindowPos(
            hwnd.get() as HWND,
            std::ptr::null_mut(),
            x,
            y,
            width.max(1) as i32,
            height.max(1) as i32,
            // Z-order was set at creation and must not be disturbed here: a
            // reorder on every frame would flicker the window above the panels.
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Hides the child immediately and asks its owning thread to destroy it.
///
/// Two steps because they answer different needs: the hide is synchronous, so
/// the stale photo disappears the moment direct rendering is switched off, and
/// the posted `WM_CLOSE` is what actually frees the window on the thread that
/// created it.
pub fn destroy(hwnd: isize) {
    let Some(hwnd) = NonZeroIsize::new(hwnd) else {
        return;
    };
    // SAFETY: both entry points are documented thread-safe; a handle that has
    // already gone makes them fail rather than misbehave.
    unsafe {
        let hwnd = hwnd.get() as HWND;
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            // Asynchronous: from another thread a plain SetWindowPos
            // waits for the main thread to handle it, and a main thread
            // waiting on the develop lock never would.
            SWP_ASYNCWINDOWPOS
                | SWP_HIDEWINDOW
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOACTIVATE,
        );
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
}

/// What Windows thinks of this window, for the diagnostic probe.
///
/// Every fact here is one I asserted while reasoning about why nothing appears
/// and never actually read back: that the window is visible, that `place` moved
/// it where the canvas is, and that it sits underneath the webview rather than
/// above it or in front of nothing. Two rounds of guessing is enough.
pub fn describe(hwnd: isize) -> String {
    let Some(hwnd) = NonZeroIsize::new(hwnd) else {
        return "no window".into();
    };
    // SAFETY: all four are read-only queries on a handle we own, and each
    // reports failure rather than misbehaving if the window has gone.
    unsafe {
        let hwnd = hwnd.get() as HWND;
        let mut window = RECT::default();
        let mut client = RECT::default();
        GetWindowRect(hwnd, &mut window);
        GetClientRect(hwnd, &mut client);

        // How many siblings sit above it. **One** is what we want: the webview,
        // composited over this window. Zero would mean the z-order call did not
        // stick and the photo is sitting on top of the panels.
        let mut above = 0;
        let mut prev = GetWindow(hwnd, GW_HWNDPREV);
        while !prev.is_null() && above < 64 {
            above += 1;
            prev = GetWindow(prev, GW_HWNDPREV);
        }

        format!(
            "hwnd {:#x}, {}, screen {},{} {}x{}, client {}x{}, {above} sibling(s) above",
            hwnd as usize,
            if IsWindowVisible(hwnd) != 0 {
                "visible"
            } else {
                "HIDDEN"
            },
            window.left,
            window.top,
            window.right - window.left,
            window.bottom - window.top,
            client.right - client.left,
            client.bottom - client.top,
        )
    }
}

/// The wgpu target for this window.
///
/// `RawHandle` directly rather than `SurfaceTargetUnsafe::from_window`: that
/// helper leaves the display handle `None` and wgpu then refuses with
/// `MissingDisplayHandle`, having no handle on the instance either. Windows has
/// exactly one display connection, so the handle is a unit value — it just has
/// to be present.
pub fn surface_target(hwnd: isize) -> Result<wgpu::SurfaceTargetUnsafe> {
    let Some(handle) = NonZeroIsize::new(hwnd) else {
        bail!("the render window has no handle");
    };
    let mut window = wgpu::rwh::Win32WindowHandle::new(handle);
    // SAFETY: reading the module handle has no preconditions.
    window.hinstance = NonZeroIsize::new(unsafe { GetModuleHandleW(std::ptr::null()) } as isize);

    Ok(wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::Windows(
            wgpu::rwh::WindowsDisplayHandle::new(),
        )),
        raw_window_handle: wgpu::rwh::RawWindowHandle::Win32(window),
    })
}
