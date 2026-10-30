import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { DirectFrame } from "../bindings";
import { api } from "./api";
import { useLatestFrame } from "./useLatestFrame";

/**
 * Develop's preview drawn straight into a GPU surface beneath the page, rather
 * than read back, wrapped as an image and decoded by the webview every frame.
 *
 * The page cuts a transparent hole where the canvas is (`direct-canvas` on the
 * root element), and the surface — a Metal layer on a Mac — shows through it.
 * Everything drawn over the photo (crop handles, masks, the zoom pill) is
 * still the page, on top, and takes the clicks as before.
 *
 * The images never go away: the split view shows two photos, which one surface
 * cannot, and anything that fails here falls back to them without a word.
 */

/** Brings the surface up while `wanted` (and Develop is open). False until it
 *  is up, and for good if it cannot be — the images carry on. */
export function useDirectSurface(wanted: boolean): boolean {
  const [up, setUp] = useState(false);
  useEffect(() => {
    if (!wanted || !("__TAURI_INTERNALS__" in window)) return;
    let live = true;
    api
      .setDirectRendering(true)
      .then(() => {
        if (live) setUp(true);
      })
      .catch((e) => console.warn("direct preview unavailable, using images", e));
    return () => {
      live = false;
      setUp(false);
      void api.setDirectRendering(false).catch(() => undefined);
    };
  }, [wanted]);
  return up;
}

/** Makes the page a hole over the canvas while `on`. */
export function useTransparentPage(on: boolean) {
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("direct-canvas", on);
    return () => document.documentElement.classList.remove("direct-canvas");
  }, [on]);
}

/**
 * Draws `frame` into the surface: one at a time, always converging on the
 * newest — the rule `useLatestFrame` keeps for the images, for the same reason
 * (a drag asks for frames far faster than they can be drawn). `version` is the
 * edit's frame number, so an edit redraws an unchanged view. `onDrawn` runs
 * once a frame is on screen, with the frame it drew.
 */
export function useDirectFrames(
  frame: DirectFrame | null,
  version: number,
  onDrawn: (frame: DirectFrame) => void,
) {
  const [shown, settled] = useLatestFrame(frame ? `${version} ${JSON.stringify(frame)}` : "");
  const onDrawnRef = useRef(onDrawn);
  onDrawnRef.current = onDrawn;

  useEffect(() => {
    if (!shown) {
      settled();
      return;
    }
    const drawn = JSON.parse(shown.slice(shown.indexOf(" ") + 1)) as DirectFrame;
    // Settled on failure too: an unreported failure would stop every frame
    // after it.
    api
      .renderDirect(drawn)
      .then(() => onDrawnRef.current(drawn))
      .catch((e) => console.warn("direct frame failed", e))
      .finally(settled);
  }, [shown, settled]);
}
