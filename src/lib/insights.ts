/**
 * The Insights view's arithmetic: what filter a clicked bar becomes,
 * and how a keeper rate reads. Keys are the backend's — see `db::insights`.
 */

import type { InsightCount, PhotoFilter } from "../bindings";

export type InsightKind =
  | "year"
  | "month"
  | "day"
  | "camera"
  | "lens"
  | "focal"
  | "iso"
  | "aperture";

const pad = (n: number) => String(n).padStart(2, "0");

/** The rail's focal range a length falls in (`db::FOCAL_RANGES`). */
export function focalRangeOf(mm: number): string {
  if (mm <= 24) return "0-24";
  if (mm <= 50) return "25-50";
  if (mm <= 105) return "51-105";
  if (mm <= 200) return "106-200";
  return "201-";
}

/** The filter a clicked bar applies: its own facet set to it, dates for time. */
export function patchFor(kind: InsightKind, key: string): Partial<PhotoFilter> {
  switch (kind) {
    case "year":
      return { dateFrom: `${key}-01-01`, dateTo: `${key}-12-31` };
    case "month": {
      const [y, m] = key.split("-").map(Number) as [number, number];
      const last = new Date(Date.UTC(y, m, 0)).getUTCDate();
      return { dateFrom: `${key}-01`, dateTo: `${key}-${pad(last)}` };
    }
    case "day":
      return { dateFrom: key, dateTo: key };
    case "camera":
      return { cameras: [key] };
    case "lens":
      return { lenses: [key] };
    case "focal":
      return { focalRanges: [key] };
    case "iso":
      return { isoRanges: [key] };
    case "aperture":
      return { apertureRanges: [key] };
  }
}

/** "38% keepers", or nothing when there were none to judge. */
export function keeperShare(c: Pick<InsightCount, "count" | "keepers">): string {
  if (c.count === 0) return "";
  return `${Math.round((c.keepers / c.count) * 100)}%`;
}

/** The larger of the counts, for scaling bars; never zero. */
export function most(counts: InsightCount[]): number {
  return Math.max(1, ...counts.map((c) => c.count));
}
