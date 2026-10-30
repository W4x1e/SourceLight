import type { Dispatch, SetStateAction } from "react";
import { t, formatNumber } from "../../i18n";
import { Icon } from "../../lib/icons";
import { hint } from "./shortcutHint";

type BaMode = "edit" | "split" | "before";

interface Props {
  baMode: BaMode;
  setBaMode: (m: BaMode) => void;
  cropping: boolean;
  softProof: boolean;
  setSoftProof: Dispatch<SetStateAction<boolean>>;
  /** A tool draws the photo at fit (crop, spots), so zoom is off. */
  canvasOwned: boolean;
  /** Output pixels per image pixel; null is fit-to-window. */
  zoom: number | null;
  setZoom: (zoom: number | null) => void;
  setPan: (pan: { x: number; y: number }) => void;
  zoomBy: (factor: number) => void;
  /** The magnification fit-to-window is showing. */
  fitZoom: () => number;
  zoomKey: string;
  zoomInKey: string;
  zoomOutKey: string;
}

/**
 * The view bar: how the photo is shown, under the photo. Compare and
 * the gamut warning change what you see, never the edit, so they
 * sit with zoom rather than among the editing actions up top. It
 * wraps onto a second line rather than crushing its switches when
 * the canvas is very narrow (the minimum window with Presets open).
 */
export default function ViewBar({
  baMode,
  setBaMode,
  cropping,
  softProof,
  setSoftProof,
  canvasOwned,
  zoom,
  setZoom,
  setPan,
  zoomBy,
  fitZoom,
  zoomKey,
  zoomInKey,
  zoomOutKey,
}: Props) {
  return (
    <div className="flex min-h-10 shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-t border-line bg-panel px-3 py-1.5">
      <div className="flex shrink-0 items-center gap-2">
        <div className="seg-group" role="group" aria-label={t("develop.view.compareLabel")}>
          {(
            [
              ["before", "develop.view.before", "develop.view.beforeTitle"],
              ["split", "develop.view.split", "develop.view.splitTitle"],
              ["edit", "develop.view.after", "develop.view.afterTitle"],
            ] as const
          ).map(([m, label, title]) => (
            <button
              key={m}
              onClick={() => setBaMode(m)}
              // Crop draws the uncropped frame; Before would sit the crop
              // box over a differently framed picture.
              disabled={cropping && m !== "edit"}
              aria-pressed={baMode === m}
              className={`px-2.5 py-1 text-caption ${baMode === m ? "seg-on" : "text-muted hover:bg-hover"}`}
              title={t(title)}
            >
              {t(label)}
            </button>
          ))}
        </div>
        {/* The bar's regular weight: as a button's medium it read as
            bolder than the Before/Split/After beside it. */}
        <button
          onClick={() => setSoftProof((v) => !v)}
          className={`btn-ghost px-2.5 py-1 font-normal ${softProof ? "seg-on" : "text-muted hover:text-ink"}`}
          aria-pressed={softProof}
          title={t("develop.view.gamutWarningTitle")}
        >
          {t("develop.view.gamutWarning")}
        </button>
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-2">
        {/* Crop and Retouch draw at fit: their zoom controls did
            nothing but change the number. */}
        <div className={`seg-group ${canvasOwned ? "pointer-events-none opacity-40" : ""}`} aria-disabled={canvasOwned}>
          <button aria-pressed={zoom === null}
            onClick={() => setZoom(null)}
            title={hint(t("develop.view.fit"), zoomKey)}
            className={`px-2.5 py-1 text-caption ${
              zoom === null ? "seg-on" : "text-muted hover:bg-hover"
            }`}
          >
            {t("develop.view.fit")}
          </button>
          <button aria-pressed={zoom === 1}
            onClick={() => {
              setZoom(1);
              setPan({ x: 0.5, y: 0.5 });
            }}
            className={`px-2.5 py-1 text-caption ${
              zoom === 1 ? "seg-on" : "text-muted hover:bg-hover"
            }`}
            title={hint(t("develop.view.actualPixelsTitle"), zoomKey)}
          >
            {t("develop.view.actualPixels")}
          </button>
        </div>
        <button onClick={() => zoomBy(1 / 1.5)} disabled={canvasOwned || zoom === null} className="icon-chip disabled:opacity-40" title={hint(t("develop.view.zoomOut"), zoomOutKey)}>
          <Icon name="minus" size={15} />
        </button>
        <span className="w-11 text-center text-caption tabular-nums text-muted">
          {formatNumber(Math.round((canvasOwned ? fitZoom() : (zoom ?? fitZoom())) * 100) / 100, { style: "percent" })}
        </span>
        <button onClick={() => zoomBy(1.5)} disabled={canvasOwned} className="icon-chip disabled:opacity-40" title={hint(t("develop.view.zoomIn"), zoomInKey)}>
          <Icon name="plus" size={15} />
        </button>
      </div>
    </div>
  );
}
