import { describe, expect, it } from "vitest";
import { evaluate, prepare } from "./curve";
import type { CurvePt } from "../types";

const evalAt = (points: CurvePt[], x: number) => evaluate(prepare(points), x);

describe("curve — parity with the Rust spline", () => {
  // The exact numbers emitted by `curve.rs`'s `the_ts_and_rust_splines_agree`.
  // If this file and that test ever disagree, one implementation drifted from
  // the other and the panel has started drawing a curve the renderer will not
  // apply. Symmetric S through (0.25, 0.15) and (0.75, 0.85).
  const S: CurvePt[] = [
    { x: 0, y: 0 },
    { x: 0.25, y: 0.15 },
    { x: 0.75, y: 0.85 },
    { x: 1, y: 1 },
  ];
  const committed: [number, number][] = [
    [0.1, 0.0504],
    [0.25, 0.15],
    [0.4, 0.3432],
    [0.5, 0.5],
    [0.6, 0.6568],
    [0.75, 0.85],
    [0.9, 0.9496],
  ];

  it.each(committed)("matches Rust at x=%s", (x, y) => {
    expect(evalAt(S, x)).toBeCloseTo(y, 5);
  });
});

describe("curve — properties", () => {
  it("leaves the diagonal untouched", () => {
    const diag: CurvePt[] = [
      { x: 0, y: 0 },
      { x: 1, y: 1 },
    ];
    for (let i = 0; i <= 20; i++) {
      const x = i / 20;
      expect(evalAt(diag, x)).toBeCloseTo(x, 5);
    }
  });

  it("passes exactly through its control points", () => {
    const pts: CurvePt[] = [
      { x: 0, y: 0.05 },
      { x: 0.25, y: 0.15 },
      { x: 0.75, y: 0.9 },
      { x: 1, y: 0.95 },
    ];
    for (const p of pts) expect(evalAt(pts, p.x)).toBeCloseTo(p.y, 4);
  });

  it("never doubles back on a steep curve (the monotone property)", () => {
    // An unconstrained cubic would dip below the flat shadow points and above
    // the plateau; on a tone curve that is inverted tones. Same fixture as the
    // Rust test `a_steep_curve_never_doubles_back`.
    const steep: CurvePt[] = [
      { x: 0, y: 0 },
      { x: 0.4, y: 0.02 },
      { x: 0.5, y: 0.9 },
      { x: 0.6, y: 0.92 },
      { x: 1, y: 1 },
    ];
    const prepared = prepare(steep);
    let prev = evaluate(prepared, 0);
    for (let i = 1; i <= 2000; i++) {
      const y = evaluate(prepared, i / 2000);
      expect(y).toBeGreaterThanOrEqual(prev - 1e-5);
      expect(y).toBeGreaterThanOrEqual(0);
      expect(y).toBeLessThanOrEqual(1);
      prev = y;
    }
  });

  it("moves tones in the dragged direction", () => {
    const lifted: CurvePt[] = [
      { x: 0, y: 0 },
      { x: 0.5, y: 0.65 },
      { x: 1, y: 1 },
    ];
    expect(evalAt(lifted, 0.5)).toBeGreaterThan(0.6);
  });
});
