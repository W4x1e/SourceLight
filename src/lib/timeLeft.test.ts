import { describe, expect, it } from "vitest";
import { secondsLeft, timeLeftText } from "./timeLeft";

describe("secondsLeft", () => {
  const start = { at: 0, done: 10 };

  it("waits for a pace worth quoting", () => {
    // Two photos in: the first carried the model's load.
    expect(secondsLeft(start, { at: 60_000, done: 12 }, 100)).toBeNull();
    // Three photos, but in under five seconds.
    expect(secondsLeft(start, { at: 4_000, done: 13 }, 100)).toBeNull();
  });

  it("projects the pace since it started being measured", () => {
    // 10 photos in 20 s is 2 s a photo; 80 are left.
    expect(secondsLeft(start, { at: 20_000, done: 20 }, 100)).toBe(160);
  });

  it("says nothing once everything is done", () => {
    expect(secondsLeft(start, { at: 20_000, done: 100 }, 100)).toBeNull();
  });
});

describe("timeLeftText", () => {
  it("rounds to what a person can use", () => {
    expect(timeLeftText(42)).toBe("About 40 s left");
    expect(timeLeftText(2)).toBe("About 5 s left");
    expect(timeLeftText(12 * 60 + 20)).toBe("About 12 min left");
    expect(timeLeftText(80 * 60)).toBe("About 80 min left");
    expect(timeLeftText(3 * 3600 + 600)).toBe("About 3 h left");
  });
});
