import { useLayoutEffect, useState, type DependencyList, type RefObject } from "react";

/**
 * The size of an element's content box: inside its padding and any scrollbar.
 * That is the space its children are laid out in.
 *
 * The survey and Compare each measured this twice, two ways: `clientWidth` at
 * mount, which includes the padding, then the observer's `contentRect`, which
 * doesn't. The first layout was 24px too wide, so a row of tiles that should
 * have fitted wrapped onto a second line and the view scrolled, and the
 * observer did not always fire again to correct it. One measurement, used for
 * both.
 */
export function useContentBox<T extends HTMLElement>(
  ref: RefObject<T | null>,
  deps: DependencyList = [],
): { width: number; height: number } {
  const [box, setBox] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const read = () => {
      const cs = getComputedStyle(el);
      const px = (v: string) => parseFloat(v) || 0;
      setBox({
        // `clientWidth` already leaves out the scrollbar and border.
        width: Math.max(0, el.clientWidth - px(cs.paddingLeft) - px(cs.paddingRight)),
        height: Math.max(0, el.clientHeight - px(cs.paddingTop) - px(cs.paddingBottom)),
      });
    };
    const ro = new ResizeObserver(read);
    ro.observe(el);
    read();
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the caller says when the element changes
  }, deps);
  return box;
}
