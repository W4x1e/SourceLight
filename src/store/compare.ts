import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";
import { comboToLabel } from "../lib/keymap";
import { resolveAll, type ResolveOptions } from "../lib/resolveDuplicates";
import { hydrateGroups, keepPlan, orderGroupsBySharpness, stepIndex } from "../lib/survey";
import { type Photo, isFilterEmpty } from "../types";
import type { Slice, State } from "./types";

/**
 * Compare and duplicate review: sets of photos judged side by side. Every
 * verdict goes through `compareApply`, which patches photos in place.
 */

/**
 * What a duplicate review, stack or selection looks within: what the
 * grid shows when it is narrowed (a shoot, a search, any filter), else the
 * whole library — which is more than the grid's capped page.
 */
export function duplicateScope(st: State): number[] | null {
  if (!st.searchResults && isFilterEmpty(st.filter)) return null;
  return (st.searchResults ?? st.photos).map((p) => p.id);
}

/** Where a compare session came from — it changes what the header offers. */
export type CompareSource = "duplicates" | "selection";

/** Attributes the compare view can write; absent fields are left alone. */
export interface PhotoPatch {
  flag?: number;
  rating?: number;
  colorLabel?: number;
  /** The user's verdict on the eye-open reading. `null` is meaningful — it
   *  clears the override — so this is `| null`, not just optional. */
  eyesOverride?: number | null;
}

/**
 * A compare/survey session: a list of sets to judge, and where in it we are.
 *
 * Duplicate review supplies many sets and steps through them; comparing a
 * selection supplies exactly one. Both are the same view, because "look at these
 * side by side and decide" is one job.
 */
export interface CompareState {
  sets: Photo[][];
  /** Which set is on screen. */
  index: number;
  /** The photo the keyboard acts on within the current set. */
  focusId: number | null;
  source: CompareSource;
  /** Similarity cutoff the sets were grouped at (duplicates only). */
  threshold: number;
}

/** How alike two frames must look to count as duplicates. Loose enough to catch
 *  a burst, tight enough that a second shot of the same scene is not swept in.
 *  A SigLIP 2 cosine: it groups as many pairs as CLIP did at 0.92, and
 *  on a labelled library every one of them was from the same scene. */
export const DUPLICATE_THRESHOLD = 0.935;

export interface CompareSlice {
  /** Settles every duplicate group on screen by one rule. */
  resolveDuplicates: (opts: ResolveOptions) => Promise<void>;
  /** Non-null while the compare/survey view is open. */
  compare: CompareState | null;
  /** True while duplicate groups are being fetched for review. */
  compareBusy: boolean;
  /** Select the extra copies in each near-duplicate group (keeping the first),
   *  ready to flag or reject. */
  selectDuplicates: () => Promise<void>;

  /** Open the compare view on every near-duplicate group, one set at a time. */
  reviewDuplicates: (threshold?: number) => Promise<void>;
  /** Open the compare view on the current selection (needs two or more). */
  compareSelection: () => void;
  closeCompare: () => void;
  /** Move to another set; the index is clamped, and focus resets to its first
   *  photo so the keyboard always has a target. */
  compareGoTo: (index: number) => void;
  compareFocus: (photoId: number) => void;
  /** Write attributes to photos and patch them in place everywhere they appear. */
  compareApply: (photoIds: number[], patch: PhotoPatch) => Promise<void>;
  /** Resolve a set: pick this one, reject the rest, move on. */
  compareKeep: (photoId: number) => Promise<void>;
  /** Reject every extra in the set, keeping the given frame untouched (no pick
   *  flag), then move on — a softer bulk cull than `compareKeep`. */
  compareRejectExtras: (keeperId: number) => Promise<void>;
}

