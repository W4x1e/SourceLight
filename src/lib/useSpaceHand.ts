import { useEffect, useRef, useState } from "react";
import { isTextEntry } from "./useEscape";

/**
 * Space held over an overlay turns a left drag into a pan. The
 * mask and pick overlays keep the left button for painting and picking, so a
 * zoomed photo could only be moved there with a middle button or a touchpad,
 * which a plain mouse or a pen doesn't have.
 *
 * Space counts only while the pointer is over the overlay, so it never takes
 * the key from a field or a button elsewhere. Over it, the key is swallowed
 * on the way down and up both: a focused button clicks on Space's keyup.
 */
export function useSpaceHand() {
  const over = useRef(false);
  const heldNow = useRef(false);
  // State as well as the ref: the ref answers a pointerdown in the same frame
  // as the keydown, the state changes the cursor.
  const [held, setHeld] = useState(false);

  useEffect(() => {
    const set = (on: boolean) => {
      heldNow.current = on;
      setHeld(on);
    };
    const down = (e: KeyboardEvent) => {
      if (e.code !== "Space" || !over.current || isTextEntry(e.target)) return;
      e.preventDefault();
      if (!heldNow.current) set(true);
    };
    const up = (e: KeyboardEvent) => {
      if (e.code !== "Space" || !heldNow.current) return;
      e.preventDefault();
      set(false);
    };
    // A keyup that lands in another window never arrives.
    const lost = () => {
      if (heldNow.current) set(false);
    };
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("blur", lost);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("blur", lost);
    };
  }, []);

  return {
    held,
    /** Whether this press pans: the middle button, or the left with Space held. */
    pans: (e: React.PointerEvent) => e.button === 1 || (e.button === 0 && heldNow.current),
    /** For the overlay's outer element, so Space counts only over it. */
    hover: {
      onPointerEnter: () => {
        over.current = true;
      },
      onPointerLeave: () => {
        over.current = false;
      },
    },
  };
}
