import { api } from "../lib/api";
import type { UpdateCheck } from "../bindings";
import { resolveLocale, setLocale, type LanguagePref } from "../i18n";
import { learnTagNames } from "../lib/tagNames";
import { IS_MAC } from "../lib/platform";
import { DEFAULT_KEYMAP, type ActionId } from "../lib/keymap";
import type { Slice } from "./types";

/**
 * The Settings panel, and the preferences that belong to no one workspace.
 */

/** The tabs of the Settings panel. */
export type SettingsTab = "library" | "ai" | "export" | "phone" | "about";

export interface SettingsSlice {
  showSettings: boolean;
  /** The Settings tab to open on. Reset to AI whenever Settings opens without one. */
  settingsTab: SettingsTab;
  /** The last update check's answer, or null until one has come back. */
  update: UpdateCheck | null;
  /** Current key bindings (defaults merged with the user's overrides). */
  keymap: Record<ActionId, string>;
  setShowSettings: (v: boolean) => void;
  /** Opens Settings on a particular tab. */
  openSettings: (tab: SettingsTab) => void;
  setUpdate: (u: UpdateCheck | null) => void;
  /** Whether the app asks the update server for a new version when it
   *  starts. Asking tells that server the app's version and the
   *  machine's address, nothing more; some people want it never asked. */
  checkUpdates: boolean;
  setCheckUpdates: (on: boolean) => void;
  setBinding: (id: ActionId, combo: string) => void;
  resetKeymap: () => void;
  /** Draw Develop's preview straight into a GPU layer beneath the page, rather
   *  than as an image (Settings › About › Diagnostics). Mac only, for now. */
  directCanvas: boolean;
  setDirectCanvas: (on: boolean) => void;
  /** Take a catalogue backup a day, at launch (Settings › Library). */
  autoBackup: boolean;
  setAutoBackup: (on: boolean) => void;
  /** The interface language (Settings › About): one the app ships, or
   *  whatever the system uses. The active locale itself lives in `i18n`. */
  language: LanguagePref;
  setLanguage: (pref: LanguagePref) => Promise<void>;
}

export const createSettingsSlice: Slice<SettingsSlice> = (set, get) => ({
  showSettings: false,
  settingsTab: "ai",
  update: null,
  keymap: { ...DEFAULT_KEYMAP },
  setShowSettings: (v) => set({ showSettings: v, settingsTab: "ai" }),
  openSettings: (tab) => set({ showSettings: true, settingsTab: tab }),
  setUpdate: (u) => set({ update: u }),
  checkUpdates: true,
  setCheckUpdates: (on) => set({ checkUpdates: on }),
  setBinding: (id, combo) => set({ keymap: { ...get().keymap, [id]: combo } }),
  resetKeymap: () => set({ keymap: { ...DEFAULT_KEYMAP } }),
  directCanvas: IS_MAC,
  setDirectCanvas: (on) => set({ directCanvas: on }),
  autoBackup: true,
  setAutoBackup: (on) => {
    set({ autoBackup: on });
    if (on) void api.backupCatalogueIfDue().catch(() => undefined);
  },
  language: "system",
  setLanguage: async (pref) => {
    // Load first, then store: the root re-renders on the change, and it
    // should find the new catalogue already there.
    await setLocale(resolveLocale(pref));
    set({ language: pref });
    void learnTagNames(get().facets);
  },
});
