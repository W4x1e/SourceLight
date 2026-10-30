/**
 * Editable keyboard shortcuts.
 *
 * One registry of named actions, each with a context it applies in and a default
 * key that follows the convention photographers expect (ratings 1–5, flags P/X/U,
 * colour labels 6–9, `\` before/after, ⌘Z undo …). The user can rebind any of
 * them; the store persists overrides and the three keyboard surfaces (library
 * grid, lightbox, develop) resolve an event through `resolveAction` rather than
 * hard-coding `e.key`.
 */

import { t, type Key } from "../i18n";

/** A Mac, whose menu bar carries some of the app's keys itself. */
const MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

/** `global` works wherever the app is, bar a text field or an open dialog. */
export type Context = "library" | "lightbox" | "develop" | "compare" | "cull" | "people" | "global";

export type ActionId =
  | "rate-0" | "rate-1" | "rate-2" | "rate-3" | "rate-4" | "rate-5"
  | "flag-pick" | "flag-reject" | "flag-none"
  | "label-1" | "label-2" | "label-3" | "label-4" | "label-5" | "label-none"
  | "undo-verdict"
  | "accept-suggestion"
  | "prev-photo" | "next-photo" | "next-undecided" | "open-develop" | "close-overlay"
  | "toggle-filters" | "select-all" | "clear-selection"
  | "toggle-zoom" | "develop-zoom-in" | "develop-zoom-out"
  | "compare-keep" | "compare-reject-extras" | "compare-prev-set" | "compare-next-set"
  | "develop-undo" | "develop-redo" | "develop-save"
  | "develop-before-after" | "develop-crop" | "develop-mask" | "develop-reset" | "develop-wb-picker"
  | "people-same" | "people-different" | "people-skip"
  | "develop-copy-settings" | "develop-paste-settings"
  | "open-palette" | "mode-browse" | "mode-cull" | "mode-places" | "mode-people" | "focus-search"
  | "add-folder" | "import-card" | "export-selected" | "open-settings" | "show-activity" | "close-view"
  | "toggle-fullscreen"
  | "copy-image"
  | "toggle-peaking" | "cycle-grid";

export interface ActionDef {
  id: ActionId;
  /** Shown through `t` where it is displayed: a table at module scope cannot hold the words. */
  label: Key;
  group: Key;
  contexts: Context[];
  /** Default combo, in the canonical form `eventToCombo` produces. "" = unbound. */
  def: string;
}

/** Anywhere a single photo is the subject of the keystroke: the grid selection,
 *  the lightbox photo, the focused frame in compare, and the current frame in
 *  Cull. */
const PHOTO: Context[] = ["library", "lightbox", "compare", "cull"];
/** A verdict is given in Develop too — but not undone there with the
 *  same key: Ctrl+Z in Develop is the edit's own undo. */
const VERDICT: Context[] = [...PHOTO, "develop"];

