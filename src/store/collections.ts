import { api } from "../lib/api";
import { ensureAi } from "../lib/aiReady";
import type { CollectionGroup } from "../bindings";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";
import { highlightCandidates } from "../lib/survey";
import { EMPTY_FILTER, type Collection, type SmartCollection } from "../types";
import type { Slice } from "./types";

/**
 * Collections, their groups, and saved views.
 */

export interface CollectionsSlice {
  /** Makes a collection of these photos — Cull's picks — and says so. */
  saveAsCollection: (name: string, ids: number[]) => Promise<void>;
  /** The best `count` of the photos in view, as a new collection. */
  makeHighlights: (count: number, label: string) => Promise<void>;

  collections: Collection[];
  loadCollections: () => Promise<void>;
  /** Groups of collections and saved views. */
  collectionGroups: CollectionGroup[];
  createCollectionGroup: (name: string) => Promise<void>;
  renameCollectionGroup: (id: number, name: string) => Promise<void>;
  deleteCollectionGroup: (id: number) => Promise<void>;
  setCollectionGroup: (id: number, smart: boolean, groupId: number | null) => Promise<void>;
  /** Moves photos within the collection shown, before another or to the end. */
  arrangeCollection: (photoIds: number[], before: number | null) => Promise<void>;
  createCollection: (name: string) => Promise<number | null>;
  renameCollection: (id: number, name: string) => Promise<void>;
  /** Takes the selected photos out of the collection the grid is showing. */
  removeSelectionFromCollection: () => Promise<void>;
  deleteCollection: (id: number) => Promise<void>;
  addSelectionToCollection: (id: number) => Promise<void>;

  smartCollections: SmartCollection[];
  loadSmartCollections: () => Promise<void>;
  saveCurrentAsSmart: (name: string) => Promise<void>;
  deleteSmartCollection: (id: number) => Promise<void>;
}

export const createCollectionsSlice: Slice<CollectionsSlice> = (set, get) => ({
  saveAsCollection: async (name, ids) => {
    if (!name.trim() || ids.length === 0) return;
    try {
      const id = await api.createCollection(name.trim());
      await api.addToCollection(id, ids);
      await get().loadCollections();
      toast(t("store.collection.saved", { count: ids.length, name: name.trim() }), "success");
    } catch (e) {
      toast(String(e), "error");
    }
  },

  makeHighlights: async (count, label) => {
    const pool = get().searchResults ?? get().photos;
    const candidates = highlightCandidates(pool);
    if (candidates.length === 0) {
      toast(t("store.highlights.nothing"), "info");
      return;
    }
    // Scored by the search model's idea of a good photo.
    if (!(await ensureAi("search"))) return;
    try {
      const ids = await api.highlights(
        candidates.map((p) => p.id),
        count,
      );
      if (ids.length === 0) {
        toast(t("store.highlights.notIndexed"), "info");
        return;
      }
      const name = t("store.highlights.collectionName", { label });
      const id = await api.createCollection(name);
      await api.addToCollection(id, ids);
      await get().loadCollections();
      get().setFilter({ ...EMPTY_FILTER, collectionId: id });
      toast(t("store.highlights.made", { count: ids.length, total: pool.length, name }), "success");
    } catch (e) {
      toast(String(e), "error");
    }
  },

  collections: [],

  loadCollections: async () => {
    try {
      const [collections, collectionGroups] = await Promise.all([
        api.listCollections(),
        api.listCollectionGroups(),
      ]);
      set({ collections, collectionGroups });
    } catch (e) {
      console.error("collections load failed", e);
      set({ error: String(e) });
    }
  },

  createCollection: async (name) => {
    if (!name.trim()) return null;
    const id = await api.createCollection(name.trim());
    await get().loadCollections();
    return id;
  },

  renameCollection: async (id, name) => {
    if (!name.trim()) return;
    await api.renameCollection(id, name.trim());
    await get().loadCollections();
  },

  removeSelectionFromCollection: async () => {
    const id = get().filter.collectionId;
    const ids = [...get().selected];
    if (id == null || ids.length === 0) return;
    await api.removeFromCollection(id, ids);
    get().clearSelection();
    await get().loadCollections();
    await get().reload();
  },

  deleteCollection: async (id) => {
    await api.deleteCollection(id);
    // Drop the filter if it was pinned to the collection just removed.
    if (get().filter.collectionId === id) {
      get().setFilter({ ...get().filter, collectionId: null });
    }
    await get().loadCollections();
  },

  addSelectionToCollection: async (id) => {
    const sel = get().selected;
    if (sel.size === 0) return;
    await api.addToCollection(id, [...sel]);
    await get().loadCollections();
  },

  smartCollections: [],

  collectionGroups: [],
  createCollectionGroup: async (name) => {
    try {
      await api.createCollectionGroup(name);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
    await get().loadCollections();
  },
  renameCollectionGroup: async (id, name) => {
    try {
      await api.renameCollectionGroup(id, name);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
    await get().loadCollections();
  },
  deleteCollectionGroup: async (id) => {
    await api.deleteCollectionGroup(id);
    await Promise.all([get().loadCollections(), get().loadSmartCollections()]);
  },
  setCollectionGroup: async (id, smart, groupId) => {
    await api.setCollectionGroup(id, smart, groupId);
    await (smart ? get().loadSmartCollections() : get().loadCollections());
  },
  arrangeCollection: async (photoIds, before) => {
    const collection = get().committedFilter.collectionId;
    if (collection == null || photoIds.length === 0) return;
    try {
      await api.moveInCollection(collection, photoIds, before);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
    await get().reload();
  },

  loadSmartCollections: async () => {
    try {
      set({ smartCollections: await api.listSmartCollections() });
    } catch (e) {
      console.error("smart collections load failed", e);
      set({ error: String(e) });
    }
  },

  // Saves the *committed* filter (what the grid is actually showing), not the
  // debounced draft, so a saved query matches what the user sees.
  saveCurrentAsSmart: async (name) => {
    if (!name.trim()) return;
    await api.saveSmartCollection(name.trim(), get().committedFilter);
    await get().loadSmartCollections();
  },

  deleteSmartCollection: async (id) => {
    await api.deleteSmartCollection(id);
    await get().loadSmartCollections();
  },
});
