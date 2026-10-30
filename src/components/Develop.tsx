import { wheelIntent } from "../lib/wheel";
import { fromView, orientedAspect, toView } from "../lib/cropView";
import { useWindowWidth } from "../lib/useWindowWidth";
import { askToSaveAsCopy } from "../lib/saveAsCopy";
import { textIsSelected } from "../lib/copyImage";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
// Aliased: `save` is already the store's save-edits action in this component,
// and the shadowing resolves silently to the wrong function.
import { open as openDirectoryDialog } from "@tauri-apps/plugin-dialog";
import { ensureAi } from "../lib/aiReady";
import { api, developUrl, type ViewPort } from "../lib/api";
import { canPan, clampCentre, panBy } from "../lib/viewport";
import { usePanDrag } from "../lib/usePanDrag";
import { useLatestFrame } from "../lib/useLatestFrame";
import { isTextEntry, overlayOpen } from "../lib/useEscape";
import { useDraftToken } from "../lib/useDraftToken";
import { useDirectFrames, useDirectSurface, useTransparentPage } from "../lib/directCanvas";
import { comboToLabel, resolveAction } from "../lib/keymap";
import { COPIED_BY_DEFAULT } from "../lib/developGroups";
import GroupsDialog, { type GroupsAsk } from "./dev/GroupsDialog";
import { confirm, prompt, toast } from "../lib/dialogs";
import { useStore, AUTO_FIELDS } from "../store";
import { t, formatList } from "../i18n";
import type {
  DevelopInfo,
  Group,
  LutInfo,
  PresetGroupInfo,
  Suggestion,
} from "../bindings";
import { presetName } from "../lib/content";
import { Icon } from "../lib/icons";
import CropOverlay from "./CropOverlay";
import MaskOverlay from "./dev/MaskOverlay";
import SpotOverlay from "./dev/SpotOverlay";
import ClickSelectOverlay, { type SelectPoint } from "./dev/ClickSelectOverlay";
import { aiStageText, isAiMaskStage, Spinner, useShownAfter, type AiMaskStage } from "./dev/AiMaskProgress";
import SpotPanel from "./SpotPanel";
import ErasePanel, { type EraseBrush, type EraseHow } from "./ErasePanel";
import EraseSelectionOverlay from "./dev/EraseSelectionOverlay";
import { NO_SELECTION, hasSelection, selectionMask, type EraseSelection } from "../lib/eraseSelection";
import { isCancelled, isDownload } from "../lib/jobs";
import MaskPanel from "./MaskPanel";
import ExportDialog from "./ExportDialog";
import CropPanel from "./dev/CropPanel";
import Histogram from "./dev/Histogram";
import Filmstrip from "./dev/Filmstrip";
import AdjustPanel, { bandsNz } from "./dev/AdjustPanel";
import CopyPasteBar from "./dev/CopyPasteBar";
import DevelopHeader from "./dev/DevelopHeader";
import ToolButton from "./dev/ToolButton";
import ViewBar from "./dev/ViewBar";
import { hint } from "./dev/shortcutHint";
import { CANVAS_MIN, PRESETS_MIN, RAIL_DEFAULT, RAIL_MAX, RAIL_MIN, railFit as fitRail, savedRailWidth } from "./dev/railWidth";
import DevelopPresets from "./DevelopPresets";
import {
  DEFAULT_STACK,
  type Mask,
  type MaskComponent,
  type MaskMode,
  maskComponents,
  newComponent,
  newSpot,
  type Spot,
  type CreatableKind,
  withStackDefaults,
  type Preset,
  type Stack,
} from "../types";

/** Which adjustment groups start open, the first time. After that the rail
 *  opens the way it was left. */
const INITIAL_OPEN: Record<string, boolean> = { wb: true, light: true, presence: true };
const OPEN_KEY = "developOpenGroups";
const SOLO_KEY = "developSolo";

function storedOpen(): Record<string, boolean> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(OPEN_KEY) ?? "null");
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, boolean>) : INITIAL_OPEN;
  } catch {
    return INITIAL_OPEN;
  }
}

/** Runs one of Develop's own actions and says why if it fails: a failed
 *  Copy to selected, preset or paste was an unhandled rejection, logged and
 *  otherwise silent. `undefined` when it failed. */
async function attempt<T>(run: () => Promise<T>): Promise<T | undefined> {
  try {
    return await run();
  } catch (e) {
    toast(t("develop.actionFailed", { error: String(e) }), "error");
    return undefined;
  }
}

