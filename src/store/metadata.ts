import { api, fileUrl } from "../lib/api";
import { pngSize } from "../lib/copyImage";
import type { BatchOutcome, CaptureTimeChange, XmpImport, SidecarTime } from "../bindings";
import { confirm, toast } from "../lib/dialogs";
import { t } from "../i18n";
import type { Slice } from "./types";

/**
 * A photo's details: copying the photo or its place and keywords, capture
 * times, titles and captions, and XMP sidecars.
 */

export interface MetadataSlice {
  /** Puts a photo, rendered with its edits, on the clipboard. */
  copyImage: (id: number) => Promise<void>;
  /** The photo whose location and keywords were copied, to paste. */
  copiedFrom: { id: number; filename: string } | null;
  copyMetaFrom: (id: number, filename: string) => void;
  pasteLocation: (ids: number[]) => Promise<void>;
  pasteKeywords: (ids: number[]) => Promise<void>;
  /** Moves on whenever a photo's own title, caption or rights change,
   *  so what shows them reads them again. */
  iptcVersion: number;
  bumpIptc: () => void;
  /** The photos the capture-time dialog is open for, or null. */
  captureTimeFor: number[] | null;
  setCaptureTimeFor: (ids: number[] | null) => void;
  /** Changes their capture times, or puts them back; says what it did. */
  changeCaptureTime: (ids: number[], change: CaptureTimeChange) => Promise<boolean>;
  revertCaptureTime: (ids: number[]) => Promise<void>;

  /** Import XMP sidecars for the given photos (empty = whole library). Returns
   * how many were imported; reloads the grid afterwards. */
  importXmp: (photoIds: number[]) => Promise<XmpImport>;
  /** Asks whether to take the capture times sidecars gave. */
  offerSidecarTimes: (times: SidecarTime[]) => Promise<void>;
  /** Write XMP sidecars for the given photos (empty = whole library), with
   *  the export dialog's metadata choices. */
  exportXmp: (photoIds: number[]) => Promise<BatchOutcome>;
}

