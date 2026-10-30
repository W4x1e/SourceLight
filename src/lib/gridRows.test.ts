import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale } from "../i18n";
import {
  buildGridRows,
  cellRowIndex,
  dayLabel,
  monthLabel,
  monthLabelAt,
  sameIds,
  type GridRow,
} from "./gridRows";
import type { GridCell } from "./stacks";
import type { Photo } from "../types";

/** A cell carrying only what the row builder reads. */
function cell(id: number, captureDate: string | null): GridCell {
  return { photo: { id, captureDate } as Photo };
}

const tiles = (rows: GridRow[]) => rows.filter((r) => r.kind === "photos");
const headings = (rows: GridRow[]) =>
  rows.filter((r) => r.kind !== "photos").map((r) => (r as { label: string }).label);

describe("buildGridRows", () => {
  it("chunks into rows of `cols` when grouping is off", () => {
    const cells = [1, 2, 3, 4, 5].map((i) => cell(i, "2026-09-06T10:00:00"));
    const rows = buildGridRows(cells, 2, false);
    expect(rows).toHaveLength(3);
    expect(tiles(rows).map((r) => r.cells.length)).toEqual([2, 2, 1]);
    expect(headings(rows)).toEqual([]);
  });

  it("puts a month heading above each month and a day heading above each day", () => {
    const cells = [
      cell(1, "2026-09-06T10:00:00"),
      cell(2, "2026-09-06T11:00:00"),
      cell(3, "2026-09-05T09:00:00"),
      cell(4, "2026-08-30T09:00:00"),
    ];
    expect(headings(buildGridRows(cells, 4, true))).toEqual([
      "September 2026",
      "Sun 6 September",
      "Sat 5 September",
      "August 2026",
      "Sun 30 August",
    ]);
  });

  it("starts each day on a fresh row rather than filling the previous one", () => {
    // Three photos on one day, one on the next, at four columns. Without the
    // break the second day's tile would ride along in the first day's row,
    // under the wrong heading.
    const cells = [
      cell(1, "2026-09-06T12:00:00"),
      cell(2, "2026-09-06T11:00:00"),
      cell(3, "2026-09-06T10:00:00"),
      cell(4, "2026-09-05T10:00:00"),
    ];
    const rows = tiles(buildGridRows(cells, 4, true));
    expect(rows).toHaveLength(2);
    expect(rows[0]!.cells.map((c) => c.photo.id)).toEqual([1, 2, 3]);
    expect(rows[1]!.cells.map((c) => c.photo.id)).toEqual([4]);
  });

  it("counts the photos in a day heading", () => {
    const cells = [
      cell(1, "2026-09-06T12:00:00"),
      cell(2, "2026-09-06T11:00:00"),
      cell(3, "2026-09-05T10:00:00"),
    ];
    const days = buildGridRows(cells, 2, true).filter((r) => r.kind === "day");
    expect(days.map((d) => (d as { count: number }).count)).toEqual([2, 1]);
  });

  it("gives undated photos one heading and no day heading", () => {
    // SQLite sorts NULL capture dates last under DESC, so they arrive as a
    // trailing run. "Undated" under "No date" would only say it twice.
    const cells = [cell(1, "2026-09-06T10:00:00"), cell(2, null), cell(3, null)];
    const rows = buildGridRows(cells, 4, true);
    expect(headings(rows)).toEqual(["September 2026", "Sun 6 September", "No date"]);
    expect(tiles(rows)).toHaveLength(2);
  });

  it("keeps firstIndex pointing back at the flat cell list across headings", () => {
    // This is what keyboard traversal and the roving tab stop resolve through,
    // so an index that drifts by the number of headings above it moves focus to
    // the wrong photo.
    const cells = [
      cell(10, "2026-09-06T12:00:00"),
      cell(11, "2026-09-05T12:00:00"),
      cell(12, "2026-08-01T12:00:00"),
    ];
    const rows = buildGridRows(cells, 1, true);
    expect(tiles(rows).map((r) => r.firstIndex)).toEqual([0, 1, 2]);
  });

  it("is empty for no cells, grouped or not", () => {
    expect(buildGridRows([], 4, true)).toEqual([]);
    expect(buildGridRows([], 4, false)).toEqual([]);
  });

  it("survives a zero column count rather than looping forever", () => {
    // `cols` is derived from a measured width, which is 0 on the first frame.
    const rows = buildGridRows([cell(1, null)], 0, true);
    expect(tiles(rows)).toHaveLength(1);
  });
});

