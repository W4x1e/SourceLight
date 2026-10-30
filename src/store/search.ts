import { api } from "../lib/api";
import { ensureAi } from "../lib/aiReady";
import { applyReading, readQuery, withoutReading, type Applied, type Reading } from "../lib/queryReading";
import { confirm, toast } from "../lib/dialogs";
import { t } from "../i18n";
import type { Photo } from "../types";
import type { Slice } from "./types";

/**
 * Search: content search and how a query was read, and the "similar" views
 * that stand in for the grid's set.
 */

/** Minimum cosine similarity for "find similar" to include a photo. Lower than
 *  {@link DUPLICATE_THRESHOLD} — similar, not identical — but high enough that
 *  the result is a handful of genuinely alike frames, not the whole library
 *  reordered. Unchanged for SigLIP 2: at 0.8 it keeps 92% of same-scene
 *  pairs (CLIP 83%) and lets in the same ~5% from other scenes. */
export const SIMILAR_THRESHOLD = 0.8;

export interface SearchSlice {
  /** CLIP semantic-search results, shown in the grid instead of the filtered
   *  set while a content search is active; null when not searching. */
  searchResults: Photo[] | null;
  /** Human label for the results banner (the query, or "Similar to …"). */
  searchLabel: string;
  searchBusy: boolean;
  /** Run a natural-language content search: routes the query into the filter's
   *  `semantic` field so the backend's fused CLIP + caption ranking drives the
   *  grid, composing with every facet and saveable as a smart collection. */
  runSemanticSearch: (query: string) => Promise<void>;
  /** What the last search was read as: the query, the reading, and
   *  what it added to the filter — so "as typed" and the other reading of a
   *  word can take it back out whole. `null` when nothing was recognised. */
  queryReading: { raw: string; reading: Reading; applied: Applied } | null;
  /** Search the last query's whole text by content, unread. */
  searchAsTyped: () => Promise<void>;
  /** Take the other reading of a word that reads two ways ("Paris" the
   *  person rather than the place), by its index in the alternatives. */
  takeAlternative: (index: number) => void;
  /** Rank the library by visual similarity to one photo. */
  /** Photos with colours like this one's, whatever they show. */
  findSimilarColours: (photoId: number, label: string) => Promise<void>;
  findSimilar: (photoId: number, label: string) => Promise<void>;
  /** Leave search mode, restoring the filtered grid. */
  clearSearch: () => void;
}

