import { DEFAULT_SLIDESHOW, playOrder, type SlideshowOptions } from "../lib/slideshow";
import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";
import { DEFAULT_SENSITIVITY, gridLabel, nextGrid, type GridKind, type PeakColor } from "../lib/composition";
import { EMPTY_FILTER, type Photo } from "../types";
import { PAGE } from "./library";
import type { Slice } from "./types";

/**
 * Looking at photos: the Lightbox, slideshows, and the focus peaking and grids
 * each place can show.
 */

/** Where focus peaking and a grid can be shown. */
export type AidPlace = "lightbox" | "cull" | "compare" | "develop";
export interface ViewAids {
  peaking: boolean;
  grid: GridKind;
}
const DEFAULT_VIEW_AIDS: Record<AidPlace, ViewAids> = {
  lightbox: { peaking: false, grid: "off" },
  cull: { peaking: false, grid: "off" },
  compare: { peaking: false, grid: "off" },
  // The crop tool always showed thirds while dragging.
  develop: { peaking: false, grid: "thirds" },
};

export interface ViewerSlice {
  /** Focus peaking and the composition grid, remembered per
   *  workspace; Develop's grid is the crop tool's guide. */
  viewAids: Record<AidPlace, ViewAids>;
  togglePeaking: (place: AidPlace) => void;
  setGrid: (place: AidPlace, grid: GridKind) => void;
  cycleGrid: (place: AidPlace) => void;
  /** How peaking marks sharp detail, everywhere. */
  peakColor: PeakColor;
  setPeakColor: (c: PeakColor) => void;
  peakSensitivity: number;
  setPeakSensitivity: (v: number) => void;
  /** A slideshow playing: the photos in play order. */
  slideshow: { photos: Photo[] } | null;
  /** Plays these photos, from the one at `start`, shuffled if that is on. */
  playSlideshow: (photos: Photo[], start?: number) => void;
  /** Plays a collection, in its order. */
  playCollection: (id: number) => Promise<void>;
  closeSlideshow: () => void;
  slideshowOptions: SlideshowOptions;
  setSlideshowOptions: (patch: Partial<SlideshowOptions>) => void;

  lightboxIndex: number | null;
  /** The photos the Lightbox steps through when a workspace opened it on a set
   *  of its own — a person, a place, a trip — so viewing them does not mean
   *  leaving for Browse. Null means the grid's set. */
  lightboxSet: Photo[] | null;

  /** Opens (or, with null, closes) the Lightbox at `index` — into `set` when
   *  given, else into the grid's set. */
  openLightbox: (index: number | null, set?: Photo[] | null) => void;
  /** Moves within the open Lightbox, keeping the set it was opened on. */
  lightboxGoTo: (index: number) => void;
}

export const createViewerSlice: Slice<ViewerSlice> = (set, get) => ({
  lightboxIndex: null,
  lightboxSet: null,
  viewAids: DEFAULT_VIEW_AIDS,
  togglePeaking: (place) =>
    set((s) => ({
      viewAids: { ...s.viewAids, [place]: { ...s.viewAids[place], peaking: !s.viewAids[place].peaking } },
    })),
  setGrid: (place, grid) =>
    set((s) => ({ viewAids: { ...s.viewAids, [place]: { ...s.viewAids[place], grid } } })),
  cycleGrid: (place) => {
    const grid = nextGrid(get().viewAids[place].grid);
    get().setGrid(place, grid);
    // Develop shows its guide only while a crop is dragged, so say which.
    if (place === "develop") toast(t("viewer.aids.cropGuide", { grid: gridLabel(grid) }), "info");
  },
  peakColor: "yellow",
  setPeakColor: (peakColor) => set({ peakColor }),
  peakSensitivity: DEFAULT_SENSITIVITY,
  setPeakSensitivity: (v) => set({ peakSensitivity: Math.min(100, Math.max(0, v)) }),
  slideshow: null,
  playSlideshow: (photos, start = 0) => {
    if (photos.length === 0) return;
    set({ slideshow: { photos: playOrder(photos, start, get().slideshowOptions.shuffle) } });
  },
  playCollection: async (id) => {
    try {
      const filter = { ...EMPTY_FILTER, collectionId: id, sort: "custom" as const };
      get().playSlideshow(await api.getPhotos(filter, PAGE, 0));
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
  },
  closeSlideshow: () => set({ slideshow: null }),
  slideshowOptions: DEFAULT_SLIDESHOW,
  setSlideshowOptions: (patch) => set((s) => ({ slideshowOptions: { ...s.slideshowOptions, ...patch } })),

  openLightbox: (index, photos = null) =>
    set({ lightboxIndex: index, lightboxSet: index === null ? null : photos }),
  lightboxGoTo: (index) => set({ lightboxIndex: index }),
});
