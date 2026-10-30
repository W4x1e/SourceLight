import { describe, expect, it } from "vitest";
import { pngSize } from "./copyImage";

describe("copying an image", () => {
  it("reads a PNG's size from its header", () => {
    const header = new Uint8Array(24);
    header.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    const view = new DataView(header.buffer);
    view.setUint32(16, 2048);
    view.setUint32(20, 1365);
    expect(pngSize(header)).toEqual({ width: 2048, height: 1365 });
  });

  it("says nothing about something that isn't one", () => {
    expect(pngSize(new Uint8Array([0xff, 0xd8, 0xff]))).toBeNull();
    expect(pngSize(new Uint8Array(30))).toBeNull();
  });
});
