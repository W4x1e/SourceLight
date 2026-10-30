import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useHeld, useLeavingList, usePresence } from "./usePresence";

describe("usePresence", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("mounts at once when opened", () => {
    const { result, rerender } = renderHook(({ open }) => usePresence(open, 100), {
      initialProps: { open: false },
    });
    expect(result.current).toEqual({ mounted: false, closing: false });
    rerender({ open: true });
    expect(result.current).toEqual({ mounted: true, closing: false });
  });

  it("stays mounted and closing for the exit, then unmounts", () => {
    const { result, rerender } = renderHook(({ open }) => usePresence(open, 100), {
      initialProps: { open: true },
    });
    rerender({ open: false });
    expect(result.current).toEqual({ mounted: true, closing: true });
    act(() => vi.advanceTimersByTime(99));
    expect(result.current.mounted).toBe(true);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toEqual({ mounted: false, closing: false });
  });

  it("reopening mid-exit keeps it mounted and cancels the unmount", () => {
    const { result, rerender } = renderHook(({ open }) => usePresence(open, 100), {
      initialProps: { open: true },
    });
    rerender({ open: false });
    act(() => vi.advanceTimersByTime(50));
    rerender({ open: true });
    act(() => vi.advanceTimersByTime(200));
    expect(result.current).toEqual({ mounted: true, closing: false });
  });
});

describe("useHeld", () => {
  it("keeps the last value while it goes null, and follows a new one", () => {
    const { result, rerender } = renderHook(({ v }: { v: string | null }) => useHeld(v), {
      initialProps: { v: "first" as string | null },
    });
    expect(result.current).toBe("first");
    rerender({ v: null });
    expect(result.current).toBe("first");
    rerender({ v: "second" });
    expect(result.current).toBe("second");
  });
});

describe("useLeavingList", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  const key = (n: number) => n;

  it("keeps a removed item where it stood, closing, for the exit", () => {
    const { result, rerender } = renderHook(({ items }) => useLeavingList(items, key, 100), {
      initialProps: { items: [1, 2, 3] },
    });
    rerender({ items: [1, 3] });
    expect(result.current).toEqual([
      { item: 1, closing: false },
      { item: 2, closing: true },
      { item: 3, closing: false },
    ]);
    act(() => vi.advanceTimersByTime(100));
    expect(result.current.map((e) => e.item)).toEqual([1, 3]);
  });

  it("brings an item back, not closing, if it returns mid-exit", () => {
    const { result, rerender } = renderHook(({ items }) => useLeavingList(items, key, 100), {
      initialProps: { items: [1, 2] },
    });
    rerender({ items: [1] });
    rerender({ items: [1, 2] });
    expect(result.current).toEqual([
      { item: 1, closing: false },
      { item: 2, closing: false },
    ]);
    act(() => vi.advanceTimersByTime(200));
    expect(result.current.map((e) => e.item)).toEqual([1, 2]);
  });
});
