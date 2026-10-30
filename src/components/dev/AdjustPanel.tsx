import type { MutableRefObject } from "react";
import type { DevelopInfo } from "../../bindings";
import { comboToLabel } from "../../lib/keymap";
import { SAT_GRADIENT, TEMP_GRADIENT, TINT_GRADIENT } from "../../lib/developCues";
import { Icon } from "../../lib/icons";
import { t, type Key } from "../../i18n";
import { BAND_NAMES, DEFAULT_STACK, type Band8, type Stack } from "../../types";
import CurveEditor from "../CurveEditor";
import GradingWheels from "../GradingWheels";
import NumberSlider from "../NumberSlider";
import AiDenoise from "./AiDenoise";
import CollapsiblePanel from "./CollapsiblePanel";

/** The mixer's eight colour bands, in the order of `BAND_NAMES`. */
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

/** Whether any of the eight bands is moved off zero. */
export const bandsNz = (b: Band8) => b.some((x) => x !== 0);

type MixerMode = "hue" | "saturation" | "luminance";
type CurveChannel = "rgb" | "red" | "green" | "blue";

interface Props {
  draft: Stack;
  /** The draft as it is this instant: a commit in the same tick as a change
   *  reads it here. */
  draftRef: MutableRefObject<Stack>;
  /** Changes the draft only; `commit` puts it in history. */
  set: (patch: Partial<Stack>) => void;
  commit: (label: string) => void;
  /** Changes and commits in one step. */
  apply: (patch: Partial<Stack>, label: string) => void;
  adjust: (label: string, patch: Partial<Stack>) => Promise<void>;
  isOpen: (id: string) => boolean;
  toggle: (id: string) => void;
  /** Which groups hold edits, for their dots and resets. */
  mod: Record<"wb" | "light" | "curve" | "presence" | "mixer" | "grading" | "detail" | "optics" | "effects", boolean>;
  develop: DevelopInfo;
  autoWb: () => Promise<void>;
  wbPicking: boolean;
  toggleWbPicker: (on: boolean) => void;
  wbKey: string;
  curveChannel: CurveChannel;
  setCurveChannel: (c: CurveChannel) => void;
  mixerMode: MixerMode;
  setMixerMode: (m: MixerMode) => void;
  zoom: number | null;
  pan: { x: number; y: number };
  setZoom: (zoom: number | null) => void;
  setPan: (pan: { x: number; y: number }) => void;
}

/**
 * The Adjust tool's controls: the groups of sliders in the rail while no other
 * tool is on, as Crop, Mask and Retouch have theirs.
 */
