import type { CurvePt } from "../types";

/**
 * Monotone cubic interpolation (Fritsch–Carlson), mirroring
 * `src-tauri/crates/sl-develop/src/develop/curve.rs`.
 *
 * **This is a deliberate duplication and the one place in the app that has
 * one.** The panel has to draw the curve the renderer will actually apply, and
 * the alternative — asking Rust to sample it — is an IPC round trip per mouse
 * move. Reimplementing the maths is the cheaper of two bad options.
 *
 * The risk is drift: change the Rust spline and this silently keeps drawing the
 * old shape, so the displayed curve stops matching the pixels. `curve.test.ts`
 * pins both sides to the same committed sample values — the Rust test
 * `the_ts_and_rust_splines_agree` asserts the identical numbers — so a change
 * to one that is not mirrored in the other fails a test rather than shipping a
 * lie. If the spline ever gains real complexity, replace this with a sampled
 * table sent once per open rather than growing a second implementation.
 *
 * The constraint being enforced is no overshoot — an unconstrained cubic dips
 * below its neighbours near a sharp change, which on a tone curve reads as
 * inverted tones.
 */

// `noUncheckedIndexedAccess` types every indexed read as `T | undefined`,
// including on typed arrays. A numeric kernel like this cannot absorb that
// without a check per term, so the reads below assert instead — every index is
// bounded by the loop that produced it. Confined to these two functions.
export function tangents(xs: number[], ys: number[]): number[] {
  const n = xs.length;
  const m = new Array<number>(n).fill(0);
  if (n < 2) return m;

  const d = new Array<number>(n - 1).fill(0);
  for (let i = 0; i < n - 1; i++) d[i] = (ys[i + 1]! - ys[i]!) / (xs[i + 1]! - xs[i]!);

  m[0] = d[0]!;
  for (let i = 1; i < n - 1; i++) {
    m[i] = d[i - 1]! * d[i]! <= 0 ? 0 : (d[i - 1]! + d[i]!) / 2;
  }
  m[n - 1] = d[n - 2]!;

  for (let i = 0; i < n - 1; i++) {
    if (Math.abs(d[i]!) < 1e-9) {
      m[i] = 0;
      m[i + 1] = 0;
      continue;
    }
    const a = m[i]! / d[i]!;
    const b = m[i + 1]! / d[i]!;
    const norm = a * a + b * b;
    if (norm > 9) {
      const tau = 3 / Math.sqrt(norm);
      m[i] = tau * a * d[i]!;
      m[i + 1] = tau * b * d[i]!;
    }
  }
  return m;
}

/** Precomputed form of a curve, so drawing 65 points solves the spline once. */
export function prepare(points: CurvePt[]) {
  const xs = points.map((p) => p.x);
  const ys = points.map((p) => p.y);
  return { xs, ys, ms: tangents(xs, ys) };
}

export type Prepared = ReturnType<typeof prepare>;

export function evaluate({ xs, ys, ms }: Prepared, x: number): number {
  if (xs.length < 2) return x;
  const last = xs.length - 1;
  if (x <= xs[0]!) return ys[0]!;
  if (x >= xs[last]!) return ys[last]!;

  let i = 0;
  while (i < last - 1 && xs[i + 1]! < x) i++;

  const h = xs[i + 1]! - xs[i]!;
  const t = (x - xs[i]!) / h;
  const t2 = t * t;
  const t3 = t2 * t;
  return Math.min(
    1,
    Math.max(
      0,
      (2 * t3 - 3 * t2 + 1) * ys[i]! +
        (t3 - 2 * t2 + t) * h * ms[i]! +
        (-2 * t3 + 3 * t2) * ys[i + 1]! +
        (t3 - t2) * h * ms[i + 1]!,
    ),
  );
}
