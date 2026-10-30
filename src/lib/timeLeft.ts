/**
 * How long a library pass has left — indexing, faces, tagging, a scan — from
 * its pace on this computer since the panel first saw it.
 *
 * The same pass takes minutes on a fast graphics card and hours on a laptop's
 * CPU, so a count alone ("312 / 12,400") says nothing about whether to wait.
 * The pace is measured, not assumed: nothing is said until a few photos and a
 * few seconds have gone by, because the first photo includes loading the
 * model and a guess from it would be wildly wrong.
 */

import { t } from "../i18n";

/** A pass's count at a moment (`at` in milliseconds). */
export interface Sample {
  at: number;
  done: number;
}

/** Photos, and seconds, a pace needs before it is worth quoting. */
const MIN_DONE = 3;
const MIN_SECONDS = 5;

/**
 * Seconds left, from `first` (when the pace started being measured) to `now`,
 * with `total` to do. `null` until there is a pace worth quoting, and when
 * nothing is left.
 */
export function secondsLeft(first: Sample, now: Sample, total: number): number | null {
  const done = now.done - first.done;
  const seconds = (now.at - first.at) / 1000;
  if (done < MIN_DONE || seconds < MIN_SECONDS || now.done >= total) return null;
  return (seconds / done) * (total - now.done);
}

/** "About 40 s left", "About 12 min left", "About 3 h left". */
export function timeLeftText(seconds: number): string {
  let duration: string;
  if (seconds < 60) {
    duration = t("settings.activity.seconds", { count: Math.max(5, Math.round(seconds / 5) * 5) });
  } else if (seconds < 90 * 60) {
    duration = t("settings.activity.minutes", { count: Math.round(seconds / 60) });
  } else {
    duration = t("settings.activity.hours", { count: Math.round(seconds / 3600) });
  }
  return t("settings.activity.timeLeft", { duration });
}
