import { useRef, useState } from "react";
import type { MaskComponent, Stroke } from "../../types";
import type { Size, View } from "../../lib/viewport";
import { useFitBox } from "./useFitBox";
import { useSpaceHand } from "../../lib/useSpaceHand";

/**
 * Drag handles for the active mask component, laid over the preview.
 *
 * It works in the framed image's 0..1 UV — the space masks are stored in — by
 * measuring where that image lands on screen (`useFitBox`). At fit-to-window
 * that box is the `object-contain` letterbox; when the user zooms or pans while
 * masking, the box follows the renderer's viewport, so the shape stays pinned to
 * the pixels the shader applies it to. Because the renderer already zooms the
 * coverage with the photo, tracking the box is all the overlay has to do.
 *
 * It edits one component at a time (the selected one); combining components with
 * add/subtract/intersect happens in the right-rail panel.
 */

interface Props {
  frameAspect: number;
  /** Framed image dimensions in pixels, and the active zoom/pan (null = fit). */
  framePx: Size;
  view: View | null;
  component: MaskComponent;
  /** Brush settings, used only when the component is a brush. Radius is a
   *  fraction of the frame width, matching `Stroke.radius`. */
  brush: { size: number; hardness: number; erase: boolean };
  onChange: (component: MaskComponent) => void;
  onCommit: (component: MaskComponent) => void;
  /** Wheel zoom and middle-drag pan, so the canvas stays navigable even though
   *  this overlay covers it. */
  /** A wheel over the overlay: the canvas decides zoom or pan (lib/wheel.ts). */
  onZoom?: (e: React.WheelEvent) => void;
  onPan?: (dxCss: number, dyCss: number) => void;
}

type Handle = "move" | "rx" | "ry" | "p1" | "p2";

