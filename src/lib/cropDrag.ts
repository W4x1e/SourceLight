import type { Crop } from "../types";

/**
 * Dragging the crop rectangle, worked out apart from the overlay so each grip
 * can be tested.
 *
 * Fractions of the uncropped frame throughout. `ratio` is width over height in
 * display terms (4:5, 1:1, the original); `frameAspect` the uncropped frame's,
 * to convert it to fractions of a frame that is not square.
 */
export type Grip = "nw" | "ne" | "sw" | "se" | "n" | "s" | "e" | "w" | "move";

/** Smallest crop the backend will accept, mirroring `CropRect::sanitised`. */
export const MIN = 0.02;

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);

/** The crop after moving `grip` by `(dx, dy)` from `start`. */
export function dragCrop(
  start: Crop,
  grip: Grip,
  dx: number,
  dy: number,
  ratio: number | null,
  frameAspect: number,
): Crop {
  const target = ratio ? ratio / frameAspect : null;

  if (grip === "move") {
    return {
      ...start,
      x: clamp(start.x + dx, 0, 1 - start.width),
      y: clamp(start.y + dy, 0, 1 - start.height),
    };
  }

  if (grip === "n" || grip === "s" || grip === "e" || grip === "w") return edge(start, grip, dx, dy, target);

  // A corner: the opposite corner stays where it is, whatever the ratio.
  // The ratio used to be applied anchored at the top-left, so three of the four
  // corners moved the one that should stay pinned.
  const left = grip === "nw" || grip === "sw";
  const top = grip === "nw" || grip === "ne";
  const fx = left ? start.x + start.width : start.x;
  const fy = top ? start.y + start.height : start.y;
  const px = (left ? start.x : start.x + start.width) + dx;
  const py = (top ? start.y : start.y + start.height) + dy;
  // Which way from the fixed corner the dragged one now lies: past it flips.
  const goesLeft = px < fx;
  const goesUp = py < fy;
  let w = Math.min(Math.abs(px - fx), goesLeft ? fx : 1 - fx);
  let h = Math.min(Math.abs(py - fy), goesUp ? fy : 1 - fy);
  if (target) {
    // Shrink the longer side, so it never grows past the room it has.
    if (w / Math.max(h, 1e-9) > target) w = h * target;
    else h = w / target;
  }
  w = Math.max(w, MIN);
  h = Math.max(h, MIN);
  return {
    x: clamp(goesLeft ? fx - w : fx, 0, 1 - w),
    y: clamp(goesUp ? fy - h : fy, 0, 1 - h),
    width: w,
    height: h,
  };
}

/** An edge: moves that side alone, the opposite one fixed. With a ratio, the
 *  other dimension follows, centred on where it was. */
function edge(start: Crop, grip: "n" | "s" | "e" | "w", dx: number, dy: number, target: number | null): Crop {
  if (grip === "n" || grip === "s") {
    const fixed = grip === "n" ? start.y + start.height : start.y;
    const moved = (grip === "n" ? start.y : start.y + start.height) + dy;
    const up = moved < fixed;
    let h = Math.max(Math.min(Math.abs(moved - fixed), up ? fixed : 1 - fixed), MIN);
    let width = start.width;
    let x = start.x;
    if (target) {
      width = Math.min(h * target, 1);
      h = width / target;
      x = clamp(start.x + start.width / 2 - width / 2, 0, 1 - width);
    }
    return { x, width, height: h, y: clamp(up ? fixed - h : fixed, 0, 1 - h) };
  }
  const fixed = grip === "w" ? start.x + start.width : start.x;
  const moved = (grip === "w" ? start.x : start.x + start.width) + dx;
  const leftward = moved < fixed;
  let w = Math.max(Math.min(Math.abs(moved - fixed), leftward ? fixed : 1 - fixed), MIN);
  let height = start.height;
  let y = start.y;
  if (target) {
    height = Math.min(w / target, 1);
    w = height * target;
    y = clamp(start.y + start.height / 2 - height / 2, 0, 1 - height);
  }
  return { y, height, width: w, x: clamp(leftward ? fixed - w : fixed, 0, 1 - w) };
}
