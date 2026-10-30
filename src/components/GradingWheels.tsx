import { useRef, useState } from "react";
import { sentenceCase } from "../lib/format";
import NumberSlider from "./NumberSlider";
import { t, type Key } from "../i18n";
import { GRADE_RANGES, type Grade, type GradeRangeName, type Grading } from "../types";

/**
 * A smooth hue ring as a single conic gradient.
 *
 * The gradient runs clockwise from 12 o'clock, but the knob maps hue
 * anticlockwise from 3 o'clock (0° = red at the east, the way a colour wheel is
 * read), so the colour at screen angle θ is hue `90 − θ`. Building the stops that
 * way lines the paint up with the knob without mirroring anything.
 */
const HUE_CONIC = `conic-gradient(from 0deg, ${Array.from({ length: 25 }, (_, i) => {
  const theta = i * 15;
  const h = (((90 - theta) % 360) + 360) % 360;
  return `hsl(${h} 62% 52%) ${theta}deg`;
}).join(", ")})`;

// Keys, not words: a table at module scope is built once, before the language
// is known (see `i18n`).
const RANGE_KEYS: Record<GradeRangeName, Key> = {
  shadows: "tools.grading.range.shadows",
  midtones: "tools.grading.range.midtones",
  highlights: "tools.grading.range.highlights",
};

/** The undo-history label for a change to one of a range's controls. */
const history = (what: "grade" | "luminance" | "saturation" | "hue" | "reset", range: GradeRangeName) =>
  t(`tools.grading.history.${what}`, { range: t(RANGE_KEYS[range]) });

/** A wheel's name: "Shadows colour". */
const wheelLabel = (range: GradeRangeName) => sentenceCase(t("tools.grading.wheelLabel", { range: t(RANGE_KEYS[range]) }));

/**
 * One wheel: angle is hue, distance from centre is saturation. The polar mapping
 * puts "which colour" and "how much" under a single gesture, and keeps
 * neighbouring hues neighbours (a 0–360 slider breaks at the seam).
 */
