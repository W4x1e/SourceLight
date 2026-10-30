import { describe, expect, it } from "vitest";
import { focalRangeOf, keeperShare, patchFor } from "./insights";

describe("insights", () => {
  it("turns a clicked bar into the rail's own filter", () => {
    expect(patchFor("year", "2024")).toEqual({ dateFrom: "2024-01-01", dateTo: "2024-12-31" });
    expect(patchFor("month", "2024-02")).toEqual({ dateFrom: "2024-02-01", dateTo: "2024-02-29" });
    expect(patchFor("day", "2024-05-14")).toEqual({ dateFrom: "2024-05-14", dateTo: "2024-05-14" });
    expect(patchFor("lens", "FE 85mm F1.8")).toEqual({ lenses: ["FE 85mm F1.8"] });
    expect(patchFor("iso", "401-1600")).toEqual({ isoRanges: ["401-1600"] });
  });

  it("files a focal length under the rail's ranges, edges included", () => {
    expect(focalRangeOf(24)).toBe("0-24");
    expect(focalRangeOf(25)).toBe("25-50");
    expect(focalRangeOf(70)).toBe("51-105");
    expect(focalRangeOf(200)).toBe("106-200");
    expect(focalRangeOf(400)).toBe("201-");
  });

  it("says what share were keepers", () => {
    expect(keeperShare({ count: 8, keepers: 3 })).toBe("38%");
    expect(keeperShare({ count: 0, keepers: 0 })).toBe("");
  });
});
