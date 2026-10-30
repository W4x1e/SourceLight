import { api } from "../lib/api";
import { confirm, toast } from "../lib/dialogs";
import { t } from "../i18n";
import { humanBytes } from "../lib/format";
import { trashName } from "../lib/platform";
import type { Photo } from "../types";
import { DUPLICATE_THRESHOLD } from "./compare";
import type { Slice } from "./types";

/**
 * Cull: its preferences, its place in the queue, and the suggested verdicts.
 */

/**
 * A cull pass's place in its queue.
 *
 * By photo id, with the index as the fallback: an id survives the set around
 * it changing (new imports, a different sort), but not the frame itself
 * leaving the strip — rejecting the last frame viewed in "To review" does
 * exactly that — and then the same position in the queue is the right place.
 */
export interface CullSession {
  photoId: number;
  index: number;
  strip: "all" | "unrated" | "picks" | "rejects" | "suggested";
  view: "loupe" | "survey";
  /** The set the position belongs to (`cullScopeKey`); resumed only
   *  on the same one. */
  scope?: string;
}

export interface CullSlice {
  /** Cull preferences, persisted: whether a verdict jumps to the next frame,
   *  and the loupe/filmstrip ordering. */
  cullAutoAdvance: boolean;
  /** The loupe keeps its zoom and pan from frame to frame. */
  cullKeepZoom: boolean;
  setCullKeepZoom: (v: boolean) => void;
  setCullAutoAdvance: (v: boolean) => void;
  cullSort: "capture" | "sharpness";
  setCullSort: (v: "capture" | "sharpness") => void;
  /** Where the cull pass is, persisted so a 2,000-frame job survives a
   *  restart. The verdicts are already in the catalogue; this is the
   *  place in the queue. */
  cullSession: CullSession | null;
  setCullSession: (s: CullSession | null) => void;
  /** Whether Cull shows its suggested verdicts, persisted with the rest
   *  of the pass. The suggestions themselves are recomputed from the photos,
   *  so they follow every verdict made since. */
  cullSuggest: boolean;
  setCullSuggest: (v: boolean) => void;
  /** Bursts and duplicates across the library, as `duplicate_groups` returns
   *  them — what the suggestions choose a best frame from. Fetched when
   *  suggestions are shown; `null` until then. */
  cullGroups: number[][] | null;
  loadCullGroups: () => Promise<void>;
  /** Make suggested flags real, as one undoable change, and patch the photos
   *  in place everywhere they appear. */
  /** Sets these flags as one undoable change, named `what` for Undo. */
  /** `suggested`: the machine's proposals, accepted in bulk — kept marked as
   *  such. A rule the person chose (Resolve all) is theirs. */
  acceptSuggestions: (flags: { id: number; flag: number }[], what?: string, suggested?: boolean) => Promise<boolean>;
  /** Moves rejects' files to the Recycle Bin after confirming, then forgets
   *  them. */
  binRejects: (ids: number[]) => Promise<void>;
  /** Deletes photos after confirming: a virtual copy's row only, an
   *  original's file to the Recycle Bin with its copies. */
  deletePhotos: (photos: Photo[]) => Promise<void>;
}

