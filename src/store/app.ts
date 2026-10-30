import type { Photo } from "../types";
import type { Slice } from "./types";

/**
 * The app's shell: the workspace in front, and the panels, menus and tour that
 * open over it.
 */

/** The app's top-level workspace. Develop, Compare and the Lightbox are
 *  photo-scoped overlays that sit *on top* of whichever mode is active, so they
 *  are tracked separately (by `develop` / `compare` / `lightboxIndex`). */
export type AppMode = "browse" | "cull" | "places" | "people";

export interface AppSlice {
  /** The active top-level workspace. */
  mode: AppMode;
  setMode: (m: AppMode) => void;
  showShortcuts: boolean;
  /** The guided tour has been finished or skipped once, so it no longer starts
   *  by itself. Persisted. Settings › About replays it. */
  tourSeen: boolean;
  /** The running tour: which step, and the workspace to return to when it ends
   *  (it switches workspace as it goes). Null when no tour is running. */
  tour: { step: number; returnMode: AppMode } | null;
  /** Starts the tour from the top, clearing anything that would cover it. */
  startTour: () => void;
  tourGo: (step: number) => void;
  /** Ends the tour, finished or skipped, and puts the user back where they were. */
  endTour: () => void;
  /** The Activity centre slide-over (scan / tag / export in one place). */
  showActivity: boolean;
  setShowActivity: (v: boolean) => void;
  error: string | null;
  /** The right-click menu on a photo: where, on which photos, and from
   *  which workspace — Browse acts on the selection, Cull on one frame. */
  /** `set` is the frames the menu's slideshow plays: Cull's strip. */
  photoMenu: { x: number; y: number; ids: number[]; context: "browse" | "cull"; set?: Photo[] } | null;
  openPhotoMenu: (x: number, y: number, ids: number[], context: "browse" | "cull", set?: Photo[]) => void;
  closePhotoMenu: () => void;
  setShowShortcuts: (v: boolean) => void;
  setError: (e: string | null) => void;
  /** The Insights view. */
  showInsights: boolean;
  setShowInsights: (v: boolean) => void;
}

export const createAppSlice: Slice<AppSlice> = (set) => ({
  mode: "browse",
  setMode: (m) => set({ mode: m }),
  showShortcuts: false,
  tourSeen: false,
  tour: null,
  startTour: () =>
    set((s) => ({
      tour: { step: 0, returnMode: s.mode },
      showSettings: false,
      showShortcuts: false,
      showActivity: false,
      lightboxIndex: null,
      compare: null,
      photoMenu: null,
    })),
  tourGo: (step) => set((s) => (s.tour ? { tour: { ...s.tour, step } } : {})),
  endTour: () =>
    set((s) => ({ tourSeen: true, tour: null, ...(s.tour ? { mode: s.tour.returnMode } : {}) })),
  showActivity: false,
  setShowActivity: (v) => set({ showActivity: v }),
  error: null,
  setShowShortcuts: (v) => set({ showShortcuts: v }),

  setError: (e) => set({ error: e }),

  photoMenu: null,
  openPhotoMenu: (x, y, ids, context, frames) => set({ photoMenu: { x, y, ids, context, set: frames } }),
  closePhotoMenu: () => set({ photoMenu: null }),
  showInsights: false,
  setShowInsights: (v) => set({ showInsights: v }),
});
