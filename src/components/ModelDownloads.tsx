import { useStore } from "../store";
import ProgressBar from "./ProgressBar";
import { cancelJob, isDownload } from "../lib/jobs";
import { humanBytes } from "../lib/format";
import { t } from "../i18n";

/**
 * The AI downloads in progress, with Cancel, shown where the feature that
 * started them was clicked. Develop sits over the Activity panel, so a
 * first Select subject or Reduce noise pointed to progress no one could open,
 * and the button looked idle for the minutes a model took to arrive.
 */
/** Whether a model is downloading now — when ModelDownloads has something to show. */
export function useDownloading(): boolean {
  return useStore((s) => Object.keys(s.jobs).some((id) => isDownload(id) && !s.jobs[id]!.error));
}

export default function ModelDownloads({ className = "" }: { className?: string }) {
  const jobs = useStore((s) => s.jobs);
  const ids = Object.keys(jobs).filter((id) => isDownload(id) && !jobs[id]!.error);
  if (ids.length === 0) return null;
  return (
    <div className={`min-w-0 ${className}`} role="status">
      {ids.map((id) => {
        const j = jobs[id]!;
        const pct = j.total ? Math.min(100, Math.round((j.current / j.total) * 100)) : 0;
        return (
          <div key={id} className="mt-1 first:mt-0">
            {/* The byte count under the bar, not beside the label: in the masks
                panel, three fixed-width pieces on one line pushed the panel wider. */}
            <div className="mb-1 flex min-w-0 items-center gap-2 text-label text-muted">
              <span className="min-w-0 flex-1 truncate">{j.label}</span>
              <button
                onClick={() => void cancelJob(id)}
                className="shrink-0 text-accent hover:underline"
                title={t("settings.activity.cancelDownloadTitle", { job: j.label })}
              >
                {t("common.cancel")}
              </button>
            </div>
            <ProgressBar pct={pct} label={j.label} />
            <div className="mt-0.5 truncate text-label tabular-nums text-faint">
              {j.total > 1
                ? t("settings.activity.bytesOf", { current: humanBytes(j.current), total: humanBytes(j.total) })
                : t("common.starting")}
            </div>
          </div>
        );
      })}
    </div>
  );
}
