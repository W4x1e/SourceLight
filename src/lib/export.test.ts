import { describe, expect, it } from "vitest";
import { batchSucceeded, batchSummary, exportPlan, outputSize, sidecarReadSummary, sidecarSummary, singleSummary } from "./export";
import type { BatchOutcome } from "../bindings";

const outcome = (o: Partial<BatchOutcome>): BatchOutcome => ({
  written: 0,
  failed: 0,
  errors: [],
  cancelled: false,
  failures: [],
  directory: null,
  ...o,
});

describe("outputSize", () => {
  it("fits the long edge, keeping the shape", () => {
    expect(outputSize(6000, 4000, 2048)).toEqual({ width: 2048, height: 1365 });
    expect(outputSize(4000, 6000, 1200)).toEqual({ width: 800, height: 1200 });
  });

  it("never enlarges, and full size is the photo's own", () => {
    expect(outputSize(1600, 1200, 4096)).toEqual({ width: 1600, height: 1200 });
    expect(outputSize(6000, 4000, null)).toEqual({ width: 6000, height: 4000 });
  });
});

describe("exportPlan", () => {
  const jpeg = { format: "jpeg", quality: 92, depth: "eight", size: 2048, space: "srgb" };

  it("says the pixels, the file type and the colour space of one photo", () => {
    expect(exportPlan(jpeg, { width: 6000, height: 4000 })).toBe("2048 × 1365 px · JPEG, quality 92 · sRGB");
  });

  it("says the bit depth of a PNG or TIFF, and that WebP is lossless", () => {
    expect(exportPlan({ ...jpeg, format: "tiff", depth: "sixteen", space: "adobe-rgb", size: null }, { width: 6000, height: 4000 })).toBe(
      "6000 × 4000 px · TIFF, 16-bit · Adobe RGB",
    );
    expect(exportPlan({ ...jpeg, format: "webp" }, { width: 300, height: 200 })).toBe("300 × 200 px · WEBP, lossless · sRGB");
  });

  it("gives a batch its count and long edge, not one photo's pixels", () => {
    expect(exportPlan(jpeg, null, 12)).toBe("12 photos · Long edge 2048 px · JPEG, quality 92 · sRGB");
    expect(exportPlan({ ...jpeg, size: null }, null, 3)).toBe("3 photos · Full size · JPEG, quality 92 · sRGB");
  });
});

describe("batchSummary", () => {
  it("counts what it wrote, and pluralises", () => {
    expect(batchSummary(outcome({ written: 1 }), 1)).toBe("Exported 1 photo");
    expect(batchSummary(outcome({ written: 12 }), 12)).toBe("Exported 12 photos");
  });

  it("says how far it got when cancelled", () => {
    expect(batchSummary(outcome({ written: 3, cancelled: true }), 10)).toBe(
      "Cancelled — exported 3 of 10",
    );
  });

  it("names the first failure rather than only counting them", () => {
    const o = outcome({ written: 8, failed: 2, errors: ["disk full", "no permission"] });
    expect(batchSummary(o, 10)).toBe("Exported 8, 2 failed — disk full");
  });

  it("survives a failure count with no error text", () => {
    expect(batchSummary(outcome({ written: 1, failed: 1 }), 2)).toBe("Exported 1, 1 failed — ");
  });
});

describe("batchSucceeded", () => {
  it("is true only for a complete, clean run", () => {
    expect(batchSucceeded(outcome({ written: 5 }))).toBe(true);
    expect(batchSucceeded(outcome({ written: 5, failed: 1 }))).toBe(false);
    expect(batchSucceeded(outcome({ written: 5, cancelled: true }))).toBe(false);
  });
});

describe("sidecarSummary", () => {
  it("counts what it wrote, and pluralises", () => {
    expect(sidecarSummary(outcome({ written: 1 }))).toBe("Wrote 1 sidecar");
    expect(sidecarSummary(outcome({ written: 40 }))).toBe("Wrote 40 sidecars");
  });

  it("names the first failure rather than silently under-counting", () => {
    const o = outcome({ written: 8, failed: 2, errors: ["a.arw: the disk is full"] });
    expect(sidecarSummary(o)).toBe("Wrote 8, 2 failed — a.arw: the disk is full");
  });

  it("survives a failure count with no error text", () => {
    expect(sidecarSummary(outcome({ written: 1, failed: 1 }))).toBe("Wrote 1, 1 failed — ");
  });
});

describe("sidecarReadSummary", () => {
  it("says when there was nothing to read", () => {
    expect(sidecarReadSummary({ read: 0, people: 0, kept: 0, times: [], verdictsChanged: 0 })).toBe("No .xmp sidecars found");
  });

  it("counts what it read, and what it did to People", () => {
    expect(sidecarReadSummary({ read: 1, people: 0, kept: 0, times: [], verdictsChanged: 0 })).toBe("Read 1 sidecar");
    expect(sidecarReadSummary({ read: 12, people: 3, kept: 0, times: [], verdictsChanged: 0 })).toBe(
      "Read 12 sidecars · 3 names added to People",
    );
    // What it overwrote, and that Undo takes it back.
    expect(sidecarReadSummary({ read: 12, people: 0, kept: 0, times: [], verdictsChanged: 5 })).toBe(
      "Read 12 sidecars · 5 ratings, labels or picks changed (Undo puts them back)",
    );
  });

  it("names the faces it left alone rather than dropping them silently", () => {
    expect(sidecarReadSummary({ read: 2, people: 1, kept: 1, times: [], verdictsChanged: 0 })).toBe(
      "Read 2 sidecars · 1 name added to People · 1 face already has another name, kept",
    );
    expect(sidecarReadSummary({ read: 2, people: 0, kept: 2, times: [], verdictsChanged: 0 })).toBe(
      "Read 2 sidecars · 2 faces already have another name, kept",
    );
  });
});

describe("singleSummary", () => {
  it("names the file and its size, in units a person reads", () => {
    // The old string was `Wrote 5.7 MB` — no filename, and hand-rolled from
    // `bytes / 1024 / 1024` with `humanBytes` one import away.
    expect(singleSummary("DSC01234.jpg", 5_976_883)).toBe("Exported DSC01234.jpg · 6 MB");
    expect(singleSummary("a.tif", 2_147_483_648)).toBe("Exported a.tif · 2.0 GB");
  });
});
