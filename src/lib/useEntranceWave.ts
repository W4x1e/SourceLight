import { useEffect, useRef, useState } from "react";
import { sameIds } from "./gridRows";

/**
 * Whether the grid's staggered entrance should be playing: for `ms` after the
 * displayed set of photos changes (a filter applied or cleared, a search run).
 *
 * Not for a patch. A rating, flag or label replaces the list with a new array
 * of the same photos, and treating that as a new set replayed the wave over
 * every visible cell on each keypress of a cull. A patch that lands mid-wave
 * leaves the wave to finish on time: the timer lives in a ref, not in the
 * effect's cleanup, which a patch would otherwise cancel.
 */
export function useEntranceWave(photos: readonly { id: number }[], ms: number): boolean {
  const [animating, setAnimating] = useState(false);
  const shown = useRef(photos);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    const before = shown.current;
    shown.current = photos;
    if (sameIds(before, photos)) return;
    setAnimating(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setAnimating(false), ms);
  }, [photos, ms]);
  useEffect(() => () => clearTimeout(timer.current), []);
  return animating;
}
