import type { Dispatch, SetStateAction } from "react";
import type { DevelopInfo } from "../../bindings";
import { t } from "../../i18n";
import { Icon } from "../../lib/icons";
import { hint } from "./shortcutHint";

interface Props {
  develop: DevelopInfo;
  filename: string;
  close: () => Promise<void>;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  undoKey: string;
  redoKey: string;
  showPresets: boolean;
  setShowPresets: Dispatch<SetStateAction<boolean>>;
  showHistory: boolean;
  setShowHistory: Dispatch<SetStateAction<boolean>>;
  goToStep: (index: number) => Promise<void>;
  saveAsCopy: () => Promise<void>;
  setShowExport: (v: boolean) => void;
  exporting: boolean;
}

/**
 * Develop's title bar.
 *
 * Three zones, each with one job. Left: where you came from, and the
 * Presets panel that opens on this side. Centre: what you are editing.
 * Right: the history of this edit, then the ways out of it — a copy
 * beside the original, or a file. The view switches (before/after,
 * gamut) moved to the bar under the photo they change, and Auto into
 * the rail with the adjustments it sets; eleven equal-weight buttons in
 * one row had made every control look as important as Export.
 */
export default function DevelopHeader({
  develop,
  filename,
  close,
  undo,
  redo,
  undoKey,
  redoKey,
  showPresets,
  setShowPresets,
  showHistory,
  setShowHistory,
  goToStep,
  saveAsCopy,
  setShowExport,
  exporting,
}: Props) {
  return (
    <header
      // The side columns never shrink below their buttons and share what is
      // left equally, which centres the filename in a wide window; in a
      // narrow one the filename gives way (truncates) instead of the buttons
      // running into it.
      className="titlebar grid h-11 shrink-0 grid-cols-[minmax(max-content,1fr)_minmax(0,auto)_minmax(max-content,1fr)] items-center gap-3 border-b border-line bg-panel px-3"
      data-tauri-drag-region="deep"
    >
      <div className="flex min-w-0 items-center gap-1">
        <button
          onClick={() => void close()}
          className="btn-ghost gap-1 px-2 text-muted hover:text-ink"
          title={t("develop.header.backToLibrary")}
        >
          <Icon name="chevron" size={14} className="rotate-90" />
          {t("develop.header.library")}
        </button>
        <span className="mx-1 h-4 w-px shrink-0 bg-line" />
        <button
          onClick={() => setShowPresets((v) => !v)}
          // Toggles show "on" with the pressed-segment state. They used to turn
          // into the key, which put two keys side by side.
          className={`btn-ghost gap-1.5 px-2 ${showPresets ? "seg-on" : "text-muted hover:text-ink"}`}
          aria-pressed={showPresets}
          title={t("develop.header.presetsTitle")}
        >
          <Icon name="layers" size={14} />
          {t("develop.header.presets")}
        </button>
      </div>

      <div className="flex min-w-0 items-baseline justify-center gap-2" data-tauri-drag-region>
        <span className="truncate text-caption font-medium text-ink">{filename}</span>
        <span className="shrink-0 text-label tabular-nums text-faint">
          {develop.width} × {develop.height}
        </span>
      </div>

      <div className="flex min-w-0 items-center justify-end gap-0.5">
        <button
          onClick={() => void undo()}
          disabled={!develop.canUndo}
          className="icon-chip disabled:opacity-40"
          title={hint(t("common.undo"), undoKey)}
          aria-label={t("common.undo")}
        >
          <Icon name="undo" size={16} />
        </button>
        <button
          onClick={() => void redo()}
          disabled={!develop.canRedo}
          className="icon-chip disabled:opacity-40"
          title={hint(t("common.redo"), redoKey)}
          aria-label={t("common.redo")}
        >
          <Icon name="redo" size={16} />
        </button>
        {/* Every step, to go back to any of them. */}
        <div className="relative">
          <button aria-pressed={showHistory}
            onClick={() => setShowHistory((v) => !v)}
            className={`btn-ghost gap-1.5 px-2 ${showHistory ? "seg-on" : "text-muted hover:text-ink"}`}
            aria-expanded={showHistory}
            title={t("develop.header.historyTitle")}
          >
            <Icon name="clock" size={14} />
            {t("develop.header.history")}
          </button>
          {showHistory && (
            <>
              {/* A click anywhere else puts the list away. Out of the title
                  bar's drag region: inside it, the click started a
                  window drag and never reached this. */}
              <div
                className="fixed inset-0 z-20"
                data-tauri-drag-region="false"
                onClick={() => setShowHistory(false)}
              />
              <div
                data-tauri-drag-region="false"
                role="menu"
                aria-label={t("develop.header.history")}
                // Opens on the step the photo is at, and ↑/↓ move through
                // the steps, Enter goes to one: a list of buttons the
                // keyboard could only Tab through from the top.
                ref={(el) => {
                  if (!el || el.contains(document.activeElement)) return;
                  el.querySelector<HTMLElement>('[aria-current="step"]')?.focus({ preventScroll: false });
                }}
                onKeyDown={(e) => {
                  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
                  e.preventDefault();
                  const items = [...e.currentTarget.querySelectorAll<HTMLElement>('[role="menuitem"]')];
                  const at = items.indexOf(document.activeElement as HTMLElement);
                  items[Math.min(items.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)))]?.focus();
                }}
                className="anim-pop pop-right absolute right-0 top-full z-30 mt-1 max-h-80 w-60 overflow-y-auto rounded-md border border-line bg-raised py-1 shadow-2xl"
              >
                {develop.history
                  .map((label, i) => ({ label, i }))
                  .reverse()
                  .map(({ label, i }) => (
                    <button
                      key={i}
                      role="menuitem"
                      aria-current={i === develop.historyAt ? "step" : undefined}
                      onClick={() => void goToStep(i)}
                      className={`flex w-full items-center gap-2 px-3 py-1 text-left text-caption hover:bg-hover focus-visible:bg-hover focus-visible:shadow-[inset_0_0_0_2px_var(--color-focus)] focus-visible:outline-none ${
                        i === develop.historyAt ? "text-ink" : i > develop.historyAt ? "text-faint" : "text-muted"
                      }`}
                      title={i > develop.historyAt ? t("develop.header.historyUndone") : label}
                    >
                      <span
                        className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                          i === develop.historyAt ? "bg-accent" : "bg-transparent"
                        }`}
                      />
                      <span className="truncate">{label}</span>
                    </button>
                  ))}
              </div>
            </>
          )}
        </div>
        <span className="mx-1.5 h-4 w-px shrink-0 bg-line" />
        {/* A copy, not a save: edits save as they are made. This keeps
            the look on screen *beside* the version already settled on,
            rather than replacing it. */}
        <button
          onClick={() => void saveAsCopy()}
          className="btn-solid"
          title={t("develop.header.saveAsCopyTitle")}
        >
          <Icon name="copy" size={14} />
          {t("develop.header.saveAsCopy")}
        </button>
        {/* The one key: finishing a photo means a file. */}
        <button onClick={() => setShowExport(true)} disabled={exporting} className="btn-accent ml-1">
          {exporting ? t("develop.header.exporting") : t("develop.header.export")}
        </button>
      </div>
    </header>
  );
}
