import { describe, expect, it } from "vitest";
import { ANNOUNCE_AFTER_MS, overallPercent, shouldAnnounceFinish } from "./dockProgress";

describe("Dock progress", () => {
  it("is hidden when nothing runs", () => {
    expect(overallPercent([])).toBeNull();
  });

  it("averages the running jobs, each counting equally", () => {
    expect(overallPercent([0.5])).toBe(50);
    expect(overallPercent([1, 0])).toBe(50);
    expect(overallPercent([0.25, 0.75, 0.5])).toBe(50);
  });

  it("counts a job with no total yet as not started", () => {
    expect(overallPercent([NaN, 1])).toBe(50);
  });

  it("asks for attention only when long work finishes behind another app", () => {
    expect(shouldAnnounceFinish(ANNOUNCE_AFTER_MS, false)).toBe(true);
    expect(shouldAnnounceFinish(ANNOUNCE_AFTER_MS, true)).toBe(false);
    expect(shouldAnnounceFinish(ANNOUNCE_AFTER_MS - 1, false)).toBe(false);
  });
});
