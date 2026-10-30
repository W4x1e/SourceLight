/**
 * How long AI noise reduction takes on this computer.
 *
 * The same photo takes 10 s on a fast graphics card and many minutes on
 * integrated graphics, so a fixed promise is wrong for most people. Instead
 * the panel measures: during a run it says how long is left, from the pace so
 * far; afterwards it remembers seconds per megapixel, so the next photo's
 * button can say "About 12 s on this computer" before anyone commits to it.
 */

import { t } from "../i18n";

/** Where the last run's pace is kept (per viewer, a convenience). */
export const PACE_KEY = "northlight.denoiseSecondsPerMp";

/** "12 s", "about 2 min", for a number of seconds. */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  if (seconds < 60) return t("tools.denoise.seconds", { count: Math.max(1, Math.round(seconds)) });
  const minutes = Math.round(seconds / 60);
  return t("tools.denoise.minutes", { count: minutes });
}

/**
 * Seconds left, from when the first tile finished (`start`, with `startDone`
 * tiles done) to now (`done` of `total`). `null` until two tiles have been
 * timed: the first includes loading the model onto the graphics card.
 */
export function secondsLeft(
  start: { at: number; done: number },
  now: { at: number; done: number },
  total: number,
): number | null {
  const tiles = now.done - start.done;
  if (tiles < 2) return null;
  const perTile = (now.at - start.at) / 1000 / tiles;
  return perTile * (total - now.done);
}

/** Seconds per megapixel of the run just finished, to remember. */
export function paceOf(seconds: number, width: number, height: number): number | null {
  const mp = (width * height) / 1e6;
  return seconds > 0 && mp > 0 ? seconds / mp : null;
}

/** The estimate for a photo of `width` × `height`, from a remembered pace. */
export function estimateFor(pace: number | null, width: number, height: number): number | null {
  return pace && pace > 0 ? (pace * width * height) / 1e6 : null;
}

export function readPace(): number | null {
  try {
    const v = Number(localStorage.getItem(PACE_KEY));
    return Number.isFinite(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
}

export function writePace(pace: number | null): void {
  if (!pace) return;
  try {
    localStorage.setItem(PACE_KEY, String(pace));
  } catch {
    // Storage unavailable: the next run measures again.
  }
}
