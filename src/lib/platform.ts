import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../i18n";

/**
 * The operating system's own names for the places the app hands files to —
 * the names people know them by. Windows is the default, and the tests' jsdom,
 * which is neither, reads as Windows.
 */
export const IS_MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

/** "Explorer" on Windows, "Finder" on a Mac. */
export const FILE_MANAGER = IS_MAC ? "Finder" : "Explorer";

/**
 * "Recycle Bin" on Windows, "Trash" on a Mac. `trash` puts files in either.
 * The system's own name for it in the user's language, so it is read when shown.
 */
export const trashName = () => t(IS_MAC ? "shell.platform.trashMac" : "shell.platform.trashWindows");

/**
 * On a Mac the window has no title bar of its own (`titleBarStyle: Overlay` in
 * tauri.macos.conf.json): the app's top bars are the title bar, and the
 * traffic lights float over the window's top-left corner. `html.mac` lets the
 * stylesheet leave room for them; `html.fullscreen` takes the room back while
 * full screen hides them. Only in the app — a browser (the mock backend's
 * preview) has neither traffic lights nor a window to ask.
 */
export function watchWindowChrome(): void {
  if (!IS_MAC || !("__TAURI_INTERNALS__" in window)) return;
  const root = document.documentElement;
  root.classList.add("mac");
  const win = getCurrentWindow();
  const sync = () =>
    void win
      .isFullscreen()
      .then((full) => root.classList.toggle("fullscreen", full))
      .catch(() => undefined);
  sync();
  void win.onResized(sync);
}
