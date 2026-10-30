import { api } from "./api";

/**
 * Cancelling long jobs from wherever their progress is shown.
 *
 * The ids are the ones the store already tracks progress under (`App.tsx`), so
 * the Activity panel can offer Cancel on exactly the rows that support it.
 */

/** What a cancelled job's error reads as — `jobs::CANCELLED` on the Rust side. */
export const CANCELLED = "Cancelled";

/** Whether an error, or a progress event's `error`, is a cancel the user chose
 *  rather than a failure — so it is not shown in red. */
export function isCancelled(e: unknown): boolean {
  const text = e instanceof Error ? e.message : String(e ?? "");
  return text === CANCELLED || text.endsWith(`: ${CANCELLED}`);
}

/** The progress rows that can be stopped: model downloads, the local AI
 *  runtime, a vision-model pull, search indexing, face analysis, people
 *  grouping, captioning, a card import, AI noise reduction, magic erase,
 *  reading or writing sidecars, reading colours. */
export function canCancel(id: string): boolean {
  return /^(model:.+|vision-runtime|vlm-model|index|faces|people|caption|import|locations|denoise|erase|sidecars|palette)$/.test(id);
}

/** Whether a job is a download — a model, the local AI runtime, the vision
 *  model — which a cancel or a quit starts again from the beginning. */
export function isDownload(id: string): boolean {
  return /^(model:.+|vision-runtime|vlm-model)$/.test(id);
}

/** Stops the job behind a progress row. Captioning predates the shared
 *  registry and keeps its own command. */
export async function cancelJob(id: string): Promise<void> {
  if (id === "caption") await api.cancelCaptioning();
  else await api.cancelJob(id);
}
