/**
 * Faces across a burst: each person's face in every frame, lined up in
 * one row, so "who blinked where" and "whose face is soft in which frame" read
 * at a glance — the judgement a group shot actually turns on.
 *
 * Pure, so the lining-up can be tested without a burst of real photos.
 */
import type { PhotoFace } from "../bindings";
import { EYES_SHUT } from "./survey";

export interface FaceRow {
  /** The person, when the faces are grouped into one. */
  personId: number | null;
  name: string | null;
  /** One cell per frame, in the frames' order; null where they aren't in it. */
  cells: (PhotoFace | null)[];
}

/** An ungrouped face joins a row only if it overlaps that row's face in the
 *  nearest frame at least this much (intersection over union). A burst moves
 *  little between frames; two different people rarely overlap this far. */
export const SAME_FACE_IOU = 0.3;

/** A face this much softer than the sharpest of its row reads as soft. Rows
 *  hold one person across a burst, so the face is about the same size in every
 *  cell — which is what makes their focus scores comparable at all. */
export const SOFT_FACE_RATIO = 0.5;

type Box = [number, number, number, number];

function boxOf(f: PhotoFace): Box | null {
  const b = f.boxFrac;
  return b.length >= 4 ? [b[0]!, b[1]!, b[2]!, b[3]!] : null;
}

/** Intersection over union of two `[x, y, w, h]` boxes; 0 when either is unknown. */
export function iou(a: Box | null, b: Box | null): number {
  if (!a || !b) return 0;
  const ix = Math.max(0, Math.min(a[0] + a[2], b[0] + b[2]) - Math.max(a[0], b[0]));
  const iy = Math.max(0, Math.min(a[1] + a[3], b[1] + b[3]) - Math.max(a[1], b[1]));
  const inter = ix * iy;
  const union = a[2] * a[3] + b[2] * b[3] - inter;
  return union > 0 ? inter / union : 0;
}

/** The row's face in the frame nearest `col` that it has one in. */
function nearestBox(row: FaceRow, col: number): Box | null {
  for (let d = 1; d < row.cells.length; d++) {
    for (const c of [col - d, col + d]) {
      const f = row.cells[c];
      if (f) return boxOf(f);
    }
  }
  return null;
}

const area = (f: PhotoFace) => {
  const b = boxOf(f);
  return b ? b[2] * b[3] : 0;
};

/**
 * Lines up the faces of `frameIds` (in that order) into rows, most present
 * first, then largest, keeping `maxRows`; `more` counts the rows left out.
 *
 * A face grouped into a person goes in that person's row. Anything else — not
 * grouped, or a second face the grouping put under a person already present in
 * that frame — joins the row whose face in the nearest frame it overlaps most,
 * or starts a row of its own.
 */
export function faceRows(
  frameIds: number[],
  faces: PhotoFace[],
  maxRows = 6,
): { rows: FaceRow[]; more: number } {
  const col = new Map(frameIds.map((id, i) => [id, i]));
  const rows: FaceRow[] = [];
  const empty = () => frameIds.map(() => null as PhotoFace | null);
  const loose: [number, PhotoFace][] = [];

  // Largest first, so a person's own face beats a stray second match.
  const sorted = [...faces].sort((a, b) => area(b) - area(a) || a.faceId - b.faceId);
  for (const f of sorted) {
    const c = col.get(f.photoId);
    if (c === undefined) continue;
    if (f.personId == null) {
      loose.push([c, f]);
      continue;
    }
    let row = rows.find((r) => r.personId === f.personId);
    if (!row) {
      row = { personId: f.personId, name: f.personName, cells: empty() };
      rows.push(row);
    }
    if (row.cells[c]) loose.push([c, f]);
    else row.cells[c] = f;
  }

  for (const [c, f] of loose) {
    let best: FaceRow | null = null;
    let bestIou = SAME_FACE_IOU;
    for (const row of rows) {
      if (row.cells[c]) continue;
      const o = iou(boxOf(f), nearestBox(row, c));
      if (o >= bestIou) {
        best = row;
        bestIou = o;
      }
    }
    if (best) best.cells[c] = f;
    else {
      const row: FaceRow = { personId: null, name: null, cells: empty() };
      row.cells[c] = f;
      rows.push(row);
    }
  }

  const present = (r: FaceRow) => r.cells.filter(Boolean).length;
  const largest = (r: FaceRow) => Math.max(...r.cells.map((f) => (f ? area(f) : 0)));
  rows.sort((a, b) => present(b) - present(a) || largest(b) - largest(a));
  return { rows: rows.slice(0, maxRows), more: Math.max(0, rows.length - maxRows) };
}

/**
 * Whether a face's eyes read as closed. `override` is the photo's own verdict
 * (`eyesOverride`): "not closed" there clears every face in it, since it was
 * given against the photo, not one face.
 */
export function faceEyesClosed(face: PhotoFace, override?: number | null): boolean {
  if (override === 0) return false;
  return face.eyeOpen != null && face.eyeOpen < EYES_SHUT;
}

/**
 * Whether everyone in a frame has their eyes open — the frame a group shot
 * wants. Only claimed for two or more faces, each with an eye reading: one
 * face is what the ordinary eyes badge is for, and a face with no reading (a
 * profile) cannot vouch for anything.
 */
export function everyoneOpen(faces: PhotoFace[], override?: number | null): boolean {
  return (
    faces.length >= 2 &&
    faces.every((f) => f.eyeOpen != null) &&
    !faces.some((f) => faceEyesClosed(f, override))
  );
}

/** The row's sharpest face, and which of its faces read soft beside it. */
export function rowFocus(row: FaceRow): { sharpest: number | null; soft: Set<number> } {
  const scored = row.cells.filter((f): f is PhotoFace => f != null && f.sharpness != null);
  const soft = new Set<number>();
  if (scored.length < 2) return { sharpest: null, soft };
  const best = scored.reduce((a, b) => (b.sharpness! > a.sharpness! ? b : a));
  for (const f of scored) if (f.sharpness! < best.sharpness! * SOFT_FACE_RATIO) soft.add(f.faceId);
  return { sharpest: best.faceId, soft };
}