export function Wheel({
  grade,
  size,
  label,
  onChange,
  onCommit,
  onReset,
}: {
  grade: Grade;
  size: number;
  /** What the wheel colours, for assistive tech: "Shadows colour". */
  label?: string;
  onChange: (g: Grade) => void;
  onCommit: (g: Grade) => void;
  /** Back to no colour, its brightness kept — a double-click, as every
   *  slider does. */
  onReset?: (g: Grade) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  // Refs, not state: pointerdown and pointerup can land in the same frame on a
  // quick tap, and state would not have updated in between — the trap the curve
  // editor hit, where the commit was skipped and the edit was lost.
  const dragging = useRef(false);
  const latest = useRef(grade);
  if (!dragging.current) latest.current = grade;

  const [live, setLive] = useState(false);
  // An arrow-key nudge commits on key-up: one step for a held key.
  const nudged = useRef(false);

  const fromEvent = (e: React.PointerEvent): Grade => {
    const r = ref.current!.getBoundingClientRect();
    const x = e.clientX - (r.left + r.width / 2);
    // Screen y grows downward; negating it makes the angle run anticlockwise
    // from the right, the direction a colour wheel is read.
    const y = e.clientY - (r.top + r.height / 2);
    const hue = (Math.atan2(-y, x) * 180) / Math.PI;
    return {
      ...latest.current,
      hue: (hue + 360) % 360,
      saturation: Math.min(1, Math.hypot(x, y) / (r.width / 2)),
    };
  };

  const emit = (g: Grade) => {
    latest.current = g;
    onChange(g);
  };

  const angle = (grade.hue * Math.PI) / 180;
  const kx = 50 + Math.cos(angle) * grade.saturation * 50;
  const ky = 50 - Math.sin(angle) * grade.saturation * 50;
  const knob = live ? 14 : 12;

  const sat = Math.round(grade.saturation * 100) / 100;
  return (
    <div
      ref={ref}
      // A slider in two dimensions: ←/→ turn the hue, ↑/↓ add or take away
      // colour, Shift for bigger steps. It was the one control in the rail
      // the keyboard could not reach.
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={sat}
      aria-valuetext={t("tools.grading.wheelValue", { hue: Math.round(grade.hue), saturation: sat })}
      title={t("tools.grading.wheelHint")}
      style={{ width: size, height: size, background: HUE_CONIC }}
      className="relative shrink-0 cursor-crosshair touch-none rounded-full outline-offset-2 focus-visible:outline-2 focus-visible:outline-focus"
      onKeyDown={(e) => {
        const big = e.shiftKey ? 10 : 1;
        const g = latest.current;
        let next: Grade | null = null;
        if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
          // As the Hue slider in Focus moves: → is up the hue circle.
          const hue = (((g.hue + (e.key === "ArrowRight" ? 1 : -1) * big) % 360) + 360) % 360;
          next = { ...g, hue };
        } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          const s = Math.min(1, Math.max(0, g.saturation + (e.key === "ArrowUp" ? 0.01 : -0.01) * big));
          next = { ...g, saturation: Math.round(s * 100) / 100 };
        }
        if (!next) return;
        // Not Develop's: the arrows there change the photo.
        e.preventDefault();
        e.stopPropagation();
        nudged.current = true;
        emit(next);
      }}
      onKeyUp={() => {
        if (!nudged.current) return;
        nudged.current = false;
        onCommit(latest.current);
      }}
      onDoubleClick={() => {
        if (!onReset) return;
        const g = { ...latest.current, hue: 0, saturation: 0 };
        latest.current = g;
        onReset(g);
      }}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        dragging.current = true;
        setLive(true);
        emit(fromEvent(e));
      }}
      onPointerMove={(e) => dragging.current && emit(fromEvent(e))}
      onPointerUp={() => {
        if (!dragging.current) return;
        dragging.current = false;
        setLive(false);
        onCommit(latest.current);
      }}
      onPointerCancel={() => {
        dragging.current = false;
        setLive(false);
      }}
    >
      {/* A small soft neutral eye at dead centre (zero saturation) — just enough
          to cover where every hue converges, not a grey disc over the wheel. */}
      <div
        className="pointer-events-none absolute inset-0 rounded-full"
        style={{
          background: "radial-gradient(circle at 50% 50%, var(--color-panel) 0%, transparent 24%)",
        }}
      />
      <div className="pointer-events-none absolute inset-0 rounded-full ring-1 ring-inset ring-line" />
      <div
        className="pointer-events-none absolute rounded-full border-2 border-black bg-white"
        style={{ left: `${kx}%`, top: `${ky}%`, width: knob, height: knob, transform: "translate(-50%, -50%)" }}
      />
    </div>
  );
}

interface Props {
  grading: Grading;
  onChange: (g: Grading) => void;
  onCommit: (g: Grading, label: string) => void;
}

/**
 * Two views, as colour-grading panels usually offer: **3-way** stacks all three
 * wheels — each large, with its luminance beside it — so a look can be roughed
 * in with every range visible; **Focus** gives one range the full width plus
 * type-in-precise hue/saturation/luminance. Clicking a range's label in 3-way
 * jumps to Focus on it, so the two are one workflow: rough all three, refine one.
 *
 * A range that has been moved is marked with a steel dot in both views, so a
 * grade is never silently hidden by the view you happen to be in.
 */
