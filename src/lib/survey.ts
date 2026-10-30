/**
 * Compare / survey maths.
 *
 * The compare view judges a handful of photos side by side — a burst, a
 * duplicate group, an arbitrary selection — so the two things it has to get
 * right are *how big each frame can be* and *what "keep this one" does to the
 * rest. Both are pure functions here rather than inline in the component, so
 * they can be tested without a DOM and without a library.
 */
import type { Photo } from "../types";
import { t, type Key } from "../i18n";

export interface SurveyLayout {
  cols: number;
  rows: number;
  /** Tile box in CSS pixels; the photo is letterboxed inside it. */
  tileW: number;
  tileH: number;
}

/**
 * The column count that gives the largest tiles for `count` frames in a
 * `width × height` box.
 *
 * Every arrangement from 1 to `count` columns is tried and the one whose tile
 * box is biggest wins, because the best split depends on the *viewport's* aspect
 * as much as on the number of photos: four frames go 2×2 on a squarish window
 * but 4×1 on a wide short one, and guessing with `ceil(sqrt(n))` gets both
 * wrong. Ties go to the fewer columns (the flatter grid), which keeps 2 photos
 * side by side rather than stacked on a typical window.
 *
 * Tiles are uniform — an honest side-by-side comparison needs every frame at the
 * same scale, so a portrait shot in the set does not get judged smaller than the
 * landscape one next to it.
 */
export function surveyLayout(
  count: number,
  width: number,
  height: number,
  gap = 8,
  aspect = 3 / 2,
  minTile = 0,
): SurveyLayout {
  if (count <= 0 || width <= 0 || height <= 0) {
    return { cols: 1, rows: 1, tileW: 0, tileH: 0 };
  }

  let best: SurveyLayout = { cols: 1, rows: count, tileW: 0, tileH: 0 };
  for (let cols = 1; cols <= count; cols++) {
    const rows = Math.ceil(count / cols);
    const boxW = (width - gap * (cols - 1)) / cols;
    const boxH = (height - gap * (rows - 1)) / rows;
    if (boxW <= 0 || boxH <= 0) continue;
    // Fit the nominal frame inside the cell; the smaller dimension decides.
    // Whole pixels: fractional widths could total a hair over the row and wrap
    // its last tile onto a line of its own.
    const tileW = Math.floor(Math.min(boxW, boxH * aspect));
    const tileH = tileW / aspect;
    if (tileW > best.tileW) best = { cols, rows, tileW, tileH };
  }

  // A 40-frame burst fitted to the window would give postage stamps you cannot
  // judge anything from. Past `minTile` the view stops shrinking and starts
  // scrolling instead — the rows returned may then exceed the box height.
  if (minTile > 0 && best.tileW < minTile) {
    const cols = Math.max(1, Math.floor((width + gap) / (minTile + gap)));
    const tileW = Math.floor((width - gap * (cols - 1)) / cols);
    best = { cols, rows: Math.ceil(count / cols), tileW, tileH: tileW / aspect };
  }
  return best;
}

/**
 * Turns id groups (from `duplicate_groups`) into groups of photos, using a flat
 * list fetched in one round trip.
 *
 * Ids with no photo are dropped — the index can hold a group whose file has
 * since been removed from the library — and a group left with fewer than two
 * members is dropped with them, since there is nothing left to compare. Order
 * within a group is **sharpest first**, so the leading member is the suggested
 * keeper (the frame most in focus) for a keep-the-first cull; unscored frames
 * sort last, ties broken by id so the order is stable.
 */
export function hydrateGroups(groups: number[][], photos: Photo[]): Photo[][] {
  const byId = new Map(photos.map((p) => [p.id, p]));
  return groups
    .map((g) => g.map((id) => byId.get(id)).filter((p): p is Photo => p !== undefined))
    .filter((g) => g.length >= 2)
    .map((g) => g.sort(keeperOrder(g)));
}

/**
 * Reorder each group's *ids* sharpest-first, using a photo lookup for the scores
 * — the id-array counterpart to [`hydrateGroups`]'s ordering, for the stacking
 * and select-duplicates paths that work on ids rather than hydrated photos. The
 * first id of each group is then the suggested keeper; ids with no loaded photo
 * (a duplicate outside the current grid) score lowest and fall to the end, ties
 * broken by id for stability.
 */
