import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { t, type Key } from "../../src/i18n";
import { Icon, type IconName } from "../../src/lib/icons";
import { fromView, toView } from "../../src/lib/cropView";
import CropOverlay from "../../src/components/CropOverlay";
import { DEFAULT_STACK, FULL_FRAME, withStackDefaults, type Stack } from "../../src/types";
import { api, editFrameUrl, type DevicePhoto, type LookInfo, type PresetGroupInfo } from "./api";
import { pushStep } from "./back";
import {
  ColourPanel,
  CropPanel,
  CurvePanel,
  DetailPanel,
  EffectsPanel,
  GradingPanel,
  isMoved,
  LightPanel,
  LutsPanel,
  PresetsPanel,
  MixerPanel,
  OpticsPanel,
  restOf,
  same,
  TOOLS,
  type Tool,
} from "./EditPanels";
import Histogram from "./Histogram";
import { ConfirmSheet, Dropdown, IconButton, MenuItem } from "./ui";

/** The tools in three groups along the bottom, as Photos has Adjust,
 *  Filters and Crop: the group's tools sit in a strip above them. */
const GROUPS: { id: "adjust" | "presets" | "crop"; icon: IconName; label: Key; tools: Tool[] }[] = [
  {
    id: "adjust",
    icon: "adjust",
    label: "mobile.edit.group.adjust",
    tools: ["light", "colour", "curve", "mixer", "grading", "detail", "effects", "optics"],
  },
  { id: "presets", icon: "layers", label: "mobile.edit.group.presets", tools: ["presets", "luts"] },
  { id: "crop", icon: "crop", label: "mobile.edit.group.crop", tools: ["crop"] },
];
const groupOf = (tool: Tool) => GROUPS.find((g) => g.tools.includes(tool))!;

/** What is being edited: one of the phone's own photos, or a library photo
 *  (by id), whose edits go to the computer. */
export type EditSource = { kind: "device"; photo: DevicePhoto } | { kind: "library"; id: number };

/** Why a library photo can't be edited here, as `edit_open_library` says
 *  it (mobile/src-tauri/src/link.rs), worded; anything else as it came. */
const LIBRARY_ERRORS = {
  gone: "mobile.libraryEdit.gone",
  raw: "mobile.libraryEdit.raw",
  computerOnly: "mobile.libraryEdit.computerOnly",
  updateComputer: "mobile.libraryEdit.updateComputer",
  updatePhone: "mobile.libraryEdit.updatePhone",
  needsComputer: "mobile.libraryEdit.needsComputer",
} as const;
export function libraryEditError(e: unknown): string {
  const code = String(e);
  return code in LIBRARY_ERRORS ? t(LIBRARY_ERRORS[code as keyof typeof LIBRARY_ERRORS]) : code;
}

/** How the photo is looked at: `k` times the size that fits it (1 is the
 *  whole photo), around the spot (`cx`, `cy`), each 0 to 1 across the framed
 *  photo. */
type View = { k: number; cx: number; cy: number };
const WHOLE: View = { k: 1, cx: 0.5, cy: 0.5 };
/** A frame being asked for: the edit, how sharp, and zoomed in how far. */
type Want = { stack: object; settled: boolean; view?: View };
/** What a frame on the canvas shows: its pixels, its size on screen when
 *  zoomed (fitted to the area otherwise), and the view it was drawn at. */
type Frame = { width: number; height: number; css: { width: number; height: number } | null; view: View };

/** A change is kept this long after the last one: the end of a drag. */
const COMMIT_MS = 600;
/** Sharp frames once nothing has moved for this long; quick ones before. */
const SETTLE_MS = 200;
/** Holding the photo this long shows it unedited. */
const HOLD_MS = 250;
/** Two taps this close together (ms) are a double tap: in to the photo's
 *  own pixels, or back out to the whole. */
const DOUBLE_TAP_MS = 300;
/** Where Copy edits keeps them, for Paste on another photo. */
const CLIPBOARD = "sourcelight.editCopy";

function copiedEdits(): Stack | null {
  try {
    const kept = localStorage.getItem(CLIPBOARD);
    return kept ? withStackDefaults(JSON.parse(kept)) : null;
  } catch {
    return null;
  }
}

/**
 * Develop on the phone (docs/MOBILE.md section 4, steps 4 and 6): one of the
 * phone's own photos, or a library photo, edited with the desktop's pipeline
 * on the phone's GPU. Every change is kept as settings; no photo is ever
 * written. A phone photo's Save copy puts the edited photo in the gallery
 * beside it; a library photo's edits go to the computer.
 *
 * The tools run along the bottom (`EditPanels`), each with a dot once it has
 * changed the photo and a Reset of its own. Light has the histogram and Auto;
 * Colour has Auto white balance and a picker (tap something that should be
 * grey). Compare, or holding the photo, shows it unedited in its crop. The
 * menu at the top right copies the edit, pastes one onto this photo, or
 * resets it all.
 *
 * Pinch or double-tap to zoom in, down to the photo's own pixels, for
 * sharpening and noise; drag to look around. While the fingers move the frame
 * there is moved and scaled to follow, and a frame of just the part showing
 * follows (`editframe` with a view).
 *
 * Frames come from `editframe://`, small and quick while a slider moves and
 * full size once it stops.
 *
 * Each change becomes a step of history once a drag ends (undo and redo
 * walk them), but nothing is kept until Done, as in Photos: a phone photo's
 * edits go in its settings, a library photo's to the computer. Cancel, or
 * Back, with changes asks before letting them go. The opening's token goes
 * with each step; if the photo has gone from the GPU under the editor, it
 * opens again with this session's edits.
 *
 * The editor takes a Back step of its own; the libraries only open it and
 * hear when it has gone (`onClose`).
 */
