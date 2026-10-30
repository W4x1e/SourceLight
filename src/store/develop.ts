import { api } from "../lib/api";
import { errorText } from "../lib/aiReady";
import type { Group } from "../bindings";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";
import type { DevelopInfo, EditStack } from "../types";
import type { Slice, State } from "./types";

/**
 * Develop: the photo open in it, its edit, and the actions that change the edit.
 */

/** The photos opened in Develop since it last closed, for `closeDevelop`. */
const developTouched = new Set<number>();

/** The stack fields the full "Auto" writes — shared by the toggle logic and the
 *  button's active-state check so the two can't disagree about what "on" means. */
export const AUTO_FIELDS: (keyof EditStack)[] = [
  "exposure",
  "whites",
  "blacks",
  "highlights",
  "shadows",
  "warmth",
  "tint",
];

/**
 * Whether Develop is on screen: open on a photo, or opening its first one,
 * the raw still decoding (a second or more). What stands aside for Develop
 * asks this rather than `develop !== null`, so it is there the moment it is
 * asked for, and the library's keys stop acting under it then too. It used to
 * appear only once the photo had decoded, with nothing to say a key had been
 * pressed in the seconds before.
 */
export function developShown(s: Pick<State, "develop" | "developWanted">): boolean {
  return s.develop !== null || s.developWanted !== null;
}

export interface DevelopSlice {
  /** Develop's copied settings: the edit as it was when copied, and
   *  which parts — kept here so they outlive Develop. */
  settingsClipboard: { stack: EditStack; groups: Group[] } | null;
  setSettingsClipboard: (c: { stack: EditStack; groups: Group[] } | null) => void;
  /** The photo open in Develop before the current one, for "Paste previous". */
  previousDevelopId: number | null;
  setPreviousDevelopId: (id: number | null) => void;

  /** Non-null while the develop view is open. */
  develop: DevelopInfo | null;
  developError: string | null;
  /** Set while a full "Auto" is applied and untouched, holding the pre-auto and
   *  post-auto stacks so clicking Auto again can toggle it back off. Cleared the
   *  moment the auto-owned fields are edited by hand. */
  autoActive: { before: EditStack; after: EditStack } | null;
  /** Replaces the develop state after a command that returns fresh info. */
  setDevelopInfo: (info: DevelopInfo) => void;
  /** The photo Develop was last asked to open, which may not be open yet:
   *  stepping goes on from it, and a slower answer for another is dropped. */
  developWanted: number | null;
  openDevelop: (photoId: number) => Promise<void>;
  closeDevelop: () => Promise<void>;
  adjust: (label: string, patch: Partial<EditStack>) => Promise<void>;
  undoDevelop: () => Promise<void>;
  redoDevelop: () => Promise<void>;
  /** Toggle full auto (tone + white balance). First click applies and remembers
   *  the pre-auto state; a second click (while untouched) reverts the auto-owned
   *  fields — a toggle rather than reaching for Undo. After a manual edit it
   *  re-applies fresh instead of reverting, so hand edits are never discarded. */
  toggleAuto: () => Promise<void>;
  /** Auto white balance only — gray-world warmth/tint from the frame's
   *  near-neutral midtones, one undoable step (no-op if too little neutral
   *  reference). Lives in the White Balance panel; a plain apply, not a toggle. */
  autoWb: () => Promise<void>;
  /** White balance from a point the user says is grey, in framed UV. */
  pickWb: (x: number, y: number) => Promise<void>;
  resetDevelop: () => Promise<void>;
  /** Keeps the current edits as a virtual copy beside the original, puts the
   *  original back to how it was when opened, and moves Develop onto the copy.
   *  Reloads so the copy appears in the grid. There is no plain "save": every
   *  edit is written as it is made. */
  saveDevelopAsCopy: (name: string | null) => Promise<void>;
}

