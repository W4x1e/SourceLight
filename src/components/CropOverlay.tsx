import { useLayoutEffect, useRef, useState } from "react";
import type { GridKind } from "../lib/composition";
import CompositionGrid from "./CompositionGrid";
import type { Crop } from "../types";
import { dragCrop, type Grip } from "../lib/cropDrag";

/** The corners, as brackets inside the crop's own corners: centred on
 *  them, half of each 16px grip was cut off at the canvas edge whenever the
 *  crop was the full frame. Each takes a 24px area for the pointer. */
const CORNERS: { grip: Grip; style: React.CSSProperties; cursor: string }[] = [
  { grip: "nw", style: { left: 0, top: 0, borderLeftWidth: 3, borderTopWidth: 3 }, cursor: "nwse-resize" },
  { grip: "ne", style: { right: 0, top: 0, borderRightWidth: 3, borderTopWidth: 3 }, cursor: "nesw-resize" },
  { grip: "sw", style: { left: 0, bottom: 0, borderLeftWidth: 3, borderBottomWidth: 3 }, cursor: "nesw-resize" },
  { grip: "se", style: { right: 0, bottom: 0, borderRightWidth: 3, borderBottomWidth: 3 }, cursor: "nwse-resize" },
];

/** The edges: a strip along each, inside the crop, that drags that side alone.
 *  Pressing an edge used to move the whole rectangle. `corner` is the
 *  corners' hit area the strips stop short of, `depth` the strip's own. */
const edges = (corner: number, depth: number): { grip: Grip; style: React.CSSProperties; bar: React.CSSProperties; cursor: string }[] => [
  { grip: "n", style: { left: corner, right: corner, top: 0, height: depth }, bar: { left: "50%", top: 0, width: 24, height: 3, marginLeft: -12 }, cursor: "ns-resize" },
  { grip: "s", style: { left: corner, right: corner, bottom: 0, height: depth }, bar: { left: "50%", bottom: 0, width: 24, height: 3, marginLeft: -12 }, cursor: "ns-resize" },
  { grip: "w", style: { top: corner, bottom: corner, left: 0, width: depth }, bar: { top: "50%", left: 0, height: 24, width: 3, marginTop: -12 }, cursor: "ew-resize" },
  { grip: "e", style: { top: corner, bottom: corner, right: 0, width: depth }, bar: { top: "50%", right: 0, height: 24, width: 3, marginTop: -12 }, cursor: "ew-resize" },
];

interface Props {
  /** Aspect ratio of the *uncropped* frame, width over height. */
  frameAspect: number;
  crop: Crop;
  /** Locks the shape while dragging. `null` is a free crop. */
  ratio: number | null;
  onChange: (crop: Crop) => void;
  onCommit: (crop: Crop) => void;
  /** The guide drawn while dragging; thirds unless another is chosen. */
  grid?: GridKind;
  /** A fine grid over the box while Straighten moves, to level against. */
  levelling?: boolean;
  /** Grips a finger can hold: the phone editor's. */
  touch?: boolean;
}

/**
 * The draggable crop rectangle, laid over the preview.
 *
 * Sits on top of an `object-contain` image, so the photo is letterboxed inside
 * whatever shape the pane happens to be. Everything here works in *image*
 * coordinates and the container is positioned to match the letterboxed area —
 * getting that wrong means the rectangle drifts away from the photo as the
 * window is resized, which is the same class of bug the canvas sizing already
 * had.
 */
