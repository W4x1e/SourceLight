import { useState } from "react";
import { t } from "../i18n";
import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { IS_MAC } from "../lib/platform";
import { useStore } from "../store";
import OnOff from "./OnOff";

/**
 * Settings → About → Diagnostics.
 *
 * The two things a bug report needs and a user cannot be expected to find:
 * the facts about their machine and install, and the folder the crash and
 * error logs are written to. Nothing is sent anywhere — Copy puts text on the
 * clipboard for the user to paste, and Open shows them the folder.
 */
export default function DiagnosticsSection() {
  const [busy, setBusy] = useState(false);
  const directCanvas = useStore((s) => s.directCanvas);
  const setDirectCanvas = useStore((s) => s.setDirectCanvas);

  const copy = async () => {
    setBusy(true);
    try {
      await navigator.clipboard.writeText(await api.diagnosticsSummary());
      toast(t("settings.diagnostics.copied"), "success");
    } catch (e) {
      toast(t("settings.diagnostics.copyFailed", { error: e instanceof Error ? e.message : String(e) }), "error");
    } finally {
      setBusy(false);
    }
  };

  const open = async () => {
    try {
      await api.openLogFolder();
    } catch (e) {
      toast(t("settings.diagnostics.openLogsFailed", { error: e instanceof Error ? e.message : String(e) }), "error");
    }
  };

  return (
    // The gap every other card on the About tab has; it sat further off.
    <div className="mt-3 rounded-md bg-raised p-3">
      <div className="text-caption font-medium text-ink">{t("settings.diagnostics.title")}</div>
      <p className="mt-0.5 text-label leading-relaxed text-faint">
        {t("settings.diagnostics.blurb")}
      </p>
      <div className="mt-2 flex gap-2">
        <button onClick={() => void copy()} disabled={busy} className="btn-ghost text-label">
          {t("settings.diagnostics.copy")}
        </button>
        <button onClick={() => void open()} className="btn-ghost text-label">
          {t("settings.diagnostics.openLogs")}
        </button>
      </div>
      {/* The direct surface (lib/directCanvas). Here rather than in a
          preferences tab because nobody should need it — it is the way back
          if the preview ever misbehaves on some Mac. */}
      {IS_MAC && (
        <div className="mt-3 flex items-start gap-3 border-t border-line pt-3">
          <div className="min-w-0 flex-1">
            <div className="text-caption text-ink">{t("settings.diagnostics.gpu.title")}</div>
            <p className="mt-0.5 text-label leading-relaxed text-faint">
              {t("settings.diagnostics.gpu.blurb")}
            </p>
          </div>
          <OnOff label={t("settings.diagnostics.gpu.title")} on={directCanvas} onChange={setDirectCanvas} />
        </div>
      )}
    </div>
  );
}
