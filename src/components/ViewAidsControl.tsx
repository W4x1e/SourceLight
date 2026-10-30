import { useState } from "react";
import { t } from "../i18n";
import { useStore, type AidPlace } from "../store";
import { GRID_KINDS, PEAK_COLORS, peakRgb } from "../lib/composition";
import { comboToLabel } from "../lib/keymap";
import { Icon } from "../lib/icons";

/**
 * Focus peaking and the composition grid for one workspace: a toggle
 * for each, and — with peaking on — its colour and how fine the detail it
 * marks must be. Sits over the photo, top right, in Lightbox, Cull and
 * Compare; the keys (P, G) do the same without it.
 */
export default function ViewAidsControl({
  place,
  status,
  className = "absolute right-3 top-3 z-10",
}: {
  place: AidPlace;
  status?: "failed" | null;
  /** Where the buttons sit; the panels hang below them, right-aligned. */
  className?: string;
}) {
  const aids = useStore((s) => s.viewAids[place]);
  const togglePeaking = useStore((s) => s.togglePeaking);
  const setGrid = useStore((s) => s.setGrid);
  const color = useStore((s) => s.peakColor);
  const setColor = useStore((s) => s.setPeakColor);
  const sensitivity = useStore((s) => s.peakSensitivity);
  const setSensitivity = useStore((s) => s.setPeakSensitivity);
  const keymap = useStore((s) => s.keymap);
  const [gridOpen, setGridOpen] = useState(false);
  const key = (id: "toggle-peaking" | "cycle-grid") =>
    keymap[id] ? ` (${comboToLabel(keymap[id])})` : "";
  const [r, g, b] = peakRgb(color);
  const swatch = (rgb: [number, number, number]) =>
    `rgb(${rgb.map((v) => Math.round(v * 255)).join(" ")})`;

  const button = (on: boolean) =>
    `grid h-8 w-8 place-items-center rounded-sm transition-colors ${
      on
        ? "bg-chip-hover text-ink"
        : "bg-chip-hover text-ink/70 hover:text-ink"
    }`;

  return (
    <div
      className={className}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <div className="relative flex gap-1.5">
        <button
          onClick={() => togglePeaking(place)}
          aria-pressed={aids.peaking}
          aria-label={`${t("viewer.aids.peaking")}${key("toggle-peaking")}`}
          title={`${t("viewer.aids.peakingTitle")}${key("toggle-peaking")}`}
          className={button(aids.peaking)}
          style={aids.peaking ? { color: swatch([r, g, b]) } : undefined}
        >
          <Icon name="peaking" size={18} />
        </button>
        <button
          onClick={() => setGridOpen((o) => !o)}
          aria-pressed={aids.grid !== "off"}
          aria-expanded={gridOpen}
          aria-label={`${t("viewer.aids.grid")}${key("cycle-grid")}`}
          title={`${t("viewer.aids.grid")}${key("cycle-grid")}`}
          className={button(aids.grid !== "off")}
        >
          <Icon name="thirds" size={18} />
        </button>
        <div className="absolute right-0 top-full z-20 mt-1.5 flex w-max flex-col items-end gap-1.5">
          {gridOpen && (
            <div
              role="menu"
              className="anim-pop pop-right flex flex-col rounded-sm bg-chip-hover py-1 text-caption backdrop-blur"
            >
              {GRID_KINDS.map((k) => (
                <button
                  key={k.id}
                  role="menuitemradio"
                  aria-checked={aids.grid === k.id}
                  onClick={() => {
                    setGrid(place, k.id);
                    setGridOpen(false);
                  }}
                  className={`px-3 py-1 text-left hover:bg-white/10 ${aids.grid === k.id ? "text-ink" : "text-muted"}`}
                >
                  {t(k.label)}
                </button>
              ))}
            </div>
          )}

          {aids.peaking && (
            <div className="flex items-center gap-2 rounded-sm bg-chip-hover px-2 py-1.5 text-caption text-ink/80 backdrop-blur">
              {status === "failed" ? (
                <span>{t("viewer.aids.peakingFailed")}</span>
              ) : (
                <>
                  {PEAK_COLORS.map((c) => (
                    <button
                      key={c.id}
                      onClick={() => setColor(c.id)}
                      aria-label={t("viewer.aids.marks", { colour: t(c.label) })}
                      aria-pressed={color === c.id}
                      title={t(c.label)}
                      className={`h-3.5 w-3.5 rounded-full ${color === c.id ? "ring-2 ring-white/80 ring-offset-1 ring-offset-black" : ""}`}
                      style={{ background: swatch(c.rgb) }}
                    />
                  ))}
                  <label className="ml-1 flex items-center gap-1.5">
                    <span className="sr-only">{t("viewer.aids.sensitivity")}</span>
                    <input
                      type="range"
                      min={0}
                      max={100}
                      value={sensitivity}
                      onChange={(e) => setSensitivity(Number(e.target.value))}
                      title={t("viewer.aids.sensitivityTitle")}
                      className="w-20"
                    />
                  </label>
                </>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
