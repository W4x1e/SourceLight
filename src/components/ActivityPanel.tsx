import { useCallback, useRef, useState } from "react";
import ProgressBar from "./ProgressBar";
import { formatNumber, inSentence, t, tx, type Key } from "../i18n";
import { ANALYSIS_JOB, useStore, type AnalysisStep } from "../store";
import { Icon } from "../lib/icons";
import { confirm, toast } from "../lib/dialogs";
import { useModal } from "../lib/useEscape";
import { usePresence } from "../lib/usePresence";
import { humanBytes } from "../lib/format";
import { batchSucceeded, sidecarReadSummary, sidecarSummary } from "../lib/export";
import { canCancel, cancelJob, isDownload } from "../lib/jobs";
import { secondsLeft, timeLeftText, type Sample } from "../lib/timeLeft";
import { api } from "../lib/api";

/**
 * The Activity centre: one slide-over for the heavy, long-running library jobs
 * that used to sit at equal weight next to Search in the top bar — Scan,
 * tagging, and XMP sidecar maintenance — plus a readout of the running export.
 *
 * Batch-export progress keeps its own floating queue (it must stay visible over
 * the Develop overlay, which this panel sits behind), so here it is a summary
 * line, not the full list.
 */
/** What each automatic-analysis step is called while it runs. */
/** The background job each step reports its progress under. */
const STEP_JOB = ANALYSIS_JOB;

/**
 * The running step, with its progress right here. It used to say "(progress
 * below)" and rely on the Background list, which only gains a row when the
 * first photo is done. The model loads before that, so the space below stayed
 * empty for as long as the load took.
 */
function AnalysisProgress({
  step,
  job: batch,
  run,
}: {
  step: AnalysisStep;
  job: { current: number; total: number } | undefined;
  run: { done: number; total: number } | null;
}) {
  if (step === "tags") {
    return <div className="mt-3 text-caption text-muted">{t("settings.activity.stepRunning", { step: t(STEP_LABEL[step]) })}</div>;
  }
  // Between two batches of descriptions the bar stays where the run got to.
  const job =
    !batch && step === "descriptions" && run && run.done > 0 ? { current: run.done, total: run.total } : batch;
  if (!job || job.total <= 1) {
    const loading = step === "search" || step === "faces";
    return (
      <div className="mt-3 text-caption text-muted">
        {tx("settings.activity.stepStatus", {
          step: t(STEP_LABEL[step]),
          status: (
            <span className="text-faint">
              {loading ? t("settings.activity.loadingTheModel") : t("settings.activity.starting")}
            </span>
          ),
        })}
      </div>
    );
  }
  const id = STEP_JOB[step];
  return (
    <Progress
      label={t(STEP_LABEL[step])}
      current={job.current}
      total={job.total}
      // Stops the run, not just this step: the next step started at
      // once, so it took three or four Stops. The Background list leaves this
      // job out rather than drawing the same bar twice.
      onStop={canCancel(id) ? () => void useStore.getState().stopAnalysis() : undefined}
    />
  );
}

const STEP_LABEL: Record<AnalysisStep, Key> = {
  colours: "settings.activity.step.colours",
  locations: "settings.activity.step.locations",
  focus: "settings.activity.step.focus",
  palette: "settings.activity.step.palette",
  search: "settings.activity.step.search",
  tags: "settings.activity.step.tags",
  faces: "settings.activity.step.faces",
  descriptions: "settings.activity.step.descriptions",
};

function Progress({
  label,
  current,
  total,
  onStop,
}: {
  label: string;
  current: number;
  total: number;
  onStop?: () => void;
}) {
  const pct = total ? Math.round((current / total) * 100) : 0;
  // The pace since this bar first showed (`lib/timeLeft`): a first run on a
  // big library takes minutes on one computer and hours on another, and the
  // count alone does not say which. A run that starts again measures again.
  const first = useRef<Sample | null>(null);
  if (!first.current || current < first.current.done) first.current = { at: Date.now(), done: current };
  const left = secondsLeft(first.current, { at: Date.now(), done: current }, total);
  return (
    <div className="mt-2">
      <div className="mb-1 flex items-center justify-between gap-2 text-caption text-muted">
        <span className="truncate">{label}</span>
        <span className="ml-auto shrink-0 tabular-nums">
          {t("common.progress", { current, total })}
        </span>
        {onStop && (
          <button
            onClick={onStop}
            className="shrink-0 text-label text-faint underline hover:text-ink"
            title={t("settings.activity.stopTitle")}
          >
            {t("common.stop")}
          </button>
        )}
      </div>
      <ProgressBar pct={pct} label={label} />
      {left !== null && <div className="mt-1 text-caption text-faint tabular-nums">{timeLeftText(left)}</div>}
    </div>
  );
}

