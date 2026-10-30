import { useEffect, useRef, useState } from "react";
import ProgressBar from "../ProgressBar";
import ModelDownloads, { useDownloading } from "../ModelDownloads";
import { ensureAi } from "../../lib/aiReady";
import { api } from "../../lib/api";
import { isCancelled } from "../../lib/jobs";
import { estimateFor, formatDuration, paceOf, readPace, secondsLeft, writePace } from "../../lib/denoiseTime";
import { useStore } from "../../store";
import { t } from "../../i18n";
import NumberSlider from "../NumberSlider";

/** The Amount a first run applies: all of it. The half-resolution method keeps
 *  the fine grain by itself (see `denoise.rs`); the Amount takes it down. */
export const DEFAULT_AI_DENOISE = 1;

interface Props {
  /** The Amount in the draft stack, 0 when off. */
  amount: number;
  /** Whether the model has run on this photo this session. */
  ready: boolean;
  /** The photo's size, for the time estimate. */
  width: number;
  height: number;
  /** The centre of the view in framed UV when zoomed in, where the model
   *  starts; `null` at fit, where it starts from the middle. */
  focus: { x: number; y: number } | null;
  onChange: (v: number) => void;
  onCommit: () => void;
}

/**
 * AI noise reduction in Develop's Detail panel.
 *
 * The model is slow, and how slow depends on the computer far more than on
 * the photo (seconds on a fast graphics card, minutes on integrated graphics),
 * so it is an action rather than a live slider, and the panel says how long it
 * will take here: from the last run's pace before it starts, and from the pace
 * so far while it runs. It starts where you are looking, and a long run shows
 * its result as it goes (Develop draws those; see `liveFrame` there). It runs
 * once per photo; after that the Amount mixes its result back in instantly. An
 * edit reopened with an Amount shows the photo undenoised until it is run
 * again (export runs it itself), so the panel says so and offers to.
 *
 * Progress is the store's `denoise` job, not this panel's own state: the
 * Detail panel can be closed and reopened during a run, and the run carries on.
 */
export default function AiDenoise({ amount, ready, width, height, focus, onChange, onCommit }: Props) {
  const setDevelopInfo = useStore((s) => s.setDevelopInfo);
  const job = useStore((s) => s.jobs["denoise"]);
  const downloading = useDownloading();
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [left, setLeft] = useState<number | null>(null);
  // When the first tile finished, for the pace so far.
  const firstTile = useRef<{ at: number; done: number } | null>(null);

  // A job with more than one step is running tiles; before that it is loading.
  const tiles = job && job.total > 1 ? job : null;
  useEffect(() => {
    if (!tiles) {
      firstTile.current = null;
      setLeft(null);
      return;
    }
    const now = { at: Date.now(), done: tiles.current };
    if (!firstTile.current) firstTile.current = now;
    setLeft(secondsLeft(firstTile.current, now, tiles.total));
  }, [tiles?.current, tiles?.total]);

  /** Runs the model; `setAmount` makes the Amount a step in history. */
  const run = async (setAmount: number | null) => {
    if (starting) return;
    // Starting from the click: a first run downloads the model first,
    // shown below with Cancel.
    setStarting(true);
    setError(null);
    const photo = useStore.getState().develop?.photoId;
    try {
      if (!(await ensureAi("denoise", { inline: true }))) return;
      // Another photo opened during the download: not this one's to denoise.
      if (useStore.getState().develop?.photoId !== photo) return;
      const startedAt = Date.now();
      setDevelopInfo(await api.developDenoise(setAmount, focus ? [focus.x, focus.y] : null));
      writePace(paceOf((Date.now() - startedAt) / 1000, width, height));
    } catch (e) {
      if (!isCancelled(e)) setError(String(e));
    } finally {
      setStarting(false);
    }
  };

  const running = starting || !!job;
  const pct = tiles ? Math.round((tiles.current / Math.max(1, tiles.total)) * 100) : null;
  const estimate = estimateFor(readPace(), width, height);
  const took = estimate !== null ? t("tools.denoise.took", { duration: formatDuration(estimate) }) : null;

  return (
    <div className="mt-2 border-t border-line pt-2">
      <span className="section-label px-0">{t("tools.denoise.title")}</span>
      {running ? (
        <div className="mt-1">
          {/* A first run downloads the model before it starts: one row, the
              download's, with its own Cancel. Both used to show — an empty
              "Starting…" bar with a Cancel over the download's bar with
              another. */}
          {!job && downloading ? (
            <ModelDownloads />
          ) : (
            <div className="flex items-center gap-2">
              <ProgressBar
                pct={pct}
                label={t("tools.denoise.title")}
                className="h-1 flex-1 overflow-hidden rounded bg-line"
                barClassName="h-full bg-accent"
              />
              <span className="text-label text-muted tabular-nums">{pct === null ? t("common.starting") : `${pct}%`}</span>
              <button className="text-label text-accent hover:underline" onClick={() => void api.cancelJob("denoise")}>
                {t("common.cancel")}
              </button>
            </div>
          )}
          {left != null && (
            <p className="mt-1 text-label text-muted">
              {t("tools.denoise.timeLeft", { duration: formatDuration(left) })}
            </p>
          )}
        </div>
      ) : ready ? (
        <NumberSlider label={t("tools.denoise.amount")} value={amount} min={0} max={1} step={0.01} onChange={onChange} onCommit={onCommit} />
      ) : amount > 0 ? (
        <div className="mt-1 text-label text-muted">
          {t("tools.denoise.notRun", { percent: Math.round(amount * 100) })}
          {took ? ` ${took}` : ""}{" "}
          <button className="text-accent hover:underline" onClick={() => void run(null)}>
            {t("tools.denoise.runNow")}
          </button>
        </div>
      ) : (
        <div className="mt-1">
          <button className="btn-solid w-full" onClick={() => void run(DEFAULT_AI_DENOISE)}>
            {t("tools.denoise.reduce")}
          </button>
          <p className="mt-1 text-label text-muted">
            {t("tools.denoise.blurb")} {took ?? t("tools.denoise.defaultEstimate")}
          </p>
        </div>
      )}
      {error && <p className="mt-1 text-label text-danger">{error}</p>}
    </div>
  );
}
