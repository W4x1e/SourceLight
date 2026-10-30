import { useEffect, useMemo, useRef, useState } from "react";
import { t, type Key } from "../../src/i18n";
import { Icon, type IconName } from "../../src/lib/icons";
import CurveEditor from "../../src/components/CurveEditor";
import { Wheel } from "../../src/components/GradingWheels";
import {
  lookFamilyName,
  lookTitle,
  presetGroupDescription,
  presetGroupName,
  presetName,
} from "../../src/lib/content";
import { SAT_GRADIENT, TEMP_GRADIENT, TINT_GRADIENT } from "../../src/lib/developCues";
import { sentenceCase } from "../../src/lib/format";
import {
  ASPECT_PRESETS,
  BAND_NAMES,
  DEFAULT_STACK,
  GRADE_RANGES,
  type Band8,
  type CurvePt,
  type GradeRangeName,
  type Stack,
  withStackDefaults,
} from "../../src/types";
import { api, editFrameUrl, type LookInfo, type PresetGroupInfo } from "./api";
import Slider, { hundred, percent } from "./Slider";

/**
 * The phone editor's tools (`Editor`), each a panel of the desktop's own
 * controls with its labels and ranges, sized for a finger. Each tool knows
 * the part of the edit it owns at rest (`restOf`), which gives it its dot in
 * the tool strip and its Reset.
 */

export const TOOLS: readonly { id: Tool; icon: IconName; label: Key }[] = [
  { id: "light", icon: "sun", label: "mobile.edit.tool.light" },
  { id: "colour", icon: "drop", label: "mobile.edit.tool.colour" },
  { id: "curve", icon: "curve", label: "mobile.edit.tool.curve" },
  { id: "mixer", icon: "mixer", label: "mobile.edit.tool.mixer" },
  { id: "grading", icon: "wheel", label: "mobile.edit.tool.grading" },
  { id: "detail", icon: "detail", label: "mobile.edit.tool.detail" },
  { id: "effects", icon: "sparkle", label: "mobile.edit.tool.effects" },
  { id: "optics", icon: "lens", label: "mobile.edit.tool.optics" },
  { id: "crop", icon: "crop", label: "mobile.edit.tool.crop" },
  { id: "presets", icon: "layers", label: "mobile.edit.tool.presets" },
  { id: "luts", icon: "cube", label: "mobile.edit.tool.luts" },
];
export type Tool =
  | "light"
  | "colour"
  | "curve"
  | "mixer"
  | "grading"
  | "detail"
  | "effects"
  | "optics"
  | "crop"
  | "presets"
  | "luts";

const D = DEFAULT_STACK;

/** The part of `stack` that `tool` owns, as it is at rest. */
export function restOf(tool: Tool, stack: Stack): Partial<Stack> {
  switch (tool) {
    case "light":
      return { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 };
    case "colour":
      return { warmth: 0, tint: 0, vibrance: 0, saturation: 0 };
    case "curve":
      return { curve: [], curveRed: [], curveGreen: [], curveBlue: [] };
    case "mixer":
      return { mixerHue: D.mixerHue, mixerSaturation: D.mixerSaturation, mixerLuminance: D.mixerLuminance };
    case "grading":
      return { grading: D.grading };
    case "detail": {
      const { sharpen, sharpenRadius, sharpenDetail, sharpenMasking, noise, colorNoise } = D.detail;
      return { detail: { ...stack.detail, sharpen, sharpenRadius, sharpenDetail, sharpenMasking, noise, colorNoise } };
    }
    case "effects":
      return { detail: { ...stack.detail, texture: 0, clarity: 0, dehaze: 0 }, effects: D.effects };
    case "optics":
      return { lens: D.lens };
    case "crop":
      return { geometry: D.geometry };
    case "presets": {
      // Taking a preset off puts every setting it owns back to neutral, and
      // takes its share off each tone slider still where it left it, as the
      // core's `presets::remove` does.
      const back = (value: number, i: number) => {
        const share = stack.preset.tone[i];
        return share?.left != null && Math.abs(value - share.left) < 1e-4 ? value - (share.added ?? 0) : value;
      };
      return {
        highlights: back(stack.highlights, 0),
        shadows: back(stack.shadows, 1),
        whites: back(stack.whites, 2),
        blacks: back(stack.blacks, 3),
        contrast: 0,
        vibrance: 0,
        saturation: 0,
        curve: [],
        curveRed: [],
        curveGreen: [],
        curveBlue: [],
        mixerHue: D.mixerHue,
        mixerSaturation: D.mixerSaturation,
        mixerLuminance: D.mixerLuminance,
        grading: D.grading,
        detail: { ...stack.detail, clarity: 0, texture: 0 },
        effects: D.effects,
        preset: D.preset,
      };
    }
    case "luts":
      return { lut: { name: "", intensity: stack.lut.intensity } };
  }
}

