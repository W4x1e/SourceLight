/**
 * What a wheel event over a photo means (the Windows half of what the Mac's
 * trackpad handling in `ZoomableImage` does).
 *
 * Windows sends three different things as `wheel`:
 * - a mouse notch: a whole line's worth, about 100 px, vertical only;
 * - a two-finger scroll on a precision touchpad: many small, often fractional
 *   steps, frequently with some sideways movement;
 * - a pinch on the same touchpad: small steps with `ctrlKey` set, the way
 *   Chromium reports it.
 *
 * Zooming 15% on every event, as the grid's viewer did, made a touchpad scroll
 * over a photo zoom it wildly. The system photo viewers' answer, and this one: a notch
 * zooms a step, a pinch zooms smoothly, a scroll pans a zoomed photo and leaves
 * a fitted one alone.
 */
export type WheelIntent =
  | { kind: "step"; zoomIn: boolean }
  | { kind: "smooth"; factor: number }
  | { kind: "pan"; dx: number; dy: number }
  | { kind: "none" };

/** Smallest vertical delta (px) a mouse notch produces. A touchpad step is smaller. */
const NOTCH = 50;

export function wheelIntent(
  e: { deltaX: number; deltaY: number; deltaMode: number; ctrlKey: boolean; metaKey?: boolean },
  zoomed: boolean,
): WheelIntent {
  if (e.ctrlKey || e.metaKey) {
    return { kind: "smooth", factor: Math.exp(-e.deltaY * 0.01) };
  }
  // Lines or pages (deltaMode 1, 2) come from a wheel; so does a large, purely
  // vertical pixel step.
  const notch = e.deltaMode !== 0 || (Math.abs(e.deltaY) >= NOTCH && e.deltaX === 0 && Number.isInteger(e.deltaY));
  if (notch) return e.deltaY === 0 ? { kind: "none" } : { kind: "step", zoomIn: e.deltaY < 0 };
  return zoomed ? { kind: "pan", dx: e.deltaX, dy: e.deltaY } : { kind: "none" };
}