describe("monthLabelAt", () => {
  it("finds the heading governing a row further down", () => {
    const cells = [
      cell(1, "2026-09-06T12:00:00"),
      cell(2, "2026-08-30T12:00:00"),
      cell(3, null),
    ];
    const rows = buildGridRows(cells, 2, true);
    // Last row is the undated group; it must report "No date", not the August
    // heading two above it — a photo with no date has nothing to derive from,
    // which is exactly why this reads the row list rather than the photo.
    expect(monthLabelAt(rows, rows.length - 1)).toBe("No date");
    expect(monthLabelAt(rows, 0)).toBe("September 2026");
  });

  it("is null when nothing above it is a heading", () => {
    const rows = buildGridRows([cell(1, "2026-09-06T12:00:00")], 2, false);
    expect(monthLabelAt(rows, 0)).toBeNull();
  });
});

describe("cellRowIndex", () => {
  it("maps every cell to the row that renders it, skipping headings", () => {
    const cells = [
      cell(1, "2026-09-06T12:00:00"),
      cell(2, "2026-09-06T11:00:00"),
      cell(3, "2026-09-05T10:00:00"),
    ];
    const rows = buildGridRows(cells, 2, true);
    const map = cellRowIndex(rows);
    expect(map.size).toBe(3);
    // Cells 0 and 1 share a row; cell 2 is below a fresh day heading.
    expect(map.get(0)).toBe(map.get(1));
    expect(map.get(2)).toBeGreaterThan(map.get(1)!);
    expect(rows[map.get(2)!]!.kind).toBe("photos");
  });
});

describe("labels", () => {
  it("names months with the year and days with the weekday", () => {
    expect(monthLabel("2026-09")).toBe("September 2026");
    expect(dayLabel("2026-09-06")).toBe("Sun 6 September");
  });

  it("falls back rather than printing NaN for a missing date", () => {
    expect(monthLabel("")).toBe("No date");
    expect(dayLabel("")).toBe("Undated");
  });
});

describe("sameIds", () => {
  const list = (...ids: number[]) => ids.map((id) => ({ id }));

  it("calls a patched copy of the same photos the same set", () => {
    const before = list(1, 2, 3);
    const patched = before.map((p) => (p.id === 2 ? { ...p, rating: 3 } : p));
    expect(sameIds(before, patched)).toBe(true);
  });

  it("calls a different, reordered, longer or shorter list a new set", () => {
    expect(sameIds(list(1, 2, 3), list(1, 2, 4))).toBe(false);
    expect(sameIds(list(1, 2, 3), list(3, 2, 1))).toBe(false);
    expect(sameIds(list(1, 2), list(1, 2, 3))).toBe(false);
    expect(sameIds(list(1, 2, 3), list(1, 2))).toBe(false);
  });
});

describe("dayLabel in other languages", () => {
  afterEach(async () => {
    await setLocale("en");
  });

  it("puts the day where German and Japanese put it, once", async () => {
    await setLocale("de");
    expect(dayLabel("2025-09-06")).toBe("Sa, 6. September");
    await setLocale("ja");
    expect(dayLabel("2025-09-06")).toBe("9月6日（土）");
  });
});

describe("rebuilding rows for a new column count", () => {
  afterEach(async () => {
    vi.restoreAllMocks();
    await setLocale("en");
  });

  it("words each heading once, not on every rebuild", () => {
    const cells = Array.from({ length: 400 }, (_, i) =>
      cell(i + 1, `2025-${String(1 + (i % 12)).padStart(2, "0")}-${String(1 + (i % 28)).padStart(2, "0")}T10:00:00`),
    );
    buildGridRows(cells, 4, true);
    // `format` is a getter handing out a bound function: counting reads of it
    // counts dates formatted. (Typed as a plain property: vitest only spies on
    // getters of properties it does not take for methods.)
    const proto = Intl.DateTimeFormat.prototype as unknown as { format: unknown };
    const formatted = vi.spyOn(proto, "format", "get");
    // The filter rail folding away: the same photos, a few column counts.
    for (const cols of [5, 6, 7, 6, 5]) buildGridRows(cells, cols, true);
    expect(formatted).not.toHaveBeenCalled();
  });

  it("words them again in a new language", async () => {
    const cells = [cell(1, "2025-09-06T10:00:00")];
    const day = (rows: GridRow[]) => rows.find((r) => r.kind === "day")!;
    expect(day(buildGridRows(cells, 4, true))).toMatchObject({ label: "Sat 6 September" });
    await setLocale("de");
    expect(day(buildGridRows(cells, 4, true))).toMatchObject({ label: "Sa, 6. September" });
  });
});
