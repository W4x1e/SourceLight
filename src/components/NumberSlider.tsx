import { useRef, useState, type CSSProperties } from "react";
import { getLocale, intlLocale, t } from "../i18n";

/** A Shift-drag moves the value this much slower than a plain one. */
const FINE = 0.1;

/** Which way each arrow key moves a slider. */
const ARROW_SIGN: Record<string, number> = { ArrowRight: 1, ArrowUp: 1, ArrowLeft: -1, ArrowDown: -1 };

/** The decimal mark of the language shown, worked out once per language: a
 *  slider renders on every frame of a drag, and an `Intl.NumberFormat` per
 *  readout per frame is not free. */
const marks = new Map<string, string>();
function decimalMark(): string {
  const locale = getLocale();
  let mark = marks.get(locale);
  if (mark === undefined) {
    mark = new Intl.NumberFormat(intlLocale(locale)).formatToParts(1.5).find((p) => p.type === "decimal")?.value ?? ".";
    marks.set(locale, mark);
  }
  return mark;
}

interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** Fractional digits, so a stop and a -1..1 ratio read sensibly. */
  precision?: number;
  suffix?: string;
  /**
   * Where the control rests, which is what double-click puts back and where the
   * fill is drawn from. Most rest at 0 — but a sharpening radius rests at 1 and
   * a vignette midpoint at 0.5, and resetting those to 0 was a different edit,
   * not a reset. Defaults to 0, or `min` for a range that does not include it.
   */
  defaultValue?: number;
  /**
   * A CSS gradient painted on the track, the way editors usually show temperature
   * (blue→yellow), tint (green→magenta) and the HSL bands. Purely a hint about
   * what the control does; when omitted the track is neutral and fills from
   * the resting value to the thumb.
   */
  gradient?: string;
  /**
   * `row` (the default): label, track and value on one line, so a panel of
   * sliders reads as a column of labels, a column of tracks and a column of
   * values — an instrument's readout. `stacked`: label and value over a
   * full-width track, for a slider that shares its width with something else
   * (a colour wheel) and would be left no track in a row.
   */
  layout?: "row" | "stacked";
  onChange: (v: number) => void;
  onCommit: () => void;
}

/**
 * A slider whose readout is an editable field.
 *
 * A slider alone cannot be set precisely: at rail width a -1..1 control has
 * about 90 pixels per unit, coarser than the 0.01 the value is displayed to.
 * Typing is the only way to say "exactly 0.35", or to copy a setting between
 * photos by reading one and entering it on another; the arrow keys nudge by a
 * step (Shift for ten) in the field, and by a step on the focused track.
 *
 * The field holds its own text while focused rather than reformatting on every
 * keystroke. Without that, typing "0.5" reformats to "0.50" after the "0.",
 * which puts the caret in the wrong place and makes the field impossible to
 * type into — the classic controlled-numeric-input trap.
 *
 * The track fills from where the control rests to where it is, so a panel
 * shows at a glance which controls have moved, which way and how far; a moved
 * control's value also lights up. Double-click the label or the track to put
 * it back.
 */