export function orderGroupsBySharpness(groups: number[][], photos: Photo[]): number[][] {
  const byId = new Map(photos.map((p) => [p.id, p]));
  return groups.map((g) => {
    const byKeeper = keeperOrder(g.map((id) => byId.get(id)).filter((p): p is Photo => p !== undefined));
    return [...g].sort((a, b) => {
      const pa = byId.get(a);
      const pb = byId.get(b);
      if (pa && pb) return byKeeper(pa, pb);
      // Ids with no loaded photo fall to the end, stable by id.
      if (pa) return -1;
      if (pb) return 1;
      return a - b;
    });
  });
}

/** Below this fraction of a set's median focus, a frame reads as "soft" —
 *  the default, for a frame with no genre or one the rubric doesn't name. */
export const SOFT_RATIO = 0.5;

/**
 * How strictly a frame is judged, by what kind of photograph it is.
 *
 * Focus is scored over the whole frame, so it reads intended out-of-focus
 * areas as softness. That is right for a landscape, where front-to-back
 * sharpness is the point and a near-miss should be caught, and wrong for a
 * portrait or a macro frame, whose bokeh is the point. Astro and abstract are
 * judged on other things entirely, so they are never called soft.
 *
 * Eyes only count where people are the subject. A blink demotes a portrait
 * below its open-eyed alternatives; a passer-by blinking in the corner of a
 * landscape should not.
 */
export interface CullRubric {
  /** Fraction of the set median below which a frame is soft; 0 never flags. */
  softRatio: number;
  /** Whether a detected blink marks the frame down. */
  eyesMatter: boolean;
}

const DEFAULT_RUBRIC: CullRubric = { softRatio: SOFT_RATIO, eyesMatter: true };

const RUBRICS: Record<string, CullRubric> = {
  landscape: { softRatio: 0.6, eyesMatter: false },
  architecture: { softRatio: 0.6, eyesMatter: false },
  "still-life": { softRatio: 0.6, eyesMatter: false },
  wildlife: { softRatio: 0.5, eyesMatter: false },
  portrait: { softRatio: 0.35, eyesMatter: true },
  event: { softRatio: 0.45, eyesMatter: true },
  street: { softRatio: 0.45, eyesMatter: true },
  sports: { softRatio: 0.5, eyesMatter: true },
  macro: { softRatio: 0.35, eyesMatter: false },
  astro: { softRatio: 0, eyesMatter: false },
  abstract: { softRatio: 0, eyesMatter: false },
};

/** The rubric for a frame's genre; the default when it has none. */
export function cullRubric(genre: string | null | undefined): CullRubric {
  return (genre && RUBRICS[genre]) || DEFAULT_RUBRIC;
}

/**
 * The set's median focus score — the basis for a **relative** soft-focus
 * threshold. Relative rather than absolute because sharpness scale swings with
 * lens, subject and scene: a deliberately soft set shouldn't be wholly flagged,
 * and a crisp set's near-misses should. `null` when nothing in the set is scored.
 */
export function medianSharpness(photos: Photo[]): number | null {
  return medianOfSorted(sharpnessScores(photos));
}

/** The median of scores already sorted, as [`sharpnessScores`] gives them, so
 *  a set's scores are sorted once for both. */
export function medianOfSorted(xs: readonly number[]): number | null {
  if (xs.length === 0) return null;
  const mid = Math.floor(xs.length / 2);
  return xs.length % 2 ? xs[mid]! : (xs[mid - 1]! + xs[mid]!) / 2;
}

/**
 * Whether a frame reads as soft *relative to its set* — scored, and below
 * [`SOFT_RATIO`] of the set median. Returns false whenever there's no basis to
 * judge (frame unscored, or no median), so the warning only ever shows when we
 * actually know it's soft — never a false accusation on an un-indexed library.
 */
export function isSoft(photo: Photo, median: number | null): boolean {
  const ratio = cullRubric(photo.genre).softRatio;
  return median != null && photo.sharpness != null && photo.sharpness < median * ratio;
}

/** The five focus bands, softest → sharpest, indexed by [`focusBand`]'s level. */
const FOCUS_BANDS: readonly Key[] = [
  "filters.focus.verySoft",
  "filters.focus.soft",
  "filters.focus.fair",
  "filters.focus.good",
  "filters.focus.sharp",
];

/** Ascending sharpness of the scored frames in a set — the basis a relative
 *  focus rank is read against. Pass it to [`focusBand`] once per set. */
export function sharpnessScores(photos: Photo[]): number[] {
  return photos
    .map((p) => p.sharpness)
    .filter((s): s is number => s != null)
    .sort((a, b) => a - b);
}

