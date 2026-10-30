import { describe, expect, it } from "vitest";
import { dayWindow, fillPeriods, inRange, periodLabel, periodRange, spanDays } from "./timeline";

describe("timeline strip", () => {
  it("fills the months between, gaps at zero, across a year's end", () => {
    const bars = fillPeriods(
      [
        { period: "2023-11", count: 4 },
        { period: "2024-02", count: 9 },
      ],
      null,
    );
    expect(bars.map((b) => `${b.period}:${b.count}`)).toEqual(["2023-11:4", "2023-12:0", "2024-01:0", "2024-02:9"]);
  });

  it("draws every day of a short range, leap day included", () => {
    const bars = fillPeriods([{ period: "2024-03-01", count: 2 }], { from: "2024-02-27", to: "2024-03-01" });
    expect(bars.map((b) => b.period)).toEqual(["2024-02-27", "2024-02-28", "2024-02-29", "2024-03-01"]);
    expect(bars.at(-1)!.count).toBe(2);
  });

  it("turns to days at three months or less", () => {
    expect(spanDays("2024-05-01", "2024-05-31")).toBe(31);
    expect(dayWindow("2024-05-01", "2024-07-31")).toEqual({ from: "2024-05-01", to: "2024-07-31" });
    expect(dayWindow("2024-01-01", "2024-12-31")).toBeNull();
    expect(dayWindow("2024-05-01", null)).toBeNull();
  });

  it("knows what a period spans and says it", () => {
    expect(periodRange("2024-02")).toEqual({ from: "2024-02-01", to: "2024-02-29" });
    expect(periodRange("2023-02")).toEqual({ from: "2023-02-01", to: "2023-02-28" });
    expect(periodRange("2024-05-14")).toEqual({ from: "2024-05-14", to: "2024-05-14" });
    expect(periodLabel("2024-05")).toBe("May 2024");
    expect(periodLabel("2024-05-14")).toBe("Tue 14 May 2024");
  });

  it("places a period against a chosen range", () => {
    expect(inRange("2024-05", "2024-05-20", null)).toBe(true);
    expect(inRange("2024-04", "2024-05-01", "2024-06-30")).toBe(false);
    expect(inRange("2024-07", null, "2024-06-30")).toBe(false);
    expect(inRange("2024-07", null, null)).toBe(true);
  });
});
