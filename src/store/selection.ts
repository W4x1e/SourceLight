import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";
import type { Photo } from "../types";
import type { Slice } from "./types";

/**
 * The selection, and the ratings, flags and labels set on it.
 */

export type SelectMode = "single" | "toggle" | "range";

export interface SelectionSlice {
  selected: Set<number>;
  /** The photo a Shift-click extends a range from. */
  anchorId: number | null;
  /** The photo the selection panel and rail actions treat as focused — the last
   *  one clicked. Held apart from the selection set so "Develop this" and
   *  "Similar to this" always have a single target even inside a multi-select. */
  focusId: number | null;

  select: (id: number, mode: SelectMode) => void;
  /** Replace the selection with an explicit id list — the cull "review picks"
   *  handoff sets it to the picks, then switches to browse where the selection
   *  panel (export, collections, tags) takes over. */
  selectPhotos: (ids: number[]) => void;
  clearSelection: () => void;

  rateSelection: (rating: number) => Promise<void>;
  flagSelection: (flag: number) => Promise<void>;
  colorLabelSelection: (label: number) => Promise<void>;
  /** Takes back the last rating, flag or colour-label change this session. */
  undoVerdict: () => Promise<void>;
}

export const createSelectionSlice: Slice<SelectionSlice> = (set, get) => ({
  selected: new Set<number>(),
  anchorId: null,
  focusId: null,

  select: (id, mode) => {
    // Ranges run over what the grid shows: search results when
    // searching. They ran over the library list underneath, so Shift-
    // clicking from the first look-alike to the fifth selected the
    // thousands of photos between their library positions. The anchor is
    // a photo, not a position, so a list that changes under it can't
    // move it.
    const { searchResults, photos, selected, anchorId } = get();
    const shown = searchResults ?? photos;

    if (mode === "toggle") {
      const next = new Set(selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      set({ selected: next, anchorId: id, focusId: id });
      return;
    }

    const from = anchorId === null ? -1 : shown.findIndex((p) => p.id === anchorId);
    const to = shown.findIndex((p) => p.id === id);
    if (mode === "range" && from >= 0 && to >= 0) {
      const [a, b] = from < to ? [from, to] : [to, from];
      const next = new Set(selected);
      for (let i = a; i <= b; i++) next.add(shown[i]!.id);
      // The anchor stays, so successive shift-clicks extend from it.
      // Focus moves to the clicked frame, the one the panel describes.
      set({ selected: next, focusId: id });
      return;
    }

    set({ selected: new Set([id]), anchorId: id, focusId: id });
  },

  selectPhotos: (ids) => set({ selected: new Set(ids), anchorId: null, focusId: ids[0] ?? null }),

  clearSelection: () => set({ selected: new Set(), anchorId: null, focusId: null }),

  undoVerdict: async () => {
    let undone;
    try {
      undone = await api.undoVerdict();
    } catch (e) {
      set({ error: String(e) });
      return;
    }
    if (!undone) {
      toast(t("store.undo.nothing"), "info");
      return;
    }
    // A change Undo can't take back: said so, rather than reaching past
    // it to an older, unrelated verdict.
    if (undone.beyond) {
      toast(t("store.undo.beyond", { label: undone.label }), "info");
      return;
    }
    // Copied develop settings put back: the grid's edited marks and
    // thumbnails follow from a reload.
    if (undone.settings.length > 0) {
      await get().reload();
      toast(t("store.undo.done", { label: undone.label }), "success");
      return;
    }
    // Patched in place, like `compareApply`, so undoing a cull verdict does
    // not make the frame on screen jump — and every copy of each photo (the
    // grid, search results, a compare set) shows the restored values.
    const byId = new Map(undone.restored.map((v) => [v.id, v]));
    const apply = (p: Photo): Photo => {
      const v = byId.get(p.id);
      return v ? { ...p, rating: v.rating, flag: v.flag, colorLabel: v.colorLabel } : p;
    };
    const compare = get().compare;
    set({
      photos: get().photos.map(apply),
      searchResults: get().searchResults?.map(apply) ?? null,
      lightboxSet: get().lightboxSet?.map(apply) ?? null,
      compare: compare ? { ...compare, sets: compare.sets.map((s) => s.map(apply)) } : null,
    });
    toast(t("store.undo.done", { label: undone.label }), "success");
  },

  // Rating, flags and labels all patch in place rather than reloading:
  // culling wants the photo to stay put under the cursor, not jump as the
  // sort/filter re-runs. The active filter re-applies on the next query.
  //
  // All three go through `compareApply`, which is where that patching
  // already lived. Rating used to `reload()` instead, which re-ran the
  // filter under the cursor (a photo could vanish mid-pass when `minRating`
  // was set) and replaced the `photos` array, re-triggering the grid's
  // entrance animation on every keystroke. Flags and labels patched only
  // `photos`, so a verdict was invisible while search results were showing.
  rateSelection: async (rating) => {
    await get().compareApply([...get().selected], { rating });
  },

  flagSelection: async (flag) => {
    await get().compareApply([...get().selected], { flag });
  },

  colorLabelSelection: async (label) => {
    await get().compareApply([...get().selected], { colorLabel: label });
  },
});
