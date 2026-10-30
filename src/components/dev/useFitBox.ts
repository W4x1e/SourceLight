import { useLayoutEffect, useRef, useState } from "react";
import { fitBox, type Rect, type Size, type View } from "../../lib/viewport";

/**
 * Measures an overlay container and returns the on-screen rect the framed image
 * occupies inside it — the letterbox at fit-to-window, or the zoomed/panned
 * placement when a `view` is given.
 *
 * Both mask overlays position their 0..1 UV against this box, so pinning the box
 * to the renderer's viewport is all it takes for handles, brush dabs and the
 * coverage preview to track the image under zoom and pan. Only the container
 * *size* needs an observer; the box itself is derived each render, so a pan or
 * zoom (a prop change) recomputes it with no effect.
 */
export function useFitBox(frameAspect: number, view: View | null, framePx: Size) {
  const outer = useRef<HTMLDivElement>(null);
  const [container, setContainer] = useState<Size>({ width: 0, height: 0 });

  useLayoutEffect(() => {
    const el = outer.current;
    if (!el) return;
    const measure = () => setContainer({ width: el.clientWidth, height: el.clientHeight });
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);

  const box: Rect = fitBox(container, frameAspect, view, framePx, window.devicePixelRatio || 1);
  return { outer, box, container };
}
