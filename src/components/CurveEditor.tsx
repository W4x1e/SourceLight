import { useId, useRef, useState } from "react";
import type { CurvePt } from "../types";
import { evaluate, prepare } from "../lib/curve";
import { formatNumber, t } from "../i18n";

/** The shape an untouched curve starts from once the user first drags it. */
const DIAGONAL: CurvePt[] = [
  { x: 0, y: 0 },
  { x: 1, y: 1 },
];

const SIZE = 100;
/** Below this, a click counts as grabbing an existing point rather than adding one. */
const GRAB = 7;
/** Two presses on one point this close together (ms) remove it. */
const DOUBLE_PRESS_MS = 320;
/** An arrow key's step, and Shift+arrow's. */
const STEP = 0.01;
const BIG_STEP = 0.05;

interface Props {
  points: CurvePt[];
  onChange: (points: CurvePt[]) => void;
  /**
   * Takes the points explicitly rather than reading them back from state.
   * Removing a point changes and commits in the same handler, and React has
   * not flushed the change by then — reading state would save the curve as it
   * was *before* the removal.
   */
  onCommit: (points: CurvePt[]) => void;
  /**
   * Stroke/point colour. Omitted for the master RGB curve, which uses the
   * accent; the per-channel curves pass their own red/green/blue so the editor
   * reads as the channel it edits.
   */
  color?: string;
  /** How near, in hundredths of the editor's width, a press must be to take
   *  hold of a point rather than add one: more for a finger. */
  grab?: number;
  /** The line under the editor, for a touch screen's words. */
  hint?: string;
}

/**
 * Draggable tone-curve panel.
 *
 * Coordinates are 0..1 with y up, matching the edit stack. SVG has y running
 * down, so every read and write flips it — kept in `toSvg`/`fromSvg` rather
 * than sprinkled through the handlers, because a missed flip produces a curve
 * that works but is upside down.
 */
