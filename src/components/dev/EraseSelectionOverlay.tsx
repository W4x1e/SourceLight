import { useEffect, useRef } from "react";
import type { Size, View } from "../../lib/viewport";
import { useFitBox } from "./useFitBox";
import { selectionCoverage, type EraseSelection } from "../../lib/eraseSelection";

/** The long edge the tint is worked out at: smooth when scaled up, and quick
 *  enough to redraw on every brush dab. */
const TINT_EDGE = 512;

interface Props {
  frameAspect: number;
  framePx: Size;
  view: View | null;
  selection: EraseSelection;
}

/**
 * What magic erase is about to take out, tinted red over the photo. Drawn
 * from the selection itself rather than by the renderer, since a selection
 * is not part of the edit until it has been erased. Placed like the other
 * overlays (`useFitBox`), so it stays on the object as the view zooms and
 * pans. Takes no pointer events: the click or brush overlay above it does.
 */
export default function EraseSelectionOverlay({ frameAspect, framePx, view, selection }: Props) {
  const { outer, box } = useFitBox(frameAspect, view, framePx);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [w, h] =
    frameAspect >= 1
      ? [TINT_EDGE, Math.max(1, Math.round(TINT_EDGE / frameAspect))]
      : [Math.max(1, Math.round(TINT_EDGE * frameAspect)), TINT_EDGE];

  useEffect(() => {
    const ctx = canvas.current?.getContext("2d");
    if (!ctx) return;
    const cov = selectionCoverage(selection, w, h);
    const img = ctx.createImageData(w, h);
    for (let i = 0; i < cov.length; i++) {
      img.data[i * 4] = 255;
      img.data[i * 4 + 1] = 48;
      img.data[i * 4 + 2] = 88;
      img.data[i * 4 + 3] = Math.round(cov[i]! * 120);
    }
    ctx.putImageData(img, 0, 0);
  }, [selection, w, h]);

  return (
    <div ref={outer} className="pointer-events-none absolute inset-0 overflow-hidden">
      <canvas
        ref={canvas}
        width={w}
        height={h}
        className="absolute"
        style={{ left: box.left, top: box.top, width: box.width, height: box.height }}
      />
    </div>
  );
}
