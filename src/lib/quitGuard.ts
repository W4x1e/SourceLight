import { listen } from "@tauri-apps/api/event";
import { isDownload } from "./jobs";
import { humanBytes } from "./format";
import { useStore } from "../store";
import { api } from "./api";
import { confirm, type ConfirmOptions } from "./dialogs";
import { t } from "../i18n";

/**
 * The question to ask before quitting, or `null` to quit without one.
 *
 * Only work that quitting would leave half-done: an export (the photos not yet
 * written are simply not exported) and a card import (the rest of the card is
 * not copied). Scans and analysis pick up where they stopped on the
 * next launch, so they do not hold anyone up — a Mac app that nags about
 * background work it will finish anyway is its own kind of annoying.
 */
export function unfinishedWork(s: ReturnType<typeof useStore.getState>): ConfirmOptions | null {
  const exporting = s.exportJob?.running ? s.exportJob : null;
  if (exporting) {
    const left = exporting.request.files.length - exporting.done;
    return {
      title: t("shell.quit.exportTitle"),
      message: t("shell.quit.exportMessage", { left, count: exporting.request.files.length }),
      confirmLabel: t("shell.quit.confirm"),
      cancelLabel: t("shell.quit.keepExporting"),
      danger: true,
    };
  }
  // A download starts again from the beginning next time: it used to
  // be counted among the things that pick up where they stopped.
  const downloading = Object.entries(s.jobs).find(([id, j]) => isDownload(id) && !j.error)?.[1];
  if (downloading) {
    return {
      title: t("shell.quit.downloadTitle"),
      message: t("shell.quit.downloadMessage", {
        current: humanBytes(downloading.current),
        total: humanBytes(downloading.total),
      }),
      confirmLabel: t("shell.quit.confirm"),
      cancelLabel: t("shell.quit.keepDownloading"),
      danger: true,
    };
  }
  const importing = s.jobs["import"];
  if (importing) {
    return {
      title: t("shell.quit.importTitle"),
      message: t("shell.quit.importMessage", { current: importing.current, count: importing.total }),
      confirmLabel: t("shell.quit.confirm"),
      cancelLabel: t("shell.quit.keepImporting"),
      danger: true,
    };
  }
  return null;
}

/** Quits now, or asks first if that would cut work off. */
export async function requestQuit(): Promise<void> {
  const question = unfinishedWork(useStore.getState());
  if (!question || (await confirm(question))) void api.quitApp();
  else void api.stayOpen();
}

/**
 * Answers the backend's `quit-requested`, which it sends instead of quitting
 * when the window's close button is pressed, or an exit is asked for another
 * way the runtime reports (`hold_quit_for_the_ui` in commands/app.rs). On a Mac, ⌘Q is
 * the menu's own Quit item, which calls `requestQuit` directly: AppKit's
 * standard Quit terminates without asking the runtime, so it could not be held.
 */
export function installQuitGuard(): void {
  if (!("__TAURI_INTERNALS__" in window)) return;
  void listen("quit-requested", () => void requestQuit());
}
