import { useEffect, useState } from "react";
import { api } from "./api";
import type { Photo } from "../types";

/** Renders asked for, by row: a fresh row (after an edit) asks again. */
const asked = new WeakMap<Photo, Promise<string | null>>();
/** One render at a time: each is an export on the GPU. */
let queue: Promise<unknown> = Promise.resolve();

function render(p: Photo): Promise<string | null> {
  let path = asked.get(p);
  if (!path) {
    // A photo that will not render shows its camera preview, as before.
    path = queue.then(() => api.renderEditedPreview(p.id)).catch(() => null);
    queue = path;
    asked.set(p, path);
  }
  return path;
}

/**
 * The photo as edited, to show in place of the camera's preview: the render
 * kept on its row, or one asked for now when its edits have none yet. Null
 * for an unedited photo, and until the render is there.
 */
export function useEditedPath(p: Photo | undefined): string | null {
  const [rendered, setRendered] = useState<{ photo: Photo; path: string | null } | null>(null);
  useEffect(() => {
    if (!p?.edited || p.editedPath) return;
    let live = true;
    void render(p).then((path) => live && setRendered({ photo: p, path }));
    return () => {
      live = false;
    };
  }, [p]);
  if (!p?.edited) return null;
  return p.editedPath ?? (rendered?.photo === p ? rendered.path : null);
}
