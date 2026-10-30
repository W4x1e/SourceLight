import type React from "react";
import { sentenceCase } from "../lib/format";
import { Icon } from "../lib/icons";
import ModelDownloads from "./ModelDownloads";
import { t, type Key } from "../i18n";
import { maskLabel } from "../lib/content";
import { SAT_GRADIENT, TEMP_GRADIENT, TINT_GRADIENT } from "../lib/developCues";
import {
  type CreatableKind,
  type Mask,
  type MaskComponent,
  type MaskMode,
  maskAdj,
  maskComponents,
  newMask,
  type MaskShape,
  type ShapeKind,
} from "../types";
import type { SelectPoint } from "./dev/ClickSelectOverlay";
import AiMaskProgress, { type AiMaskStage } from "./dev/AiMaskProgress";
import NumberSlider from "./NumberSlider";

/**
 * The masking side panel: AI subject and click-to-select, the list of masks,
 * the selected mask's components, brush settings, and its local adjustments.
 *
 * Moved out of `Develop.tsx` with its JSX unchanged. It owns no
 * state: the masks live in the edit stack, and the selection, rename, brush,
 * overlay and AI state in Develop, which also drives the canvas those share.
 * So this is a view over them plus the actions that change them — the same
 * `label` / `commit` shape Develop's own sliders use, so undo history reads
 * the same.
 */

/** The shape kinds offered as create buttons, in panel order (AI is generated,
 *  not drawn, so it is not here). */
const SHAPE_KINDS: CreatableKind[] = ["radial", "linear", "brush", "luminance", "colorRange"];

// Keys, not words: a table at module scope is built once, before the language
// is known (see `i18n`).
const SHAPE_KEYS: Record<ShapeKind, Key> = {
  radial: "tools.mask.shape.radial",
  linear: "tools.mask.shape.linear",
  brush: "tools.mask.shape.brush",
  luminance: "tools.mask.shape.luminance",
  colorRange: "tools.mask.shape.colorRange",
  ai: "tools.mask.shape.ai",
};

/** A human label for a component: AI masks carry their own ("Subject", "Sky"),
 *  which the core stores in English and `maskLabel` words when drawn. */
function shapeLabel(shape: MaskShape): string {
  return shape.type === "ai" ? (shape.label ? maskLabel(shape.label) : t(SHAPE_KEYS.ai)) : t(SHAPE_KEYS[shape.type]);
}

export interface Brush {
  size: number;
  hardness: number;
  erase: boolean;
}

/** A mask's display name: its own, an AI mask's label, or its first shape and
 *  number. */
export function maskName(m: Mask, i: number): string {
  // The user's own name as they typed it; a derived one sentence-cased.
  const cs = maskComponents(m);
  const ai = cs.find((c) => c.shape.type === "ai");
  // The core names an AI mask by its English label ("Subject"): that is not a
  // name the user gave it, so it is worded in their language (Mac test).
  const ownName = m.name && m.name.trim() && !(ai?.shape.type === "ai" && m.name === ai.shape.label);
  if (ownName) return m.name!;
  if (ai && ai.shape.type === "ai") return sentenceCase(ai.shape.label ? maskLabel(ai.shape.label) : t("tools.mask.subject"));
  const first = cs[0];
  return sentenceCase(
    t("tools.mask.defaultName", {
      shape: first ? shapeLabel(first.shape) : t("tools.mask.fallbackName"),
      number: i + 1,
    }),
  );
}

export interface MaskPanelProps {
  masks: Mask[];
  selectedMask: number;
  setSelectedMask: (i: number) => void;
  selectedComp: number;
  setSelectedComp: (i: number) => void;
  renamingMask: number | null;
  setRenamingMask: (i: number | null) => void;
  brush: Brush;
  setBrush: React.Dispatch<React.SetStateAction<Brush>>;
  maskOverlay: boolean;
  setMaskOverlay: (on: boolean) => void;
  picking: boolean;
  pickPoints: SelectPoint[];
  setPickMode: (on: boolean) => void;
  aiBusy: boolean;
  /** The step an AI mask run is on, as the backend last said. */
  aiStage: AiMaskStage | null;
  aiError: string | null;
  runAiSubject: () => void | Promise<void>;
  /** Leaves masking. */
  setMaskMode: (on: boolean) => void;
  addMask: (m: Mask) => void;
  removeMask: (i: number) => void;
  renameMask: (i: number, name: string) => void;
  /** `commit` false while dragging, true for a final value. */
  editMask: (i: number, m: Mask, label: string, commit: boolean) => void;
  addComponent: (kind: CreatableKind) => void;
  removeComponent: (i: number) => void;
  editComponent: (i: number, c: MaskComponent, label: string, commit: boolean) => void;
  setCompMode: (i: number, mode: MaskMode) => void;
  toggleCompInvert: (i: number) => void;
  /** Commits the dragged value as one undo step, labelled. */
  commit: (label: string) => void;
}

