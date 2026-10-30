import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** How long an overlay's exit runs: `--dur-fast` in styles.css. */
export const EXIT_MS = 140;

/**
 * Keeps an overlay mounted for its exit animation.
 *
 * Overlays used to be `open && <Panel />`, so closing one removed it in the
 * same frame and it could only ever snap shut. This returns `mounted` true
 * until `exitMs` after `open` goes false, and `closing` true for that stretch,
 * which the overlay puts on its root as `data-closing` to run its exit (see
 * the `anim-*` classes in styles.css). Opening again mid-exit cancels the
 * unmount and the entrance runs over it.
 *
 * Under reduced motion the CSS cuts the animation to nothing and the overlay
 * sits invisible, taking no clicks, for the rest of `exitMs`, which nobody sees.
 */
export function usePresence(open: boolean, exitMs = EXIT_MS): { mounted: boolean; closing: boolean } {
  const [mounted, setMounted] = useState(open);
  useEffect(() => {
    if (open) {
      setMounted(true);
      return;
    }
    if (!mounted) return;
    const timer = window.setTimeout(() => setMounted(false), exitMs);
    return () => window.clearTimeout(timer);
  }, [open, mounted, exitMs]);
  return { mounted: open || mounted, closing: !open && mounted };
}

/**
 * The last non-null value, held while it goes null.
 *
 * An overlay driven by store state (`captureTimeFor`, the pending confirm)
 * loses its content the moment that state clears, which is the moment its exit
 * starts. Rendering from this instead keeps the card showing what it showed
 * while it fades, rather than going blank or throwing on the way out.
 */
export function useHeld<T>(value: T | null | undefined): T | null | undefined {
  const held = useRef(value);
  if (value != null) held.current = value;
  return value ?? held.current;
}

/**
 * A list whose removed items stay a moment longer, flagged `closing`, so they
 * can animate out where they stood instead of vanishing (the toast stack).
 * An item reappearing under the same key mid-exit simply comes back.
 */
export function useLeavingList<T>(
  items: readonly T[],
  keyOf: (item: T) => string | number,
  exitMs = EXIT_MS,
): { item: T; closing: boolean }[] {
  const prev = useRef(items);
  const [leaving, setLeaving] = useState<{ item: T; index: number }[]>([]);
  const timers = useRef(new Set<number>());

  // Before paint, so a removed item never flashes out for a frame first.
  useLayoutEffect(() => {
    const now = new Set(items.map(keyOf));
    const gone = prev.current
      .map((item, index) => ({ item, index }))
      .filter(({ item }) => !now.has(keyOf(item)));
    prev.current = items;
    setLeaving((l) => {
      const kept = l.filter(({ item }) => !now.has(keyOf(item)));
      return gone.length || kept.length !== l.length ? [...kept, ...gone] : l;
    });
    if (gone.length === 0) return;
    const goneKeys = new Set(gone.map(({ item }) => keyOf(item)));
    const timer = window.setTimeout(() => {
      timers.current.delete(timer);
      setLeaving((l) => l.filter(({ item }) => !goneKeys.has(keyOf(item))));
    }, exitMs);
    timers.current.add(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyOf is a pure accessor
  }, [items, exitMs]);

  useEffect(() => {
    const pending = timers.current;
    return () => pending.forEach((t) => window.clearTimeout(t));
  }, []);

  const out: { item: T; closing: boolean }[] = items.map((item) => ({ item, closing: false }));
  for (const { item, index } of [...leaving].sort((a, b) => a.index - b.index)) {
    out.splice(Math.min(index, out.length), 0, { item, closing: true });
  }
  return out;
}
