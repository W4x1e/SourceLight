import { useLayoutEffect, useRef, useState } from "react";
import type { Spot } from "../../types";
import { spotVals } from "../../types";

/**
 * Spot / heal / clone overlay. Click empty space to drop a spot; drag its filled
 * target handle to move what's repaired, or its hollow source handle to move
 * what's copied from. Measures the letterboxed image and works in framed-image
 * UV, the space spots are stored in — spot mode is fit-to-window (see `Develop`).
 *
 * Circles are drawn in a stretched `viewBox` so they match the shader's UV-space
 * coverage exactly (an ellipse on a non-square frame), the same convention the
 * radial mask overlay uses.
 */

interface Props {
  frameAspect: number;
  spots: Spot[];
  selected: number;
  onSelect: (i: number) => void;
  onAdd: (x: number, y: number) => void;
  onChange: (i: number, s: Spot) => void;
  onCommit: (i: number, s: Spot) => void;
}

type Grab = { i: number; which: "target" | "source" } | null;

export default function SpotOverlay({ frameAspect, spots, selected, onSelect, onAdd, onChange, onCommit }: Props) {
  const outer = useRef<HTMLDivElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState({ left: 0, top: 0, width: 0, height: 0 });

  useLayoutEffect(() => {
    const el = outer.current;
    if (!el) return;
    const measure = () => {
      const w = el.clientWidth;
      const h = el.clientHeight;
      let bw = w;
      let bh = w / frameAspect;
      if (bh > h) {
        bh = h;
        bw = h * frameAspect;
      }
      setFit({ width: bw, height: bh, left: (w - bw) / 2, top: (h - bh) / 2 });
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, [frameAspect]);

  const held = useRef<Grab>(null);
  const latest = useRef(spots);
  latest.current = spots;

  const uvAt = (e: React.PointerEvent) => {
    const r = boxRef.current!.getBoundingClientRect();
    return {
      x: Math.min(Math.max((e.clientX - r.left) / r.width, 0), 1),
      y: Math.min(Math.max((e.clientY - r.top) / r.height, 0), 1),
    };
  };

  const beginGrab = (e: React.PointerEvent, i: number, which: "target" | "source") => {
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    held.current = { i, which };
    onSelect(i);
  };

  const onMove = (e: React.PointerEvent) => {
    const g = held.current;
    if (!g) return;
    const p = uvAt(e);
    const s = latest.current[g.i];
    if (!s) return;
    onChange(
      g.i,
      g.which === "target"
        ? { ...s, targetX: p.x, targetY: p.y }
        : { ...s, sourceX: p.x, sourceY: p.y },
    );
  };

  const end = () => {
    const g = held.current;
    if (!g) return;
    held.current = null;
    const s = latest.current[g.i];
    if (s) onCommit(g.i, s);
  };

  const pct = (v: number) => `${v * 100}%`;

  return (
    <div
      ref={outer}
      className="absolute inset-0 touch-none"
      onPointerMove={onMove}
      onPointerUp={end}
      onPointerCancel={end}
    >
      <div
        ref={boxRef}
        className="absolute"
        style={{ left: fit.left, top: fit.top, width: fit.width, height: fit.height }}
      >
        {/* Click empty space to add a spot at that point. */}
        <div
          className="absolute inset-0 cursor-crosshair"
          onPointerDown={(e) => {
            const p = uvAt(e);
            onAdd(p.x, p.y);
          }}
        />
        <svg
          viewBox="0 0 100 100"
          preserveAspectRatio="none"
          className="pointer-events-none absolute inset-0 h-full w-full overflow-visible"
        >
          {spots.map((sp, i) => {
            const v = spotVals(sp);
            const active = i === selected;
            return (
              <g key={i}>
                {/* Each outline twice, a dark one under a white one, at real
                    pixel widths: 0.3px white vanished over a pale sky. */}
                {(["halo", "line"] as const).map((pass) => (
                  <g key={pass}>
                    <line
                      x1={v.sourceX * 100}
                      y1={v.sourceY * 100}
                      x2={v.targetX * 100}
                      y2={v.targetY * 100}
                      stroke={pass === "halo" ? "rgba(0,0,0,0.5)" : "rgba(255,255,255,0.7)"}
                      strokeWidth={pass === "halo" ? 2 : 1}
                      strokeDasharray="3 3"
                      vectorEffect="non-scaling-stroke"
                    />
                    <circle
                      cx={v.sourceX * 100}
                      cy={v.sourceY * 100}
                      r={v.radius * 100}
                      fill="none"
                      stroke={pass === "halo" ? "rgba(0,0,0,0.5)" : "rgba(255,255,255,0.75)"}
                      strokeWidth={pass === "halo" ? 2 : 1}
                      strokeDasharray="4 3"
                      vectorEffect="non-scaling-stroke"
                    />
                    <circle
                      cx={v.targetX * 100}
                      cy={v.targetY * 100}
                      r={v.radius * 100}
                      fill={pass === "halo" ? "none" : active ? "rgba(255,255,255,0.16)" : "rgba(255,255,255,0.06)"}
                      stroke={pass === "halo" ? "rgba(0,0,0,0.5)" : "white"}
                      strokeWidth={pass === "halo" ? (active ? 3 : 2) : active ? 2 : 1}
                      vectorEffect="non-scaling-stroke"
                    />
                  </g>
                ))}
              </g>
            );
          })}
        </svg>
        {spots.map((sp, i) => {
          const v = spotVals(sp);
          return (
            <div key={i}>
              <Handle left={pct(v.targetX)} top={pct(v.targetY)} onDown={(e) => beginGrab(e, i, "target")} filled active={i === selected} />
              <Handle left={pct(v.sourceX)} top={pct(v.sourceY)} onDown={(e) => beginGrab(e, i, "source")} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Handle({
  left,
  top,
  onDown,
  filled,
  active,
}: {
  left: string;
  top: string;
  onDown: (e: React.PointerEvent) => void;
  filled?: boolean;
  active?: boolean;
}) {
  return (
    <div
      onPointerDown={onDown}
      /* White on a dark ring over any photo: the accent is white now,
         so a filled target was white on a pale sky. The selected spot is the
         bigger one, not a different white. */
      className={`absolute -translate-x-1/2 -translate-y-1/2 cursor-grab rounded-full border-2 border-white shadow-[0_0_0_1.5px_rgb(0_0_0/0.65)] ${
        active ? "h-4 w-4" : "h-3 w-3"
      } ${filled ? "bg-white/80" : "bg-chip"}`}
      style={{ left, top }}
    />
  );
}
