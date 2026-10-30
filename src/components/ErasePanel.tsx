import { Icon } from "../lib/icons";
import { t } from "../i18n";
import type { Erase } from "../types";
import NumberSlider from "./NumberSlider";
import ProgressBar from "./ProgressBar";
import ModelDownloads, { useDownloading } from "./ModelDownloads";

/** How the object to erase is chosen: by clicking it, or by brushing over it. */
export type EraseHow = "click" | "brush";

/** The brush that paints a selection: its size (a fraction of the frame's
 *  width, as `Stroke.radius`), and whether it takes away rather than adds. */
export interface EraseBrush {
  size: number;
  remove: boolean;
}

export interface ErasePanelProps {
  erases: Erase[];
  how: EraseHow;
  onHow: (how: EraseHow) => void;
  brush: EraseBrush;
  onBrush: (brush: EraseBrush) => void;
  /** Something is selected to erase. */
  selected: boolean;
  /** Clicks so far, which change what the click hint says. */
  points: number;
  /** Click to select is working out the selection. */
  picking: boolean;
  /** An erase is running: `pct` once the model has started, `null` while it
   *  loads or downloads. */
  running: boolean;
  pct: number | null;
  /** The erase has a job, so the model is loaded: no download to show. */
  started: boolean;
  error: string | null;
  onErase: () => void;
  onClear: () => void;
  onCancel: () => void;
  onRemove: (i: number) => void;
  onDone: () => void;
}

/**
 * Magic erase's side panel, in Retouch: how to select an object, the button
 * that erases it, and the erases already made.
 *
 * It owns no state. The selection lives in Develop, which draws it over the
 * photo and sends it to `develop_erase`; the erases live in the edit stack,
 * where only the backend changes them.
 */
export default function ErasePanel({
  erases,
  how,
  onHow,
  brush,
  onBrush,
  selected,
  points,
  picking,
  running,
  pct,
  started,
  error,
  onErase,
  onClear,
  onCancel,
  onRemove,
  onDone,
}: ErasePanelProps) {
  // A segment of the rail's one either/or control (`.seg-group`), not a
  // loose raised button: these read as two actions rather than one choice.
  const downloading = useDownloading();
  const seg = (on: boolean) =>
    `flex-1 py-1 text-caption ${on ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"}`;
  return (
    <div className="p-3">
      <div className="mb-3 flex items-center gap-2">
        <span className="flex-1 text-title font-semibold">{t("tools.erase.title")}</span>
        {/* Solid, not the key: Export is the one key on Develop's screen. */}
        <button onClick={onDone} className="btn-solid px-2.5 py-1 text-caption">
          {t("common.done")}
        </button>
      </div>
      <p className="mb-3 text-label leading-relaxed text-faint">{t("tools.erase.intro")}</p>

      <div className="mb-2 seg-group">
        <button aria-pressed={how === "click"} onClick={() => onHow("click")} className={seg(how === "click")}>
          {t("tools.erase.click")}
        </button>
        <button aria-pressed={how === "brush"} onClick={() => onHow("brush")} className={seg(how === "brush")}>
          {t("tools.erase.brush")}
        </button>
      </div>
      {how === "click" ? (
        <p className="mb-2 text-label text-muted">
          {picking ? t("tools.erase.selecting") : t(points ? "tools.erase.clickRefine" : "tools.erase.clickPrompt")}
        </p>
      ) : (
        <div className="mb-2">
          <div className="mb-1 seg-group">
            <button aria-pressed={!brush.remove} onClick={() => onBrush({ ...brush, remove: false })} className={seg(!brush.remove)}>
              {t("tools.erase.brushAdd")}
            </button>
            <button aria-pressed={brush.remove} onClick={() => onBrush({ ...brush, remove: true })} className={seg(brush.remove)}>
              {t("tools.erase.brushRemove")}
            </button>
          </div>
          <NumberSlider
            label={t("tools.erase.brushSize")}
            value={brush.size}
            min={0.005}
            max={0.2}
            step={0.005}
            onChange={(size) => onBrush({ ...brush, size })}
            onCommit={() => {}}
          />
        </div>
      )}

      {running ? (
        <div className="mb-3">
          {/* While the model downloads, its row alone, with its Cancel: not
              an empty "Starting…" bar and a second Cancel above it. */}
          {!started && downloading ? (
            <ModelDownloads />
          ) : (
            <div className="flex items-center gap-2">
              <ProgressBar
                pct={pct}
                label={t("tools.erase.title")}
                className="h-1 flex-1 overflow-hidden rounded bg-line"
                barClassName="h-full bg-accent"
              />
              <span className="text-label text-muted tabular-nums">{pct === null ? t("common.starting") : `${pct}%`}</span>
              <button className="text-label text-accent hover:underline" onClick={onCancel}>
                {t("common.cancel")}
              </button>
            </div>
          )}
        </div>
      ) : (
        <div className="mb-3 flex items-center gap-2">
          <button className="btn-solid flex-1" disabled={!selected || picking} onClick={onErase}>
            {t("tools.erase.run")}
          </button>
          <button
            className="btn-ghost text-caption text-muted hover:text-ink disabled:pointer-events-none disabled:opacity-40"
            disabled={!selected && points === 0}
            onClick={onClear}
          >
            {t("tools.erase.clear")}
          </button>
        </div>
      )}
      {error && <p className="mb-3 text-label text-danger">{error}</p>}

      {erases.length === 0 ? (
        <p className="text-label text-faint">{t("tools.erase.empty")}</p>
      ) : (
        <div className="flex flex-col gap-0.5 border-t border-line pt-2">
          {erases.map((_, i) => (
            <div key={i} className="group flex items-center gap-2 rounded-sm px-2 py-1.5 text-muted hover:bg-hover">
              <Icon name="heal" size={13} />
              <span className="flex-1 text-caption">{t("tools.erase.itemName", { number: i + 1 })}</span>
              <button
                onClick={() => onRemove(i)}
                className="text-caption text-faint opacity-0 transition-opacity hover:text-danger focus-visible:opacity-100 group-hover:opacity-100"
                title={t("tools.erase.delete")}
                aria-label={t("tools.erase.delete")}
              >
                <Icon name="close" size={12} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
