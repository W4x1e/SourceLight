import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { sameFrames, useStable } from "./useStable";

const frame = (id: number, extra: Record<string, unknown> = {}) => ({
  id,
  captureDate: `2025-01-0${id}`,
  sharpness: id * 10,
  rating: 0,
  ...extra,
});

describe("useStable", () => {
  it("keeps the old array while only verdicts change, so what is built from it stays", () => {
    const first = [frame(1), frame(2)];
    const { result, rerender } = renderHook(({ v }) => useStable(v, sameFrames), { initialProps: { v: first } });
    rerender({ v: [frame(1, { rating: 4 }), frame(2)] });
    expect(result.current).toBe(first);
  });

  it("takes the new array when a photo, its date or its focus changes", () => {
    const first = [frame(1), frame(2)];
    const { result, rerender } = renderHook(({ v }) => useStable(v, sameFrames), { initialProps: { v: first } });
    for (const next of [
      [frame(1)],
      [frame(2), frame(1)],
      [frame(1, { captureDate: "2024-12-31" }), frame(2)],
      [frame(1, { sharpness: 3 }), frame(2)],
    ]) {
      rerender({ v: next });
      expect(result.current).toBe(next);
    }
  });
});
