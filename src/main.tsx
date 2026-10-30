import React, { useSyncExternalStore } from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import ErrorBoundary from "./components/ErrorBoundary";
import { reportUiError } from "./lib/errorLog";
import { watchWindowChrome } from "./lib/platform";
import { installAppMenu } from "./lib/appMenu";
import { installDockProgress } from "./lib/dockProgress";
import { installQuitGuard } from "./lib/quitGuard";
import { installFolderDrop } from "./lib/folderDrop";
import { onLocaleChange, resolveLocale, setLocale, textVersion } from "./i18n";
import { useStore } from "./store";
import "./styles.css";

// Anything that escapes a component boundary — a listener, a timer, a rejected
// promise nobody awaited — would otherwise reach only the devtools console,
// which a user does not have open and cannot be asked to open. So it is also
// written to the log folder beside the crash logs, where Settings →
// Advanced can open it.
window.addEventListener("error", (e) => {
  console.error("uncaught error", e.error ?? e.message);
  reportUiError("window", e.error ?? e.message);
});
window.addEventListener("unhandledrejection", (e) => {
  console.error("unhandled promise rejection", e.reason);
  reportUiError("unhandled promise", e.reason);
});

watchWindowChrome();
installAppMenu();
installDockProgress();
installQuitGuard();
installFolderDrop();

/**
 * Re-renders the whole tree when the language changes. Nothing in it is
 * memoised, so every component calls `t` again and picks up the new words —
 * no provider, and no hook for each component to remember (see `i18n`).
 */
function LocaleRoot() {
  useSyncExternalStore(onLocaleChange, textVersion);
  return (
    // The last resort. App has finer-grained boundaries around the workspaces
    // and each overlay, so this only catches a throw in the shell itself — but
    // without it, that throw is a black window.
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  );
}

// "Use the system language" follows the system if it changes while the app runs.
window.addEventListener("languagechange", () => {
  const pref = useStore.getState().language;
  if (pref === "system") void setLocale(resolveLocale(pref));
});

async function mount() {
  // The catalogue loads before the first render, so the window never shows
  // English and then swaps. The persisted store has hydrated by now:
  // localStorage is synchronous.
  await setLocale(resolveLocale(useStore.getState().language));
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <LocaleRoot />
    </React.StrictMode>,
  );
}

// The window starts hidden (tauri.conf.json) so a launch never shows an empty
// frame while the web view starts: show it once the first frame has painted.
// The backend shows it anyway after a few seconds, in case this never runs.
function showWindow() {
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      getCurrentWindow()
        .show()
        .catch(() => {});
    }),
  );
}

// In a plain browser during development there is no Tauri runtime to answer
// the app's commands; stand in a fake library so the UI can still be looked
// at (see dev/mockBackend). Never in a production build, and never inside
// `tauri dev`, where the real backend is present.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  void import("./dev/mockBackend").then(({ installMockBackend }) => {
    installMockBackend();
    void mount();
  });
} else {
  void mount().then(() => {
    if ("__TAURI_INTERNALS__" in window) showWindow();
  });
}
