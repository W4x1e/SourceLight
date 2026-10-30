import { useRef, type CSSProperties } from "react";

/** A drag that comes this close to where the control rests, as a share of
 *  its range, lands on it: a finger can't find zero by itself. */
const SNAP = 0.015;
/** Two taps this close in time (ms) and place (px) are a double tap. */
const DOUBLE_TAP = { ms: 320, px: 28 };
/** Further than this (px) between press and release is a drag, not a tap. */
const TAP_SLOP = 10;

/**
 * A labelled slider sized for a thumb, drawn as the desktop's (DESIGN.md,
 * Sliders): a hairline track that fills from where the control rests to the
 * thumb, a tick at the rest of a two-way control, and the label and value in
 * white once it has moved. Dragging near the rest snaps onto it; a double
 * tap anywhere on the row puts it back, as one change (`now`).
 */
export default function Slider({
  label,
  value,
  min,
  max,
  step,
  format,
  onChange,
  resetTo = 0,
  gradient,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (v: number) => string;
  /** `now` when the change is done with at once (a reset), rather than
   *  part of a drag. */
  onChange: (v: number, now?: boolean) => void;
  resetTo?: number;
  /** A colour cue painted in the track, as the desktop's temperature, tint
   *  and saturation sliders have (lib/developCues). */
  gradient?: string;
}) {
  const span = max - min || 1;
  const at = (v: number) => Math.min(1, Math.max(0, (v - min) / span));
  const moved = Math.abs(value - resetTo) > 1e-6;
  const track = {
    "--a": Math.min(at(resetTo), at(value)),
    "--b": Math.max(at(resetTo), at(value)),
    "--z": at(resetTo),
    ...(gradient ? { "--track": gradient } : {}),
  } as CSSProperties;

  // Where the finger went down, and the last tap, for telling a double tap.
  const press = useRef<{ x: number; y: number } | null>(null);
  const lastTap = useRef<{ t: number; x: number; y: number } | null>(null);
  const onPointerDown = (e: React.PointerEvent) => {
    press.current = { x: e.clientX, y: e.clientY };
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const from = press.current;
    press.current = null;
    if (!from || Math.hypot(e.clientX - from.x, e.clientY - from.y) > TAP_SLOP) return;
    const now = performance.now();
    const last = lastTap.current;
    if (last && now - last.t < DOUBLE_TAP.ms && Math.hypot(e.clientX - last.x, e.clientY - last.y) < DOUBLE_TAP.px) {
      lastTap.current = null;
      onChange(resetTo, true);
    } else lastTap.current = { t: now, x: e.clientX, y: e.clientY };
  };

  return (
    <label
      className={`slider phone-slider block py-1 ${min < 0 && max > 0 ? "slider-bipolar" : ""} ${gradient ? "slider-graded" : ""}`}
      onPointerDown={onPointerDown}
      onPointerUp={onPointerUp}
      onPointerCancel={() => (press.current = null)}
    >
      <span className="flex items-baseline justify-between text-caption">
        <span className={moved ? "text-ink" : "text-muted"}>{label}</span>
        <span className={`font-mono tabular-nums ${moved ? "text-ink" : "text-faint"}`}>{format(value)}</span>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => {
          const v = Number(e.target.value);
          onChange(Math.abs(v - resetTo) <= span * SNAP ? resetTo : v);
        }}
        className="slider-track block w-full"
        style={track}
      />
    </label>
  );
}

/** -1..1 shown as -100..100, signed. */
export const percent = (v: number) => {
  const n = Math.round(v * 100);
  return n > 0 ? `+${n}` : String(n);
};

/** 0..1 shown as 0..100. */
export const hundred = (v: number) => String(Math.round(v * 100));
