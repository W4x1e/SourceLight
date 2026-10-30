import { useRef } from "react";
import type { Size, View } from "../../lib/viewport";
import { useFitBox } from "./useFitBox";
import { useSpaceHand } from "../../lib/useSpaceHand";
import { t } from "../../i18n";

/**
 * Click-to-select overlay for AI masking (SAM2).
 *
 * Click the thing you want; alt/right-click to exclude a region. Reports clicks
 * in framed-image UV — the space masks (and the model's point prompts) live in —
 * by measuring where the framed image lands on screen (`useFitBox`), so a click
 * lands on the same pixel whether the view is fit-to-window or zoomed/panned.
 */

export interface SelectPoint {
  x: number;
  y: number;
  include: boolean;
}

interface Props {
  frameAspect: number;
  framePx: Size;
  view: View | null;
  points: SelectPoint[];
  busy: boolean;
  onPick: (point: SelectPoint) => void;
  /** A wheel over the overlay: the canvas decides zoom or pan (lib/wheel.ts). */
  onZoom?: (e: React.WheelEvent) => void;
  onPan?: (dxCss: number, dyCss: number) => void;
}

export default function ClickSelectOverlay({
  frameAspect,
  framePx,
  view,
  points,
  busy,
  onPick,
  onZoom,
  onPan,
}: Props) {
  const { outer, box: fit } = useFitBox(frameAspect, view, framePx);
  const boxEl = useRef<HTMLDivElement>(null);
  const pan = useRef<{ x: number; y: number } | null>(null);
  const hand = useSpaceHand();
  const lastPoint = points[points.length - 1];

  const pick = (e: React.PointerEvent, include: boolean) => {
    const r = boxEl.current!.getBoundingClientRect();
    onPick({
      x: Math.min(Math.max((e.clientX - r.left) / r.width, 0), 1),
      y: Math.min(Math.max((e.clientY - r.top) / r.height, 0), 1),
      include,
    });
  };

  return (
    <div
      ref={outer}
      className="absolute inset-0 touch-none overflow-hidden"
      {...hand.hover}
      onPointerDown={(e) => {
        if (hand.pans(e) && onPan && view) {
          e.preventDefault();
          e.currentTarget.setPointerCapture(e.pointerId);
          pan.current = { x: e.clientX, y: e.clientY };
        }
      }}
      onPointerMove={(e) => {
        if (!pan.current) return;
        onPan?.(e.clientX - pan.current.x, e.clientY - pan.current.y);
        pan.current = { x: e.clientX, y: e.clientY };
      }}
      onPointerUp={() => {
        pan.current = null;
      }}
      onWheel={(e) => onZoom?.(e)}
    >
      <div
        ref={boxEl}
        className="absolute"
        style={{ left: fit.left, top: fit.top, width: fit.width, height: fit.height }}
      >
        <div
          className={`absolute inset-0 ${busy ? "cursor-wait" : hand.held ? "cursor-grab" : "cursor-crosshair"}`}
          onPointerDown={(e) => {
            if (busy || hand.pans(e)) return; // middle-drag, or Space and drag, pans
            e.stopPropagation();
            // Alt or a secondary button marks a region to exclude.
            pick(e, !(e.altKey || e.button === 2));
          }}
          onContextMenu={(e) => e.preventDefault()}
        />
        {points.map((p, i) => (
          // Include and exclude by shape, + and −, on a dark chip:
          // white and red dots vanished on snow and told apart by hue alone.
          <div
            key={i}
            className="pointer-events-none absolute grid h-4 w-4 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full border-2 border-white bg-chip-hover text-[11px] font-bold leading-none text-white shadow-[0_0_0_1.5px_rgb(0_0_0/0.65)]"
            style={{ left: `${p.x * 100}%`, top: `${p.y * 100}%` }}
            title={p.include ? t("tools.pick.include") : t("tools.pick.exclude")}
          >
            {p.include ? "+" : "−"}
          </div>
        ))}
        {/* The click being worked on turns a ring round its point, where the
            eye already is; the chip above and the rail say which step. */}
        {busy && lastPoint && (
          <div
            className="pointer-events-none absolute -translate-x-1/2 -translate-y-1/2"
            style={{ left: `${lastPoint.x * 100}%`, top: `${lastPoint.y * 100}%` }}
          >
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-white/25 border-t-white shadow-[0_0_0_1.5px_rgb(0_0_0/0.35)]" />
          </div>
        )}
      </div>
    </div>
  );
}
