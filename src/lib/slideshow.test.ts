import { describe, expect, it } from "vitest";
import { captionFor, playOrder, stepFrom, wantsFull } from "./slideshow";

describe("slideshow", () => {
  it("plays in the grid's order from the photo asked for, round to the start", () => {
    expect(playOrder([1, 2, 3, 4], 2, false)).toEqual([3, 4, 1, 2]);
    expect(playOrder([1, 2, 3], 9, false)).toEqual([3, 1, 2]);
    expect(playOrder([], 0, false)).toEqual([]);
  });

  it("shuffles the rest but starts where asked", () => {
    let seed = 3;
    const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    const out = playOrder([1, 2, 3, 4, 5, 6], 3, true, random);
    expect(out[0]).toBe(4);
    expect([...out].sort()).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("steps round the end only when looping", () => {
    expect(stepFrom(2, 3, 1, true)).toBe(0);
    expect(stepFrom(2, 3, 1, false)).toBeNull();
    expect(stepFrom(0, 3, -1, true)).toBe(2);
    expect(stepFrom(0, 3, -1, false)).toBeNull();
    expect(stepFrom(1, 3, 1, false)).toBe(2);
  });

  it("captions a photo by name, date or place", () => {
    const p = { filename: "DSC01001.ARW", captureDate: "2025-06-14 10:00:00", place: "Cape Point" };
    expect(captionFor(p, "name")).toBe("DSC01001.ARW");
    expect(captionFor(p, "date")).toBe("14 June 2025");
    expect(captionFor(p, "place")).toBe("Cape Point");
    expect(captionFor(p, "none")).toBeNull();
    expect(captionFor({ ...p, captureDate: null }, "date")).toBeNull();
  });

  it("fetches full resolution only where the screen shows it", () => {
    expect(wantsFull(1920, 1080, 1)).toBe(true);
    expect(wantsFull(1440, 900, 1)).toBe(false);
    expect(wantsFull(3840, 2160, 1)).toBe(true);
  });
});
