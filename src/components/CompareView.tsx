import { useEffect, useMemo, useRef, useState } from "react";
import { overlayOpen } from "../lib/useEscape";
import { useFocusTrap } from "../lib/useFocusTrap";

import ViewAidsControl from "./ViewAidsControl";
import { textIsSelected } from "../lib/copyImage";
import type { PhotoFace } from "../bindings";
import { api, fileUrl, fullUrl } from "../lib/api";
import { everyoneOpen, faceEyesClosed, faceRows, rowFocus } from "../lib/faceRows";
import { Icon } from "../lib/icons";
import { useContentBox } from "../lib/useContentBox";
import FaceCrop from "./FaceCrop";
import { ColorLabelDot, FlagBadge } from "./PhotoBadges";
import ZoomableImage, { FITTED, type ZoomableImageHandle, type ZoomView } from "./ZoomableImage";
import { comboToLabel, resolveAction, type ActionId } from "../lib/keymap";
import {
  allCopies,
  eyesClosed,
  folderOf,
  isSoft,
  medianSharpness,
  sameShot,
  sharpestId,
  stepIndex,
  surveyLayout,
} from "../lib/survey";
import { t, type Key } from "../i18n";
import { DUPLICATE_THRESHOLD, useStore } from "../store";
import ResolveDuplicatesDialog from "./ResolveDuplicatesDialog";
import type { Photo } from "../types";

/** Gap between tiles. Small — the frames are the comparison, not the gutters. */
const GAP = 10;

/** Below this a frame is too small to judge, so a very large set scrolls
 *  instead of shrinking further. */
const MIN_TILE = 150;

/** How alike duplicates must look. Offered because the right cutoff depends on
 *  how you shoot: a burst at 8fps sits far higher than two takes of a portrait. */
// SigLIP 2 cosines, each mapped from the CLIP preset it replaces by
// matching how many pairs of a real library it groups.
const THRESHOLDS: { value: number; label: Key }[] = [
  { value: 0.855, label: "cull.compare.similarity.loose" },
  { value: DUPLICATE_THRESHOLD, label: "cull.compare.similarity.normal" },
  { value: 0.97, label: "cull.compare.similarity.strict" },
  { value: 0.99, label: "cull.compare.similarity.nearIdentical" },
];

/** The set's typical aspect, so the tiles waste as little space as possible.
 *  Median rather than mean: one panorama in a set of stills should not squash
 *  every other frame. */
function setAspect(photos: Photo[]): number {
  const ratios = photos
    .map((p) => (p.width > 0 && p.height > 0 ? p.width / p.height : 0))
    .filter((r) => r > 0)
    .sort((a, b) => a - b);
  return ratios[Math.floor(ratios.length / 2)] ?? 3 / 2;
}

