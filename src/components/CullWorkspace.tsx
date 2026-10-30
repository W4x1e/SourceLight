import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePreload } from "../lib/usePreload";
import { sameFrames, useStable } from "../lib/useStable";
import { tileName } from "../lib/tileName";
import { cullScopeKey, cullScopeLabel } from "../lib/cullScope";
import { isTextEntry, overlayOpen } from "../lib/useEscape";
import type { PeakingStatus } from "./FocusPeaking";
import ViewAidsControl from "./ViewAidsControl";
import { textIsSelected } from "../lib/copyImage";
import { useVirtualizer } from "@tanstack/react-virtual";
import { listen } from "@tauri-apps/api/event";
import { api, fileUrl, fullUrl } from "../lib/api";
import { ensureAi } from "../lib/aiReady";
import { formatNumber, t, tx, type Key } from "../i18n";
import { developShown, useStore } from "../store";
import { prompt, toast } from "../lib/dialogs";
import { Icon } from "../lib/icons";
import { trashName, IS_MAC } from "../lib/platform";
import { useContentBox } from "../lib/useContentBox";
import ZoomableImage, {
  type ZoomableImageHandle,
  type ZoomSource,
  zoomSourceLabel,
} from "./ZoomableImage";
import { comboToLabel, resolveAction, type ActionId } from "../lib/keymap";
import { clickSelect, flagForAll } from "../lib/cullSelection";
import {
  eyesClosed,
  facesAnalysed,
  focusBand,
  isSoft,
  medianOfSorted,
  medianSharpness,
  releaseIndex,
  resumeIndex,
  sharpnessScores,
  stepIndex,
  surveyLayout,
} from "../lib/survey";
import { EyesBadge, FaceCountBadge, FocusMeter } from "./QualityBits";
import { COLOR_LABELS, type Photo } from "../types";
import { SUGGEST_REASON, suggestCull, suggestionCounts, untouched, type Suggestion } from "../lib/suggest";
import { faceEyesClosed } from "../lib/faceRows";
import type { PhotoFace } from "../bindings";
import FaceCrop from "./FaceCrop";

/**
 * Cull — a first-class home for the highest-volume job in the app. What used to
 * be scattered across rating keys, a "Rejected" toggle, a Duplicates button and
 * Compare-via-selection is one keyboard-first surface: a loupe to pass through
 * frames one at a time, a survey to judge many at one scale, a running
 * pick/reject tally, and a colour-coded filmstrip.
 *
 * It reuses the compare/survey engine — `compareApply` writes a verdict in place
 * (so the frame never jumps), `surveyLayout` sizes the survey — rather than
 * inventing new machinery.
 */

const STRIPS = [
  { id: "all", label: "cull.strip.all" },
  { id: "unrated", label: "cull.strip.toReview" },
  { id: "picks", label: "cull.strip.picks" },
  // What "Bin N rejects…" would bin, to check first.
  { id: "rejects", label: "cull.strip.rejects" },
  // Only offered while suggestions are shown: the frames the suggested
  // cull would reject, to check before accepting — or before binning.
  { id: "suggested", label: "cull.strip.suggestedRejects" },
] as const satisfies readonly { id: string; label: Key }[];
type Strip = (typeof STRIPS)[number]["id"];

/** Untouched = no rating and no flag: what "still to review" means. A colour
 *  label alone doesn't count: it marks a frame for something — to edit,
 *  for the client — rather than judging it. */
const isUntouched = untouched;

const NO_SUGGESTIONS = new Map<number, Suggestion>();
const NO_IDS: ReadonlySet<number> = new Set();

/** How many close-ups the verdict panel shows: the largest faces first. */
const MAX_CLOSE_UPS = 6;

/** A suggestion wears its verdict's colour, drawn lighter than a real flag. */
const suggestColor = (s: Suggestion) => (s.flag > 0 ? "var(--color-positive)" : "var(--color-danger)");
const suggestVerdict = (s: Suggestion) => t(s.flag > 0 ? "cull.suggest.pick" : "cull.suggest.reject");
const suggestLabel = (s: Suggestion) =>
  t("cull.suggest.withReason", { verdict: suggestVerdict(s), reason: SUGGEST_REASON[s.reason] });

/** Survey grid spacing, and the tile size past which it scrolls rather than shrinks. */
const SURVEY_GAP = 10;
const SURVEY_MIN_TILE = 150;

/** Filmstrip tile geometry — 68px tall at 3:2, plus the gap between frames. */
const STRIP_THUMB_H = 68;
const STRIP_THUMB_W = Math.round(STRIP_THUMB_H * (3 / 2));
const STRIP_GAP = 8;

/**
 * Derivatives are 512px on the long edge for thumbs and 1600px for previews
 * (`raw::write_derivatives`). A survey tile is 150px at its smallest and only
 * exceeds thumb resolution when a handful of frames fill the window — so use
 * the thumb until the tile actually outgrows it, rather than pulling a 1600px
 * preview for every tile on screen. "Outgrows" is in device pixels: at 150%
 * scaling a 400px tile is 600 real pixels, and a 512px thumb there is soft.
 */
const THUMB_EDGE = 512;

function exposureLine(p: Photo): string {
  return [p.aperture ? `ƒ/${p.aperture}` : null, p.shutter ? `${p.shutter}s` : null, p.iso ? `ISO ${p.iso}` : null]
    .filter(Boolean)
    .join(" · ");
}