export const createMetadataSlice: Slice<MetadataSlice> = (set, get) => ({
  copyImage: async (id) => {
    const p = [...(get().searchResults ?? []), ...get().photos].find((x) => x.id === id);
    const name = p?.filename ?? t("store.copy.thePhoto");
    let size: { width: number; height: number } | null = null;
    // Made now, from a promise of the image, so the gesture that asked
    // still counts when the render finishes.
    const png = (async () => {
      const path = await api.renderForClipboard(id);
      const url = fileUrl(path);
      if (!url) throw new Error("the render could not be found");
      const res = await fetch(url);
      if (!res.ok) throw new Error(`the render could not be read (${res.status})`);
      const bytes = new Uint8Array(await res.arrayBuffer());
      size = pngSize(bytes);
      return new Blob([bytes], { type: "image/png" });
    })();
    try {
      await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
      const s = size as { width: number; height: number } | null;
      toast(
        s
          ? t("store.copy.doneSized", { name, width: s.width, height: s.height })
          : t("store.copy.done", { name }),
        "success",
      );
    } catch (e) {
      // The render's own error, when that is what failed.
      const why = await png.then(() => e, (err: unknown) => err);
      const said =
        why instanceof DOMException && why.name === "NotAllowedError"
          ? t("store.copy.refused")
          : String(why).replace(/^Error: /, "");
      // A reason that is already a sentence keeps its own full stop.
      toast(t("store.copy.failed", { name, why: said.replace(/[.!?]\s*$/, "") }), "error");
    }
  },
  copiedFrom: null,
  copyMetaFrom: (id, filename) => {
    set({ copiedFrom: { id, filename } });
    toast(t("store.paste.copied", { name: filename }), "info");
  },
  pasteLocation: async (ids) => {
    const from = get().copiedFrom;
    if (!from) return;
    try {
      const out = await api.pasteLocation(ids, from.id);
      const parts = [t("store.paste.place", { name: from.filename, count: out.placed })];
      if (out.positioned > 0) parts.push(t("store.paste.positionToo", { count: out.positioned }));
      if (out.placed === 0) parts[0] = t("store.paste.positionOnly", { name: from.filename, count: out.positioned });
      toast(parts.join(" "), "success");
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
    await get().refreshPhotos(ids);
  },
  pasteKeywords: async (ids) => {
    const from = get().copiedFrom;
    if (!from) return;
    try {
      const n = await api.pasteKeywords(ids, from.id);
      toast(t("store.paste.keywords", { name: from.filename, count: n, photos: t("common.photos", { count: ids.length }) }), "success");
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
    }
    await Promise.all([get().refreshPhotos(ids), get().loadKeywords()]);
  },
  iptcVersion: 0,
  bumpIptc: () => set((s) => ({ iptcVersion: s.iptcVersion + 1 })),
  captureTimeFor: null,
  setCaptureTimeFor: (ids) => set({ captureTimeFor: ids && ids.length > 0 ? ids : null }),
  changeCaptureTime: async (ids, change) => {
    try {
      const out = await api.setCaptureTime(ids, change);
      // In the interface's language and number format, not the system's
      // (`toLocaleString()`), and not English whatever the language.
      const parts = [t("store.captureTime.changed", { count: out.changed })];
      if (out.undated > 0) parts.push(t("store.captureTime.undated", { count: out.undated }));
      toast(parts.join(" "), out.changed > 0 ? "success" : "info");
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
      return false;
    }
    await get().reload();
    void get().offerTrackReplace(ids);
    return true;
  },
  revertCaptureTime: async (ids) => {
    try {
      const n = await api.revertCaptureTime(ids);
      toast(
        n > 0
          ? t("store.captureTime.reverted", { count: n })
          : t("store.captureTime.nothingToRevert"),
        n > 0 ? "success" : "info",
      );
      if (n > 0) void get().offerTrackReplace(ids);
    } catch (e) {
      toast(String(e), "error");
    }
    await get().reload();
  },

  importXmp: async (photoIds) => {
    const read = await api.importXmp(photoIds);
    // Ratings, flags, labels, tags and people may have changed under the
    // current filter, and a sidecar's keyword paths may have filed some.
    await Promise.all([get().refreshPhotos(photoIds), get().loadKeywords()]);
    // Asked, not applied: a date from elsewhere is the user's to take.
    if (read.times.length > 0) void get().offerSidecarTimes(read.times);
    return read;
  },

  offerSidecarTimes: async (times) => {
    const n = times.length;
    const one = n === 1 ? [...(get().searchResults ?? []), ...get().photos].find((p) => p.id === times[0]!.photoId) : null;
    const ok = await confirm({
      title: one
        ? t("store.sidecarTimes.titleOne", { name: one.filename })
        : t("store.sidecarTimes.titleMany", { count: n }),
      message: [
        one
          ? t("store.sidecarTimes.oneDiffers", {
              sidecar: times[0]!.time.slice(0, 16),
              here: one.captureDate?.slice(0, 16) ?? t("store.sidecarTimes.undated"),
            })
          : t("store.sidecarTimes.manyDiffer"),
        t("store.sidecarTimes.why", {
          revert: t("library.captureTime.revert"),
          dialog: t("library.captureTime.title"),
        }),
      ].join(" "),
      confirmLabel: n === 1 ? t("store.sidecarTimes.useIts") : t("store.sidecarTimes.useTheirs"),
      cancelLabel: t("store.sidecarTimes.keep"),
    });
    if (!ok) return;
    try {
      const out = await api.useSidecarTimes(times);
      toast(t("store.captureTime.fromSidecars", { count: out.changed }), "success");
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
      return;
    }
    await get().reload();
    // New times move photos a GPX track placed off it, as any change does.
    void get().offerTrackReplace(times.map((t) => t.photoId));
  },

  exportXmp: async (photoIds) => api.exportXmp(photoIds, get().exportMetadata),
});
