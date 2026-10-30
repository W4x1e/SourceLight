import { humanBytes } from "./format";
import type { BatchOutcome, XmpImport } from "../bindings";
import { t } from "../i18n";

/**
 * How an export reports what it did.
 *
 * Two copies of this existed — one in the store's quick export, one in
 * Develop's batch export — and they had already drifted apart from the
 * single-photo path, which hand-rolled `bytes / 1024 / 1024` while
 * `humanBytes` sat one import away.
 *
 * More to the point, none of it ever reached the user. A single export left
 * `Wrote 5.7 MB` in a local variable inside the export dialog, so closing the
 * dialog — the obvious thing to do once the export has run — destroyed the only
 * evidence it had happened at all.
 */

/** What a finished batch says in the queue panel and in the toast. */
export function batchSummary(outcome: BatchOutcome, attempted: number): string {
  if (outcome.cancelled) {
    return t("transfer.summary.cancelled", { written: outcome.written, attempted });
  }
  if (outcome.failed > 0) {
    return t("transfer.summary.someFailed", {
      written: outcome.written,
      failed: outcome.failed,
      error: outcome.errors[0] ?? "",
    });
  }
  return t("transfer.summary.exported", { count: outcome.written });
}

/** Whether a finished batch went well enough to read as a success. */
export function batchSucceeded(outcome: BatchOutcome): boolean {
  return !outcome.cancelled && outcome.failed === 0;
}

/**
 * What writing XMP sidecars says in the toast.
 *
 * Same shape as {@link batchSummary} but for sidecars, and it must name a
 * failure: a full disk used to under-count silently and still claim success, so
 * a photo could lose its metadata with nothing said. There is no cancel
 * on this path, so no cancelled branch.
 */
export function sidecarSummary(outcome: BatchOutcome): string {
  if (outcome.failed > 0) {
    return t("transfer.summary.sidecarsSomeFailed", {
      written: outcome.written,
      failed: outcome.failed,
      error: outcome.errors[0] ?? "",
    });
  }
  // Left as they were because they already said it.
  const unchanged = outcome.unchanged ?? 0;
  if (unchanged > 0) {
    return t("transfer.summary.sidecarsWroteSomeSame", { written: outcome.written, unchanged });
  }
  return t("transfer.summary.sidecarsWrote", { count: outcome.written });
}

/**
 * What reading XMP sidecars says in the toast: how many were read, and
 * what they did to People — including the names left off a face People
 * already calls someone else, which would otherwise vanish without a word.
 */
export function sidecarReadSummary(r: XmpImport): string {
  if (r.read === 0) return t("transfer.summary.noSidecars");
  const parts = [t("transfer.summary.sidecarsRead", { count: r.read })];
  if (r.people > 0) parts.push(t("transfer.summary.namesAdded", { count: r.people }));
  if (r.kept > 0) parts.push(t("transfer.summary.facesKept", { count: r.kept }));
  // What came in over the catalogue's own verdicts can go back.
  if (r.verdictsChanged > 0) parts.push(t("transfer.summary.verdictsUndo", { count: r.verdictsChanged }));
  return parts.join(" · ");
}

/**
 * The pixel size an export of a `width` × `height` photo comes out at with a
 * long edge of `maxEdge` (null for full size). As the core's `fit_within`:
 * the aspect is kept and nothing is enlarged.
 */
export function outputSize(width: number, height: number, maxEdge: number | null): { width: number; height: number } {
  const longest = Math.max(width, height);
  if (!maxEdge || longest <= maxEdge || longest === 0) return { width, height };
  const scale = maxEdge / longest;
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

/** What an export will write, said before it does: the size, the file
 *  type and how it is encoded, and the colour space. `photo` is the open
 *  photo's framed size; without one (a batch) the size is the long edge. */
export function exportPlan(
  s: { format: string; quality: number; depth: string; size: number | null; space: string },
  photo: { width: number; height: number } | null,
  count = 1,
): string {
  const parts: string[] = [];
  if (count > 1) parts.push(t("common.photos", { count }));
  if (photo) {
    const out = outputSize(photo.width, photo.height, s.size);
    // Strings, as Develop's header writes a photo's size: "6000 × 4000", not
    // "6,000 × 4,000".
    parts.push(t("transfer.export.planPixels", { width: String(out.width), height: String(out.height) }));
  } else {
    parts.push(s.size ? t("transfer.export.planLongEdge", { size: String(s.size) }) : t("transfer.export.planFullSize"));
  }
  const format = s.format === "jpeg" ? "JPEG" : s.format.toUpperCase();
  parts.push(
    s.format === "jpeg"
      ? t("transfer.export.planQuality", { format, quality: s.quality })
      : s.format === "webp"
        ? t("transfer.export.planLossless", { format })
        : t("transfer.export.planBits", { format, bits: s.depth === "sixteen" ? 16 : 8 }),
  );
  parts.push(s.space === "srgb" ? "sRGB" : s.space === "display-p3" ? "Display P3" : "Adobe RGB");
  return parts.join(" · ");
}

/** What a finished single-photo export says: the file, and how big it came out. */
export function singleSummary(filename: string, bytes: number): string {
  return t("transfer.summary.single", { name: filename, size: humanBytes(bytes) });
}
