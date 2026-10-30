import { describe, expect, it } from "vitest";
import { fromView, orientedAspect, toView, transform } from "./cropView";

const geom = (over: Partial<{ crop: { x: number; y: number; width: number; height: number }; straighten: number; quarterTurns: number; flip: boolean }> = {}) => ({
  crop: { x: 0.1, y: 0.2, width: 0.5, height: 0.6 },
  straighten: 0,
  quarterTurns: 0,
  flip: false,
  ...over,
});
const ASPECT = 3 / 2;

const near = (a: { x: number; y: number; width: number; height: number }, b: typeof a, d = 1e-3) => {
  expect(a.x).toBeCloseTo(b.x, -Math.log10(d));
  expect(a.y).toBeCloseTo(b.y, -Math.log10(d));
  expect(a.width).toBeCloseTo(b.width, -Math.log10(d));
  expect(a.height).toBeCloseTo(b.height, -Math.log10(d));
};

/** The crop tool's view keeps the turn, flip and straighten. */
describe("cropView", () => {
  it("is the crop itself with no framing beyond it", () => {
    const g = geom();
    near(toView(g, ASPECT), g.crop);
  });

  it("turns the box with the photo", () => {
    // A quarter turn clockwise: the crop's left becomes the view's top.
    const v = toView(geom({ quarterTurns: 1 }), ASPECT);
    expect(v.width).toBeCloseTo(0.6, 6);
    expect(v.height).toBeCloseTo(0.5, 6);
  });

  for (const over of [
    {},
    { quarterTurns: 1 },
    { quarterTurns: 2 },
    { quarterTurns: 3 },
    { flip: true },
    { quarterTurns: 1, flip: true },
    { straighten: 3.5 },
    { straighten: -6, quarterTurns: 1 },
    { straighten: 8, flip: true },
  ]) {
    it(`goes there and back: ${JSON.stringify(over)}`, () => {
      const g = geom({ crop: { x: 0.25, y: 0.25, width: 0.4, height: 0.45 }, ...over });
      near(fromView(toView(g, ASPECT), g, ASPECT), g.crop);
    });
  }

  // A corner dragged in from the full frame leaves the box against the
  // opposite edges, off centre: it used to come back as the full frame.
  for (const over of [{}, { quarterTurns: 1 }, { flip: true }, { straighten: 4 }]) {
    it(`keeps a box against the edges where it is: ${JSON.stringify(over)}`, () => {
      const g = geom({ crop: { x: 0, y: 0, width: 1, height: 1 }, ...over });
      for (const box of [
        { x: 0.13, y: 0.12, width: 0.87, height: 0.88 },
        { x: 0, y: 0, width: 0.6, height: 0.7 },
        { x: 0.3, y: 0, width: 0.7, height: 1 },
      ]) {
        const crop = fromView(box, g, ASPECT);
        near(toView({ ...g, crop }, ASPECT), box);
      }
    });
  }

  it("recovers the oriented aspect from the framed size", () => {
    // A 6000×4000 photo cropped to its left half and turned: framed 4000×3000.
    const g = geom({ crop: { x: 0, y: 0, width: 0.5, height: 1 }, quarterTurns: 1 });
    expect(orientedAspect(4000, 3000, g)).toBeCloseTo(1.5, 6);
  });
});

/** The same numbers geometry.rs pins for this geometry: the mirror must match. */
it("matches the develop pass's transform", () => {
  const t = transform(
    { crop: { x: 0.25, y: 0.25, width: 0.4, height: 0.45 }, straighten: -6, quarterTurns: 1, flip: true },
    6000 / 4000,
  );
  const want = [-0.031358525, 0.39780876, 0.44753486, 0.06271705];
  t.m.forEach((v, i) => expect(v).toBeCloseTo(want[i]!, 5));
  expect(t.c[0]).toBeCloseTo(0.45, 6);
  expect(t.c[1]).toBeCloseTo(0.475, 6);
});
