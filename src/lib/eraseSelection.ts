import type { Mask, MaskComponent, Stroke } from "../types";

/**
 * What magic erase is about to take out: an object picked by clicking
 * (SAM 2's selection, refined by each click) and what has been brushed onto
 * or off it. Held by Develop until Erase is pressed, and never in the edit
 * stack: only the erase it becomes is.
 *
 * It goes to the backend as an ordinary `Mask` ([`selectionMask`]), so the
 * erase is cut from the same coverage the masking tools would give, and is
 * drawn over the photo from the same maths ([`selectionCoverage`]).
 */
export interface EraseSelection {
  /** The clicks so far: + to include, − to exclude. */
  points: { x: number; y: number; include: boolean }[];
  /** SAM 2's selection from those clicks. */
  picked: Mask | null;
  /** Brushed onto the selection, and brushed off it. */
  add: Stroke[];
  remove: Stroke[];
}

export const NO_SELECTION: EraseSelection = { points: [], picked: null, add: [], remove: [] };

/** Whether anything is selected to erase. */
export function hasSelection(sel: EraseSelection): boolean {
  return sel.picked !== null || sel.add.length > 0;
}

/** The selection as the mask `develop_erase` takes: the picked object, plus
 *  what was brushed on, minus what was brushed off. */
export function selectionMask(sel: EraseSelection): Mask | null {
  const components: MaskComponent[] = [...(sel.picked?.components ?? [])];
  if (sel.add.length) {
    components.push({ shape: { type: "brush", strokes: sel.add }, inverted: false, mode: "add" });
  }
  if (sel.remove.length && components.length) {
    components.push({ shape: { type: "brush", strokes: sel.remove }, inverted: false, mode: "subtract" });
  }
  return components.length ? { components } : null;
}

/** An AI selection's stored coverage: base64 of (run, value) byte pairs, as
 *  `mask::encode_ai` writes it. Short or malformed data reads as empty. */
export function decodeCoverage(rle: string, width: number, height: number): Uint8Array {
  const out = new Uint8Array(width * height);
  let bytes: string;
  try {
    bytes = atob(rle);
  } catch {
    return out;
  }
  let at = 0;
  for (let i = 0; i + 1 < bytes.length && at < out.length; i += 2) {
    const run = Math.min(bytes.charCodeAt(i), out.length - at);
    out.fill(bytes.charCodeAt(i + 1), at, at + run);
    at += run;
  }
  return out;
}

function smoothstep(a: number, b: number, x: number): number {
  if (Math.abs(b - a) < 1e-6) return x < a ? 0 : 1;
  const t = Math.min(Math.max((x - a) / (b - a), 0), 1);
  return t * t * (3 - 2 * t);
}

/** Lays brush dabs into `cov` (0..1, `w` x `h`), adding or taking away, as
 *  `mask::rasterize` does. */
function brush(cov: Float32Array, w: number, h: number, strokes: Stroke[], take: boolean) {
  const aspect = h / w;
  for (const s of strokes) {
    const r = Math.max(s.radius ?? 0, 1e-4);
    const inner = r * Math.min(Math.max(s.hardness ?? 0, 0), 1);
    const sx = s.x ?? 0;
    const sy = s.y ?? 0;
    const ry = r / Math.max(aspect, 1e-4);
    const x0 = Math.max(Math.floor((sx - r) * w), 0);
    const x1 = Math.min(Math.ceil((sx + r) * w), w);
    const y0 = Math.max(Math.floor((sy - ry) * h), 0);
    const y1 = Math.min(Math.ceil((sy + ry) * h), h);
    for (let py = y0; py < y1; py++) {
      const dy = ((py + 0.5) / h - sy) * aspect;
      for (let px = x0; px < x1; px++) {
        const dx = (px + 0.5) / w - sx;
        const c = 1 - smoothstep(inner, r, Math.hypot(dx, dy));
        if (c <= 0) continue;
        const k = py * w + px;
        const now = cov[k]!;
        cov[k] = take || s.erase ? now * (1 - c) : now + c * (1 - now);
      }
    }
  }
}

/** The selection's coverage, 0..1, at `w` x `h` over the framed photo: for
 *  drawing it in red before it is erased. */
export function selectionCoverage(sel: EraseSelection, w: number, h: number): Float32Array {
  const cov = new Float32Array(w * h);
  for (const c of sel.picked?.components ?? []) {
    if (c.shape.type !== "ai") continue;
    const { width: aw, height: ah } = c.shape;
    const src = decodeCoverage(c.shape.rle, aw, ah);
    for (let y = 0; y < h; y++) {
      const sy = Math.min(Math.floor(((y + 0.5) / h) * ah), ah - 1);
      for (let x = 0; x < w; x++) {
        const sx = Math.min(Math.floor(((x + 0.5) / w) * aw), aw - 1);
        cov[y * w + x] = Math.max(cov[y * w + x]!, src[sy * aw + sx]! / 255);
      }
    }
  }
  brush(cov, w, h, sel.add, false);
  brush(cov, w, h, sel.remove, true);
  return cov;
}
