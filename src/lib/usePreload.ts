import { useEffect } from "react";

/**
 * Fetches and decodes these images ahead of need: the loupe's next and
 * previous frames, so a step shows the new frame at once instead of the old
 * one's pixels under the new one's verdict chips while its preview loads.
 * The webview keeps a fetched image, and `decode()` holds it until decoded.
 * A frame that can't be read is simply not ready early.
 */
export function usePreload(urls: readonly (string | undefined)[]) {
  const key = urls.join("\n");
  useEffect(() => {
    for (const url of key.split("\n")) {
      if (!url) continue;
      const img = new Image();
      img.src = url;
      // Not every engine has decode() (a test's DOM hasn't); the fetch alone
      // still warms the cache.
      img.decode?.().catch(() => undefined);
    }
  }, [key]);
}
