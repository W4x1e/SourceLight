import { Icon } from "../lib/icons";
import { t, tx } from "../i18n";
import { spotVals, type Spot } from "../types";
import NumberSlider from "./NumberSlider";

/**
 * The spot-removal side panel: the list of heal/clone spots on the photo and
 * the controls for the selected one.
 *
 * Extracted from `Develop.tsx`. It owns no state. The spots live in
 * the edit stack and the selection in Develop, which also turns clicks on the
 * photo into new spots, so this is a view over them plus the callbacks that
 * change them — the same `label` / `commit` shape Develop's own sliders use,
 * so undo history reads the same.
 */
export interface SpotPanelProps {
  spots: Spot[];
  selected: number;
  onSelect: (i: number) => void;
  onRemove: (i: number) => void;
  /** `commit` false while a slider is being dragged, true for a final value. */
  onEdit: (i: number, spot: Spot, label: string, commit: boolean) => void;
  /** Commits the dragged value as one undo step, labelled. */
  onCommit: (label: string) => void;
  onDone: () => void;
}

export default function SpotPanel({
  spots,
  selected,
  onSelect,
  onRemove,
  onEdit,
  onCommit,
  onDone,
}: SpotPanelProps) {
  const sel = spots[selected];
  return (
    <div className="p-3">
      <div className="mb-3 flex items-center gap-2">
        <span className="flex-1 text-title font-semibold">{t("tools.spot.title")}</span>
        {/* Solid, not the key: Export is the one key on Develop's screen. */}
        <button
          onClick={onDone}
          className="btn-solid px-2.5 py-1 text-caption"
        >
          {t("common.done")}
        </button>
      </div>
      <p className="mb-3 text-label leading-relaxed text-faint">
        {tx("tools.spot.intro", {
          heal: <b>{t("tools.spot.heal")}</b>,
          clone: <b>{t("tools.spot.clone")}</b>,
        })}
      </p>
      {spots.length === 0 ? (
        <p className="text-label text-faint">{t("tools.spot.empty")}</p>
      ) : (
        <div className="mb-3 flex flex-col gap-0.5">
          {spots.map((sp, i) => (
            <div
              key={i}
              onClick={() => onSelect(i)}
              className={`group flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 ${
                i === selected ? "bg-accent-dim/25 text-ink" : "text-muted hover:bg-hover"
              }`}
            >
              <Icon name="spot" size={13} className="shrink-0" />
              {/* A button, so Tab reaches the list (it was a click-only row). */}
              <button
                className="min-w-0 flex-1 truncate text-left text-caption"
                aria-pressed={i === selected}
                onClick={(e) => {
                  e.stopPropagation();
                  onSelect(i);
                }}
              >
                {t("tools.spot.itemName", {
                  mode: spotVals(sp).heal ? t("tools.spot.heal") : t("tools.spot.clone"),
                  number: i + 1,
                })}
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onRemove(i);
                }}
                className="text-caption text-faint opacity-0 transition-opacity hover:text-danger focus-visible:opacity-100 group-hover:opacity-100"
                title={t("tools.spot.delete")}
                aria-label={t("tools.spot.delete")}
              >
                <Icon name="close" size={12} />
              </button>
            </div>
          ))}
        </div>
      )}
      {sel && (
        <div className="border-t border-line pt-3">
          <div className="mb-2 seg-group">
            <button aria-pressed={spotVals(sel).heal}
              onClick={() => onEdit(selected, { ...sel, heal: true }, t("tools.spot.history.healMode"), true)}
              className={`flex-1 py-1 text-caption ${
                spotVals(sel).heal ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
              }`}
            >
              {t("tools.spot.heal")}
            </button>
            <button aria-pressed={!spotVals(sel).heal}
              onClick={() => onEdit(selected, { ...sel, heal: false }, t("tools.spot.history.cloneMode"), true)}
              className={`flex-1 py-1 text-caption ${
                !spotVals(sel).heal ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
              }`}
            >
              {t("tools.spot.clone")}
            </button>
          </div>
          <NumberSlider
            label={t("tools.spot.size")}
            value={spotVals(sel).radius}
            min={0.005}
            max={0.2}
            step={0.005}
            onChange={(v) => onEdit(selected, { ...sel, radius: v }, t("tools.spot.history.size"), false)}
            onCommit={() => onCommit(t("tools.spot.history.size"))}
          />
          <NumberSlider
            label={t("tools.spot.feather")}
            value={spotVals(sel).feather}
            min={0}
            max={1}
            step={0.01}
            onChange={(v) => onEdit(selected, { ...sel, feather: v }, t("tools.spot.history.feather"), false)}
            onCommit={() => onCommit(t("tools.spot.history.feather"))}
          />
        </div>
      )}
    </div>
  );
}
