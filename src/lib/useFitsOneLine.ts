import { useLayoutEffect, useState, type RefObject } from "react";

/**
 * Whether a one-line, truncating element shows its text whole.
 *
 * For a hint that is worth reading in full or not at all: cut short at a
 * narrow window it read as "Photos grouped by w…". Measured before paint and
 * again whenever the element is resized or `deps` change (the text itself).
 */
export function useFitsOneLine(ref: RefObject<HTMLElement | null>, deps: unknown[] = []): boolean {
  const [fits, setFits] = useState(true);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setFits(el.scrollWidth <= el.clientWidth + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the caller says what changes the text
  }, deps);
  return fits;
}
