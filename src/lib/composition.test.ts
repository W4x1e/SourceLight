import { describe, expect, it } from "vitest";
import { gridFractions, nextGrid, peakSize, peakThreshold } from "./composition";

describe("composition grids", () => {
  it("puts each grid's lines where its rule says", () => {
    expect(gridFractions("thirds")).toEqual([1 / 3, 2 / 3]);
    const [a, b] = gridFractions("golden");
    expect(a).toBeCloseTo(0.382, 3);
    expect(b).toBeCloseTo(0.618, 3);
    expect(gridFractions("centre")).toEqual([0.5]);
    expect(gridFractions("off")).toEqual([]);
  });

  it("cycles through every grid and back to none", () => {
    expect(nextGrid("off")).toBe("thirds");
    expect(nextGrid("thirds")).toBe("golden");
    expect(nextGrid("golden")).toBe("centre");
    expect(nextGrid("centre")).toBe("off");
  });
});

describe("focus peaking", () => {
  it("asks less of detail, and of contrast, the more sensitive it is", () => {
    expect(peakThreshold(0)).toEqual({ share: 0.26, floor: 0.1 });
    const most = peakThreshold(100);
    expect(most.share).toBeCloseTo(0.1, 5);
    expect(most.floor).toBeCloseTo(0.02, 5);
    expect(peakThreshold(25).share).toBeGreaterThan(peakThreshold(75).share);
    expect(peakThreshold(-10)).toEqual(peakThreshold(0));
    expect(peakThreshold(500)).toEqual(peakThreshold(100));
  });

  it("measures a source at its own size unless a texture can't hold it", () => {
    expect(peakSize(6000, 4000)).toEqual({ width: 6000, height: 4000 });
    expect(peakSize(11648, 8736)).toEqual({ width: 8192, height: 6144 });
    expect(peakSize(4000, 6000, 3000)).toEqual({ width: 2000, height: 3000 });
  });
});