export default function Editor({
  source,
  preview,
  onClose,
  onSaved,
}: {
  source: EditSource;
  /** The loupe's copy, shown until the first frame. */
  preview: string;
  onClose: () => void;
  onSaved?: () => void;
}) {
  const library = source.kind === "library";
  const [stack, setStack] = useState<Stack | null>(null);
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);
  const [tool, setTool] = useState<Tool>("light");
  // The crop's shape while dragging: a ratio chosen below, or free.
  const [ratio, setRatio] = useState<number | null>(null);
  // In the Crop tool the photo shows uncropped, still turned, flipped and
  // straightened, for the box to be dragged back out into, as on the
  // desktop.
  const cropping = tool === "crop";
  const croppingRef = useRef(cropping);
  croppingRef.current = cropping;
  const [presetGroups, setPresetGroups] = useState<PresetGroupInfo[]>([]);
  const [looks, setLooks] = useState<LookInfo[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [frame, setFrame] = useState<Frame | null>(null);
  // How the photo is looked at, kept in a ref too for the frame pump.
  const [view, setViewState] = useState<View>(WHOLE);
  const viewRef = useRef<View>(WHOLE);
  // The framed photo's shape, from the last frame of all of it.
  const aspect = useRef<number | null>(null);
  // Sharp frames drawn so far: the histogram reads each.
  const [sharpFrames, setSharpFrames] = useState(0);
  // A frame that couldn't be had, with a way to ask again.
  const [frameError, setFrameError] = useState<string | null>(null);
  // Showing the photo unedited; picking a grey for the white balance; Auto
  // or the picker working.
  const [comparing, setComparing] = useState(false);
  const [picking, setPicking] = useState(false);
  const [working, setWorking] = useState(false);
  // The menu at the top right, under its button.
  const [menu, setMenu] = useState<DOMRect | null>(null);
  // This opening's token, and a count that opens the photo again when it
  // has gone from under the editor.
  const token = useRef(0);
  const [opening, setOpening] = useState(0);

  const area = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const strip = useRef<HTMLElement>(null);
  const [box, setBox] = useState({ width: 0, height: 0 });

  // The image area's size, for fitting frames to it.
  useLayoutEffect(() => {
    const el = area.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setBox({ width: el.clientWidth, height: el.clientHeight }));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // The photo, by identity: a new source object for the same photo is not a
  // reason to open it again.
  const key = source.kind === "device" ? `d${source.photo[0]}_${source.photo[2]}` : `l${source.id}`;
  useEffect(() => {
    let gone = false;
    (source.kind === "device" ? api.editOpen(source.photo[0], source.photo[2]) : api.editOpenLibrary(source.id))
      .then((o) => {
        if (gone) return;
        token.current = o.token;
        const opened = withStackDefaults(o.stack);
        // Opened again after it went: this session's edits, not yet kept,
        // stay on screen and become a step of the new opening.
        const again = openedWith.current !== null;
        if (!again) openedWith.current = opened;
        const pending = latest.current ?? (again ? stackNow.current : null);
        setStack(pending ?? opened);
        if (pending) {
          latest.current = pending;
          void commit();
        }
        setSize({ width: o.width, height: o.height });
        setCanUndo(o.canUndo);
        setCanRedo(o.canRedo);
        setFrameError(null);
      })
      .catch((e) => !gone && setError(libraryEditError(e)));
    return () => {
      gone = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, opening]);
  useEffect(() => {
    api.editPresets().then(setPresetGroups).catch(() => setPresetGroups([]));
    api.editLooks().then(setLooks).catch(() => setLooks([]));
  }, []);
  /** The photo went from under the editor: open it again. */
  const reopen = useCallback(() => setOpening((n) => n + 1), []);

  // An edit that went on a copy, as the photo was edited on the computer
  // too: said here, as the rest of this editing goes there as well.
  const libraryId = source.kind === "library" ? source.id : null;
  useEffect(() => {
    if (libraryId === null) return;
    const un = listen<number>("library-edit-copied", (e) => {
      if (e.payload === libraryId) setNotice(t("mobile.libraryEdit.onCopy"));
    });
    return () => void un.then((f) => f());
  }, [libraryId]);

  // Latest wins: one frame in flight, then the newest edit if it moved.
  const want = useRef<Want | null>(null);
  const busy = useRef(false);
  const boxRef = useRef(box);
  boxRef.current = box;
  const pump = useCallback(async () => {
    const b = boxRef.current;
    if (busy.current || !want.current || b.width === 0) return;
    const asked = want.current;
    want.current = null;
    busy.current = true;
    try {
      // Half size while moving: a quarter of the pixels to render and send.
      const scale = window.devicePixelRatio * (asked.settled ? 1 : 0.5);
      const url = editFrameUrl(
        Math.round(b.width * scale),
        Math.round(b.height * scale),
        asked.settled ? 85 : 70,
        asked.stack,
        asked.view,
      );
      const res = await fetch(url);
      if (!res.ok) {
        const why = await res.text();
        // Gone from the GPU (the app was away a long while): open it again,
        // and this frame follows once it is.
        if (why === "No photo is open.") {
          want.current ??= asked;
          reopen();
          return;
        }
        throw new Error(why);
      }
      const bitmap = await createImageBitmap(await res.blob());
      const c = canvas.current;
      if (c) {
        if (c.width !== bitmap.width || c.height !== bitmap.height) Object.assign(c, { width: bitmap.width, height: bitmap.height });
        c.getContext("2d")!.drawImage(bitmap, 0, 0);
        setFrame({
          width: bitmap.width,
          height: bitmap.height,
          css: asked.view ? { width: bitmap.width / scale, height: bitmap.height / scale } : null,
          view: asked.view ?? WHOLE,
        });
        if (!asked.view) aspect.current = bitmap.width / bitmap.height;
        // The histogram is of the whole photo, so not of a zoomed frame.
        if (asked.settled && !asked.view) setSharpFrames((n) => n + 1);
      }
      bitmap.close();
      setFrameError(null);
    } catch (e) {
      setFrameError(String(e));
    } finally {
      busy.current = false;
      if (want.current) void pump();
    }
  }, [reopen]);

  const show = useCallback(
    (s: object, settled: boolean) => {
      const framed = s as Partial<Stack>;
      const view =
        croppingRef.current && framed.geometry ? { ...framed, geometry: { ...framed.geometry, crop: FULL_FRAME } } : s;
      const zoomed = viewRef.current.k > 1.001 ? viewRef.current : undefined;
      want.current = { stack: view, settled, view: zoomed };
      void pump();
    },
    [pump],
  );
  // Into and out of the Crop tool: the photo uncropped, or cropped again.
  useEffect(() => {
    if (stack) show(stack, true);
    // Only when the tool changes: edits ask for their own frames.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cropping]);

  // The first frame once the photo is open and the area measured; a sharp
  // one again, of all of it, if the area changes size (the phone turned).
  useEffect(() => {
    viewRef.current = WHOLE;
    setViewState(WHOLE);
    if (stack && box.width > 0) show(stack, true);
    // Only on opening and resizing: changes ask for their own frames.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [size, box.width, box.height]);

  // What the photo had when the editor opened it, to tell whether Cancel
  // would lose anything; and the edit on screen, for opening again.
  const openedWith = useRef<Stack | null>(null);
  const stackNow = useRef(stack);
  stackNow.current = stack;

  // Changes become one step of history once they stop.
  const latest = useRef<Stack | null>(null);
  const commitTimer = useRef(0);
  const settleTimer = useRef(0);
  const commit = useCallback(async () => {
    window.clearTimeout(commitTimer.current);
    commitTimer.current = 0;
    const s = latest.current;
    if (!s) return;
    latest.current = null;
    try {
      const step = await api.editCommit(token.current, s);
      setCanUndo(step.canUndo);
      setCanRedo(step.canRedo);
    } catch (e) {
      // Kept for the next try, unless a newer change came meanwhile.
      latest.current ??= s;
      if (String(e) === "closed") reopen();
      else setError(String(e));
    }
  }, [reopen]);

  const change = (next: Stack, now = false) => {
    setStack(next);
    latest.current = next;
    window.clearTimeout(settleTimer.current);
    if (now) {
      show(next, true);
      void commit();
      return;
    }
    show(next, false);
    settleTimer.current = window.setTimeout(() => show(next, true), SETTLE_MS);
    window.clearTimeout(commitTimer.current);
    commitTimer.current = window.setTimeout(() => void commit(), COMMIT_MS);
  };
  const set = (patch: Partial<Stack>, now = false) => {
    if (stack) change({ ...stack, ...patch }, now);
  };
  /** The edit as it stands, a change not yet kept included. */
  const current = () => latest.current ?? stack;

  const step = async (which: "undo" | "redo") => {
    await commit();
    try {
      const s = which === "undo" ? await api.editUndo(token.current) : await api.editRedo(token.current);
      const next = withStackDefaults(s.stack);
      // A step that changes the framing changes what a zoomed view shows.
      if (stack && JSON.stringify(stack.geometry) !== JSON.stringify(next.geometry)) {
        viewRef.current = WHOLE;
        setViewState(WHOLE);
      }
      setStack(next);
      setCanUndo(s.canUndo);
      setCanRedo(s.canRedo);
      show(next, true);
    } catch (e) {
      if (String(e) === "closed") reopen();
      else setError(String(e));
    }
  };

  /** Whether leaving now would lose changes. */
  const dirty = () => {
    const s = latest.current ?? stackNow.current;
    return s !== null && openedWith.current !== null && !same(s, openedWith.current);
  };
  // Leaving: the photo leaves the GPU, and the editor's Back step goes; the
  // libraries hear once it has (`onClose`), so their own Back handling
  // never sees it.
  const leaving = useRef(false);
  const leave = () => {
    leaving.current = true;
    void api.editClose(token.current).catch(() => {});
    history.back();
  };
  const [discarding, setDiscarding] = useState(false);
  const discardingNow = useRef(discarding);
  discardingNow.current = discarding;
  const cancel = async () => {
    await commit();
    if (dirty()) setDiscarding(true);
    else leave();
  };
  const [finishing, setFinishing] = useState(false);
  const done = async () => {
    setFinishing(true);
    try {
      await commit();
      await api.editDone(token.current);
      leave();
    } catch (e) {
      if (String(e) === "closed") reopen();
      else setError(String(e));
    } finally {
      setFinishing(false);
    }
  };
  const onCloseNow = useRef(onClose);
  onCloseNow.current = onClose;
  const dirtyNow = useRef(dirty);
  dirtyNow.current = dirty;
  useEffect(() => {
    // Once: StrictMode runs this twice in development, and a second step
    // would leave Done and Cancel landing on the first.
    if (!(history.state as { editor?: boolean } | null)?.editor) pushStep("editor");
    const onPop = () => {
      const at = history.state as { editor?: boolean; editMenu?: boolean } | null;
      if (leaving.current) {
        if (!at?.editor) onCloseNow.current();
        return;
      }
      // Back that closes the menu lands on the editor's own step.
      if (at?.editor || at?.editMenu) return;
      // Back from the editor itself: the question first, if there are
      // changes; Back again while it is asked keeps editing.
      if (discardingNow.current) {
        pushStep("editor");
        setDiscarding(false);
      } else if (dirtyNow.current()) {
        pushStep("editor");
        setDiscarding(true);
      } else {
        leaving.current = true;
        void api.editClose(token.current).catch(() => {});
        onCloseNow.current();
      }
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);
  // However the editor goes, the photo leaves the GPU. The token keeps a
  // slow close off a photo opened after it.
  useEffect(
    () => () => {
      const mine = token.current;
      void api.editClose(mine).catch(() => {});
    },
    [],
  );

  const save = async () => {
    await commit();
    setSaving(true);
    setNotice(null);
    try {
      const name = await api.editSave(token.current);
      setNotice(t("mobile.edit.saved", { name }));
      onSaved?.();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  // Something worked out on the photo (Auto, the picker): made a change like
  // any other once it comes back.
  const work = async (run: () => Promise<Stack | null>) => {
    setWorking(true);
    try {
      const next = await run();
      if (next) change(next, true);
    } catch (e) {
      if (String(e) === "closed") reopen();
      else setError(String(e));
    } finally {
      setWorking(false);
    }
  };
  const auto = (kind: "tone" | "wb") =>
    work(async () => {
      const base = current();
      if (!base) return null;
      const next = withStackDefaults(await api.editAuto(token.current, base, kind));
      // Auto white balance leaves it be when too little is near grey.
      if (kind === "wb" && next.warmth === base.warmth && next.tint === base.tint) {
        setNotice(t("mobile.edit.autoWbNone"));
        return null;
      }
      return next;
    });
  // The picker: a tap on the photo, in the framed photo from 0 to 1,
  // wherever it is zoomed to.
  const pickAt = (clientX: number, clientY: number) => {
    const fit = fitSize();
    if (!fit) return;
    const v = viewRef.current;
    const p = local({ x: clientX, y: clientY });
    const x = v.cx + p.x / (v.k * fit.width);
    const y = v.cy + p.y / (v.k * fit.height);
    if (x < 0 || x > 1 || y < 0 || y > 1) return;
    setPicking(false);
    void work(async () => {
      const base = current();
      return base ? withStackDefaults(await api.editPickWb(token.current, base, x, y)) : null;
    });
  };

  // Compare: the photo unedited while held.
  /** The photo as it came, framed as the edit frames it, so the original
   *  lines up with the edit, zoomed or not. */
  const original = () => {
    const s = current();
    return s ? { version: s.version, geometry: s.geometry } : {};
  };
  const compareOn = () => {
    if (!stack || comparing) return;
    setComparing(true);
    show(original(), true);
  };
  const compareOff = () => {
    if (!comparing) return;
    setComparing(false);
    const s = current();
    if (s) show(s, true);
  };
  /** A point on screen, from the middle of the photo's area. */
  const local = (p: { x: number; y: number }) => {
    const r = area.current!.getBoundingClientRect();
    return { x: p.x - r.left - r.width / 2, y: p.y - r.top - r.height / 2 };
  };
  /** The size the whole framed photo shows at, in CSS pixels. */
  const fitSize = () => {
    const a = aspect.current;
    const b = boxRef.current;
    if (!a || !b.width || !b.height) return null;
    const width = Math.min(b.width, b.height * a);
    return { width, height: width / a };
  };
  /** How far in shows the photo's own pixels, one to one on the screen. */
  const ownPixels = () => {
    const fit = fitSize();
    if (!fit || !size || !stack) return 3;
    const g = stack.geometry;
    const turned = (((g.quarterTurns ?? 0) % 4) + 4) % 4 % 2 === 1;
    const across = (turned ? size.height : size.width) * g.crop.width;
    return across / (fit.width * window.devicePixelRatio);
  };
  /** `v` kept to what can be shown: no further out than the whole photo, no
   *  further in than twice its own pixels, and the photo filling the view. */
  const within = (v: View): View => {
    const fit = fitSize();
    const b = boxRef.current;
    if (!fit) return WHOLE;
    const k = Math.min(Math.max(2, ownPixels() * 2), Math.max(1, v.k));
    const ex = Math.min(1, b.width / (k * fit.width));
    const ey = Math.min(1, b.height / (k * fit.height));
    return {
      k,
      cx: Math.min(1 - ex / 2, Math.max(ex / 2, v.cx)),
      cy: Math.min(1 - ey / 2, Math.max(ey / 2, v.cy)),
    };
  };
  const look = (v: View, settled: boolean) => {
    const next = within(v);
    viewRef.current = next;
    setViewState(next);
    const s = comparing ? original() : current();
    if (s) show(s, settled);
  };

  // On the photo: hold to compare, a tap for the picker, pinch or double-tap
  // to zoom, and drag to look around once zoomed. Not while cropping, where
  // the box takes the fingers.
  const holdTimer = useRef(0);
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  // A gesture, restarted whenever a finger comes or goes: the view and the
  // fingers' middle and spread when it began.
  const gesture = useRef<{ view: View; at: { x: number; y: number }; spread: number } | null>(null);
  const moved = useRef(false);
  const lastTap = useRef(0);
  const fingers = () => {
    const [a, b] = [...pointers.current.values()];
    if (!a) return { at: { x: 0, y: 0 }, spread: 0 };
    if (!b) return { at: local(a), spread: 0 };
    return { at: local({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }), spread: Math.hypot(a.x - b.x, a.y - b.y) };
  };
  const begin = () => {
    const { at, spread } = fingers();
    gesture.current = { view: viewRef.current, at, spread };
  };
  const onDown = (e: React.PointerEvent) => {
    if (cropping) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size === 1) {
      moved.current = false;
      if (!picking) holdTimer.current = window.setTimeout(compareOn, HOLD_MS);
    } else window.clearTimeout(holdTimer.current);
    begin();
  };
  const onMove = (e: React.PointerEvent) => {
    const g = gesture.current;
    if (!g || !pointers.current.has(e.pointerId)) return;
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const { at, spread } = fingers();
    if (!moved.current && (pointers.current.size > 1 || Math.hypot(at.x - g.at.x, at.y - g.at.y) > 10)) {
      moved.current = true;
      window.clearTimeout(holdTimer.current);
    }
    const fit = fitSize();
    if (!moved.current || !fit) return;
    if (pointers.current.size === 2 && g.spread > 0) {
      // The spot that was between the fingers stays between them.
      const k = within({ ...g.view, k: (g.view.k * spread) / g.spread }).k;
      const ux = g.view.cx + g.at.x / (g.view.k * fit.width);
      const uy = g.view.cy + g.at.y / (g.view.k * fit.height);
      look({ k, cx: ux - at.x / (k * fit.width), cy: uy - at.y / (k * fit.height) }, false);
    } else if (pointers.current.size === 1 && g.view.k > 1) {
      look(
        {
          k: g.view.k,
          cx: g.view.cx - (at.x - g.at.x) / (g.view.k * fit.width),
          cy: g.view.cy - (at.y - g.at.y) / (g.view.k * fit.height),
        },
        false,
      );
    }
  };
  const onUp = (e: React.PointerEvent) => {
    if (!pointers.current.delete(e.pointerId)) return;
    // A finger still down carries on from where things are.
    if (pointers.current.size > 0) return begin();
    window.clearTimeout(holdTimer.current);
    gesture.current = null;
    if (comparing) return compareOff();
    if (moved.current) {
      // A sharp frame of where the fingers left it.
      look(viewRef.current, true);
      return;
    }
    if (picking) return pickAt(e.clientX, e.clientY);
    const now = performance.now();
    if (now - lastTap.current < DOUBLE_TAP_MS) {
      lastTap.current = 0;
      const fit = fitSize();
      if (viewRef.current.k > 1.01 || !fit) return look(WHOLE, true);
      // In to the photo's own pixels, around the spot tapped.
      const at = local({ x: e.clientX, y: e.clientY });
      const k = Math.max(2, ownPixels());
      const u = { x: 0.5 + at.x / fit.width, y: 0.5 + at.y / fit.height };
      look({ k, cx: u.x - at.x / (k * fit.width), cy: u.y - at.y / (k * fit.height) }, true);
    } else lastTap.current = now;
  };

  // Crop to a ratio, shaped as the photo shows now, as the desktop does.
  const cropTo = (r: number) => {
    if (!stack || !size) return;
    setRatio(r);
    const orient = size.width / size.height;
    const turned = (((stack.geometry.quarterTurns ?? 0) % 4) + 4) % 4 % 2 === 1;
    const target = r / (turned ? 1 / orient : orient);
    const width = target > 1 ? 1 : target;
    const height = target > 1 ? 1 / target : 1;
    const crop = fromView({ x: (1 - width) / 2, y: (1 - height) / 2, width, height }, stack.geometry, orient);
    set({ geometry: { ...stack.geometry, crop } }, true);
  };

  // The tool last used in each group, to come back to.
  const group = groupOf(tool);
  const lastIn = useRef<Partial<Record<(typeof GROUPS)[number]["id"], Tool>>>({});
  lastIn.current[group.id] = tool;
  const chooseTool = (next: Tool) => {
    // The crop box is drawn on the whole photo.
    if (next === "crop" && viewRef.current.k > 1) {
      viewRef.current = WHOLE;
      setViewState(WHOLE);
    }
    setTool(next);
    setPicking(false);
    panel.current?.scrollTo({ top: 0 });
    strip.current
      ?.querySelector(`[data-tool="${next}"]`)
      ?.scrollIntoView({ inline: "nearest", block: "nearest", behavior: "smooth" });
  };

  // The menu at the top right takes a Back step of its own; the libraries
  // leave the editor open when Back lands on its step again.
  const openMenu = (anchor: DOMRect) => {
    pushStep("editMenu");
    setMenu(anchor);
  };
  useEffect(() => {
    const onPop = () => setMenu(null);
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);
  const fromMenu = (run: () => void | Promise<void>) => () => {
    history.back();
    void run();
  };
  const copyEdits = async () => {
    await commit();
    const s = current();
    if (!s) return;
    try {
      localStorage.setItem(CLIPBOARD, JSON.stringify(s));
      setNotice(t("mobile.edit.copied"));
    } catch (e) {
      setError(String(e));
    }
  };
  const pasteEdits = () =>
    work(async () => {
      const copied = copiedEdits();
      const base = current();
      if (!copied || !base) return null;
      const next = withStackDefaults(await api.editPaste(copied, base));
      setNotice(t("mobile.edit.pasted"));
      return next;
    });
  // Everything back as it came, but for what the phone can't edit (masks
  // and retouching made on the computer), which only the computer may take
  // off. Undo brings it all back.
  const resetAll = () => {
    const s = current();
    if (s) change({ ...DEFAULT_STACK, version: s.version, masks: s.masks, spots: s.spots, erases: s.erases }, true);
  };

  // The frame's size on screen: a zoomed one at the size it was drawn for,
  // one of the whole photo fitted to the area.
  const fitted = !frame || box.width === 0
    ? null
    : (frame.css ??
      (() => {
        const a = frame.width / frame.height;
        const width = Math.min(box.width, box.height * a);
        return { width, height: width / a };
      })());
  // Until a frame for the view wanted lands, the one there is moved and
  // scaled to match it: what follows the fingers.
  const follow = (() => {
    const fit = fitSize();
    if (!frame || !fit) return undefined;
    const s = view.k / frame.view.k;
    const tx = (frame.view.cx - view.cx) * view.k * fit.width;
    const ty = (frame.view.cy - view.cy) * view.k * fit.height;
    if (Math.abs(s - 1) < 1e-3 && Math.abs(tx) < 0.5 && Math.abs(ty) < 0.5) return undefined;
    return `translate(${tx}px, ${ty}px) scale(${s})`;
  })();

  const g = stack?.geometry;
  // The Crop tool's frame: the photo upright, turned as the edit turns it.
  const uncroppedAspect =
    size && g ? ((((g.quarterTurns ?? 0) % 4) + 4) % 4) % 2 === 1 ? size.height / size.width : size.width / size.height : 1;
  const info = TOOLS.find((x) => x.id === tool)!;
  const toolButton = (on: boolean) => `${on ? "btn-accent" : "btn-solid"} min-h-10 gap-1.5 px-3 text-caption`;

  return (
    <div className="flex h-full flex-col bg-black beside:flex-row beside:pt-[var(--safe-top)]">
      <div
        ref={area}
        // A margin while cropping keeps the box's grips clear of the top bar,
        // and of the sides where a swipe is the system's Back.
        className={`relative order-2 min-h-0 flex-1 touch-none overflow-hidden select-none beside:order-1 ${cropping ? "my-4 mr-[max(1rem,calc(var(--gesture-right,0px)+0.5rem))] ml-[max(1rem,calc(var(--gesture-left,0px)+0.5rem))]" : ""}`}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onContextMenu={(e) => e.preventDefault()}
      >
        {!frame && <img src={preview} alt="" draggable={false} className="absolute inset-0 h-full w-full object-contain opacity-60" />}
        <canvas
          ref={canvas}
          className={`absolute inset-0 m-auto ${picking ? "cursor-crosshair" : ""}`}
          style={fitted ? { width: fitted.width, height: fitted.height, transform: follow } : { display: "none" }}
        />
        {!stack && !error && (
          <p className="absolute inset-x-0 bottom-3 text-center text-label text-muted">{t("mobile.edit.opening")}</p>
        )}
        {comparing && (
          <span className="pointer-events-none absolute top-2 left-2 rounded-sm bg-chip px-2 py-1 text-label text-white">
            {t("mobile.edit.original")}
          </span>
        )}
        {view.k > 1.01 && (
          // How far in, as a share of the photo's own pixels: 100% is one to one.
          <span className="pointer-events-none absolute top-2 right-2 rounded-sm bg-chip px-2 py-1 font-mono text-label tabular-nums text-white">
            {Math.round((view.k / ownPixels()) * 100)}%
          </span>
        )}
        {picking && (
          <span className="pointer-events-none absolute inset-x-3 top-2 rounded-sm bg-chip px-3 py-2 text-center text-caption text-white">
            {t("mobile.edit.pickHint")}
          </span>
        )}
        {frameError && stack && (
          <div className="absolute inset-x-3 bottom-3 flex items-center gap-3 rounded-md bg-chip px-3 py-2">
            <p className="min-w-0 flex-1 text-caption text-white">{t("mobile.edit.frameFailed")}</p>
            <button
              className="btn-accent min-h-10 shrink-0 px-3 text-caption"
              title={frameError}
              onClick={() => show(current() ?? stack, true)}
            >
              {t("mobile.link.retry")}
            </button>
          </div>
        )}
        {cropping && stack && size && frame && (
          <CropOverlay
            touch
            frameAspect={uncroppedAspect}
            crop={toView(stack.geometry, size.width / size.height)}
            ratio={ratio}
            onChange={(next) => set({ geometry: { ...stack.geometry, crop: fromView(next, stack.geometry, size.width / size.height) } })}
            onCommit={(next) =>
              set({ geometry: { ...stack.geometry, crop: fromView(next, stack.geometry, size.width / size.height) } }, true)
            }
          />
        )}
      </div>

      {/* The bars and the tools: above and below the photo, or in a column
          beside it when the phone is on its side. Upright, `contents` lets
          them join the outer column, placed by `order`. */}
      <div className="contents beside:order-2 beside:flex beside:w-80 beside:shrink-0 beside:flex-col beside:border-l beside:border-line beside:bg-base">
        <div className="order-1 flex items-center gap-1 pt-[calc(var(--safe-top)+0.25rem)] pr-4 pb-1 pl-2 beside:pt-1 beside:pr-3">
          <IconButton icon="close" label={t("common.cancel")} onClick={() => void cancel()} />
          <IconButton icon="undo" label={t("common.undo")} disabled={!canUndo} onClick={() => void step("undo")} />
          <IconButton icon="redo" label={t("common.redo")} disabled={!canRedo} onClick={() => void step("redo")} />
          {/* Held, not tapped: the photo unedited for as long as it is down.
              On its side the bar is narrow, and holding the photo does it. */}
          <button
            type="button"
            className={`flex h-12 w-12 shrink-0 touch-none items-center justify-center rounded-md beside:hidden ${comparing ? "bg-raised text-ink" : "text-ink"}`}
            aria-label={t("mobile.edit.compare")}
            title={t("mobile.edit.compare")}
            disabled={!stack}
            onPointerDown={compareOn}
            onPointerUp={compareOff}
            onPointerLeave={compareOff}
            onPointerCancel={compareOff}
            onContextMenu={(e) => e.preventDefault()}
          >
            <Icon name="before-after" size={22} />
          </button>
          <div className="flex-1" />
          <IconButton
            icon="more"
            label={t("mobile.edit.menu")}
            disabled={!stack}
            onClick={(e) => openMenu(e.currentTarget.getBoundingClientRect())}
          />
          <button
            className="btn-accent ml-1 min-h-10 shrink-0 px-4 text-body"
            disabled={!stack || finishing || saving}
            title={library ? t("mobile.libraryEdit.syncs") : undefined}
            onClick={() => void done()}
          >
            {t("mobile.edit.done")}
          </button>
        </div>

        {(error || notice) && (
          <p
            role="status"
            className={`order-3 px-4 py-2 text-label ${error ? "text-danger" : "text-ink"}`}
            onClick={() => (setError(null), setNotice(null))}
          >
            {error ?? notice}
          </p>
        )}

        <div
          ref={panel}
          // Clear of the sides where a swipe is the system's Back: a slider
          // resting at its left end would otherwise close the editor.
          className="order-3 max-h-[40%] overflow-y-auto bg-base pr-[max(1rem,calc(var(--gesture-right,0px)+0.5rem))] pb-2 pl-[max(1rem,calc(var(--gesture-left,0px)+0.5rem))] beside:max-h-none beside:min-h-0 beside:flex-1"
        >
          {/* The tool's name, what it can work out, and its Reset, staying
              in view as its controls scroll. */}
          <div className="sticky top-0 z-10 flex min-h-12 items-center gap-2 bg-base">
            {/* On its side the column is narrow, and the strip below says
                which tool this is. */}
            <h2 className="min-w-0 flex-1 truncate font-display text-title text-ink beside:sr-only">{t(info.label)}</h2>
            <div className="hidden flex-1 beside:block" />
            {stack && tool === "light" && (
              <button className={toolButton(false)} disabled={working} onClick={() => void auto("tone")}>
                <Icon name="adjust" size={16} />
                {t("develop.auto.label")}
              </button>
            )}
            {stack && tool === "colour" && (
              <>
                <button className={toolButton(false)} disabled={working} onClick={() => void auto("wb")}>
                  <Icon name="adjust" size={16} />
                  {t("develop.auto.label")}
                </button>
                <button className={toolButton(picking)} aria-pressed={picking} disabled={working} onClick={() => setPicking(!picking)}>
                  <Icon name="dropper" size={16} />
                  {t("mobile.edit.pick")}
                </button>
              </>
            )}
            <button
              className="btn-ghost min-h-10 shrink-0 px-2 text-caption text-muted disabled:opacity-40"
              disabled={!stack || !isMoved(tool, stack)}
              onClick={() => stack && set(restOf(tool, stack), true)}
            >
              {t("common.reset")}
            </button>
          </div>
          {stack && tool === "light" && (
            <>
              <Histogram canvas={canvas} frame={sharpFrames} />
              <LightPanel stack={stack} set={set} />
            </>
          )}
          {stack && tool === "colour" && <ColourPanel stack={stack} set={set} />}
          {stack && tool === "curve" && <CurvePanel stack={stack} set={set} />}
          {stack && tool === "mixer" && <MixerPanel stack={stack} set={set} />}
          {stack && tool === "grading" && <GradingPanel stack={stack} set={set} />}
          {stack && tool === "detail" && <DetailPanel stack={stack} set={set} />}
          {stack && tool === "effects" && <EffectsPanel stack={stack} set={set} />}
          {stack && tool === "optics" && <OpticsPanel stack={stack} set={set} />}
          {stack && tool === "crop" && (
            <CropPanel stack={stack} set={set} size={size} ratio={ratio} setRatio={setRatio} cropTo={cropTo} />
          )}
          {stack && tool === "presets" && (
            <PresetsPanel stack={stack} set={set} change={change} groups={presetGroups} />
          )}
          {stack && tool === "luts" && <LutsPanel stack={stack} set={set} looks={looks} />}
        </div>

        {/* The group's tools, sideways when they don't all fit, each with a
            dot once it has changed the photo; Crop is a group of one. */}
        {group.tools.length > 1 && (
          <nav ref={strip} className="order-3 flex shrink-0 overflow-x-auto border-t border-line bg-base">
            {group.tools.map((id) => {
              const x = TOOLS.find((tool) => tool.id === id)!;
              return (
                <button
                  key={x.id}
                  data-tool={x.id}
                  className={`flex min-h-14 w-[4.5rem] shrink-0 flex-col items-center justify-center gap-0.5 text-label ${tool === x.id ? "seg-on font-medium text-ink" : "text-muted"}`}
                  aria-current={tool === x.id ? "page" : undefined}
                  onClick={() => chooseTool(x.id)}
                >
                  <span className="relative">
                    <Icon name={x.icon} size={22} />
                    {stack && isMoved(x.id, stack) && (
                      <span aria-hidden className="absolute -top-0.5 -right-1.5 h-1.5 w-1.5 rounded-full bg-accent" />
                    )}
                  </span>
                  {t(x.label)}
                </button>
              );
            })}
          </nav>
        )}
        {/* The groups, along the very bottom: a segmented control, so it
            reads apart from the strip of tools above it. */}
        <nav
          className="order-3 flex shrink-0 gap-1 border-t border-line bg-base px-2 pt-1.5 pb-[max(0.375rem,var(--safe-bottom))]"
          aria-label={t("mobile.edit.groups")}
        >
          {GROUPS.map((g) => (
            <button
              key={g.id}
              className={`flex min-h-11 flex-1 items-center justify-center gap-2 rounded-md text-caption ${group.id === g.id ? "bg-raised font-medium text-ink" : "text-muted"}`}
              aria-current={group.id === g.id ? "page" : undefined}
              onClick={() => chooseTool(g.id === group.id ? tool : (lastIn.current[g.id] ?? g.tools[0]!))}
            >
              <span className="relative">
                <Icon name={g.icon} size={18} />
                {stack && g.tools.some((id) => isMoved(id, stack)) && (
                  <span aria-hidden className="absolute -top-0.5 -right-1.5 h-1.5 w-1.5 rounded-full bg-accent" />
                )}
              </span>
              {t(g.label)}
            </button>
          ))}
        </nav>
      </div>

      {discarding && (
        <ConfirmSheet
          title={t("mobile.edit.discardTitle")}
          message={library ? t("mobile.edit.discardLibrary") : undefined}
          confirm={t("mobile.edit.discard")}
          cancel={t("mobile.edit.keepEditing")}
          onConfirm={() => {
            setDiscarding(false);
            leave();
          }}
          onCancel={() => setDiscarding(false)}
        />
      )}
      {menu && (
        <Dropdown anchor={menu} align="right" onClose={() => history.back()}>
          {!library && (
            <MenuItem
              icon="import"
              label={saving ? t("mobile.edit.saving") : t("mobile.edit.save")}
              onClick={fromMenu(save)}
            />
          )}
          <MenuItem icon="copy" label={t("mobile.edit.copy")} onClick={fromMenu(copyEdits)} />
          {copiedEdits() && <MenuItem icon="import" label={t("mobile.edit.paste")} onClick={fromMenu(pasteEdits)} />}
          <MenuItem icon="rotate-left" label={t("mobile.edit.resetAll")} onClick={fromMenu(resetAll)} />
        </Dropdown>
      )}
    </div>
  );
}
