import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import { describeCapture, earliest, formatCapture, parseCapture, shiftSeconds, toInputValue } from "./captureTime";

describe("capture times", () => {
  it("reads and writes the catalogue's form without moving the clock", () => {
    const d = parseCapture("2025-06-14 10:00:05")!;
    expect(formatCapture(d)).toBe("2025-06-14 10:00:05");
    expect(toInputValue(d)).toBe("2025-06-14T10:00:05");
    // A date-time field's own form, with and without seconds.
    expect(formatCapture(parseCapture("2025-06-14T10:30")!)).toBe("2025-06-14 10:30:00");
    expect(parseCapture(null)).toBeNull();
    expect(parseCapture("soon")).toBeNull();
  });

  it("totals a shift in seconds, earlier as negative", () => {
    expect(shiftSeconds({ later: true, days: 1, hours: 2, minutes: 3, seconds: 4 })).toBe(93784);
    expect(shiftSeconds({ later: false, days: 0, hours: 1, minutes: 0, seconds: 0 })).toBe(-3600);
    expect(shiftSeconds({ later: true, days: NaN, hours: 1.9, minutes: 0, seconds: 0 })).toBe(3600);
  });

  it("finds the earliest of a selection, skipping photos with no time", () => {
    const first = earliest([
      { captureDate: "2025-06-14 10:05:00" },
      { captureDate: null },
      { captureDate: "2025-06-14 09:59:59" },
    ]);
    expect(formatCapture(first!)).toBe("2025-06-14 09:59:59");
    expect(earliest([{ captureDate: null }])).toBeNull();
  });

  describe("said for people", () => {
    afterEach(async () => {
      await setLocale("en");
    });

    it("reads day first on a 24-hour clock in English, as the grid does", () => {
      expect(describeCapture(parseCapture("2025-06-14 15:04:05"))).toBe("14 Jun 2025, 15:04:05");
      expect(describeCapture(parseCapture("2025-01-03 09:02:00"))).toBe("3 Jan 2025, 09:02:00");
    });

    it("leaves other languages to their own order", async () => {
      await setLocale("de");
      expect(describeCapture(parseCapture("2025-06-14 15:04:05"))).toMatch(/^14\. Juni? 2025/);
    });
  });
});
