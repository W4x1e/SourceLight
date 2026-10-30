import { api } from "./api";
import { sentenceCase } from "./format";
import { formatList, formatNumber, t } from "../i18n";
import { confirm, toast } from "./dialogs";
import type { AiPurpose } from "../bindings";
import { isCancelled } from "./jobs";

/**
 * The front door to every AI feature.
 *
 * The backend ladder (`ensure_ai_ready`) does the work — choosing a provider,
 * starting the server, pulling the model, loading it. A call that comes back
 * with `needs` is repeated with consent, so the download runs, with progress
 * and Cancel in Activity. Before that, when the backend says `askFirst` — the
 * vision model, or any download on a metered connection — it becomes a single
 * dialog naming everything and its total. Otherwise the user clicked a feature
 * that needs a few hundred megabytes, and a dialog would only ask them to click
 * it again, so a toast says what is downloading instead.
 *
 * Callers treat this as a gate: `if (!(await ensureAi("tagging"))) return;`.
 * That is deliberately different from the old shape, where buttons were
 * *disabled* until AI happened to be ready — which left a fresh install with a
 * greyed-out Tag button and no path from it to the setup that would ungrey it.
 */

/** Rounds a total to the unit a person would use out loud. */
export function downloadSize(mb: number): string {
  return mb >= 1024
    ? t("settings.aiReady.sizeGb", { size: formatNumber(mb / 1024, { minimumFractionDigits: 1, maximumFractionDigits: 1 }) })
    : t("settings.aiReady.sizeMb", { size: formatNumber(Math.round(mb)) });
}

/** An error's words, without the "Error: " that `String(e)` puts in front —
 *  which reads as a crash in a sentence meant for a person. */
export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Ensures AI can serve `purpose`, asking once if a download is needed.
 *
 * Returns false when the user declined, or when nothing could make it work —
 * in which case the reason has already been shown, so the caller just stops.
 * `inline` is a caller that shows download progress itself, beside the button
 * that started it (Develop, which sits over Activity).
 */
export async function ensureAi(purpose: AiPurpose, { inline = false }: { inline?: boolean } = {}): Promise<boolean> {
  let state;
  try {
    state = await api.ensureAiReady(purpose, false);
  } catch (e) {
    toast(errorText(e), "error");
    return false;
  }

  if (state.ready) return true;

  if (state.blocked) {
    // Switched off: asked, not refused in red. Off now means nothing
    // runs, so a feature used on purpose offers the way back on.
    if (state.provider === "off") {
      const on = await confirm({
        title: t("settings.aiReady.offTitle"),
        message: t("settings.aiReady.offMessage"),
        confirmLabel: t("settings.aiReady.offConfirm"),
      });
      if (!on) return false;
      try {
        await api.turnAiOn();
      } catch (e) {
        toast(errorText(e), "error");
        return false;
      }
      // Imported here rather than at the top: the store imports this module.
      const { useStore } = await import("../store");
      void useStore.getState().refreshAi();
      return ensureAi(purpose, { inline });
    }
    toast(state.blocked, "error");
    return false;
  }

  if (state.needs.length > 0) {
    const total = state.needs.reduce((sum, r) => sum + r.approxMb, 0);
    const what = formatList(state.needs.map((r) => r.label));
    if (state.askFirst) {
      const agreed = await confirm({
        title: t("settings.aiReady.setupTitle"),
        message: t(state.metered ? "settings.aiReady.setupMessageMetered" : "settings.aiReady.setupMessage", {
          what,
          size: downloadSize(total),
        }),
        confirmLabel: t("settings.aiReady.setupConfirm"),
      });
      if (!agreed) return false;
    } else {
      const key = inline ? "settings.aiReady.downloadingHere" : "settings.aiReady.downloading";
      // "das Modell …" opens the German sentence: capitalised (Mac test).
      toast(sentenceCase(t(key, { what, size: downloadSize(total) })), "info");
    }

    try {
      // The download runs inside this call and reports on the same progress
      // events the Activity panel already listens to, so the panel shows it
      // wherever the user started from.
      const done = await api.ensureAiReady(purpose, true);
      if (done.blocked) {
        toast(done.blocked, "error");
        return false;
      }
      return done.ready;
    } catch (e) {
      // Cancelled from the Activity panel mid-download: said plainly, not as
      // an error.
      toast(isCancelled(e) ? t("settings.aiReady.cancelled") : errorText(e), isCancelled(e) ? "info" : "error");
      return false;
    }
  }

  // Not ready, nothing to download and nothing named as blocking: report the
  // provider's own words rather than failing silently.
  toast(state.detail || t("settings.aiReady.notReady"), "error");
  return false;
}