export default function CullWorkspace() {
  const allPhotos = useStore((s) => s.searchResults ?? s.photos);
  const compareApply = useStore((s) => s.compareApply);
  const openDevelop = useStore((s) => s.openDevelop);
  const setMode = useStore((s) => s.setMode);
  const reviewDuplicates = useStore((s) => s.reviewDuplicates);
  const compareBusy = useStore((s) => s.compareBusy);
  const reload = useStore((s) => s.reload);
  const selectPhotos = useStore((s) => s.selectPhotos);
  const autoAdvance = useStore((s) => s.cullAutoAdvance);
  const setAutoAdvance = useStore((s) => s.setCullAutoAdvance);
  const sort = useStore((s) => s.cullSort);
  const setSort = useStore((s) => s.setCullSort);
  const keymap = useStore((s) => s.keymap);
  const key = (id: ActionId) => comboToLabel(keymap[id]);
  const suggestOn = useStore((s) => s.cullSuggest);
  const setSuggestOn = useStore((s) => s.setCullSuggest);
  const cullGroups = useStore((s) => s.cullGroups);
  const loadCullGroups = useStore((s) => s.loadCullGroups);
  const acceptSuggestions = useStore((s) => s.acceptSuggestions);
  const keepZoom = useStore((s) => s.cullKeepZoom);
  const setKeepZoom = useStore((s) => s.setCullKeepZoom);

  // Focus-scoring backfill: photos scanned before this feature have no score, so
  // offer a one-off pass (no model, no download) right where culling happens.
  const [unscored, setUnscored] = useState<number | null>(null);
  const [scoring, setScoring] = useState<{ current: number; total: number } | null>(null);
  const refreshUnscored = useCallback(
    () => void api.qualityStatus().then(setUnscored).catch(() => setUnscored(null)),
    [],
  );
  useEffect(() => refreshUnscored(), [refreshUnscored]);
  useEffect(() => {
    const un = listen<{ current: number; total: number; phase: string }>("quality-progress", (e) => {
      const p = e.payload;
      if (p.phase === "done") {
        setScoring(null);
        refreshUnscored();
        void reload(); // pull the fresh scores into the grid the cull reads from
      } else {
        setScoring({ current: p.current, total: p.total });
      }
    });
    return () => void un.then((f) => f());
  }, [refreshUnscored, reload]);
  const startScoring = () => {
    setScoring({ current: 0, total: unscored ?? 1 });
    void api.scoreQuality().catch((e) => {
      console.error("focus scoring failed", e);
      setScoring(null);
    });
  };

  // Faces backfill: a heavier, optional pass (needs the "Face tools" model) that
  // finds faces and flags closed eyes / soft faces. Same shape as focus scoring.
  const [unanalysed, setUnanalysed] = useState<number | null>(null);
  const [analysing, setAnalysing] = useState<{ current: number; total: number } | null>(null);
  const [facesError, setFacesError] = useState(false);
  const refreshUnanalysed = useCallback(
    // Cull runs detection only; faces waiting for the People recogniser are
    // that page's business.
    () => void api.facesStatus().then((s) => setUnanalysed(s.toDetect)).catch(() => setUnanalysed(null)),
    [],
  );
  useEffect(() => refreshUnanalysed(), [refreshUnanalysed]);
  useEffect(() => {
    const un = listen<{ current: number; total: number; phase: string }>("faces-progress", (e) => {
      const p = e.payload;
      if (p.phase === "done") {
        setAnalysing(null);
        refreshUnanalysed();
        void reload(); // pull the fresh face signals into the grid
      } else {
        setAnalysing({ current: p.current, total: p.total });
      }
    });
    return () => void un.then((f) => f());
  }, [refreshUnanalysed, reload]);
  /**
   * "Not closed" — the downcast gaze the classifier reads as a blink.
   *
   * Goes through `compareApply` so the frame is patched in place rather than
   * reloaded: a cull decision must not make the photo under the cursor jump,
   * and this one changes the keeper ranking, which would otherwise reorder the
   * set you are looking at.
   */
  const clearEyes = (photoId: number) =>
    useStore.getState().compareApply([photoId], { eyesOverride: 0 });

  const startAnalysing = async () => {
    setFacesError(false);
    // Fetches the ~1 MB "Face tools" bundle if it isn't there, rather than
    // failing and pointing at a Settings tab the user then has to find.
    if (!(await ensureAi("faces"))) return;
    setAnalysing({ current: 0, total: unanalysed ?? 1 });
    void api.analyzeFaces().catch((e) => {
      console.error("face analysis failed", e);
      setAnalysing(null);
      setFacesError(true);
    });
  };

  // The saved session is read once, at mount: this workspace owns the
  // position from then on and only writes it back.
  // Only a position from this same set: a session saved on another
  // shoot started this one at its old frame number. Elsewhere it starts at
  // the first frame still to review. A shoot's "Continue culling" saves -1.
  const [saved] = useState(() => {
    const st = useStore.getState();
    const session = st.cullSession;
    if (!session || session.photoId === -1) return session;
    if (session.scope === cullScopeKey(st.filter, st.searchLabel)) return session;
    return { photoId: -1, index: 0, strip: "unrated" as const, view: session.view };
  });
  const setCullSession = useStore((s) => s.setCullSession);
  const [view, setView] = useState<"loupe" | "survey">(saved?.view ?? "loupe");

  // The key legend teaches, then gets out of the way. It sat over the
  // bottom of the frame permanently — the part of the photo being judged, at
  // 1:1 too. Now it shows on arrival and whenever the pointer moves over the
  // loupe, and fades once the pointer rests: someone deciding with the
  // keyboard already knows the keys.
  const [legendShown, setLegendShown] = useState(true);
  const legendTimer = useRef<number | undefined>(undefined);
  const nudgeLegend = useCallback(() => {
    setLegendShown(true);
    window.clearTimeout(legendTimer.current);
    legendTimer.current = window.setTimeout(() => setLegendShown(false), 2500);
  }, []);
  useEffect(() => {
    nudgeLegend();
    return () => window.clearTimeout(legendTimer.current);
  }, [nudgeLegend]);

  const zoomRef = useRef<ZoomableImageHandle | null>(null);
  const aids = useStore((s) => s.viewAids.cull);
  const peakColor = useStore((s) => s.peakColor);
  const peakSensitivity = useStore((s) => s.peakSensitivity);
  const [peakStatus, setPeakStatus] = useState<PeakingStatus | null>(null);
  const [zoomed, setZoomed] = useState(false);
  const [zoomSource, setZoomSource] = useState<ZoomSource>("preview");
  const [strip, setStrip] = useState<Strip>(saved?.strip ?? "all");
  // `sort` and `autoAdvance` live in the store so they persist across sessions.
  const [cursor, setCursor] = useState(0);
  // Frames chosen together in the survey, by photo id. A verdict key
  // applies to all of them; with none chosen it applies to the cursor's frame.
  const [chosen, setChosen] = useState<Set<number>>(() => new Set());
  // Frames judged without advancing, kept in a strip they no longer belong to
  // (a pick in "To review") so the frame stays under the cursor until you move
  // off it. Without this the strip closed up on every verdict and the next
  // frame slid in, which read as auto-advance being on when it was off.
  // Belongs to the strip and view it was made in.
  const [held, setHeld] = useState<{ strip: Strip; view: typeof view; ids: ReadonlySet<number> }>(() => ({
    strip,
    view,
    ids: NO_IDS,
  }));
  const heldIds = held.strip === strip && held.view === view ? held.ids : NO_IDS;
  const hold = (ids: number[]) =>
    setHeld((h) => ({
      strip,
      view,
      ids: new Set([...(h.strip === strip && h.view === view ? h.ids : []), ...ids]),
    }));

  // Suggested verdicts, over the whole Cull set rather than the strip,
  // recomputed as verdicts land: judging one frame of a burst re-plans the rest
  // of it. The bursts come from `duplicate_groups`, fetched whenever
  // suggestions are shown, so photos scanned since are taken into account.
  useEffect(() => {
    if (suggestOn) void loadCullGroups();
  }, [suggestOn, loadCullGroups]);
  // The frames as they stand apart from verdicts: their order and
  // focus, kept while only ratings, flags and labels move.
  const frames = useStable(allPhotos, sameFrames);
  const allMedian = useMemo(() => medianSharpness(frames), [frames]);
  const suggestions = useMemo(
    () => (suggestOn && cullGroups ? suggestCull(allPhotos, cullGroups, allMedian) : NO_SUGGESTIONS),
    [suggestOn, cullGroups, allPhotos, allMedian],
  );
  const suggested = useMemo(() => suggestionCounts(suggestions), [suggestions]);
  // Bursts are found through the search index. A frame not in it yet is judged
  // on its own, so every frame of an unindexed burst could be suggested as a
  // keeper — worth saying while it is the case.
  const [unindexed, setUnindexed] = useState(0);
  useEffect(() => {
    if (!suggestOn) return;
    api
      .searchIndexStatus()
      .then(setUnindexed)
      .catch(() => setUnindexed(0));
  }, [suggestOn]);

  // The frames this pass is stepping through, per the strip filter — optionally
  // ordered sharpest-first so a reviewer can triage the softest frames fast.
  // Sorted once per change of the frames, not per verdict: a verdict
  // changes which frames a strip keeps, never their order, and sorting 20,000
  // by date on every key dropped frames.
  const ordered = useMemo(() => {
    if (sort === "sharpness") {
      return [...frames].sort(
        (a, b) => (b.sharpness ?? -Infinity) - (a.sharpness ?? -Infinity) || a.id - b.id,
      );
    }
    // "Capture" is capture order, oldest first, whatever Browse is sorted by:
    // Browse defaults to newest first, so a wedding was culled
    // backwards, every burst in reverse. Undated frames go last.
    return [...frames].sort(
      (a, b) => (a.captureDate ?? "\uffff").localeCompare(b.captureDate ?? "\uffff") || a.id - b.id,
    );
  }, [frames, sort]);
  const set = useMemo(() => {
    const now = new Map(allPhotos.map((p) => [p.id, p]));
    const inOrder = ordered.map((p) => now.get(p.id) ?? p);
    const keep = (fits: (p: Photo) => boolean) => inOrder.filter((p) => fits(p) || heldIds.has(p.id));
    if (strip === "unrated") return keep(isUntouched);
    if (strip === "picks") return keep((p) => p.flag === 1);
    if (strip === "rejects") return keep((p) => p.flag === -1);
    if (strip === "suggested" && suggestOn) return keep((p) => suggestions.get(p.id)?.flag === -1);
    return inOrder;
  }, [allPhotos, ordered, strip, suggestOn, suggestions, heldIds]);

  // Median focus of the working set, for a *relative* soft-focus warning, and
  // the set's sorted scores, for the per-frame focus band (both relative — the
  // sharpness metric only means anything against its peer set).
  // Sorted once and shared, and kept across verdicts that leave the set's
  // frames as they were.
  const setFrames = useStable(set, sameFrames);
  const setScores = useMemo(() => sharpnessScores(setFrames), [setFrames]);
  const setMedian = useMemo(() => medianOfSorted(setScores), [setScores]);

  // Keep the cursor in range as the set changes (a reject can drain the queue).
  const clamped = Math.min(cursor, Math.max(0, set.length - 1));
  useEffect(() => {
    if (clamped !== cursor) setCursor(clamped);
  }, [clamped, cursor]);
  const current = set[clamped];
  // The frames either side, ready before a step reaches them.
  const near = (i: number) => set[i] && fileUrl(set[i]!.previewPath ?? set[i]!.thumbPath);
  usePreload([near(clamped + 1), near(clamped + 2), near(clamped - 1)]);
  // What the live regions say: the frame as the cursor reaches it, and the
  // last verdict (see the regions).
  const cursorName = useMemo(() => (current ? tileName(current) : ""), [current?.id]);
  const [said, setSaid] = useState("");

  // Resume where the last pass stopped, once the set has loaded. `restoring`
  // holds the index being restored to until the cursor has actually moved
  // there, so the save below cannot write the not-yet-moved position over
  // the session it is restoring.
  const restoring = useRef<number | null | "done">(saved ? null : "done");
  useEffect(() => {
    if (restoring.current !== null || !saved || set.length === 0) return;
    const index = resumeIndex(
      set.map((p) => p.id),
      saved,
    );
    restoring.current = index;
    setCursor(index);
    if (index > 0) {
      toast(t("cull.toast.resumed", { frame: index + 1, total: set.length }));
    }
  }, [saved, set]);

  // Save the position on every move. Synchronous localStorage underneath, so a
  // crash loses at most the frame in flight.
  useEffect(() => {
    if (restoring.current !== "done") {
      if (restoring.current === null || clamped !== restoring.current) return;
      restoring.current = "done";
    }
    if (!current) return;
    const st = useStore.getState();
    setCullSession({
      photoId: current.id,
      index: clamped,
      strip,
      view,
      scope: cullScopeKey(st.filter, st.searchLabel),
    });
  }, [current, clamped, strip, view, setCullSession]);

  // Close-ups of the current frame's faces. Fetched once the frame has
  // been on screen a moment: arrowing through a burst must not ask for every
  // frame on the way, and the first look at a frame cuts its portraits.
  const [closeUps, setCloseUps] = useState<{ id: number; faces: PhotoFace[] } | null>(null);
  const currentId = current?.id;
  const currentHasFaces = !!current && facesAnalysed(current) && (current.faceCount ?? 0) > 0;
  useEffect(() => {
    if (currentId === undefined || !currentHasFaces) return;
    let live = true;
    const timer = window.setTimeout(() => {
      api
        .photoFaces([currentId])
        .then((faces) => live && setCloseUps({ id: currentId, faces }))
        .catch(() => live && setCloseUps({ id: currentId, faces: [] }));
    }, 150);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [currentId, currentHasFaces]);
  const faces = closeUps && closeUps.id === currentId ? closeUps.faces : null;

  // Tally over the whole grid set, not the filtered strip.
  const tally = useMemo(() => {
    let picks = 0, rejects = 0, remaining = 0;
    for (const p of allPhotos) {
      if (p.flag === 1) picks++;
      else if (p.flag === -1) rejects++;
      if (isUntouched(p)) remaining++;
    }
    return { picks, rejects, remaining, total: allPhotos.length };
  }, [allPhotos]);
  const pickIds = useMemo(() => allPhotos.filter((p) => p.flag === 1).map((p) => p.id), [allPhotos]);
  const rejectIds = useMemo(() => allPhotos.filter((p) => p.flag === -1).map((p) => p.id), [allPhotos]);
  const binRejects = useStore((s) => s.binRejects);
  const openPhotoMenu = useStore((s) => s.openPhotoMenu);
  const scopeLabel = useStore((s) =>
    cullScopeLabel(s.filter, s.searchResults ? s.searchLabel : "", {
      shoot: s.shoots.find((x) => x.id === s.filter.shootId)?.name,
      collection: s.collections.find((x) => x.id === s.filter.collectionId)?.name,
      folder: s.libraryFolders?.folders.find((x) => x.id === s.filter.folderIds?.[0])?.path.split(/[\\/]/).pop(),
    }),
  );
  const exportSelection = useStore((s) => s.exportSelection);
  const saveAsCollection = useStore((s) => s.saveAsCollection);
  // Reviewing the picks happens here, on the Picks strip — it used to select
  // them and jump to Browse, and getting back meant finding your place again.
  // From that strip the same button exports them, the job Browse was for.
  const reviewPicks = () => {
    if (pickIds.length === 0) return;
    setStrip("picks");
    setCursor(0);
  };
  const exportPicks = () => {
    if (pickIds.length === 0) return;
    selectPhotos(pickIds);
    void exportSelection();
  };

  /** Whether a frame belongs in the current strip. */
  const inStrip = (p: Photo) =>
    strip === "unrated"
      ? isUntouched(p)
      : strip === "picks"
        ? p.flag === 1
        : strip === "rejects"
          ? p.flag === -1
          : strip === "suggested" && suggestOn
          ? isUntouched(p) && suggestions.get(p.id)?.flag === -1
          : true;
  /**
   * Moves the cursor to `i`, an index into the strip as it stands, optionally
   * switching view on the way. In the loupe, frames held after a verdict leave
   * as you move off them. The survey keeps them until you leave it, so tiles
   * don't reflow under the pointer.
   */
  const moveTo = (i: number, nextView: typeof view = view) => {
    const target = set[i]?.id;
    if ((view === "survey" && nextView === "survey") || heldIds.size === 0) {
      setCursor(i);
      return;
    }
    const leaving = new Set(
      set.filter((p) => heldIds.has(p.id) && p.id !== target && !inStrip(p)).map((p) => p.id),
    );
    setHeld({
      strip,
      view: nextView,
      ids: target !== undefined && heldIds.has(target) ? new Set([target]) : NO_IDS,
    });
    setCursor(releaseIndex(set.map((p) => p.id), i, leaving));
  };
  const go = (delta: number) => {
    if (set.length > 0) moveTo(stepIndex(clamped, delta, set.length));
  };
  /**
   * Auto-advance after a verdict. A verdict that takes the frame out of the
   * strip — any verdict at all in "To review" or "Suggested rejects" — already
   * brings the next frame under the cursor as the strip closes up, so stepping
   * on as well skipped a frame every time.
   */
  const advanceFrom = (p: Photo, patch: Partial<Photo>) => {
    if (inStrip({ ...p, ...patch })) go(1);
    // Held from an earlier verdict: let it go, so the strip closes up.
    else if (heldIds.has(p.id)) setHeld({ strip, view, ids: new Set([...heldIds].filter((id) => id !== p.id)) });
  };
  /** A verdict on one frame: onward when `advance`, otherwise it stays put. */
  const judge = (p: Photo, patch: Partial<Photo>, advance: boolean) => {
    setSaid(tileName({ ...p, ...patch }));
    void compareApply([p.id], patch);
    if (advance) advanceFrom(p, patch);
    else hold([p.id]);
  };
  /** The verdict panel's buttons. Clearing a value (a second click) never advances. */
  const verdict = (patch: { flag?: number; rating?: number; colorLabel?: number }) => {
    if (!current) return;
    judge(current, patch, autoAdvance && Object.values(patch).some((v) => v !== 0));
  };

  const toggleSuggest = () => {
    if (suggestOn && strip === "suggested") setStrip("all");
    setSuggestOn(!suggestOn);
  };
  /** Accepting one suggestion is just that verdict, made by the person. */
  const acceptSuggestion = (p: Photo) => {
    const s = suggestions.get(p.id);
    if (!s) return;
    judge(p, { flag: s.flag }, autoAdvance);
  };
  /** The rejects or the picks proposed, as separate actions: the only
   *  bulk action took both, so clearing the obvious rejects also picked every
   *  frame with "nothing wrong found". Kept marked as suggestions. */
  const acceptKind = async (flag: 1 | -1) => {
    const flags = [...suggestions].filter(([, s]) => s.flag === flag).map(([id, s]) => ({ id, flag: s.flag }));
    if (flags.length === 0) return;
    await acceptSuggestions(flags, undefined, true);
    toast(
      t(flag < 0 ? "cull.toast.acceptedRejects" : "cull.toast.acceptedPicks", {
        count: flags.length,
        key: key("undo-verdict"),
      }),
      "success",
    );
  };

  // Keyboard: the standard rate/flag/label bindings resolve in the "cull"
  // context; reject additionally advances (the fast "X = reject & next" pass).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      if (developShown(s) || s.compare || s.lightboxIndex !== null) return;
      // A menu, dialog or panel on top owns the keyboard: Escape on a
      // menu left Cull, and X under a confirm rejected the frame behind it.
      if (overlayOpen()) return;
      if (isTextEntry(e.target)) return;
      const action = resolveAction(s.keymap, "cull", e);
      if (!action) return;
      if (action === "undo-verdict") {
        e.preventDefault();
        void s.undoVerdict();
        return;
      }
      const cur = set[Math.min(cursor, set.length - 1)];
      // Several frames chosen: a verdict goes to all of them, as one undo step,
      // and nothing advances — there is no single next frame.
      if (chosen.size > 0 && /^(rate|flag|label)-/.test(action)) {
        const many = set.filter((p) => chosen.has(p.id));
        const ids = many.map((p) => p.id);
        const patch = action.startsWith("rate-")
          ? { rating: Number(action.slice(5)) }
          : action === "flag-pick"
            ? { flag: flagForAll(many, 1) }
            : action === "flag-reject"
              ? { flag: flagForAll(many, -1) }
              : action === "flag-none"
                ? { flag: 0 }
                : action === "label-none"
                  ? { colorLabel: 0 }
                  : { colorLabel: Number(action.slice(6)) };
        if (ids.length > 0) {
          void compareApply(ids, patch);
          hold(ids);
          // Done with: the next key goes to the ringed frame again. The
          // choice used to stay, so the next X un-rejected all six.
          setChosen(new Set());
        }
        e.preventDefault();
        return;
      }
      // Accepting goes to the chosen frames too.
      if (chosen.size > 0 && action === "accept-suggestion") {
        const flags = set
          .filter((p) => chosen.has(p.id) && suggestions.has(p.id))
          .map((p) => ({ id: p.id, flag: suggestions.get(p.id)!.flag }));
        if (flags.length > 0) {
          void acceptSuggestions(flags, undefined, true);
          hold(flags.map((f) => f.id));
        }
        setChosen(new Set());
        e.preventDefault();
        return;
      }
      if (action === "close-overlay" && chosen.size > 0) setChosen(new Set());
      else if (action === "close-overlay") setMode("browse");
      else if (action === "prev-photo") go(-1);
      else if (action === "next-photo") go(1);
      else if (action === "next-undecided") {
        // Cycle to the next frame still needing a verdict — pairs with
        // auto-advance for a fast second sweep over what's left.
        const start = Math.min(cursor, set.length - 1);
        for (let i = 1; i <= set.length; i++) {
          const j = (start + i) % set.length;
          if (isUntouched(set[j]!)) {
            moveTo(j);
            break;
          }
        }
      } else if (!cur) return;
      else if (action === "accept-suggestion") {
        if (!suggestions.has(cur.id)) return;
        acceptSuggestion(cur);
      } else if (action === "open-develop") void openDevelop(cur.id);
      else if (action === "copy-image") {
        if (textIsSelected()) return;
        void useStore.getState().copyImage(cur.id);
      } else if (action === "toggle-zoom") {
        if (view === "loupe") zoomRef.current?.toggle();
      } else if (action === "toggle-peaking") useStore.getState().togglePeaking("cull");
      else if (action === "cycle-grid") useStore.getState().cycleGrid("cull");
      else if (action.startsWith("rate-")) {
        judge(cur, { rating: Number(action.slice(5)) }, autoAdvance);
      } else if (action === "flag-pick") {
        const becomingPick = cur.flag !== 1;
        judge(cur, { flag: becomingPick ? 1 : 0 }, autoAdvance && becomingPick);
      } else if (action === "flag-reject") {
        // Symmetrical with pick, and it honours the toggle: reject used to
        // advance unconditionally, so turning auto-advance off still moved you,
        // and there was no way to take a reject back with the same key.
        const becomingReject = cur.flag !== -1;
        judge(cur, { flag: becomingReject ? -1 : 0 }, autoAdvance && becomingReject);
      } else if (action === "flag-none") judge(cur, { flag: 0 }, false);
      else if (action === "label-none") judge(cur, { colorLabel: 0 }, false);
      else if (action.startsWith("label-")) {
        judge(cur, { colorLabel: Number(action.slice(6)) }, autoAdvance);
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [set, cursor, compareApply, openDevelop, setMode, autoAdvance, view, suggestions, chosen, heldIds, strip]);

  // A choice made in one strip or view means nothing in another.
  useEffect(() => setChosen(new Set()), [strip, view]);

  // Survey measuring.
  const surveyRef = useRef<HTMLDivElement>(null);
  // Re-measured when the view changes: the survey's box only exists in Survey.
  const box = useContentBox(surveyRef, [view]);

  const flagColor = (f: number) => (f === 1 ? "var(--color-positive)" : f === -1 ? "var(--color-danger)" : "transparent");

  // Memoised because `surveyLayout` searches every column count from 1 to
  // `set.length` — on the "All" strip of a large library that is tens of
  // thousands of iterations, and it used to run on every render.
  const layout = useMemo(
    () => surveyLayout(set.length, box.width, box.height, SURVEY_GAP, 3 / 2, SURVEY_MIN_TILE),
    [set.length, box.width, box.height],
  );

  // Both the survey and the filmstrip render `set`, which on the "All" strip is
  // the whole library (up to `PAGE`, 20,000). Virtualised for the same reason
  // the browse grid is — the survey especially, since each tile carries an
  // image. Rows for the survey, columns for the strip.
  const surveyVirtualizer = useVirtualizer({
    count: Math.ceil(set.length / Math.max(1, layout.cols)),
    getScrollElement: () => surveyRef.current,
    // Gaps *between* rows, as `surveyLayout` counts them. Folding the gap into
    // each row's size added one after the last row too, so a set sized to fit
    // exactly still scrolled by a gap's height.
    estimateSize: () => layout.tileH,
    gap: SURVEY_GAP,
    overscan: 3,
  });
  // Tile height changes with the window and the set size; re-measure so the
  // row offsets stay right.
  useEffect(() => {
    surveyVirtualizer.measure();
  }, [layout.tileH, layout.cols, surveyVirtualizer]);

  const stripRef = useRef<HTMLDivElement>(null);
  // Focus follows the cursor while it is on a frame, so a magnifier
  // that tracks focus moves with the cull instead of staying put. Only then:
  // a cull driven from the keyboard with focus elsewhere keeps it there.
  useEffect(() => {
    const at = document.activeElement;
    if (!(at instanceof HTMLElement) || at.dataset.cullIndex === undefined) return;
    if (Number(at.dataset.cullIndex) === clamped) return;
    const frame = requestAnimationFrame(() => {
      const box = at.closest("[data-cull-frames]") ?? document;
      box.querySelector<HTMLElement>(`[data-cull-index="${clamped}"]`)?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [clamped]);
  const stripVirtualizer = useVirtualizer({
    horizontal: true,
    count: set.length,
    getScrollElement: () => stripRef.current,
    estimateSize: () => STRIP_THUMB_W + STRIP_GAP,
    overscan: 8,
  });

  // Keep the cursor in view in whichever surface is showing. The filmstrip did
  // this with `scrollIntoView` on a live ref, which a virtualised list has only
  // for frames already on screen; the survey never did it at all, so a keyboard
  // step could leave the ringed tile off-screen.
  useEffect(() => {
    // And when the strip appears: back from Survey, the current
    // frame could be off the strip altogether.
    if (view !== "loupe") return;
    stripVirtualizer.scrollToIndex(clamped, { align: "center" });
  }, [clamped, strip, stripVirtualizer, view]);
  useEffect(() => {
    if (view !== "survey" || layout.cols <= 0) return;
    surveyVirtualizer.scrollToIndex(Math.floor(clamped / layout.cols), { align: "auto" });
  }, [clamped, view, layout.cols, surveyVirtualizer]);

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-canvas-void">
      {/* Read out as they happen: an arrow, C or X did their work in
          silence. The frame when the cursor reaches it; the verdict when one
          is made, on the frame it was made on, which auto-advance has
          already left. */}
      <div className="sr-only" aria-live="polite" aria-atomic="true">
        {cursorName}
      </div>
      <div className="sr-only" aria-live="polite" aria-atomic="true">
        {said}
      </div>
      {/* Header */}
      <div
        className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line bg-panel px-4"
        data-tauri-drag-region="deep"
      >
        <div className="flex min-w-0 items-center gap-2.5">
          <Icon name="cull" size={17} className="shrink-0 text-accent" />
          <span className="text-body font-semibold">{t("cull.header.title")}</span>
          {/* What is being culled, and how many: it was whatever Browse
              last showed, often the whole library, and nothing said so. */}
          <button
            onClick={() => useStore.getState().setMode("browse")}
            className="min-w-0 truncate rounded px-1 text-caption text-muted hover:bg-hover hover:text-ink"
            title={t("cull.scope.change")}
          >
            {t("cull.scope.label", { scope: scopeLabel, count: allPhotos.length })}
          </button>
          {current && <span className="truncate text-caption text-faint">· {current.filename}</span>}
        </div>
        <div className="flex-1" />
        {/* Below 1280px the secondary buttons drop to their icon (and count):
            with both catch-up passes pending, the full labels overran a 940px
            window and squeezed Loupe/Survey to nothing. Each keeps its name as
            a tooltip and an accessible label. */}
        <div className="seg-group shrink-0">
          {(["loupe", "survey"] as const).map((v) => (
            <button aria-pressed={view === v}
              key={v}
              onClick={() => {
                moveTo(clamped, v);
                setView(v);
              }}
              className={`px-3 py-1 text-caption ${view === v ? "seg-on" : "text-muted hover:bg-hover"}`}
            >
              {t(v === "loupe" ? "cull.view.loupe" : "cull.view.survey")}
            </button>
          ))}
        </div>
        {(scoring || (unscored != null && unscored > 0)) && (
          <button
            onClick={startScoring}
            disabled={!!scoring}
            className="btn-solid shrink-0"
            aria-label={t("cull.header.scoreFrames")}
            title={t("cull.header.scoreTip")}
          >
            <Icon name="loupe" size={14} />
            <span className="max-xl:hidden">
              {scoring ? t("cull.header.scoring") : t("cull.header.scoreFrames")}
            </span>
            {scoring
              ? t("cull.header.passProgress", { current: scoring.current, total: scoring.total })
              : unscored != null && formatNumber(unscored)}
          </button>
        )}
        {(analysing || (unanalysed != null && unanalysed > 0)) && (
          <button
            onClick={() => void startAnalysing()}
            disabled={!!analysing}
            className="btn-solid shrink-0"
            aria-label={t("cull.header.analyseFaces")}
            title={
              facesError
                ? t("cull.header.analyseFailedTip")
                : t("cull.header.analyseTip")
            }
          >
            <Icon name="eye" size={14} />
            {analysing ? (
              <>
                <span className="max-xl:hidden">{t("cull.header.analysing")}</span>{" "}
                {t("cull.header.passProgress", { current: analysing.current, total: analysing.total })}
              </>
            ) : facesError ? (
              t("cull.header.analyseFailed")
            ) : (
              <>
                <span className="max-xl:hidden">{t("cull.header.analyseFaces")}</span>{" "}
                {unanalysed != null && formatNumber(unanalysed)}
              </>
            )}
          </button>
        )}
        <button
          onClick={() => void reviewDuplicates()}
          disabled={compareBusy || allPhotos.length === 0}
          className="btn-solid shrink-0"
          aria-label={t("cull.header.duplicates")}
          title={t("cull.header.duplicatesTip")}
        >
          <Icon name="copy" size={14} />
          <span className="max-xl:hidden">{compareBusy ? t("cull.header.findingDuplicates") : t("cull.header.duplicates")}</span>
        </button>
        <button
          onClick={toggleSuggest}
          data-tour="cull-suggest"
          disabled={allPhotos.length === 0}
          className={`btn-solid shrink-0 ${suggestOn ? "seg-on" : ""}`}
          aria-pressed={suggestOn}
          title={t("cull.header.suggestTip")}
        >
          <Icon name="flag" size={14} />
          {t("cull.header.suggest")}
        </button>
        <button
          onClick={() => setAutoAdvance(!autoAdvance)}
          // A toggle: pressed reads as the active-segment state (a tone step and
          // a white underline), not a green outline of its own invention.
          className={`btn-solid shrink-0 ${autoAdvance ? "seg-on" : ""}`}
          aria-pressed={autoAdvance}
          aria-label={t("cull.header.autoAdvance")}
          title={t("cull.header.autoAdvanceTip")}
        >
          {/* Pointing on, to the next frame: the bare glyph is the filter
              rail's "fold away to the left". */}
          <Icon name="chevrons" size={14} className="rotate-180" />
          <span className="max-xl:hidden">
            {t(autoAdvance ? "cull.header.autoAdvanceOn" : "cull.header.autoAdvanceOff")}
          </span>
        </button>
      </div>

      {/* Progress + tally */}
      <div className="flex h-[38px] shrink-0 items-center gap-4 border-b border-line bg-panel/60 px-4">
        <span className="section-label px-0 pb-0">{t("cull.tally.label")}</span>
        <div className="flex h-1.5 flex-1 overflow-hidden rounded-full bg-line">
          <span style={{ width: `${(tally.picks / Math.max(1, tally.total)) * 100}%`, background: "var(--color-positive)" }} />
          <span style={{ width: `${(tally.rejects / Math.max(1, tally.total)) * 100}%`, background: "var(--color-danger)" }} />
        </div>
        <span className="text-caption text-positive">
          {tx("cull.tally.picks", { count: <span className="tabular-nums">{formatNumber(tally.picks)}</span> }, tally.picks)}
        </span>
        <span className="text-caption text-danger">
          {tx("cull.tally.rejects", { count: <span className="tabular-nums">{formatNumber(tally.rejects)}</span> }, tally.rejects)}
        </span>
        <span className="text-caption text-faint">
          {tx("cull.tally.toReview", { count: <span className="tabular-nums">{formatNumber(tally.remaining)}</span> })}
        </span>
        {/* The end of a cull: rejects were only ever flagged, so the disk
            never got any space back. Recoverable from the Recycle Bin, and
            confirmed first — see `store.binRejects`. */}
        {tally.rejects > 0 && (
          <button
            onClick={() => void binRejects(rejectIds)}
            className="btn-ghost ml-1 text-caption"
            title={t("cull.tally.binTip", { trash: trashName() })}
          >
            {t("cull.tally.bin", { count: tally.rejects })}
          </button>
        )}
        {chosen.size > 0 && (
          <span
            className="text-caption text-accent"
            title={t("cull.tally.chosenTip", { modifier: IS_MAC ? "⌘" : "Ctrl" })}
          >
            {tx("cull.tally.chosen", { count: <span className="tabular-nums">{formatNumber(chosen.size)}</span> })}
          </span>
        )}
        {tally.picks > 0 && strip === "picks" && (
          <button
            onClick={() =>
              void prompt({
                title: t("cull.tally.saveCollectionTitle"),
                placeholder: t("cull.tally.collectionPlaceholder"),
                confirmLabel: t("common.save"),
              }).then((name) => name && void saveAsCollection(name, pickIds))
            }
            className="btn-ghost ml-1 text-caption"
            title={t("cull.tally.saveCollectionTip")}
          >
            {t("cull.tally.saveCollection")}
          </button>
        )}
        {tally.picks > 0 &&
          (strip === "picks" ? (
            <button
              onClick={exportPicks}
              className="btn-solid ml-1 text-caption"
              title={t("cull.tally.exportPicksTip")}
            >
              {t("cull.tally.exportPicks", { count: tally.picks })}
            </button>
          ) : (
            <button
              onClick={reviewPicks}
              className="btn-solid ml-1 text-caption"
              title={t("cull.tally.reviewPicksTip")}
            >
              {t("cull.tally.reviewPicks", { count: tally.picks })}
              <Icon name="chevron" size={13} className="-rotate-90" />
            </button>
          ))}
      </div>

      {/* Suggested cull: what it proposes, and the one way to make it
          real in bulk. Individual suggestions are accepted frame by frame. */}
      {suggestOn && (
        <div className="flex h-[36px] shrink-0 items-center gap-3 border-b border-line bg-panel/40 px-4 text-caption">
          <span className="section-label px-0 pb-0">{t("cull.suggestBar.label")}</span>
          {cullGroups === null ? (
            <span className="text-faint">{t("cull.suggestBar.findingBursts")}</span>
          ) : suggestions.size === 0 ? (
            <span className="min-w-0 truncate text-faint" title={t("cull.suggestBar.nothing")}>
              {t("cull.suggestBar.nothing")}
            </span>
          ) : (
            <>
              <span className="text-positive">
                {tx("cull.tally.picks", { count: <span className="tabular-nums">{formatNumber(suggested.picks)}</span> }, suggested.picks)}
              </span>
              <span className="text-danger">
                {tx("cull.tally.rejects", { count: <span className="tabular-nums">{formatNumber(suggested.rejects)}</span> }, suggested.rejects)}
              </span>
              {/* Cut short at the minimum window, so whole on hover. */}
              <span className="min-w-0 truncate text-faint" title={t("cull.suggestBar.footnote")}>
                {t("cull.suggestBar.footnote")}
              </span>
            </>
          )}
          {unindexed > 0 && (
            <span
              className="shrink-0 text-warning"
              title={t("cull.suggestBar.unindexedTip")}
            >
              {t("cull.suggestBar.unindexed", { count: unindexed })}
            </span>
          )}
          <div className="flex-1" />
          {suggested.rejects > 0 && strip !== "suggested" && (
            <button
              onClick={() => {
                setStrip("suggested");
                setCursor(0);
              }}
              className="btn-ghost shrink-0 text-caption"
              title={t("cull.suggestBar.checkRejectsTip")}
            >
              {t("cull.suggestBar.checkRejects")}
            </button>
          )}
          {suggested.rejects > 0 && (
            <button onClick={() => void acceptKind(-1)} className="btn-solid shrink-0 text-caption">
              {t("cull.suggestBar.acceptRejects", { count: suggested.rejects })}
            </button>
          )}
          {/* Not on the rejects strip: it is for checking rejects. */}
          {suggested.picks > 0 && strip !== "suggested" && (
            <button onClick={() => void acceptKind(1)} className="btn-solid shrink-0 text-caption">
              {t("cull.suggestBar.acceptPicks", { count: suggested.picks })}
            </button>
          )}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        {/* Stage */}
        {view === "loupe" ? (
          <div
            className="relative flex min-w-0 flex-1 items-center justify-center overflow-hidden"
            onMouseMove={nudgeLegend}
          >
            {current ? (
              <ZoomableImage
                src={fileUrl(current.previewPath ?? current.thumbPath)}
                alt={current.filename}
                resetKey={current.id}
                keepView={keepZoom}
                controlRef={zoomRef}
                onZoomChange={setZoomed}
                fullSrc={fullUrl(current.id)}
                onSourceChange={setZoomSource}
                peaking={aids.peaking ? { color: peakColor, sensitivity: peakSensitivity } : null}
                onPeakingStatus={setPeakStatus}
                grid={aids.grid}
              />
            ) : (
              <div className="text-title text-faint">{t("cull.loupe.empty")}</div>
            )}

            {current && zoomed && (
              <div className="loupe-chip absolute bottom-4 left-4 flex items-center gap-2 px-2 py-1 text-label text-white/85">
                <span className="pointer-events-none">{zoomSourceLabel(zoomSource)}</span>
                {/* Stay zoomed from frame to frame: checking focus
                    through a burst meant Z and a pan on every frame. */}
                <label className="flex items-center gap-1 text-white/70" title={t("cull.loupe.keepZoomTitle")}>
                  <input type="checkbox" checked={keepZoom} onChange={(e) => setKeepZoom(e.target.checked)} />
                  {t("cull.loupe.keepZoom")}
                </label>
              </div>
            )}

            {current && (
              <>
                {/* Frame state, top-left. On a scrim, like the survey's chips:
                    bare text over the photo vanished on anything bright. */}
                <div className="loupe-chip pointer-events-none absolute left-4 top-4 flex flex-col gap-2 px-2.5 py-2">
                  <div className="text-xl leading-none text-accent">
                    {"★".repeat(current.rating)}
                    <span className="text-white/35">{"★".repeat(5 - current.rating)}</span>
                  </div>
                  {current.flag !== 0 && (
                    <div className="flex items-center gap-2 text-caption font-semibold text-ink">
                      <Icon
                        name={current.flag > 0 ? "flag" : "flag-off"}
                        size={15}
                        className={current.flag > 0 ? "text-positive" : "text-danger"}
                      />
                      {current.flag > 0 ? t("cull.loupe.flagPick") : t("cull.loupe.flagReject")}
                      {/* Said apart from the person's own. */}
                      {current.flagSuggested && (
                        <span className="font-normal text-white/60">{t("cull.loupe.acceptedSuggestion")}</span>
                      )}
                    </div>
                  )}
                  {(() => {
                    const sug = suggestions.get(current.id);
                    if (!sug) return null;
                    return (
                      <div className="flex items-center gap-2 text-caption" style={{ color: suggestColor(sug) }}>
                        <Icon name={sug.flag > 0 ? "flag" : "flag-off"} size={15} className="opacity-60" />
                        <span>
                          {tx("cull.suggest.withReason", {
                            verdict: <span className="font-semibold opacity-80">{suggestVerdict(sug)}</span>,
                            reason: <span className="text-white/70">{SUGGEST_REASON[sug.reason]}</span>,
                          })}
                        </span>
                      </div>
                    );
                  })()}
                  {(() => {
                    // Which frame is the better one, and the way to it.
                    const keeperId = suggestions.get(current.id)?.keeperId;
                    if (keeperId == null) return null;
                    const at = set.findIndex((p) => p.id === keeperId);
                    const keeper = at >= 0 ? set[at]! : useStore.getState().photos.find((p) => p.id === keeperId);
                    if (!keeper) return null;
                    return at >= 0 ? (
                      <button
                        className="pointer-events-auto -mt-1 self-start text-label text-white/75 underline hover:text-white"
                        onClick={() => moveTo(at)}
                      >
                        {t("cull.suggest.keeper", { name: keeper.filename })}
                      </button>
                    ) : (
                      <span className="-mt-1 text-label text-white/60">{t("cull.suggest.keeper", { name: keeper.filename })}</span>
                    );
                  })()}
                  {(() => {
                    const fb = focusBand(current, setScores, setMedian);
                    if (!fb) return null;
                    const soft = fb.soft;
                    return (
                      <div
                        className="flex items-center gap-2 text-caption font-semibold"
                        style={{ color: soft ? "var(--color-warning)" : "rgb(255 255 255 / 0.85)" }}
                        title={t("cull.loupe.sharperThan", { percentile: fb.percentile })}
                      >
                        <FocusMeter level={fb.level} />
                        {fb.label}
                      </div>
                    );
                  })()}
                  {facesAnalysed(current) && (current.faceCount ?? 0) > 0 && (
                    <div className="flex items-center gap-2 text-caption font-semibold text-white/85">
                      <Icon name="user" size={15} />
                      {t("cull.faces", { count: current.faceCount ?? 0 })}
                    </div>
                  )}
                  {eyesClosed(current) && (
                    <div className="text-caption font-semibold">
                      <EyesBadge onClear={() => void clearEyes(current.id)} />
                    </div>
                  )}
                </div>
                <div className="loupe-chip pointer-events-none absolute right-4 top-4 px-2 py-1 text-caption tabular-nums text-white/85">
                  {t("common.progress", { current: clamped + 1, total: set.length })}
                </div>
                <ViewAidsControl
                  place="cull"
                  status={aids.peaking && peakStatus === "failed" ? "failed" : null}
                  className="absolute right-4 top-14 z-10"
                />
                {/* Keyboard legend */}
                <div
                  className={`pointer-events-none absolute inset-x-0 bottom-4 flex flex-wrap justify-center gap-2.5 px-4 transition-opacity duration-500 ${
                    legendShown ? "opacity-100" : "opacity-0"
                  }`}
                  aria-hidden={!legendShown}
                >
                  {/* Read from the live keymap — the legend used to spell out
                      the defaults and so lied to anyone who rebound a key. */}
                  <span className="key-pill">
                    {tx("cull.legend.rate", {
                      from: <span className="kbd">{key("rate-1")}</span>,
                      to: <span className="kbd">{key("rate-5")}</span>,
                    })}
                  </span>
                  <span className="key-pill">
                    {tx("cull.legend.pick", { key: <span className="kbd">{key("flag-pick")}</span> })}
                  </span>
                  <span className="key-pill">
                    {tx(autoAdvance ? "cull.legend.rejectAndNext" : "cull.legend.reject", {
                      key: <span className="kbd">{key("flag-reject")}</span>,
                    })}
                  </span>
                  <span className="key-pill">
                    {tx("cull.legend.move", {
                      prev: <span className="kbd">{key("prev-photo")}</span>,
                      next: <span className="kbd">{key("next-photo")}</span>,
                    })}
                  </span>
                  <span className="key-pill">
                    {tx("cull.legend.nextToReview", { key: <span className="kbd">{key("next-undecided")}</span> })}
                  </span>
                  {suggestions.has(current.id) && (
                    <span className="key-pill">
                      {tx("cull.legend.acceptSuggestion", {
                        key: <span className="kbd">{key("accept-suggestion")}</span>,
                      })}
                    </span>
                  )}
                </div>
              </>
            )}
          </div>
        ) : (
          <div ref={surveyRef} data-cull-frames className="min-w-0 flex-1 overflow-y-auto p-3">
            <div
              style={{
                height: surveyVirtualizer.getTotalSize(),
                width: "100%",
                position: "relative",
              }}
            >
              {surveyVirtualizer.getVirtualItems().map((row) => {
                const start = row.index * layout.cols;
                return (
                  <div
                    key={row.key}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      height: layout.tileH,
                      transform: `translateY(${row.start}px)`,
                      display: "flex",
                      justifyContent: "center",
                      gap: SURVEY_GAP,
                    }}
                  >
                    {set.slice(start, start + layout.cols).map((p, ci) => {
                      const i = start + ci;
                      return (
                      <div
                        key={p.id}
                        role="button"
                        tabIndex={i === clamped ? 0 : -1}
                        aria-label={tileName(p)}
                        aria-current={i === clamped ? true : undefined}
                        data-cull-index={i}
                        onClick={(e) => {
                          setChosen(
                            clickSelect(
                              chosen,
                              set.map((x) => x.id),
                              clamped,
                              i,
                              { toggle: e.ctrlKey || e.metaKey, range: e.shiftKey },
                            ),
                          );
                          moveTo(i);
                        }}
                        // Into the loupe on this frame, not out to Develop: a
                        // closer look is still part of the cull.
                        onDoubleClick={() => {
                          moveTo(i, "loupe");
                          setView("loupe");
                        }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          moveTo(i);
                          openPhotoMenu(e.clientX, e.clientY, [p.id], "cull", set);
                        }}
                        style={{ width: layout.tileW, height: layout.tileH }}
                        className={`group relative shrink-0 overflow-hidden rounded-sm bg-raised ring-offset-1 ring-offset-black ${
                          i === clamped
                            ? "ring-2 ring-accent"
                            : chosen.has(p.id)
                              ? "ring-2 ring-accent/60"
                              : "hover:ring-1 hover:ring-line"
                        }`}
                      >
                        <img
                          src={fileUrl(
                            layout.tileW * (window.devicePixelRatio || 1) > THUMB_EDGE
                              ? (p.previewPath ?? p.thumbPath)
                              : (p.thumbPath ?? p.previewPath),
                          )}
                          alt={p.filename}
                          loading="lazy"
                          draggable={false}
                          // A reject dims, but not its badge with it.
                          className={`h-full w-full object-contain ${p.flag === -1 ? "opacity-45" : ""}`}
                        />
                        {p.flag !== 0 && (
                          <span className="absolute left-1.5 top-1.5 inline-flex items-center gap-1 rounded-sm bg-chip-hover px-1 text-label text-ink backdrop-blur">
                            <Icon
                              name={p.flag > 0 ? "flag" : "flag-off"}
                              size={11}
                              className={p.flag > 0 ? "text-positive" : "text-danger"}
                            />
                            {p.flag > 0 ? t("cull.survey.pick") : t("cull.survey.reject")}
                          </span>
                        )}
                        {(() => {
                          const sug = suggestions.get(p.id);
                          return sug ? (
                            <span
                              className="absolute left-1.5 top-1.5 rounded-sm border border-dashed bg-chip px-1 text-label backdrop-blur"
                              style={{ color: suggestColor(sug), borderColor: suggestColor(sug) }}
                              title={suggestLabel(sug)}
                            >
                              {sug.flag > 0 ? t("cull.survey.suggestPick") : t("cull.survey.suggestReject")}
                            </span>
                          ) : null;
                        })()}
                        {(() => {
                          const fb = focusBand(p, setScores, setMedian);
                          return fb ? (
                            <span
                              className="absolute bottom-1.5 left-1.5 rounded-sm bg-chip px-1 py-[3px] backdrop-blur"
                              title={t("cull.survey.focusTip", { label: fb.label, percentile: fb.percentile })}
                            >
                              <FocusMeter level={fb.level} size="sm" onDark />
                            </span>
                          ) : null;
                        })()}
                        <div className="absolute right-1.5 top-1.5 flex items-center gap-1">
                          {facesAnalysed(p) && (p.faceCount ?? 0) > 0 && (
                            <FaceCountBadge count={p.faceCount!} size="sm" />
                          )}
                          {eyesClosed(p) && <EyesBadge size="sm" />}
                        </div>
                      </div>
                      );
                    })}
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Verdict panel */}
        <aside data-tour="cull-verdict" className="flex min-h-0 w-[var(--panel-w)] shrink-0 flex-col border-l border-line bg-panel">
          {current ? (
            <>
              {/* Scrolls on a short window now that Faces can make it tall;
                  Develop stays pinned below. */}
              <div className="min-h-0 flex-1 overflow-y-auto">
                <div className="border-b border-line/60 px-4 py-4">
                  <div className="section-label px-0 pb-2.5">{t("cull.panel.verdict")}</div>
                  <div className="mb-3 flex gap-2">
                    {/* On is the seg-on fill and underline, not a hue alone. */}
                    <button
                      onClick={() => verdict({ flag: current.flag === 1 ? 0 : 1 })}
                      aria-pressed={current.flag === 1}
                      className={`btn-solid flex-1 justify-center ${current.flag === 1 ? "seg-on" : ""}`}
                      style={{ color: "var(--color-positive)" }}
                    >
                      <Icon name="flag" size={15} />
                      {t("cull.panel.pick")}
                    </button>
                    <button
                      onClick={() => verdict({ flag: current.flag === -1 ? 0 : -1 })}
                      aria-pressed={current.flag === -1}
                      className={`btn-solid flex-1 justify-center ${current.flag === -1 ? "seg-on" : ""}`}
                      style={{ color: "var(--color-danger-ink)" }}
                    >
                      <Icon name="flag-off" size={15} />
                      {t("cull.panel.reject")}
                    </button>
                  </div>
                  {(() => {
                    const sug = suggestions.get(current.id);
                    if (!sug) return null;
                    return (
                      <div
                        className="mb-3 flex items-center gap-2 rounded-sm border border-dashed px-2.5 py-2 text-caption"
                        style={{ borderColor: suggestColor(sug) }}
                      >
                        <span className="min-w-0 flex-1">
                          {tx("cull.suggest.withReason", {
                            verdict: <span style={{ color: suggestColor(sug) }}>{suggestVerdict(sug)}</span>,
                            reason: <span className="text-faint">{SUGGEST_REASON[sug.reason]}</span>,
                          })}
                        </span>
                        <button
                          onClick={() => acceptSuggestion(current)}
                          className="btn-ghost shrink-0 text-caption"
                          title={t("cull.panel.acceptTip", { key: key("accept-suggestion") })}
                        >
                          {t("cull.panel.accept")}
                        </button>
                      </div>
                    );
                  })()}
                  <div className="flex items-center justify-center gap-1 text-2xl">
                    {[1, 2, 3, 4, 5].map((n) => (
                      <button
                        key={n}
                        onClick={() => verdict({ rating: current.rating === n ? 0 : n })}
                        aria-label={t("library.photoMenu.rate", { rating: n })}
                        aria-pressed={current.rating === n}
                        className={`leading-none ${current.rating >= n ? "text-accent" : "text-faint hover:text-muted"}`}
                      >
                        ★
                      </button>
                    ))}
                  </div>
                </div>
                <div className="border-b border-line/60 px-4 py-4">
                  <div className="section-label px-0 pb-2.5">{t("cull.panel.colour")}</div>
                  <div className="flex items-center gap-2.5">
                    {COLOR_LABELS.map((c) => (
                      <button
                        key={c.value}
                        onClick={() => verdict({ colorLabel: current.colorLabel === c.value ? 0 : c.value })}
                        aria-label={t(c.nameKey)}
                        aria-pressed={current.colorLabel === c.value}
                        className={`h-5 w-5 rounded-full border transition-transform hover:scale-110 ${
                          current.colorLabel === c.value ? "border-ink ring-2 ring-accent" : "border-black/30"
                        }`}
                        style={{ backgroundColor: c.hex }}
                        title={t(c.nameKey)}
                      />
                    ))}
                  </div>
                </div>
                {currentHasFaces && faces && faces.length > 0 && (
                  <div className="border-b border-line/60 px-4 py-4">
                    <div className="section-label px-0 pb-2.5">{t("cull.panel.faces")}</div>
                    <div className="grid grid-cols-3 gap-2">
                      {faces.slice(0, MAX_CLOSE_UPS).map((f) => {
                        const closed = faceEyesClosed(f, current.eyesOverride);
                        const name = f.personName ?? t("cull.panel.unnamed");
                        return (
                          <div key={f.faceId} className="min-w-0" title={closed ? t("cull.panel.faceEyesClosed", { name }) : name}>
                            <div className="relative overflow-hidden rounded-sm">
                              <FaceCrop
                                src={fileUrl(current.thumbPath ?? current.previewPath)}
                                box={f.boxFrac}
                                portrait={f.portraitPath}
                              />
                              {closed && (
                                <span className="absolute right-1 top-1 grid h-4 w-4 place-items-center rounded-sm bg-chip text-warning">
                                  <Icon name="eye-off" size={11} />
                                </span>
                              )}
                            </div>
                            <div className={`truncate pt-1 text-label ${f.personName ? "text-muted" : "text-faint"}`}>
                              {name}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                    {faces.length > MAX_CLOSE_UPS && (
                      <div className="pt-1.5 text-caption text-faint">
                        {t("cull.panel.smallerFaces", { count: faces.length - MAX_CLOSE_UPS })}
                      </div>
                    )}
                  </div>
                )}
                <div className="px-4 py-4 text-caption">
                  <div className="flex justify-between py-0.5"><span className="text-faint">{t("cull.panel.exposure")}</span><span className="text-muted">{exposureLine(current) || "—"}</span></div>
                  <div className="flex justify-between py-0.5"><span className="text-faint">{t("cull.panel.camera")}</span><span className="truncate text-muted">{current.camera ?? "—"}</span></div>
                </div>
              </div>
              <div className="flex shrink-0 justify-end border-t border-line px-4 py-3.5">
                <button onClick={() => void openDevelop(current.id)} className="btn-accent">
                  <Icon name="adjust" size={15} />
                  {t("cull.panel.develop")}
                </button>
              </div>
            </>
          ) : (
            <div className="p-4 text-caption text-faint">{t("cull.panel.noFrame")}</div>
          )}
        </aside>
      </div>

      {/* Filmstrip — loupe only. In Survey it just repeated the grid above
          it; the Sort and Show controls stay either way. */}
      {/* The strip keeps room for three frames: at the minimum window
          the Sort and Show controls left it one or two. When both don't fit,
          the controls take a row of their own under it. */}
      <div className="flex shrink-0 flex-wrap items-center gap-x-2 border-t border-line bg-panel px-4 py-2">
        <div
          ref={stripRef}
          data-cull-frames
          className={`h-[84px] min-w-[336px] flex-1 items-center overflow-x-auto overflow-y-hidden ${
            view === "loupe" ? "flex" : "hidden"
          }`}
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
              const p = set[v.index];
              if (!p) return null;
              const i = v.index;
              return (
                <button
                  key={p.id}
                  tabIndex={i === clamped ? 0 : -1}
                  aria-label={tileName(p)}
                  aria-current={i === clamped ? true : undefined}
                  data-cull-index={i}
                  onClick={() => moveTo(i)}
                  onDoubleClick={() => void openDevelop(p.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    moveTo(i);
                    openPhotoMenu(e.clientX, e.clientY, [p.id], "cull", set);
                  }}
                  title={p.filename}
                  style={{
                    position: "absolute",
                    left: v.start,
                    top: 0,
                    width: STRIP_THUMB_W,
                    height: STRIP_THUMB_H,
                  }}
                  className={`relative overflow-hidden rounded-sm border-2 ${
                    i === clamped ? "border-accent" : "border-transparent opacity-70 transition-opacity hover:opacity-100"
                  } ${p.flag === -1 ? "opacity-40" : ""}`}
                >
                  {p.thumbPath ? (
                    <img src={fileUrl(p.thumbPath)} alt="" loading="lazy" draggable={false} className="h-full w-full object-cover" />
                  ) : (
                    <div className="grid h-full w-full place-items-center bg-raised text-faint">?</div>
                  )}
                  {p.flag !== 0 && (
                    <span className="absolute inset-x-0 bottom-0 h-1" style={{ background: flagColor(p.flag) }} />
                  )}
                  {(() => {
                    const sug = suggestions.get(p.id);
                    return sug ? (
                      // A suggested pick is a solid bar, a suggested reject a
                      // broken one: told apart without the hue.
                      <span
                        className="absolute inset-x-0 bottom-0 h-1"
                        style={{
                          background:
                            sug.flag > 0
                              ? suggestColor(sug)
                              : `repeating-linear-gradient(90deg, ${suggestColor(sug)} 0 4px, transparent 4px 7px)`,
                        }}
                      />
                    ) : null;
                  })()}
                  <div className="absolute right-0.5 top-0.5 flex flex-col gap-0.5">
                    {isSoft(p, setMedian) && (
                      <span className="grid h-3.5 w-3.5 place-items-center rounded-sm bg-chip text-warning">
                        <Icon name="warn" size={9} />
                      </span>
                    )}
                    {eyesClosed(p) && <EyesBadge size="xs" />}
                  </div>
                </button>
              );
            })}
          </div>
        </div>
        {/* Label beside its control, both centred on one line: stacked
            labels over segments of different widths never lined up. */}
        <div className="ml-auto flex shrink-0 items-center gap-4 pl-2">
          <div className="flex items-center gap-2">
            <span className="section-label px-0 pb-0">{t("cull.filmstrip.sort")}</span>
            <div className="seg-group">
              {([
                { id: "capture", label: "cull.filmstrip.sortCapture" },
                { id: "sharpness", label: "cull.filmstrip.sortSharpest" },
              ] as const satisfies readonly { id: string; label: Key }[]).map((s) => (
                <button aria-pressed={sort === s.id}
                  key={s.id}
                  onClick={() => setSort(s.id)}
                  title={t(s.id === "sharpness" ? "cull.filmstrip.sortSharpestTip" : "cull.filmstrip.sortCaptureTip")}
                  className={`px-2.5 py-1 text-caption ${sort === s.id ? "seg-on" : "text-muted hover:bg-hover"}`}
                >
                  {t(s.label)}
                </button>
              ))}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <span className="section-label px-0 pb-0">{t("cull.filmstrip.show")}</span>
            <div className="seg-group">
              {STRIPS.filter((s) => s.id !== "suggested" || suggestOn).map((s) => (
                <button aria-pressed={strip === s.id}
                  key={s.id}
                  onClick={() => setStrip(s.id)}
                  title={
                    s.id === "unrated"
                      ? t("cull.filmstrip.toReviewTip")
                      : undefined
                  }
                  className={`px-2.5 py-1 text-caption ${strip === s.id ? "seg-on" : "text-muted hover:bg-hover"}`}
                >
                  {t(s.label)}
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