export const ACTIONS: ActionDef[] = [
  { id: "rate-1", label: "shell.keymap.action.rate1", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "1" },
  { id: "rate-2", label: "shell.keymap.action.rate2", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "2" },
  { id: "rate-3", label: "shell.keymap.action.rate3", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "3" },
  { id: "rate-4", label: "shell.keymap.action.rate4", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "4" },
  { id: "rate-5", label: "shell.keymap.action.rate5", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "5" },
  { id: "rate-0", label: "shell.keymap.action.rate0", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "0" },
  // C beside X, so one hand makes both calls while the other steps frames.
  // Pick was P, another editor's key, which sits across the keyboard from X.
  { id: "flag-pick", label: "shell.keymap.action.flagPick", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "c" },
  { id: "flag-reject", label: "shell.keymap.action.flagReject", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "x" },
  { id: "flag-none", label: "shell.keymap.action.flagNone", group: "shell.keymap.group.ratingAndFlags", contexts: VERDICT, def: "u" },

  { id: "label-1", label: "shell.keymap.action.label1", group: "shell.keymap.group.colourLabels", contexts: VERDICT, def: "6" },
  { id: "label-2", label: "shell.keymap.action.label2", group: "shell.keymap.group.colourLabels", contexts: VERDICT, def: "7" },
  { id: "label-3", label: "shell.keymap.action.label3", group: "shell.keymap.group.colourLabels", contexts: VERDICT, def: "8" },
  { id: "label-4", label: "shell.keymap.action.label4", group: "shell.keymap.group.colourLabels", contexts: VERDICT, def: "9" },
  { id: "label-5", label: "shell.keymap.action.label5", group: "shell.keymap.group.colourLabels", contexts: VERDICT, def: "" },
  { id: "label-none", label: "shell.keymap.action.labelNone", group: "shell.keymap.group.colourLabels", contexts: VERDICT, def: "" },
  // The same key as Develop's undo: in the library it takes back the last
  // rating, flag or label instead. The two never share a context, so they
  // do not conflict.
  { id: "undo-verdict", label: "shell.keymap.action.undoVerdict", group: "shell.keymap.group.ratingAndFlags", contexts: PHOTO, def: "mod+z" },
  // A, beside C and X: the three calls a suggested cull asks for sit under one hand.
  { id: "accept-suggestion", label: "shell.keymap.action.acceptSuggestion", group: "shell.keymap.group.ratingAndFlags", contexts: ["cull"], def: "a" },

  { id: "prev-photo", label: "shell.keymap.action.prevPhoto", group: "shell.keymap.group.navigation", contexts: ["lightbox", "compare", "cull", "develop"], def: "arrowleft" },
  { id: "next-photo", label: "shell.keymap.action.nextPhoto", group: "shell.keymap.group.navigation", contexts: ["lightbox", "compare", "cull", "develop"], def: "arrowright" },
  { id: "next-undecided", label: "shell.keymap.action.nextUndecided", group: "shell.keymap.group.navigation", contexts: ["cull"], def: "n" },
  { id: "open-develop", label: "shell.keymap.action.openDevelop", group: "shell.keymap.group.navigation", contexts: PHOTO, def: "d" },
  // In Compare it zooms every frame of the set together.
  { id: "toggle-zoom", label: "shell.keymap.action.toggleZoom", group: "shell.keymap.group.navigation", contexts: ["lightbox", "cull", "compare", "develop"], def: "z" },
  // Works with nothing selected: it changes the view, not a photo.
  { id: "toggle-filters", label: "shell.keymap.action.toggleFilters", group: "shell.keymap.group.navigation", contexts: ["library"], def: "f" },
  // ⌘A selects every photo in view, as in Finder and Photos. Without it the
  // keystroke fell through to the webview, which selected the page's text.
  { id: "select-all", label: "shell.keymap.action.selectAll", group: "shell.keymap.group.navigation", contexts: ["library"], def: "mod+a" },
  // Its opposite, on the key every overlay already closes with: the grid had
  // no way out of a selection but the panel's × (or clicking a single photo).
  { id: "clear-selection", label: "shell.keymap.action.clearSelection", group: "shell.keymap.group.navigation", contexts: ["library"], def: "escape" },
  // Ctrl+C copies the photo, rendered with its edits. Selected text
  // still copies as text: every view checks for it before taking the key.
  { id: "copy-image", label: "shell.keymap.action.copyImage", group: "shell.keymap.group.navigation", contexts: VERDICT, def: "mod+c" },
  // Viewing aids for judging a frame. P was Pick's key before C took
  // it; G is free everywhere these apply. In Develop, G changes the crop guide.
  { id: "toggle-peaking", label: "shell.keymap.action.togglePeaking", group: "shell.keymap.group.navigation", contexts: ["lightbox", "cull", "compare"], def: "p" },
  { id: "cycle-grid", label: "shell.keymap.action.cycleGrid", group: "shell.keymap.group.navigation", contexts: ["lightbox", "cull", "compare", "develop"], def: "g" },
  { id: "close-overlay", label: "shell.keymap.action.closeOverlay", group: "shell.keymap.group.navigation", contexts: ["lightbox", "develop", "compare", "cull"], def: "escape" },

  { id: "compare-keep", label: "shell.keymap.action.compareKeep", group: "shell.keymap.group.compare", contexts: ["compare"], def: "k" },
  { id: "compare-reject-extras", label: "shell.keymap.action.compareRejectExtras", group: "shell.keymap.group.compare", contexts: ["compare"], def: "shift+x" },
  { id: "compare-prev-set", label: "shell.keymap.action.comparePrevSet", group: "shell.keymap.group.compare", contexts: ["compare"], def: "[" },
  { id: "compare-next-set", label: "shell.keymap.action.compareNextSet", group: "shell.keymap.group.compare", contexts: ["compare"], def: "]" },

  { id: "develop-undo", label: "shell.keymap.action.developUndo", group: "shell.keymap.group.develop", contexts: ["develop"], def: "mod+z" },
  { id: "develop-redo", label: "shell.keymap.action.developRedo", group: "shell.keymap.group.develop", contexts: ["develop"], def: "mod+shift+z" },
  { id: "develop-save", label: "shell.keymap.action.developSave", group: "shell.keymap.group.develop", contexts: ["develop"], def: "mod+s" },
  { id: "develop-before-after", label: "shell.keymap.action.developBeforeAfter", group: "shell.keymap.group.develop", contexts: ["develop"], def: "\\" },
  { id: "develop-crop", label: "shell.keymap.action.developCrop", group: "shell.keymap.group.develop", contexts: ["develop"], def: "r" },
  { id: "develop-mask", label: "shell.keymap.action.developMask", group: "shell.keymap.group.develop", contexts: ["develop"], def: "m" },
  { id: "develop-wb-picker", label: "shell.keymap.action.developWbPicker", group: "shell.keymap.group.develop", contexts: ["develop"], def: "w" },
  // Plain keys, not Ctrl: Ctrl +/− scale the whole window.
  { id: "develop-zoom-in", label: "shell.keymap.action.developZoomIn", group: "shell.keymap.group.develop", contexts: ["develop"], def: "=" },
  { id: "develop-zoom-out", label: "shell.keymap.action.developZoomOut", group: "shell.keymap.group.develop", contexts: ["develop"], def: "-" },
  { id: "develop-reset", label: "shell.keymap.action.developReset", group: "shell.keymap.group.develop", contexts: ["develop"], def: "mod+shift+r" },
  // Selective copy and paste: choose the parts, then paste them on.
  // Anywhere: the palette, the four workspaces, and search.
  { id: "open-palette", label: "shell.keymap.action.openPalette", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+k" },
  { id: "mode-browse", label: "shell.keymap.action.modeBrowse", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+1" },
  { id: "mode-cull", label: "shell.keymap.action.modeCull", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+2" },
  { id: "mode-places", label: "shell.keymap.action.modePlaces", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+3" },
  { id: "mode-people", label: "shell.keymap.action.modePeople", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+4" },
  { id: "focus-search", label: "shell.keymap.action.focusSearch", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+f" },
  // The keys a desktop app answers to (#Windows native). A Mac's menu bar
  // already serves them; Windows has no menu bar, so without these the keys
  // people type without thinking did nothing there.
  { id: "add-folder", label: "shell.keymap.action.addFolder", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+o" },
  { id: "import-card", label: "shell.keymap.action.importCard", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+shift+i" },
  { id: "export-selected", label: "shell.keymap.action.exportSelected", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+shift+e" },
  { id: "open-settings", label: "shell.keymap.action.openSettings", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+," },
  { id: "show-activity", label: "shell.keymap.action.showActivity", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+alt+a" },
  { id: "close-view", label: "shell.keymap.action.closeView", group: "shell.keymap.group.app", contexts: ["global"], def: "mod+w" },
  // F11 is Windows' full screen; a Mac has ⌃⌘F in its View menu.
  { id: "toggle-fullscreen", label: "shell.keymap.action.toggleFullscreen", group: "shell.keymap.group.app", contexts: ["global"], def: MAC ? "" : "f11" },
  { id: "develop-copy-settings", label: "shell.keymap.action.developCopySettings", group: "shell.keymap.group.develop", contexts: ["develop"], def: "mod+shift+c" },
  { id: "develop-paste-settings", label: "shell.keymap.action.developPasteSettings", group: "shell.keymap.group.develop", contexts: ["develop"], def: "mod+shift+v" },
  // "Same person?" in People: they were hard-coded, so not rebindable
  // and missing from Shortcuts.
  { id: "people-same", label: "shell.keymap.action.peopleSame", group: "shell.keymap.group.people", contexts: ["people"], def: "y" },
  { id: "people-different", label: "shell.keymap.action.peopleDifferent", group: "shell.keymap.group.people", contexts: ["people"], def: "n" },
  { id: "people-skip", label: "shell.keymap.action.peopleSkip", group: "shell.keymap.group.people", contexts: ["people"], def: "s" },
];

export const DEFAULT_KEYMAP: Record<ActionId, string> = Object.fromEntries(
  ACTIONS.map((a) => [a.id, a.def]),
) as Record<ActionId, string>;

/**
 * Defaults that have since changed. The persisted keymap stores every binding,
 * not just the ones someone changed, so an install from before a change would
 * otherwise keep the old key forever and never see the new one.
 */
const RETIRED_DEFAULTS: Partial<Record<ActionId, string>> = {
  "flag-pick": "p",
};

/**
 * A saved keymap merged over today's defaults. A binding still on a retired
 * default moves to the new one. A key the person chose stays, because a key
 * that happens to equal the old default was almost certainly never chosen.
 */
export function migrateKeymap(saved: Partial<Record<ActionId, string>> | undefined): Record<ActionId, string> {
  const map = { ...DEFAULT_KEYMAP, ...(saved ?? {}) };
  for (const [id, old] of Object.entries(RETIRED_DEFAULTS) as [ActionId, string][]) {
    // Only when nothing else now claims the new key, or the move would create a clash.
    const taken = Object.entries(map).some(([other, combo]) => other !== id && combo === DEFAULT_KEYMAP[id]);
    if (map[id] === old && !taken) map[id] = DEFAULT_KEYMAP[id];
  }
  return map;
}

/**
 * Canonical string for a key event: modifiers (Ctrl/Cmd folded into `mod`) in a
 * fixed order, then the lowercased key. Returns "" for a bare modifier press so
 * holding Shift alone never resolves to an action.
 */
export function eventToCombo(e: KeyboardEvent): string {
  const k = e.key;
  if (k === "Control" || k === "Meta" || k === "Shift" || k === "Alt" || k === "AltGraph") return "";
  const parts = modifiers(e);
  if (e.shiftKey) parts.push("shift");
  let key = k.toLowerCase();
  if (key === " ") key = "space";
  parts.push(key);
  return parts.join("+");
}

/** Ctrl/⌘ and Alt, except as AltGr: on a Windows German, French or
 *  Spanish layout, `\`, `[` and `]` are typed with AltGr, which the browser
 *  reports as Ctrl+Alt, so they arrived as "mod+alt+\\" and matched nothing. */
function modifiers(e: KeyboardEvent): string[] {
  const parts: string[] = [];
  if (e.getModifierState?.("AltGraph")) return parts;
  if (e.ctrlKey || e.metaKey) parts.push("mod");
  if (e.altKey) parts.push("alt");
  return parts;
}

/** The keys looked up by where they are when what they type finds nothing:
 *  the number row, the brackets and the backslash. */
const PHYSICAL: Record<string, string> = {
  Backslash: "\\",
  IntlBackslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
};

/**
 * The combo for the physical key, for a layout whose characters differ:
 * on French AZERTY the number row types `&`, `é`, `"` … and gives
 * digits only with Shift, so the ratings 1–5 and labels 6–9 did nothing
 * without a numpad. A digit key is its digit, Shift or not; the brackets and
 * backslash are themselves wherever the layout puts their characters.
 * Empty when the physical key is not one of those.
 */
export function physicalCombo(e: KeyboardEvent): string {
  const code = e.code ?? "";
  const digit = /^(?:Digit|Numpad)([0-9])$/.exec(code)?.[1];
  const key = digit ?? PHYSICAL[code];
  if (!key) return "";
  const parts = modifiers(e);
  if (!digit && e.shiftKey) parts.push("shift");
  parts.push(key);
  return parts.join("+");
}

const IS_MAC_PLATFORM = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
const modLabel = () => (IS_MAC_PLATFORM ? "⌘" : t("shell.keymap.key.ctrl"));
const KEY_LABEL: Record<string, () => string> = {
  arrowleft: () => "←", arrowright: () => "→", arrowup: () => "↑", arrowdown: () => "↓",
  escape: () => t("shell.keymap.key.escape"), space: () => t("shell.keymap.key.space"),
  "\\": () => "\\", enter: () => t("shell.keymap.key.enter"),
};

/** Human-readable form of a combo, for the settings UI and tooltips. */
export function comboToLabel(combo: string): string {
  if (!combo) return "—";
  return combo
    .split("+")
    .map((p) => {
      if (p === "mod") return modLabel();
      if (p === "shift") return "⇧";
      if (p === "alt") return t("shell.keymap.key.alt");
      const named = KEY_LABEL[p];
      // Other named keys (Tab, Delete, F11 …) are shown as the browser names them.
      return named ? named() : p.length === 1 ? p.toUpperCase() : p.replace(/^\w/, (c) => c.toUpperCase());
    })
    .join(IS_MAC_PLATFORM ? "" : "+");
}

/**
 * A keydown that names its action outright — what a menu-bar item sends
 * (`lib/appMenu.ts`), so the item runs through exactly the handler its key
 * would, in whatever context is open, and still works for an action with no
 * key bound.
 */
export function actionEvent(id: ActionId): KeyboardEvent {
  const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true });
  Object.defineProperty(e, "actionId", { value: id });
  return e;
}

/**
 * The action bound to this event in the given context, or null. First match
 * wins; the settings UI prevents two actions in one context sharing a combo.
 * An `actionEvent` resolves to its own action wherever that action applies.
 */
export function resolveAction(
  keymap: Record<ActionId, string>,
  context: Context,
  e: KeyboardEvent,
): ActionId | null {
  const named = (e as KeyboardEvent & { actionId?: ActionId }).actionId;
  if (named) return ACTIONS.find((a) => a.id === named)?.contexts.includes(context) ? named : null;
  const combo = eventToCombo(e);
  if (!combo) return null;
  const find = (c: string) =>
    ACTIONS.find((a) => a.contexts.includes(context) && keymap[a.id] && keymap[a.id] === c)?.id ?? null;
  // By the character first; by the key's place on the board when that finds
  // nothing, for a layout that types something else there.
  const physical = physicalCombo(e);
  return find(combo) ?? (physical && physical !== combo ? find(physical) : null);
}

/** Actions that would collide with `combo` in any context shared with `actionId`. */
export function conflicts(
  keymap: Record<ActionId, string>,
  actionId: ActionId,
  combo: string,
): ActionId[] {
  if (!combo) return [];
  const self = ACTIONS.find((a) => a.id === actionId)!;
  // A global key is live everywhere, so it clashes with a key in any context.
  const overlap = (a: ActionDef) =>
    a.contexts.includes("global") ||
    self.contexts.includes("global") ||
    a.contexts.some((c) => self.contexts.includes(c));
  return ACTIONS.filter((a) => a.id !== actionId && keymap[a.id] === combo && overlap(a)).map((a) => a.id);
}