/** Whether two parts of an edit are the same, whatever order their fields
 *  came in (the computer's edits arrive in Rust's order) and allowing for
 *  float dust. */
export function same(a: unknown, b: unknown): boolean {
  if (typeof a === "number" && typeof b === "number") return Math.abs(a - b) < 1e-6;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => same(x, b[i]));
  if (a && b && typeof a === "object" && typeof b === "object") {
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    return (
      ka.length === kb.length &&
      ka.every((k) => same((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]))
    );
  }
  return a === b;
}

/** A curve of only its two ends, straight from corner to corner, changes
 *  nothing: what's left once every point added has been taken off. */
export const straight = (points: CurvePt[]) =>
  points.length === 0 ||
  (points.length === 2 && same(points[0], { x: 0, y: 0 }) && same(points[1], { x: 1, y: 1 }));

/** Whether `tool`'s part of the edit has moved from rest. */
export function isMoved(tool: Tool, stack: Stack): boolean {
  // A preset's settings are the other tools' too: its dot is for one being on.
  if (tool === "presets") return stack.preset.id !== "";
  if (tool === "curve") return !(["curve", "curveRed", "curveGreen", "curveBlue"] as const).every((c) => straight(stack[c]));
  const rest = restOf(tool, stack);
  return (Object.keys(rest) as (keyof Stack)[]).some((k) => !same(stack[k], rest[k]));
}

/** What every panel is given: the edit, and a way to change part of it,
 *  `now` when the change is done with (not part of a drag). */
export type PanelProps = {
  stack: Stack;
  set: (patch: Partial<Stack>, now?: boolean) => void;
};

/** A small heading inside a panel. */
function Heading({ children }: { children: React.ReactNode }) {
  return <div className="mt-3 text-label text-muted first:mt-1">{children}</div>;
}

/** A row of either/or choices: the desktop's segmented switch, with a dot
 *  on a choice holding edits the switch is hiding. */
function Segments<T extends string>({
  options,
  value,
  onPick,
  marked = () => false,
}: {
  options: readonly (readonly [T, string])[];
  value: T;
  onPick: (v: T) => void;
  marked?: (v: T) => boolean;
}) {
  return (
    <div className="seg-group my-1">
      {options.map(([id, label]) => (
        <button
          key={id}
          aria-pressed={value === id}
          className={`relative min-h-10 flex-1 text-caption ${value === id ? "seg-on text-ink" : "text-muted"}`}
          onClick={() => onPick(id)}
        >
          {label}
          {marked(id) && value !== id && <span aria-hidden className="ml-1 text-accent">•</span>}
        </button>
      ))}
    </div>
  );
}

const signedEv = (v: number) => `${v > 0 ? "+" : ""}${v.toFixed(2)} EV`;

// ------------------------------------------------------------------ Light

export function LightPanel({ stack, set }: PanelProps) {
  return (
    <>
      <Slider label={t("develop.light.exposure")} value={stack.exposure} min={-5} max={5} step={0.05} format={signedEv} onChange={(v, now) => set({ exposure: v }, now)} />
      <Slider label={t("develop.light.contrast")} value={stack.contrast} min={-1} max={1} step={0.01} format={percent} onChange={(v, now) => set({ contrast: v }, now)} />
      {(["highlights", "shadows", "whites", "blacks"] as const).map((k) => (
        <Slider key={k} label={t(`develop.light.${k}`)} value={stack[k]} min={-1} max={1} step={0.01} format={percent} onChange={(v, now) => set({ [k]: v }, now)} />
      ))}
    </>
  );
}

