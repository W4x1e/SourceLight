import type { BatchOutcome, ExportOptions } from "../bindings";
import { api } from "./api";
import { toast } from "./dialogs";
import { batchSucceeded, batchSummary } from "./export";
import { useStore } from "../store";
import { t } from "../i18n";

/** A batch export, kept with its job so the failed ones can run again. */
export interface BatchRequest {
  ids: number[];
  /** Each photo's filename, in the same order, for the queue's rows. */
  files: string[];
  directory: string;
  options: ExportOptions;
  rename: string | null;
  subfolder: string | null;
  /** Send to phone: the folder is the outbox's, not one to show or retry into. */
  toPhone?: boolean;
}

/**
 * Runs a batch export through the queue panel: its rows, each photo's result
 * as it lands, and the summary. One batch at a time: a second one used
 * to reset the first's Cancel and drive the same queue.
 */
export async function runBatch(req: BatchRequest): Promise<void> {
  const st = useStore.getState();
  if (st.exportJob?.running) {
    toast(t("transfer.queue.busy"), "error");
    return;
  }
  st.startExportJob(req);
  try {
    const outcome = await api.exportBatch(req.ids, req.directory, req.options, req.rename, req.subfolder);
    const summary = batchSummary(outcome, req.files.length);
    useStore.getState().finishExportJob(summary, outcome);
    toast(summary, batchSucceeded(outcome) ? "success" : "error");
  } catch (e) {
    useStore.getState().finishExportJob(t("transfer.export.failed", { error: String(e) }));
  }
}

/**
 * Send to phone (docs/MOBILE.md): the same batch export, into a folder in the
 * outbox, then put on the list the paired phone fetches from when it next
 * syncs. The queue panel shows it like any export.
 */
export async function sendToPhone(req: Omit<BatchRequest, "directory" | "subfolder" | "toPhone">): Promise<void> {
  const st = useStore.getState();
  if (st.exportJob?.running) {
    toast(t("transfer.queue.busy"), "error");
    return;
  }
  let directory: string;
  try {
    directory = await api.phoneOutboxStart();
  } catch (e) {
    toast(t("transfer.export.failed", { error: String(e) }), "error");
    return;
  }
  const full: BatchRequest = { ...req, directory, subfolder: null, toPhone: true };
  st.startExportJob(full);
  let outcome: BatchOutcome;
  try {
    outcome = await api.exportBatch(full.ids, directory, full.options, full.rename, null);
  } catch (e) {
    useStore.getState().finishExportJob(t("transfer.export.failed", { error: String(e) }));
    return;
  }
  const summary = batchSummary(outcome, full.files.length);
  useStore.getState().finishExportJob(summary, outcome);
  try {
    const queued = await api.phoneOutboxQueue(directory);
    if (queued > 0) toast(t("transfer.phone.sent", { count: queued }), batchSucceeded(outcome) ? "success" : "error");
    else toast(summary, "error");
  } catch (e) {
    toast(t("transfer.export.failed", { error: String(e) }), "error");
  }
}

/**
 * The failed photos of a finished batch, again, into the folder the rest went
 * to. Null when they can't be: numbered names count from the batch's first
 * photo, so a retry of three would be numbered 001–003 beside the 001 already
 * written.
 */
export function retryRequest(): BatchRequest | null {
  const job = useStore.getState().exportJob;
  if (!job || job.running || !job.directory || job.request.toPhone) return null;
  if (job.request.rename?.includes("{seq}")) return null;
  const failed = job.request.ids.filter((id) => typeof job.results[id] === "string");
  if (failed.length === 0) return null;
  const files = new Map(job.request.ids.map((id, i) => [id, job.request.files[i]]));
  return {
    ...job.request,
    ids: failed,
    files: failed.map((id) => files.get(id) ?? String(id)),
    // The subfolder is already part of the folder the batch reported.
    directory: job.directory,
    subfolder: null,
  };
}
