import { describe, expect, it } from "vitest";
import { NO_SELECTION, decodeCoverage, hasSelection, selectionCoverage, selectionMask } from "./eraseSelection";
import type { Mask } from "../types";

/** `mask::encode_ai`: base64 of (run, value) byte pairs. */
const encode = (data: number[]) => {
  let bytes = "";
  for (let i = 0; i < data.length; ) {
    let run = 1;
    while (i + run < data.length && data[i + run] === data[i] && run < 255) run++;
    bytes += String.fromCharCode(run, data[i]!);
    i += run;
  }
  return btoa(bytes);
};

/** A 4 x 2 AI selection of its right half. */
const picked: Mask = {
  components: [
    {
      shape: { type: "ai", label: "Person", width: 4, height: 2, rle: encode([0, 0, 255, 255, 0, 0, 255, 255]) },
      inverted: false,
      mode: "add",
    },
  ],
};

const dab = (x: number, y: number) => ({ x, y, radius: 0.1, hardness: 1, erase: false });

describe("an erase selection", () => {
  it("is empty until something is picked or brushed on", () => {
    expect(hasSelection(NO_SELECTION)).toBe(false);
    expect(selectionMask(NO_SELECTION)).toBeNull();
    // Brushing off nothing is still nothing.
    expect(selectionMask({ ...NO_SELECTION, remove: [dab(0.5, 0.5)] })).toBeNull();
    expect(hasSelection({ ...NO_SELECTION, picked })).toBe(true);
    expect(hasSelection({ ...NO_SELECTION, add: [dab(0.5, 0.5)] })).toBe(true);
  });

  it("goes to the backend as the picked object, plus and minus the brush", () => {
    const mask = selectionMask({ points: [], picked, add: [dab(0.1, 0.1)], remove: [dab(0.9, 0.9)] })!;
    expect(mask.components.map((c) => [c.shape.type, c.mode])).toEqual([
      ["ai", "add"],
      ["brush", "add"],
      ["brush", "subtract"],
    ]);
  });

  it("reads an AI selection's stored coverage", () => {
    const shape = picked.components[0]!.shape;
    expect([...decodeCoverage(shape.type === "ai" ? shape.rle : "", 4, 2)]).toEqual([0, 0, 255, 255, 0, 0, 255, 255]);
    // Malformed or short data is empty, never a throw.
    expect([...decodeCoverage("not base64!", 2, 1)]).toEqual([0, 0]);
    expect([...decodeCoverage(btoa(String.fromCharCode(1, 9)), 3, 1)]).toEqual([9, 0, 0]);
  });

  it("is drawn where it will be erased", () => {
    const cov = selectionCoverage({ points: [], picked, add: [], remove: [] }, 40, 20);
    expect(cov[10 * 40 + 5]).toBe(0);
    expect(cov[10 * 40 + 35]).toBe(1);
    // Brushed on at the left, brushed off at the right.
    const edited = selectionCoverage({ points: [], picked, add: [dab(0.1, 0.5)], remove: [dab(0.9, 0.5)] }, 40, 20);
    expect(edited[10 * 40 + 4]).toBeGreaterThan(0.9);
    expect(edited[10 * 40 + 36]).toBeLessThan(0.1);
    expect(edited[10 * 40 + 24]).toBe(1);
  });
});
