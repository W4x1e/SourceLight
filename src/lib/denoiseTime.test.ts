import { describe, expect, it } from "vitest";
import { estimateFor, formatDuration, paceOf, secondsLeft } from "./denoiseTime";

describe("how long AI noise reduction takes", () => {
  it("reads as seconds, then minutes", () => {
    expect(formatDuration(0.2)).toBe("1 s");
    expect(formatDuration(12.4)).toBe("12 s");
    expect(formatDuration(59)).toBe("59 s");
    expect(formatDuration(95)).toBe("2 min");
    expect(formatDuration(-1)).toBe("");
  });

  it("estimates what is left from the pace since the first tile", () => {
    const start = { at: 10_000, done: 1 };
    expect(secondsLeft(start, { at: 10_500, done: 2 }, 100)).toBeNull(); // one tile timed
    // 10 tiles in 5 s, 89 to go.
    expect(secondsLeft(start, { at: 15_000, done: 11 }, 100)).toBeCloseTo(44.5);
  });

  it("remembers a pace and scales it to the next photo", () => {
    const pace = paceOf(10, 6000, 4000); // 10 s for 24 MP
    expect(pace).toBeCloseTo(10 / 24);
    expect(estimateFor(pace, 3000, 2000)).toBeCloseTo(2.5);
    expect(estimateFor(null, 3000, 2000)).toBeNull();
    expect(paceOf(0, 6000, 4000)).toBeNull();
  });
});
