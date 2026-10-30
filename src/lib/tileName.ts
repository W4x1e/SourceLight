import { t } from "../i18n";
import { COLOR_LABELS, type Photo } from "../types";

/**
 * What a grid tile says to a screen reader: the file, then its state —
 * stars, verdict, label, a missing file, an edit or a copy — which the badges
 * show to the eye. The name was the filename alone, so a cull in Browse by
 * keyboard was blind.
 */
export function tileName(
  p: Pick<Photo, "filename" | "rating" | "flag" | "colorLabel" | "missing" | "edited" | "variant" | "variantName">,
): string {
  const parts = [p.filename];
  if (p.rating > 0) parts.push(t("library.grid.aria.stars", { count: p.rating }));
  if (p.flag > 0) parts.push(t("library.badges.pick"));
  if (p.flag < 0) parts.push(t("library.badges.reject"));
  const label = COLOR_LABELS.find((c) => c.value === p.colorLabel);
  if (label) parts.push(t("library.grid.aria.label", { name: t(label.nameKey) }));
  if (p.missing) parts.push(t("library.grid.fileMissing"));
  else if (p.variant > 0) parts.push(p.variantName ? `${t("library.grid.copy")} ${p.variantName}` : t("library.grid.copy"));
  else if (p.edited) parts.push(t("library.grid.aria.edited"));
  return parts.join(t("library.grid.aria.join"));
}
