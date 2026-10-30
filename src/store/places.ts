import { api } from "../lib/api";
import { confirm, toast } from "../lib/dialogs";
import { t } from "../i18n";
import type { Slice } from "./types";

/**
 * Places: putting photos on the map, and a phone's GPS track waiting for the
 * map to open it.
 */

export interface PlacesSlice {
  /** A GPS track a phone sent (docs/MOBILE.md), waiting for Places to open
   *  its track dialog. */
  phoneTrack: string | null;
  setPhoneTrack: (path: string | null) => void;
  /** After a time change, offers to move the photos a GPX track placed. */
  offerTrackReplace: (ids: number[]) => Promise<void>;

  assignPlaceToSelection: (name: string) => Promise<void>;
  /** Takes the place off the selected photos; the place itself stays. */
  clearPlaceFromSelection: () => Promise<void>;
}

export const createPlacesSlice: Slice<PlacesSlice> = (set, get) => ({
  phoneTrack: null,
  setPhoneTrack: (path) => set({ phoneTrack: path }),
  offerTrackReplace: async (ids) => {
    let n = 0;
    try {
      n = await api.countTrackPlaced(ids);
    } catch {
      return;
    }
    if (n === 0) return;
    const ok = await confirm({
      title: t("store.track.title", { count: n }),
      message: t("store.track.message", { count: n }),
      confirmLabel: t("store.track.confirm"),
      cancelLabel: t("store.track.leave"),
    });
    if (!ok) return;
    try {
      const out = await api.gpxReplace(ids);
      const parts = [t("store.track.moved", { count: out.moved })];
      if (out.outside > 0) parts.push(t("store.track.outside", { count: out.outside }));
      if (out.missing > 0) parts.push(t("store.track.missing", { count: out.missing }));
      toast(parts.join(" "), out.moved > 0 ? "success" : "info");
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
    await get().reload();
  },

  assignPlaceToSelection: async (name) => {
    const ids = [...get().selected];
    if (!name.trim() || ids.length === 0) return;
    await api.assignPlace(ids, name.trim());
    await get().refreshPhotos(ids);
  },

  clearPlaceFromSelection: async () => {
    const ids = [...get().selected];
    if (ids.length === 0) return;
    await api.clearPlace(ids);
    await get().refreshPhotos(ids);
  },
});