export default function CurveEditor({ points, onChange, onCommit, color, grab = GRAB, hint }: Props) {
  const svg = useRef<SVGSVGElement>(null);

  /**
   * Which point is being dragged, in a ref rather than state.
   *
   * State does not update until React re-renders, and a fast enough click
   * delivers pointerdown and pointerup within one frame — so the pointerup
   * handler would still see `null`, skip the commit, and the new point would
   * live in the draft without ever reaching the backend. Invisible in ordinary
   * dragging, and a silently lost edit on a quick tap.
   *
   * The state copy below is kept only to redraw the grabbed point larger.
   */
  const dragIndex = useRef<number | null>(null);
  /** The last press on a point and when, for a double press removing it. */
  const lastPress = useRef<{ index: number; time: number } | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);

  const beginDrag = (i: number) => {
    dragIndex.current = i;
    setDragging(i);
  };

  // An empty stack means the identity, but the panel still has to show
  // something draggable, so the diagonal stands in until first touched.
  const pts = points.length >= 2 ? points : DIAGONAL;

  /**
   * The most recent points this component emitted.
   *
   * Handlers cannot read `pts` for the same reason `dragIndex` is a ref: within
   * one frame there is no re-render, so the prop still holds the pre-click
   * value and a quick tap would commit the curve as it was *before* the point
   * was added. Between interactions the props are authoritative, so this is
   * resynced on every render.
   */
  const latest = useRef<CurvePt[]>(pts);
  if (dragIndex.current === null) latest.current = pts;

  /** Emits a change and remembers it, so the next handler sees it. */
  const emit = (next: CurvePt[]) => {
    latest.current = next;
    onChange(next);
  };

  const toSvg = (p: CurvePt) => ({ x: p.x * SIZE, y: (1 - p.y) * SIZE });

  const fromEvent = (e: React.PointerEvent): CurvePt => {
    const rect = svg.current!.getBoundingClientRect();
    return {
      x: Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)),
      y: Math.min(1, Math.max(0, 1 - (e.clientY - rect.top) / rect.height)),
    };
  };

  const path = () => {
    const steps = 64;
    const spline = prepare(pts);
    return Array.from({ length: steps + 1 }, (_, i) => {
      const x = i / steps;
      const p = toSvg({ x, y: evaluate(spline, x) });
      return `${i === 0 ? "M" : "L"}${p.x.toFixed(2)},${p.y.toFixed(2)}`;
    }).join(" ");
  };

  const onPointerDown = (e: React.PointerEvent) => {
    const at = fromEvent(e);
    const svgAt = toSvg(at);

    // Grab the nearest point if the click was close to one, in screen terms
    // rather than curve terms — otherwise points in a steep region become
    // almost impossible to hit.
    let nearest = -1;
    let best = grab;
    pts.forEach((p, i) => {
      const s = toSvg(p);
      const dist = Math.hypot(s.x - svgAt.x, s.y - svgAt.y);
      if (dist < best) {
        best = dist;
        nearest = i;
      }
    });

    e.currentTarget.setPointerCapture(e.pointerId);

    if (nearest >= 0) {
      // A second press on the point removes it. Told here rather than by a
      // double-click on the point: once the drawing holds the pointer, the
      // double-click lands on the drawing, and a finger's never reached the
      // point at all.
      const now = performance.now();
      const last = lastPress.current;
      lastPress.current = { index: nearest, time: now };
      if (last && last.index === nearest && now - last.time < DOUBLE_PRESS_MS && remove(nearest)) {
        lastPress.current = null;
        return;
      }
      // Materialise the stand-in diagonal so dragging an endpoint edits a real
      // stack rather than a value that vanishes on the next render.
      if (points.length < 2) emit([...DIAGONAL]);
      beginDrag(nearest);
      return;
    }

    const next = [...pts, at].sort((a, b) => a.x - b.x);
    emit(next);
    beginDrag(next.indexOf(at));
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const held = dragIndex.current;
    if (held === null) return;
    const at = fromEvent(e);
    const base = latest.current;
    const next = base.map((p, i) => (i === held ? at : p));

    // Keep x strictly increasing: dragging one point past its neighbour would
    // otherwise reorder the array under the index being dragged, and the point
    // would jump out from under the cursor.
    const before = next[held - 1];
    const after = next[held + 1];
    const lo = before ? before.x + 0.01 : 0;
    const hi = after ? after.x - 0.01 : 1;
    next[held] = { x: Math.min(hi, Math.max(lo, at.x)), y: at.y };
    emit(next);
  };

  const endDrag = () => {
    if (dragIndex.current === null) return;
    dragIndex.current = null;
    setDragging(null);
    onCommit(latest.current);
  };

  /** Removes a point. The two endpoints stay — without them there is no curve. */
  const remove = (i: number) => {
    const current = latest.current;
    if (current.length <= 2 || i === 0 || i === current.length - 1) return false;
    const next = current.filter((_, j) => j !== i);
    emit(next);
    onCommit(next);
    return true;
  };

  // ---- Keyboard: the curve was pointer-only. ----
  // Each point is a slider: the arrows move it, Delete removes it, Insert or
  // + adds one halfway to the next. A held arrow is one edit, committed when
  // the key comes up, as a dragged point is on release.
  const hintId = useId();
  const pointRefs = useRef<(SVGCircleElement | null)[]>([]);
  const [focused, setFocused] = useState<number | null>(null);
  const keyMoved = useRef(false);
  const focusPoint = (i: number) => requestAnimationFrame(() => pointRefs.current[i]?.focus());

  const onPointKey = (e: React.KeyboardEvent, i: number) => {
    const base = latest.current;
    const step = e.shiftKey ? BIG_STEP : STEP;
    const move = (dx: number, dy: number) => {
      const p = base[i]!;
      const lo = i > 0 ? base[i - 1]!.x + 0.01 : 0;
      const hi = i < base.length - 1 ? base[i + 1]!.x - 0.01 : 1;
      const next = base.map((q, j) =>
        j === i ? { x: Math.min(hi, Math.max(lo, p.x + dx)), y: Math.min(1, Math.max(0, p.y + dy)) } : q,
      );
      keyMoved.current = true;
      emit(next);
    };
    switch (e.key) {
      case "ArrowUp": move(0, step); break;
      case "ArrowDown": move(0, -step); break;
      case "ArrowLeft": move(-step, 0); break;
      case "ArrowRight": move(step, 0); break;
      case "Delete":
      case "Backspace":
        if (remove(i)) focusPoint(i - 1);
        break;
      case "Insert":
      case "+":
      case "=": {
        const right = base[i + 1] ?? base[i - 1];
        if (!right) return;
        const x = (base[i]!.x + right.x) / 2;
        if (Math.abs(right.x - base[i]!.x) < 0.03) return;
        const at = { x, y: evaluate(prepare(base), x) };
        const next = [...base, at].sort((a, b) => a.x - b.x);
        emit(next);
        onCommit(next);
        focusPoint(next.indexOf(at));
        break;
      }
      default:
        return;
    }
    // The canvas below has its own arrow keys; these are the curve's.
    e.preventDefault();
    e.stopPropagation();
  };
  const onPointKeyUp = () => {
    if (!keyMoved.current) return;
    keyMoved.current = false;
    onCommit(latest.current);
  };
  const pct = (v: number) => formatNumber(Math.round(v * 100) / 100, { style: "percent" });

  return (
    <div>
      <svg
        ref={svg}
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        className="mx-auto block aspect-square w-full max-w-[300px] cursor-crosshair touch-none rounded-sm border border-line/60 bg-black/30"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        {[25, 50, 75].map((v) => (
          <g key={v} className="stroke-line/40" strokeWidth={0.4}>
            <line x1={v} y1={0} x2={v} y2={SIZE} />
            <line x1={0} y1={v} x2={SIZE} y2={v} />
          </g>
        ))}
        {/* The identity, so any deviation from it is visible at a glance. */}
        <line x1={0} y1={SIZE} x2={SIZE} y2={0} className="stroke-line/50" strokeWidth={0.5} strokeDasharray="2 2" />

        <path
          d={path()}
          fill="none"
          className={color ? undefined : "stroke-accent"}
          stroke={color}
          strokeWidth={1.5}
        />

        {pts.map((p, i) => {
          const s = toSvg(p);
          return (
            <g key={i}>
              {/* The focus ring: an outline doesn't draw on an SVG shape everywhere. */}
              {focused === i && (
                <circle cx={s.x} cy={s.y} r={4.6} fill="none" className="stroke-ink" strokeWidth={0.8} />
              )}
              <circle
                ref={(el) => {
                  pointRefs.current[i] = el;
                }}
                cx={s.x}
                cy={s.y}
                r={dragging === i ? 3.2 : 2.4}
                className={`outline-none ${color ? "stroke-black" : "fill-accent stroke-black"}`}
                fill={color}
                strokeWidth={0.6}
                tabIndex={0}
                role="slider"
                aria-label={t("tools.curve.point", { n: i + 1 })}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(p.y * 100)}
                aria-valuetext={t("tools.curve.pointValue", { input: pct(p.x), output: pct(p.y) })}
                aria-describedby={hintId}
                onFocus={() => setFocused(i)}
                onBlur={() => {
                  setFocused(null);
                  onPointKeyUp();
                }}
                onKeyDown={(e) => onPointKey(e, i)}
                onKeyUp={onPointKeyUp}
              />
            </g>
          );
        })}
      </svg>
      <p className="mt-1 text-label text-faint">{hint ?? t("tools.curve.hint")}</p>
      <p id={hintId} className="sr-only">
        {t("tools.curve.keysHint")}
      </p>
    </div>
  );
}
