import { api } from "../lib/api";
import { nextStep } from "../lib/shoots";
import type { Shoot } from "../bindings";
import { EMPTY_FILTER } from "../types";
import { PAGE, viewFilterFor } from "./library";
import type { Slice } from "./types";

/**
 * Card imports and the shoots they make.
 */

/** What the card import remembers between launches. */
export interface ImportSettings {
  destination: string;
  layout: string;
  rename: boolean;
  folderKeywords: boolean;
  backup: string | null;
  template: number | null;
}
export const DEFAULT_IMPORT: ImportSettings = {
  destination: "",
  layout: "{yyyy}/{yyyy-mm-dd}",
  rename: false,
  folderKeywords: false,
  backup: null,
  template: null,
};

export interface ShootsSlice {
  /** The import-from-card dialog. */
  showImport: boolean;
  setShowImport: (v: boolean) => void;
  /** The shoot an import adds to, when opened from that shoot's "Import the
   *  rest"; null for a new one. */
  importShootId: number | null;
  /** Opens the import dialog to add to `shootId`'s shoot. */
  importForShoot: (shootId: number) => void;
  /** The recent card imports, with how far along each is. */
  shoots: Shoot[];
  loadShoots: () => Promise<void>;
  /** Does a shoot's next step: culls it, develops its picks, or exports them. */
  continueShoot: (shoot: Shoot) => Promise<void>;
  /** Forgets a shoot; its photos stay. */
  forgetShoot: (id: number) => Promise<void>;
  /** The card import's last choices: forgotten at every launch, the
   *  backup drive included, so a morning import made one copy without a word. */
  importSettings: ImportSettings;
  setImportSettings: (patch: Partial<ImportSettings>) => void;
}

export const createShootsSlice: Slice<ShootsSlice> = (set, get) => ({
  showImport: false,
  setShowImport: (v) => set(v ? { showImport: true } : { showImport: false, importShootId: null }),
  importShootId: null,
  importForShoot: (shootId) => set({ showImport: true, importShootId: shootId }),

  shoots: [],
  loadShoots: async () => {
    try {
      set({ shoots: await api.listShoots() });
    } catch (e) {
      console.error("shoots load failed", e);
    }
  },
  continueShoot: async (shoot) => {
    const st = get();
    const base = { ...EMPTY_FILTER, shootId: shoot.id, sort: st.filter.sort ?? "newest" };
    const step = nextStep(shoot).step;
    if (step === "cull") {
      // Cull opens on what's left to judge.
      st.setCullSession({ photoId: -1, index: 0, strip: "unrated", view: "loupe" });
      st.setFilter(base);
      st.setMode("cull");
      return;
    }
    if (step === "develop" || step === "export") {
      const picksOnly = { ...base, flags: [1] };
      st.setFilter(picksOnly);
      st.setMode("browse");
      const picks = await api.getPhotos(viewFilterFor(get(), picksOnly), PAGE, 0);
      if (step === "develop") {
        const first = picks.find((p) => !p.edited) ?? picks[0];
        if (first) await st.openDevelop(first.id);
      } else {
        st.selectPhotos(picks.map((p) => p.id));
        await get().exportSelection();
      }
    }
  },
  forgetShoot: async (id) => {
    await api.deleteShoot(id);
    if (get().filter.shootId === id) get().setFilter({ ...get().filter, shootId: null });
    await get().loadShoots();
  },
  importSettings: DEFAULT_IMPORT,
  setImportSettings: (patch) => set({ importSettings: { ...get().importSettings, ...patch } }),
});
