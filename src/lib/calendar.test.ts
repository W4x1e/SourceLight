import { describe, expect, it } from "vitest";
import { daysInMonth, monthGrid, parseISO, shiftMonth, toISO } from "./calendar";

describe("parseISO", () => {
  it("reads a valid date and rejects junk", () => {
    expect(parseISO("2026-08-15")).toEqual({ year: 2026, month0: 7, day: 15 });
    expect(parseISO(null)).toBeNull();
    expect(parseISO("")).toBeNull();
    expect(parseISO("2026/08/15")).toBeNull();
    expect(parseISO("2026-13-01")).toBeNull(); // no month 13
    expect(parseISO("2026-08-00")).toBeNull(); // no day 0
  });
});

describe("toISO", () => {
  it("zero-pads month and day", () => {
    expect(toISO(2026, 0, 3)).toBe("2026-01-03");
    expect(toISO(2026, 11, 25)).toBe("2026-12-25");
  });

  it("round-trips with parseISO", () => {
    const p = parseISO("2024-02-29")!; // leap day
    expect(toISO(p.year, p.month0, p.day)).toBe("2024-02-29");
  });
});

describe("daysInMonth", () => {
  it("knows month lengths, including leap February", () => {
    expect(daysInMonth(2024, 1)).toBe(29); // Feb 2024 is a leap year
    expect(daysInMonth(2026, 1)).toBe(28); // Feb 2026 is not
    expect(daysInMonth(2026, 3)).toBe(30); // April
    expect(daysInMonth(2026, 0)).toBe(31); // January
  });
});

describe("monthGrid", () => {
  it("pads to whole weeks and lays days in the right cells", () => {
    // August 2026: the 1st is a Saturday (weekday 6).
    const grid = monthGrid(2026, 7);
    expect(grid.length % 7).toBe(0);
    expect(grid.slice(0, 6).every((c) => c === null)).toBe(true); // Sun–Fri pad
    expect(grid[6]).toBe(1); // Saturday holds the 1st
    expect(grid.filter((c) => c !== null)).toHaveLength(31);
  });
});

describe("shiftMonth", () => {
  it("rolls the year over at both ends", () => {
    expect(shiftMonth(2026, 11, 1)).toEqual({ year: 2027, month0: 0 });
    expect(shiftMonth(2026, 0, -1)).toEqual({ year: 2025, month0: 11 });
    expect(shiftMonth(2026, 5, 0)).toEqual({ year: 2026, month0: 5 });
  });
});
