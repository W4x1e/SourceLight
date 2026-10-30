import { formatDate, t } from "../../src/i18n";

/** A capture time as the catalogue writes it (`YYYY-MM-DD HH:MM:SS`, the
 *  camera's clock), as a local date; none for none or nonsense. */
export function catalogueDate(s: string | null): Date | null {
  const m = s && /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?/.exec(s);
  if (!m) return null;
  const [, y, mo, d, h = "0", mi = "0", se = "0"] = m;
  return new Date(+y!, +mo! - 1, +d!, +h, +mi, +se);
}

/** The month a photo belongs to under the grid's headings, `YYYY-MM`, or ""
 *  for a photo without a date. Cheap: it is asked of every photo. */
export function monthKey(date: Date | null): string {
  if (!date) return "";
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;
}

/** A month heading: "September 2026", or "No date". */
export function monthLabel(key: string): string {
  if (!key) return t("mobile.grid.noDate");
  const [y, m] = key.split("-");
  return formatDate(new Date(+y!, +m! - 1, 1), { month: "long", year: "numeric" });
}

/** When a photo was taken, for the loupe: "Fri 18 Sep 2026, 10:15". */
export function takenText(date: Date): string {
  return formatDate(date, {
    weekday: "short",
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}
