/**
 * Turning a flat list of grid cells into the rows the library actually renders,
 * with date headings between them.
 *
 * The grid was an undifferentiated wall of tiles: 20,000 photos with nothing to
 * say where one shoot ended and the next began. Scrolling it told you nothing
 * about *when*, which is the axis a photo library is actually organised on.
 *
 * The maths lives here, pure, for the same reason `stacks.ts` does: the
 * virtualiser needs a row descriptor list with per-row heights and a stable
 * mapping back to cell indices, and every one of those is easy to get subtly
 * wrong in a way that only shows up as a misplaced tile 4,000 rows down.
 *
 * Grouping is only meaningful when the list is ordered by date — which it is
 * for a normal query (`ORDER BY capture_date DESC`) but is not for semantic
 * search results, which come back by relevance. Callers pass `grouped: false`
 * there, and get plain chunked rows.
 */
import { parseISO } from "./calendar";
import type { GridCell } from "./stacks";
import { formatDate, t, textVersion } from "../i18n";

export type GridRow =
  /** A month heading: "September 2026", or "No date". */
  | { kind: "month"; key: string; label: string }
  /** A day heading under it: "Sat 6 September", with that day's photo count. */
  | { kind: "day"; key: string; label: string; count: number }
  /** One row of tiles. `firstIndex` is the position of `cells[0]` in the flat
   *  cell list, which is what keyboard traversal and the roving tab stop use. */
  | { kind: "photos"; key: string; cells: GridCell[]; firstIndex: number };

/** Height in pixels of each heading kind, so `estimateSize` can answer without
 *  measuring. Month headings carry the display face and need the room. */
export const MONTH_ROW_H = 44;
export const DAY_ROW_H = 26;

/** The `YYYY-MM` a photo belongs to, or "" when it has no capture date. */
export function monthKey(captureDate: string | null): string {
  return parseISO(captureDate?.slice(0, 10)) ? captureDate!.slice(0, 7) : "";
}

/** The `YYYY-MM-DD` a photo belongs to, or "" when it has no capture date. */
export function dayKey(captureDate: string | null): string {
  return parseISO(captureDate?.slice(0, 10)) ? captureDate!.slice(0, 10) : "";
}

/** "September 2026" for a `YYYY-MM` key. */
export function monthLabel(key: string): string {
  const parts = parseISO(`${key}-01`);
  // UTC on both ends, so the viewer's time zone cannot move the date across a month edge.
  return parts
    ? formatDate(Date.UTC(parts.year, parts.month0, 1), { month: "long", year: "numeric", timeZone: "UTC" })
    : t("library.grid.noDate");
}

/** "Sat 6 September" for a `YYYY-MM-DD` key — the year is on the month heading
 *  above it, so repeating it on every day would be noise. */
export function dayLabel(key: string): string {
  const parts = parseISO(key);
  if (!parts) return t("library.grid.undated");
  const at = Date.UTC(parts.year, parts.month0, parts.day);
  return t("library.grid.dayLabel", {
    weekday: formatDate(at, { weekday: "short", timeZone: "UTC" }),
    // The bare number: Intl's own day carries a suffix in some languages
    // (Japanese "6日"), which the message adds where it belongs.
    day: String(parts.day),
    month: formatDate(at, { month: "long", timeZone: "UTC" }),
  });
}

/**
 * Headings already worded, by key, for the text they were worded in.
 *
 * The rows are rebuilt whenever the column count changes (the window resized,
 * the filter rail folding away), and every heading used to be formatted afresh
 * each time: for a large library that was most of the rebuild, a visible
 * hitch in the middle of the rail's animation. The words only change with the
 * language, so they are kept until `textVersion` moves.
 */
let labelsFor = -1;
const labels = new Map<string, string>();
function remembered(key: string, word: () => string): string {
  const v = textVersion();
  if (v !== labelsFor) {
    labels.clear();
    labelsFor = v;
  }
  let label = labels.get(key);
  if (label === undefined) {
    label = word();
    labels.set(key, label);
  }
  return label;
}

/**
 * Lays `cells` out into rows of `cols`, inserting month and day headings when
 * `grouped`.
 *
 * A day's tiles always start on a fresh row, so a heading is never followed by
 * the tail of the previous day. Undated photos — which SQLite sorts last under
 * `DESC` — get a single "No date" month heading and no day headings, since
 * there is no day to name.
 */
export function buildGridRows(cells: GridCell[], cols: number, grouped: boolean): GridRow[] {
  const width = Math.max(1, cols);
  const rows: GridRow[] = [];

  const pushTiles = (group: GridCell[], firstIndex: number) => {
    for (let i = 0; i < group.length; i += width) {
      rows.push({
        kind: "photos",
        // Keyed on the first cell's photo, which is unique and stable across a
        // reflow — a positional key would remount every tile on resize.
        key: `p${group[i]!.photo.id}`,
        cells: group.slice(i, i + width),
        firstIndex: firstIndex + i,
      });
    }
  };

  if (!grouped || cells.length === 0) {
    pushTiles(cells, 0);
    return rows;
  }

  let i = 0;
  let openMonth: string | null = null;
  while (i < cells.length) {
    const day = dayKey(cells[i]!.photo.captureDate);
    const month = monthKey(cells[i]!.photo.captureDate);

    if (month !== openMonth) {
      rows.push({ kind: "month", key: `m${month || "none"}`, label: remembered(`m${month}`, () => monthLabel(month)) });
      openMonth = month;
    }

    // Consume the whole run for this day. The list is date-ordered, so a day is
    // a contiguous run and one pass is enough.
    let end = i;
    while (end < cells.length && dayKey(cells[end]!.photo.captureDate) === day) end++;

    // An undated run has no day to name, and "Undated" under "No date" would
    // just say it twice.
    if (day !== "") {
      rows.push({
        kind: "day",
        key: `d${day}`,
        label: remembered(`d${day}`, () => dayLabel(day)),
        count: end - i,
      });
    }
    pushTiles(cells.slice(i, end), i);
    i = end;
  }

  return rows;
}

/**
 * The month heading governing `index` — the nearest one at or above it.
 *
 * What the pinned bar at the top of the viewport shows. Reading it back off the
 * row list rather than re-deriving it from a photo's date keeps the bar and the
 * heading it shadows in exact agreement, including for the undated group, whose
 * photos have no date to derive anything from.
 */
export function monthLabelAt(rows: GridRow[], index: number): string | null {
  for (let i = Math.min(index, rows.length - 1); i >= 0; i--) {
    const row = rows[i];
    if (row?.kind === "month") return row.label;
  }
  return null;
}

/**
 * Row index for each flat cell index, so focus can be scrolled to and the tab
 * stop resolved without scanning the row list on every keystroke.
 */
export function cellRowIndex(rows: GridRow[]): Map<number, number> {
  const map = new Map<number, number>();
  rows.forEach((row, r) => {
    if (row.kind !== "photos") return;
    row.cells.forEach((_, c) => map.set(row.firstIndex + c, r));
  });
  return map;
}

/**
 * Whether two photo lists hold the same photos in the same order. A rating,
 * flag or label patches the list into a new array of the same photos, which
 * the grid must not treat as a new set.
 */
export function sameIds(a: readonly { id: number }[], b: readonly { id: number }[]): boolean {
  return a === b || (a.length === b.length && a.every((p, i) => p.id === b[i]?.id));
}