/**
 * A frame's focus as a relative band (0 softest … 4 sharpest) plus the
 * percentage of the set it out-sharpens — because the raw variance-of-Laplacian
 * number is only meaningful *against its peers* (it also reads shallow depth of
 * field as "soft"). `sorted` is [`sharpnessScores`] for the set. Returns null
 * when the frame is unscored or there's nothing to compare against.
 */
export function focusBand(
  photo: Photo,
  sorted: number[],
  median: number | null = null,
): { level: number; label: string; percentile: number; soft: boolean } | null {
  if (photo.sharpness == null || sorted.length === 0) return null;
  const s = photo.sharpness;
  const below = sorted.filter((v) => v < s).length;
  const percentile = Math.round((100 * below) / sorted.length);
  const level = Math.min(4, Math.floor((5 * below) / sorted.length));
  // "Soft" only when the suggestions would say so too: ranked by
  // percentile alone, two in five frames of a tack-sharp burst read "Soft"
  // beside "nothing wrong found". Low in the ranking but not soft is said as
  // a rank.
  const soft = isSoft(photo, median);
  const label = level <= 1 && !soft ? t("filters.focus.lessSharp") : t(FOCUS_BANDS[level]!);
  return { level, label, percentile, soft };
}

/** Eyes read as shut when the worst eye in the frame is below this (0 shut … 1
 *  open) — an absolute cut, since eye-openness (unlike focus) has a real scale.
 *  0.5 is the midpoint of the calibrated gap: with the tuned eye crop, open eyes
 *  score ~0.9+ and closed ≤0.15, so this sits clear of both. */
export const EYES_SHUT = 0.5;

/**
 * The frames Highlights may choose from: not rejected, not a blink the
 * eye reading saw, not soft for its genre against the set's own median. The
 * rest (taste, variety, who is in it) is the backend's.
 */
export function highlightCandidates(photos: Photo[]): Photo[] {
  const median = medianSharpness(photos);
  return photos.filter((p) => p.flag !== -1 && !eyesClosed(p) && !isSoft(p, median));
}

/** Whether the face pass has run on a frame (it stamps a face count, even 0). */
export function facesAnalysed(photo: Photo): boolean {
  return photo.faceCount != null;
}

/**
 * Whether a frame likely has a blink — analysed for faces, and its worst eye
 * below [`EYES_SHUT`]. False whenever there's no basis (unanalysed, or no face),
 * so it never accuses a photo the face pass hasn't seen.
 *
 * The user's own verdict wins outright. The classifier reads a downcast gaze as
 * a blink — someone looking at their hands, at a child, at the ground — and
 * that is not a rare frame in a real library.
 *
 * The reading comes from MediaPipe's face mesh and blendshapes,
 * which tell a blink from a smile, and says "can't tell" (null) for a profile,
 * sunglasses or a face too small to read, rather than guessing. On the owner's
 * real photos open eyes read open on every readable face checked. It ranks a
 * burst's keeper again (`keeperOrder`), but still suggests no reject: a face
 * looking right down can read shut, and that is the user's call.
 */
export function eyesClosed(photo: Photo): boolean {
  if (photo.eyesOverride != null) return photo.eyesOverride !== 0;
  // The model's own reading only counts where people are the subject; the
  // user's verdict above counts everywhere.
  if (!cullRubric(photo.genre).eyesMatter) return false;
  return photo.eyesOpen != null && photo.eyesOpen < EYES_SHUT;
}

/**
 * Whether a set's keeper is judged on its faces rather than the whole frame:
 * where people are the subject of every frame and every frame has a
 * face in focus to measure. A portrait's background is soft on purpose, so
 * whole-frame focus reads its bokeh as blur and can prefer the frame where the
 * background happens to be busier over the one where the eyes are sharp.
 *
 * Decided once for the set, never per pair: face and whole-frame scores are on
 * different scales, and mixing them would make the order depend on which pairs
 * a sort happened to compare.
 */
function judgedOnFaces(set: Photo[]): boolean {
  return set.length > 0 && set.every((p) => cullRubric(p.genre).eyesMatter && p.faceSharpness != null);
}

/**
 * Keeper ordering for a set, best first: a likely blink is demoted below any
 * open- or unknown-eye frame (so a sharp blink never leads over an open-eyed
 * alternative), then focus decides within an eye tier — the faces' focus where
 * [`judgedOnFaces`], the whole frame's otherwise — then id for stability. A
 * frame whose eyes can't be read counts as the good tier: only a blink the
 * reading actually saw is demoted.
 *
 * Off while the old eye reading called smiles blinks, and back with the face
 * mesh.
 */