// ----------------------------------------------------------------- Colour

export function ColourPanel({ stack, set }: PanelProps) {
  return (
    <>
      <Slider label={t("develop.wb.temperature")} value={stack.warmth} min={-1} max={1} step={0.01} format={percent} gradient={TEMP_GRADIENT} onChange={(v, now) => set({ warmth: v }, now)} />
      <Slider label={t("develop.wb.tint")} value={stack.tint} min={-1} max={1} step={0.01} format={percent} gradient={TINT_GRADIENT} onChange={(v, now) => set({ tint: v }, now)} />
      <Slider label={t("develop.presence.vibrance")} value={stack.vibrance} min={-1} max={1} step={0.01} format={percent} gradient={SAT_GRADIENT} onChange={(v, now) => set({ vibrance: v }, now)} />
      <Slider label={t("develop.presence.saturation")} value={stack.saturation} min={-1} max={1} step={0.01} format={percent} gradient={SAT_GRADIENT} onChange={(v, now) => set({ saturation: v }, now)} />
    </>
  );
}

// ------------------------------------------------------------------ Curve

type Channel = "curve" | "curveRed" | "curveGreen" | "curveBlue";
// The desktop's channel colours: the master curve in the accent.
const CHANNELS: readonly { id: Channel; label: Key; color?: string }[] = [
  { id: "curve", label: "develop.curve.rgb" },
  { id: "curveRed", label: "develop.curve.red", color: "#e5484d" },
  { id: "curveGreen", label: "develop.curve.green", color: "#46a758" },
  { id: "curveBlue", label: "develop.curve.blue", color: "#3e63dd" },
];

export function CurvePanel({ stack, set }: PanelProps) {
  const [channel, setChannel] = useState<Channel>("curve");
  const chan = CHANNELS.find((c) => c.id === channel)!;
  return (
    <>
      <Segments
        options={CHANNELS.map((c) => [c.id, t(c.label)] as const)}
        value={channel}
        onPick={setChannel}
        marked={(c) => !straight(stack[c])}
      />
      <div className="mx-auto mt-2 mb-3 max-w-[260px]">
        <CurveEditor
          // One per channel, so a point being dragged never carries over.
          key={channel}
          points={stack[channel]}
          color={chan.color}
          // A fingertip's reach, in hundredths of the editor.
          grab={11}
          hint={t("mobile.edit.curveHint")}
          onChange={(points) => set({ [channel]: points })}
          onCommit={(points) => set({ [channel]: points }, true)}
        />
      </div>
    </>
  );
}

// ------------------------------------------------------------------ Mixer

type Mix = "mixerHue" | "mixerSaturation" | "mixerLuminance";
/** A representative hue per band, for its swatch and its sliders' cues, as
 *  the desktop shows them. The pipeline sets the real band centres. */
const BAND_HUES = [0, 30, 52, 130, 185, 220, 275, 320];
const BAND_KEYS: Key[] = [
  "develop.mixer.band.red",
  "develop.mixer.band.orange",
  "develop.mixer.band.yellow",
  "develop.mixer.band.green",
  "develop.mixer.band.aqua",
  "develop.mixer.band.blue",
  "develop.mixer.band.purple",
  "develop.mixer.band.magenta",
];

