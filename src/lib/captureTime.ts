import { formatDate, getLocale, intlLocale, t } from "../i18n";
/**
 * The capture-time dialog's arithmetic, kept out of the component.
 *
 * Capture times are the catalogue's `YYYY-MM-DD HH:MM:SS`: wall-clock time as
 * the camera recorded it, with no time zone. They are read and written here as
 * local `Date`s only so the arithmetic and a date-time field can use them — a
 * time is never converted between zones.
 */

/** A catalogue capture time as a `Date`, or null for none or nonsense. */
export function parseCapture(s: string | null | undefined): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})(?::(\d{2}))?/.exec(s ?? "");
  if (!m) return null;
  // Seconds are optional, and Number(undefined) is NaN, not undefined.
  const [, y, mo, d, h, mi, se] = m.map((v) => Number(v ?? 0));
  const date = new Date(y!, mo! - 1, d!, h!, mi!, se!);
  return Number.isNaN(date.getTime()) ? null : date;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** A `Date` as the catalogue writes a capture time. */
export function formatCapture(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** A `Date` as a `datetime-local` field wants it, to the second. */
export function toInputValue(d: Date): string {
  return formatCapture(d).replace(" ", "T");
}

export interface Shift {
  later: boolean;
  days: number;
  hours: number;
  minutes: number;
  seconds: number;
}

/** A shift in whole seconds, negative for earlier. Blank or silly fields count
 *  as nought rather than NaN. */
export function shiftSeconds(s: Shift): number {
  const n = (v: number) => (Number.isFinite(v) ? Math.trunc(v) : 0);
  const total = n(s.days) * 86400 + n(s.hours) * 3600 + n(s.minutes) * 60 + n(s.seconds);
  return s.later ? total : -total;
}

/** The earliest capture time among `photos`, or null when none has one. */
export function earliest(photos: { captureDate: string | null }[]): Date | null {
  let first: Date | null = null;
  for (const p of photos) {
    const d = parseCapture(p.captureDate);
    if (d && (!first || d < first)) first = d;
  }
  return first;
}

/** A capture time for people: "14 Jun 2025, 10:00:05". */
export function describeCapture(d: Date | null): string {
  if (!d) return t("library.captureTime.noTime");
  // English in the day-month order and 24-hour clock the grid, timeline and
  // trips read in (see `periodLabel`): Intl's "en" is American, and the
  // dialog said "Jun 14, 2025, 10:00:05 AM" under "Sat 14 June".
  if (getLocale().startsWith("en")) {
    const time = formatDate(d, { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" });
    return `${d.getDate()} ${formatDate(d, { month: "short" })} ${d.getFullYear()}, ${time}`;
  }
  return d.toLocaleString(intlLocale(), {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
