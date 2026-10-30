import { describe, expect, it } from "vitest";
import { dragCrop, type Grip } from "./cropDrag";

/** A 3:2 frame with a centred square locked at 1:1, as the review replayed. */
const ASPECT = 3 / 2;
const square = { x: 1 / 6, y: 0, width: 2 / 3, height: 1 };

const corner = (c: { x: number; y: number; width: number; height: number }, g: Grip) => ({
  x: g === "nw" || g === "sw" ? c.x : c.x + c.width,
  y: g === "nw" || g === "ne" ? c.y : c.y + c.height,
});
const opposite: Record<string, Grip> = { nw: "se", ne: "sw", sw: "ne", se: "nw" };

describe("dragCrop", () => {
  for (const g of ["nw", "ne", "sw", "se"] as const) {
    it(`keeps the opposite corner pinned when ${g} is dragged with a ratio locked`, () => {
      const inward = { nw: [0.1, 0.1], ne: [-0.1, 0.1], sw: [0.1, -0.1], se: [-0.1, -0.1] }[g];
      const out = dragCrop(square, g, inward[0]!, inward[1]!, 1, ASPECT);
      const fixed = corner(square, opposite[g]!);
      const now = corner(out, opposite[g]!);
      expect(now.x).toBeCloseTo(fixed.x, 6);
      expect(now.y).toBeCloseTo(fixed.y, 6);
      // Still square on screen.
      expect((out.width * ASPECT) / out.height).toBeCloseTo(1, 6);
    });
  }

  it("keeps a free corner drag's opposite corner pinned too", () => {
    const c = { x: 0.2, y: 0.2, width: 0.5, height: 0.5 };
    const out = dragCrop(c, "nw", 0.05, -0.1, null, ASPECT);
    expect(out.x + out.width).toBeCloseTo(0.7, 6);
    expect(out.y + out.height).toBeCloseTo(0.7, 6);
    expect(out.x).toBeCloseTo(0.25, 6);
    expect(out.y).toBeCloseTo(0.1, 6);
  });

  it("drags one edge alone", () => {
    const c = { x: 0, y: 0, width: 1, height: 1 };
    const out = dragCrop(c, "s", 0, -0.2, null, ASPECT);
    expect(out).toEqual({ x: 0, y: 0, width: 1, height: 0.8 });
    const left = dragCrop(c, "w", 0.25, 0, null, ASPECT);
    expect(left.x).toBeCloseTo(0.25, 6);
    expect(left.width).toBeCloseTo(0.75, 6);
  });

  it("keeps the ratio on an edge drag, centred on the other axis", () => {
    const out = dragCrop(square, "e", -0.2, 0, 1, ASPECT);
    expect((out.width * ASPECT) / out.height).toBeCloseTo(1, 6);
    expect(out.x).toBeCloseTo(square.x, 6);
    expect(out.y + out.height / 2).toBeCloseTo(0.5, 6);
  });

  it("never leaves the frame or goes below the smallest crop", () => {
    const c = { x: 0.1, y: 0.1, width: 0.3, height: 0.3 };
    const moved = dragCrop(c, "move", 2, 2, null, ASPECT);
    expect(moved.x + moved.width).toBeLessThanOrEqual(1);
    expect(moved.y + moved.height).toBeLessThanOrEqual(1);
    const tiny = dragCrop(c, "se", -0.3, -0.3, null, ASPECT);
    expect(tiny.width).toBeGreaterThanOrEqual(0.02);
    expect(tiny.height).toBeGreaterThanOrEqual(0.02);
  });
});
