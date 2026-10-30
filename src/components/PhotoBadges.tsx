import { Icon } from "../lib/icons";
import { COLOR_LABELS } from "../types";
import { t } from "../i18n";

/**
 * The verdict marks a photo carries over its thumbnail — flag, star rating and
 * colour label — shared by the library grid and Compare, which used to draw
 * each one separately. One vocabulary, so a pick looks
 * the same wherever it is judged.
 */

/** Pick or reject. `labelled` adds the word, for Compare's larger tiles. */
export function FlagBadge({ flag, labelled = false }: { flag: number; labelled?: boolean }) {
  if (flag === 0) return null;
  const tone = flag > 0 ? "text-positive" : "text-danger";
  const icon = flag > 0 ? "flag" : "flag-off";
  // Over the photo: the darker chip, so the mark clears 3:1 over a
  // white frame, its own shape for each verdict, and the word in ink — the
  // verdict colour stays on the glyph only.
  if (labelled) {
    return (
      <span className="inline-flex items-center gap-1 rounded-sm bg-chip-hover px-1 text-label text-ink backdrop-blur">
        <Icon name={icon} size={11} className={tone} />
        {flag > 0 ? t("library.badges.pick") : t("library.badges.reject")}
      </span>
    );
  }
  return (
    <span className="rounded-sm bg-chip-hover p-0.5 backdrop-blur">
      <Icon name={icon} size={12} className={tone} />
    </span>
  );
}

/** Stars, when there are any. */
export function RatingBadge({ rating }: { rating: number }) {
  if (rating <= 0) return null;
  return (
    <span className="rounded-sm bg-chip px-1 text-label text-accent backdrop-blur">
      {"★".repeat(rating)}
    </span>
  );
}

/** The colour label's dot, when there is one. */
export function ColorLabelDot({ label, className = "" }: { label: number; className?: string }) {
  if (label <= 0) return null;
  const colour = COLOR_LABELS.find((c) => c.value === label);
  return (
    <span
      className={`h-2.5 w-2.5 rounded-full border border-black/30 ${className}`}
      style={{ backgroundColor: colour?.hex ?? "#888" }}
      title={colour ? t(colour.nameKey) : undefined}
    />
  );
}
