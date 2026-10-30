import { describe, expect, it } from "vitest";
import { cullScopeKey, cullScopeLabel } from "./cullScope";
import { EMPTY_FILTER } from "../types";

/** What Cull says it is culling, and what a saved position belongs to. */
describe("cull scope", () => {
  it("names the shoot, a search, or says it is the whole library", () => {
    expect(cullScopeLabel({ ...EMPTY_FILTER, shootId: 3 }, "", { shoot: "Smith wedding" })).toBe("Smith wedding");
    expect(cullScopeLabel({ ...EMPTY_FILTER, semantic: "first dance" }, "", {})).toBe("Search: first dance");
    expect(cullScopeLabel(EMPTY_FILTER, "", {})).toBe("Whole library");
    expect(cullScopeLabel({ ...EMPTY_FILTER, minRating: 3 }, "", {})).toBe("Filtered view");
    expect(cullScopeLabel(EMPTY_FILTER, "Similar to DSC01.ARW", {})).toBe("Similar to DSC01.ARW");
  });

  it("keys a position by its set, not by how it is sorted", () => {
    const shoot = { ...EMPTY_FILTER, shootId: 3 };
    expect(cullScopeKey({ ...shoot, sort: "oldest" }, "")).toBe(cullScopeKey({ ...shoot, sort: "newest" }, ""));
    expect(cullScopeKey(shoot, "")).not.toBe(cullScopeKey({ ...EMPTY_FILTER, shootId: 4 }, ""));
  });
});
