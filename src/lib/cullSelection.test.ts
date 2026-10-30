import { describe, expect, it } from "vitest";
import { clickSelect, flagForAll } from "./cullSelection";
import type { Photo } from "../types";

const IDS = [10, 11, 12, 13, 14];
const none = new Set<number>();
const sorted = (s: Set<number>) => [...s].sort((a, b) => a - b);

describe("choosing frames in the survey", () => {
  it("chooses nothing on a plain click", () => {
    expect(sorted(clickSelect(new Set([11, 12]), IDS, 0, 3, { toggle: false, range: false }))).toEqual([]);
  });

  it("adds the frame being looked at on the first Ctrl-click, then toggles", () => {
    const one = clickSelect(none, IDS, 0, 2, { toggle: true, range: false });
    expect(sorted(one)).toEqual([10, 12]);
    const two = clickSelect(one, IDS, 2, 2, { toggle: true, range: false });
    expect(sorted(two)).toEqual([10]);
    const three = clickSelect(two, IDS, 2, 4, { toggle: true, range: false });
    expect(sorted(three)).toEqual([10, 14]);
  });

  it("chooses a run with Shift, either way, and adds it with Ctrl too", () => {
    expect(sorted(clickSelect(none, IDS, 1, 3, { toggle: false, range: true }))).toEqual([11, 12, 13]);
    expect(sorted(clickSelect(none, IDS, 3, 1, { toggle: false, range: true }))).toEqual([11, 12, 13]);
    expect(sorted(clickSelect(new Set([14]), IDS, 0, 1, { toggle: true, range: true }))).toEqual([10, 11, 14]);
    expect(sorted(clickSelect(new Set([14]), IDS, 0, 1, { toggle: false, range: true }))).toEqual([10, 11]);
  });

  it("takes a flag off every frame only when they all have it", () => {
    const p = (flag: number) => ({ flag }) as Photo;
    expect(flagForAll([p(1), p(1)], 1)).toBe(0);
    expect(flagForAll([p(1), p(0)], 1)).toBe(1);
    expect(flagForAll([p(-1), p(1)], -1)).toBe(-1);
    expect(flagForAll([], 1)).toBe(1);
  });
});
