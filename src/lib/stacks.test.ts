import { describe, expect, it } from "vitest";
import { buildStackCells } from "./stacks";
import type { Photo } from "../types";

/** A photo row with only the fields stacking looks at. */
const photo = (id: number): Photo => ({ id, filename: `IMG_${id}.ARW` }) as Photo;
const photos = (...ids: number[]): Photo[] => ids.map(photo);

/** Compact view of a cell list: id, plus a stack marker when present. */
const shape = (cells: ReturnType<typeof buildStackCells>) =>
  cells.map((c) =>
    c.stack ? { id: c.photo.id, head: c.stack.head, count: c.stack.count } : { id: c.photo.id },
  );

describe("buildStackCells", () => {
  it("wraps every photo one per cell when there are no groups", () => {
    expect(shape(buildStackCells(photos(1, 2, 3), [], new Set()))).toEqual([
      { id: 1 },
      { id: 2 },
      { id: 3 },
    ]);
  });

  it("collapses a group to a single head cell and hides the rest", () => {
    const cells = buildStackCells(photos(1, 2, 3, 4), [[2, 3]], new Set());
    expect(shape(cells)).toEqual([
      { id: 1 },
      { id: 2, head: true, count: 2 },
      { id: 4 },
    ]);
  });

  it("sits the stack where its first member appears, dropping later members' slots", () => {
    // 2 and 5 are a burst; the stack lands at 2's position and 5 vanishes from
    // between 4 and 6.
    const cells = buildStackCells(photos(1, 2, 3, 4, 5, 6), [[2, 5]], new Set());
    expect(shape(cells)).toEqual([
      { id: 1 },
      { id: 2, head: true, count: 2 },
      { id: 3 },
      { id: 4 },
      { id: 6 },
    ]);
  });

  it("expands a stack inline, members contiguous after the head", () => {
    const cells = buildStackCells(photos(1, 2, 3, 4), [[2, 4]], new Set([2]));
    expect(shape(cells)).toEqual([
      { id: 1 },
      { id: 2, head: true, count: 2 },
      { id: 4, head: false, count: 2 },
      { id: 3 },
    ]);
  });

  it("shows the group's first frame — its sharpest — where its earliest frame sat", () => {
    // Stacking orders each group sharpest first: 5 is the best frame, 2 comes
    // first on screen. The stack sits at 2's place and shows 5.
    const cells = buildStackCells(photos(1, 2, 3, 5), [[5, 2]], new Set());
    expect(shape(cells)).toEqual([{ id: 1 }, { id: 5, head: true, count: 2 }, { id: 3 }]);
    const open = buildStackCells(photos(1, 2, 3, 5), [[5, 2]], new Set([5]));
    expect(shape(open)).toEqual([
      { id: 1 },
      { id: 5, head: true, count: 2 },
      { id: 2, head: false, count: 2 },
      { id: 3 },
    ]);
  });

  it("ignores members no longer on screen and groups that fall under two", () => {
    // 9 was indexed but filtered out of the grid, leaving [3] — not a stack.
    const cells = buildStackCells(photos(1, 3), [[3, 9]], new Set());
    expect(shape(cells)).toEqual([{ id: 1 }, { id: 3 }]);
  });

  it("counts only the on-screen members", () => {
    // Group of three, but one (7) is off screen: the badge should read 2.
    const cells = buildStackCells(photos(6, 8), [[6, 7, 8]], new Set());
    expect(shape(cells)).toEqual([{ id: 6, head: true, count: 2 }]);
  });
});