export default function ActivityPanel() {
  const show = useStore((s) => s.showActivity);
  const setShow = useStore((s) => s.setShowActivity);
  // The list object, not `?.folders ?? []`: a fresh [] per read is a new
  // snapshot every time, and zustand re-renders until React gives up.
  const libraryFolders = useStore((s) => s.libraryFolders);
  const folders = libraryFolders?.folders ?? [];
  // The library's count: the grid's list is whatever filter or search is on.
  const photoCount = useStore((s) => s.facets?.total ?? s.photos.length);
  const hasPhotos = photoCount > 0;
  const untagged = useStore((s) => s.facets?.untagged ?? 0);
  const ai = useStore((s) => s.ai);
  const scanning = useStore((s) => s.scanning);
  const tagging = useStore((s) => s.tagging);
  const exportJob = useStore((s) => s.exportJob);
  const jobs = useStore((s) => s.jobs);
  const describeRun = useStore((s) => s.describeRun);

  const runScan = useStore((s) => s.runScan);
  const autoAnalyse = useStore((s) => s.autoAnalyse);
  const setAutoAnalyse = useStore((s) => s.setAutoAnalyse);
  const autoAnalysis = useStore((s) => s.autoAnalysis);
  const analyseNewPhotos = useStore((s) => s.analyseNewPhotos);
  const heldForPower = useStore((s) => s.heldForPower);
  const retagAll = useStore((s) => s.retagAll);
  const cancelTagging = useStore((s) => s.cancelTagging);
  const importXmp = useStore((s) => s.importXmp);
  const exportXmp = useStore((s) => s.exportXmp);
  // While "Import all" counts, asks and reads, so it can't be started twice.
  const [reading, setReading] = useState(false);

  const panel = useRef<HTMLElement>(null);
  useModal(show, panel, useCallback(() => setShow(false), [setShow]));

  // Background lists every job except the one automatic analysis is running,
  // which the Analysis section already shows with its progress bar and Stop.
  const analysisJob = autoAnalysis ? STEP_JOB[autoAnalysis] : null;
  const jobIds = Object.keys(jobs).filter((id) => id !== analysisJob);
  const { mounted, closing } = usePresence(show);
  if (!mounted) return null;

  const busy = scanning !== null || tagging !== null;
  // One sidecar run at a time: the backend refuses a second too.
  const sidecarsBusy = "sidecars" in jobs;
  const dotColor =
    ai?.provider === "none" ? "bg-faint" : ai?.ready ? "bg-positive" : "bg-warning";

  // Reading sidecars overwrites ratings, labels, picks, titles and rights
  // across the library, so it asks first, saying how many and what.
  // The backend backs the catalogue up and makes the verdicts one undo.
  const importAll = async () => {
    setReading(true);
    try {
      const n = await api.countSidecars([]);
      if (n === 0) {
        toast(t("transfer.summary.noSidecars"), "info");
        return;
      }
      const ok = await confirm({
        title: t("settings.activity.readSidecars.title", { count: n }),
        message: t("settings.activity.readSidecars.message"),
        confirmLabel: t("settings.activity.readSidecars.confirm"),
        danger: true,
      });
      if (!ok) return;
      const read = await importXmp([]);
      toast(sidecarReadSummary(read), read.read > 0 ? "success" : "info");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setReading(false);
    }
  };
  const exportAll = async () => {
    if (!(await confirm({
      title: t("settings.activity.writeSidecars.title"),
      message: t("settings.activity.writeSidecars.message"),
      confirmLabel: t("settings.activity.writeSidecars.confirm"),
    }))) return;
    try {
      const outcome = await exportXmp([]);
      toast(sidecarSummary(outcome), batchSucceeded(outcome) ? "success" : "error");
    } catch (e) {
      toast(String(e), "error");
    }
  };

  return (
    // The scrim is this element, not a child of it. As a child it sat *over*
    // the container and intercepted every click outside the aside, so
    // `e.target === e.currentTarget` never held and clicking away did nothing —
    // the panel could only be left by aiming at the ×. SettingsPanel and the
    // dialogs have always had it this way round.
    <div
      className="anim-scrim fixed inset-0 z-40 flex justify-end bg-scrim-veil"
      data-closing={closing || undefined}
      onClick={(e) => e.target === e.currentTarget && setShow(false)}
    >
      <aside
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby="activity-title"
        className="anim-panel relative flex h-full w-[360px] flex-col border-l border-line bg-panel shadow-2xl"
      >
        <div className="flex items-center gap-2 border-b border-line px-4 py-3">
          <Icon name="activity" size={16} className="text-accent" />
          <h2 id="activity-title" className="flex-1 font-display text-display text-ink">{t("settings.activity.title")}</h2>
          <button onClick={() => setShow(false)} className="icon-chip" title={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto">
          {/* Library / scan */}
          <section className="border-b border-line/60 px-4 py-4">
            <div className="section-label px-0 pb-2">{t("settings.activity.library")}</div>
            <div
              className="mb-2 truncate text-caption text-faint"
              title={folders.map((f) => f.path).join("\n")}
            >
              {folders.length === 0
                ? t("settings.activity.noFolders")
                : folders.length === 1
                  ? folders[0]?.path
                  : t("settings.activity.folderCount", { count: folders.length })}
            </div>
            {/* The number is the readout (mono); the words are not. */}
            <div className="mb-3 text-caption text-muted">
              {tx(
                "settings.activity.photosIndexed",
                { count: <span className="tabular-nums">{formatNumber(photoCount)}</span> },
                photoCount,
              )}
            </div>
            <button
              onClick={() => void runScan()}
              className="btn-solid w-full justify-center"
              disabled={busy || folders.length === 0}
            >
              <Icon name="import" size={15} />
              {scanning
                ? t("settings.activity.scanning", { current: scanning.current, total: scanning.total })
                : t("settings.activity.scan")}
            </button>
            {scanning && <Progress label={t("settings.activity.scanningLabel")} current={scanning.current} total={scanning.total} />}

          </section>

          {/* Analysis. It used to be a Tag button here, an Index button
              beside Scan, and more in Settings and Cull — and none of them ran
              by themselves, so this week's shoot stayed unsearchable until
              someone remembered which to press. Now the passes run after every
              scan that adds photos; this is where that is switched and seen. */}
          <section className="border-b border-line/60 px-4 py-4">
            <div className="section-label flex items-center gap-2 px-0 pb-2">
              <span>{t("settings.activity.analysis")}</span>
              <span className={`h-1.5 w-1.5 rounded-full ${dotColor}`} />
            </div>
            <div className="flex items-start gap-3">
              <p className="min-w-0 flex-1 text-caption leading-relaxed text-faint">
                {t("settings.activity.analysisBlurb")}
              </p>
              <div className="seg-group shrink-0" role="radiogroup" aria-label={t("settings.activity.autoAnalyse")}>
                {([false, true] as const).map((v) => (
                  <button
                    key={String(v)}
                    role="radio"
                    aria-checked={autoAnalyse === v}
                    onClick={() => setAutoAnalyse(v)}
                    className={`px-2.5 py-1 text-caption ${autoAnalyse === v ? "seg-on" : "text-muted hover:bg-hover"}`}
                  >
                    {v ? t("common.on") : t("common.off")}
                  </button>
                ))}
              </div>
            </div>

            {autoAnalysis ? (
              <>
                <AnalysisProgress step={autoAnalysis} job={jobs[STEP_JOB[autoAnalysis]]} run={describeRun} />
                {/* While a model loads there is no bar to stop from. */}
                {!(jobs[STEP_JOB[autoAnalysis]] && canCancel(STEP_JOB[autoAnalysis])) && (
                  <button
                    onClick={() => void useStore.getState().stopAnalysis()}
                    className="mt-1 text-label text-faint underline hover:text-ink"
                    title={t("settings.activity.stopTitle")}
                  >
                    {t("common.stop")}
                  </button>
                )}
              </>
            ) : (
              <button
                onClick={() => void analyseNewPhotos(true)}
                className="btn-solid mt-3 w-full justify-center"
                disabled={busy || !hasPhotos}
                title={t("settings.activity.analyseNowTitle")}
              >
                {t("settings.activity.analyseNow")}
              </button>
            )}
            {heldForPower && !autoAnalysis && (
              <p className="mt-2 text-caption leading-relaxed text-faint">
                {t("settings.activity.heldForPower")}
              </p>
            )}

            {tagging && (
              <>
                <button onClick={cancelTagging} className="btn-ghost mt-2 w-full justify-center">
                  {/* The model load happens before photo 1, and used to look
                      like a hang at 0/N. Name it while it happens. */}
                  {tagging.phase === "loading"
                    ? t("settings.activity.loadingModel")
                    : t("settings.activity.stopTagging", { current: tagging.current, total: tagging.total })}
                </button>
                {tagging.phase !== "loading" && (
                  <Progress label={t("settings.activity.taggingLabel")} current={tagging.current} total={tagging.total} />
                )}
              </>
            )}

            {/* The one explicit re-run: automatic analysis only ever fills
                gaps, it never redoes a photo. */}
            {!tagging && untagged === 0 && hasPhotos && (
              <button
                onClick={() => void retagAll()}
                className="btn-ghost mt-2 w-full justify-center text-caption"
                disabled={busy}
                title={t("settings.activity.retagTitle")}
              >
                {t("settings.activity.retag")}
              </button>
            )}
          </section>

          {/* XMP sidecars */}
          <section className="border-b border-line/60 px-4 py-4">
            <div className="section-label px-0 pb-2">{t("settings.activity.sidecars")}</div>
            <p className="mb-3 text-caption leading-relaxed text-faint">
              {t("settings.activity.sidecarsBlurb")}
            </p>
            <div className="flex gap-2">
              <button onClick={() => void importAll()} className="btn-solid flex-1 justify-center" disabled={busy || reading || sidecarsBusy || !hasPhotos}>
                {t("settings.activity.importAll")}
              </button>
              <button onClick={() => void exportAll()} className="btn-solid flex-1 justify-center" disabled={busy || sidecarsBusy || !hasPhotos}>
                {t("settings.activity.exportAll")}
              </button>
            </div>
          </section>

          {/* Background jobs — model downloads, captioning, indexing, faces,
              focus scoring. These run from panels that unmount (Settings, Cull),
              so without a home here a multi-gigabyte download had nowhere to
              show itself once you closed the panel that started it. */}
          {jobIds.length > 0 && (
            <section className="border-b border-line/60 px-4 py-4">
              <div className="section-label px-0 pb-2">{t("settings.activity.background")}</div>
              {jobIds.map((id) => {
                const j = jobs[id]!;
                const pct = j.total ? Math.round((j.current / j.total) * 100) : 0;
                if (j.error) {
                  // A download that failed stays, saying why: the row
                  // used to vanish, and the reason with it.
                  return (
                    <div key={id} className="mt-2 first:mt-0">
                      <div className="flex items-center justify-between gap-2 text-caption text-muted">
                        <span className="truncate">{j.label}</span>
                        <button
                          onClick={() => useStore.getState().setJob(id, null)}
                          className="ml-auto shrink-0 text-label text-faint underline hover:text-ink"
                        >
                          {t("transfer.queue.dismiss")}
                        </button>
                      </div>
                      <p className="text-label text-danger-ink">{j.error}</p>
                    </div>
                  );
                }
                return (
                  <div key={id} className="mt-2 first:mt-0">
                    <div className="mb-1 flex items-center justify-between gap-2 text-caption text-muted">
                      <span className="truncate">{j.label}</span>
                      <span className="ml-auto shrink-0 tabular-nums text-faint">
                        {j.total <= 1
                          ? t("common.starting")
                          : j.unit === "bytes"
                            ? t("settings.activity.bytesOf", { current: humanBytes(j.current), total: humanBytes(j.total) })
                            : t("common.progress", { current: j.current, total: j.total })}
                      </span>
                      {/* A multi-gigabyte pull started by mistake, or on a
                          metered connection, must be stoppable where it shows.
                          What already finished is kept. */}
                      {canCancel(id) && (
                        <button
                          onClick={() => void cancelJob(id)}
                          className="shrink-0 text-label text-faint underline hover:text-ink"
                          title={t(isDownload(id) ? "settings.activity.cancelDownloadTitle" : "settings.activity.cancelTitle", {
                            job: inSentence(j.label),
                          })}
                        >
                          {t("common.cancel")}
                        </button>
                      )}
                    </div>
                    <ProgressBar pct={pct} label={j.label} />
                  </div>
                );
              })}
            </section>
          )}

          {/* Exports */}
          {exportJob && (
            <section className="px-4 py-4">
              <div className="section-label px-0 pb-2">{t("settings.activity.export")}</div>
              <div className="text-caption text-muted">
                {exportJob.running
                  ? t("settings.activity.exporting", { done: exportJob.done, total: exportJob.request.files.length })
                  : (exportJob.summary ?? t("common.done"))}
              </div>
              <p className="mt-1 text-caption text-faint">{t("settings.activity.exportNote")}</p>
            </section>
          )}
        </div>
      </aside>
    </div>
  );
}
