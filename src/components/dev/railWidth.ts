/** The editing rail's width: its clamp bounds and default, in px, and how wide
 *  it draws in a window of a given width. Shared by Develop and the shell it
 *  shows while its first photo opens, so the rail does not move between them. */
export const RAIL_MIN = 300;
export const RAIL_MAX = 620;
export const RAIL_DEFAULT = 360;
/** Canvas the panels leave, at the least. */
export const CANVAS_MIN = 400;
/** The Presets panel's own minimum (DevelopPresets). */
export const PRESETS_MIN = 220;

/** The width the rail was last left at, or the default. */
export function savedRailWidth(): number {
  let saved = NaN;
  try {
    saved = Number(localStorage.getItem("developRailW"));
  } catch {
    // Storage unavailable: the default.
  }
  return Number.isFinite(saved) && saved >= RAIL_MIN && saved <= RAIL_MAX ? saved : RAIL_DEFAULT;
}

/** The rail as wide as asked, but never so wide the photo goes: at
 *  least CANVAS_MIN of canvas, Presets giving way first, then the rail, down to
 *  their minimums. */
export function railFit(railW: number, windowW: number, presetsOpen: boolean): number {
  return Math.max(RAIL_MIN, Math.min(railW, windowW - CANVAS_MIN - (presetsOpen ? PRESETS_MIN : 0)));
}
