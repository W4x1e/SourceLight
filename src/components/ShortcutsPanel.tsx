import { useCallback, useEffect, useState } from "react";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { confirm } from "../lib/dialogs";
import { useEscape } from "../lib/useEscape";
import { t } from "../i18n";
import {
  ACTIONS,
  comboToLabel,
  conflicts,
  eventToCombo,
  type ActionId,
} from "../lib/keymap";

/**
 * Editable keyboard-shortcut map. Click a binding to capture a new key; the
 * next keystroke becomes the shortcut (Escape cancels). Assigning a combo that
 * another action in an overlapping context already uses clears that other
 * binding, so a key never means two things at once.
 */
/** `closing` runs the exit animation: App keeps the panel mounted for it (see `usePresence`). */
export default function ShortcutsPanel({ closing = false }: { closing?: boolean }) {
  const keymap = useStore((s) => s.keymap);
  const setBinding = useStore((s) => s.setBinding);
  const resetKeymap = useStore((s) => s.resetKeymap);
  const onClose = useCallback(() => useStore.getState().setShowShortcuts(false), []);

  const [capturing, setCapturing] = useState<ActionId | null>(null);

  // Escape closes — except while capturing, where Escape means "cancel this
  // capture" and the capture listener (which runs on the capture phase, ahead
  // of this one) has already claimed it.
  useEscape(capturing === null, onClose);

  // While capturing, the next key press (that isn't a bare modifier) is bound.
  useEffect(() => {
    if (!capturing) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setCapturing(null);
        return;
      }
      const combo = eventToCombo(e);
      if (!combo) return; // bare modifier — keep waiting
      // Free the combo from anything it would clash with, then assign.
      for (const other of conflicts(useStore.getState().keymap, capturing, combo)) {
        setBinding(other, "");
      }
      setBinding(capturing, combo);
      setCapturing(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capturing, setBinding]);

  const groups = [...new Set(ACTIONS.map((a) => a.group))];

  return (
    <div
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      data-closing={closing || undefined}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="anim-card flex max-h-[min(680px,calc(100vh-4rem))] w-full max-w-lg flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl">
        <div className="flex items-center gap-3 border-b border-line px-4 py-3">
          <div>
            <h2 className="font-display text-display text-ink">{t("shell.shortcuts.title")}</h2>
            <p className="text-caption text-faint">
              {t("shell.shortcuts.subtitle")}
            </p>
          </div>
          <button onClick={onClose} className="icon-chip ml-auto" aria-label={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-4 py-3">
          {groups.map((group) => (
            <section key={group} className="mb-4 last:mb-0">
              <div className="section-label px-0 mb-1">{t(group)}</div>
              <div className="overflow-hidden rounded-md border border-line">
                {ACTIONS.filter((a) => a.group === group).map((a, i) => {
                  const combo = keymap[a.id];
                  const isCapturing = capturing === a.id;
                  return (
                    <div
                      key={a.id}
                      className={`flex items-center gap-3 px-3 py-2 text-caption ${
                        i > 0 ? "border-t border-line" : ""
                      }`}
                    >
                      <span className="flex-1 text-ink">{t(a.label)}</span>
                      {combo && !isCapturing && (
                        <button
                          onClick={() => setBinding(a.id, "")}
                          className="text-faint hover:text-ink"
                          title={t("shell.shortcuts.remove")}
                          aria-label={t("shell.shortcuts.removeFor", { action: t(a.label) })}
                        >
                          <Icon name="close" size={12} />
                        </button>
                      )}
                      <button
                        onClick={() => setCapturing(isCapturing ? null : a.id)}
                        className={`min-w-[74px] rounded-sm border px-2 py-1 text-center font-mono text-caption tabular-nums ${
                          isCapturing
                            ? "border-accent bg-accent/15 text-accent"
                            : "border-line bg-raised text-muted hover:border-accent-dim hover:text-ink"
                        }`}
                      >
                        {isCapturing ? t("shell.shortcuts.pressKey") : comboToLabel(combo)}
                      </button>
                    </div>
                  );
                })}
              </div>
            </section>
          ))}
        </div>

        <div className="flex items-center gap-2 border-t border-line px-4 py-3">
          <button
            onClick={() =>
              void confirm({
                title: t("shell.shortcuts.resetTitle"),
                message: t("shell.shortcuts.resetMessage"),
                confirmLabel: t("common.reset"),
              }).then((ok) => ok && resetKeymap())
            }
            className="btn-solid"
          >
            {t("shell.shortcuts.resetToDefaults")}
          </button>
          <button onClick={onClose} className="btn-accent ml-auto">
            {t("common.done")}
          </button>
        </div>
      </div>
    </div>
  );
}