export function MixerPanel({ stack, set }: PanelProps) {
  const [band, setBand] = useState(0);
  const h = BAND_HUES[band] ?? 0;
  const moved = (i: number) =>
    stack.mixerHue[i] !== 0 || stack.mixerSaturation[i] !== 0 || stack.mixerLuminance[i] !== 0;
  const setBandValue = (field: Mix, v: number, now?: boolean) => {
    const next = [...stack[field]] as Band8;
    next[band] = v;
    set({ [field]: next }, now);
  };
  return (
    <>
      <div className="my-1 grid grid-cols-8" role="radiogroup" aria-label={t("develop.panel.colorMixer")}>
        {BAND_NAMES.map((name, i) => (
          <button
            key={name}
            role="radio"
            aria-checked={band === i}
            aria-label={t(BAND_KEYS[i]!)}
            title={t(BAND_KEYS[i]!)}
            className="relative flex h-11 items-center justify-center"
            onClick={() => setBand(i)}
          >
            <span
              className={`block h-7 w-7 rounded-full ${band === i ? "ring-2 ring-white ring-offset-2 ring-offset-base" : ""}`}
              style={{ background: `hsl(${BAND_HUES[i]} 72% 52%)` }}
            />
            {moved(i) && <span aria-hidden className="absolute top-0.5 right-0.5 h-1.5 w-1.5 rounded-full bg-accent" />}
          </button>
        ))}
      </div>
      <div className="text-caption text-ink">{t(BAND_KEYS[band]!)}</div>
      <Slider
        label={t("develop.mixer.hue")}
        value={stack.mixerHue[band] ?? 0}
        min={-1}
        max={1}
        step={0.01}
        format={percent}
        gradient={`linear-gradient(90deg, hsl(${h - 25} 70% 55%), hsl(${h} 80% 55%), hsl(${h + 25} 70% 55%))`}
        onChange={(v, now) => setBandValue("mixerHue", v, now)}
      />
      <Slider
        label={t("develop.mixer.saturation")}
        value={stack.mixerSaturation[band] ?? 0}
        min={-1}
        max={1}
        step={0.01}
        format={percent}
        gradient={`linear-gradient(90deg, #7c7c82, hsl(${h} 82% 52%))`}
        onChange={(v, now) => setBandValue("mixerSaturation", v, now)}
      />
      <Slider
        label={t("develop.mixer.luminance")}
        value={stack.mixerLuminance[band] ?? 0}
        min={-1}
        max={1}
        step={0.01}
        format={percent}
        gradient={`linear-gradient(90deg, hsl(${h} 45% 22%), hsl(${h} 42% 50%), hsl(${h} 55% 82%))`}
        onChange={(v, now) => setBandValue("mixerLuminance", v, now)}
      />
    </>
  );
}

// ---------------------------------------------------------------- Grading

const RANGE_KEYS: Record<GradeRangeName, Key> = {
  shadows: "tools.grading.range.shadows",
  midtones: "tools.grading.range.midtones",
  highlights: "tools.grading.range.highlights",
};

export function GradingPanel({ stack, set }: PanelProps) {
  const [range, setRange] = useState<GradeRangeName>("midtones");
  const g = stack.grading;
  const grade = g[range];
  const setGrade = (patch: Partial<typeof grade>, now?: boolean) =>
    set({ grading: { ...g, [range]: { ...grade, ...patch } } }, now);
  return (
    <>
      <Segments
        options={GRADE_RANGES.map((r) => [r, sentenceCase(t(RANGE_KEYS[r]))] as const)}
        value={range}
        onPick={setRange}
        marked={(r) => g[r].saturation > 0.001 || Math.abs(g[r].luminance) > 0.001}
      />
      <div className="mt-2 flex items-center gap-4">
        <Wheel
          size={132}
          grade={grade}
          onChange={(next) => set({ grading: { ...g, [range]: next } })}
          onCommit={(next) => set({ grading: { ...g, [range]: next } }, true)}
        />
        <div className="min-w-0 flex-1">
          <Slider label={t("tools.grading.hue")} value={grade.hue} min={0} max={359} step={1} resetTo={0} format={(v) => `${Math.round(v)}°`} onChange={(v, now) => setGrade({ hue: v }, now)} />
          <Slider label={t("tools.grading.saturation")} value={grade.saturation} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setGrade({ saturation: v }, now)} />
        </div>
      </div>
      <Slider label={t("tools.grading.luminance")} value={grade.luminance} min={-1} max={1} step={0.01} format={percent} onChange={(v, now) => setGrade({ luminance: v }, now)} />
      <Slider label={t("develop.grading.blending")} value={g.blending} min={-1} max={1} step={0.01} resetTo={D.grading.blending} format={percent} onChange={(v, now) => set({ grading: { ...g, blending: v } }, now)} />
      <Slider label={t("develop.grading.balance")} value={g.balance} min={-1} max={1} step={0.01} resetTo={D.grading.balance} format={percent} onChange={(v, now) => set({ grading: { ...g, balance: v } }, now)} />
    </>
  );
}