export default function Develop() {
  const develop = useStore((s) => s.develop);
  const error = useStore((s) => s.developError);
  const adjust = useStore((s) => s.adjust);
  const close = useStore((s) => s.closeDevelop);
  const undo = useStore((s) => s.undoDevelop);
  const redo = useStore((s) => s.redoDevelop);
  const toggleAuto = useStore((s) => s.toggleAuto);
  const autoWb = useStore((s) => s.autoWb);
  const pickWb = useStore((s) => s.pickWb);
  const wbKey = useStore((s) => s.keymap["develop-wb-picker"]);
  const undoKey = useStore((s) => s.keymap["develop-undo"]);
  const redoKey = useStore((s) => s.keymap["develop-redo"]);
  const cropKey = useStore((s) => s.keymap["develop-crop"]);
  const maskKey = useStore((s) => s.keymap["develop-mask"]);
  const copyKey = useStore((s) => s.keymap["develop-copy-settings"]);
  const pasteKey = useStore((s) => s.keymap["develop-paste-settings"]);
  // Said in their buttons' tooltips, as Undo's and Crop's are: Z, = and −
  // and Ctrl+Shift+R did these with nothing on screen to say so.
  const zoomKey = useStore((s) => s.keymap["toggle-zoom"]);
  const zoomInKey = useStore((s) => s.keymap["develop-zoom-in"]);
  const zoomOutKey = useStore((s) => s.keymap["develop-zoom-out"]);
  const resetKey = useStore((s) => s.keymap["develop-reset"]);
  const autoActive = useStore((s) => s.autoActive);
  const reset = useStore((s) => s.resetDevelop);

  // Slider positions are held locally while dragging so the control stays
  // responsive; the backend is the source of truth between interactions.
  const [draft, setDraft] = useState<Stack>(withStackDefaults(develop?.stack));

  // `draft` mirrored into a ref so a control that changes *and* commits in the
  // same tick — typing a value then Enter, or an arrow-key nudge — commits the
  // value it just set rather than the pre-edit state React has not re-rendered
  // yet. Slider *drag* escaped this because its commit fires in a later event;
  // the keyboard paths did not, which is why a typed value silently reverted.
  // Every draft write goes through `writeDraft` to keep ref and state in lockstep.
  const draftRef = useRef(draft);
  // The edit as committed, as the draft was last reset to. A draft that is any
  // other object is an edit mid-drag — shown by the surface and the histogram
  // before the release commits it.
  const committedDraft = useRef<Stack | null>(null);
  const writeDraft = (next: Stack) => {
    draftRef.current = next;
    setDraft(next);
  };

  // Whether the full Auto is currently applied and untouched — drives the toggle
  // button's active look, using the same field set the toggle logic compares.
  const autoOn =
    !!autoActive &&
    develop != null &&
    AUTO_FIELDS.every((f) => develop.stack[f] === autoActive.after[f]);

  const [presets, setPresets] = useState<Preset[]>([]);
  const [luts, setLuts] = useState<LutInfo[]>([]);
  // Photos selected in the library besides this one: what "Copy to selected"
  // would reach. This photo is often in the selection it was opened from, and
  // copying an edit onto itself is not something to offer.
  const selectedOthers = useStore((s) => {
    let n = 0;
    for (const id of s.selected) if (id !== s.develop?.photoId) n++;
    return n;
  });
  const setDevelop = useStore((s) => s.setDevelopInfo);
  // Choosing the parts of an edit to copy, paste or keep as a preset.
  const [groupsAsk, setGroupsAsk] = useState<GroupsAsk | null>(null);
  const askGroups = (q: Omit<GroupsAsk, "resolve">) =>
    new Promise<{ groups: Group[]; name: string } | null>((resolve) =>
      setGroupsAsk({
        ...q,
        resolve: (answer) => {
          setGroupsAsk(null);
          resolve(answer);
        },
      }),
    );
  // What Ctrl+Shift+C copied: the edit as it was then, and which parts.
  // In the store, so it outlives Develop: picking the next batch in the
  // library used to lose it, and Paste read the source's edit at paste time,
  // so an experiment made on it since came along instead.
  const clipboard = useStore((s) => s.settingsClipboard);
  const setClipboard = useStore((s) => s.setSettingsClipboard);
  const hasCopied = clipboard !== null;
  // The photo open before this one, for "Paste previous".
  const previousId = useStore((s) => s.previousDevelopId);
  const setPreviousId = useStore((s) => s.setPreviousDevelopId);
  const openId = useRef<number | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  useEffect(() => {
    const now = develop?.photoId ?? null;
    if (now !== openId.current) {
      if (openId.current !== null) setPreviousId(openId.current);
      openId.current = now;
    }
  }, [develop?.photoId]);

  const refreshPresets = () => void api.listPresets().then(setPresets);
  useEffect(refreshPresets, []);

  // The built-in presets, by group. A preset is computed onto the edit by the
  // core and applied like any other change, so it and its Amount go through
  // the same history as the sliders it moves.
  const [presetGroups, setPresetGroups] = useState<PresetGroupInfo[]>([]);
  useEffect(() => void api.listPresetGroups().then(setPresetGroups).catch(() => setPresetGroups([])), []);
  const builtinName = (id: string) => {
    for (const g of presetGroups) {
      const p = g.presets.find((x) => x.id === id);
      if (p) return presetName(p.id, p.name);
    }
    return id;
  };
  /** Puts built-in preset `id` on at full amount, or takes it off with "": one step. */
  const useBuiltin = (id: string) =>
    attempt(async () => {
      const next = withStackDefaults(await api.presetStack(draftRef.current, id, 1));
      apply(
        next,
        id
          ? t("develop.history.preset", { name: builtinName(id) })
          : t("develop.history.presetOff"),
      );
    });
  // The Amount slider: each move is computed by the core, latest wins, and the
  // release commits once the last of them has landed.
  const amountSeq = useRef(0);
  const amountPending = useRef<Promise<void>>(Promise.resolve());
  const changeAmount = (amount: number) => {
    const id = draftRef.current.preset.id;
    if (!id) return;
    const seq = ++amountSeq.current;
    amountPending.current = api
      .presetStack(draftRef.current, id, amount)
      .then((next) => {
        if (seq === amountSeq.current) set(withStackDefaults(next));
      })
      .catch(() => undefined);
  };
  const commitAmount = () => void amountPending.current.then(() => commit(t("develop.history.presetAmount")));

  // Suggested edits for this photo (live previews). Rendered when the panel
  // opens on a photo, and cleared when the photo changes so stale previews
  // never show.
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  // True until the first answer, so the panel opens on its loading tiles
  // rather than on "nothing to suggest" for the frame before it asks.
  const [suggesting, setSuggesting] = useState(true);
  useEffect(() => setSuggestions([]), [develop?.photoId]);
  const fetchSuggestions = useCallback(async () => {
    setSuggesting(true);
    try {
      setSuggestions(await api.suggestEdits());
    } catch {
      setSuggestions([]);
    } finally {
      setSuggesting(false);
    }
  }, []);
  const applySuggestion = async (s: Suggestion) => {
    if (s.kind === "auto") setDevelop(await api.autoAllDevelop());
    else await useBuiltin(s.id);
  };

  const refreshLuts = () => void api.listLuts().then(setLuts).catch(() => setLuts([]));
  useEffect(refreshLuts, []);

  // Live thumbnails, keyed by id (saved preset ids as strings, LUTs by file
  // name). Each is a GPU render, so they are fetched when they would show —
  // a tab, or a group of built-in presets, opened on this photo — and dropped
  // when the photo changes so a stale look never sits under a different image.
  const [builtinPreviews, setBuiltinPreviews] = useState<Record<string, string>>({});
  const [presetPreviews, setPresetPreviews] = useState<Record<string, string>>({});
  const [lutPreviews, setLutPreviews] = useState<Record<string, string>>({});
  // Which photo each tab's previews were loaded for (null = not yet).
  const previewsFor = useRef<{ presets: number | null; luts: number | null }>({ presets: null, luts: null });
  // The built-in previews asked for on the current photo.
  const builtinAsked = useRef(new Set<string>());
  const photoIdRef = useRef<number | null>(null);
  photoIdRef.current = develop?.photoId ?? null;
  const wantBuiltinPreviews = useCallback((ids: string[]) => {
    const missing = ids.filter((id) => !builtinAsked.current.has(id));
    if (missing.length === 0) return;
    for (const id of missing) builtinAsked.current.add(id);
    const photo = photoIdRef.current;
    void api
      .builtinPresetPreviews(missing)
      .then((rows) => {
        if (photoIdRef.current !== photo) return;
        setBuiltinPreviews((was) => ({ ...was, ...Object.fromEntries(rows.map((r) => [r.id, r.preview])) }));
      })
      .catch(() => {
        // Ask again next time the group shows.
        for (const id of missing) builtinAsked.current.delete(id);
      });
  }, []);
  const loadPresetPreviews = useCallback(async () => {
    try {
      const rows = await api.presetPreviews();
      setPresetPreviews(Object.fromEntries(rows.map((r) => [r.id, r.preview])));
    } catch {
      /* leave the swatches as plain tiles */
    }
  }, []);
  const loadLutPreviews = useCallback(async () => {
    try {
      const rows = await api.lutPreviews();
      setLutPreviews(Object.fromEntries(rows.map((r) => [r.id, r.preview])));
    } catch {
      /* leave the swatches as plain tiles */
    }
  }, []);
  useEffect(() => {
    setBuiltinPreviews({});
    setPresetPreviews({});
    setLutPreviews({});
    builtinAsked.current = new Set();
    previewsFor.current = { presets: null, luts: null };
  }, [develop?.photoId]);

  // Choosing a LUT (or "None") is a discrete change — one undoable step. Picking
  // one from "None" defaults intensity to full so the look is visible at once.
  const chooseLut = (name: string) => {
    const intensity = draftRef.current.lut.intensity > 0 ? draftRef.current.lut.intensity : 1;
    apply({ lut: { name, intensity: name ? intensity : draftRef.current.lut.intensity } }, t("develop.history.lut"));
  };

  /** The group headings the user has already made, for the "move to" prompts. */
  const lutGroupNames = useMemo(
    () => [...new Set(luts.filter((l) => l.grouped).map((l) => l.family))].sort(),
    [luts],
  );

  const importLut = async () => {
    const picked = await openDirectoryDialog({
      multiple: false,
      filters: [{ name: "Cube LUT", extensions: ["cube"] }],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    // Ask where it belongs while the answer is still obvious. Skipping the
    // prompt files it under "Imported", which is the old behaviour.
    const group = await prompt({
      title: t("develop.lut.importTitle"),
      message: lutGroupNames.length
        ? t("develop.lut.importAsk", { groups: formatList(lutGroupNames) })
        : t("develop.lut.importAskNone"),
      placeholder: t("develop.lut.groupPlaceholder"),
      confirmLabel: t("develop.lut.importConfirm"),
    });
    try {
      const info = await api.importLut(path, group ?? undefined);
      refreshLuts();
      void loadLutPreviews(); // the list changed — refresh its thumbnails
      chooseLut(info.name); // select what was just imported
    } catch (e) {
      toast(t("develop.lut.importFailed", { error: String(e) }), "error");
    }
  };

  /** Files one LUT under a group of the user's naming. */
  const moveLutToGroup = async (name: string, current: string, grouped: boolean) => {
    const group = await prompt({
      title: t("develop.lut.groupTitle", { name: name.replace(/\.cube$/i, "") }),
      message: lutGroupNames.length
        ? t("develop.lut.groupAsk", { groups: formatList(lutGroupNames) })
        : t("develop.lut.groupAskNone"),
      placeholder: t("develop.lut.groupPlaceholder"),
      defaultValue: grouped ? current : "",
      confirmLabel: t("develop.lut.moveConfirm"),
    });
    // `prompt` resolves null for both cancel and an empty box, so ungrouping
    // has its own button rather than being a blank submit nobody would guess.
    if (group === null) return;
    try {
      setLuts(await api.setLutGroup([name], group));
    } catch (e) {
      toast(t("develop.lut.moveFailed", { error: String(e) }), "error");
    }
  };

  /** Sends a LUT back to its default heading. */
  const ungroupLut = async (name: string) => {
    try {
      setLuts(await api.setLutGroup([name], null));
    } catch (e) {
      toast(t("develop.lut.ungroupFailed", { error: String(e) }), "error");
    }
  };

  // Deleting asks first: it was one hover click, and a deleted LUT
  // quietly drops out of every photo whose look used it.
  const removeLut = async (name: string) => {
    const used = await api.lutUsage(name).catch(() => 0);
    const yes = await confirm({
      title: t("develop.luts.deleteTitle", { name }),
      message:
        used > 0 ? t("develop.luts.deleteUsed", { count: used }) : t("develop.luts.deleteUnused"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!yes) return;
    await api.deleteLut(name);
    if (draftRef.current.lut.name === name) chooseLut(""); // deselect if in use
    refreshLuts();
    void loadLutPreviews(); // the list changed — refresh its thumbnails
  };

  const storePreset = async () => {
    const answer = await askGroups({
      title: t("develop.preset.saveTitle"),
      message: t("develop.preset.saveMessage"),
      confirmLabel: t("common.save"),
      withName: true,
    });
    if (!answer) return;
    if (!(await attempt(() => api.savePreset(answer.name, answer.groups)))) return;
    refreshPresets();
    void loadPresetPreviews(); // the list changed — refresh its thumbnails
  };

  const removePreset = async (id: number) => {
    const yes = await confirm({
      title: t("develop.preset.deleteTitle"),
      message: t("develop.preset.deleteMessage"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!yes) return;
    await api.deletePreset(id);
    refreshPresets();
    void loadPresetPreviews(); // the list changed — refresh its thumbnails
  };

  // Applying a preset lands in history as one step, so it can be undone as a
  // single action rather than slider by slider.
  const usePreset = (id: number) => attempt(async () => setDevelop(await api.applyPreset(id)));

  const pasteToSelection = async () => {
    const st = useStore.getState();
    const ids = [...st.selected].filter((id) => id !== st.develop?.photoId);
    if (ids.length === 0) return;
    const answer = await askGroups({
      title: t("develop.copy.toSelectedAsk", { count: ids.length }),
      message: t("develop.copy.toSelectedAskMessage"),
      confirmLabel: t("develop.copy.confirm"),
    });
    if (!answer) return;
    const n = await attempt(() => api.copySettingsTo(ids, answer.groups));
    if (n === undefined) return;
    toast(t("develop.copy.toSelectedDone", { count: n }), "success");
  };

  const copySettings = async () => {
    const photoId = useStore.getState().develop?.photoId;
    if (photoId == null) return;
    const answer = await askGroups({ title: t("develop.copy.ask"), confirmLabel: t("develop.copy.confirm") });
    if (!answer) return;
    // The edit as it is now, not a pointer to the photo.
    setClipboard({ stack: draftRef.current, groups: answer.groups });
    const paste = comboToLabel(useStore.getState().keymap["develop-paste-settings"]);
    toast(t("develop.copy.copied", { paste }), "success");
  };
  const pasteSettings = async () => {
    const from = useStore.getState().settingsClipboard;
    if (!from) {
      const copy = comboToLabel(useStore.getState().keymap["develop-copy-settings"]);
      toast(t("develop.copy.nothingCopied", { copy }), "info");
      return;
    }
    await attempt(async () => setDevelop(await api.pasteSettings(from.stack, from.groups)));
  };
  // "Previous", as photographers know it: the last photo's settings, less what
  // fits only it.
  const pastePrevious = async () => {
    if (previousId == null) return;
    await attempt(async () => setDevelop(await api.pasteSettingsFrom(previousId, COPIED_BY_DEFAULT)));
  };
  const goToStep = async (index: number) => setDevelop(await api.developGoTo(index));

  // Verdicts and stepping through the shoot, as everywhere else.
  const verdict = (patch: { rating?: number; flag?: number; colorLabel?: number }) => {
    const id = useStore.getState().develop?.photoId;
    if (id != null) void useStore.getState().compareApply([id], patch);
  };
  const step = (delta: number) => {
    const st = useStore.getState();
    // From the photo last asked for, not the one open yet: three presses
    // before the first open lands go three photos on, not one.
    const from = st.developWanted ?? st.develop?.photoId;
    const i = st.photos.findIndex((p) => p.id === from);
    const next = st.photos[i + delta];
    if (i >= 0 && next) void st.openDevelop(next.id);
  };

  // One set of eight sliders, switched between the three adjustments, rather
  // than twenty-four at once. The layout photographers know from other editors.
  const [mixerMode, setMixerMode] = useState<"hue" | "saturation" | "luminance">("saturation");

  // Which channel of the Tone Curve panel is being edited: the master RGB
  // (luminance) curve, or one of the per-channel R/G/B curves.
  const [curveChannel, setCurveChannel] = useState<"rgb" | "red" | "green" | "blue">("rgb");

  // Crop mode shows the whole frame with a rectangle over it. Cropping a photo
  // you can only see the crop of is impossible — there is nothing to drag the
  // edges back out into.
  const [cropping, setCropping] = useState(false);
  const [aspect, setAspect] = useState<number | null>(null);

  // Local adjustments. Mask mode swaps the right rail to the mask editor and
  // shows the selected mask's handles over the (fit-to-window) canvas.
  const [masking, setMasking] = useState(false);
  const [selectedMask, setSelectedMask] = useState(0);
  // Which mask's name is being edited inline in the list, if any.
  const [renamingMask, setRenamingMask] = useState<number | null>(null);
  // Which component of the selected mask is active (drives the overlay handles
  // and the shape sliders).
  const [selectedComp, setSelectedComp] = useState(0);
  // Retouch: heal/clone spots, or magic erase.
  const [spotting, setSpotting] = useState(false);
  const [retouch, setRetouch] = useState<"spots" | "erase">("spots");
  const erasing = spotting && retouch === "erase";
  // Magic erase: what is selected to erase, and how it is being chosen.
  const [eraseHow, setEraseHow] = useState<EraseHow>("click");
  const [eraseBrush, setEraseBrush] = useState<EraseBrush>({ size: 0.04, remove: false });
  const [eraseSel, setEraseSel] = useState<EraseSelection>(NO_SELECTION);
  const [erasePicking, setErasePicking] = useState(false);
  const [eraseStarting, setEraseStarting] = useState(false);
  const [eraseError, setEraseError] = useState<string | null>(null);
  const eraseJob = useStore((s) => s.jobs["erase"]);
  // A selection is one photo's: another photo opens without it.
  useEffect(() => {
    setEraseSel(NO_SELECTION);
    setEraseError(null);
  }, [develop?.photoId]);
  // While erasing, the photo as it now is gets encoded for click to select,
  // and the erase model loaded, in the background: on a new photo, and after
  // every erase, which changes what a click would select.
  const erasedFills = develop?.stack.erases?.map((e) => e.pieces?.[0]?.fill ?? "").join() ?? "";
  useEffect(() => {
    if (erasing) void api.prepareErase().catch(() => {});
  }, [erasing, develop?.photoId, erasedFills]);
  // The white-balance eyedropper: the next click on the photo sets it.
  const [wbPicking, setWbPicking] = useState(false);
  const [selectedSpot, setSelectedSpot] = useState(0);
  // AI auto-masking (Select Subject) — one-shot, with a busy + error state.
  const [aiBusy, setAiBusy] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  // The step a run is on (waiting for the graphics card, loading the model,
  // running), as the backend says: the models give no percentage.
  const [aiStage, setAiStage] = useState<AiMaskStage | null>(null);
  useEffect(() => {
    const un = listen<string>("ai-mask-stage", (e) => {
      if (isAiMaskStage(e.payload)) setAiStage(e.payload);
    });
    return () => void un.then((f) => f());
  }, []);
  // Said on the photo too, once a run has lasted long enough to need saying.
  const aiSlow = useShownAfter(aiBusy);
  // A first AI mask downloads its model before it runs: the chip on the photo
  // says so, with how far it has got, rather than "Getting ready…" for the
  // minute the rail spends counting megabytes.
  const download = useStore((s) => {
    const id = Object.keys(s.jobs).find((k) => isDownload(k) && !s.jobs[k]!.error);
    const j = id ? s.jobs[id]! : null;
    return j ? `${j.label}${j.total > 1 ? ` · ${Math.min(100, Math.round((j.current / j.total) * 100))}%` : ""}` : null;
  });
  // Click-to-select (SAM2): accumulated points and which mask they are building.
  const [picking, setPicking] = useState(false);
  // While the Straighten slider moves, the crop box shows a fine grid to
  // level against.
  const [straightening, setStraightening] = useState(false);
  const [pickPoints, setPickPoints] = useState<SelectPoint[]>([]);
  // Points taken back by Undo while picking, newest last, for Redo.
  const [pickRedo, setPickRedo] = useState<SelectPoint[]>([]);
  const pickMask = useRef<number | null>(null);
  // The clicks behind each picked mask, by its bitmap: the mask keeps only the
  // bitmap, and picking it again carries on from its points. One photo's.
  const pickedPoints = useRef(new Map<string, SelectPoint[]>());
  useEffect(() => {
    pickedPoints.current.clear();
    setPicking(false);
    setPickPoints([]);
    setPickRedo([]);
    pickMask.current = null;
  }, [develop?.photoId]);
  // Active brush settings (radius as a frame-width fraction), applied to new dabs.
  const [brush, setBrush] = useState({ size: 0.06, hardness: 0.5, erase: false });
  // Tint the painted area so the mask is visible; toggle off to see the edit.
  const [maskOverlay, setMaskOverlay] = useState(true);

  // `develop.width/height` is the *framed* size, so it already reflects the
  // crop. Dividing it back out recovers the uncropped shape the overlay needs,
  // including the axis swap a quarter turn causes.
  //
  // Divide out the geometry `develop` was actually *rendered* with, not the live
  // `draft`. During a corner drag the draft's crop changes on every pointer move
  // while `develop` still reflects the last commit; mixing the two made the
  // recovered aspect — and so the overlay's letterbox — lurch every frame, which
  // is what threw the crop box around the pane.
  // The oriented image's own shape, and the crop tool's view of it: turned,
  // since the tool now draws the turn, so a portrait-turned photo is a
  // portrait frame to crop in.
  const orient = (() => {
    if (!develop) return 1;
    return orientedAspect(develop.width, develop.height, withStackDefaults(develop.stack).geometry);
  })();
  const uncroppedAspect = ((draft.geometry.quarterTurns % 4) + 4) % 4 % 2 === 1 ? 1 / orient : orient;

  const setGeometry = (patch: Partial<Stack["geometry"]>, label: string) => {
    const geometry = { ...draftRef.current.geometry, ...patch };
    const next = { ...draftRef.current, geometry };
    writeDraft(next);
    void adjust(label, next);
  };

  /**
   * Picks an aspect ratio and immediately reshapes the crop to match.
   *
   * Setting the constraint alone does nothing until a handle is dragged, which
   * made every preset after the first look broken: the first click was visible
   * only because it opened crop mode. A preset is a request to reframe, so it
   * has to reframe.
   */
  const applyAspect = (ratio: number | null, label: string) => {
    setAspect(ratio);
    // "Free" releases the aspect lock and leaves the rectangle where it is.
    // It used to reset to the full frame as well, when there was no visible
    // reset to reach for instead; the crop panel has one now (Reset crop).
    // Picking a real ratio enters crop and reshapes the rectangle to it.
    if (ratio === null) return;
    setCropMode(true);

    // Shaped in the view, so 4:5 on a frame turned to portrait is portrait,
    // then stored in the image's own coordinates.
    const target = ratio / uncroppedAspect;
    const width = target > 1 ? 1 : target;
    const height = target > 1 ? 1 / target : 1;
    const box = { x: (1 - width) / 2, y: (1 - height) / 2, width, height };
    setGeometry({ crop: fromView(box, draftRef.current.geometry, orient) }, t("develop.history.cropRatio", { label }));
  };


  // Export lives in its own dialog off the toolbar (ExportDialog), which owns
  // every export setting; Develop only opens it and shows when it is busy.
  const [showExport, setShowExport] = useState(false);
  const [exporting, setExporting] = useState(false);


  const filename = useStore(
    (s) => s.photos.find((p) => p.id === develop?.photoId)?.filename ?? "export",
  );
  // The photo asked for while another is still on screen: stepping with the
  // arrows or the filmstrip. Decoding a raw is the one slow step, and the old
  // photo sat there unchanged meanwhile, as if the step had not been taken.
  const nextName = useStore((s) =>
    s.developWanted !== null && s.developWanted !== s.develop?.photoId && !s.developError
      ? (s.photos.find((p) => p.id === s.developWanted)?.filename ?? "")
      : null,
  );
  const opening = useShownAfter(nextName !== null, 250);

  // Render to the canvas size rather than a fixed proxy: every pixel asked for
  // is a pixel rendered, read back and copied, and nobody sees the ones outside
  // the canvas.
  const canvasRef = useRef<HTMLElement>(null);
  const [edge, setEdge] = useState(0);
  // Device pixels, both axes. Fit-to-window only needs the longest edge, but a
  // zoomed view is drawing into the whole area and needs its actual shape.
  const [canvas, setCanvas] = useState({ width: 0, height: 0, left: 0, top: 0 });
  useLayoutEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    const measure = () => {
      const { width, height, left, top } = el.getBoundingClientRect();
      const dpr = window.devicePixelRatio;
      setCanvas({ width: width * dpr, height: height * dpr, left: left * dpr, top: top * dpr });
      const longest = Math.max(width, height) * dpr;
      // Render up to the canvas's real device size so a fit view isn't upscaled
      // (and softened) on a high-DPI display — the "blurry after zooming out"
      // report. Capped so a 5K/6K panel can't ask for an absurd JPEG each frame;
      // the encode cost scales with pixel count, so this is the quality/latency knob.
      setEdge(Math.min(3840, Math.ceil(longest / 100) * 100));
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    window.addEventListener("resize", measure);
    measure();
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);

  /**
   * Zoom and pan, deliberately *not* in the edit stack.
   *
   * `zoom` is output pixels per image pixel, so 1 is one-to-one — the only
   * magnification at which per-pixel work like sharpening can be judged. `null`
   * means fit-to-window.
   */
  const [zoom, setZoom] = useState<number | null>(null);
  const [pan, setPan] = useState({ x: 0.5, y: 0.5 });
  // Press-and-hold the photo to peek the unedited "before" (fit view only, where
  // there's no pan to conflict with) — a quick compare on top of the toolbar's
  // edit/split/before switch.
  const [peeking, setPeeking] = useState(false);

  /** Clamps against this photo and this canvas. See `lib/viewport.ts`. */
  const clampPan = (p: { x: number; y: number }, z: number) =>
    develop && canvas.width ? clampCentre(p, z, develop, canvas) : p;

  /**
   * Enters or leaves crop mode, snapping back to fit-to-window on the way in.
   *
   * Crop mode shows the whole uncropped frame and the overlay is positioned
   * against that fit-to-window letterbox — so a leftover zoom would render a
   * magnified region the overlay's geometry no longer matches, which is exactly
   * why the crop rectangle only covered part of a zoomed image.
   */
  const setCropMode = (on: boolean) => {
    setCropping(on);
    // Fit on the way out as well: a zoom set while the tool drew at fit
    // used to land the moment Done was pressed.
    setZoom(null);
    setPan({ x: 0.5, y: 0.5 });
    if (on) {
      setMasking(false);
      setSpotting(false);
    }
  };

  /** Enters/leaves mask mode. Starts at fit-to-window (a clean overview), but
   *  the mask overlay tracks the viewport, so zoom and pan work while masking —
   *  wheel to zoom, middle-drag to pan. */
  const setMaskMode = (on: boolean) => {
    setMasking(on);
    if (on) {
      setCropping(false);
      setSpotting(false);
      setZoom(null);
      setPan({ x: 0.5, y: 0.5 });
    }
  };

  /** Enters/leaves spot mode, fit-to-window like the others so the spot handles
   *  sit on the framed image. */
  const setSpotMode = (on: boolean) => {
    setSpotting(on);
    setZoom(null);
    setPan({ x: 0.5, y: 0.5 });
    setEraseSel(NO_SELECTION);
    setEraseError(null);
    if (on) {
      setCropping(false);
      setMasking(false);
    }
  };

  /** Switches Retouch between spots and erasing. Spots draw at fit; erasing
   *  follows the view, since an object is often best selected zoomed in. */
  const setRetouchMode = (mode: "spots" | "erase") => {
    setRetouch(mode);
    setEraseSel(NO_SELECTION);
    setEraseError(null);
    if (mode === "spots") {
      setZoom(null);
      setPan({ x: 0.5, y: 0.5 });
    }
  };

  /** Crop and spot removal draw the frame at fit, whatever the zoom. */
  const canvasOwned = cropping || (spotting && !erasing);
  /** Which tool owns the rail and the canvas; Adjust is the one with none on. */
  const tool = cropping ? "crop" : masking ? "mask" : spotting ? "retouch" : "adjust";
  const leaveTools = () => {
    if (cropping) setCropMode(false);
    if (masking) setMaskMode(false);
    if (spotting) setSpotMode(false);
  };

  const view =
    zoom !== null && canvas.width > 0
      ? { zoom, cx: pan.x, cy: pan.y, width: canvas.width, height: canvas.height }
      : null;

  const pannable =
    view && develop && canvas.width ? canPan(view.zoom, develop, canvas) : { x: false, y: false };

  const { dragging, handlers: panHandlers } = usePanDrag(
    !cropping && (pannable.x || pannable.y),
    (dx, dy) => {
      if (!view || !develop) return;
      setPan((p) => panBy(p, dx, dy, window.devicePixelRatio, view.zoom, develop, canvas));
    },
  );

  // Before/after presentation. Edit shows the current edit; Before shows the
  // unedited render; Split shows both, divided down the middle.
  const [baMode, setBaMode] = useState<"edit" | "split" | "before">("edit");
  const showBefore = baMode === "before";
  // Soft-proof (gamut check): mark colours the output profile (sRGB) can't hold,
  // so out-of-gamut areas are visible before export. A viewing aid on the live
  // edit only — the "before" render is never proofed.
  const [softProof, setSoftProof] = useState(false);
  // Crop and spot render fit-to-window: their overlays are letterbox-only and a
  // zoom would magnify a region they do not track. Masking *does* track the
  // viewport (the mask overlay measures the zoomed box), so it renders the live
  // view like normal develop.
  const frameView = canvasOwned ? null : view;

  // AI noise reduction changes the image without a command this view made:
  // its result so far while it runs, and an export that runs it. A look
  // only needs a new frame drawn, and must not touch the edit, or a slider
  // held mid-drag would snap back; an export's result is a new state.
  const [liveFrame, setLiveFrame] = useState<{ photoId: number; frame: number } | null>(null);
  useEffect(() => {
    const unLook = listen<{ photoId: number; frame: number }>("denoise-preview", (e) => setLiveFrame(e.payload));
    const unUpdated = listen<DevelopInfo>("develop-updated", (e) => setDevelop(e.payload));
    return () => {
      void unLook.then((f) => f());
      void unUpdated.then((f) => f());
    };
  }, [setDevelop]);
  // Frame numbers never repeat and only grow, so the larger is the newest.
  const frame = develop
    ? Math.max(develop.frame, liveFrame?.photoId === develop.photoId ? liveFrame.frame : 0)
    : 0;
  const beforeSrc = develop ? developUrl(frame, true, edge, cropping, frameView) : "";

  // Hold-to-peek is offered in the plain edit view; the tool modes own the
  // canvas. Zoomed in, a press is also the start of a pan, so the peek waits
  // for the pointer to hold still (peekTimer) and a move cancels it.
  const peekable = baMode === "edit" && !cropping && !masking && !spotting;
  const peekTimer = useRef<number | undefined>(undefined);
  const endPeek = () => {
    window.clearTimeout(peekTimer.current);
    setPeeking(false);
  };

  // The direct surface (lib/directCanvas): the photo drawn straight onto the
  // GPU beneath a transparent page, where the platform has it — the split
  // view too, both halves in one frame.
  const directUp = useDirectSurface(useStore((s) => s.directCanvas));
  // Only over a photo: Develop can be up before one has loaded, and a
  // transparent page with nothing drawn beneath it is an empty window.
  const direct = directUp && develop !== null;
  useTransparentPage(direct);

  // The readback preview follows a drag too: the edit being dragged
  // is handed over and the URL asks for it, so the photo moves with the
  // slider rather than on release. The surface has its own way to the draft.
  const draftToken = useDraftToken(
    !direct && develop && draft !== committedDraft.current ? draft : null,
  );

  // What the selected mask covers, tinted by the render — every kind of
  // component, combined as the mask combines them. Range and colour
  // masks showed nothing until an adjustment moved.
  const shownMask = masking && maskOverlay && draft.masks?.[selectedMask] ? selectedMask : null;

  /**
   * One preview render in flight at a time, always converging on the newest.
   * See `useLatestFrame`.
   */
  const targetSrc = develop
    ? developUrl(
        frame,
        showBefore,
        edge,
        cropping,
        frameView,
        softProof && !showBefore,
        showBefore ? null : draftToken,
        showBefore ? null : shownMask,
      )
    : "";

  const [shownSrc, frameSettled] = useLatestFrame(direct ? "" : targetSrc);
  // No image to load while the surface draws: release whatever frame was in
  // flight when it took over, or the images would be stuck when it hands back.
  useEffect(() => {
    if (direct) frameSettled();
  }, [direct, shownSrc, frameSettled]);

  // Remember the viewport each preview URL encodes, so when a frame finally
  // lands the mask overlay can move to *that* frame's view rather than the
  // latest requested one. A render is throttled and (in a debug build) slow, so
  // without this the overlay races ahead of the image under the cursor — the
  // "mask moves but the photo doesn't" report. Tracking the shown frame's view
  // keeps shape and photo glued together, whatever the render latency.
  const viewByUrl = useRef(new Map<string, ViewPort | null>());
  if (targetSrc) {
    viewByUrl.current.set(targetSrc, frameView);
    if (viewByUrl.current.size > 24) {
      const oldest = viewByUrl.current.keys().next().value;
      if (oldest !== undefined) viewByUrl.current.delete(oldest);
    }
  }
  const [shownView, setShownView] = useState<ViewPort | null>(null);
  const onFrameSettled = useCallback(() => {
    setShownView(viewByUrl.current.get(shownSrc) ?? null);
    frameSettled();
  }, [shownSrc, frameSettled]);

  useDirectFrames(
    direct && develop && canvas.width > 0
      ? {
          x: Math.round(canvas.left),
          y: Math.round(canvas.top),
          width: Math.round(canvas.width),
          height: Math.round(canvas.height),
          zoom: frameView?.zoom ?? null,
          cx: frameView?.cx ?? 0.5,
          cy: frameView?.cy ?? 0.5,
          before: peeking || showBefore,
          uncropped: cropping,
          showMask: showBefore || peeking ? null : shownMask,
          softProof: softProof && !showBefore && !peeking,
          split: baMode === "split",
          // Mid-drag the edit lives only here; the surface can show it at
          // once, before the release commits it.
          draft: draft === committedDraft.current ? null : draft,
        }
      : null,
    develop?.frame ?? 0,
    // As with the images: the mask overlay follows the frame on screen, not
    // the one asked for.
    (f) =>
      setShownView(
        f.zoom === null
          ? null
          : { zoom: f.zoom, cx: f.cx ?? 0.5, cy: f.cy ?? 0.5, width: f.width, height: f.height },
      ),
  );

  /** Zooming also re-clamps the pan, since zooming out narrows the legal range. */
  const zoomBy = (factor: number) => {
    const f = fitZoom();
    const next = (zoom ?? f) * factor;
    // Fit-to-window is the floor. Below it the photo renders smaller than the
    // canvas, and softer with every step (the view path skips the sharp `edge`
    // proxy), so snap back to the crisp null-view fit path instead.
    if (next <= f * 1.02) {
      if (zoom !== null) {
        setZoom(null);
        setPan({ x: 0.5, y: 0.5 });
      }
      return;
    }
    const clamped = Math.min(8, next);
    setZoom(clamped);
    setPan((p) => clampPan(p, clamped));
  };

  /** The magnification fit-to-window is currently showing. */
  const fitZoom = () => {
    if (!develop || !canvas.width) return 1;
    return Math.min(canvas.width / develop.width, canvas.height / develop.height);
  };

  // Mask/pick overlays cover the canvas, so the image's own wheel-zoom and
  // pan-drag never fire while masking. These let the overlay drive them: wheel
  // zooms, middle-drag pans (left drag stays for painting and handles).
  // A notch zooms a step, a pinch smoothly, a touchpad scroll pans: the three
  // kinds of wheel (lib/wheel.ts). Zooming a step on every event made a
  // two-finger scroll over the photo zoom it wildly, on a Mac as on Windows.
  const overlayZoom = (e: React.WheelEvent) => {
    const intent = wheelIntent(e, pannable.x || pannable.y);
    if (intent.kind === "none") return;
    e.preventDefault();
    if (intent.kind === "pan") overlayPan(-intent.dx, -intent.dy);
    else zoomBy(intent.kind === "smooth" ? intent.factor : intent.zoomIn ? 1.15 : 1 / 1.15);
  };
  const overlayPan = (dx: number, dy: number) => {
    if (!view || !develop) return;
    setPan((p) => panBy(p, dx, dy, window.devicePixelRatio, view.zoom, develop, canvas));
  };

  useEffect(() => {
    if (!develop) return;
    const committed = withStackDefaults(develop.stack);
    committedDraft.current = committed;
    writeDraft(committed);
  }, [develop]);

  // Latest handlers + toggle state for the keydown listener, so it binds once
  // and never fires a stale closure (crop/mask toggles read the current mode).
  /**
   * Keeps this edit as a copy, and puts the photo it came from back to how it
   * was when it was opened. Develop then carries on editing the copy.
   *
   * Asks for a name because a library of "Copy 1" is no better than no copies
   * at all, but takes an empty answer — the badge and the position beside the
   * original already say what it is.
   */
  const saveAsCopy = () => askToSaveAsCopy((name) => useStore.getState().saveDevelopAsCopy(name));

  // The picker works on the framed photo, so it leaves the tools that show
  // something else (crop's uncropped frame, masks, spots) first.
  const toggleWbPicker = (on: boolean) => {
    if (on) {
      if (cropping) setCropMode(false);
      if (masking) setMaskMode(false);
      if (spotting) setSpotMode(false);
    }
    setWbPicking(on);
  };

  // The crop tool's guide, chosen with G.
  const cropGuide = useStore((s) => s.viewAids.develop.grid);

  // Zoom and pan from the keyboard: Z for 1:1 and back, = and − a
  // step, and the arrows pan while zoomed in (at fit they change photo).
  const toggleZoom = () => {
    if (canvasOwned) return;
    if (zoom === null) {
      setZoom(1);
      setPan({ x: 0.5, y: 0.5 });
    } else setZoom(null);
  };
  const keyZoom = (factor: number) => {
    if (!canvasOwned) zoomBy(factor);
  };
  const keyPan = (key: string) => {
    // A zoom with nowhere to pan leaves the arrows to change photo.
    if (!view || canvasOwned || !(pannable.x || pannable.y)) return false;
    const step = (Math.max(canvas.width, canvas.height) / window.devicePixelRatio) * 0.2;
    const move: Record<string, [number, number]> = { ArrowLeft: [step, 0], ArrowRight: [-step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
    const [dx, dy] = move[key] ?? [0, 0];
    if (!dx && !dy) return false;
    overlayPan(dx, dy);
    return true;
  };

  const kb = useRef({ close, undo, redo, setBaMode, setCropMode, setMaskMode, setSpotMode, reset, cropping, masking, spotting, picking, endPicking: () => {}, clearErase: (): boolean => false, copySettings, pasteSettings, verdict, step, wbPicking, toggleWbPicker, showHistory, setShowHistory, toggleZoom, keyZoom, keyPan });
  kb.current = { close, undo, redo, setBaMode, setCropMode, setMaskMode, setSpotMode, reset, cropping, masking, spotting, picking, endPicking: () => {}, clearErase: (): boolean => false, copySettings, pasteSettings, verdict, step, wbPicking, toggleWbPicker, showHistory, setShowHistory, toggleZoom, keyZoom, keyPan };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A dialog, menu or panel on top owns the keyboard: Escape on
      // Export closed Develop under it, and → changed the photo it exported.
      if (overlayOpen()) return;
      // A value field owns its own keys (Enter/Esc/arrows); don't let "r"/"m"
      // etc. trigger tools while the user is typing a number. A slider is not
      // a value field: it kept focus after a drag and every key went dead.
      // It keeps its own arrows, when it was reached from the keyboard.
      if (isTextEntry(e.target)) return;
      if (e.target instanceof HTMLInputElement && e.target.type === "range" && ARROWS.has(e.key)) return;
      const c = kb.current;
      // The History list takes the arrows to move through its steps; they
      // used to change the photo under it as well.
      if (c.showHistory && ARROWS.has(e.key)) return;
      // Zoomed in, the arrows look around the photo rather than leave it.
      if (!e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && c.keyPan(e.key)) {
        e.preventDefault();
        return;
      }
      const action =
        resolveAction(useStore.getState().keymap, "develop", e) ??
        // + is = shifted on some layouts and a key of its own on others.
        (e.key === "+" && !e.ctrlKey && !e.metaKey ? "develop-zoom-in" : null);
      if (!action) return;
      switch (action) {
        case "toggle-zoom": c.toggleZoom(); break;
        case "develop-zoom-in": c.keyZoom(1.5); break;
        case "develop-zoom-out": c.keyZoom(1 / 1.5); break;
        // Esc closes the innermost thing: the history list, then the
        // picker, then the tool in use, and only then Develop.
        case "close-overlay":
          if (c.showHistory) c.setShowHistory(false);
          else if (c.wbPicking) c.toggleWbPicker(false);
          else if (c.picking) c.endPicking();
          else if (c.cropping) c.setCropMode(false);
          else if (c.masking) c.setMaskMode(false);
          else if (c.clearErase()) break;
          else if (c.spotting) c.setSpotMode(false);
          else void c.close();
          break;
        case "develop-wb-picker": c.toggleWbPicker(!c.wbPicking); break;
        case "cycle-grid": useStore.getState().cycleGrid("develop"); break;
        case "develop-before-after": c.setBaMode((m) => (m === "before" ? "edit" : "before")); break;
        case "develop-undo": void c.undo(); break;
        case "develop-redo": void c.redo(); break;
        // Nothing to do — every edit is already in the catalogue — but
        // Ctrl+S is muscle memory, so answer it rather than ignore it.
        case "develop-save": toast(t("develop.saved"), "success"); break;
        case "develop-crop": c.setCropMode(!c.cropping); break;
        case "develop-mask": c.setMaskMode(!c.masking); break;
        case "develop-reset": void c.reset(); break;
        case "develop-copy-settings": void c.copySettings(); break;
        case "develop-paste-settings": void c.pasteSettings(); break;
        // The photo as edited so far: every edit is already saved.
        case "copy-image": {
          const s = useStore.getState();
          if (textIsSelected() || !s.develop) return;
          void s.copyImage(s.develop.photoId);
          break;
        }
        case "prev-photo": c.step(-1); break;
        case "next-photo": c.step(1); break;
        case "flag-pick": c.verdict({ flag: 1 }); break;
        case "flag-reject": c.verdict({ flag: -1 }); break;
        case "flag-none": c.verdict({ flag: 0 }); break;
        case "label-none": c.verdict({ colorLabel: 0 }); break;
        default:
          if (action.startsWith("rate-")) c.verdict({ rating: Number(action.slice(5)) });
          else if (action.startsWith("label-")) c.verdict({ colorLabel: Number(action.slice(6)) });
          else return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Collapsible group state, with an optional solo (one-open) mode. Both are
  // remembered: the groups someone works in are the ones they want open next
  // time, and a rail that springs back to the defaults undoes that every visit.
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>(storedOpen);
  const [solo, setSolo] = useState(() => localStorage.getItem(SOLO_KEY) === "1");
  useEffect(() => {
    localStorage.setItem(OPEN_KEY, JSON.stringify(openGroups));
  }, [openGroups]);
  useEffect(() => {
    localStorage.setItem(SOLO_KEY, solo ? "1" : "0");
  }, [solo]);
  const isOpen = (id: string) => !!openGroups[id];
  const toggle = (id: string) =>
    setOpenGroups((prev) => {
      if (solo) return prev[id] ? {} : { [id]: true };
      return { ...prev, [id]: !prev[id] };
    });
  const toggleSolo = () =>
    setSolo((on) => {
      if (!on) {
        // Entering solo: keep only the first open group.
        const first = Object.keys(openGroups).find((k) => openGroups[k]);
        setOpenGroups(first ? { [first]: true } : {});
      }
      return !on;
    });

  // Presets is its own panel on the left of the canvas, not a tab sharing the
  // right rail with the adjustment groups: a preset and the sliders it moved
  // are things you want to see at the same time.
  //
  // Closed whenever Develop opens, and not remembered past it: opening onto a
  // gallery crowded the photo you came to edit (and at the minimum window left
  // it a third of the width). It stays as you set it while you step through
  // photos, since Develop stays mounted for that.
  const [showPresets, setShowPresets] = useState(false);
  const [panelTab, setPanelTab] = useState<"presets" | "luts">("presets");

  // Render what the showing tab needs, once per photo: the suggestions and the
  // saved presets' previews for Presets (built-in groups ask as they open),
  // the LUT previews for LUTs.
  useEffect(() => {
    const pid = develop?.photoId;
    if (!showPresets || pid == null) return;
    if (panelTab === "presets" && previewsFor.current.presets !== pid) {
      previewsFor.current.presets = pid;
      void fetchSuggestions();
      void loadPresetPreviews();
    }
    if (panelTab === "luts" && previewsFor.current.luts !== pid) {
      previewsFor.current.luts = pid;
      void loadLutPreviews();
    }
  }, [showPresets, panelTab, develop?.photoId, fetchSuggestions, loadPresetPreviews, loadLutPreviews]);

  // Right editing rail is user-resizable (a wider panel gives the colour wheels
  // and sliders more room). Width persists across sessions; the drag mutates the
  // DOM directly and only commits to state on release, so the heavy adjustment
  // tree doesn't re-render on every pointer move.
  const railRef = useRef<HTMLElement>(null);
  const [railW, setRailW] = useState<number>(savedRailWidth);
  useEffect(() => {
    localStorage.setItem("developRailW", String(railW));
  }, [railW]);
  // The panels as wide as asked, but never so wide the photo goes: at
  // least CANVAS_MIN of canvas, Presets giving way first, then the rail, down to
  // their minimums. Their widths stay remembered; this is only how they draw.
  const winW = useWindowWidth();
  const railFit = fitRail(railW, winW, showPresets);
  const presetsFit = Math.max(PRESETS_MIN, winW - CANVAS_MIN - railFit);

  const startRailResize = (e: React.PointerEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = railRef.current?.offsetWidth ?? railW;
    const clamp = (w: number) => Math.min(RAIL_MAX, Math.max(RAIL_MIN, w));
    const onMove = (ev: PointerEvent) => {
      // Rail is on the right, so dragging its left edge leftward widens it.
      if (railRef.current) railRef.current.style.width = `${clamp(startW + (startX - ev.clientX))}px`;
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      setRailW(railRef.current?.offsetWidth ?? railW);
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  // Develop takes the keyboard when it opens, and gives it back when it
  // closes. Opened with D from the grid, focus stayed on the grid cell under
  // it, whose own arrow-key handling moved the library's selection behind
  // Develop instead of stepping to the next photo in it.
  const shellRef = useRef<HTMLDivElement>(null);
  const shown = develop !== null;
  useEffect(() => {
    if (!shown) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const shell = shellRef.current;
    if (shell && !shell.contains(opener)) shell.focus({ preventScroll: true });
    return () => {
      if (opener && opener !== document.body && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, [shown]);

  if (!develop) return null;

  const set = (patch: Partial<Stack>) => {
    writeDraft({ ...draftRef.current, ...patch });
  };
  const commit = (label: string) => void adjust(label, draftRef.current);
  /** Set locally *and* commit in one step — for resets and discrete controls. */
  const apply = (patch: Partial<Stack>, label: string) => {
    const next = { ...draftRef.current, ...patch };
    writeDraft(next);
    void adjust(label, next);
  };

  // Local-adjustment masks live in the stack like any other control.
  const masks = draft.masks;
  const addMask = (m: Mask) => {
    // The masks as they are now, not as they were when this render ran: an
    // AI mask arrives seconds or minutes after its click.
    const next = [...draftRef.current.masks, m];
    apply({ masks: next }, t("develop.history.addMask"));
    setSelectedMask(next.length - 1);
    setSelectedComp(0);
  };
  const removeMask = (i: number) => {
    apply({ masks: masks.filter((_, j) => j !== i) }, t("develop.history.deleteMask"));
    setSelectedMask((s) => Math.max(0, s > i ? s - 1 : s));
    setSelectedComp(0);
  };
  /** Replaces mask `i`; commits to history only on `commit` (drag end). */
  const editMask = (i: number, m: Mask, label: string, commit: boolean) => {
    const next = masks.map((x, j) => (j === i ? m : x));
    if (commit) apply({ masks: next }, label);
    else set({ masks: next });
  };
  /** Sets a mask's user-facing name (empty clears it back to the derived label). */
  const renameMask = (i: number, name: string) => {
    const m = masks[i];
    if (!m) return;
    const trimmed = name.trim();
    editMask(i, { ...m, name: trimmed === "" ? null : trimmed }, t("develop.history.renameMask"), true);
  };
  /** Display name: the user's, else the AI subject's own label, else a type + index. */
  // Captured once so `noUncheckedIndexedAccess` narrows it after a truthy check.
  const selMask = masks[selectedMask];
  const comps = selMask ? maskComponents(selMask) : [];
  const selComp = comps[selectedComp];

  /** Replaces component `i` of the selected mask. */
  const editComponent = (i: number, c: MaskComponent, label: string, commitIt: boolean) => {
    if (!selMask) return;
    const next = maskComponents(selMask).map((x, j) => (j === i ? c : x));
    editMask(selectedMask, { ...selMask, components: next }, label, commitIt);
  };
  /** Appends a component of `kind` (combining by Add) and selects it. */
  const addComponent = (kind: CreatableKind) => {
    if (!selMask) return;
    const next = [...maskComponents(selMask), newComponent(kind, "add")];
    editMask(selectedMask, { ...selMask, components: next }, t("develop.history.addComponent"), true);
    setSelectedComp(next.length - 1);
  };
  /** Removes component `i`; removing the last component removes the mask. */
  const removeComponent = (i: number) => {
    if (!selMask) return;
    const current = maskComponents(selMask);
    if (current.length <= 1) {
      removeMask(selectedMask);
      return;
    }
    const next = current.filter((_, j) => j !== i);
    editMask(selectedMask, { ...selMask, components: next }, t("develop.history.deleteComponent"), true);
    setSelectedComp((s) => Math.min(s > i ? s - 1 : s, next.length - 1));
  };
  const setCompMode = (i: number, mode: MaskMode) => {
    const c = comps[i];
    if (c) editComponent(i, { ...c, mode }, t("develop.history.combineMode"), true);
  };
  const toggleCompInvert = (i: number) => {
    const c = comps[i];
    if (c) editComponent(i, { ...c, inverted: !c.inverted }, t("develop.history.invertComponent"), true);
  };

  /** Starts (or ends) click-to-select. Starting on a mask picked before
   *  carries on from its points; on any other mask it starts a new one. */
  const setPickMode = (on: boolean) => {
    const points = on ? pickedPoints.current.get(aiRle(masks[selectedMask])) : undefined;
    setPicking(on);
    setPickPoints(points ?? []);
    setPickRedo([]);
    pickMask.current = points ? selectedMask : null;
    setAiError(null);
  };

  /** Whether the history step `offset` from the applied one is a click's. */
  const isPickStep = (offset: 0 | 1) => {
    const d = useStore.getState().develop;
    const label = d?.history[d.historyAt + offset];
    return label === t("develop.history.addMask") || label === t("develop.history.refineSelection");
  };
  /** Undo while picking takes back the last click with its step. */
  const undoPick = async () => {
    const last = pickPoints.at(-1);
    if (!picking || aiBusy || !last || !isPickStep(0)) return undo();
    await undo();
    const next = pickPoints.slice(0, -1);
    setPickPoints(next);
    setPickRedo((r) => [...r, last]);
    const m = useStore.getState().develop?.stack.masks?.[pickMask.current ?? -1];
    if (next.length && m) pickedPoints.current.set(aiRle(m), next);
  };
  const redoPick = async () => {
    const point = pickRedo.at(-1);
    if (!picking || aiBusy || !point || !isPickStep(1)) return redo();
    await redo();
    const masksNow = useStore.getState().develop?.stack.masks ?? [];
    // Redoing the first click adds the mask back, at the end.
    if (pickPoints.length === 0) pickMask.current = masksNow.length - 1;
    const next = [...pickPoints, point];
    setPickPoints(next);
    setPickRedo((r) => r.slice(0, -1));
    const m = masksNow[pickMask.current ?? -1];
    if (m) pickedPoints.current.set(aiRle(m), next);
  };
  kb.current.undo = undoPick;
  kb.current.redo = redoPick;
  // Escape's way out of click-to-select, declared after the key
  // handler's table, so put into it here.
  kb.current.endPicking = () => setPickMode(false);

  /**
   * Adds a click and re-runs the selection. The first click creates the mask;
   * each later one refines the same mask in place, so the user sees the
   * selection tighten rather than accumulating masks.
   */
  const addPickPoint = async (p: SelectPoint) => {
    // Only the first click of a selection has to check — once a mask exists the
    // model is plainly loaded, and asking again mid-refine would put a dialog
    // between someone and the point they just clicked.
    if (aiBusy) return;
    // Busy from the click: the model can take minutes to download,
    // and the overlay looked idle meanwhile.
    setAiBusy(true);
    setAiStage(null);
    setAiError(null);
    const photo = useStore.getState().develop?.photoId;
    const stillHere = () => useStore.getState().develop?.photoId === photo;
    try {
      const first = pickPoints.length === 0;
      if (first && !(await ensureAi("clickSelect", { inline: true }))) return;
      if (!stillHere()) return;
      const next = [...pickPoints, p];
      setPickPoints(next);
      setPickRedo([]);
      const mask = await api.aiSelectPoints(next);
      // Another photo opened meanwhile: the selection was this one's.
      if (!stillHere()) return;
      pickedPoints.current.set(aiRle(mask), next);
      if (first) {
        pickMask.current = draftRef.current.masks.length;
        addMask(mask);
      } else if (pickMask.current !== null) {
        editMask(pickMask.current, mask, t("develop.history.refineSelection"), true);
      }
    } catch (e) {
      setAiError(String(e));
    } finally {
      setAiBusy(false);
    }
  };

  /** Runs AI subject segmentation and adds the result as a mask. */
  const runAiSubject = async () => {
    // Develop was the one workspace `ensureAi` never reached, so with no model
    // downloaded these buttons were enabled, failed, and rendered a raw
    // filesystem path as red 10px text.
    if (aiBusy) return;
    // Busy from the click, not from after the download: a second
    // click used to fail with "already downloading".
    setAiBusy(true);
    setAiStage(null);
    setAiError(null);
    const photo = useStore.getState().develop?.photoId;
    const stillHere = () => useStore.getState().develop?.photoId === photo;
    try {
      if (!(await ensureAi("selectSubject", { inline: true })) || !stillHere()) return;
      const mask = await api.aiSelectSubject();
      // Another photo opened meanwhile: its masks are its own, and
      // this subject is the last photo's.
      if (stillHere()) addMask(mask);
    } catch (e) {
      setAiError(String(e));
    } finally {
      setAiBusy(false);
    }
  };

  // Spot / heal / clone patches live in the stack alongside masks.
  const spots = draft.spots;
  const addSpot = (x: number, y: number) => {
    const next = [...spots, newSpot(x, y)];
    apply({ spots: next }, t("develop.history.addSpot"));
    setSelectedSpot(next.length - 1);
  };
  const removeSpot = (i: number) => {
    apply({ spots: spots.filter((_, j) => j !== i) }, t("develop.history.deleteSpot"));
    setSelectedSpot((s) => Math.max(0, s > i ? s - 1 : s));
  };
  const editSpot = (i: number, s: Spot, label: string, commitIt: boolean) => {
    const next = spots.map((x, j) => (j === i ? s : x));
    if (commitIt) apply({ spots: next }, label);
    else set({ spots: next });
  };

  // Magic erase. The selection is Develop's until Erase is pressed; the
  // erases are the backend's, which alone adds and removes them.
  const erases = draft.erases;
  const eraseRunning = eraseStarting || !!eraseJob;
  /** Adds a click to the selection and asks SAM 2 for it again, as
   *  click-to-select does for a mask. */
  const addErasePoint = async (p: SelectPoint) => {
    if (erasePicking || eraseRunning) return;
    setErasePicking(true);
    setEraseError(null);
    const photo = useStore.getState().develop?.photoId;
    const stillHere = () => useStore.getState().develop?.photoId === photo;
    try {
      if (eraseSel.points.length === 0 && !(await ensureAi("clickSelect", { inline: true }))) return;
      if (!stillHere()) return;
      const points = [...eraseSel.points, p];
      setEraseSel((s) => ({ ...s, points }));
      const picked = await api.aiSelectPoints(points);
      if (stillHere()) setEraseSel((s) => ({ ...s, picked }));
    } catch (e) {
      setEraseError(String(e));
    } finally {
      setErasePicking(false);
    }
  };
  const runErase = async () => {
    const selection = selectionMask(eraseSel);
    if (!selection || eraseRunning) return;
    setEraseStarting(true);
    setEraseError(null);
    const photo = useStore.getState().develop?.photoId;
    const stillHere = () => useStore.getState().develop?.photoId === photo;
    try {
      // The first erase downloads the model, shown in the panel with Cancel.
      if (!(await ensureAi("erase", { inline: true })) || !stillHere()) return;
      const info = await api.eraseDevelop(selection, eraseSel.picked ? "Selection" : "Brushed");
      if (!stillHere()) return;
      setDevelop(info);
      setEraseSel(NO_SELECTION);
    } catch (e) {
      if (!isCancelled(e)) setEraseError(String(e));
    } finally {
      setEraseStarting(false);
    }
  };
  const removeErase = async (i: number) => {
    try {
      setDevelop(await api.removeErase(i));
    } catch (e) {
      setEraseError(String(e));
    }
  };
  // Escape's first press while erasing lets go of the selection.
  kb.current.clearErase = () => {
    if (!erasing || (!hasSelection(eraseSel) && eraseSel.points.length === 0)) return false;
    setEraseSel(NO_SELECTION);
    return true;
  };

  // Runs every render against every stack field. The arrays (masks with their
  // brush strokes, spots, curve points) default to empty, so a length check
  // answers "not default" without serialising a mask's strokes each time; the
  // stringify is left for the small objects, where it is cheap.
  const same = (a: unknown, b: unknown) => {
    if (a === b) return true;
    if (Array.isArray(a) && Array.isArray(b) && a.length !== b.length) return false;
    return a !== null && typeof a === "object" && JSON.stringify(a) === JSON.stringify(b);
  };
  const edited = (Object.keys(DEFAULT_STACK) as (keyof Stack)[]).some(
    (k) => k !== "version" && !same(draft[k], DEFAULT_STACK[k]),
  );

  // Per-group "modified" flags, for the header dots.
  const nz = (...v: number[]) => v.some((x) => x !== 0);
  const mod = {
    wb: nz(draft.warmth, draft.tint),
    light: nz(draft.contrast, draft.highlights, draft.shadows, draft.whites, draft.blacks, draft.exposure),
    curve:
      draft.curve.length > 0 ||
      draft.curveRed.length > 0 ||
      draft.curveGreen.length > 0 ||
      draft.curveBlue.length > 0,
    presence: nz(draft.detail.texture, draft.detail.clarity, draft.detail.dehaze, draft.vibrance, draft.saturation),
    mixer: bandsNz(draft.mixerHue) || bandsNz(draft.mixerSaturation) || bandsNz(draft.mixerLuminance),
    grading: !same(draft.grading, DEFAULT_STACK.grading),
    detail: nz(draft.detail.sharpen, draft.detail.sharpenDetail, draft.detail.sharpenMasking, draft.detail.noise, draft.detail.colorNoise, draft.detail.aiDenoise) || draft.detail.sharpenRadius !== 1,
    optics: nz(draft.lens.distortion, draft.lens.vignetting, draft.lens.chromatic),
    geometry: !same(draft.geometry, DEFAULT_STACK.geometry),
    effects: nz(draft.effects.vignette, draft.effects.grain, draft.effects.halation),
  };

  // The photo's pointer handling — wheel to zoom, drag to pan, hold to peek at
  // the before — on whichever element shows it: the image, or the empty box
  // over the direct surface.
  const photoPointer: React.HTMLAttributes<HTMLElement> = {
    style: {
      cursor: !(pannable.x || pannable.y) ? undefined : dragging ? "grabbing" : "grab",
    },
    onWheel: (e) => {
      if (cropping) return;
      overlayZoom(e);
    },
    ...panHandlers,
    onPointerDown: (e) => {
      panHandlers.onPointerDown(e);
      if (!peekable) return;
      e.currentTarget.setPointerCapture(e.pointerId);
      if (pannable.x || pannable.y) peekTimer.current = window.setTimeout(() => setPeeking(true), 250);
      else setPeeking(true);
    },
    onPointerMove: (e) => {
      panHandlers.onPointerMove(e);
      // A drag while zoomed is a pan, not a peek.
      if ((pannable.x || pannable.y) && (e.movementX || e.movementY)) endPeek();
    },
    onPointerUp: () => {
      panHandlers.onPointerUp();
      endPeek();
    },
    onPointerCancel: () => {
      panHandlers.onPointerCancel();
      endPeek();
    },
    onPointerLeave: endPeek,
  };

  return (
    <div ref={shellRef} tabIndex={-1} className="develop-shell fixed inset-0 z-50 flex flex-col bg-base outline-none">
      <DevelopHeader
        develop={develop}
        filename={filename}
        close={close}
        undo={undoPick}
        redo={redoPick}
        undoKey={undoKey}
        redoKey={redoKey}
        showPresets={showPresets}
        setShowPresets={setShowPresets}
        showHistory={showHistory}
        setShowHistory={setShowHistory}
        goToStep={goToStep}
        saveAsCopy={saveAsCopy}
        setShowExport={setShowExport}
        exporting={exporting}
      />

      <GroupsDialog ask={groupsAsk} />
      {error && (
        <div className="border-b border-danger/40 bg-danger/10 px-3 py-2 text-caption text-danger-ink">
          {error}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        {/* Presets and LUTs, on the opposite side of the canvas from the
            adjustments, so the two can be open at once. */}
        {showPresets && (
          <DevelopPresets
            maxWidth={presetsFit}
            groups={presetGroups}
            presetId={draft.preset.id}
            presetAmount={draft.preset.amount}
            onUseBuiltin={(id) => void useBuiltin(id)}
            onAmountChange={changeAmount}
            onAmountCommit={commitAmount}
            builtinPreviews={builtinPreviews}
            onWantPreviews={wantBuiltinPreviews}
            suggestions={suggestions}
            suggesting={suggesting}
            onRefreshSuggestions={() => void fetchSuggestions()}
            onApplySuggestion={(s) => void applySuggestion(s)}
            saved={presets}
            savedPreviews={presetPreviews}
            onUseSaved={(id) => void usePreset(id)}
            onRemoveSaved={(id) => void removePreset(id)}
            onStoreSaved={() => void storePreset()}
            luts={luts}
            lutName={draft.lut.name}
            lutIntensity={draft.lut.intensity}
            lutPreviews={lutPreviews}
            onChooseLut={chooseLut}
            onRemoveLut={(name) => void removeLut(name)}
            onImportLut={() => void importLut()}
            onGroupLut={(name, family, grouped) => void moveLutToGroup(name, family, grouped)}
            onUngroupLut={(name) => void ungroupLut(name)}
            onLutIntensityChange={(v) => set({ lut: { ...draftRef.current.lut, intensity: v } })}
            onLutIntensityCommit={() => commit(t("develop.history.lutIntensity"))}
            onTabShown={setPanelTab}
            aspect={develop.width / Math.max(1, develop.height)}
            onClose={() => setShowPresets(false)}
          />
        )}

        {/* Center: canvas, toolbar, filmstrip. */}
        <div className="flex min-w-0 flex-1 flex-col">
          <main
            ref={canvasRef}
            className="develop-canvas relative min-h-0 min-w-0 flex-1 overflow-hidden bg-canvas-void"
          >
            {baMode === "split" ? (
              <>
                {/* On the surface both halves are one frame beneath the
                    page; only the line and the labels are drawn here. */}
                {!direct && (
                  <>
                    <img
                      src={beforeSrc}
                      alt=""
                      draggable={false}
                      className="absolute inset-0 h-full w-full object-contain"
                      style={{ clipPath: "inset(0 50% 0 0)" }}
                    />
                    <img
                      src={shownSrc}
                      onLoad={onFrameSettled}
                      onError={onFrameSettled}
                      alt=""
                      draggable={false}
                      className="absolute inset-0 h-full w-full object-contain"
                      style={{ clipPath: "inset(0 0 0 50%)" }}
                    />
                  </>
                )}
                {/* Takes the pointer over both halves, so Split zooms and pans too. */}
                <div className="absolute inset-0 h-full w-full" {...photoPointer} />
                <div className="pointer-events-none absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-white/70" />
                <div className="pointer-events-none absolute bottom-3 left-1/2 flex -translate-x-1/2 gap-6 text-label font-semibold">
                  <span className="rounded-sm bg-chip px-2 py-0.5 text-white backdrop-blur">{t("develop.view.before")}</span>
                  <span className="rounded-sm bg-chip px-2 py-0.5 text-white backdrop-blur">{t("develop.view.after")}</span>
                </div>
              </>
            ) : (
              <>
                {direct ? (
                  // The photo is beneath the page; this box only takes the
                  // pointer, as the image does.
                  <div className="absolute inset-0 h-full w-full" {...photoPointer} />
                ) : (
                  <img
                    src={peeking ? beforeSrc : shownSrc}
                    onLoad={onFrameSettled}
                    onError={onFrameSettled}
                    alt=""
                    draggable={false}
                    className="absolute inset-0 h-full w-full object-contain"
                    {...photoPointer}
                  />
                )}
                {peeking && (
                  <div className="pointer-events-none absolute left-1/2 top-3 -translate-x-1/2 rounded-sm bg-chip px-2.5 py-0.5 text-label font-semibold text-white backdrop-blur">
                    {t("develop.view.before")}
                  </div>
                )}
              </>
            )}

            {cropping && (
              <CropOverlay
                grid={cropGuide}
                frameAspect={uncroppedAspect}
                levelling={straightening}
                // The box in the tool's turned, straightened view; the crop in
                // the image's own coordinates (lib/cropView).
                crop={toView(draft.geometry, orient)}
                ratio={aspect}
                onChange={(box) =>
                  set({ geometry: { ...draftRef.current.geometry, crop: fromView(box, draftRef.current.geometry, orient) } })
                }
                onCommit={(box) =>
                  void adjust(t("develop.history.crop"), {
                    ...draftRef.current,
                    geometry: { ...draftRef.current.geometry, crop: fromView(box, draftRef.current.geometry, orient) },
                  })
                }
              />
            )}

            {masking && selMask && selComp && (
              <MaskOverlay
                // Masks live in the cropped output's UV; the overlay places that
                // frame using the live view, so handles track zoom and pan.
                frameAspect={develop.width / develop.height}
                framePx={{ width: develop.width, height: develop.height }}
                view={shownView}
                component={selComp}
                brush={brush}
                onChange={(c) => editComponent(selectedComp, c, t("develop.history.mask"), false)}
                onCommit={(c) => editComponent(selectedComp, c, t("develop.history.paintMask"), true)}
                onZoom={overlayZoom}
                onPan={overlayPan}
              />
            )}

            {wbPicking && (
              <ClickSelectOverlay
                frameAspect={develop.width / develop.height}
                framePx={{ width: develop.width, height: develop.height }}
                view={shownView}
                points={[]}
                busy={false}
                onPick={(p) => {
                  setWbPicking(false);
                  void pickWb(p.x, p.y);
                }}
                onZoom={overlayZoom}
                onPan={overlayPan}
              />
            )}

            {masking && picking && (
              <ClickSelectOverlay
                frameAspect={develop.width / develop.height}
                framePx={{ width: develop.width, height: develop.height }}
                view={shownView}
                points={pickPoints}
                busy={aiBusy}
                onPick={(p) => void addPickPoint(p)}
                onZoom={overlayZoom}
                onPan={overlayPan}
              />
            )}

            {erasing && (
              <EraseSelectionOverlay
                frameAspect={develop.width / develop.height}
                framePx={{ width: develop.width, height: develop.height }}
                view={shownView}
                selection={eraseSel}
              />
            )}

            {erasing && eraseHow === "click" && (
              <ClickSelectOverlay
                frameAspect={develop.width / develop.height}
                framePx={{ width: develop.width, height: develop.height }}
                view={shownView}
                points={eraseSel.points}
                busy={erasePicking || eraseRunning}
                onPick={(p) => void addErasePoint(p)}
                onZoom={overlayZoom}
                onPan={overlayPan}
              />
            )}

            {erasing && eraseHow === "brush" && (
              <MaskOverlay
                frameAspect={develop.width / develop.height}
                framePx={{ width: develop.width, height: develop.height }}
                view={shownView}
                // The strokes being painted: onto the selection, or off it.
                component={{
                  shape: { type: "brush", strokes: eraseBrush.remove ? eraseSel.remove : eraseSel.add },
                  inverted: false,
                  mode: "add",
                }}
                // Hard-edged, as an object's outline is; dashed to take away.
                brush={{ size: eraseBrush.size, hardness: 0.8, erase: eraseBrush.remove }}
                onChange={(c) => {
                  if (c.shape.type !== "brush") return;
                  // Taking away is its own list, subtracted whole: a dab marked
                  // to erase would only rub out its own list.
                  const strokes = c.shape.strokes.map((s) => ({ ...s, erase: false }));
                  setEraseSel((s) => (eraseBrush.remove ? { ...s, remove: strokes } : { ...s, add: strokes }));
                }}
                onCommit={() => {}}
                onZoom={overlayZoom}
                onPan={overlayPan}
              />
            )}

            {/* What a click on the photo will do now: the picker, select
                and spot tools share one crosshair, and their hints sat in the
                rail, out of sight when a group was folded. */}
            {/* An AI mask taking a while says what it is doing here as well
                as in the rail, in place of the hint while picking. */}
            {opening && nextName !== null ? (
              <div role="status" className="loupe-chip pointer-events-none absolute left-1/2 top-3 z-20 flex max-w-[80%] -translate-x-1/2 items-center gap-2 px-2.5 py-1 text-label text-white/85">
                <Spinner />
                <span className="truncate">{t("develop.view.opening", { name: nextName })}</span>
              </div>
            ) : masking && aiBusy && aiSlow ? (
              <div role="status" className="loupe-chip pointer-events-none absolute left-1/2 top-3 z-20 flex -translate-x-1/2 items-center gap-2 px-2.5 py-1 text-label text-white/85">
                <Spinner />
                {download ?? aiStageText(aiStage, picking && pickPoints.length > 1)}
              </div>
            ) : (wbPicking || (masking && picking) || spotting) && (
              <div className="loupe-chip pointer-events-none absolute left-1/2 top-3 z-20 -translate-x-1/2 px-2.5 py-1 text-label text-white/85">
                {wbPicking
                  ? t("develop.mode.wb")
                  : masking && picking
                    ? t(pickPoints.length ? "develop.mode.pickRefine" : "develop.mode.pick")
                    : erasing
                      ? t(eraseHow === "click" ? "develop.mode.erasePick" : "develop.mode.eraseBrush")
                      : t("develop.mode.spot")}
              </div>
            )}

            {spotting && !erasing && (
              <SpotOverlay
                frameAspect={develop.width / develop.height}
                spots={spots}
                selected={selectedSpot}
                onSelect={(i) => setSelectedSpot(i)}
                onAdd={addSpot}
                onChange={(i, s) => editSpot(i, s, t("develop.history.spot"), false)}
                onCommit={(i, s) => editSpot(i, s, t("develop.history.moveSpot"), true)}
              />
            )}
          </main>

          <ViewBar
            baMode={baMode}
            setBaMode={setBaMode}
            cropping={cropping}
            softProof={softProof}
            setSoftProof={setSoftProof}
            canvasOwned={canvasOwned}
            zoom={zoom}
            setZoom={setZoom}
            setPan={setPan}
            zoomBy={zoomBy}
            fitZoom={fitZoom}
            zoomKey={zoomKey}
            zoomInKey={zoomInKey}
            zoomOutKey={zoomOutKey}
          />

          <Filmstrip />
        </div>

        {/* Drag handle to resize the editing panel — sits over the rail's left
            edge, widening the hit area without shifting layout. */}
        <div
          onPointerDown={startRailResize}
          // Back to the default width, as the Presets panel's edge does.
          onDoubleClick={() => {
            if (railRef.current) railRef.current.style.width = "";
            setRailW(RAIL_DEFAULT);
          }}
          role="separator"
          aria-orientation="vertical"
          aria-label={t("develop.view.resizeRailLabel")}
          title={t("develop.presets.resizeTitle")}
          className="group relative z-10 -mr-1 w-2 shrink-0 cursor-col-resize"
        >
          <div className="absolute inset-y-0 right-0 w-px bg-transparent transition-colors group-hover:bg-accent" />
        </div>

        {/* Right rail, top to bottom: the histogram (a reading of the photo),
            the tools (which decide what the rest of the rail is), the
            controls for the chosen tool, and at the foot the settings that
            travel between photos. */}
        <aside
          ref={railRef}
          style={{ width: railFit }}
          className="flex shrink-0 flex-col border-l border-line bg-panel"
        >
          <>
          <Histogram frame={develop.frame} draft={draft === committedDraft.current ? null : draft} />
          {/* A JPEG, PNG or TIFF: the camera has already rendered it,
              so blown highlights and blocked shadows are gone for good. */}
          {!develop.fromRaw && (
            <div
              className="border-b border-line px-3 py-1.5 text-label text-faint"
              title={t("develop.rail.notRawTitle")}
            >
              {t("develop.rail.notRaw")}
            </div>
          )}

          {/* The tools, as one switch: Adjust is a tool too, so it is always
              clear which one owns the rail and the canvas, and getting back
              from a tool is a click on the same row rather than a hunt for
              its Done. A tool that holds edits says so — a dot for a crop,
              a count for masks and spots. */}
          <div className="grid grid-cols-4 gap-1 border-b border-line p-2" role="group" aria-label={t("develop.rail.toolLabel")}>
            <ToolButton icon="adjust" label="develop.tool.adjust" active={tool === "adjust"} onClick={leaveTools} title={t("develop.tool.adjustTitle")} />
            <ToolButton
              icon="crop"
              label="develop.tool.crop"
              active={cropping}
              onClick={() => setCropMode(!cropping)}
              title={hint(t("develop.tool.cropTitle"), cropKey)}
              marker={mod.geometry}
            />
            <ToolButton
              icon="mask"
              label="develop.tool.mask"
              active={masking}
              onClick={() => setMaskMode(!masking)}
              title={hint(t("develop.tool.maskTitle"), maskKey)}
              marker={masks.length || false}
            />
            <ToolButton
              icon="heal"
              label="develop.tool.retouch"
              active={spotting}
              onClick={() => setSpotMode(!spotting)}
              title={t("develop.tool.retouchTitle")}
              marker={spots.length + erases.length || false}
            />
          </div>

          {/* Whole-edit controls, above the groups they act on. */}
          {tool === "adjust" && (
            <div className="flex items-center gap-1 border-b border-line px-2 py-1.5">
              <button
                onClick={() => void toggleAuto()}
                className={`btn-ghost gap-1.5 px-2 py-1 ${autoOn ? "seg-on" : "text-muted hover:text-ink"}`}
                aria-pressed={autoOn}
                title={autoOn ? t("develop.auto.onTitle") : t("develop.auto.title")}
              >
                <Icon name="adjust" size={14} />
                {t("develop.auto.label")}
              </button>
              <div className="flex-1" />
              <button
                onClick={() => void reset()}
                disabled={!edited}
                className="btn-ghost px-2 py-1 text-muted hover:text-ink disabled:pointer-events-none disabled:opacity-40"
                title={hint(t("develop.rail.resetAllTitle"), resetKey)}
              >
                {t("develop.rail.resetAll")}
              </button>
              <button
                onClick={toggleSolo}
                aria-pressed={solo}
                className={`icon-chip h-7 w-7 ${solo ? "seg-on" : ""}`}
                title={solo ? t("develop.rail.soloOnTitle") : t("develop.rail.soloTitle")}
              >
                <Icon name="solo" size={15} />
              </button>
            </div>
          )}

          <div className="min-h-0 flex-1 overflow-y-auto">
            {masking && (
              <MaskPanel
                masks={masks}
                selectedMask={selectedMask}
                setSelectedMask={setSelectedMask}
                selectedComp={selectedComp}
                setSelectedComp={setSelectedComp}
                renamingMask={renamingMask}
                setRenamingMask={setRenamingMask}
                brush={brush}
                setBrush={setBrush}
                maskOverlay={maskOverlay}
                setMaskOverlay={setMaskOverlay}
                picking={picking}
                pickPoints={pickPoints}
                setPickMode={setPickMode}
                aiBusy={aiBusy}
                aiStage={aiStage}
                aiError={aiError}
                runAiSubject={runAiSubject}
                setMaskMode={setMaskMode}
                addMask={addMask}
                removeMask={removeMask}
                renameMask={renameMask}
                editMask={editMask}
                addComponent={addComponent}
                removeComponent={removeComponent}
                editComponent={editComponent}
                setCompMode={setCompMode}
                toggleCompInvert={toggleCompInvert}
                commit={commit}
              />
            )}

            {spotting && (
              <div className="mx-3 mt-3 seg-group" role="group" aria-label={t("develop.tool.retouch")}>
                {(["spots", "erase"] as const).map((m) => (
                  <button
                    key={m}
                    aria-pressed={retouch === m}
                    onClick={() => setRetouchMode(m)}
                    className={`flex-1 py-1 text-caption ${
                      retouch === m ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
                    }`}
                  >
                    {t(m === "spots" ? "develop.retouch.spots" : "develop.retouch.erase")}
                  </button>
                ))}
              </div>
            )}

            {spotting && !erasing && (
              <SpotPanel
                spots={spots}
                selected={selectedSpot}
                onSelect={setSelectedSpot}
                onRemove={removeSpot}
                onEdit={editSpot}
                onCommit={commit}
                onDone={() => setSpotMode(false)}
              />
            )}

            {erasing && (
              <ErasePanel
                erases={erases}
                how={eraseHow}
                onHow={setEraseHow}
                brush={eraseBrush}
                onBrush={setEraseBrush}
                selected={hasSelection(eraseSel)}
                points={eraseSel.points.length}
                picking={erasePicking}
                running={eraseRunning}
                pct={eraseJob && eraseJob.total > 0 ? Math.round((eraseJob.current / eraseJob.total) * 100) : null}
                started={!!eraseJob}
                error={eraseError}
                onErase={() => void runErase()}
                onClear={() => setEraseSel(NO_SELECTION)}
                onCancel={() => void api.cancelJob("erase")}
                onRemove={(i) => void removeErase(i)}
                onDone={() => setSpotMode(false)}
              />
            )}

            {cropping && (
              <CropPanel
                geometry={draft.geometry}
                aspect={aspect}
                originalAspect={uncroppedAspect}
                changed={mod.geometry}
                onAspect={applyAspect}
                onGeometry={setGeometry}
                onStraighten={(v) => {
                  setStraightening(true);
                  set({ geometry: { ...draftRef.current.geometry, straighten: v } });
                }}
                onStraightenCommit={() => {
                  setStraightening(false);
                  commit(t("develop.history.straighten"));
                }}
                onReset={() => {
                  setAspect(null);
                  setGeometry(DEFAULT_STACK.geometry, t("develop.history.resetCrop"));
                }}
                onDone={() => setCropMode(false)}
              />
            )}

            {tool === "adjust" && (
              <AdjustPanel
                draft={draft}
                draftRef={draftRef}
                set={set}
                commit={commit}
                apply={apply}
                adjust={adjust}
                isOpen={isOpen}
                toggle={toggle}
                mod={mod}
                develop={develop}
                autoWb={autoWb}
                wbPicking={wbPicking}
                toggleWbPicker={toggleWbPicker}
                wbKey={wbKey}
                curveChannel={curveChannel}
                setCurveChannel={setCurveChannel}
                mixerMode={mixerMode}
                setMixerMode={setMixerMode}
                zoom={zoom}
                pan={pan}
                setZoom={setZoom}
                setPan={setPan}
              />
            )}
          </div>

          <CopyPasteBar
            copySettings={copySettings}
            pasteSettings={pasteSettings}
            pastePrevious={pastePrevious}
            pasteToSelection={pasteToSelection}
            hasCopied={hasCopied}
            previousId={previousId}
            selectedOthers={selectedOthers}
            copyKey={copyKey}
            pasteKey={pasteKey}
          />
          </>
        </aside>
      </div>

      <ExportDialog
        open={showExport}
        onClose={() => setShowExport(false)}
        filename={filename}
        onExportingChange={setExporting}
      />
    </div>
  );
}

/** Keys a focused slider keeps for itself. */
/** An AI mask's bitmap, which tells picked masks apart; "" for any other. */
function aiRle(m: Mask | undefined): string {
  const shape = m && maskComponents(m).find((c) => c.shape.type === "ai")?.shape;
  return shape?.type === "ai" ? shape.rle : "";
}

const ARROWS = new Set(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"]);