export const createCompareSlice: Slice<CompareSlice> = (set, get) => ({
  compare: null,
  compareBusy: false,

  resolveDuplicates: async (opts) => {
    const compare = get().compare;
    if (!compare) return;
    const r = resolveAll(compare.sets, opts);
    if (r.changes.length === 0) {
      toast(r.skipped > 0 ? t("store.resolve.allJudged") : t("store.resolve.nothing"), "info");
      return;
    }
    if (!(await get().acceptSuggestions(r.changes, "Resolved duplicates"))) return;
    const undo = get().keymap["undo-verdict"];
    toast(
      [
        t("store.resolve.done", { count: r.rejects, groups: t("store.resolve.groups", { count: r.groups }) }),
        r.skipped > 0 ? t("store.resolve.skipped", { count: r.skipped }) : "",
        undo ? t("store.resolve.undo", { key: comboToLabel(undo) }) : "",
      ]
        .filter(Boolean)
        .join(" "),
      "success",
    );
  },

  reviewDuplicates: async (threshold = DUPLICATE_THRESHOLD) => {
    set({ compareBusy: true, error: null });
    try {
      const groups = await api.duplicateGroups(threshold, duplicateScope(get()));
      const ids = [...new Set(groups.flat())];
      // One round trip for every group's photos — and by id rather than by
      // filter, since a duplicate can sit outside whatever the grid shows.
      const sets = hydrateGroups(groups, ids.length > 0 ? await api.getPhotosByIds(ids) : []);
      if (sets.length === 0) {
        // Close on the way out, so tightening the threshold to nothing does
        // not leave the previous sets on screen with no explanation. The
        // toast floats above the overlay, unlike the old error banner.
        set({ compare: null });
        toast(t("store.duplicates.noneLooser"), "info");
        return;
      }
      set({
        compare: {
          sets,
          index: 0,
          focusId: sets[0]?.[0]?.id ?? null,
          source: "duplicates",
          threshold,
        },
        lightboxIndex: null,
      });
    } catch (e) {
      console.error("duplicate review failed", e);
      set({ error: String(e) });
    } finally {
      set({ compareBusy: false });
    }
  },

  compareSelection: () => {
    const { selected, searchResults, photos } = get();
    // Compare what the grid is showing, in the order it shows it — the
    // selection is a set, so it carries no order of its own.
    const set_ = (searchResults ?? photos).filter((p) => selected.has(p.id));
    if (set_.length < 2) {
      toast(t("store.compare.selectTwo"), "info");
      return;
    }
    set({
      compare: {
        sets: [set_],
        index: 0,
        focusId: set_[0]?.id ?? null,
        source: "selection",
        threshold: DUPLICATE_THRESHOLD,
      },
      lightboxIndex: null,
    });
  },

  closeCompare: () => set({ compare: null }),

  compareGoTo: (index) => {
    const compare = get().compare;
    if (!compare) return;
    const next = stepIndex(index, 0, compare.sets.length);
    set({ compare: { ...compare, index: next, focusId: compare.sets[next]?.[0]?.id ?? null } });
  },

  compareFocus: (photoId) => {
    const compare = get().compare;
    if (compare) set({ compare: { ...compare, focusId: photoId } });
  },

  compareApply: async (photoIds, patch) => {
    if (photoIds.length === 0) return;
    try {
      if (patch.flag !== undefined) await api.setFlag(photoIds, patch.flag);
      if (patch.rating !== undefined) await api.setRating(photoIds, patch.rating);
      if (patch.colorLabel !== undefined) await api.setColorLabel(photoIds, patch.colorLabel);
      // `undefined` means "not part of this patch"; `null` is a real value
      // here — it clears the override and defers to the model again.
      if (patch.eyesOverride !== undefined) {
        await api.setEyesOverride(photoIds, patch.eyesOverride);
      }
    } catch (e) {
      console.error("compare apply failed", e);
      set({ error: String(e) });
      return;
    }
    // Patch rather than reload: a cull decision must not make the frame you
    // are looking at jump. The same photo can be on screen more than once
    // (a compare set or a workspace's Lightbox set, and the grid behind
    // it), so every copy is updated.
    const touched = new Set(photoIds);
    const apply = (p: Photo): Photo => (touched.has(p.id) ? { ...p, ...patch } : p);
    const show = get().slideshow;
    if (show) set({ slideshow: { photos: show.photos.map(apply) } });
    const compare = get().compare;
    set({
      photos: get().photos.map(apply),
      searchResults: get().searchResults?.map(apply) ?? null,
      lightboxSet: get().lightboxSet?.map(apply) ?? null,
      compare: compare ? { ...compare, sets: compare.sets.map((s) => s.map(apply)) } : null,
    });
  },

  compareKeep: async (photoId) => {
    const compare = get().compare;
    const current = compare?.sets[compare.index];
    if (!compare || !current) return;
    const plan = keepPlan(
      current.map((p) => p.id),
      photoId,
    );
    // Flags, not deletion: the decision stays visible, filterable and
    // reversible, and removing files remains the user's own step.
    await get().compareApply([plan.keep], { flag: 1 });
    await get().compareApply(plan.reject, { flag: -1 });
    // Straight on to the next set — the point of the view is a fast pass
    // through many groups. The last set stays put rather than closing, so a
    // final decision is still visible when it lands.
    get().compareGoTo(compare.index + 1);
  },

  compareRejectExtras: async (keeperId) => {
    const compare = get().compare;
    const current = compare?.sets[compare.index];
    if (!compare || !current) return;
    // The reject list is exactly `keepPlan`'s, but the keeper's flag is left
    // alone: this action only clears the redundant copies, it doesn't assert
    // the survivor is a "pick" the way `compareKeep` does.
    const { reject } = keepPlan(
      current.map((p) => p.id),
      keeperId,
    );
    if (reject.length === 0) return;
    await get().compareApply(reject, { flag: -1 });
    get().compareGoTo(compare.index + 1);
  },

  selectDuplicates: async () => {
    try {
      const groups = await api.duplicateGroups(DUPLICATE_THRESHOLD, duplicateScope(get()));
      // Keep the sharpest of each group, select the softer rest for review.
      const ordered = orderGroupsBySharpness(groups, get().searchResults ?? get().photos);
      const dupes = new Set<number>();
      for (const g of ordered) g.slice(1).forEach((id) => dupes.add(id));
      set({ selected: dupes, anchorId: null });
      if (dupes.size === 0) {
        toast(t("store.duplicates.noneNotIndexed"), "info");
      }
    } catch (e) {
      console.error("find duplicates failed", e);
      set({ error: String(e) });
    }
  },
});
