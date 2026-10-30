import { describe, expect, it } from "vitest";
import { wheelIntent } from "./wheel";

const ev = (deltaX: number, deltaY: number, extra: Partial<{ deltaMode: number; ctrlKey: boolean }> = {}) => ({
  deltaX,
  deltaY,
  deltaMode: 0,
  ctrlKey: false,
  ...extra,
});

describe("what a wheel event over a photo means", () => {
  it("a mouse notch zooms a step, in or out", () => {
    expect(wheelIntent(ev(0, -100), false)).toEqual({ kind: "step", zoomIn: true });
    expect(wheelIntent(ev(0, 100), true)).toEqual({ kind: "step", zoomIn: false });
    expect(wheelIntent(ev(0, 3, { deltaMode: 1 }), false)).toEqual({ kind: "step", zoomIn: false });
  });

  it("a pinch zooms smoothly, in proportion", () => {
    const i = wheelIntent(ev(0, -4.5, { ctrlKey: true }), false);
    expect(i.kind).toBe("smooth");
    expect(i.kind === "smooth" && i.factor).toBeCloseTo(Math.exp(0.045));
  });

  it("a touchpad scroll pans a zoomed photo and leaves a fitted one alone", () => {
    expect(wheelIntent(ev(2.5, 7.25), true)).toEqual({ kind: "pan", dx: 2.5, dy: 7.25 });
    expect(wheelIntent(ev(0, 12), false)).toEqual({ kind: "none" });
    // Even a fast flick, if it moves sideways at all, is a touchpad.
    expect(wheelIntent(ev(1, 120), true).kind).toBe("pan");
  });
});
