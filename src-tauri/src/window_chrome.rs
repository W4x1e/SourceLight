//! The window's own chrome in the app's colours.
//!
//! On Windows the title bar is the system's, and it should stay so: its
//! caption buttons, Snap Layouts on the maximise button, the system menu and
//! drag-to-dock all come with it, and a hand-drawn replacement loses some of
//! them. What made it look like a browser window was only its colour: a
//! grey or white strip above the app's own dark bar. Windows 11 lets an app
//! set the caption, its text and the window border, so the title bar takes
//! the top bar's panel colour and reads as one surface with it, the Windows
//! counterpart of the Mac's single bar (`tauri.macos.conf.json`).
//!
//! Windows 10 ignores the colour attributes and keeps the dark title bar the
//! window's `theme` already asks for.

/// The top bar's background, `--color-panel` in styles.css.
pub const PANEL: u32 = 0x1b1b1b;
/// The title's text, `--color-ink`.
pub const INK: u32 = 0xececec;
/// The window's hairline border, `--color-line`.
pub const LINE: u32 = 0x262626;

/// A `0xRRGGBB` colour as the `COLORREF` (`0x00BBGGRR`) Win32 wants.
pub const fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff)
}

/// Paints the title bar of the window `hwnd` in the app's colours. Failures
/// are ignored: an older Windows keeps its own title bar, which works.
#[cfg(windows)]
///
/// `hwnd` is the window handle as a number, as Tauri hands it over: DWM only
/// reads it, and only while the window exists.
pub fn match_title_bar(hwnd: isize) {
    let hwnd = hwnd as *mut core::ffi::c_void;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    // The attribute constants are the enum's i32; the call takes its u32.
    // SAFETY: DWM reads the one `u32` it is told of from `value`, which
    // outlives the call, and answers a stale handle with an error.
    let set = |attr: i32, value: u32| unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attr as u32,
            &value as *const u32 as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    set(DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
    set(DWMWA_CAPTION_COLOR, colorref(PANEL));
    set(DWMWA_TEXT_COLOR, colorref(INK));
    set(DWMWA_BORDER_COLOR, colorref(LINE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_turned_round_for_win32() {
        assert_eq!(colorref(0x123456), 0x563412);
        assert_eq!(
            colorref(PANEL),
            0x1b1b1b,
            "a grey reads the same either way"
        );
    }

    /// The constants follow the stylesheet: a change to the panel colour there
    /// that isn't made here would leave a seam above the top bar.
    #[test]
    fn the_colours_match_the_stylesheet() {
        let css = include_str!("../../src/styles.css");
        for (token, value) in [
            ("--color-panel", PANEL),
            ("--color-ink", INK),
            ("--color-line", LINE),
        ] {
            let want = format!("{token}: #{value:06x};");
            assert!(css.contains(&want), "styles.css should say `{want}`");
        }
    }
}
