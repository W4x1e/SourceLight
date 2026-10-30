import { describe, expect, it } from "vitest";
import { tileName } from "./tileName";

const base = {
  filename: "IMG_2231.CR3",
  rating: 0,
  flag: 0,
  colorLabel: 0,
  missing: false,
  edited: false,
  editedPath: null,
  variant: 0,
  variantName: null,
};

describe("tileName", () => {
  it("is the filename alone when there is nothing to say", () => {
    expect(tileName(base)).toBe("IMG_2231.CR3");
  });

  it("carries what the badges show", () => {
    expect(tileName({ ...base, rating: 3, flag: 1, colorLabel: 1, missing: true })).toBe(
      "IMG_2231.CR3, 3 stars, pick, Red label, File missing",
    );
    expect(tileName({ ...base, rating: 1, flag: -1 })).toBe("IMG_2231.CR3, 1 star, reject");
  });

  it("says a copy or an edit", () => {
    expect(tileName({ ...base, edited: true })).toBe("IMG_2231.CR3, edited");
    expect(tileName({ ...base, variant: 1, edited: true })).toBe("IMG_2231.CR3, Copy");
    expect(tileName({ ...base, variant: 2, variantName: "Mono" })).toBe("IMG_2231.CR3, Copy Mono");
  });
});
