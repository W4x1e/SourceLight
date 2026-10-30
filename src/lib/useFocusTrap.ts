import { useEffect, type RefObject } from "react";
import { focusables, overlayOpen } from "./useEscape";

/**
 * Keeps Tab inside a full-screen view — the Lightbox, Compare, the slideshow
 * — and puts focus back where it was when the view closes.
 *
 * They cover the whole window but aren't modal in `useModal`'s sense: they
 * answer their own keys (arrows, ratings, the rebindable close), and
 * counting as modal would make `overlayOpen()` true and silence those very
 * keys. Without this, Tab walked out of them into the library hidden
 * underneath — the selection panel's stars and inputs, unseen — and a view
 * opened from the grid with Enter left focus on nothing when it closed.
 *
 * `root` should be focusable (`tabIndex={-1}`); focus moves to it on open
 * unless something inside already has it. A dialog or panel opened over
 * the view traps Tab itself, so this stands aside while one is up.
 */
export function useFocusTrap(active: boolean, root: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    if (!active) return;
    const opener = document.activeElement as HTMLElement | null;
    const frame = requestAnimationFrame(() => {
      const el = root.current;
      if (el && !el.contains(document.activeElement)) el.focus({ preventScroll: true });
    });
    const onKey = (e: KeyboardEvent) => {
      const el = root.current;
      if (e.key !== "Tab" || !el || overlayOpen()) return;
      const list = focusables(el);
      const [first, last] = [list[0], list[list.length - 1]];
      if (!first || !last) return;
      const at = document.activeElement;
      if (!el.contains(at) || at === el) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && at === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && at === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKey, true);
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, [active, root]);
}