export const createDevelopSlice: Slice<DevelopSlice> = (set, get) => ({
  settingsClipboard: null,
  setSettingsClipboard: (c) => set({ settingsClipboard: c }),
  previousDevelopId: null,
  setPreviousDevelopId: (id) => set({ previousDevelopId: id }),

  develop: null,
  developError: null,
  autoActive: null,

  setDevelopInfo: (info) => set({ develop: info }),

  developWanted: null,
  openDevelop: async (photoId) => {
    // The photo asked for last wins: stepping fast asks for several
    // before the first is open, and an answer for one since passed over
    // is dropped rather than shown for a moment.
    set({ developError: null, autoActive: null, developWanted: photoId });
    try {
      // Decode plus demosaic — the one slow step. Everything after this is
      // a single fullscreen pass.
      const opened = await api.openDevelop(photoId);
      if (get().developWanted !== photoId) {
        // Left while it was opening: the core has it open all the same,
        // and holds automatic analysis back while it is.
        if (get().developWanted === null && get().develop === null) void api.closeDevelop();
        return;
      }
      developTouched.add(photoId);
      set({ develop: opened, lightboxIndex: null });
    } catch (e) {
      if (get().developWanted !== photoId || String(e).includes("superseded by a later open")) return;
      // Said, and the photo already open stays. Stepping onto one
      // that would not open — a reader unplugged, a damaged file, a body
      // LibRaw doesn't know — closed Develop for the grid with nothing
      // said, and D on it did nothing. `developWanted` stays on it, so the
      // next step goes on from there.
      const st = get();
      const name =
        (st.searchResults ?? st.photos).find((p) => p.id === photoId)?.filename ??
        t("store.develop.thisPhoto");
      const said = t("store.develop.openFailed", { name, error: errorText(e) });
      if (st.develop) {
        set({ developError: said });
      } else {
        // Nothing open yet: Develop, there since it was asked for, goes
        // again, and the toast says why.
        set({ developError: said, developWanted: null });
        toast(said, "error");
      }
    }
  },

  closeDevelop: async () => {
    const open = get().develop?.photoId;
    set({ develop: null, developError: null, developWanted: null });
    await api.closeDevelop();
    // Edits change how a photo looks in the grid once previews are
    // regenerated, and its rating, flag or tags may have moved too: only
    // the photos it opened, not the page. A copy saved on the way
    // reloaded the page when it was made.
    if (open != null) developTouched.add(open);
    const touched = [...developTouched];
    developTouched.clear();
    await get().refreshPhotos(touched);
  },

  adjust: async (label, patch) => {
    const current = get().develop;
    if (!current) return;
    const next = { ...current.stack, ...patch };
    // A new framing, with the masks and spots as they were: they are
    // carried along with the content. They used to stay put in the
    // frame while the photo moved under them.
    const reframed =
      JSON.stringify(next.geometry) !== JSON.stringify(current.stack.geometry) &&
      ((next.masks?.length ?? 0) > 0 || (next.spots?.length ?? 0) > 0) &&
      JSON.stringify(next.masks) === JSON.stringify(current.stack.masks) &&
      JSON.stringify(next.spots) === JSON.stringify(current.stack.spots);
    try {
      set({ develop: await (reframed ? api.reframeDevelop(label, next) : api.setDevelopStack(label, next)) });
    } catch (e) {
      set({ developError: String(e) });
    }
  },

  undoDevelop: async () => {
    if (!get().develop) return;
    set({ develop: await api.undoDevelop() });
  },

  redoDevelop: async () => {
    if (!get().develop) return;
    set({ develop: await api.redoDevelop() });
  },

  toggleAuto: async () => {
    const dev = get().develop;
    if (!dev) return;
    const active = get().autoActive;
    // "On and untouched" → toggle off: restore just the auto-owned fields to
    // their pre-auto values, leaving any other edits made since in place.
    if (active && AUTO_FIELDS.every((f) => dev.stack[f] === active.after[f])) {
      const restored = { ...dev.stack };
      // All AUTO_FIELDS are numeric scalars; the record cast sidesteps TS's
      // (unsound) correlated-index-write check without loosening the type.
      const dst = restored as Record<string, number>;
      const src = active.before as unknown as Record<string, number>;
      for (const f of AUTO_FIELDS) dst[f] = src[f]!;
      set({ develop: await api.setDevelopStack("Auto off", restored), autoActive: null });
      return;
    }
    // Otherwise apply (fresh, snapshotting whatever's current — so a manual
    // edit since the last Auto is never discarded, just re-based).
    const before = dev.stack;
    const info = await api.autoAllDevelop();
    set({ develop: info, autoActive: { before, after: info.stack } });
  },

  autoWb: async () => {
    if (!get().develop) return;
    // A plain apply; also clears any toggle state since warmth/tint changed.
    set({ develop: await api.autoWbDevelop(), autoActive: null });
  },

  pickWb: async (x, y) => {
    if (!get().develop) return;
    try {
      set({ develop: await api.pickWbDevelop(x, y), autoActive: null });
    } catch (e) {
      // "Too dark to read": a hint, not a failure.
      toast(String(e), "info");
    }
  },

  resetDevelop: async () => {
    if (!get().develop) return;
    // The backend's own default, so a new parameter is reset too, and the
    // erases, which a stack sent from here never changes. The version
    // too: reset means "as a new photo opens", and an old stack opens
    // where it was edited, not at the camera's own brightness.
    try {
      set({ develop: await api.resetDevelop() });
    } catch (e) {
      set({ developError: String(e) });
    }
  },

  saveDevelopAsCopy: async (name) => {
    if (!get().develop) return;
    try {
      // The session now follows the copy, so the info that comes back is
      // the copy's — its id is what the filmstrip and grid should mark.
      // `developWanted` follows too, or "Opening <original>…" stays up.
      const copy = await api.saveDevelopAsCopy(name);
      developTouched.add(copy.photoId);
      set({ develop: copy, developWanted: copy.photoId });
      // Reload so the copy is in the grid behind the overlay — the point of
      // making one is seeing it beside the original.
      await get().reload();
      toast(
        name ? t("store.develop.copySavedNamed", { name }) : t("store.develop.copySaved"),
        "success",
      );
    } catch (e) {
      set({ developError: String(e) });
    }
  },
});
