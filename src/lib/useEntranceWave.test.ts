import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useEntranceWave } from "./useEntranceWave";

const list = (...ids: number[]) => ids.map((id) => ({ id, rating: 0 }));

describe("useEntranceWave", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const mount = (photos: { id: number; rating: number }[]) =>
    renderHook(({ photos }) => useEntranceWave(photos, 700), { initialProps: { photos } });

  it("plays when the set of photos changes, then stops", () => {
    const { result, rerender } = mount(list(1, 2));
    expect(result.current).toBe(false);
    rerender({ photos: list(3, 4) });
    expect(result.current).toBe(true);
    act(() => vi.advanceTimersByTime(700));
    expect(result.current).toBe(false);
  });

  it("does not play for a rating on the same photos", () => {
    const before = list(1, 2, 3);
    const { result, rerender } = mount(before);
    rerender({ photos: before.map((p) => (p.id === 2 ? { ...p, rating: 3 } : p)) });
    expect(result.current).toBe(false);
  });

  it("still stops on time when a rating lands mid-wave", () => {
    const { result, rerender } = mount(list(1, 2));
    const shown = list(3, 4);
    rerender({ photos: shown });
    act(() => vi.advanceTimersByTime(300));
    rerender({ photos: shown.map((p) => ({ ...p, rating: 5 })) });
    expect(result.current).toBe(true);
    act(() => vi.advanceTimersByTime(400));
    expect(result.current).toBe(false);
  });
});
