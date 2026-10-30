/**
 * Small, dependency-free calendar maths for the date picker, kept pure so the
 * grid layout and the ISO parsing/formatting are unit-tested rather than eyeballed.
 */

export interface DateParts {
  year: number;
  /** 0-based, matching `Date` (January = 0). */
  month0: number;
  day: number;
}

const MONTH_RE = /^(\d{4})-(\d{2})-(\d{2})$/;

/** Parses a `YYYY-MM-DD` string; returns null for anything malformed. */
export function parseISO(s: string | null | undefined): DateParts | null {
  if (!s) return null;
  const m = MONTH_RE.exec(s);
  if (!m) return null;
  const year = Number(m[1]);
  const month0 = Number(m[2]) - 1;
  const day = Number(m[3]);
  if (month0 < 0 || month0 > 11 || day < 1 || day > 31) return null;
  return { year, month0, day };
}

/** Formats parts as a zero-padded `YYYY-MM-DD` string. */
export function toISO(year: number, month0: number, day: number): string {
  const mm = String(month0 + 1).padStart(2, "0");
  const dd = String(day).padStart(2, "0");
  return `${year}-${mm}-${dd}`;
}

export function daysInMonth(year: number, month0: number): number {
  // Day 0 of the next month is the last day of this one.
  return new Date(year, month0 + 1, 0).getDate();
}

/** Weekday (0 = Sunday) the month starts on. */
export function firstWeekday(year: number, month0: number): number {
  return new Date(year, month0, 1).getDay();
}

/**
 * A whole-weeks grid for the month: leading/trailing `null`s pad the first and
 * last weeks so the length is always a multiple of 7, and day numbers fill the
 * middle. Feeds a 7-column grid directly.
 */
export function monthGrid(year: number, month0: number): (number | null)[] {
  const cells: (number | null)[] = [];
  for (let i = 0; i < firstWeekday(year, month0); i++) cells.push(null);
  for (let d = 1; d <= daysInMonth(year, month0); d++) cells.push(d);
  while (cells.length % 7 !== 0) cells.push(null);
  return cells;
}

/** Steps a {year, month0} view by whole months, rolling the year over. */
export function shiftMonth(year: number, month0: number, delta: number): { year: number; month0: number } {
  const total = year * 12 + month0 + delta;
  return { year: Math.floor(total / 12), month0: ((total % 12) + 12) % 12 };
}
