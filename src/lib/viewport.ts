/**
 * Viewport arithmetic, pulled out of the canvas so it can be tested.
 *
 * **This mirrors `ViewPort::anchored` in the renderer and has to keep agreeing
 * with it.** The renderer bounds what it draws; if this does not bound what it
 * reports, dragging past an edge accumulates an offset nothing on screen
 * reflects, and dragging back then does nothing until that offset unwinds. The
 * overshoot is unbounded, so it reads as the drag having died rather than as a
 * limit being reached.
 */

export interface Centre {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** A zoomed view: `zoom` is device pixels per image pixel, `cx`/`cy` the centre in UV. */
export interface View {
  zoom: number;
  cx: number;
  cy: number;
}

/**
 * The on-screen rectangle, in CSS pixels within a `container`, that the whole
 * framed image [0,1]² occupies — the box an overlay measures its 0..1 UV
 * against.
 *
 * With no `view` it is the fit-to-window letterbox. With a `view` it is placed
 * so the view centre `(cx,cy)` lands at the container centre and one image pixel
 * spans `zoom/dpr` CSS pixels — the exact inverse of `panBy`'s mapping, so the
 * handles an overlay draws track the pixels the renderer draws. The box may
 * extend past the container (the visible part is what the renderer shows);
 * callers clip it.
 */
export function fitBox(
  container: Size,
  frameAspect: number,
  view: View | null,
  framePx: Size,
  dpr: number,
): Rect {
  const w = container.width;
  const h = container.height;

  if (view && view.zoom > 0 && framePx.width > 0 && dpr > 0) {
    const imgW = (view.zoom * framePx.width) / dpr;
    const imgH = (view.zoom * framePx.height) / dpr;
    return { left: w / 2 - view.cx * imgW, top: h / 2 - view.cy * imgH, width: imgW, height: imgH };
  }

  // Fit-to-window: the largest frame-aspect box centred in the container.
  let bw = w;
  let bh = frameAspect > 0 ? w / frameAspect : h;
  if (bh > h) {
    bh = h;
    bw = h * frameAspect;
  }
  return { left: (w - bw) / 2, top: (h - bh) / 2, width: bw, height: bh };
}

/**
 * Half the visible region on one axis, as a fraction of the frame.
 *
 * `area` is in device pixels and `zoom` is device pixels per image pixel, so
 * `area / zoom` is the visible span in image pixels. Capped at the whole frame:
 * once the photo fits, there is nothing left to pan and the centre is pinned.
 */
export function halfExtent(area: number, frame: number, zoom: number): number {
  if (!(zoom > 0) || frame <= 0) return 0.5;
  return Math.min(1, area / zoom / frame) / 2;
}

/**
 * Whether there is anywhere to pan on either axis.
 *
 * **The control has to say so.** Below the zoom where the photo overflows the
 * canvas the centre is pinned and dragging correctly does nothing — but offering
 * a `grab` cursor and then ignoring the drag is indistinguishable from a broken
 * one, which is exactly how it was reported. On a portrait frame in a landscape
 * canvas the two axes unpin at very different zooms, so this is per-axis rather
 * than a single flag.
 */
export function canPan(zoom: number, frame: Size, area: Size): { x: boolean; y: boolean } {
  return {
    x: halfExtent(area.width, frame.width, zoom) < 0.5,
    y: halfExtent(area.height, frame.height, zoom) < 0.5,
  };
}

/** Holds a view centre inside the photograph. */
export function clampCentre(centre: Centre, zoom: number, frame: Size, area: Size): Centre {
  const hx = halfExtent(area.width, frame.width, zoom);
  const hy = halfExtent(area.height, frame.height, zoom);
  return {
    x: Math.min(Math.max(centre.x, hx), 1 - hx),
    y: Math.min(Math.max(centre.y, hy), 1 - hy),
  };
}

/**
 * Moves the centre by a pointer delta in **CSS** pixels.
 *
 * The delta is scaled by the device pixel ratio because `zoom` counts device
 * pixels: without it a photo moves by the wrong amount on any scaled display,
 * which looks like sluggish dragging rather than a unit error.
 */
export function panBy(
  centre: Centre,
  dxCss: number,
  dyCss: number,
  dpr: number,
  zoom: number,
  frame: Size,
  area: Size,
): Centre {
  return clampCentre(
    {
      x: centre.x - (dxCss * dpr) / zoom / frame.width,
      y: centre.y - (dyCss * dpr) / zoom / frame.height,
    },
    zoom,
    frame,
    area,
  );
}
