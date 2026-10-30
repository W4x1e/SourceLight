import { describe, expect, it } from "vitest";
import { facetClick } from "./facetClick";

describe("facetClick", () => {
  it("chooses and unchooses with a plain click", () => {
    expect(facetClick([], [], "dog", false)).toEqual({ chosen: ["dog"], excluded: [] });
    expect(facetClick(["dog"], [], "dog", false)).toEqual({ chosen: [], excluded: [] });
  });

  it("leaves a value out with Alt, taking it off the chosen list", () => {
    expect(facetClick(["dog", "cat"], [], "dog", true)).toEqual({ chosen: ["cat"], excluded: ["dog"] });
  });

  it("lets a left-out value back in with any click, without choosing it", () => {
    expect(facetClick([], ["dog"], "dog", false)).toEqual({ chosen: [], excluded: [] });
    expect(facetClick([], ["dog"], "dog", true)).toEqual({ chosen: [], excluded: [] });
  });
});
