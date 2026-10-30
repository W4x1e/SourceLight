import { describe, expect, it } from "vitest";
import { humanBytes, sentenceCase } from "./format";

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;

describe("humanBytes", () => {
  it("keeps small counts in bytes", () => {
    expect(humanBytes(0)).toBe("0 B");
    expect(humanBytes(512)).toBe("512 B");
  });

  it("steps up at each unit boundary rather than one short of it", () => {
    expect(humanBytes(1023)).toBe("1023 B");
    expect(humanBytes(KB)).toBe("1 KB");
    expect(humanBytes(1023 * KB)).toBe("1023 KB");
    expect(humanBytes(MB)).toBe("1 MB");
    expect(humanBytes(1023 * MB)).toBe("1023 MB");
    expect(humanBytes(GB)).toBe("1.0 GB");
  });

  it("formats the sizes the model catalogue actually quotes", () => {
    // The CLIP tagger (608 MB) and the largest vision model (6 GB).
    expect(humanBytes(608 * MB)).toBe("608 MB");
    expect(humanBytes(6 * GB)).toBe("6.0 GB");
  });

  it("gives gigabytes one decimal so a slow pull still visibly moves", () => {
    expect(humanBytes(1.4 * GB)).toBe("1.4 GB");
  });

  // A progress event can arrive before the total is known, and the placeholder
  // bars start at 0; neither should render as "NaN".
  it("returns a dash rather than NaN for a missing or negative count", () => {
    expect(humanBytes(NaN)).toBe("—");
    expect(humanBytes(-1)).toBe("—");
    expect(humanBytes(Infinity)).toBe("—");
  });
});

/** Machine labels capitalised by their first letter only. */
describe("sentenceCase", () => {
  it("capitalises the first letter and leaves the rest as written", () => {
    expect(sentenceCase("coucher de soleil")).toBe("Coucher de soleil");
    expect(sentenceCase("")).toBe("");
    expect(sentenceCase("vue d’ensemble")).toBe("Vue d’ensemble");
  });
});
