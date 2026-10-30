import { describe, expect, it } from "vitest";
import { canPan, clampCentre, fitBox, halfExtent, panBy } from "./viewport";

// The reported case throughout: a portrait sensor shown in a landscape canvas.
const FRAME = { width: 4024, height: 6024 };
const CANVAS = { width: 1850, height: 1050 };

describe("halfExtent", () => {
  it("pins (returns 0.5) when the frame fits inside the area", () => {
    // At a low zoom the whole 4024px width is on screen, so there is no room
    // to pan on that axis — the exact case that made panning look broken.
    expect(halfExtent(CANVAS.width, FRAME.width, 0.26)).toBe(0.5);
  });

  it("shrinks as zoom rises, opening room to pan", () => {
    // 1:1 on a 4024px frame in an 1850px canvas shows 1850/4024 of the width.
    expect(halfExtent(CANVAS.width, FRAME.width, 1)).toBeCloseTo(1850 / 4024 / 2, 5);
  });

  it("guards a zero or negative zoom rather than dividing by it", () => {
    expect(halfExtent(CANVAS.width, FRAME.width, 0)).toBe(0.5);
  });
});

describe("canPan", () => {
  it("reports no pannable axis at fit-to-window", () => {
    const fit = Math.min(CANVAS.width / FRAME.width, CANVAS.height / FRAME.height);
    expect(canPan(fit, FRAME, CANVAS)).toEqual({ x: false, y: false });
  });

  it("unpins the two axes at different zooms for a portrait frame", () => {
    // The bug: at 26% the width is fully shown (x pinned) while height is not.
    // A single flag would have been wrong; the two axes must be independent.
    expect(canPan(0.26, FRAME, CANVAS)).toEqual({ x: false, y: true });
  });

  it("pans both axes once zoomed in far enough", () => {
    expect(canPan(1, FRAME, CANVAS)).toEqual({ x: true, y: true });
  });
});

describe("clampCentre", () => {
  it("holds the centre inside the photo so the view cannot leave it", () => {
    const c = clampCentre({ x: 2, y: -1 }, 1, FRAME, CANVAS);
    const hx = halfExtent(CANVAS.width, FRAME.width, 1);
    const hy = halfExtent(CANVAS.height, FRAME.height, 1);
    expect(c.x).toBeCloseTo(1 - hx, 5);
    expect(c.y).toBeCloseTo(hy, 5);
  });

  it("pins a pinned axis to the centre regardless of input", () => {
    // x is pinned at 0.26; whatever comes in, it must land at 0.5.
    expect(clampCentre({ x: 0.9, y: 0.5 }, 0.26, FRAME, CANVAS).x).toBeCloseTo(0.5, 5);
  });
});

describe("panBy", () => {
  it("moves one image pixel per screen pixel at 1:1", () => {
    // A 100px leftward drag at zoom 1 on a 4024-wide frame moves the centre by
    // exactly 100/4024. This is the arithmetic the harness confirmed by hand.
    const moved = panBy({ x: 0.5, y: 0.5 }, 100, 0, 1, 1, FRAME, CANVAS);
    expect(moved.x).toBeCloseTo(0.5 - 100 / 4024, 5);
  });

  it("scales the delta by the device pixel ratio", () => {
    // Same drag on a 2x display should move twice as far in image space.
    const one = panBy({ x: 0.5, y: 0.5 }, 100, 0, 1, 1, FRAME, CANVAS).x;
    const two = panBy({ x: 0.5, y: 0.5 }, 100, 0, 2, 1, FRAME, CANVAS).x;
    expect(0.5 - two).toBeCloseTo((0.5 - one) * 2, 5);
  });

  it("stops at the edge instead of accumulating overshoot", () => {
    // The freeze-adjacent bug: an unclamped pan kept accumulating past the edge,
    // so dragging back did nothing until the hidden overshoot unwound.
    const shoved = panBy({ x: 0.5, y: 0.5 }, -100000, 0, 1, 1, FRAME, CANVAS);
    const hx = halfExtent(CANVAS.width, FRAME.width, 1);
    expect(shoved.x).toBeCloseTo(1 - hx, 5);
  });
});

describe("fitBox", () => {
  const DPR = 2;
  const aspect = FRAME.width / FRAME.height;

  it("letterboxes the framed image, centred, when there is no view", () => {
    // Portrait frame in a landscape container: height-bound, so the box is the
    // container height tall and centred horizontally — the fit-to-window box the
    // overlay measured before zoom was a thing.
    const box = fitBox(CANVAS, aspect, null, FRAME, DPR);
    expect(box.height).toBeCloseTo(CANVAS.height, 5);
    expect(box.width).toBeCloseTo(CANVAS.height * aspect, 5);
    expect(box.top).toBeCloseTo(0, 5);
    expect(box.left).toBeCloseTo((CANVAS.width - box.width) / 2, 5);
  });

  it("puts the view centre at the container centre and scales by zoom/dpr", () => {
    const box = fitBox(CANVAS, aspect, { zoom: 1, cx: 0.5, cy: 0.5 }, FRAME, DPR);
    expect(box.width).toBeCloseTo(FRAME.width / DPR, 5); // one image px = 1/dpr css px
    expect(box.left + 0.5 * box.width).toBeCloseTo(CANVAS.width / 2, 5);
    expect(box.top + 0.5 * box.height).toBeCloseTo(CANVAS.height / 2, 5);
  });

  it("is the exact inverse of panBy, so a handle tracks the pixel under it", () => {
    // The load-bearing guarantee: when the renderer pans by a drag of `dx`, the
    // UV under a fixed screen point must shift by the same amount, or a mask
    // handle drifts off the photo the moment you pan while zoomed.
    const dx = 120;
    const view = { zoom: 1, cx: 0.5, cy: 0.5 };
    const box = fitBox(CANVAS, aspect, view, FRAME, DPR);
    const screenX = 900;
    const uvBefore = (screenX - box.left) / box.width;

    const panned = panBy({ x: view.cx, y: view.cy }, dx, 0, DPR, view.zoom, FRAME, CANVAS);
    const box2 = fitBox(CANVAS, aspect, { ...view, cx: panned.x, cy: panned.y }, FRAME, DPR);
    const uvAfter = (screenX - box2.left) / box2.width;

    expect(uvAfter - uvBefore).toBeCloseTo(-dx / box.width, 6);
  });

  it("matches the letterbox when the view is exactly fit-to-window", () => {
    // `zoom` is device px per image px, so fit uses the container in device px.
    const fit = Math.min(
      (CANVAS.width * DPR) / FRAME.width,
      (CANVAS.height * DPR) / FRAME.height,
    );
    const viewed = fitBox(CANVAS, aspect, { zoom: fit, cx: 0.5, cy: 0.5 }, FRAME, DPR);
    const letter = fitBox(CANVAS, aspect, null, FRAME, DPR);
    expect(viewed.width).toBeCloseTo(letter.width, 3);
    expect(viewed.height).toBeCloseTo(letter.height, 3);
    expect(viewed.left).toBeCloseTo(letter.left, 3);
    expect(viewed.top).toBeCloseTo(letter.top, 3);
  });
});
