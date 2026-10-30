import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";
import type { EditStack } from "../bindings";

/**
 * Hands the edit being dragged to the backend, so the `develop://` preview can
 * draw it before the release commits it, and returns the token the
 * preview URL carries to ask for it. `null` when there is no draft — at rest,
 * or after a commit — and the preview shows the committed frame.
 *
 * Without this the readback preview, Windows' default, only followed the
 * committed edit: a slider redrew the photo on release, so every adjustment
 * was drag, release, look, repeat.
 *
 * One hand-over in flight at a time, always heading for the newest draft, as
 * `useLatestFrame` does for the image: a drag sends one per render the
 * backend can keep up with, not one per pointer move.
 */
export function useDraftToken(draft: EditStack | null): number | null {
  const [token, setToken] = useState<number | null>(null);
  const latest = useRef(draft);
  latest.current = draft;
  const sent = useRef<EditStack | null>(null);
  const busy = useRef(false);

  const pump = useCallback(() => {
    if (busy.current || latest.current === sent.current) return;
    const next = latest.current;
    sent.current = next;
    if (!next) {
      setToken(null);
      return;
    }
    busy.current = true;
    void api
      .setDevelopDraft(next)
      .then(setToken)
      // A failed hand-over leaves the committed frame showing, as before.
      .catch(() => setToken(null))
      .finally(() => {
        busy.current = false;
        pump();
      });
  }, []);

  // Every render may carry a new draft; whether to send it is runtime state.
  useEffect(pump);

  return draft ? token : null;
}
