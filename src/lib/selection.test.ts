import { describe, expect, it } from "vitest";
import { toggleTarget, uniformity } from "./selection";

describe("uniformity", () => {
  it("reports empty for no selection", () => {
    expect(uniformity([])).toEqual({ kind: "empty" });
  });

  it("reports the shared value when every photo agrees", () => {
    expect(uniformity([3, 3, 3])).toEqual({ kind: "uniform", value: 3 });
  });

  it("reports uniform for a single photo", () => {
    expect(uniformity([4])).toEqual({ kind: "uniform", value: 4 });
  });

  it("reports mixed as soon as one differs", () => {
    expect(uniformity([3, 3, 1])).toEqual({ kind: "mixed" });
  });

  // 0 is a real value (no rating, no flag, no label), not an absence.
  it("treats a uniform zero as a value, not as empty", () => {
    expect(uniformity([0, 0])).toEqual({ kind: "uniform", value: 0 });
  });

  it("sees a mix of zero and non-zero as mixed", () => {
    expect(uniformity([0, 2])).toEqual({ kind: "mixed" });
  });

  it("handles negative flags", () => {
    expect(uniformity([-1, -1])).toEqual({ kind: "uniform", value: -1 });
    expect(uniformity([-1, 1])).toEqual({ kind: "mixed" });
  });
});

describe("toggleTarget", () => {
  it("clears when the whole selection already carries the clicked value", () => {
    expect(toggleTarget({ kind: "uniform", value: 3 }, 3)).toBe(0);
  });

  it("assigns when the selection is uniform at a different value", () => {
    expect(toggleTarget({ kind: "uniform", value: 3 }, 5)).toBe(5);
  });

  // The regression this whole module exists for: clicking the value the
  // *focused* photo happened to hold used to clear every selected photo.
  it("assigns rather than clears when the selection is mixed", () => {
    expect(toggleTarget({ kind: "mixed" }, 3)).toBe(3);
  });

  it("assigns for an empty selection", () => {
    expect(toggleTarget({ kind: "empty" }, 2)).toBe(2);
  });

  it("uses the given cleared value for flags", () => {
    expect(toggleTarget({ kind: "uniform", value: -1 }, -1, 0)).toBe(0);
    expect(toggleTarget({ kind: "mixed" }, -1, 0)).toBe(-1);
  });
});
