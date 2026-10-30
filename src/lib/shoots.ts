import type { Shoot } from "../bindings";
import { t } from "../i18n";

/**
 * Where a shoot is on the way from card to finished photos, and the one
 * thing to do next — so after a break the answer to "where was I?" is a
 * button, not a hunt.
 */
export type ShootStep = "cull" | "develop" | "export" | "done";

export function nextStep(s: Shoot): { step: ShootStep; label: string } {
  // Copied but not yet catalogued: the scan after the import is still going.
  if (s.photos === 0) return { step: "done", label: t("library.shoots.stillScanning") };
  if (s.judged < s.photos) return { step: "cull", label: t("library.shoots.continueCulling") };
  if (s.picks === 0) return { step: "done", label: t("library.shoots.noPicks") };
  // Exported is the end, edited or not: not every pick needs an edit.
  if (s.exportedPicks >= s.picks) return { step: "done", label: t("library.shoots.done") };
  if (s.editedPicks < s.picks) return { step: "develop", label: t("library.shoots.developPicks") };
  return { step: "export", label: t("library.shoots.exportPicks") };
}

/** The shoot's progress in a line: "120 photos · 60% judged · 14 picks · 3 edited · 0 exported". */
export function progressLine(s: Shoot): string {
  const judged = s.photos === 0 ? 0 : Math.round((100 * s.judged) / s.photos);
  const parts = [t("common.photos", { count: s.photos }), t("library.shoots.judged", { percent: judged })];
  if (s.picks > 0)
    parts.push(
      t("library.shoots.picks", { count: s.picks }),
      t("library.shoots.edited", { count: s.editedPicks }),
      t("library.shoots.exported", { count: s.exportedPicks }),
    );
  return parts.join(" · ");
}