// ----------------------------------------------------------------- Detail

/** The desktop's sharpening and noise reduction. The phone shows the photo
 *  at screen size, where these read gently; they are made for the full
 *  photo, as a saved copy or the computer shows it. */
export function DetailPanel({ stack, set }: PanelProps) {
  const d = stack.detail;
  const setDetail = (patch: Partial<typeof d>, now?: boolean) => set({ detail: { ...d, ...patch } }, now);
  return (
    <>
      <Heading>{t("develop.detail.sharpening")}</Heading>
      <Slider label={t("develop.detail.amount")} value={d.sharpen} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setDetail({ sharpen: v }, now)} />
      <Slider label={t("develop.detail.radius")} value={d.sharpenRadius} min={0.5} max={3} step={0.1} resetTo={D.detail.sharpenRadius} format={(v) => v.toFixed(1)} onChange={(v, now) => setDetail({ sharpenRadius: v }, now)} />
      <Slider label={t("develop.detail.detail")} value={d.sharpenDetail} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setDetail({ sharpenDetail: v }, now)} />
      <Slider label={t("develop.detail.masking")} value={d.sharpenMasking} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setDetail({ sharpenMasking: v }, now)} />
      <Heading>{t("develop.detail.noiseReduction")}</Heading>
      <Slider label={t("develop.detail.luminanceNoise")} value={d.noise} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setDetail({ noise: v }, now)} />
      <Slider label={t("develop.detail.colorNoise")} value={d.colorNoise} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setDetail({ colorNoise: v }, now)} />
      <p className="mt-2 mb-1 text-label leading-relaxed text-faint">{t("mobile.edit.detailNote")}</p>
    </>
  );
}

// ---------------------------------------------------------------- Effects

export function EffectsPanel({ stack, set }: PanelProps) {
  const d = stack.detail;
  const e = stack.effects;
  const setEffects = (patch: Partial<typeof e>, now?: boolean) => set({ effects: { ...e, ...patch } }, now);
  return (
    <>
      {(["texture", "clarity", "dehaze"] as const).map((k) => (
        <Slider key={k} label={t(`develop.presence.${k}`)} value={d[k]} min={-1} max={1} step={0.01} format={percent} onChange={(v, now) => set({ detail: { ...d, [k]: v } }, now)} />
      ))}
      <Heading>{t("develop.effects.vignetteHeading")}</Heading>
      <Slider label={t("develop.effects.amount")} value={e.vignette} min={-1} max={1} step={0.01} format={percent} onChange={(v, now) => setEffects({ vignette: v }, now)} />
      <Slider label={t("develop.effects.midpoint")} value={e.vignetteMidpoint} min={0} max={1} step={0.01} resetTo={D.effects.vignetteMidpoint} format={hundred} onChange={(v, now) => setEffects({ vignetteMidpoint: v }, now)} />
      <Heading>{t("develop.effects.grainHeading")}</Heading>
      <Slider label={t("develop.effects.amount")} value={e.grain} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setEffects({ grain: v }, now)} />
      <Slider label={t("develop.effects.grainSize")} value={e.grainSize} min={0.3} max={3} step={0.05} resetTo={D.effects.grainSize} format={(v) => v.toFixed(2)} onChange={(v, now) => setEffects({ grainSize: v }, now)} />
      <Slider label={t("develop.effects.cleanHighlights")} value={e.grainHighlightRolloff} min={0} max={1} step={0.01} resetTo={D.effects.grainHighlightRolloff} format={hundred} onChange={(v, now) => setEffects({ grainHighlightRolloff: v }, now)} />
      <Heading>{t("develop.effects.halationHeading")}</Heading>
      <Slider label={t("develop.effects.amount")} value={e.halation} min={0} max={1} step={0.01} format={hundred} onChange={(v, now) => setEffects({ halation: v }, now)} />
      <Slider label={t("develop.effects.threshold")} value={e.halationThreshold} min={0} max={1} step={0.01} resetTo={D.effects.halationThreshold} format={hundred} onChange={(v, now) => setEffects({ halationThreshold: v }, now)} />
    </>
  );
}