export default function MaskPanel({
  masks,
  selectedMask,
  setSelectedMask,
  selectedComp,
  setSelectedComp,
  renamingMask,
  setRenamingMask,
  brush,
  setBrush,
  maskOverlay,
  setMaskOverlay,
  picking,
  pickPoints,
  setPickMode,
  aiBusy,
  aiStage,
  aiError,
  runAiSubject,
  setMaskMode,
  addMask,
  removeMask,
  renameMask,
  editMask,
  addComponent,
  removeComponent,
  editComponent,
  setCompMode,
  toggleCompInvert,
  commit,
}: MaskPanelProps) {
  // Captured once so `noUncheckedIndexedAccess` narrows them after a check.
  const selMask = masks[selectedMask];
  const comps = selMask ? maskComponents(selMask) : [];
  const selComp = comps[selectedComp];

  return (
  <div className="p-3">
    <div className="mb-3 flex items-center gap-2">
      <span className="flex-1 text-title font-semibold">{t("tools.mask.title")}</span>
      {/* Solid, not the key: Export is the one key on Develop's screen. */}
      <button
        onClick={() => setMaskMode(false)}
        className="btn-solid px-2.5 py-1 text-caption"
      >
        {t("common.done")}
      </button>
    </div>
    <button
      onClick={runAiSubject}
      disabled={aiBusy || picking}
      className="btn-solid mb-1 w-full justify-center gap-1.5 text-caption disabled:opacity-50"
      title={t("tools.mask.selectSubjectTitle")}
    >
      <Icon name="mask" size={13} className="shrink-0" />
      {aiBusy && !picking ? t("tools.mask.selectingSubject") : t("tools.mask.selectSubject")}
    </button>
    <button aria-pressed={picking}
      onClick={() => setPickMode(!picking)}
      // The same button as Select subject above it, in the same size: it
      // was a pixel-shorter hand-rolled one.
      className={`btn-solid mb-1 w-full justify-center gap-1.5 text-caption ${picking ? "seg-on" : ""}`}
      title={t("tools.mask.pickTitle")}
    >
      {!picking && <Icon name="pointer" size={13} className="shrink-0" />}
      {picking ? t("tools.mask.donePicking") : t("tools.mask.clickToSelect")}
    </button>
    {picking && (
      <p className="mb-2 text-label leading-relaxed text-faint">
        {pickPoints.length === 0 ? t("tools.mask.pickPrompt") : t("tools.mask.pickRefine")}
      </p>
    )}
    {/* A first use downloads the model: shown here, with Cancel. */}
    {aiBusy && <ModelDownloads className="mb-2" />}
    <AiMaskProgress busy={aiBusy} stage={aiStage} refining={picking && pickPoints.length > 1} className="mb-2" />
    {aiError && (
      // text-caption, not text-label: this was 10px red text
      // carrying a raw filesystem path, which is unreadable at
      // exactly the moment someone needs to read it.
      <p className="mb-2 text-caption leading-relaxed text-danger">{aiError}</p>
    )}
    <div className="section-label mt-2 px-0">{t("tools.mask.newMask")}</div>
    {/* The three drawn shapes over the two ranges, on a six-column grid so
        both rows fill the rail. Wrapped as they came, five buttons left
        "+ Colour" alone on a full-width row of its own. */}
    <div className="mb-3 grid grid-cols-6 gap-1">
      {SHAPE_KINDS.map((kind, i) => (
        <button
          key={kind}
          onClick={() => addMask(newMask(kind))}
          className={`btn-solid min-w-0 justify-center px-1.5 text-caption ${i < 3 ? "col-span-2" : "col-span-3"}`}
          title={t("tools.mask.newShapeTitle", { shape: t(SHAPE_KEYS[kind]) })}
        >
          <span className="truncate">{t("tools.mask.addShapeButton", { shape: t(SHAPE_KEYS[kind]) })}</span>
        </button>
      ))}
    </div>

    {masks.length === 0 ? (
      <p className="text-label leading-relaxed text-faint">
        {t("tools.mask.empty")}
      </p>
    ) : (
      <div className="mb-3 flex flex-col gap-0.5">
        {masks.map((m, i) => (
          <div
            key={i}
            onClick={() => {
              setSelectedMask(i);
              setSelectedComp(0);
            }}
            className={`group flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 ${
              i === selectedMask ? "bg-accent-dim/25 text-ink" : "text-muted hover:bg-hover"
            }`}
          >
            <Icon name="mask" size={13} className="shrink-0" />
            {renamingMask === i ? (
              <input
                autoFocus
                defaultValue={maskName(m, i)}
                onClick={(e) => e.stopPropagation()}
                onBlur={(e) => {
                  renameMask(i, e.target.value);
                  setRenamingMask(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    renameMask(i, (e.target as HTMLInputElement).value);
                    setRenamingMask(null);
                  } else if (e.key === "Escape") {
                    setRenamingMask(null);
                  }
                }}
                className="flex-1 rounded-sm bg-raised px-1 py-0.5 text-caption text-ink outline-none ring-1 ring-accent-dim"
              />
            ) : (
              // A button, so Tab reaches the list: the row was a click-only
              // div, and a mask could not be chosen from the keyboard.
              <button
                className="min-w-0 flex-1 truncate text-left text-caption"
                aria-pressed={i === selectedMask}
                onClick={(e) => {
                  e.stopPropagation();
                  setSelectedMask(i);
                  setSelectedComp(0);
                }}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  setRenamingMask(i);
                }}
                title={t("tools.mask.renameHint")}
              >
                {maskComponents(m).length > 1
                  ? t("tools.mask.nameWithCount", { name: maskName(m, i), count: maskComponents(m).length })
                  : maskName(m, i)}
              </button>
            )}
            <button
              onClick={(e) => {
                e.stopPropagation();
                setRenamingMask(i);
              }}
              className="text-faint opacity-0 transition-opacity hover:text-ink focus-visible:opacity-100 group-hover:opacity-100"
              title={t("tools.mask.rename")}
              aria-label={t("tools.mask.rename")}
            >
              <Icon name="pencil" size={12} />
            </button>
            <button
              onClick={(e) => {
                e.stopPropagation();
                removeMask(i);
              }}
              className="text-caption text-faint opacity-0 transition-opacity hover:text-danger focus-visible:opacity-100 group-hover:opacity-100"
              title={t("tools.mask.delete")}
              aria-label={t("tools.mask.delete")}
            >
              <Icon name="close" size={12} />
            </button>
          </div>
        ))}
      </div>
    )}

    {selMask && (
      <div className="border-t border-line pt-3">
        <div className="section-label px-0">{t("tools.mask.components")}</div>
        <div className="mb-2 flex flex-col gap-0.5">
          {comps.map((c, i) => (
            <div
              key={i}
              onClick={() => setSelectedComp(i)}
              className={`group flex cursor-default items-center gap-1.5 rounded-sm px-1.5 py-1 ${
                i === selectedComp ? "bg-accent-dim/25 text-ink" : "text-muted hover:bg-hover"
              }`}
            >
              {i === 0 ? (
                <span className="w-[70px] shrink-0 text-label text-faint">{t("tools.mask.base")}</span>
              ) : (
                <select
                  value={c.mode ?? "add"}
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) => setCompMode(i, e.target.value as MaskMode)}
                  className="w-[70px] shrink-0 rounded-sm bg-raised px-1 py-0.5 text-label text-ink"
                >
                  <option value="add">{t("tools.mask.mode.add")}</option>
                  <option value="subtract">{t("tools.mask.mode.subtract")}</option>
                  <option value="intersect">{t("tools.mask.mode.intersect")}</option>
                </select>
              )}
              <button
                className="min-w-0 flex-1 truncate text-left text-caption"
                aria-pressed={i === selectedComp}
                onClick={(e) => {
                  e.stopPropagation();
                  setSelectedComp(i);
                }}
              >
                {c.inverted
                  ? t("tools.mask.invertedName", { shape: sentenceCase(shapeLabel(c.shape)) })
                  : shapeLabel(c.shape)}
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  toggleCompInvert(i);
                }}
                className={`grid place-items-center rounded-sm px-1 ${c.inverted ? "text-accent" : "text-faint hover:text-ink"}`}
                title={t("tools.mask.invertTitle")}
                aria-label={t("tools.mask.invertTitle")}
                aria-pressed={c.inverted}
              >
                <Icon name="invert" size={12} />
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  removeComponent(i);
                }}
                className="text-caption text-faint opacity-0 transition-opacity hover:text-danger focus-visible:opacity-100 group-hover:opacity-100"
                title={t("tools.mask.deleteComponent")}
                aria-label={t("tools.mask.deleteComponent")}
              >
                <Icon name="close" size={12} />
              </button>
            </div>
          ))}
        </div>
        <div className="mb-3 flex flex-wrap gap-1">
          {SHAPE_KINDS.map((kind) => (
            <button
              key={kind}
              onClick={() => addComponent(kind)}
              className="rounded-sm bg-raised px-1.5 py-0.5 text-label text-muted hover:text-ink"
              title={t("tools.mask.addComponentTitle", { shape: t(SHAPE_KEYS[kind]) })}
            >
              {t("tools.mask.addShapeButton", { shape: t(SHAPE_KEYS[kind]) })}
            </button>
          ))}
        </div>

        {selComp && (
        <>
        <OverlayToggle on={maskOverlay} onChange={setMaskOverlay} label={t("tools.mask.showMask")} />
        {selComp.shape.type === "radial" && (
          <NumberSlider
            label={t("tools.mask.feather")}
            value={selComp.shape.type === "radial" ? (selComp.shape.feather ?? 0.5) : 0.5}
            min={0}
            max={1}
            step={0.01}
            defaultValue={0.5}
            onChange={(v) => {
              const c = selComp;
              if (c.shape.type !== "radial") return;
              editComponent(selectedComp, { ...c, shape: { ...c.shape, feather: v } }, t("tools.mask.feather"), false);
            }}
            onCommit={() => commit(t("tools.mask.feather"))}
          />
        )}
        {selComp.shape.type === "brush" && (
          <>
            <p className="mb-2 text-label leading-relaxed text-faint">
              {t("tools.mask.brushHelp")}
            </p>
            <NumberSlider
              label={t("tools.mask.brushSize")}
              value={brush.size}
              min={0.01}
              max={0.35}
              step={0.005}
              defaultValue={0.06}
              onChange={(v) => setBrush((b) => ({ ...b, size: v }))}
              onCommit={() => {}}
            />
            <NumberSlider
              label={t("tools.mask.hardness")}
              value={brush.hardness}
              min={0}
              max={1}
              step={0.01}
              defaultValue={0.5}
              onChange={(v) => setBrush((b) => ({ ...b, hardness: v }))}
              onCommit={() => {}}
            />
            <div className="mb-2 flex items-center gap-2">
              <button aria-pressed={brush.erase}
                onClick={() => setBrush((b) => ({ ...b, erase: !b.erase }))}
                className={`btn-solid flex-1 justify-center py-1 text-caption ${brush.erase ? "seg-on" : ""}`}
              >
                {brush.erase ? t("tools.mask.erasing") : t("tools.mask.erase")}
              </button>
              <button
                onClick={() => {
                  const c = selComp;
                  if (c.shape.type !== "brush") return;
                  editComponent(
                    selectedComp,
                    { ...c, shape: { type: "brush", strokes: [] } },
                    t("tools.mask.clearBrush"),
                    true,
                  );
                }}
                className="btn-ghost flex-1 justify-center py-1 text-caption text-muted hover:text-ink"
              >
                {t("common.clear")}
              </button>
            </div>
          </>
        )}
        {selComp.shape.type === "luminance" && (
          <>
            <p className="mb-2 text-label leading-relaxed text-faint">
              {t("tools.mask.luminanceHelp")}
            </p>
            {(
              [
                ["low", t("tools.mask.low"), 0.5],
                ["high", t("tools.mask.high"), 1],
                ["feather", t("tools.mask.feather"), 0.15],
              ] as const
            ).map(([field, label, rest]) => (
              <NumberSlider
                key={field}
                label={label}
                defaultValue={rest}
                value={selComp.shape.type === "luminance" ? (selComp.shape[field] ?? 0) : 0}
                min={0}
                max={1}
                step={0.01}
                onChange={(v) => {
                  const c = selComp;
                  if (c.shape.type !== "luminance") return;
                  editComponent(
                    selectedComp,
                    { ...c, shape: { ...c.shape, [field]: v } },
                    label,
                    false,
                  );
                }}
                onCommit={() => commit(t("tools.mask.rangeHistory", { label }))}
              />
            ))}
          </>
        )}
        {selComp.shape.type === "colorRange" && (
          <>
            <p className="mb-2 text-label leading-relaxed text-faint">
              {t("tools.mask.colorHelp")}
            </p>
            {(
              [
                ["hue", t("tools.mask.hue"), 0.08],
                ["range", t("tools.mask.range"), 0.1],
                ["feather", t("tools.mask.feather"), 0.08],
              ] as const
            ).map(([field, label, rest]) => (
              <NumberSlider
                key={field}
                label={label}
                defaultValue={rest}
                value={selComp.shape.type === "colorRange" ? (selComp.shape[field] ?? 0) : 0}
                min={0}
                max={field === "hue" ? 1 : 0.5}
                step={0.01}
                onChange={(v) => {
                  const c = selComp;
                  if (c.shape.type !== "colorRange") return;
                  editComponent(
                    selectedComp,
                    { ...c, shape: { ...c.shape, [field]: v } },
                    label,
                    false,
                  );
                }}
                onCommit={() => commit(t("tools.mask.colourHistory", { label }))}
              />
            ))}
          </>
        )}
        {selComp.shape.type === "ai" && (
          <>
            <p className="mb-2 text-label leading-relaxed text-faint">
              {t("tools.mask.aiHelp")}
            </p>
          </>
        )}
        </>
        )}
        {/* What the mask does, under its own heading and in the order of the
            whole-photo rail (white balance, light, presence), so a control is
            where the eye already looks for it. They followed only a divider,
            with Temperature between Shadows and Saturation. */}
        <div className="section-label mt-3 border-t border-line px-0 pt-3">{t("tools.mask.adjustHeading")}</div>
        {(
          [
            // Same colour cues the global panel uses, so temperature
            // and tint read the same in both places.
            ["temperature", "tools.mask.adj.temperature", "", TEMP_GRADIENT],
            ["tint", "tools.mask.adj.tint", "", TINT_GRADIENT],
            ["exposure", "tools.mask.adj.exposure", " EV", undefined],
            ["contrast", "tools.mask.adj.contrast", "", undefined],
            ["highlights", "tools.mask.adj.highlights", "", undefined],
            ["shadows", "tools.mask.adj.shadows", "", undefined],
            ["whites", "tools.mask.adj.whites", "", undefined],
            ["blacks", "tools.mask.adj.blacks", "", undefined],
            ["texture", "tools.mask.adj.texture", "", undefined],
            ["clarity", "tools.mask.adj.clarity", "", undefined],
            // Grey at the low end → vivid at the high end: a "how much
            // colour" hint, harmless and unambiguous.
            ["saturation", "tools.mask.adj.saturation", "", SAT_GRADIENT],
          ] as const
        ).map(([field, labelKey, suffix, gradient]) => (
          <NumberSlider
            key={field}
            label={t(labelKey)}
            value={maskAdj(selMask)[field]}
            min={field === "exposure" ? -2 : -1}
            max={field === "exposure" ? 2 : 1}
            step={0.01}
            suffix={suffix}
            gradient={gradient}
            onChange={(v) => {
              const m = selMask;
              editMask(
                selectedMask,
                { ...m, adjustments: { ...maskAdj(m), [field]: v } },
                t(labelKey),
                false,
              );
            }}
            onCommit={() => commit(t("tools.mask.localHistory", { label: t(labelKey) }))}
          />
        ))}
      </div>
    )}
  </div>
  );
}

/** A clear show/hide switch for the mask overlay — an eye, a label and an
 *  On/Off word, so it reads as a real toggle rather than a stray checkbox. */
function OverlayToggle({
  on,
  onChange,
  label,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  label: string;
}) {
  return (
    <button
      onClick={() => onChange(!on)}
      title={on ? t("tools.mask.overlayHideTitle") : t("tools.mask.overlayShowTitle")}
      className={`mb-2 flex w-full items-center gap-2 rounded-md border px-2.5 py-1.5 text-caption transition-colors ${
        on
          ? "border-accent-dim bg-accent-dim/20 text-ink"
          : "border-line bg-raised text-muted hover:text-ink"
      }`}
    >
      <Icon name={on ? "eye" : "eye-off"} size={14} className={on ? "text-accent" : ""} />
      <span className="flex-1 text-left">{label}</span>
      <span className={`font-semibold ${on ? "text-accent" : "text-faint"}`}>{on ? t("common.on") : t("common.off")}</span>
    </button>
  );
}