export const createCullSlice: Slice<CullSlice> = (set, get) => ({
  cullAutoAdvance: true,
  setCullAutoAdvance: (v) => set({ cullAutoAdvance: v }),
  cullKeepZoom: true,
  setCullKeepZoom: (v) => set({ cullKeepZoom: v }),
  cullSort: "capture",
  setCullSort: (v) => set({ cullSort: v }),
  cullSession: null,
  setCullSession: (v) => set({ cullSession: v }),
  cullSuggest: false,
  setCullSuggest: (v) => set({ cullSuggest: v }),
  cullGroups: null,
  loadCullGroups: async () => {
    try {
      // The frames being culled, not the whole library.
      const scope = (get().searchResults ?? get().photos).map((p) => p.id);
      set({ cullGroups: await api.duplicateGroups(DUPLICATE_THRESHOLD, scope) });
    } catch (e) {
      console.error("burst grouping for suggestions failed", e);
      // Without bursts every frame is judged on its own — still useful, so
      // suggest on rather than fail the whole feature.
      set({ cullGroups: [] });
    }
  },
  acceptSuggestions: async (flags, what, suggested = false) => {
    if (flags.length === 0) return false;
    try {
      await api.setFlags(flags, what ?? null, suggested);
    } catch (e) {
      console.error("accepting suggestions failed", e);
      set({ error: String(e) });
      return false;
    }
    // Patched in place, like `compareApply`, so the frame on screen stays put.
    const byId = new Map(flags.map((f) => [f.id, f.flag]));
    const apply = (p: Photo): Photo => {
      const flag = byId.get(p.id);
      return flag === undefined ? p : { ...p, flag, flagSuggested: suggested && flag !== 0 };
    };
    const compare = get().compare;
    set({
      photos: get().photos.map(apply),
      searchResults: get().searchResults?.map(apply) ?? null,
      lightboxSet: get().lightboxSet?.map(apply) ?? null,
      compare: compare ? { ...compare, sets: compare.sets.map((s) => s.map(apply)) } : null,
    });
    return true;
  },

  binRejects: async (ids) => {
    if (ids.length === 0) return;
    const n = ids.length;
    const trash = trashName();
    const ok = await confirm({
      title: t("store.bin.title", { count: n, trash }),
      message: t("store.bin.message", { trash }),
      confirmLabel: t("store.bin.confirm", { trash }),
      danger: true,
    });
    if (!ok) return;
    try {
      const out = await api.binRejects(ids);
      const parts: string[] = [];
      if (out.files > 0) {
        parts.push(t("store.bin.moved", { count: out.files, trash, freed: humanBytes(out.bytes ?? 0) }));
      }
      if (out.kept > 0) {
        parts.push(t("store.bin.kept", { count: out.kept }));
      }
      if (out.missing > 0) {
        parts.push(t("store.bin.skipped", { count: out.missing }));
      }
      toast(parts.join(" ") || t("store.bin.nothing"), out.files > 0 ? "success" : "info");
    } catch (e) {
      toast(String(e), "error");
    }
    get().clearSelection();
    await get().reload();
  },

  deletePhotos: async (photos) => {
    if (photos.length === 0) return;
    const n = photos.length;
    const trash = trashName();
    const copiesOnly = photos.every((p) => p.variant > 0);
    const ok = await confirm({
      title: t(copiesOnly ? "store.delete.copiesTitle" : "store.delete.title", { count: n }),
      message: copiesOnly ? t("store.delete.copiesMessage") : t("store.delete.message", { trash }),
      confirmLabel: copiesOnly ? t("store.delete.confirm") : t("store.bin.confirm", { trash }),
      danger: true,
    });
    if (!ok) return;
    try {
      const out = await api.deletePhotos(photos.map((p) => p.id));
      const parts: string[] = [];
      if (out.files > 0) {
        parts.push(t("store.delete.moved", { count: out.files, trash, freed: humanBytes(out.bytes ?? 0) }));
      } else if (out.rows > 0) {
        parts.push(t("store.delete.deleted", { count: out.rows }));
      }
      if (out.missing > 0) {
        parts.push(t("store.bin.skipped", { count: out.missing }));
      }
      toast(parts.join(" ") || t("store.bin.nothing"), out.rows > 0 ? "success" : "info");
    } catch (e) {
      toast(String(e), "error");
    }
    get().clearSelection();
    await get().reload();
    // The Lightbox stays open, on the photo after. A set of its own (a person,
    // a place) isn't touched by a reload, so it is re-read.
    const frames = get().lightboxSet;
    if (frames) {
      const kept = new Set((await api.getPhotosByIds(frames.map((p) => p.id))).map((p) => p.id));
      if (get().lightboxSet === frames) set({ lightboxSet: frames.filter((p) => kept.has(p.id)) });
    }
    const index = get().lightboxIndex;
    if (index !== null) {
      const len = (get().lightboxSet ?? get().searchResults ?? get().photos).length;
      set({ lightboxIndex: len === 0 ? null : Math.min(index, len - 1) });
    }
  },
});