// ----------------------------------------------------------------- Optics

export function OpticsPanel({ stack, set }: PanelProps) {
  const l = stack.lens;
  return (
    <>
      {(
        [
          ["distortion", "develop.optics.distortion"],
          ["vignetting", "develop.optics.vignetting"],
          ["chromatic", "develop.optics.fringing"],
        ] as const
      ).map(([k, label]) => (
        <Slider key={k} label={t(label)} value={l[k]} min={-1} max={1} step={0.01} format={percent} onChange={(v, now) => set({ lens: { ...l, [k]: v } }, now)} />
      ))}
    </>
  );
}

// ------------------------------------------------------------------- Crop

export function CropPanel({
  stack,
  set,
  size,
  ratio,
  setRatio,
  cropTo,
}: PanelProps & {
  size: { width: number; height: number } | null;
  ratio: number | null;
  setRatio: (r: number | null) => void;
  cropTo: (r: number) => void;
}) {
  const g = stack.geometry;
  const turned = (g.quarterTurns ?? 0) % 2 === 1;
  const chip = (on: boolean) => `min-h-10 px-3 text-caption ${on ? "btn-accent" : "btn-solid"}`;
  return (
    <>
      <Heading>{t("tools.crop.aspectRatio")}</Heading>
      <div className="mt-1 flex flex-wrap gap-2">
        {size && (
          <button className={chip(false)} onClick={() => cropTo(turned ? size.height / size.width : size.width / size.height)}>
            {t("tools.crop.original")}
          </button>
        )}
        {ASPECT_PRESETS.map((p) => (
          <button
            key={p.label}
            className={chip(ratio === p.ratio)}
            aria-pressed={ratio === p.ratio}
            onClick={() => (p.ratio === null ? setRatio(null) : cropTo(p.ratio))}
          >
            {p.labelKey ? t(p.labelKey) : p.label}
          </button>
        ))}
      </div>
      <div className="mt-3 flex gap-2">
        <button className="btn-solid min-h-12 flex-1 gap-2 text-caption" onClick={() => set({ geometry: { ...g, quarterTurns: ((g.quarterTurns ?? 0) + 3) % 4 } }, true)}>
          <Icon name="rotate-left" size={18} />
          {t("tools.crop.rotateLeftTitle")}
        </button>
        <button className="btn-solid min-h-12 flex-1 gap-2 text-caption" onClick={() => set({ geometry: { ...g, quarterTurns: ((g.quarterTurns ?? 0) + 1) % 4 } }, true)}>
          <Icon name="rotate-right" size={18} />
          {t("tools.crop.rotateRightTitle")}
        </button>
        <button className={`min-h-12 flex-1 gap-2 text-caption ${g.flip ? "btn-accent" : "btn-solid"}`} aria-pressed={g.flip} onClick={() => set({ geometry: { ...g, flip: !g.flip } }, true)}>
          <Icon name="flip" size={18} />
          {t("tools.crop.flip")}
        </button>
      </div>
      <Slider label={t("tools.crop.straighten")} value={g.straighten ?? 0} min={-15} max={15} step={0.1} format={(v) => `${v.toFixed(1)}°`} onChange={(v, now) => set({ geometry: { ...g, straighten: v } }, now)} />
    </>
  );
}

// ------------------------------------------------------------------ Presets

/** A preview card's side, in CSS pixels. */
const CARD = 84;

/** A card image: `s` applied to the photo, square, at the screen's density. */
const cardSrc = (s: object) => {
  const side = Math.round(CARD * window.devicePixelRatio);
  return editFrameUrl(side, side, 70, s);
};

/**
 * The built-in presets: a list of groups, by style and by subject, each
 * opening onto its presets as cards showing each on this photo. The preset
 * that is on gets its Amount above the list.
 *
 * Cards are drawn on the edit as it stood when the panel opened, with any
 * preset taken off, so the Amount moving doesn't redraw every card.
 */
