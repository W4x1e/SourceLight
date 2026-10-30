/**
 * Full screen: in the app, the window; in a browser, the page. Shared by the
 * palette's toggle and the slideshow, which goes full screen and puts
 * things back as they were.
 */

/** The app's window, or null outside the app — including the browser
 *  preview, whose mock backend looks like the app but has no window. */
async function appWindow() {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return null;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return getCurrentWindow();
  } catch {
    return null;
  }
}

export async function isFullscreen(): Promise<boolean> {
  const win = await appWindow();
  if (win) return win.isFullscreen().catch(() => false);
  return !!document.fullscreenElement;
}

export async function setFullscreen(on: boolean): Promise<void> {
  const win = await appWindow();
  if (win) {
    await win.setFullscreen(on).catch(() => undefined);
  } else if (on && !document.fullscreenElement) {
    await document.documentElement.requestFullscreen().catch(() => undefined);
  } else if (!on && document.fullscreenElement) {
    await document.exitFullscreen().catch(() => undefined);
  }
}

export async function toggleFullscreen(): Promise<void> {
  await setFullscreen(!(await isFullscreen()));
}