export const createSearchSlice: Slice<SearchSlice> = (set, get) => ({
  searchResults: null,
  searchLabel: "",
  searchBusy: false,
  queryReading: null,

  runSemanticSearch: async (query) => {
    const q = query.trim();
    if (!q) {
      get().setFilter({ ...get().filter, search: null, semantic: null });
      return;
    }

    // Read what the catalogue knows exactly — people, places, dates, gear
    // — out of the query first; content search gets the rest. If
    // the names can't be had, search the lot by content as before.
    let reading: Reading | null = null;
    try {
      const found = readQuery(q, await api.searchVocabulary(), new Date());
      if (found.parts.length > 0) reading = found;
    } catch {
      reading = null;
    }
    const content = reading ? reading.rest : q;
    // A new search replaces the last one's reading rather than adding to
    // it. Read when applied, not now: a dialog may come between.
    const base = () => {
      const prior = get().queryReading;
      return prior ? withoutReading(get().filter, prior.applied) : get().filter;
    };
    if (!content && reading) {
      // All of it was facts: no model needed, and nothing to index.
      const { filter, applied } = applyReading(base(), reading);
      set({ queryReading: { raw: q, reading, applied } });
      get().setFilter({ ...filter, search: null, semantic: null });
      return;
    }

    // Content search is the headline feature, so it must never answer a
    // fresh install with an empty grid and no explanation — the user
    // concludes it is broken rather than un-set-up. Two things have to be
    // true, and each has its own remedy: the CLIP model has to exist, and
    // the library has to have been embedded with it.
    if (!(await ensureAi("search"))) return;

    // A model with nothing indexed still "works" — the backend fuses CLIP
    // with caption/OCR text, so it degrades rather than fails — but on a
    // library that has never been indexed the CLIP half contributes
    // nothing, which looks identical to a bad query. Say so, and offer the
    // one action that fixes it.
    let unindexed = 0;
    try {
      unindexed = await api.searchIndexStatus();
    } catch {
      // Status is a convenience, not a gate. If it cannot be read, search
      // anyway rather than blocking on a diagnostic.
    }
    if (unindexed > 0 && get().jobs["index"]) {
      // Already indexing (automatic analysis after a scan): no second
      // run to offer, only what is happening.
      toast(t("store.search.stillIndexing", { count: unindexed }), "info");
    } else if (unindexed > 0) {
      const agreed = await confirm({
        title: t("store.search.indexTitle"),
        message: t("store.search.indexMessage", { count: unindexed }),
        confirmLabel: t("store.search.indexConfirm"),
        cancelLabel: t("store.search.indexCancel"),
      });
      if (agreed) await get().runIndexing();
    }

    // Content search lives in the filter. The literal `search` box and a
    // content query describe different things (a filename match vs. what the
    // photo is *of*), so promoting to content search moves the text into
    // `semantic` and clears `search`. Going through `setFilter` means the
    // fused results flow through the normal grid, honour the active facets,
    // and can be saved as a smart collection — none of which the old
    // separate result set could do.
    if (reading) {
      const { filter, applied } = applyReading(base(), reading);
      set({ queryReading: { raw: q, reading, applied } });
      get().setFilter({ ...filter, search: null, semantic: content });
    } else {
      const filter = base();
      set({ queryReading: null });
      get().setFilter({ ...filter, search: null, semantic: q });
    }
  },

  searchAsTyped: async () => {
    const last = get().queryReading;
    if (!last) return;
    if (!(await ensureAi("search"))) return;
    set({ queryReading: null });
    get().setFilter({ ...withoutReading(get().filter, last.applied), search: null, semantic: last.raw });
  },

  takeAlternative: (index) => {
    const last = get().queryReading;
    const alt = last?.reading.alternatives[index];
    if (!last || !alt) return;
    const reading: Reading = {
      ...last.reading,
      parts: last.reading.parts.map((p) => (p === alt.chosen ? alt.other : p)),
      alternatives: last.reading.alternatives.filter((_, i) => i !== index),
    };
    const { filter, applied } = applyReading(withoutReading(get().filter, last.applied), reading);
    set({ queryReading: { raw: last.raw, reading, applied } });
    get().setFilter(filter);
  },

  findSimilarColours: async (photoId, label) => {
    set({ searchBusy: true, searchLabel: `Colours like ${label}` });
    try {
      const results = await api.similarColours(photoId, 300);
      set({ searchResults: results, lightboxIndex: null });
      get().clearSelection();
      if (results.length === 0) {
        toast(t("store.colours.noneClose"), "info");
      }
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    } finally {
      set({ searchBusy: false });
    }
  },

  findSimilar: async (photoId, label) => {
    // Same model and same failure mode as content search: without CLIP this
    // returns nothing and looks broken. See `runSemanticSearch`.
    if (!(await ensureAi("search"))) return;
    set({ searchBusy: true, searchLabel: t("store.similar.label", { label }) });
    try {
      const results = await api.similarPhotos(photoId, 300, SIMILAR_THRESHOLD);
      set({ searchResults: results, lightboxIndex: null });
      get().clearSelection();
      if (results.length === 0) {
        toast(t("store.similar.none"), "info");
      }
    } catch (e) {
      console.error("find similar failed", e);
      set({ error: String(e) });
    } finally {
      set({ searchBusy: false });
    }
  },

  clearSearch: () => set({ searchResults: null, searchLabel: "" }),
});
