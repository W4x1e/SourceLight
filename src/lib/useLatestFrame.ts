import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Keeps at most one preview frame in flight, always heading for the newest one.
 *
 * **Without this the app locks up during a drag.** Pointer moves arrive at the
 * mouse's polling rate — commonly 125/s, often far more — and each distinct
 * preview URL starts a render costing ~13ms of GPU readback and five megabytes
 * in release, ~110ms in a debug build. Asking for one per event queues work an
 * order of magnitude faster than it can be done.
 *
 * And the queue cannot be drained by giving up: when the browser abandons an
 * `<img>` whose `src` changed, the custom-scheme handler in Rust still runs to
 * completion, and it holds the develop lock while it does. So the backlog
 * blocks every other command too, which is why the whole window stops
 * responding rather than just the photo.
 *
 * The rule here is therefore *completion-driven*, not time-driven: no timer, no
 * debounce interval to tune. One frame is out at a time, the latest request is
 * remembered, and it is issued the moment the previous one lands. That adapts
 * on its own to a fast machine, a slow one, and a debug build.
 */
export function useLatestFrame(target: string): [string, () => void] {
  const [shown, setShown] = useState(target);
  const shownRef = useRef(target);
  const targetRef = useRef(target);
  // The first frame is in flight from the start: it is shown at once, so
  // nothing issues it, and counting it idle let the next request start beside
  // it — two renders at once, the thing this exists to prevent.
  const busy = useRef(target !== "");
  targetRef.current = target;

  // **Both callbacks must be referentially stable.** They read only refs and
  // `setShown`, so there is nothing to close over — and an identity that
  // changed every render would break any caller that lists `settled` in a
  // dependency array, re-running their effect and cancelling the very frame it
  // is waiting on. That deadlocks at the first frame: measured as a burst of
  // 200 updates settling on update 1.
  const flush = useCallback(() => {
    if (busy.current || targetRef.current === shownRef.current) return;
    busy.current = true;
    shownRef.current = targetRef.current;
    setShown(targetRef.current);
  }, []);

  // Deliberately no dependency array: every render is a chance that the target
  // moved, and whether it is issued depends on runtime state rather than on
  // anything React can compare.
  useEffect(flush);

  /**
   * Call when the frame finishes — on **error as well as load**. An unreported
   * failure would leave `busy` set forever and freeze the preview far more
   * thoroughly than the problem this solves.
   */
  const settled = useCallback(() => {
    busy.current = false;
    flush();
  }, [flush]);

  return [shown, settled];
}