export default function GradingWheels({ grading, onChange, onCommit }: Props) {
  const [view, setView] = useState<"three" | "focus">("three");
  const [range, setRange] = useState<GradeRangeName>("midtones");

  const withRange = (name: GradeRangeName, grade: Grade): Grading => ({
    ...grading,
    [name]: grade,
  });

  const active = grading[range];
  const touched = (g: Grade) => g.saturation > 0.001 || Math.abs(g.luminance) > 0.001;

  return (
    <div>
      {/* View switch — the same segmented pattern as the canvas Fit/1:1 control. */}
      <div className="mb-3 seg-group">
        {(
          [
            ["three", "tools.grading.threeWay"],
            ["focus", "tools.grading.focus"],
          ] as const
        ).map(([id, label]) => (
          <button aria-pressed={view === id}
            key={id}
            onClick={() => setView(id)}
            className={`flex-1 py-1 text-caption ${
              view === id ? "seg-on" : "text-muted hover:bg-hover"
            }`}
          >
            {t(label)}
          </button>
        ))}
      </div>

      {view === "three" ? (
        <div className="flex flex-col gap-3">
          {GRADE_RANGES.map((name) => (
            <div key={name} className="flex items-center gap-3">
              <Wheel
                size={140}
                grade={grading[name]}
                label={wheelLabel(name)}
                onChange={(g) => onChange(withRange(name, g))}
                onCommit={(g) => onCommit(withRange(name, g), history("grade", name))}
                onReset={(g) => onCommit(withRange(name, g), history("reset", name))}
              />
              <div className="min-w-0 flex-1">
                <button
                  onClick={() => {
                    setRange(name);
                    setView("focus");
                  }}
                  title={t("tools.grading.refine", { range: t(RANGE_KEYS[name]) })}
                  className="mb-1.5 flex items-center gap-1 text-caption font-semibold
                             text-muted hover:text-ink"
                >
                  {sentenceCase(t(RANGE_KEYS[name]))}
                  {touched(grading[name]) && <span className="h-1.5 w-1.5 rounded-full bg-accent" />}
                </button>
                <NumberSlider
                  layout="stacked"
                  label={t("tools.grading.luminance")}
                  value={grading[name].luminance}
                  min={-1}
                  max={1}
                  step={0.01}
                  onChange={(v) => onChange(withRange(name, { ...grading[name], luminance: v }))}
                  onCommit={() => onCommit(withRange(name, grading[name]), history("luminance", name))}
                />
              </div>
            </div>
          ))}
        </div>
      ) : (
        <>
          {/* The rail's one either/or pattern, with the dot it uses for a
              choice holding edits the switch is hiding. */}
          <div className="mb-3 seg-group">
            {GRADE_RANGES.map((name) => (
              <button aria-pressed={range === name}
                key={name}
                onClick={() => setRange(name)}
                className={`flex flex-1 items-center justify-center gap-1 py-1 text-caption ${
                  range === name ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
                }`}
              >
                {sentenceCase(t(RANGE_KEYS[name]))}
                {touched(grading[name]) && range !== name && <span className="h-1 w-1 rounded-full bg-accent" />}
              </button>
            ))}
          </div>

          <div className="mb-3 flex justify-center">
            <Wheel
              size={220}
              grade={active}
              label={wheelLabel(range)}
              onChange={(g) => onChange(withRange(range, g))}
              onCommit={(g) => onCommit(withRange(range, g), history("grade", range))}
              onReset={(g) => onCommit(withRange(range, g), history("reset", range))}
            />
          </div>

          {/* Brightness is kept off the wheel: mapping it to a third polar
              dimension would make the other two impossible to set precisely. */}
          <NumberSlider
            label={t("tools.grading.luminance")}
            value={active.luminance}
            min={-1}
            max={1}
            step={0.01}
            onChange={(v) => onChange(withRange(range, { ...active, luminance: v }))}
            onCommit={() => onCommit(withRange(range, active), history("luminance", range))}
          />
          <NumberSlider
            label={t("tools.grading.saturation")}
            value={active.saturation}
            min={0}
            max={1}
            step={0.01}
            onChange={(v) => onChange(withRange(range, { ...active, saturation: v }))}
            onCommit={() => onCommit(withRange(range, active), history("saturation", range))}
          />
          <NumberSlider
            label={t("tools.grading.hue")}
            value={active.hue}
            min={0}
            max={359}
            step={1}
            precision={0}
            suffix="°"
            onChange={(v) => onChange(withRange(range, { ...active, hue: v }))}
            onCommit={() => onCommit(withRange(range, active), history("hue", range))}
          />
        </>
      )}
    </div>
  );
}