export function keeperOrder(set: Photo[]): (a: Photo, b: Photo) => number {
  const faces = judgedOnFaces(set);
  const tier = (p: Photo) => (eyesClosed(p) ? 0 : 1);
  const sharp = (p: Photo) => (faces ? p.faceSharpness : p.sharpness) ?? -Infinity;
  return (a, b) => tier(b) - tier(a) || sharp(b) - sharp(a) || a.id - b.id;
}

/** The id of the suggested keeper in a set — the sharpest scored frame (the
 *  sharpest face, where people are the subject), but a detected blink is
 *  demoted below any open-eyed alternative — or null if none is scored. Ties
 *  resolve to the lower id, matching the group ordering. */
export function sharpestId(photos: Photo[]): number | null {
  const scored = photos.filter((p) => p.sharpness != null);
  if (scored.length === 0) return null;
  return [...scored].sort(keeperOrder(scored))[0]!.id;
}

/** What "keep this one" does to a set: one pick, everything else rejected. */
export interface KeepPlan {
  keep: number;
  reject: number[];
}

/**
 * The cull decision for a set. Nothing is deleted — the keeper is flagged as a
 * pick and the others as rejects, so the choice is visible, filterable and
 * reversible, and the actual deletion stays the user's own deliberate step.
 */
export function keepPlan(ids: number[], keeperId: number): KeepPlan {
  return { keep: keeperId, reject: ids.filter((id) => id !== keeperId) };
}

/** Steps an index within `[0, length)`, clamping at both ends (no wrap: the end
 *  of a cull should feel like the end, not loop silently back to the start). */
export function stepIndex(index: number, delta: number, length: number): number {
  if (length <= 0) return 0;
  return Math.min(Math.max(index + delta, 0), length - 1);
}

/**
 * Where the cursor lands after moving to `target` when the frames in `leaving`
 * drop out of the strip on the same move.
 *
 * With auto-advance off, a frame judged in a strip it no longer belongs to (a
 * pick in "To review") is held in place until you move off it, then leaves.
 * Each one before the target shifts the target down one place.
 */
export function releaseIndex(
  ids: readonly number[],
  target: number,
  leaving: ReadonlySet<number>,
): number {
  let index = 0;
  for (let i = 0; i < target && i < ids.length; i++) if (!leaving.has(ids[i]!)) index++;
  return index;
}

/**
 * Where a resumed cull pass lands in its queue: the saved photo if it is
 * still in the set, otherwise the same position — which is right when the
 * frame left the set because it was judged (rejecting the last frame viewed in
 * "To review" removes it from that strip), and clamped when the set shrank.
 */
export function resumeIndex(
  ids: readonly number[],
  saved: { photoId: number; index: number },
): number {
  if (ids.length === 0) return 0;
  const byId = ids.indexOf(saved.photoId);
  if (byId >= 0) return byId;
  return Math.min(Math.max(saved.index, 0), ids.length - 1);
}

/**
 * The folder a photo sits in, as its last two path segments ("Pictures\A6400").
 *
 * Duplicate review folds byte-identical copies in with the lookalikes
 * (`dedup::exact_groups`), and two copies of one file are the same picture
 * with the same name: side by side, the folder is the only thing that tells
 * them apart — and which folder to keep is the decision being made.
 */
export function folderOf(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  parts.pop(); // the file itself
  const sep = path.includes("\\") ? "\\" : "/";
  return parts.slice(-2).join(sep);
}

/**
 * Whether two photos are the same shot catalogued twice: another path, but the
 * same name, capture time, size and exposure. Nothing in a photo row can prove
 * the bytes match, but a copy of a card in a second folder looks exactly so,
 * and two different exposures essentially never do.
 */
export function sameShot(a: Photo, b: Photo): boolean {
  return (
    a.id !== b.id &&
    a.path !== b.path &&
    a.filename === b.filename &&
    a.captureDate === b.captureDate &&
    a.width === b.width &&
    a.height === b.height &&
    a.shutter === b.shutter &&
    a.aperture === b.aperture &&
    a.iso === b.iso
  );
}

/** A set that is nothing but copies of one shot: every frame the same as the first. */
export function allCopies(set: Photo[]): boolean {
  const [first, ...rest] = set;
  return !!first && rest.length > 0 && rest.every((p) => sameShot(first, p));
}
