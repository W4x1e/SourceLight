import type { Crop, Stack } from "../types";

type Geometry = Stack["geometry"];

/**
 * The crop tool's view of the photo: turned, flipped and straightened as the
 * edit says, only the crop released. Crop mode used to draw the photo
 * with all its framing removed, so Rotate, Flip and Straighten showed nothing
 * until Done — Right pressed twice ended upside down.
 *
 * The crop is stored in the oriented image's coordinates, before any turn.
 * The box the tool draws and drags is in this view's. These convert between
 * them through the same transform the develop pass uses
 * (`Geometry::transform` in geometry.rs, mirrored here — keep the two in step).
 */

const FULL: Crop = { x: 0, y: 0, width: 1, height: 1 };

/** Framed UV → oriented-image UV: `m·(p − ½) + c`. `aspect` is the oriented
 *  image's width over height; only the ratio matters. */
export function transform(g: Geometry, aspect: number): { m: [number, number, number, number]; c: [number, number] } {
  const crop = g.crop;
  const w = aspect;
  const h = 1;
  const turns = ((g.quarterTurns % 4) + 4) % 4;
  const [ex, ey] = turns % 2 === 1 ? [crop.height * h, crop.width * w] : [crop.width * w, crop.height * h];
  const angle = (-(turns * 90 + g.straighten) * Math.PI) / 180;
  const sin = Math.sin(angle);
  const cos = Math.cos(angle);
  const fx = g.flip ? -1 : 1;
  const colX = [cos * ex * fx, sin * ex * fx];
  const colY = [-sin * ey, cos * ey];
  const m0 = colX[0]! / w;
  const m1 = colY[0]! / w;
  const m2 = colX[1]! / h;
  const m3 = colY[1]! / h;
  const cx = crop.x + crop.width / 2;
  const cy = crop.y + crop.height / 2;
  // The auto-crop on straighten, as geometry.rs: the largest scale that keeps
  // every sample inside the frame.
  const reachX = 0.5 * (Math.abs(m0) + Math.abs(m1));
  const reachY = 0.5 * (Math.abs(m2) + Math.abs(m3));
  const availX = Math.max(Math.min(cx, 1 - cx), 1e-4);
  const availY = Math.max(Math.min(cy, 1 - cy), 1e-4);
  const s = Math.min(1, availX / Math.max(reachX, 1e-6), availY / Math.max(reachY, 1e-6));
  return { m: [m0 * s, m1 * s, m2 * s, m3 * s], c: [cx, cy] };
}

/** The geometry the crop tool's view is drawn with. */
export function viewGeometry(g: Geometry): Geometry {
  return { ...g, crop: FULL };
}

const apply = (t: ReturnType<typeof transform>, p: [number, number]): [number, number] => {
  const x = p[0] - 0.5;
  const y = p[1] - 0.5;
  return [t.m[0] * x + t.m[1] * y + t.c[0], t.m[2] * x + t.m[3] * y + t.c[1]];
};
const unapply = (t: ReturnType<typeof transform>, q: [number, number]): [number, number] => {
  const [a, b, c, d] = t.m;
  const det = a * d - b * c || 1e-12;
  const x = q[0] - t.c[0];
  const y = q[1] - t.c[1];
  return [(d * x - b * y) / det + 0.5, (-c * x + a * y) / det + 0.5];
};

/** Where the stored crop lies in the crop tool's view. */
export function toView(g: Geometry, aspect: number): Crop {
  const crop = transform(g, aspect);
  const view = transform(viewGeometry(g), aspect);
  const a = unapply(view, apply(crop, [0, 0]));
  const b = unapply(view, apply(crop, [1, 1]));
  return {
    x: Math.min(a[0], b[0]),
    y: Math.min(a[1], b[1]),
    width: Math.abs(b[0] - a[0]),
    height: Math.abs(b[1] - a[1]),
  };
}

/**
 * The stored crop that shows as `box` in the crop tool's view. The centre is
 * exact; the size is found by iterating, since the straighten's auto-crop
 * scale depends on the crop it scales.
 */
export function fromView(box: Crop, g: Geometry, aspect: number): Crop {
  const view = transform(viewGeometry(g), aspect);
  const centre = apply(view, [box.x + box.width / 2, box.y + box.height / 2]);
  // The box's sides in oriented-image UV units, along the view's own axes.
  const along = (v: [number, number]) => Math.hypot(v[0] * aspect, v[1]);
  const wantX = along([view.m[0] * box.width, view.m[2] * box.width]);
  const wantY = along([view.m[1] * box.height, view.m[3] * box.height]);
  const turned = ((g.quarterTurns % 4) + 4) % 4 % 2 === 1;
  // Start at the box's size as if nothing were auto-cropped, and grow from
  // there. Started at the full frame, a box off centre and against an edge,
  // as every corner drag from the full frame is, stopped at once: the
  // auto-crop shrank the full frame to exactly the box's size, so the full
  // frame looked like the answer, and the box sprang back to it.
  const unit = transform({ ...g, crop: { x: 0.495, y: 0.495, width: 0.01, height: 0.01 } }, aspect);
  const perX = along([unit.m[0], unit.m[2]]) / 0.01;
  const perY = along([unit.m[1], unit.m[3]]) / 0.01;
  let crop: Crop = { ...FULL };
  let width = Math.min(Math.max((turned ? wantY / perY : wantX / perX) || 1, 0.02), 1);
  let height = Math.min(Math.max((turned ? wantX / perX : wantY / perY) || 1, 0.02), 1);
  for (let i = 0; i < 12; i++) {
    crop = {
      x: centre[0] - width / 2,
      y: centre[1] - height / 2,
      width,
      height,
    };
    const t = transform({ ...g, crop }, aspect);
    const gotX = along([t.m[0], t.m[2]]);
    const gotY = along([t.m[1], t.m[3]]);
    // A turned frame's output x runs along the crop's height.
    if (turned) {
      height *= wantX / Math.max(gotX, 1e-9);
      width *= wantY / Math.max(gotY, 1e-9);
    } else {
      width *= wantX / Math.max(gotX, 1e-9);
      height *= wantY / Math.max(gotY, 1e-9);
    }
    width = Math.min(Math.max(width, 0.02), 1);
    height = Math.min(Math.max(height, 0.02), 1);
  }
  return {
    width,
    height,
    x: Math.min(Math.max(centre[0] - width / 2, 0), 1 - width),
    y: Math.min(Math.max(centre[1] - height / 2, 0), 1 - height),
  };
}

/** The oriented image's width over height, from Develop's framed size and the
 *  geometry it was framed with. */
export function orientedAspect(framedW: number, framedH: number, g: Geometry): number {
  const turned = ((g.quarterTurns % 4) + 4) % 4 % 2 === 1;
  const w = turned ? framedH / g.crop.width : framedW / g.crop.width;
  const h = turned ? framedW / g.crop.height : framedH / g.crop.height;
  return w / h;
}