export default function CropOverlay({ frameAspect, crop, ratio, onChange, onCommit, grid = "thirds", levelling = false, touch = false }: Props) {
  const corner = touch ? 44 : 24;
  const EDGES = edges(corner, touch ? 24 : 10);
  const outer = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);

  // The letterboxed rectangle the `object-contain` image occupies, measured
  // explicitly rather than reproduced with a `margin:auto + inset:0 +
  // aspect-ratio` CSS trick — that trick resolves inconsistently against a
  // flex parent and left the crop rectangle detached from the photo.
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
  // Refs rather than state, for the reason the curve editor documents: a fast
  // drag can deliver pointerdown and pointerup within one frame, and state read
  // in the handler would still be the pre-drag value.
  const grip = useRef<Grip | null>(null);
  const origin = useRef({ x: 0, y: 0, crop });
  const latest = useRef(crop);
  if (!grip.current) latest.current = crop;

  const [active, setActive] = useState(false);

  const atEvent = (e: React.PointerEvent) => {
    const r = box.current!.getBoundingClientRect();
    return {
      x: (e.clientX - r.left) / r.width,
      y: (e.clientY - r.top) / r.height,
    };
  };

  const onMove = (e: React.PointerEvent) => {
    const held = grip.current;
    if (!held) return;
    const at = atEvent(e);
    const start = origin.current;
    // A corner keeps the opposite one pinned, an edge moves its side alone,
    // the ratio holds either way (lib/cropDrag).
    const next = dragCrop(start.crop, held, at.x - start.x, at.y - start.y, ratio, frameAspect);
    latest.current = next;
    onChange(next);
  };

  const end = () => {
    if (!grip.current) return;
    grip.current = null;
    setActive(false);
    onCommit(latest.current);
  };

  const begin = (e: React.PointerEvent, which: Grip) => {
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    grip.current = which;
    origin.current = { ...atEvent(e), crop: latest.current };
    setActive(true);
  };

  const pct = (v: number) => `${v * 100}%`;
  /** A corner's 24px hit area sits in the same corner as its bracket. */
  const pin = (s: React.CSSProperties): React.CSSProperties => ({
    left: s.left,
    right: s.right,
    top: s.top,
    bottom: s.bottom,
  });

  return (
    <div
      ref={outer}
      className="absolute inset-0 touch-none"
      onPointerMove={onMove}
      onPointerUp={end}
      onPointerCancel={end}
    >
    <div
      ref={box}
      className="absolute"
      style={{ left: fit.left, top: fit.top, width: fit.width, height: fit.height }}
    >
      {/* Four panels rather than one box-shadow: they darken exactly the
          discarded area, so what survives the crop reads at its true tone. */}
      {[
        { left: 0, top: 0, width: "100%", height: pct(crop.y) },
        { left: 0, top: pct(crop.y + crop.height), width: "100%", bottom: 0 },
        { left: 0, top: pct(crop.y), width: pct(crop.x), height: pct(crop.height) },
        {
          left: pct(crop.x + crop.width),
          top: pct(crop.y),
          right: 0,
          height: pct(crop.height),
        },
      ].map((s, i) => (
        <div key={i} className="absolute bg-black/55" style={s} />
      ))}

      <div
        className="absolute cursor-move border border-white/80"
        style={{
          left: pct(crop.x),
          top: pct(crop.y),
          width: pct(crop.width),
          height: pct(crop.height),
        }}
        onPointerDown={(e) => begin(e, "move")}
      >
        {/* The framing guide — thirds unless another is chosen with G — and
            only while dragging, so it does not compete with the photo the rest
            of the time. */}
        {active && <CompositionGrid kind={grid} />}
        {levelling && (
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0"
            style={{
              backgroundImage:
                "linear-gradient(to right, rgba(255,255,255,0.28) 1px, transparent 1px), linear-gradient(to bottom, rgba(255,255,255,0.28) 1px, transparent 1px)",
              backgroundSize: `${100 / 12}% ${100 / 12}%`,
            }}
          />
        )}
        {EDGES.map(({ grip: g, style, bar, cursor }) => (
          <div key={g} className="absolute" style={{ ...style, cursor }} onPointerDown={(e) => begin(e, g)}>
            <div className="absolute rounded-full bg-white" style={bar} />
          </div>
        ))}
        {CORNERS.map(({ grip: g, style, cursor }) => (
          <div
            key={g}
            onPointerDown={(e) => begin(e, g)}
            className="absolute"
            style={{ ...pin(style), width: corner, height: corner, cursor }}
          >
            <div className="absolute h-4 w-4 border-0 border-white" style={style} />
          </div>
        ))}
      </div>
    </div>
    </div>
  );
}