export function PresetsPanel({
  stack,
  change,
  groups,
}: PanelProps & {
  /** Replaces the whole edit, as applying a preset does. */
  change: (next: Stack, now?: boolean) => void;
  groups: PresetGroupInfo[];
}) {
  const groupOf = (id: string) => groups.find((g) => g.presets.some((p) => p.id === id));
  const [open, setOpen] = useState<string | null>(() => groupOf(stack.preset.id)?.id ?? null);
  const group = groups.find((g) => g.id === open);

  // The edit the cards are drawn on: this one, without its preset.
  const [base, setBase] = useState<Stack | null>(null);
  useEffect(() => {
    let gone = false;
    api
      .editPreset(stack, "", 1)
      .then((s) => !gone && setBase(withStackDefaults(s)))
      .catch(() => {});
    return () => {
      gone = true;
    };
    // Once, as the panel opens: see above.
  }, []);
  // Each preset of the open group laid over the base, for its card.
  const [cards, setCards] = useState<Record<string, Stack>>({});
  useEffect(() => {
    if (!base || !group) return;
    let gone = false;
    void Promise.all(group.presets.map((p) => api.editPreset(base, p.id, 1).then((s) => [p.id, withStackDefaults(s)] as const)))
      .then((pairs) => !gone && setCards((was) => ({ ...was, ...Object.fromEntries(pairs) })))
      .catch(() => {});
    return () => {
      gone = true;
    };
  }, [base, group]);

  const use = async (id: string) => {
    try {
      change(withStackDefaults(await api.editPreset(stack, id, 1)), true);
    } catch {
      // A preset this build doesn't know: nothing happens.
    }
  };
  // The Amount: each move is computed by the core, and only the latest lands.
  const seq = useRef(0);
  const setAmount = (amount: number, now?: boolean) => {
    const n = ++seq.current;
    void api
      .editPreset(stack, stack.preset.id, amount)
      .then((s) => n === seq.current && change(withStackDefaults(s), now))
      .catch(() => {});
  };

  const on = groupOf(stack.preset.id);
  const onInfo = on?.presets.find((p) => p.id === stack.preset.id);

  return (
    <div className="pb-3">
      {on && onInfo && (
        <div className="mb-1">
          <div className="mt-1 flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-label text-ink">
              {t("develop.presets.appliedName", {
                group: presetGroupName(on.id, on.name),
                name: presetName(onInfo.id, onInfo.name),
              })}
            </span>
            <button className="min-h-10 shrink-0 px-2 text-label text-muted active:text-ink" onClick={() => void use("")}>
              {t("common.remove")}
            </button>
          </div>
          <Slider
            label={t("develop.presets.amount")}
            value={stack.preset.amount}
            min={0}
            max={1.5}
            step={0.01}
            resetTo={1}
            format={(v) => `${Math.round(v * 100)}%`}
            onChange={setAmount}
          />
        </div>
      )}
      {group ? (
        <>
          <button className="-ml-1 mt-1 flex min-h-10 items-center gap-1 text-body text-ink" onClick={() => setOpen(null)}>
            <Icon name="chevron" size={18} className="rotate-90 text-muted" />
            {presetGroupName(group.id, group.name)}
          </button>
          <p className="text-label text-muted">{presetGroupDescription(group.id, group.description)}</p>
          <Cards>
            {group.presets.map((p) => (
              <Card
                key={p.id}
                label={presetName(p.id, p.name)}
                src={cards[p.id] ? cardSrc(cards[p.id]!) : null}
                selected={stack.preset.id === p.id}
                onClick={() => void use(stack.preset.id === p.id ? "" : p.id)}
              />
            ))}
          </Cards>
        </>
      ) : (
        (["style", "subject"] as const).map((kind) => (
          <div key={kind}>
            <Heading>{t(kind === "style" ? "develop.presets.byStyle" : "develop.presets.bySubject")}</Heading>
            {groups
              .filter((g) => g.kind === kind)
              .map((g) => (
                <button
                  key={g.id}
                  className="flex min-h-12 w-full items-center gap-2 border-b border-line py-2 text-left last:border-b-0"
                  onClick={() => setOpen(g.id)}
                >
                  <span className="min-w-0 flex-1">
                    <span className="block text-body text-ink">
                      {presetGroupName(g.id, g.name)}
                      {on?.id === g.id && <span className="ml-1.5 inline-block size-1.5 rounded-full bg-accent align-middle" />}
                    </span>
                    <span className="block truncate text-label text-muted">{presetGroupDescription(g.id, g.description)}</span>
                  </span>
                  <span className="shrink-0 text-label tabular-nums text-faint">{g.presets.length}</span>
                  <Icon name="chevron" size={18} className="shrink-0 -rotate-90 text-faint" />
                </button>
              ))}
          </div>
        ))
      )}
    </div>
  );
}

