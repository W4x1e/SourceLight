import { useEffect, useRef, type RefObject } from "react";

/**
 * Escape closes the overlay — the innermost one only.
 *
 * Every overlay in the app is meant to answer Escape — the dialogs do, and
 * Lightbox/Compare/Develop/Cull do through the `close-overlay` binding. The
 * panels each had to grow their own listener to join in, so Settings had one
 * and Activity and Shortcuts did not, which is why those two could only be left
 * by aiming at their × button.
 *
 * **Innermost first, and nothing behind it.** Each open overlay is on a
 * stack; the listener runs in the capture phase, so it hears the key before
 * any workspace does, and only the newest overlay answers — then stops the key
 * there. Before, Escape on a menu or the Export dialog also reached Develop or
 * Cull underneath and closed it.
 *
 * The `.dialog-scrim` guard: a confirm opened from inside a panel owns Escape
 * first, and without it the keypress meant to cancel the dialog would close
 * the panel out from under it. `own` is the overlay's own element, so a
 * scrim it sits in (GroupsDialog draws one) isn't mistaken for another's.
 *
 * `active` lets a caller keep hook order stable while the overlay is hidden.
 */
export function useEscape(active: boolean, onClose: () => void, own?: RefObject<HTMLElement | null>): void {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!active) return;
    const token = Symbol("overlay");
    stack.push(token);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || stack[stack.length - 1] !== token) return;
      const mine = own?.current;
      const scrims = Array.from(document.querySelectorAll(SCRIM));
      if (scrims.some((s) => !mine || !s.contains(mine))) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      close.current();
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      stack.splice(stack.indexOf(token), 1);
    };
  }, [active, own]);
}

/** Open overlays, newest last. */
const stack: symbol[] = [];

/**
 * Not one on its way out. An overlay fading after it closed (`data-closing`,
 * see `usePresence`) takes no clicks and no keys, and must not hold anyone
 * else's either: counted as open, it swallowed the first key pressed after a
 * dialog was answered.
 */
const LIVE = ":not([data-closing]):not([data-closing] *)";
/** A dialog's scrim that is really there. */
export const SCRIM = `.dialog-scrim${LIVE}`;

/** Whether a dialog (a confirm, the palette, a modal) is open over everything. */
export function scrimOpen(): boolean {
  return document.querySelector(SCRIM) !== null;
}

/** Modal overlays open now, counted by `useModal`. */
let modals = 0;

/**
 * Whether something modal is over the workspace — a modal panel or dialog, a
 * confirm, a menu — so a workspace's own keys (verdicts, arrows, Escape) stand
 * aside rather than act on the photo behind it.
 */
export function overlayOpen(): boolean {
  return modals > 0 || document.querySelector(`${SCRIM}, [aria-modal="true"]${LIVE}, [data-overlay]${LIVE}`) !== null;
}

/**
 * A modal overlay's behaviour, in one place, as the confirm dialog and
 * the guided tour already had it: Escape closes it (innermost only), Tab stays
 * inside `card`, focus moves into it on open — to `[data-autofocus]`, else
 * the first control — and goes back where it was on close. The card itself
 * should carry `role="dialog"`, `aria-modal="true"` and `aria-labelledby`.
 */
export function useModal(active: boolean, card: RefObject<HTMLElement | null>, onClose: () => void): void {
  useEscape(active, onClose, card);
  useEffect(() => {
    if (!active) return;
    modals += 1;
    const opener = document.activeElement as HTMLElement | null;
    const frame = requestAnimationFrame(() => {
      const root = card.current;
      if (!root || root.contains(document.activeElement)) return;
      const target = root.querySelector<HTMLElement>("[data-autofocus]") ?? focusables(root)[0] ?? root;
      target.focus();
    });
    const onKey = (e: KeyboardEvent) => {
      const root = card.current;
      if (e.key !== "Tab" || !root) return;
      // Only while this is the top layer: a confirm over it traps its own.
      if (scrimOpen() && !root.closest(SCRIM)) return;
      const list = focusables(root);
      const [first, last] = [list[0], list[list.length - 1]];
      if (!first || !last) return;
      const at = document.activeElement;
      if (!root.contains(at)) {
        e.preventDefault();
        first.focus();
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
      modals -= 1;
      // Back where it came from, if that is still on the page.
      if (opener?.isConnected) opener.focus();
    };
  }, [active, card]);
}

export function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((el) => !el.hasAttribute("disabled") && el.getAttribute("aria-hidden") !== "true");
}

/**
 * Whether a key's target is somewhere text is typed, where a workspace's
 * letter and arrow keys mustn't act. A slider or checkbox is not: a
 * range input kept focus after a drag, and Develop's keys went dead until
 * the photo was clicked.
 */
export function isTextEntry(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable || el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  if (el instanceof HTMLInputElement) {
    return !["range", "checkbox", "radio", "button", "submit", "reset", "color"].includes(el.type);
  }
  return false;
}
