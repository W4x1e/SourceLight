import PhotoDescription from "./PhotoDescription";
import { usePreload } from "../lib/usePreload";
import { overlayOpen } from "../lib/useEscape";
import { useFocusTrap } from "../lib/useFocusTrap";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { PeakingStatus } from "./FocusPeaking";
import ViewAidsControl from "./ViewAidsControl";
import { textIsSelected } from "../lib/copyImage";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api, fileUrl, fullUrl } from "../lib/api";
import { useEditedPath } from "../lib/useEditedPath";
import { useStore, type PhotoPatch } from "../store";
import { Icon } from "../lib/icons";
import LightboxFaces from "./LightboxFaces";
import ZoomableImage, {
  type ZoomableImageHandle,
  type ZoomSource,
  zoomSourceLabel,
} from "./ZoomableImage";
import { COLOR_LABELS } from "../types";
import { formatNumber, t } from "../i18n";
import { tagName } from "../i18n/tags";
import { sentenceCase } from "../lib/format";
import { localPlaceName } from "../lib/countries";
import { comboToLabel, resolveAction, type ActionId } from "../lib/keymap";
import { eyesClosed, facesAnalysed, focusBand, sharpnessScores } from "../lib/survey";
import { EyesBadge, FocusMeter } from "./QualityBits";

/** Filmstrip tile geometry — 64px tall at 3:2, plus the gap between frames. */
const STRIP_THUMB_H = 64;
const STRIP_THUMB_W = Math.round(STRIP_THUMB_H * (3 / 2));
const STRIP_GAP = 6;

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  if (value === null || value === undefined || value === "") return null;
  return (
    <div className="flex justify-between gap-3 py-[3px]">
      <span className="shrink-0 text-faint">{label}</span>
      <span className="truncate text-right text-muted" title={String(value)}>
        {value}
      </span>
    </div>
  );
}

/** The photo the Lightbox was on is gone from its set: it closes, rather
 *  than drawing nothing over a grid whose keys it still holds. */
function Gone({ onGone }: { onGone: () => void }) {
  useEffect(() => onGone(), [onGone]);
  return null;
}

