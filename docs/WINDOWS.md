# Windows

How SourceLight should feel on Windows, the counterpart of the Mac plan in
[`MACOS.md`](MACOS.md) ("Plan 2 — Feel like a Mac app"). Windows is the
platform the app was built on, so what is missing is not a port. It is the
conventions the Mac work named and Windows never got.

**The same principle: native chrome and behaviour; keep the instrument.**
`DESIGN.md`'s identity stays (IBM Plex, flat neutral greys). What gives a
webview app away on Windows is a pale title bar over a dark app, keys that
do nothing, and a touchpad that behaves like a mouse. It isn't the absence
of Fluent widgets. No Mica or acrylic, for the reason the Mac plan gives
for vibrancy: nothing tints the judgement of a photo.

## Tier 1 — window and keys (done)

1. **One surface at the top.** The title bar stays the system's. Its caption
   buttons, Snap Layouts on maximise, the system menu and Aero Snap all come
   with it, and a hand-drawn replacement loses some of them. What made it
   read as a browser was its colour. `window_chrome::match_title_bar` sets the
   caption, title text and window border (DWM, Windows 11) to the top bar's
   panel, ink and hairline colours; a test keeps them in step with
   `styles.css`. Windows 10 keeps its dark title bar from the window's
   `theme`.
2. **The keys people type without thinking.** A Mac's menu bar carries them;
   Windows has none, so they did nothing. They're now keymap actions,
   rebindable and listed in Shortcuts and the command palette. A Mac's menu
   consumes them first, so they don't double up there:
   - Ctrl+O: add a folder;
   - Ctrl+Shift+I: import from a card;
   - Ctrl+Shift+E: export the selection;
   - Ctrl+,: Settings;
   - Ctrl+Alt+A: Activity;
   - Ctrl+W: close Develop, the Lightbox or Compare;
   - F11: full screen.
3. **Remember the window.** Already done for both platforms
   (`tauri-plugin-window-state`).

## Tier 2 — behaviour

4. **Touchpad and wheel (done, both platforms in Develop).** A precision touchpad's two-finger scroll
   and a mouse notch both arrive as `wheel`, and the viewer zoomed 15% on
   every event, so a scroll over a photo zoomed it wildly. `lib/wheel.ts`
   tells them apart, as the system's own photo viewer does:
   - a notch zooms a step;
   - a pinch (Ctrl+wheel from Chromium) zooms smoothly;
   - a scroll pans a zoomed photo and leaves a fitted one alone.

   Develop's canvas and its mask overlays had the same problem on a Mac too
   (the Mac work fixed only the Lightbox and the loupe), so it's fixed there
   on both.
5. **Taskbar progress and attention (already done).** `lib/dockProgress.ts`
   drives Tauri's progress bar, which on Windows is the taskbar button's, and
   flashes the button when long work ends in the background.
6. **Drag photos out (already done).** The `drag` crate is cross-platform;
   dragging from the grid gives Explorer, mail or an editor the originals, as
   copies.
7. **Asking before quitting mid-export (already done).** The close button and
   Alt+F4 go through `quit-requested` (`lib/quitGuard.ts`).
8. **Right-click menus: open, needs a look.** The Mac shows a real `NSMenu`
   (`PhotoMenu.tsx`, `NATIVE`). On Windows Tauri's menus are Win32 popups,
   and whether they follow the app's dark theme on Windows 11 has to be seen
   in the app. A light menu over a dark app is worse than today's HTML one.
   Try it by widening `NATIVE` to Windows and right-clicking a photo.
9. **Confirmations: keep the in-app ones.** Windows' `MessageBox` and task
   dialogs are light-only, and in-app dialogs are the platform norm for a
   dark creative app.
10. **Scroll bars: open, needs a look.** Windows 11 draws thin overlay scroll
    bars that widen on hover. WebView2 can be asked for them
    (`--enable-features=msOverlayScrollbarWinStyle,msOverlayScrollbarWinStyleAnimation`
    in the window's `additionalBrowserArgs`, keeping Tauri's own
    `--disable-features=…` flags), after which `styles.css` should stop styling
    `::-webkit-scrollbar` on Windows as it already does on a Mac. Whether the
    flag still works in the shipped runtime has to be seen.

## Tier 3 — identity and integration

11. **Jump list.** Recent folders and "Import from card" on the taskbar
    button's right-click menu. Needs the app identity settled first (below).
12. **Notifications.** A toast when a long export or import ends in the
    background, as the Mac posts one. The taskbar flash is there already.
13. **Explorer.** "Open in SourceLight" on folders, and drop a folder on the
    taskbar button. Both need registration by the installer.
14. **Identity and signing.** The publisher and the MSIX route are still
    open.

## Not yet checked in the running app

- the title bar's colours after launch, maximised and restored, focused and
  not;
- F11 in and out of full screen;
- a precision-touchpad pinch and two-finger scroll in the Lightbox, Cull's
  loupe and Develop;
- items 8 and 10 above.
