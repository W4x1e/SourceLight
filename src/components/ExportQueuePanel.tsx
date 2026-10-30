import { useStore } from "../store";
import ProgressBar from "./ProgressBar";
import { Icon } from "../lib/icons";
import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { retryRequest, runBatch } from "../lib/batchExport";
import { FILE_MANAGER } from "../lib/platform";
import { t } from "../i18n";

/**
 * The export queue: a floating panel showing the running (or just-finished)
 * batch export, file by file. It reads from the global store rather than the
 * Develop dialog, so it keeps showing progress after the dialog is closed.
 *
 * Above Develop: a batch started from Develop's export dialog used to
 * run beneath it, with no progress and no Cancel until Develop closed.
 */
export default function ExportQueuePanel() {
  const job = useStore((s) => s.exportJob);
  const cancel = useStore((s) => s.cancelExportJob);
  const dismiss = useStore((s) => s.dismissExportJob);

  if (!job) return null;

  const total = job.request.files.length;
  const pct = total ? Math.round((job.done / total) * 100) : 0;
  const retry = retryRequest();

  return (
    <div className="queue-in fixed bottom-3 right-3 z-[51] w-[320px] overflow-hidden rounded-lg border border-line bg-panel shadow-2xl">
      <div className="flex items-center gap-2 border-b border-line px-3 py-2">
        <Icon name="layers" size={14} className="text-muted" />
        <span className="flex-1 text-caption font-semibold">{t("transfer.queue.title")}</span>
        <span className="text-caption tabular-nums text-muted">
          {t("common.progress", { current: job.done, total })}
        </span>
      </div>

      <div className="max-h-56 overflow-y-auto p-1.5">
        {job.request.files.map((f, i) => {
          // Ticked by what happened to the file, not by its place in the
          // queue: a failed file was ticked green.
          const id = job.request.ids[i];
          const result = id === undefined ? undefined : job.results[id];
          const state =
            result === null
              ? "done"
              : typeof result === "string"
                ? "failed"
                : i === job.done && job.running
                  ? "current"
                  : "pending";
          return (
            <div key={i} className="flex items-start gap-2 px-2 py-1 text-caption">
              <span className="mt-0.5 grid h-3.5 w-3.5 shrink-0 place-items-center">
                {state === "done" ? (
                  <Icon name="check" size={12} strokeWidth={3} className="text-positive" />
                ) : state === "failed" ? (
                  <Icon name="close" size={12} strokeWidth={3} className="text-danger-ink" />
                ) : state === "current" ? (
                  <span className="h-1.5 w-1.5 rounded-full bg-accent" />
                ) : null}
              </span>
              <span className="min-w-0 flex-1">
                <span
                  className={`block truncate ${
                    state === "pending" ? "text-faint" : state === "done" ? "text-muted" : "text-ink"
                  }`}
                >
                  {f}
                </span>
                {state === "failed" && (
                  <span className="block break-words text-label text-danger-ink">{result}</span>
                )}
              </span>
            </div>
          );
        })}
      </div>

      {job.running && (
        <ProgressBar
          pct={pct}
          label={t("transfer.queue.title")}
          className="mx-3 mb-2 h-1 overflow-hidden rounded-full bg-base"
          barClassName="h-full bg-accent transition-[width]"
        />
      )}

      {/* Its own line once finished: beside three buttons it read "Exported 3, 1…". */}
      {!job.running && job.summary && (
        <p className="break-words border-t border-line px-3 pt-2 text-caption text-muted">{job.summary}</p>
      )}
      {/* Wraps in German (Mac test): "Ausblenden" ran off the edge. */}
      <div className={`flex flex-wrap items-center justify-end gap-2 px-3 py-2 ${job.running || !job.summary ? "border-t border-line" : ""}`}>
        <span className="min-w-0 flex-1 truncate text-caption text-faint">{job.running ? job.message : ""}</span>
        {job.running ? (
          <button onClick={() => void cancel()} className="btn-solid text-caption">
            {t("common.cancel")}
          </button>
        ) : (
          <>
            {retry && (
              <button onClick={() => void runBatch(retry)} className="btn-solid text-caption">
                {t("transfer.queue.retryFailed")}
              </button>
            )}
            {/* Named, as the single export's toast names it: a bare folder
                icon was the one way to the files and easy to miss. */}
            {job.directory && !job.request.toPhone && (
              <button
                onClick={() =>
                  void api.showFolder(job.directory!).catch((e) => toast(String(e), "error"))
                }
                className="btn-ghost gap-1.5 text-caption"
                title={t("transfer.queue.showFolder")}
              >
                <Icon name="folder" size={14} />
                {t("common.showInFileManager", { fileManager: FILE_MANAGER })}
              </button>
            )}
            <button onClick={dismiss} className="btn-ghost text-caption">
              {t("transfer.queue.dismiss")}
            </button>
          </>
        )}
      </div>
    </div>
  );
}
