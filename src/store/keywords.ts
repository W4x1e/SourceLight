import { tagName } from "../i18n/tags";
import { api } from "../lib/api";
import type { KeywordNode } from "../bindings";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";
import type { PhotoFilter } from "../types";
import type { Slice } from "./types";

/**
 * Keywords: the tree, putting them on photos and taking them off, and whether
 * the tagger's own show.
 */

export interface KeywordsSlice {
  /** Show the tagger's unconfirmed keywords. Off, only the user's own
   *  keywords show, match and count — see `viewFilter`. */
  showAiKeywords: boolean;
  setShowAiKeywords: (on: boolean) => Promise<void>;
  addTagToSelection: (name: string) => Promise<void>;
  /** Puts a keyword on these photos, or takes it off. */
  setKeywordOn: (ids: number[], name: string, on: boolean) => Promise<void>;
  /** Turns folder names into keywords: one library folder's photos,
   *  one shoot's, or (both null) every photo. Says what it did in a toast. */
  addFolderKeywords: (folderId: number | null, shootId: number | null) => Promise<void>;
  /** Takes keywords made from folder names back off (all for null). */
  removeFolderKeywords: (folderId: number | null) => Promise<void>;
  /** Makes the tagger's keywords on these photos the user's own: the
   *  named ones, or all of them. */
  confirmKeywords: (ids: number[], names: string[]) => Promise<void>;

  /** The keyword tree's shape: what each keyword is filed under. */
  keywords: KeywordNode[];
  loadKeywords: () => Promise<void>;
  /** Each resolves true when it worked; a refusal is toasted. */
  createKeyword: (name: string, parent: string | null) => Promise<boolean>;
  moveKeyword: (name: string, parent: string | null) => Promise<boolean>;
  renameKeyword: (name: string, newName: string) => Promise<boolean>;
  deleteKeyword: (name: string) => Promise<boolean>;
}

export const createKeywordsSlice: Slice<KeywordsSlice> = (set, get) => ({
  showAiKeywords: true,
  setShowAiKeywords: async (on) => {
    set({ showAiKeywords: on });
    await get().reload();
  },

  addTagToSelection: async (name) => {
    const ids = [...get().selected];
    if (!name.trim() || ids.length === 0) return;
    await api.addTag(ids, name.trim(), "subject");
    // A path ("places > france") may have filed new keywords.
    await Promise.all([get().refreshPhotos(ids), get().loadKeywords()]);
  },

  setKeywordOn: async (ids, name, on) => {
    if (ids.length === 0) return;
    try {
      if (on) await api.addTag(ids, name, "subject");
      else await api.removeTag(ids, name);
    } catch (e) {
      toast(String(e), "error");
    }
    await Promise.all([get().refreshPhotos(ids), get().loadKeywords()]);
  },

  addFolderKeywords: async (folderId, shootId) => {
    try {
      const out = await api.addFolderKeywords(folderId, shootId);
      toast(
        out.photos > 0
          ? t("store.folderKeywords.added", { count: out.keywords, photos: t("common.photos", { count: out.photos }) })
          : t("store.folderKeywords.none"),
        out.photos > 0 ? "success" : "info",
      );
    } catch (e) {
      toast(String(e), "error");
    }
    await Promise.all([get().loadKeywords(), get().reload()]);
  },

  removeFolderKeywords: async (folderId) => {
    try {
      const n = await api.removeFolderKeywords(folderId);
      toast(
        n > 0
          ? t("store.folderKeywords.removed", { count: n })
          : t("store.folderKeywords.noneToRemove"),
        n > 0 ? "success" : "info",
      );
    } catch (e) {
      toast(String(e), "error");
    }
    await Promise.all([get().loadKeywords(), get().reload()]);
  },

  confirmKeywords: async (ids, names) => {
    if (ids.length === 0) return;
    try {
      const n = await api.confirmKeywords(ids, names);
      if (names.length === 0 && n > 0)
        toast(t("store.keywords.confirmed", { count: n }), "success");
    } catch (e) {
      toast(String(e), "error");
    }
    await get().refreshPhotos(ids);
  },

  keywords: [],
  loadKeywords: async () => {
    try {
      set({ keywords: await api.keywordTree() });
    } catch (e) {
      console.error("keywords load failed", e);
    }
  },

  createKeyword: async (name, parent) => {
    try {
      await api.createKeyword(name, parent);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
      return false;
    }
    await get().loadKeywords();
    return true;
  },

  moveKeyword: async (name, parent) => {
    try {
      await api.moveKeyword(name, parent);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
      return false;
    }
    // Counts follow the branch it now belongs to.
    await Promise.all([get().loadKeywords(), get().reload()]);
    return true;
  },

  renameKeyword: async (name, newName) => {
    let renamed: string;
    try {
      renamed = await api.renameKeyword(name, newName);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
      return false;
    }
    // The live filter follows it, as saved views do in the backend —
    // patched in place rather than through `setFilter`, which would clear
    // the selection for what is still the same set of photos.
    const swap = (list: string[]) => [...new Set(list.map((s) => (s === name ? renamed : s)))];
    const follow = (f: PhotoFilter): PhotoFilter =>
      f.subjects.includes(name) || (f.excludeSubjects ?? []).includes(name)
        ? { ...f, subjects: swap(f.subjects), excludeSubjects: swap(f.excludeSubjects ?? []) }
        : f;
    set({ filter: follow(get().filter), committedFilter: follow(get().committedFilter) });
    await Promise.all([get().loadKeywords(), get().reload()]);
    return true;
  },

  deleteKeyword: async (name) => {
    let photos: number;
    try {
      photos = await api.deleteKeyword(name);
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
      return false;
    }
    const f = get().filter;
    if (f.subjects.includes(name) || (f.excludeSubjects ?? []).includes(name))
      get().setFilter({
        ...f,
        subjects: f.subjects.filter((s) => s !== name),
        excludeSubjects: (f.excludeSubjects ?? []).filter((s) => s !== name),
      });
    toast(
      photos > 0
        ? t("store.keywords.deletedFrom", { name: tagName(name), count: photos })
        : t("store.keywords.deleted", { name: tagName(name) }),
      "success",
    );
    await Promise.all([get().loadKeywords(), get().reload()]);
    return true;
  },
});
