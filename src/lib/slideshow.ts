/**
 * The slideshow's rules: its options, its order, and where it goes
 * next. The playing lives in `Slideshow`; what it plays and when is here,
 * where it can be tested.
 */

import type { Photo } from "../types";
import type { Key } from "../i18n";

export type SlideCaption = "none" | "name" | "date" | "place";

export interface SlideshowOptions {
  /** How long each photo stays, in seconds. */
  seconds: number;
  /** How long one fades into the next, in seconds; 0 cuts. */
  fade: number;
  shuffle: boolean;
  /** Round again after the last photo, or stop there. */
  loop: boolean;
  caption: SlideCaption;
}

export const DEFAULT_SLIDESHOW: SlideshowOptions = {
  seconds: 5,
  fade: 0.8,
  shuffle: false,
  loop: true,
  caption: "none",
};

export const SLIDE_CAPTIONS: { id: SlideCaption; label: Key }[] = [
  { id: "none", label: "viewer.slideshow.captionNone" },
  { id: "name", label: "viewer.slideshow.captionName" },
  { id: "date", label: "viewer.slideshow.captionDate" },
  { id: "place", label: "viewer.slideshow.captionPlace" },
];

/** The photos in the order they will play: as given, or shuffled — with the
 *  photo asked to start on first either way. `random` is for tests. */
export function playOrder<T>(items: T[], start: number, shuffle: boolean, random = Math.random): T[] {
  if (items.length === 0) return [];
  const at = Math.min(Math.max(0, start), items.length - 1);
  if (!shuffle) return [...items.slice(at), ...items.slice(0, at)];
  const rest = items.filter((_, i) => i !== at);
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [rest[i], rest[j]] = [rest[j]!, rest[i]!];
  }
  return [items[at]!, ...rest];
}

/** The index after (or before) `i`, round the end when looping; `null` at an
 *  end when not. */
export function stepFrom(i: number, n: number, dir: 1 | -1, loop: boolean): number | null {
  if (n === 0) return null;
  const j = i + dir;
  if (j >= 0 && j < n) return j;
  return loop ? (j + n) % n : null;
}

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** What the caption says under a photo, or null for none. */
export function captionFor(photo: Pick<Photo, "filename" | "captureDate" | "place">, mode: SlideCaption): string | null {
  switch (mode) {
    case "name":
      return photo.filename;
    case "date": {
      const d = photo.captureDate;
      if (!d) return null;
      const [y, m, day] = d.slice(0, 10).split("-").map(Number) as [number, number, number];
      return `${day} ${MONTHS[m - 1]} ${y}`;
    }
    case "place":
      return photo.place ?? null;
    default:
      return null;
  }
}

/** Whether this screen shows more pixels than a cached preview has, so the
 *  photo's full resolution is worth fetching. */
export function wantsFull(screenWidth: number, screenHeight: number, dpr: number, previewEdge = 1600): boolean {
  return Math.max(screenWidth, screenHeight) * dpr > previewEdge * 1.1;
}
