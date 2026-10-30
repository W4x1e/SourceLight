import { describe, expect, it } from "vitest";
import {
  groupByCountry,
  namedMessage,
  otherPlaces,
  shortName,
  tripDates,
  tripKey,
  tripLength,
  tripSuggestion,
} from "./PlacesWorkspace";

/**
 * The date formatting is the part of the trips card that is easy to get wrong
 * in a way nobody notices — a trip spanning a month or a year boundary is rare
 * enough in testing and common enough in a real library.
 */
describe("tripDates", () => {
  it("says a single day once", () => {
    expect(tripDates("2026-09-06 10:00:00", "2026-09-06 18:30:00")).toBe("6 Sep 2026");
  });

  it("collapses a run inside one month", () => {
    expect(tripDates("2025-08-12 09:00:00", "2025-08-15 20:00:00")).toBe("12–15 Aug 2025");
  });

  it("names both months when a trip crosses one", () => {
    expect(tripDates("2025-08-30 09:00:00", "2025-09-02 20:00:00")).toBe("30 Aug – 2 Sep 2025");
  });

  it("names both years when a trip crosses new year", () => {
    expect(tripDates("2025-12-29 09:00:00", "2026-01-03 20:00:00")).toBe(
      "29 Dec 2025 – 3 Jan 2026",
    );
  });
});

describe("tripLength", () => {
  it("counts inclusive days, so a single day is one", () => {
    expect(tripLength("2026-09-06 08:00:00", "2026-09-06 22:00:00")).toBe("one day");
    expect(tripLength("2025-08-12 09:00:00", "2025-08-15 20:00:00")).toBe("4 days");
  });

  it("counts across a month and a year boundary", () => {
    expect(tripLength("2025-08-30 09:00:00", "2025-09-02 20:00:00")).toBe("4 days");
    expect(tripLength("2025-12-31 09:00:00", "2026-01-01 20:00:00")).toBe("2 days");
  });

  it("is unaffected by daylight saving, because it works in UTC", () => {
    // Local-time arithmetic across a DST change gives 23 or 25 hours and rounds
    // to the wrong day count. UK clocks went forward on 2026-03-29.
    expect(tripLength("2026-03-28 12:00:00", "2026-03-30 12:00:00")).toBe("3 days");
  });
});

describe("tripKey", () => {
  it("identifies a trip by its span, not its place in the list", () => {
    const a = { startDate: "2025-08-12 09:00:00", endDate: "2025-08-15 20:00:00" };
    const b = { startDate: "2025-09-01 09:00:00", endDate: "2025-09-01 18:00:00" };
    // The same trip keys the same wherever the list puts it...
    expect(tripKey({ ...a })).toBe(tripKey(a));
    // ...and two trips never collide.
    expect(tripKey(a)).not.toBe(tripKey(b));
  });
});

describe("namedMessage", () => {
  it("says what was placed", () => {
    expect(namedMessage("Skye", 12, 0)).toBe("Placed 12 photos in Skye.");
  });

  it("owns up to the photos it left where their GPS put them", () => {
    expect(namedMessage("Skye", 12, 2)).toBe(
      "Placed 12 photos in Skye. 2 photos with their own GPS kept their place.",
    );
    expect(namedMessage("Skye", 1, 1)).toBe(
      "Placed 1 photo in Skye. 1 photo with its own GPS kept its place.",
    );
  });
});

describe("otherPlaces", () => {
  const p = (name: string, count = 1) => ({ name, count });

  it("leaves out the trip's own name", () => {
    expect(otherPlaces({ existingPlace: "Skye", places: [p("Skye", 9)] })).toBeNull();
    expect(otherPlaces({ existingPlace: "Skye", places: [p("Skye", 9), p("Edinburgh")] })).toBe("Edinburgh");
  });

  it("counts the rest rather than listing them", () => {
    expect(otherPlaces({ existingPlace: null, places: [p("Edinburgh"), p("Inverness"), p("Fort William")] })).toBe(
      "Edinburgh + 2 more",
    );
  });
});

describe("tripSuggestion", () => {
  const trip = { existingPlace: null, places: [{ name: "Edinburgh, United Kingdom", count: 3 }], located: 3, count: 14 };

  it("offers where the trip's GPS photos were taken", () => {
    expect(tripSuggestion(trip)).toBe("Edinburgh, United Kingdom");
  });

  it("offers nothing once named, with no GPS, or with nothing left to place", () => {
    expect(tripSuggestion({ ...trip, existingPlace: "Skye" })).toBeNull();
    expect(tripSuggestion({ ...trip, located: 0, places: [] })).toBeNull();
    expect(tripSuggestion({ ...trip, located: 14 })).toBeNull();
  });
});

describe("shortName", () => {
  it("drops the country a heading already gives", () => {
    expect(shortName("Edinburgh, United Kingdom", "United Kingdom")).toBe("Edinburgh");
    expect(shortName("Karoo", "South Africa")).toBe("Karoo");
  });

  it("takes the last part as the country when none is given", () => {
    expect(shortName("Edinburgh, United Kingdom")).toBe("Edinburgh");
    expect(shortName("Karoo")).toBe("Karoo");
  });
});

describe("groupByCountry", () => {
  it("puts the most-photographed country first and the unknown last", () => {
    const groups = groupByCountry([
      { country: "United Kingdom", count: 40 },
      { country: null, count: 90 },
      { country: "South Africa", count: 12 },
      { country: "South Africa", count: 35 },
    ]);
    expect(groups.map((g) => [g.country, g.count])).toEqual([
      ["South Africa", 47],
      ["United Kingdom", 40],
      [null, 90],
    ]);
  });
});