export default function AdjustPanel({
  draft,
  draftRef,
  set,
  commit,
  apply,
  adjust,
  isOpen,
  toggle,
  mod,
  develop,
  autoWb,
  wbPicking,
  toggleWbPicker,
  wbKey,
  curveChannel,
  setCurveChannel,
  mixerMode,
  setMixerMode,
  zoom,
  pan,
  setZoom,
  setPan,
}: Props) {
  const mixerField = (
    { hue: "mixerHue", saturation: "mixerSaturation", luminance: "mixerLuminance" } as const
  )[mixerMode];

  // Representative hue per HSL band, so each band's slider can show what it acts
  // on: the hue shifting in Hue mode, grey→colour in Saturation, dark→light in
  // Luminance. Purely indicative — the pipeline defines the real band centres.
  const BAND_HUES = [0, 30, 52, 130, 185, 220, 275, 320];
  const bandGradient = (i: number) => {
    const h = BAND_HUES[i] ?? 0;
    if (mixerMode === "hue")
      return `linear-gradient(90deg, hsl(${h - 25} 70% 55%), hsl(${h} 80% 55%), hsl(${h + 25} 70% 55%))`;
    if (mixerMode === "saturation")
      return `linear-gradient(90deg, #7c7c82, hsl(${h} 82% 52%))`;
    return `linear-gradient(90deg, hsl(${h} 45% 22%), hsl(${h} 42% 50%), hsl(${h} 55% 82%))`;
  };

  const bandSlider = (
    <>
      {BAND_NAMES.map((band, i) => (
        <NumberSlider
          key={band}
          label={t(BAND_KEYS[i] ?? "develop.mixer.band.red")}
          value={draft[mixerField][i] ?? 0}
          min={-1}
          max={1}
          step={0.01}
          gradient={bandGradient(i)}
          onChange={(v) => {
            const next = [...draft[mixerField]] as Band8;
            next[i] = v;
            set({ [mixerField]: next } as Partial<Stack>);
          }}
          onCommit={() =>
            commit(
              t(
                mixerMode === "hue"
                  ? "develop.history.mixerHue"
                  : mixerMode === "saturation"
                    ? "develop.history.mixerSaturation"
                    : "develop.history.mixerLuminance",
                { band: t(BAND_KEYS[i] ?? "develop.mixer.band.red") },
              ),
            )
          }
        />
      ))}
    </>
  );

  /** A segmented switch — the one pattern for every either/or in the rail.
   *  `marked` puts a dot on an option that holds edits the switch is hiding. */
  const segmented = <T extends string>(
    options: readonly (readonly [T, Key])[],
    value: T,
    onPick: (v: T) => void,
    marked: (v: T) => boolean = () => false,
  ) => (
    <div className="mb-2 seg-group">
      {options.map(([id, label]) => (
        <button
          key={id}
          onClick={() => onPick(id)}
          aria-pressed={value === id}
          className={`flex flex-1 items-center justify-center gap-1 py-1 text-caption ${
            value === id ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
          }`}
        >
          {t(label)}
          {marked(id) && value !== id && <span className="h-1 w-1 rounded-full bg-accent" />}
        </button>
      ))}
    </div>
  );

  return (
    <>
      <CollapsiblePanel
        title={t("develop.panel.whiteBalance")}
        open={isOpen("wb")}
        onToggle={() => toggle("wb")}
        modified={mod.wb}
        onReset={() => apply({ warmth: 0, tint: 0 }, t("develop.history.resetWhiteBalance"))}
      >
        <div className="mb-1.5 seg-group">
          <button
            onClick={() => void autoWb()}
            className="flex flex-1 items-center justify-center gap-1.5 py-1 text-caption text-muted hover:bg-hover hover:text-ink"
            title={t("develop.wb.autoTitle")}
          >
            <Icon name="adjust" size={13} className="shrink-0" />
            {t("develop.auto.label")}
          </button>
          <button
            onClick={() => toggleWbPicker(!wbPicking)}
            className={`flex flex-1 items-center justify-center gap-1.5 py-1 text-caption ${
              wbPicking ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
            }`}
            aria-pressed={wbPicking}
            title={wbKey ? t("develop.wb.pickerTitleKey", { key: comboToLabel(wbKey) }) : t("develop.wb.pickerTitle")}
          >
            <Icon name="dropper" size={13} className="shrink-0" />
            {t("develop.wb.pickNeutral")}
          </button>
        </div>
        {wbPicking && (
          <p className="mb-1.5 text-label text-muted">{t("develop.wb.pickerHint")}</p>
        )}
        <NumberSlider
          label={t("develop.wb.temperature")}
          value={draft.warmth}
          min={-1}
          max={1}
          step={0.01}
          gradient={TEMP_GRADIENT}
          onChange={(v) => set({ warmth: v })}
          onCommit={() => commit(t("develop.wb.temperature"))}
        />
        <NumberSlider
          label={t("develop.wb.tint")}
          value={draft.tint}
          min={-1}
          max={1}
          step={0.01}
          gradient={TINT_GRADIENT}
          onChange={(v) => set({ tint: v })}
          onCommit={() => commit(t("develop.wb.tint"))}
        />
      </CollapsiblePanel>

      {/* Presets and the creative LUT live in the Presets panel, on the
          other side of the photo. */}

      <CollapsiblePanel
        title={t("develop.panel.light")}
        open={isOpen("light")}
        onToggle={() => toggle("light")}
        modified={mod.light}
        onReset={() =>
          apply(
            { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
            t("develop.history.resetLight"),
          )
        }
      >
        <NumberSlider label={t("develop.light.exposure")} value={draft.exposure} min={-5} max={5} step={0.05} suffix=" EV" onChange={(v) => set({ exposure: v })} onCommit={() => commit(t("develop.light.exposure"))} />
        <NumberSlider label={t("develop.light.contrast")} value={draft.contrast} min={-1} max={1} step={0.01} onChange={(v) => set({ contrast: v })} onCommit={() => commit(t("develop.light.contrast"))} />
        <div className="my-1.5 border-t border-line" />
        <NumberSlider label={t("develop.light.highlights")} value={draft.highlights} min={-1} max={1} step={0.01} onChange={(v) => set({ highlights: v })} onCommit={() => commit(t("develop.light.highlights"))} />
        <NumberSlider label={t("develop.light.shadows")} value={draft.shadows} min={-1} max={1} step={0.01} onChange={(v) => set({ shadows: v })} onCommit={() => commit(t("develop.light.shadows"))} />
        <NumberSlider label={t("develop.light.whites")} value={draft.whites} min={-1} max={1} step={0.01} onChange={(v) => set({ whites: v })} onCommit={() => commit(t("develop.light.whites"))} />
        <NumberSlider label={t("develop.light.blacks")} value={draft.blacks} min={-1} max={1} step={0.01} onChange={(v) => set({ blacks: v })} onCommit={() => commit(t("develop.light.blacks"))} />
      </CollapsiblePanel>

      <CollapsiblePanel
        title={t("develop.panel.toneCurve")}
        open={isOpen("curve")}
        onToggle={() => toggle("curve")}
        modified={mod.curve}
        onReset={() =>
          apply(
            { curve: [], curveRed: [], curveGreen: [], curveBlue: [] },
            t("develop.history.resetCurve"),
          )
        }
      >
        {(() => {
          // Each channel edits its own field; the master "RGB" tab is the
          // existing luminance curve, the others the new per-channel ones.
          const chan = {
            rgb: { field: "curve", label: t("develop.history.toneCurve"), color: undefined as string | undefined },
            red: { field: "curveRed", label: t("develop.history.redCurve"), color: "#e5484d" },
            green: { field: "curveGreen", label: t("develop.history.greenCurve"), color: "#46a758" },
            blue: { field: "curveBlue", label: t("develop.history.blueCurve"), color: "#3e63dd" },
          }[curveChannel];
          const points = draft[chan.field as "curve" | "curveRed" | "curveGreen" | "curveBlue"];
          const curveOf = { rgb: draft.curve, red: draft.curveRed, green: draft.curveGreen, blue: draft.curveBlue };
          return (
            <>
              {segmented(
                [
                  ["rgb", "develop.curve.rgb"],
                  ["red", "develop.curve.red"],
                  ["green", "develop.curve.green"],
                  ["blue", "develop.curve.blue"],
                ] as const,
                curveChannel,
                setCurveChannel,
                (c) => curveOf[c].length > 0,
              )}
              <CurveEditor
                // Remount per channel so the drag refs never carry one
                // channel's in-flight point into another.
                key={curveChannel}
                points={points}
                color={chan.color}
                onChange={(pts) => set({ [chan.field]: pts })}
                onCommit={(pts) => void adjust(chan.label, { ...draftRef.current, [chan.field]: pts })}
              />
            </>
          );
        })()}
      </CollapsiblePanel>

      {/* Presence, the mixer and grading were three tabs inside one Color
          group, so an edit on a tab you were not looking at was invisible
          and a reset had nowhere to live. They are groups of their own,
          each with its own dot and reset, as other editors lay them out. */}
      <CollapsiblePanel
        title={t("develop.panel.presence")}
        open={isOpen("presence")}
        onToggle={() => toggle("presence")}
        modified={mod.presence}
        onReset={() =>
          apply(
            { detail: { ...draftRef.current.detail, texture: 0, clarity: 0, dehaze: 0 }, vibrance: 0, saturation: 0 },
            t("develop.history.resetPresence"),
          )
        }
      >
        <NumberSlider label={t("develop.presence.texture")} value={draft.detail.texture} min={-1} max={1} step={0.01} onChange={(v) => set({ detail: { ...draft.detail, texture: v } })} onCommit={() => commit(t("develop.presence.texture"))} />
        <NumberSlider label={t("develop.presence.clarity")} value={draft.detail.clarity} min={-1} max={1} step={0.01} onChange={(v) => set({ detail: { ...draft.detail, clarity: v } })} onCommit={() => commit(t("develop.presence.clarity"))} />
        <NumberSlider label={t("develop.presence.dehaze")} value={draft.detail.dehaze} min={-1} max={1} step={0.01} onChange={(v) => set({ detail: { ...draft.detail, dehaze: v } })} onCommit={() => commit(t("develop.presence.dehaze"))} />
        <div className="my-1.5 border-t border-line" />
        {/* Vibrance lifts the least-saturated colour first, so it grows
            muted tones without pushing skin and vivid areas to a poster;
            Saturation is the flat, everything-equally control. Both wear
            the grey→vivid cue the mask panel's saturation already uses. */}
        <NumberSlider label={t("develop.presence.vibrance")} value={draft.vibrance} min={-1} max={1} step={0.01} gradient={SAT_GRADIENT} onChange={(v) => set({ vibrance: v })} onCommit={() => commit(t("develop.presence.vibrance"))} />
        <NumberSlider label={t("develop.presence.saturation")} value={draft.saturation} min={-1} max={1} step={0.01} gradient={SAT_GRADIENT} onChange={(v) => set({ saturation: v })} onCommit={() => commit(t("develop.presence.saturation"))} />
      </CollapsiblePanel>

      <CollapsiblePanel
        title={t("develop.panel.colorMixer")}
        open={isOpen("mixer")}
        onToggle={() => toggle("mixer")}
        modified={mod.mixer}
        onReset={() =>
          apply(
            {
              mixerHue: DEFAULT_STACK.mixerHue,
              mixerSaturation: DEFAULT_STACK.mixerSaturation,
              mixerLuminance: DEFAULT_STACK.mixerLuminance,
            },
            t("develop.history.resetColorMixer"),
          )
        }
      >
        {segmented(
          [
            ["hue", "develop.mixer.hue"],
            ["saturation", "develop.mixer.saturation"],
            ["luminance", "develop.mixer.luminance"],
          ] as const,
          mixerMode,
          setMixerMode,
          (m) =>
            bandsNz(
              draft[({ hue: "mixerHue", saturation: "mixerSaturation", luminance: "mixerLuminance" } as const)[m]],
            ),
        )}
        {bandSlider}
      </CollapsiblePanel>

      <CollapsiblePanel
        title={t("develop.panel.colorGrading")}
        open={isOpen("grading")}
        onToggle={() => toggle("grading")}
        modified={mod.grading}
        onReset={() => apply({ grading: DEFAULT_STACK.grading }, t("develop.history.resetColorGrading"))}
      >
        <GradingWheels
          grading={draft.grading}
          onChange={(grading) => set({ grading })}
          onCommit={(grading, label) => void adjust(label, { ...draftRef.current, grading })}
        />
        <NumberSlider label={t("develop.grading.blending")} value={draft.grading.blending} min={-1} max={1} step={0.01} defaultValue={DEFAULT_STACK.grading.blending} onChange={(v) => set({ grading: { ...draft.grading, blending: v } })} onCommit={() => commit(t("develop.history.gradeBlending"))} />
        <NumberSlider label={t("develop.grading.balance")} value={draft.grading.balance} min={-1} max={1} step={0.01} defaultValue={DEFAULT_STACK.grading.balance} onChange={(v) => set({ grading: { ...draft.grading, balance: v } })} onCommit={() => commit(t("develop.history.gradeBalance"))} />
      </CollapsiblePanel>

      <CollapsiblePanel
        title={t("develop.panel.detail")}
        open={isOpen("detail")}
        onToggle={() => toggle("detail")}
        modified={mod.detail}
        onReset={() =>
          apply(
            { detail: { ...draftRef.current.detail, sharpen: 0, sharpenRadius: 1, sharpenDetail: 0, sharpenMasking: 0, noise: 0, colorNoise: 0, aiDenoise: 0 } },
            t("develop.history.resetDetail"),
          )
        }
      >
        <div className="mb-0.5 flex items-center justify-between">
          <span className="section-label px-0 pb-0">{t("develop.detail.sharpening")}</span>
          {zoom === null && (
            <button
              onClick={() => {
                setZoom(1);
                setPan({ x: 0.5, y: 0.5 });
              }}
              className="rounded-sm px-1.5 py-0.5 text-label text-muted hover:bg-hover hover:text-ink"
              title={t("develop.detail.viewActualPixelsTitle")}
            >
              {t("develop.detail.viewActualPixels")}
            </button>
          )}
        </div>
        {(
          [
            ["sharpen", "develop.detail.amount", "develop.history.sharpening", 0, 1, 0.01, 0],
            ["sharpenRadius", "develop.detail.radius", "develop.history.sharpeningRadius", 0.5, 3, 0.1, 1],
            ["sharpenDetail", "develop.detail.detail", "develop.history.sharpeningDetail", 0, 1, 0.01, 0],
            ["sharpenMasking", "develop.detail.masking", "develop.history.sharpeningMasking", 0, 1, 0.01, 0],
          ] as const
        ).map(([field, label, historyLabel, min, max, step, rest]) => (
          <NumberSlider
            key={field}
            label={t(label)}
            value={draft.detail[field]}
            min={min}
            max={max}
            step={step}
            defaultValue={rest}
            onChange={(v) => set({ detail: { ...draft.detail, [field]: v } })}
            onCommit={() => commit(t(historyLabel))}
          />
        ))}
        {/* Its own heading: under "Sharpening" these read as two more
            sharpening controls. */}
        <div className="mb-0.5 mt-2 section-label px-0 pb-0">{t("develop.detail.noiseReduction")}</div>
        {(
          [
            ["noise", "develop.detail.luminanceNoise", "develop.history.luminanceNoise"],
            ["colorNoise", "develop.detail.colorNoise", "develop.history.colorNoise"],
          ] as const
        ).map(([field, label, historyLabel]) => (
          <NumberSlider
            key={field}
            label={t(label)}
            value={draft.detail[field]}
            min={0}
            max={1}
            step={0.01}
            onChange={(v) => set({ detail: { ...draft.detail, [field]: v } })}
            onCommit={() => commit(t(historyLabel))}
          />
        ))}
        <AiDenoise
          amount={draft.detail.aiDenoise}
          ready={develop.denoiseReady}
          width={develop.width}
          height={develop.height}
          focus={zoom !== null ? pan : null}
          onChange={(v) => set({ detail: { ...draft.detail, aiDenoise: v } })}
          onCommit={() => commit(t("develop.history.aiNoiseReduction"))}
        />
      </CollapsiblePanel>

      <CollapsiblePanel
        title={t("develop.panel.optics")}
        open={isOpen("optics")}
        onToggle={() => toggle("optics")}
        modified={mod.optics}
        onReset={() => apply({ lens: { distortion: 0, vignetting: 0, chromatic: 0 } }, t("develop.history.resetOptics"))}
      >
        {(
          [
            ["distortion", "develop.optics.distortion"],
            ["vignetting", "develop.optics.vignetting"],
            ["chromatic", "develop.optics.fringing"],
          ] as const
        ).map(([field, label]) => (
          <NumberSlider
            key={field}
            label={t(label)}
            value={draft.lens[field]}
            min={-1}
            max={1}
            step={0.01}
            onChange={(v) => set({ lens: { ...draft.lens, [field]: v } })}
            onCommit={() => commit(t(label))}
          />
        ))}
      </CollapsiblePanel>

      <CollapsiblePanel
        title={t("develop.panel.effects")}
        open={isOpen("effects")}
        onToggle={() => toggle("effects")}
        modified={mod.effects}
        onReset={() => apply({ effects: DEFAULT_STACK.effects }, t("develop.history.resetEffects"))}
      >
        <div className="mb-0.5 section-label px-0 pb-0">{t("develop.effects.vignetteHeading")}</div>
        <NumberSlider
          label={t("develop.effects.amount")}
          value={draft.effects.vignette}
          min={-1}
          max={1}
          step={0.01}
          onChange={(v) => set({ effects: { ...draft.effects, vignette: v } })}
          onCommit={() => commit(t("develop.history.vignette"))}
        />
        <NumberSlider
          label={t("develop.effects.midpoint")}
          value={draft.effects.vignetteMidpoint}
          min={0}
          max={1}
          step={0.01}
          defaultValue={DEFAULT_STACK.effects.vignetteMidpoint}
          onChange={(v) => set({ effects: { ...draft.effects, vignetteMidpoint: v } })}
          onCommit={() => commit(t("develop.history.vignetteMidpoint"))}
        />
        <div className="mb-0.5 mt-2 section-label px-0 pb-0">{t("develop.effects.grainHeading")}</div>
        <NumberSlider
          label={t("develop.effects.amount")}
          value={draft.effects.grain}
          min={0}
          max={1}
          step={0.01}
          onChange={(v) => set({ effects: { ...draft.effects, grain: v } })}
          onCommit={() => commit(t("develop.history.grain"))}
        />
        <NumberSlider
          label={t("develop.effects.grainSize")}
          value={draft.effects.grainSize}
          min={0.3}
          max={3}
          step={0.05}
          defaultValue={DEFAULT_STACK.effects.grainSize}
          onChange={(v) => set({ effects: { ...draft.effects, grainSize: v } })}
          onCommit={() => commit(t("develop.history.grainSize"))}
        />
        {/* How much the grain spares the highlights: film grain thins
            out in the brightest parts of a print. */}
        <NumberSlider
          label={t("develop.effects.cleanHighlights")}
          value={draft.effects.grainHighlightRolloff}
          min={0}
          max={1}
          step={0.01}
          defaultValue={DEFAULT_STACK.effects.grainHighlightRolloff}
          onChange={(v) => set({ effects: { ...draft.effects, grainHighlightRolloff: v } })}
          onCommit={() => commit(t("develop.history.grainRolloff"))}
        />
        {/* Halation: the warm bloom out of a highlight that film gives
            you and a sensor never does. */}
        <div className="mb-0.5 mt-2 section-label px-0 pb-0">{t("develop.effects.halationHeading")}</div>
        <NumberSlider
          label={t("develop.effects.amount")}
          value={draft.effects.halation}
          min={0}
          max={1}
          step={0.01}
          onChange={(v) => set({ effects: { ...draft.effects, halation: v } })}
          onCommit={() => commit(t("develop.history.halation"))}
        />
        <NumberSlider
          label={t("develop.effects.threshold")}
          value={draft.effects.halationThreshold}
          min={0}
          max={1}
          step={0.01}
          defaultValue={DEFAULT_STACK.effects.halationThreshold}
          onChange={(v) => set({ effects: { ...draft.effects, halationThreshold: v } })}
          onCommit={() => commit(t("develop.history.halationThreshold"))}
        />
      </CollapsiblePanel>
    </>
  );
}