/** Shot settings, the line that usually decides which frame survives. */
function exposureLine(p: Photo): string {
  return [
    p.aperture ? `ƒ/${p.aperture}` : null,
    p.shutter ? `${p.shutter}s` : null,
    p.iso ? `ISO ${p.iso}` : null,
    p.focalLength ? `${p.focalLength}mm` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * Compare / survey — several frames at once, at the same scale, to decide which
 * one survives.
 *
 * It is the review half of AI culling: `duplicate_groups` finds the bursts and
 * near-copies, and this is where the choice actually gets made, one set at a
 * time. It also serves the plain case of comparing an arbitrary selection.
 *
 * Deciding writes flags, never deletions — "keep this one" picks the survivor
 * and rejects its set-mates, which is visible, filterable and undoable, and
 * leaves removing files as the user's own deliberate act.
 */
export default function CompareView() {
  const compare = useStore((s) => s.compare);
  const compareBusy = useStore((s) => s.compareBusy);
  const closeCompare = useStore((s) => s.closeCompare);
  const compareGoTo = useStore((s) => s.compareGoTo);
  const compareFocus = useStore((s) => s.compareFocus);
  const compareApply = useStore((s) => s.compareApply);
  const compareKeep = useStore((s) => s.compareKeep);
  const compareRejectExtras = useStore((s) => s.compareRejectExtras);
  const reviewDuplicates = useStore((s) => s.reviewDuplicates);
  const selectDuplicates = useStore((s) => s.selectDuplicates);
  // Every hint names the key it is bound to now, so rebinding one in
  // Settings changes what the hints say. Typed out, they drift: this
  // one said "Pick (P)" while pick was C.
  const keymap = useStore((s) => s.keymap);
  const key = (id: ActionId) => comboToLabel(keymap[id]);

  const boxRef = useRef<HTMLDivElement>(null);
  // Tiles are sized from the measured area, like the grid: the view has to work
  // at the 940px window floor and at full screen without a breakpoint.
  const box = useContentBox(boxRef);

  // One zoom for the whole set: judging which frame of a burst is sharp
  // means looking at the same eye in each, at 1:1, and moving to the next eye
  // in all of them at once. Each tile starts a new set fitted.
  const [view, setView] = useState<ZoomView>(FITTED);
  const aids = useStore((s) => s.viewAids.compare);
  const peakColor = useStore((s) => s.peakColor);
  const peakSensitivity = useStore((s) => s.peakSensitivity);
  const [resolving, setResolving] = useState(false);
  // Per tile, so `Z` can ask the focused frame for its own 1:1 scale.
  const controls = useRef(new Map<number, { current: ZoomableImageHandle | null }>());
  const controlFor = (id: number) => {
    let ref = controls.current.get(id);
    if (!ref) {
      ref = { current: null };
      controls.current.set(id, ref);
    }
    return ref;
  };

  // Faces across the set: fetched per set, only when some frame has one.
  const setPhotos = useMemo(() => compare?.sets[compare.index] ?? [], [compare]);
  const idsKey = setPhotos.map((p) => p.id).join(",");
  const anyFaces = setPhotos.some((p) => (p.faceCount ?? 0) > 0);
  const [faces, setFaces] = useState<PhotoFace[] | null>(null);
  useEffect(() => {
    setFaces(null);
    if (!anyFaces || !idsKey) return;
    let live = true;
    api
      .photoFaces(idsKey.split(",").map(Number))
      .then((f) => live && setFaces(f))
      .catch(() => live && setFaces([]));
    return () => {
      live = false;
    };
  }, [idsKey, anyFaces]);
  const { rows, more } = useMemo(
    () => (faces ? faceRows(idsKey.split(",").map(Number), faces) : { rows: [], more: 0 }),
    [faces, idsKey],
  );
  // Frames where everyone's eyes are open: the one a group shot wants.
  const allOpen = useMemo(() => {
    const out = new Set<number>();
    if (!faces) return out;
    for (const p of setPhotos) {
      if (everyoneOpen(faces.filter((f) => f.photoId === p.id), p.eyesOverride)) out.add(p.id);
    }
    return out;
  }, [faces, setPhotos]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "SELECT" || el.isContentEditable)) return;
      // A dialog or menu on top (Resolve all, a confirmation) owns the keyboard.
      if (overlayOpen()) return;

      const state = useStore.getState();
      const current = state.compare;
      const photos = current?.sets[current.index];
      if (!current || !photos) return;

      const action = resolveAction(state.keymap, "compare", e);
      if (!action) return;
      if (action === "undo-verdict") {
        e.preventDefault();
        void state.undoVerdict();
        return;
      }
      const focusId = current.focusId ?? photos[0]?.id ?? null;
      const at = photos.findIndex((p) => p.id === focusId);

      if (action === "close-overlay") state.closeCompare();
      else if (action === "prev-photo" || action === "next-photo") {
        const next = photos[stepIndex(at, action === "next-photo" ? 1 : -1, photos.length)];
        if (next) state.compareFocus(next.id);
      } else if (action === "compare-prev-set") state.compareGoTo(current.index - 1);
      else if (action === "compare-next-set") state.compareGoTo(current.index + 1);
      else if (action === "toggle-peaking") state.togglePeaking("compare");
      else if (action === "cycle-grid") state.cycleGrid("compare");
      else if (action === "toggle-zoom") {
        const id = focusId ?? photos[0]?.id;
        if (id !== undefined) controls.current.get(id)?.current?.toggle();
      }
      else if (focusId === null) return;
      else if (action === "copy-image") {
        if (textIsSelected()) return;
        void state.copyImage(focusId);
      } else if (action === "compare-keep") void state.compareKeep(focusId);
      else if (action === "compare-reject-extras") void state.compareRejectExtras(focusId);
      else if (action === "open-develop") {
        state.closeCompare();
        void state.openDevelop(focusId);
      } else if (action.startsWith("rate-"))
        void state.compareApply([focusId], { rating: Number(action.slice(5)) });
      else if (action === "flag-pick") void state.compareApply([focusId], { flag: 1 });
      else if (action === "flag-reject") void state.compareApply([focusId], { flag: -1 });
      else if (action === "flag-none") void state.compareApply([focusId], { flag: 0 });
      else if (action === "label-none") void state.compareApply([focusId], { colorLabel: 0 });
      else if (action.startsWith("label-"))
        void state.compareApply([focusId], { colorLabel: Number(action.slice(6)) });
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Tab stays in the view, and focus goes back where it was on close.
  const rootRef = useRef<HTMLDivElement>(null);
  useFocusTrap(compare !== null, rootRef);

  if (!compare) return null;

  const photos = setPhotos;
  const zoomKey = key("toggle-zoom");
  const isDuplicates = compare.source === "duplicates";
  const layout = surveyLayout(
    photos.length,
    box.width,
    box.height,
    GAP,
    setAspect(photos),
    MIN_TILE,
  );
  const focusId = compare.focusId ?? photos[0]?.id ?? null;
  const focused = photos.find((p) => p.id === focusId) ?? null;
  // The sharpest frame is the suggested keeper — marked, never auto-picked; the
  // reviewer still presses Keep. Soft frames (relative to this set) are flagged.
  const suggestedId = sharpestId(photos);
  const median = medianSharpness(photos);
  // A set that is one photo in several folders: the frames are identical, so
  // "sharpest" means nothing and the choice is which folder keeps it.
  const copies = isDuplicates && allCopies(photos);

  return (
    // Opaque: at 92% black the workspace underneath showed through as a ghost,
    // most visibly through a rejected (dimmed) frame.
    <div
      ref={rootRef}
      tabIndex={-1}
      role="dialog"
      aria-label={isDuplicates ? t("cull.compare.titleDuplicates") : t("cull.compare.title")}
      className="anim-view fixed inset-0 z-50 flex flex-col bg-canvas-void outline-none"
    >
      <header
        className="titlebar flex h-11 shrink-0 items-center gap-2 border-b border-line bg-panel px-3"
        data-tauri-drag-region="deep"
      >
        <Icon name="compare" size={15} className="shrink-0 text-accent" />
        <span className="shrink-0 whitespace-nowrap text-caption font-medium text-ink">
          {isDuplicates ? t("cull.compare.titleDuplicates") : t("cull.compare.title")}
        </span>

        {compare.sets.length > 1 && (
          <div className="ml-2 flex shrink-0 items-center gap-1">
            <button
              onClick={() => compareGoTo(compare.index - 1)}
              disabled={compare.index === 0}
              className="icon-chip disabled:opacity-30"
              title={t("cull.compare.previousSet")}
            >
              <Icon name="chevron" size={15} className="rotate-90" />
            </button>
            <span className="min-w-[74px] text-center text-caption tabular-nums text-muted">
              {t("cull.compare.setPosition", { current: compare.index + 1, total: compare.sets.length })}
            </span>
            <button
              onClick={() => compareGoTo(compare.index + 1)}
              disabled={compare.index === compare.sets.length - 1}
              className="icon-chip disabled:opacity-30"
              title={t("cull.compare.nextSet")}
            >
              <Icon name="chevron" size={15} className="-rotate-90" />
            </button>
          </div>
        )}

        <span className="whitespace-nowrap text-caption text-faint">
          {copies
            ? t("cull.compare.copiesSummary", { count: photos.length })
            : t("cull.compare.frameCount", { count: photos.length })}
        </span>

        {isDuplicates && (
          <label className="ml-2 flex shrink-0 items-center gap-1.5 text-caption text-faint">
            <span className="max-xl:hidden">{t("cull.compare.similarity")}</span>
            <select
              className="rounded-sm border border-line bg-base px-1.5 py-1 text-caption text-ink outline-none"
              aria-label={t("cull.compare.similarity")}
              title={t("cull.compare.similarity")}
              value={compare.threshold}
              disabled={compareBusy}
              onChange={(e) => {
                e.currentTarget.blur();
                void reviewDuplicates(Number(e.target.value));
              }}
            >
              {THRESHOLDS.map((th) => (
                <option key={th.value} value={th.value}>
                  {t(th.label)}
                </option>
              ))}
            </select>
          </label>
        )}

        {view.scale > 1 && (
          <span className="truncate text-caption text-faint max-xl:hidden">
            {t("cull.compare.zoomedTogether", { key: zoomKey })}
          </span>
        )}

        {/* Below 1280px the secondary actions drop to their icon, as Cull's
            header does: at the minimum window their labels ran Keep this one
            and Close off the right edge — by 64px in English, 247 in German. */}
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          <ViewAidsControl place="compare" className="relative z-20 mr-1" />
          {isDuplicates && compare.sets.length > 1 && (
            <button
              onClick={() => setResolving(true)}
              className="btn-ghost text-muted"
              title={t("viewer.resolve.openTitle")}
              aria-label={t("viewer.resolve.open")}
            >
              <Icon name="check" size={14} />
              <span className="max-xl:hidden">{t("viewer.resolve.open")}</span>
            </button>
          )}
          {isDuplicates && (
            <button
              onClick={() => {
                closeCompare();
                void selectDuplicates();
              }}
              className="btn-ghost text-muted"
              title={t("cull.compare.selectExtrasTip")}
              aria-label={t("cull.compare.selectExtras")}
            >
              <Icon name="select" size={14} />
              <span className="max-xl:hidden">{t("cull.compare.selectExtras")}</span>
            </button>
          )}
          <button
            onClick={() => focused && void compareRejectExtras(focused.id)}
            disabled={!focused || photos.length < 2}
            className="btn-ghost text-muted"
            title={t("cull.compare.rejectExtrasTip", { key: key("compare-reject-extras") })}
            aria-label={t("cull.compare.rejectExtras")}
          >
            <Icon name="flag-off" size={14} />
            <span className="max-xl:hidden">{t("cull.compare.rejectExtras")}</span>
          </button>
          <button
            onClick={() => focused && void compareKeep(focused.id)}
            disabled={!focused || photos.length < 2}
            className="btn-accent"
            title={t("cull.compare.keepThisTip", { key: key("compare-keep") })}
          >
            {t("cull.compare.keepThis")}
          </button>
          <button
            onClick={closeCompare}
            className="icon-chip"
            title={t("cull.compare.closeTip", { key: key("close-overlay") })}
            aria-label={t("cull.compare.closeTip", { key: key("close-overlay") })}
          >
            <Icon name="close" size={15} />
          </button>
        </div>
      </header>

      <div ref={boxRef} className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden p-3">
        {compareBusy && (
          <div className="absolute inset-0 z-10 grid place-items-center text-caption text-muted">
            {t("cull.compare.findingDuplicates")}
          </div>
        )}
        {/* `min-h-full` rather than `h-full`: a set past the tile floor grows
            past the box and scrolls, and centring then becomes a no-op. */}
        <div
          className="flex min-h-full w-full flex-wrap content-center justify-center"
          style={{ gap: GAP }}
        >
          {photos.map((p) => {
            const isFocused = p.id === focusId;
            return (
              <div
                key={p.id}
                onClick={() => compareFocus(p.id)}
                style={{ width: layout.tileW, height: layout.tileH }}
                // Double-click zooms, as in the loupe and the Lightbox; Develop
                // is D. It used to open Develop, which the zoom now needs.
                className={`group relative flex overflow-hidden rounded-sm bg-raised ring-offset-1 ring-offset-black transition-opacity ${
                  isFocused ? "ring-2 ring-accent" : "hover:ring-1 hover:ring-line"
                } ${p.flag === -1 ? "opacity-45" : ""}`}
              >
                {p.thumbPath || p.previewPath ? (
                  <ZoomableImage
                    src={fileUrl(p.previewPath ?? p.thumbPath)}
                    fullSrc={fullUrl(p.id)}
                    alt={p.filename}
                    resetKey={p.id}
                    controlRef={controlFor(p.id)}
                    view={view}
                    onViewChange={setView}
                    peaking={aids.peaking ? { color: peakColor, sensitivity: peakSensitivity } : null}
                    grid={aids.grid}
                  />
                ) : (
                  <div className="grid h-full w-full place-items-center text-faint">?</div>
                )}

                {/* Over-image chrome: white on black scrims, no accent — the
                    photograph is the thing being judged. */}
                <div className="pointer-events-none absolute left-1.5 top-1.5 flex items-center gap-1">
                  <FlagBadge flag={p.flag} labelled />
                  <ColorLabelDot label={p.colorLabel} />
                </div>

                {/* Focus / face cues, top-right: the suggested keeper (sharpest),
                    a blink warning, or a soft-focus warning — recommendations,
                    never actions. Eyes-closed shows even on the keeper, since a
                    sharp frame with a blink still matters. */}
                <div className="pointer-events-none absolute right-1.5 top-1.5 flex items-center gap-1">
                  {allOpen.has(p.id) && (
                    <span
                      className="inline-flex items-center gap-1 rounded-sm bg-chip px-1 text-label text-positive backdrop-blur"
                      title={t("cull.compare.allOpenTip")}
                    >
                      <Icon name="eye" size={11} />
                      {t("cull.compare.allOpen")}
                    </span>
                  )}
                  {!copies && photos.length > 1 && p.id !== suggestedId && photos.some((o) => sameShot(p, o)) && (
                    <span
                      className="inline-flex items-center gap-1 rounded-sm bg-chip px-1 text-label text-white/85 backdrop-blur"
                      title={t("cull.compare.copyTip")}
                    >
                      <Icon name="copy" size={11} />
                      {t("cull.compare.copy")}
                    </span>
                  )}
                  {!copies && photos.length > 1 && p.id === suggestedId && (
                    <span
                      // The suggested keeper is a pick in waiting, so the
                      // pick's green — not the amber of the blink beside it.
                      className="inline-flex items-center gap-1 rounded-sm bg-chip px-1 text-label text-positive backdrop-blur"
                      title={t("cull.compare.sharpestTip")}
                    >
                      <Icon name="star" size={11} />
                      {t("cull.compare.sharpest")}
                    </span>
                  )}
                  {eyesClosed(p) && (
                    // Clickable here too: this badge is the reason the frame is
                    // ranked below its neighbours in the set being compared.
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void compareApply([p.id], { eyesOverride: 0 });
                      }}
                      className="inline-flex items-center gap-1 rounded-sm bg-chip px-1 text-label text-warning backdrop-blur transition-colors hover:text-ink"
                      title={t("cull.compare.eyesTip")}
                    >
                      <Icon name="eye-off" size={11} />
                      {t("cull.compare.eyes")}
                    </button>
                  )}
                  {!copies && !eyesClosed(p) && p.id !== suggestedId && isSoft(p, median) && (
                    <span
                      className="inline-flex items-center gap-1 rounded-sm bg-chip px-1 text-label text-warning backdrop-blur"
                      title={t("cull.compare.softTip")}
                    >
                      <Icon name="warn" size={11} />
                      {t("cull.compare.soft")}
                    </span>
                  )}
                </div>

                <div className="absolute inset-x-0 bottom-0 flex items-center gap-2 bg-gradient-to-t from-black/85 to-transparent px-2 pb-1.5 pt-5 text-caption text-white/90">
                  <div className="min-w-0 flex-1">
                    <div className="truncate" title={p.filename}>
                      {p.filename}
                    </div>
                    <div
                      className="truncate text-label tabular-nums text-white/55"
                      title={exposureLine(p) || `${p.width} × ${p.height}`}
                    >
                      {exposureLine(p) || `${p.width} × ${p.height}`}
                    </div>
                    {/* Where the file is: for two copies of one photo, the only
                        difference there is. */}
                    <div className="flex items-center gap-1 truncate text-label text-white/55" title={p.path}>
                      <Icon name="folder" size={11} className="shrink-0" />
                      <span className="truncate">{folderOf(p.path)}</span>
                    </div>
                  </div>

                  <div className="flex shrink-0 items-center">
                    {[1, 2, 3, 4, 5].map((n) => (
                      <button
                        key={n}
                        onClick={(e) => {
                          e.stopPropagation();
                          void compareApply([p.id], { rating: p.rating === n ? 0 : n });
                        }}
                        aria-label={t("cull.compare.rate", { rating: n })}
                        aria-pressed={p.rating === n}
                        // A 24px-tall target, not the 12px glyph (WCAG 2.5.8).
                        className={`grid h-6 w-5 place-items-center text-caption leading-none ${
                          p.rating >= n ? "text-white" : "text-white/30 hover:text-white/70"
                        }`}
                        title={t("cull.compare.rate", { rating: n })}
                      >
                        ★
                      </button>
                    ))}
                  </div>

                  <div className="flex shrink-0 items-center gap-1">
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void compareApply([p.id], { flag: p.flag === 1 ? 0 : 1 });
                      }}
                      aria-pressed={p.flag === 1}
                      className={`leading-none ${
                        p.flag === 1 ? "text-positive" : "text-white/40 hover:text-white"
                      }`}
                      title={t("cull.compare.pickTip", { key: key("flag-pick") })}
                    >
                      <Icon name="flag" size={14} />
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void compareApply([p.id], { flag: p.flag === -1 ? 0 : -1 });
                      }}
                      aria-pressed={p.flag === -1}
                      className={`leading-none ${
                        p.flag === -1 ? "text-danger" : "text-white/40 hover:text-white"
                      }`}
                      title={t("cull.compare.rejectTip", { key: key("flag-reject") })}
                    >
                      <Icon name="flag-off" size={14} />
                    </button>
                  </div>

                  {photos.length > 1 && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void compareKeep(p.id);
                      }}
                      className="shrink-0 rounded-sm bg-chip px-2 py-0.5 text-label text-white/80 opacity-0 backdrop-blur transition-opacity hover:text-white focus-visible:opacity-100 group-hover:opacity-100"
                      title={t("cull.compare.keepTip")}
                    >
                      {t("cull.compare.keep")}
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Faces across the set: one row per person, one column per frame,
          so who blinked where, and whose face is soft in which frame, read at a
          glance. Focus is compared along a row only: the same person is about
          the same size in every frame of a burst, which two different faces in
          one frame are not. */}
      {rows.length > 0 && (
        <section className="max-h-[40%] shrink-0 overflow-auto border-t border-line bg-panel px-3 py-2">
          <div className="flex items-baseline gap-2 pb-1.5">
            <span className="section-label px-0 pb-0">{t("cull.compare.faces")}</span>
            <span className="text-caption text-faint">
              {t("cull.compare.facesHelp")}
            </span>
          </div>
          <table className="border-separate" style={{ borderSpacing: 6 }}>
            <thead>
              <tr>
                <th />
                {photos.map((p, i) => (
                  <th
                    key={p.id}
                    className={`text-label font-normal tabular-nums ${p.id === focusId ? "text-ink" : "text-faint"}`}
                  >
                    <span className="inline-flex items-center gap-1">
                      {i + 1}
                      {allOpen.has(p.id) && (
                        <span className="text-positive" title={t("cull.compare.allOpenTip")}>
                          <Icon name="eye" size={11} />
                        </span>
                      )}
                    </span>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, r) => {
                const focus = rowFocus(row);
                return (
                  <tr key={row.personId ?? `face-${r}`}>
                    <th className="max-w-[120px] truncate pr-2 text-left text-caption font-normal text-muted">
                      {row.name ?? t("cull.compare.unnamed")}
                    </th>
                    {row.cells.map((f, i) => {
                      const p = photos[i]!;
                      if (!f) {
                        return (
                          <td key={p.id} className="text-center text-caption text-faint" title={t("cull.compare.notInFrame")}>
                            —
                          </td>
                        );
                      }
                      const closed = faceEyesClosed(f, p.eyesOverride);
                      const sharpest = focus.sharpest === f.faceId;
                      const soft = focus.soft.has(f.faceId);
                      const why = [
                        closed ? t("cull.compare.whyEyesClosed") : null,
                        sharpest
                          ? t("cull.compare.whySharpest")
                          : soft
                            ? t("cull.compare.whySoft")
                            : null,
                      ].filter((w): w is string => w !== null);
                      const who = row.name ?? t("cull.compare.unnamed");
                      return (
                        <td key={p.id}>
                          <button
                            onClick={() => compareFocus(p.id)}
                            title={
                              why.length
                                ? t("cull.compare.faceCellWhy", { name: who, frame: i + 1, why: why.join(" · ") })
                                : t("cull.compare.faceCell", { name: who, frame: i + 1 })
                            }
                            className={`relative block overflow-hidden rounded-sm ring-offset-1 ring-offset-black ${
                              p.id === focusId ? "ring-2 ring-accent" : "hover:ring-1 hover:ring-line"
                            }`}
                          >
                            <FaceCrop src={fileUrl(p.thumbPath ?? p.previewPath)} box={f.boxFrac} portrait={f.portraitPath} size={56} />
                            <span className="absolute right-0.5 top-0.5 flex flex-col gap-0.5">
                              {closed && (
                                <span className="grid h-3.5 w-3.5 place-items-center rounded-sm bg-chip text-warning">
                                  <Icon name="eye-off" size={9} />
                                </span>
                              )}
                              {soft && (
                                <span className="grid h-3.5 w-3.5 place-items-center rounded-sm bg-chip text-warning">
                                  <Icon name="warn" size={9} />
                                </span>
                              )}
                              {sharpest && (
                                <span className="grid h-3.5 w-3.5 place-items-center rounded-sm bg-chip text-white/85">
                                  <Icon name="star" size={9} />
                                </span>
                              )}
                            </span>
                          </button>
                        </td>
                      );
                    })}
                  </tr>
                );
              })}
            </tbody>
          </table>
          {more > 0 && (
            <div className="pt-1 text-caption text-faint">
              {t("cull.compare.moreFaces", { count: more })}
            </div>
          )}
        </section>
      )}

      <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-line bg-panel px-3 text-caption text-faint">
        {/* One line, whatever the width: the bar has a fixed height, and at the
            940px floor the hint wrapped out of it. What's cut off is in the tooltip. */}
        {(() => {
          const hint = t(compare.sets.length > 1 ? "cull.compare.hintSets" : "cull.compare.hint", {
            prev: key("prev-photo"),
            next: key("next-photo"),
            keep: key("compare-keep"),
            extras: key("compare-reject-extras"),
            prevSet: key("compare-prev-set"),
            nextSet: key("compare-next-set"),
            zoom: zoomKey,
            rate1: key("rate-1"),
            rate5: key("rate-5"),
            pick: key("flag-pick"),
            reject: key("flag-reject"),
            develop: key("open-develop"),
            close: key("close-overlay"),
          });
          return (
            <span className="min-w-0 truncate" title={hint}>
              {hint}
            </span>
          );
        })()}
        <span className="ml-auto shrink-0">
          {isDuplicates
            ? t("cull.compare.extraCopies", {
                copies: compare.sets.reduce((n, s) => n + s.length - 1, 0),
                count: compare.sets.length,
              })
            : t("cull.compare.selectedCount", { count: photos.length })}
        </span>
      </footer>
      {resolving && <ResolveDuplicatesDialog onClose={() => setResolving(false)} />}
    </div>
  );
}