export default function MaskOverlay({
  frameAspect,
  framePx,
  view,
  component,
  brush,
  onChange,
  onCommit,
  onZoom,
  onPan,
}: Props) {
  const { outer, box: fit } = useFitBox(frameAspect, view, framePx);
  const hand = useSpaceHand();
  const boxEl = useRef<HTMLDivElement>(null);

  const held = useRef<Handle | null>(null);
  const latest = useRef(component);
  if (!held.current) latest.current = component;

  const uvAt = (e: React.PointerEvent) => {
    const r = boxEl.current!.getBoundingClientRect();
    return {
      x: Math.min(Math.max((e.clientX - r.left) / r.width, 0), 1),
      y: Math.min(Math.max((e.clientY - r.top) / r.height, 0), 1),
    };
  };

  const begin = (e: React.PointerEvent, h: Handle) => {
    if (e.button !== 0 || hand.pans(e)) return; // leave panning to the outer
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    held.current = h;
  };

  const emit = (c: MaskComponent) => {
    latest.current = c;
    onChange(c);
  };

  const onHandleMove = (e: React.PointerEvent) => {
    const h = held.current;
    if (!h) return;
    const p = uvAt(e);
    const c = latest.current;
    const s = c.shape;

    if (s.type === "radial") {
      const cx = s.cx ?? 0.5;
      const cy = s.cy ?? 0.5;
      if (h === "move") {
        emit({ ...c, shape: { ...s, cx: p.x, cy: p.y } });
      } else if (h === "rx") {
        emit({ ...c, shape: { ...s, radiusX: Math.max(0.02, Math.abs(p.x - cx)) } });
      } else if (h === "ry") {
        emit({ ...c, shape: { ...s, radiusY: Math.max(0.02, Math.abs(p.y - cy)) } });
      }
    } else if (s.type === "linear") {
      if (h === "p1") emit({ ...c, shape: { ...s, x1: p.x, y1: p.y } });
      else if (h === "p2") emit({ ...c, shape: { ...s, x2: p.x, y2: p.y } });
    }
  };

  const endHandle = () => {
    if (!held.current) return;
    held.current = null;
    onCommit(latest.current);
  };

  // ---- Middle-drag, or Space and drag, pans (left drag paints / moves handles) ----
  const pan = useRef<{ x: number; y: number } | null>(null);

  const onOuterDown = (e: React.PointerEvent) => {
    if (hand.pans(e) && onPan && view) {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      pan.current = { x: e.clientX, y: e.clientY };
    }
  };
  const onOuterMove = (e: React.PointerEvent) => {
    if (pan.current) {
      onPan?.(e.clientX - pan.current.x, e.clientY - pan.current.y);
      pan.current = { x: e.clientX, y: e.clientY };
      return;
    }
    onHandleMove(e);
  };
  const onOuterUp = () => {
    if (pan.current) {
      pan.current = null;
      return;
    }
    endHandle();
  };

  // ---- Brush painting ----
  const painting = useRef(false);
  // Where the brush is, to draw its size and hardness before the first dab:
  // only the dab told you how big it was.
  const [ring, setRing] = useState<{ x: number; y: number } | null>(null);
  const lastDab = useRef<{ x: number; y: number } | null>(null);

  const dab = (p: { x: number; y: number }) => {
    const c = latest.current;
    if (c.shape.type !== "brush") return;
    const stroke: Stroke = {
      x: p.x,
      y: p.y,
      radius: brush.size,
      hardness: brush.hardness,
      erase: brush.erase,
    };
    emit({ ...c, shape: { ...c.shape, strokes: [...c.shape.strokes, stroke] } });
    lastDab.current = p;
  };

  const onPaintDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || hand.pans(e)) return; // let panning bubble to the outer
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    painting.current = true;
    dab(uvAt(e));
  };
  const onPaintMove = (e: React.PointerEvent) => {
    setRing(uvAt(e));
    if (!painting.current) return;
    const p = uvAt(e);
    const last = lastDab.current;
    // Dab spacing: skip until the pointer has moved a quarter of the brush
    // radius, so a stroke is continuous without thousands of dabs.
    if (last && Math.hypot(p.x - last.x, p.y - last.y) < brush.size * 0.25) return;
    dab(p);
  };
  const onPaintUp = () => {
    if (!painting.current) return;
    painting.current = false;
    lastDab.current = null;
    onCommit(latest.current);
  };

  const s = component.shape;

  const pct = (v: number) => `${v * 100}%`;

  return (
    <div
      ref={outer}
      className={`absolute inset-0 touch-none overflow-hidden ${hand.held ? "cursor-grab" : ""}`}
      {...hand.hover}
      onPointerDown={onOuterDown}
      onPointerMove={onOuterMove}
      onPointerUp={onOuterUp}
      onPointerCancel={onOuterUp}
      onWheel={(e) => onZoom?.(e)}
    >
      <div
        ref={boxEl}
        className="absolute"
        style={{ left: fit.left, top: fit.top, width: fit.width, height: fit.height }}
      >
        {s.type === "brush" ? (
          // Paint surface — captures dabs across the whole framed image.
          <div
            className={`absolute inset-0 ${hand.held ? "cursor-grab" : "cursor-crosshair"}`}
            onPointerDown={onPaintDown}
            onPointerMove={onPaintMove}
            onPointerLeave={() => setRing(null)}
            onPointerUp={onPaintUp}
            onPointerCancel={onPaintUp}
          >
            {ring && (
              <>
                {/* The brush: its radius, and its hard core; dashed to erase. */}
                <div
                  className={`pointer-events-none absolute rounded-full border border-white/90 shadow-[0_0_0_1px_rgba(0,0,0,0.5)] ${brush.erase ? "border-dashed" : ""}`}
                  style={{
                    left: `${ring.x * 100}%`,
                    top: `${ring.y * 100}%`,
                    width: 2 * brush.size * fit.width,
                    height: 2 * brush.size * fit.width,
                    transform: "translate(-50%, -50%)",
                  }}
                />
                <div
                  className="pointer-events-none absolute rounded-full border border-white/50"
                  style={{
                    left: `${ring.x * 100}%`,
                    top: `${ring.y * 100}%`,
                    width: 2 * brush.size * brush.hardness * fit.width,
                    height: 2 * brush.size * brush.hardness * fit.width,
                    transform: "translate(-50%, -50%)",
                  }}
                />
              </>
            )}
          </div>
        ) : s.type === "radial" || s.type === "linear" ? (
          <>
            <svg
              viewBox="0 0 100 100"
              preserveAspectRatio="none"
              className="pointer-events-none absolute inset-0 h-full w-full overflow-visible"
            >
              {s.type === "radial" ? (
                <>
                  <ellipse
                    cx={(s.cx ?? 0.5) * 100}
                    cy={(s.cy ?? 0.5) * 100}
                    rx={(s.radiusX ?? 0.25) * 100}
                    ry={(s.radiusY ?? 0.25) * 100}
                    fill="rgba(125,144,255,0.14)"
                    stroke="white"
                    strokeWidth={0.4}
                    vectorEffect="non-scaling-stroke"
                  />
                  <ellipse
                    cx={(s.cx ?? 0.5) * 100}
                    cy={(s.cy ?? 0.5) * 100}
                    rx={(s.radiusX ?? 0.25) * (1 - (s.feather ?? 0.5)) * 100}
                    ry={(s.radiusY ?? 0.25) * (1 - (s.feather ?? 0.5)) * 100}
                    fill="none"
                    stroke="rgba(255,255,255,0.4)"
                    strokeWidth={0.4}
                    strokeDasharray="2 2"
                    vectorEffect="non-scaling-stroke"
                  />
                </>
              ) : (
                <line
                  x1={(s.x1 ?? 0.2) * 100}
                  y1={(s.y1 ?? 0.5) * 100}
                  x2={(s.x2 ?? 0.8) * 100}
                  y2={(s.y2 ?? 0.5) * 100}
                  stroke="white"
                  strokeWidth={0.4}
                  vectorEffect="non-scaling-stroke"
                />
              )}
            </svg>

            {/* Handles as positioned dots, so their hit target is a real size
                regardless of the frame's pixel dimensions. */}
            {s.type === "radial" ? (
              <>
                <Grip left={pct(s.cx ?? 0.5)} top={pct(s.cy ?? 0.5)} onDown={(e) => begin(e, "move")} filled />
                <Grip left={pct((s.cx ?? 0.5) + (s.radiusX ?? 0.25))} top={pct(s.cy ?? 0.5)} onDown={(e) => begin(e, "rx")} />
                <Grip left={pct(s.cx ?? 0.5)} top={pct((s.cy ?? 0.5) + (s.radiusY ?? 0.25))} onDown={(e) => begin(e, "ry")} />
              </>
            ) : (
              <>
                <Grip left={pct(s.x1 ?? 0.2)} top={pct(s.y1 ?? 0.5)} onDown={(e) => begin(e, "p1")} filled />
                <Grip left={pct(s.x2 ?? 0.8)} top={pct(s.y2 ?? 0.5)} onDown={(e) => begin(e, "p2")} />
              </>
            )}
          </>
        ) : null}
      </div>
    </div>
  );
}

function Grip({
  left,
  top,
  onDown,
  filled,
}: {
  left: string;
  top: string;
  onDown: (e: React.PointerEvent) => void;
  filled?: boolean;
}) {
  return (
    <div
      onPointerDown={onDown}
      /* A dark ring under the white: the filled grip was white on white
         over a white dress. */
      className={`absolute h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 cursor-grab rounded-full border-2 border-white shadow-[0_0_0_1.5px_rgb(0_0_0/0.65)] ${
        filled ? "bg-white/80" : "bg-chip"
      }`}
      style={{ left, top }}
    />
  );
}
