import { afterEach, describe, expect, it, vi } from "vitest";
import { formatDate, formatList, formatNumber, setLocale } from ".";

describe("formatters", () => {
  afterEach(async () => {
    vi.restoreAllMocks();
    await setLocale("en");
  });

  it("builds a date formatter once for the same options, and reuses it", () => {
    const opts = { weekday: "long", timeZone: "UTC" } as const;
    formatDate(Date.UTC(2025, 8, 6), opts);
    const built = vi.spyOn(Intl, "DateTimeFormat");
    for (let d = 1; d <= 30; d++) formatDate(Date.UTC(2025, 8, d), { ...opts });
    expect(built).not.toHaveBeenCalled();
    expect(formatDate(Date.UTC(2025, 8, 6), opts)).toBe("Saturday");
  });

  it("keeps each language's formatters apart", async () => {
    const opts = { month: "long", timeZone: "UTC" } as const;
    expect(formatDate(Date.UTC(2025, 8, 6), opts)).toBe("September");
    await setLocale("de");
    expect(formatDate(Date.UTC(2025, 8, 6), opts)).toBe("September");
    await setLocale("fr");
    expect(formatDate(Date.UTC(2025, 8, 6), opts)).toBe("septembre");
    // French puts a narrow no-break space before the sign.
    expect(formatNumber(0.5, { style: "percent" })).toMatch(/^50\s%$/);
    expect(formatList(["a", "b", "c"])).toBe("a, b et c");
  });
});
