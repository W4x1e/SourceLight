import { useRef } from "react";

/**
 * `value`, or the one before it while `same` says nothing that matters
 * changed: a value derived from it with `useMemo` is then kept too.
 * A verdict replaces the whole photos array, though only one photo's rating
 * moved; the grid's rows, Cull's order and the timeline's counts don't depend
 * on ratings, and rebuilding them on every key cost frames.
 */
export function useStable<T>(value: T, same: (a: T, b: T) => boolean): T {
  const kept = useRef(value);
  if (kept.current !== value && !same(kept.current, value)) kept.current = value;
  return kept.current;
}

type Framed = { id: number; captureDate: string | null; sharpness?: number | null };

/** The same photos in the same order, with the same dates and focus scores:
 *  all a layout or an order depends on. */
export function sameFrames(a: readonly Framed[], b: readonly Framed[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i]!;
    const y = b[i]!;
    if (x.id !== y.id || x.captureDate !== y.captureDate || (x.sharpness ?? null) !== (y.sharpness ?? null)) return false;
  }
  return true;
}