export default function Lightbox() {
  // Navigate the set it was opened on: a workspace's own (a person, a place),
  // else the same set the grid shows (search results when searching).
  const scoped = useStore((s) => s.lightboxSet);
  const photos = useStore((s) => s.lightboxSet ?? s.searchResults ?? s.photos);
  // Only rendered while `lightboxIndex` is non-null, so the fallback is unreachable.
  const index = useStore((s) => s.lightboxIndex) ?? 0;
  const openLightbox = useStore((s) => s.openLightbox);
  const lightboxGoTo = useStore((s) => s.lightboxGoTo);
  const reload = useStore((s) => s.reload);
  const openDevelop = useStore((s) => s.openDevelop);
  const findSimilar = useStore((s) => s.findSimilar);
  const compareApply = useStore((s) => s.compareApply);
  const deletePhotos = useStore((s) => s.deletePhotos);
  // Shortcut hints are read from the live keymap rather than spelled out, so a
  // rebound key does not leave the tooltips quietly lying.
  const keymap = useStore((s) => s.keymap);
  const key = (id: ActionId) => comboToLabel(keymap[id]);

  const onIndexChange = lightboxGoTo;
  const onClose = useCallback(() => openLightbox(null), [openLightbox]);
  // Tags change the photo's shape beyond a verdict patch, so they still reload
  // — and a workspace's set is its own copy, so it is re-read too.
  const onMutated = useCallback(() => {
    // A change that can move the photo out of the filter — a tag, a face —
    // first pins the set the Lightbox is stepping through: reloaded
    // under a fixed index, another photo slid in, or at the end of the list
    // the Lightbox vanished and left every key dead. The grid reloads; the
    // Lightbox keeps its photo and its neighbours.
    const st = useStore.getState();
    if (!st.lightboxSet) useStore.setState({ lightboxSet: st.searchResults ?? st.photos });
    void reload();
    const set = useStore.getState().lightboxSet;
    if (!set) return;
    void api.getPhotosByIds(set.map((p) => p.id)).then((fresh) => {
      const byId = new Map(fresh.map((p) => [p.id, p]));
      // Only if the Lightbox is still on this set.
      if (useStore.getState().lightboxSet === set) {
        useStore.setState({ lightboxSet: set.map((p) => byId.get(p.id) ?? p) });
      }
    });
  }, [reload]);

  // Verdicts patch in place instead. Reloading here re-ran the active filter,
  // and `index` points into that list: rating a 1 while filtered to `minRating`
  // 3 used to slide a different photo under the cursor — or, at the end of the
  // list, leave `photos[index]` undefined and silently close the Lightbox.
  const applyVerdict = useCallback(
    (id: number, patch: PhotoPatch) => void compareApply([id], patch),
    [compareApply],
  );

  const photo = photos[index];
  const edited = useEditedPath(photo);
  const [newTag, setNewTag] = useState("");
  // The frames either side, ready before a step reaches them.
  const near = (i: number) => photos[i] && fileUrl(photos[i]!.previewPath ?? photos[i]!.thumbPath);
  usePreload([near(index + 1), near(index - 1)]);

  // Focus is relative, so rank this frame against the visible set (search
  // results when searching, else the library).
  const focusScores = useMemo(() => sharpnessScores(photos), [photos]);
  const focus = photo ? focusBand(photo, focusScores) : null;

  // The filmstrip is virtualised for the same reason the grid is: `photos` can
  // hold up to `PAGE` (20,000) entries, and rendering a button and an <img> for
  // every one of them cost tens of thousands of nodes before the Lightbox could
  // paint. Fixed-size tiles, so the measurement is just arithmetic.
  const stripRef = useRef<HTMLDivElement>(null);
  const zoomRef = useRef<ZoomableImageHandle | null>(null);
  const aids = useStore((s) => s.viewAids.lightbox);
  const peakColor = useStore((s) => s.peakColor);
  const peakSensitivity = useStore((s) => s.peakSensitivity);
  const [peakStatus, setPeakStatus] = useState<PeakingStatus | null>(null);
  const [zoomed, setZoomed] = useState(false);
  const [zoomSource, setZoomSource] = useState<ZoomSource>("preview");
  const stripVirtualizer = useVirtualizer({
    horizontal: true,
    count: photos.length,
    getScrollElement: () => stripRef.current,
    estimateSize: () => STRIP_THUMB_W + STRIP_GAP,
    overscan: 8,
  });

  // Keep the current thumbnail centred as you browse. `scrollToIndex` replaces
  // the old `scrollIntoView` on a live ref, which a virtualised strip no longer
  // has for off-screen frames.
  useEffect(() => {
    stripVirtualizer.scrollToIndex(index, { align: "center" });
  }, [index, stripVirtualizer]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A dialog or menu over the Lightbox owns the keyboard.
      if (overlayOpen()) return;
      // Let a text field (the tag box) keep its own keys; Esc just blurs it.
      if (e.target instanceof HTMLInputElement) {
        if (e.key === "Escape" || e.key === "Esc") e.target.blur();
        return;
      }
      const action = resolveAction(useStore.getState().keymap, "lightbox", e);
      if (!action || !photo) return;
      if (action === "undo-verdict") {
        e.preventDefault();
        void useStore.getState().undoVerdict();
        return;
      }
      if (action === "close-overlay") onClose();
      else if (action === "prev-photo") onIndexChange(Math.max(index - 1, 0));
      else if (action === "next-photo") onIndexChange(Math.min(index + 1, photos.length - 1));
      else if (action === "open-develop") void openDevelop(photo.id);
      else if (action === "copy-image") {
        if (textIsSelected()) return;
        void useStore.getState().copyImage(photo.id);
      } else if (action === "toggle-zoom") zoomRef.current?.toggle();
      else if (action === "toggle-peaking") useStore.getState().togglePeaking("lightbox");
      else if (action === "cycle-grid") useStore.getState().cycleGrid("lightbox");
      else if (action.startsWith("rate-")) applyVerdict(photo.id, { rating: Number(action.slice(5)) });
      else if (action === "flag-pick") applyVerdict(photo.id, { flag: 1 });
      else if (action === "flag-reject") applyVerdict(photo.id, { flag: -1 });
      else if (action === "flag-none") applyVerdict(photo.id, { flag: 0 });
      else if (action === "label-none") applyVerdict(photo.id, { colorLabel: 0 });
      else if (action.startsWith("label-")) applyVerdict(photo.id, { colorLabel: Number(action.slice(6)) });
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [index, photos.length, photo, onClose, onIndexChange, applyVerdict, openDevelop]);

  // Tab stays in the Lightbox, and focus goes back to the grid on close.
  const rootRef = useRef<HTMLDivElement>(null);
  useFocusTrap(true, rootRef);

  if (!photo) return <Gone onGone={onClose} />;

  const addTag = async () => {
    const name = newTag.trim();
    if (!name) return;
    await api.addTag([photo.id], name, "subject");
    setNewTag("");
    onMutated();
  };

  return (
    // Opaque, like Compare: at 92% black the workspace underneath ghosted through.
    <div
      ref={rootRef}
      tabIndex={-1}
      role="dialog"
      aria-label={photo.filename}
      className="anim-view fixed inset-0 z-50 flex bg-canvas-void outline-none"
    >
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {/* Clicking the empty space around the photo closes, as in any viewer.
            The currentTarget check keeps clicks on the photo itself from closing. */}
        <div
          className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden p-6"
          onClick={(e) => {
            if (e.target === e.currentTarget) onClose();
          }}
        >
          <ZoomableImage
            src={fileUrl(edited ?? photo.previewPath ?? photo.thumbPath)}
            alt={photo.filename}
            resetKey={photo.id}
            controlRef={zoomRef}
            onZoomChange={setZoomed}
            // The full-size file is the camera's, without the edits.
            fullSrc={photo.edited ? undefined : fullUrl(photo.id)}
            onSourceChange={setZoomSource}
            peaking={aids.peaking ? { color: peakColor, sensitivity: peakSensitivity } : null}
            onPeakingStatus={setPeakStatus}
            grid={aids.grid}
          />

          <ViewAidsControl place="lightbox" status={aids.peaking && peakStatus === "failed" ? "failed" : null} />

          {zoomed && (
            <div className="pointer-events-none absolute bottom-3 left-3 rounded-md bg-chip-hover px-2 py-1 text-label text-ink">
              {zoomSourceLabel(zoomSource)}
            </div>
          )}

          {index > 0 && (
            <button
              onClick={() => onIndexChange(index - 1)}
              aria-label={t("viewer.lightbox.previous")}
              className="absolute left-3 grid h-10 w-10 place-items-center rounded-sm bg-chip text-ink hover:bg-chip-hover"
            >
              <Icon name="chevron" size={20} className="rotate-90" />
            </button>
          )}
          {index < photos.length - 1 && (
            <button
              onClick={() => onIndexChange(index + 1)}
              aria-label={t("viewer.lightbox.next")}
              className="absolute right-3 grid h-10 w-10 place-items-center rounded-sm bg-chip text-ink hover:bg-chip-hover"
            >
              <Icon name="chevron" size={20} className="-rotate-90" />
            </button>
          )}

          <div className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-sm bg-chip px-3 py-1 text-caption tabular-nums text-muted backdrop-blur">
            {index + 1} / {photos.length}
          </div>
        </div>

        {/* Filmstrip: the shoot, one click per frame. */}
        <div
          ref={stripRef}
          className="flex h-[84px] shrink-0 items-center overflow-x-auto overflow-y-hidden border-t border-line bg-chip px-2.5 py-2"
        >
          <div
            style={{
              width: stripVirtualizer.getTotalSize(),
              height: STRIP_THUMB_H,
              position: "relative",
              flexShrink: 0,
            }}
          >
            {stripVirtualizer.getVirtualItems().map((v) => {
              const p = photos[v.index];
              if (!p) return null;
              const active = v.index === index;
              return (
                <button
                  key={p.id}
                  onClick={() => onIndexChange(v.index)}
                  title={p.filename}
                  // One tab stop for the strip, on the frame shown; the
                  // arrows move along it. Every frame was a stop, so Tab
                  // walked the whole shoot before reaching the panel.
                  tabIndex={active ? 0 : -1}
                  aria-current={active ? "true" : undefined}
                  style={{
                    position: "absolute",
                    left: v.start,
                    top: 0,
                    width: STRIP_THUMB_W,
                    height: STRIP_THUMB_H,
                  }}
                  className={`overflow-hidden rounded-sm border-2 transition-opacity ${
                    active
                      ? "border-accent opacity-100"
                      : "border-transparent opacity-60 hover:opacity-100"
                  }`}
                >
                  {p.thumbPath ? (
                    <img
                      src={fileUrl(p.thumbPath)}
                      alt=""
                      loading="lazy"
                      draggable={false}
                      className="h-full w-full object-cover"
                    />
                  ) : (
                    <div className="grid h-full w-full place-items-center bg-raised text-faint">
                      ?
                    </div>
                  )}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      <aside className="flex w-[var(--panel-w)] shrink-0 flex-col border-l border-line bg-panel">
        {/* The filename on a line of its own: beside the buttons it
            was "DSC0…", and just "…" in German and French. */}
        <div className="flex items-center gap-2 px-3 pt-2">
          <span className="min-w-0 flex-1 truncate text-caption font-medium" title={photo.filename}>
            {photo.filename}
          </span>
          <button
            onClick={() => void deletePhotos([photo])}
            className="icon-chip hover:text-danger-ink"
            aria-label={t("library.photoMenu.delete", { count: 1 })}
            title={t("library.photoMenu.delete", { count: 1 })}
          >
            <Icon name="trash" size={15} />
          </button>
          <button onClick={onClose} className="icon-chip" title={t("viewer.lightbox.close", { key: key("close-overlay") })}>
            <Icon name="close" size={15} />
          </button>
        </div>
        <div className="flex items-center justify-end gap-2 border-b border-line px-3 pb-2 pt-1.5">
          {/* Similar shows its results in the Browse grid, so it is left out
              where the Lightbox was opened from People or Places on a set of
              their own: it would close this and change a grid you can't see. */}
          {!scoped && (
            <button
              onClick={() => void findSimilar(photo.id, photo.filename)}
              className="btn-solid shrink-0 gap-1.5 text-caption"
              title={t("viewer.lightbox.similarHint")}
            >
              <Icon name="compare" size={14} />
              {t("viewer.lightbox.similar")}
            </button>
          )}
          <button
            onClick={() => void openDevelop(photo.id)}
            className="btn-accent shrink-0"
            title={t("viewer.lightbox.developHint")}
          >
            <Icon name="adjust" size={14} />
            {t("viewer.lightbox.develop")}
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-3 py-2 text-caption">
          <div className="mb-2.5 flex items-center gap-1.5">
            {[1, 2, 3, 4, 5].map((n) => (
              <button
                key={n}
                onClick={() => applyVerdict(photo.id, { rating: photo.rating === n ? 0 : n })}
                aria-label={t("library.photoMenu.rate", { rating: n })}
                aria-pressed={photo.rating === n}
                className={`text-xl leading-none ${
                  photo.rating >= n ? "text-accent" : "text-faint hover:text-muted"
                }`}
              >
                ★
              </button>
            ))}
            <span className="mx-1.5 h-5 w-px bg-line" />
            <button
              onClick={() => applyVerdict(photo.id, { flag: photo.flag === 1 ? 0 : 1 })}
              aria-pressed={photo.flag === 1}
              className={`leading-none ${photo.flag === 1 ? "text-positive" : "text-faint hover:text-muted"}`}
              title={t("viewer.lightbox.pick", { key: key("flag-pick") })}
            >
              <Icon name="flag" size={19} />
            </button>
            <button
              onClick={() => applyVerdict(photo.id, { flag: photo.flag === -1 ? 0 : -1 })}
              aria-pressed={photo.flag === -1}
              className={`leading-none ${photo.flag === -1 ? "text-danger" : "text-faint hover:text-muted"}`}
              title={t("viewer.lightbox.reject", { key: key("flag-reject") })}
            >
              <Icon name="flag-off" size={19} />
            </button>
          </div>

          <div className="mb-3 flex items-center gap-2">
            {COLOR_LABELS.map((c) => (
              <button
                key={c.value}
                onClick={() =>
                  applyVerdict(photo.id, { colorLabel: photo.colorLabel === c.value ? 0 : c.value })
                }
                aria-label={t(c.nameKey)}
                aria-pressed={photo.colorLabel === c.value}
                className={`h-5 w-5 rounded-full border transition-transform hover:scale-110 ${
                  photo.colorLabel === c.value ? "border-ink ring-2 ring-accent" : "border-black/30"
                }`}
                style={{ backgroundColor: c.hex }}
                title={t("viewer.lightbox.colourLabel", { name: t(c.nameKey), key: key(`label-${c.value}` as ActionId) })}
              />
            ))}
          </div>

          <div className="section-label px-0">{t("viewer.lightbox.capture")}</div>
          <Row label={t("viewer.lightbox.date")} value={photo.captureDate?.replace(" ", " · ")} />
          <Row label={t("viewer.lightbox.camera")} value={photo.camera} />
          <Row label={t("viewer.lightbox.lens")} value={photo.lens} />
          <Row label={t("viewer.lightbox.iso")} value={photo.iso} />
          <Row
            label={t("viewer.lightbox.aperture")}
            value={photo.aperture ? t("viewer.lightbox.apertureValue", { value: String(photo.aperture) }) : null}
          />
          <Row
            label={t("viewer.lightbox.shutter")}
            value={photo.shutter ? t("viewer.lightbox.shutterValue", { value: String(photo.shutter) }) : null}
          />
          <Row
            label={t("viewer.lightbox.focal")}
            value={photo.focalLength ? t("viewer.lightbox.focalValue", { value: String(photo.focalLength) }) : null}
          />
          <Row
            label={t("viewer.lightbox.size")}
            value={t("viewer.lightbox.sizeValue", { width: String(photo.width), height: String(photo.height) })}
          />

          <div className="section-label mt-4 px-0">{t("viewer.lightbox.quality")}</div>
          {focus ? (
            <div className="flex items-center justify-between gap-3 py-[3px]">
              <span className="shrink-0 text-faint">{t("viewer.lightbox.focus")}</span>
              <span
                className="flex items-center gap-2"
                title={t("viewer.lightbox.focusHint", { percentile: focus.percentile })}
              >
                <FocusMeter level={focus.level} />
                <span className="text-muted">{focus.label}</span>
              </span>
            </div>
          ) : (
            <Row
              label={t("viewer.lightbox.focus")}
              value={<span className="text-faint">{t("viewer.lightbox.notScored")}</span>}
            />
          )}
          <Row
            label={t("viewer.lightbox.faces")}
            value={
              !facesAnalysed(photo) ? (
                <span className="text-faint">{t("viewer.lightbox.notAnalysed")}</span>
              ) : photo.faceCount === 0 ? (
                <span className="text-faint">{t("common.none")}</span>
              ) : (
                formatNumber(photo.faceCount ?? 0)
              )
            }
          />
          {facesAnalysed(photo) && (photo.faceCount ?? 0) > 0 && (
            <div className="flex items-center justify-between gap-3 py-[3px]">
              <span className="shrink-0 text-faint">{t("viewer.lightbox.eyes")}</span>
              {eyesClosed(photo) ? (
                <EyesBadge
                  onClear={() =>
                    void useStore.getState().compareApply([photo.id], { eyesOverride: 0 })
                  }
                />
              ) : photo.eyesOverride != null ? (
                // Overridden open: say so, and offer the way back to the model's
                // own reading rather than making the correction one-way.
                <button
                  onClick={() =>
                    void useStore.getState().compareApply([photo.id], { eyesOverride: null })
                  }
                  className="flex items-center gap-1.5 rounded-sm px-1 py-0.5 text-positive transition-colors hover:bg-hover hover:text-ink"
                  title={t("viewer.lightbox.eyesOverrideHint")}
                >
                  <Icon name="eye" size={13} />
                  {t("viewer.lightbox.eyesOpen")}
                  <span className="text-label text-faint">{t("viewer.lightbox.eyesYours")}</span>
                </button>
              ) : photo.eyesOpen == null ? (
                // No face here the reading could see eyes on: a profile,
                // sunglasses, a face too small. Not "open".
                <span
                  className="text-faint"
                  title={t("viewer.lightbox.eyesCantTellHint")}
                >
                  {t("viewer.lightbox.eyesCantTell")}
                </span>
              ) : (
                <span className="flex items-center gap-1.5 text-positive">
                  <Icon name="eye" size={13} />
                  {t("viewer.lightbox.eyesOpen")}
                </span>
              )}
            </div>
          )}

          <LightboxFaces photo={photo} onChanged={onMutated} />

          <div className="section-label mt-4 px-0">{t("viewer.lightbox.classification")}</div>
          {/* Capitalised as the grid's chips and the selection panel's
              keywords are: it read "portrait" beside their "Portrait". */}
          <Row label={t("viewer.lightbox.genre")} value={photo.genre && sentenceCase(tagName(photo.genre))} />
          <Row label={t("viewer.lightbox.place")} value={photo.place ? localPlaceName(photo.place) : photo.place} />

          <PhotoDescription photo={photo} />

          <div className="section-label mt-4 px-0">{t("viewer.lightbox.tags")}</div>
          <div className="flex flex-wrap gap-1">
            {photo.subjects.length === 0 && <span className="text-faint">{t("viewer.lightbox.noTags")}</span>}
            {photo.subjects.map((s) => (
              <span key={s} className="chip">
                {sentenceCase(tagName(s))}
                <button
                  onClick={() => api.removeTag([photo.id], s).then(onMutated)}
                  className="chip-x"
                  aria-label={t("viewer.lightbox.removeTag", { tag: tagName(s) })}
                >
                  <Icon name="close" size={11} strokeWidth={2.4} />
                </button>
              </span>
            ))}
          </div>
          <input
            className="input mt-2"
            placeholder={t("viewer.lightbox.addTag")}
            value={newTag}
            onChange={(e) => setNewTag(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void addTag();
            }}
          />

          <div className="section-label mt-4 px-0">{t("viewer.lightbox.file")}</div>
          <div className="break-all text-label text-faint">{photo.path}</div>
        </div>

        <div className="border-t border-line px-3 py-2 text-label text-faint">
          {t("viewer.lightbox.footerHint", {
            prev: key("prev-photo"),
            next: key("next-photo"),
            rate0: key("rate-0"),
            rate5: key("rate-5"),
            close: key("close-overlay"),
          })}
        </div>
      </aside>
    </div>
  );
}