// ------------------------------------------------------------------ LUTs

/**
 * The built-in film LUTs as cards showing each on this photo, by family,
 * with the strength of the one on. Cards are drawn on the edit as it stood
 * when the panel opened, without its LUT.
 */
export function LutsPanel({ stack, set, looks }: PanelProps & { looks: LookInfo[] }) {
  const [base] = useState<Stack>(() => ({ ...stack, lut: { name: "", intensity: 1 } }));
  const choose = (file: string) => {
    // Chosen from none, a LUT starts at full strength, as on the desktop.
    const intensity = stack.lut.intensity > 0 ? stack.lut.intensity : 1;
    set({ lut: { name: file, intensity: file ? intensity : stack.lut.intensity } }, true);
  };
  const families = useMemo(
    () =>
      looks.reduce<{ family: string; looks: LookInfo[] }[]>((groups, l) => {
        const last = groups[groups.length - 1];
        if (last && last.family === l.family) last.looks.push(l);
        else groups.push({ family: l.family, looks: [l] });
        return groups;
      }, []),
    [looks],
  );

  return (
    <div className="pb-3">
      {stack.lut.name && (
        <Slider
          label={t("develop.luts.intensity")}
          value={stack.lut.intensity}
          min={0}
          max={1}
          step={0.01}
          resetTo={1}
          format={(v) => `${Math.round(v * 100)}%`}
          onChange={(v, now) => set({ lut: { ...stack.lut, intensity: v } }, now)}
        />
      )}
      {families.map((f) => (
        <div key={f.family}>
          <Heading>{lookFamilyName(f.family)}</Heading>
          <Cards>
            {f.looks.map((l) => (
              <Card
                key={l.file}
                label={lookTitle({ name: l.file, title: l.title, blurb: l.blurb })}
                src={cardSrc({ ...base, lut: { name: l.file, intensity: 1 } })}
                selected={stack.lut.name === l.file}
                onClick={() => choose(stack.lut.name === l.file ? "" : l.file)}
              />
            ))}
          </Cards>
        </div>
      ))}
    </div>
  );
}

/** A row of preview cards that scrolls sideways. */
function Cards({ children }: { children: React.ReactNode }) {
  return <div className="mt-1 flex gap-2 overflow-x-auto pb-1">{children}</div>;
}

/** A Style or look on the photo, with its name under it; the one on is
 *  outlined. */
function Card({
  label,
  src,
  selected,
  onClick,
}: {
  label: string;
  src: string | null;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button className="flex shrink-0 flex-col gap-1 text-left" style={{ width: CARD }} aria-pressed={selected} onClick={onClick}>
      <span
        className={`relative block overflow-hidden rounded-sm bg-raised ${selected ? "outline outline-2 outline-offset-1 outline-accent" : ""}`}
        style={{ width: CARD, height: CARD }}
      >
        {src && (
          <img
            // A new preview is a new image: one that failed doesn't hide the next.
            key={src}
            src={src}
            alt=""
            loading="lazy"
            decoding="async"
            draggable={false}
            className="h-full w-full object-cover"
            onError={(e) => (e.currentTarget.style.visibility = "hidden")}
          />
        )}
      </span>
      <span className={`truncate text-label ${selected ? "text-ink" : "text-muted"}`}>{label}</span>
    </button>
  );
}
