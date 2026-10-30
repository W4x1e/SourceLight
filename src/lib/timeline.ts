/**
 * The timeline strip's arithmetic: which periods to draw, what each
 * spans, and when bars become days. Periods are the backend's: `2024-05` for
 * a month, `2024-05-14` for a day.
 */

import type { TimelineBar } from "../bindings";
import { dateFormat, formatDate, getLocale } from "../i18n";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

/** Bars become days when the chosen range is this short or shorter. */
export const DAY_SPAN = 92;

const pad = (n: number) => String(n).padStart(2, "0");
const daysIn = (y: number, m: number) => new Date(Date.UTC(y, m, 0)).getUTCDate();

/** Whole days from `from` to `to`, both `YYYY-MM-DD`, inclusive. */
export function spanDays(from: string, to: string): number {
  const a = Date.parse(`${from}T00:00:00Z`);
  const b = Date.parse(`${to}T00:00:00Z`);
  return Number.isNaN(a) || Number.isNaN(b) ? Infinity : Math.round((b - a) / 86_400_000) + 1;
}

/** The days to draw, when a range short enough is chosen; else months. */
export function dayWindow(from: string | null, to: string | null): { from: string; to: string } | null {
  if (!from || !to) return null;
  return spanDays(from, to) <= DAY_SPAN ? { from, to } : null;
}

/** The period after this one. */
function next(period: string): string {
  const [y, m, d] = period.split("-").map(Number) as [number, number, number?];
  if (d === undefined) return m === 12 ? `${y + 1}-01` : `${y}-${pad(m + 1)}`;
  const t = new Date(Date.UTC(y, m - 1, d + 1));
  return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
}

/**
 * Every period from the first to the last, the empty ones at zero — gaps are
 * part of the library's shape. With a day window, every day in it.
 */
export function fillPeriods(bars: TimelineBar[], days: { from: string; to: string } | null): TimelineBar[] {
  const counts = new Map(bars.map((b) => [b.period, b.count]));
  const first = days ? days.from : bars[0]?.period;
  const last = days ? days.to : bars[bars.length - 1]?.period;
  if (!first || !last) return [];
  const out: TimelineBar[] = [];
  // Bounded: a corrupt date must not spin forever.
  for (let p = first; p <= last && out.length < 20_000; p = next(p)) {
    out.push({ period: p, count: counts.get(p) ?? 0 });
  }
  return out;
}

/** The dates a period covers, `YYYY-MM-DD`, inclusive. */
export function periodRange(period: string): { from: string; to: string } {
  const [y, m, d] = period.split("-").map(Number) as [number, number, number?];
  if (d !== undefined) return { from: period, to: period };
  return { from: `${period}-01`, to: `${period}-${pad(daysIn(y, m))}` };
}

/** "May 2024", or "Tue 14 May 2024" for a day. */
export function periodLabel(period: string): string {
  const [y, m, d] = period.split("-").map(Number) as [number, number, number?];
  // Other languages in their own words and order; English keeps the
  // day-month order it has always had, which Intl's "en" would turn round.
  if (!getLocale().startsWith("en")) {
    return formatDate(
      Date.UTC(y, m - 1, d ?? 1),
      d === undefined
        ? { month: "long", year: "numeric", timeZone: "UTC" }
        : { weekday: "short", day: "numeric", month: "short", year: "numeric", timeZone: "UTC" },
    );
  }
  if (d === undefined) return `${MONTHS_LONG[m - 1]} ${y}`;
  const day = dateFormat({ weekday: "short", timeZone: "UTC" }, "en-GB").format(Date.UTC(y, m - 1, d));
  return `${day} ${d} ${MONTHS[m - 1]} ${y}`;
}

/** Whether a period lies within a chosen date range (either end open). */
export function inRange(period: string, from: string | null, to: string | null): boolean {
  const { from: a, to: b } = periodRange(period);
  return (!from || b >= from) && (!to || a <= to);
}
