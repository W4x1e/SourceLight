import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { coalescer } from "./coalesce";

describe("coalescer", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("passes on only the newest value per key, once per interval", () => {
    const apply = vi.fn();
    const report = coalescer<number>(apply, 100);
    for (let i = 1; i <= 50; i++) report("scan", i);
    report("faces", 7);
    expect(apply).not.toHaveBeenCalled();
    vi.advanceTimersByTime(100);
    expect(apply.mock.calls).toEqual([
      ["scan", 50],
      ["faces", 7],
    ]);
  });

  it("lets a last value through at once, and nothing stale after it", () => {
    const apply = vi.fn();
    const report = coalescer<string>(apply, 100);
    report("scan", "40 of 50");
    report("scan", "done", true);
    expect(apply.mock.calls).toEqual([["scan", "done"]]);
    vi.advanceTimersByTime(500);
    expect(apply).toHaveBeenCalledTimes(1);
  });
});