export default function NumberSlider({
  label,
  value,
  min,
  max,
  step,
  precision = 2,
  suffix = "",
  defaultValue,
  gradient,
  layout = "row",
  onChange,
  onCommit,
}: Props) {
  const [text, setText] = useState<string | null>(null);
  // Escape leaves the field by blurring it, and the blur would otherwise
  // commit the text it was asked to throw away.
  const cancelled = useRef(false);
  // An arrow-key nudge in the field is committed on key-up, as the track's
  // is: one step in History for a held key, not one per repeat.
  const nudged = useRef(false);
  // A Shift-drag on the track, which this handles itself rather than the
  // native slider: where it started, and from what value.
  const fine = useRef<{ x: number; from: number; width: number } | null>(null);

  const rest = defaultValue ?? Math.min(max, Math.max(min, 0));
  // Bipolar: rests somewhere inside the range, so it moves both ways and a
  // positive value is worth a sign.
  const bipolar = rest > min && rest < max;
  const moved = Math.abs(value - rest) > step / 1000;

  const format = (v: number) => {
    // No "-0.00": a value that rounds to zero reads as zero.
    const shown = Math.abs(v) < 0.5 * 10 ** -precision ? 0 : v;
    // In the language's own notation (0,35 in German), as every other number
    // in the interface is.
    const fixed = shown.toFixed(precision);
    const mark = decimalMark();
    return mark === "." ? fixed : fixed.replace(".", mark);
  };
  const clamp = (v: number) => Math.min(max, Math.max(min, v));
  /** On the slider's own steps, without the float dust of 0.1 + 0.2. */
  const snap = (v: number) => Number((Math.round((v - min) / step) * step + min).toFixed(10));
  const display = (v: number) => (bipolar && rest === 0 && format(v) !== format(0) && v > 0 ? "+" : "") + format(v);

  const commitText = (raw: string) => {
    setText(null);
    // Either decimal mark: a German keyboard types 0,5 and someone used to
    // another app types 0.5, and both mean a half. Unicode's minus too.
    const cleaned = raw.replace(suffix.trim(), "").trim().replace(",", ".").replace("\u2212", "-");
    const parsed = Number(cleaned);
    if (cleaned === "" || !Number.isFinite(parsed)) return;
    // Clamped rather than rejected: someone typing 500 into a -1..1 control
    // means "as far as it goes", and refusing the entry silently is worse.
    const next = clamp(parsed);
    // Clicking into the field and out again is not an edit, and must not
    // leave a step in the history saying it was.
    if (format(next) === format(value)) return;
    onChange(next);
    onCommit();
  };

  const reset = () => {
    onChange(rest);
    onCommit();
  };

  const span = max - min || 1;
  const at = (v: number) => Math.min(1, Math.max(0, (v - min) / span));
  const track = {
    "--a": Math.min(at(rest), at(value)),
    "--b": Math.max(at(rest), at(value)),
    "--z": at(rest),
    ...(gradient ? { "--track": gradient } : {}),
  } as CSSProperties;

  return (
    <div
      className={`slider slider-${layout} ${moved ? "slider-moved" : ""} ${bipolar ? "slider-bipolar" : ""} ${
        gradient ? "slider-graded" : ""
      }`}
    >
      <span
        className="slider-label"
        onDoubleClick={reset}
        title={`${t("tools.slider.resetHint", { label })} · ${t("tools.slider.fineHint")}`}
      >
        {label}
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        aria-label={label}
        onChange={(e) => onChange(Number(e.target.value))}
        // Shift-drag is the fine adjustment: the value moves a tenth as far as
        // the pointer, from where it was rather than jumping to the pointer,
        // so a control 90 px per unit can be set to the step it shows. The
        // native slider would take the press as a jump, so it is not given it.
        onPointerDown={(e) => {
          if (!e.shiftKey || e.button !== 0) return;
          e.preventDefault();
          e.currentTarget.setPointerCapture(e.pointerId);
          fine.current = { x: e.clientX, from: value, width: e.currentTarget.getBoundingClientRect().width };
        }}
        onMouseDown={(e) => {
          if (fine.current) e.preventDefault();
        }}
        onPointerMove={(e) => {
          const f = fine.current;
          if (!f) return;
          onChange(clamp(snap(f.from + ((e.clientX - f.x) / Math.max(1, f.width)) * span * FINE)));
        }}
        // Commit on release, not per input event: dragging fires continuously
        // and every distinct value would otherwise become a history entry.
        // …and gives focus back after a drag: a slider that kept it
        // swallowed Develop's keys, and → then nudged the slider instead of
        // moving to the next photo. Reached with Tab, it keeps it and its
        // arrows, as a slider should.
        onPointerUp={(e) => {
          fine.current = null;
          onCommit();
          e.currentTarget.blur();
        }}
        onPointerCancel={() => {
          if (!fine.current) return;
          fine.current = null;
          onCommit();
        }}
        // Shift+arrow takes ten steps, as it does in the value field; the
        // native slider takes one whatever the modifier.
        onKeyDown={(e) => {
          if (!e.shiftKey || !(e.key in ARROW_SIGN)) return;
          e.preventDefault();
          onChange(clamp(snap(value + ARROW_SIGN[e.key]! * step * 10)));
        }}
        onKeyUp={onCommit}
        onDoubleClick={reset}
        className="slider-track"
        style={track}
      />
      <span className="slider-value">
        <input
          type="text"
          inputMode="decimal"
          aria-label={t("tools.slider.valueLabel", { label })}
          value={text ?? display(value)}
          onChange={(e) => setText(e.target.value)}
          onFocus={(e) => {
            setText(format(value));
            e.target.select();
          }}
          onBlur={(e) => {
            if (cancelled.current) {
              cancelled.current = false;
              setText(null);
              return;
            }
            commitText(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.currentTarget.blur();
            } else if (e.key === "Escape") {
              // Puts the value back and leaves, without committing what was
              // typed — and without Develop taking the Escape to leave a tool.
              e.preventDefault();
              e.stopPropagation();
              cancelled.current = true;
              e.currentTarget.blur();
            }
            // Arrow keys nudge by one step, which is the other half of being
            // able to set a value exactly.
            else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
              e.preventDefault();
              const delta = (e.key === "ArrowUp" ? 1 : -1) * step * (e.shiftKey ? 10 : 1);
              const next = clamp(snap(value + delta));
              setText(format(next));
              onChange(next);
              nudged.current = true;
            }
          }}
          onKeyUp={(e) => {
            if (!nudged.current || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
            nudged.current = false;
            onCommit();
          }}
        />
        {suffix && <span className="slider-suffix">{suffix.trim()}</span>}
      </span>
    </div>
  );
}
