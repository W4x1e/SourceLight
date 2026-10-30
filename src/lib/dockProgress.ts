import { getCurrentWindow, ProgressBarStatus, UserAttentionType } from "@tauri-apps/api/window";
import { useStore } from "../store";

/** Everything in the store that is running now, as fractions done (NaN = unknown). */
export function runningFractions(s: ReturnType<typeof useStore.getState>): number[] {
  const out: number[] = [];
  const add = (current: number, total: number) => out.push(total > 0 ? Math.min(1, current / total) : NaN);
  if (s.scanning) add(s.scanning.current, s.scanning.total);
  if (s.tagging) add(s.tagging.current, s.tagging.total);
  if (s.exportJob?.running) add(s.exportJob.done, s.exportJob.request.files.length);
  for (const job of Object.values(s.jobs)) add(job.current, job.total);
  return out;
}

/**
 * The share of the running work that is done, 0-100, or `null` when nothing
 * runs. Jobs count equally rather than by size: a scan of 70 photos and a 785
 * MB download are not in the same units, and the bar only has to say "still
 * going, about this far". Jobs with no total yet count as not started.
 */
export function overallPercent(fractions: number[]): number | null {
  if (fractions.length === 0) return null;
  const done = fractions.reduce((sum, f) => sum + (Number.isNaN(f) ? 0 : f), 0);
  return Math.round((done / fractions.length) * 100);
}

/** Work shorter than this finishes before anyone has looked away. */
export const ANNOUNCE_AFTER_MS = 20_000;

/**
 * Whether finishing should ask for attention: only work that ran long enough
 * for someone to switch to another app, and only when they did. On a Mac the
 * Dock icon bounces once; on Windows the taskbar button flashes.
 */
export function shouldAnnounceFinish(ranForMs: number, focused: boolean): boolean {
  return !focused && ranForMs >= ANNOUNCE_AFTER_MS;
}

/**
 * Background work on the Dock tile (the taskbar button on Windows): a model
 * download, an analysis pass, a scan or an export shows as a progress bar on
 * the app's icon, so it can be followed with the window behind others — the
 * way Finder shows a copy. The Activity panel stays the detailed view. When a
 * long run finishes while the app is in the background, the icon asks for
 * attention once (`shouldAnnounceFinish`).
 */
export function installDockProgress(): void {
  if (!("__TAURI_INTERNALS__" in window)) return;
  const win = getCurrentWindow();
  let shown: number | null = null;
  let startedAt: number | null = null;
  const update = () => {
    const percent = overallPercent(runningFractions(useStore.getState()));
    if (percent === shown) return;
    if (shown === null && percent !== null) startedAt = Date.now();
    if (shown !== null && percent === null && startedAt !== null) {
      if (shouldAnnounceFinish(Date.now() - startedAt, document.hasFocus())) {
        void win.requestUserAttention(UserAttentionType.Informational).catch(() => undefined);
      }
      startedAt = null;
    }
    shown = percent;
    void win
      .setProgressBar(
        percent === null
          ? { status: ProgressBarStatus.None }
          : { status: ProgressBarStatus.Normal, progress: percent },
      )
      .catch(() => undefined);
  };
  useStore.subscribe(update);
  update();
}
