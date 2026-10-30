import { useRef, useState } from "react";

/**
 * Drag-to-pan, as handler props for whatever element shows the photo.
 *
 * **Extracted so the wiring can be exercised without the app.** The arithmetic
 * was testable and correct while dragging did not work at all, which is the
 * signature of an event problem — and this codebase has had one before, where
 * a drag's bookkeeping lived in React state and a fast tap read it stale. The
 * start point is therefore a ref: pointerdown and pointermove can land in the
 * same frame, and a re-render is not guaranteed between them.
 *
 * `dragging` is state rather than a ref on purpose — it only feeds the cursor,
 * and a ref read during render gives last render's value, so the cursor never
 * changes.
 */
export function usePanDrag(enabled: boolean, onPan: (dxCss: number, dyCss: number) => void) {
  const from = useRef<{ x: number; y: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  const stop = () => {
    from.current = null;
    setDragging(false);
  };

  return {
    dragging,
    handlers: {
      onPointerDown: (e: React.PointerEvent) => {
        if (!enabled) return;
        // Capture, so a fast drag that leaves the element keeps reporting.
        e.currentTarget.setPointerCapture(e.pointerId);
        from.current = { x: e.clientX, y: e.clientY };
        setDragging(true);
      },
      onPointerMove: (e: React.PointerEvent) => {
        const start = from.current;
        if (!start) return;
        onPan(e.clientX - start.x, e.clientY - start.y);
        // Deltas are incremental, so the origin advances with the pointer.
        from.current = { x: e.clientX, y: e.clientY };
      },
      onPointerUp: stop,
      onPointerCancel: stop,
    },
  };
}
