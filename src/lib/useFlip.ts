import { useLayoutEffect, useRef, type RefObject } from "react";

/** Mirrors --dur-base / --ease-out in styles.css; WAAPI cannot read them. */
const MOVE_MS = 190;
const FADE_MS = 160;
const EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

/** Animations this hook created, so it only ever cancels its own. */
const flipAnims = new WeakSet<Animation>();

const prefersReducedMotion = () =>
  typeof window !== "undefined" &&
  typeof window.matchMedia === "function" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * FLIP animation over a container's `[data-flip]` children.
 *
 * When `dep` changes and the children reflow — because a facet list above them
 * grew or shrank, or a list reordered — each surviving child slides from its
 * previous position to its new one instead of snapping there, and children that
 * appear fade in. This is what turns "the whole rail jumps when I pick a genre"
 * into a settle.
 *
 * FLIP = First (measure old), Last (measure new), Invert (offset to look
 * unmoved), Play (transition the offset away). Positions are read *after*
 * cancelling any in-flight FLIP on the node, so a fast burst of filter changes
 * stays coordinated rather than compounding half-finished transforms. Vertical
 * only — the rail never moves a row sideways. Honours reduced-motion (measures,
 * records, animates nothing), and skips a move under half a pixel.
 */
export function useFlip(container: RefObject<HTMLElement | null>, dep: unknown) {
  const prev = useRef<Map<string, DOMRect>>(new Map());

  useLayoutEffect(() => {
    const el = container.current;
    if (!el) return;

    const nodes = Array.from(el.querySelectorAll<HTMLElement>("[data-flip]"));

    // Cancel our own in-flight FLIP everywhere first, so every measurement below
    // reflects the settled layout rather than a node mid-slide.
    for (const n of nodes) {
      for (const a of n.getAnimations()) if (flipAnims.has(a)) a.cancel();
    }

    const reduced = prefersReducedMotion();
    const next = new Map<string, DOMRect>();

    for (const n of nodes) {
      const key = n.dataset.flip;
      if (!key) continue;
      const rect = n.getBoundingClientRect();
      next.set(key, rect);
      if (reduced) continue;

      const old = prev.current.get(key);
      if (old) {
        const dy = old.top - rect.top;
        if (Math.abs(dy) > 0.5) {
          const anim = n.animate(
            [{ transform: `translateY(${dy}px)` }, { transform: "translateY(0)" }],
            { duration: MOVE_MS, easing: EASE },
          );
          flipAnims.add(anim);
        }
      } else {
        // A newly present block (first paint, or a section that gained content).
        const anim = n.animate([{ opacity: 0 }, { opacity: 1 }], {
          duration: FADE_MS,
          easing: EASE,
        });
        flipAnims.add(anim);
      }
    }

    prev.current = next;
  }, [container, dep]);
}
