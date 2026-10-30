import { useEffect, useRef, useState } from "react";
import ProgressBar from "../ProgressBar";
import { useStore } from "../../store";
import { isDownload } from "../../lib/jobs";
import { formatDuration } from "../../lib/denoiseTime";
import { t, type Key } from "../../i18n";

/**
 * How far an AI mask has got, for Select subject and click-to-select.
 *
 * The models report no percentage: each is one run on the graphics card. What
 * the backend does say is which step it is on (`ai-mask-stage`), and the two
 * slow ones are steps nobody would guess: waiting for photo tagging to let go
 * of the graphics card, and loading the model the first time. So the panel
 * names the step, sweeps a bar and counts the seconds, and the photo carries
 * the same words where the eye is.
 */

/** The steps the backend reports, in order; `null` before the first. */
export type AiMaskStage = "waiting" | "loading" | "subject" | "encoding";

const STAGE_KEYS: Record<AiMaskStage, Key> = {
  waiting: "tools.mask.stage.waiting",
  loading: "tools.mask.stage.loading",
  subject: "tools.mask.stage.subject",
  encoding: "tools.mask.stage.encoding",
};

export function isAiMaskStage(s: unknown): s is AiMaskStage {
  return typeof s === "string" && s in STAGE_KEYS;
}

/**
 * What to say for `stage`. With no step reported yet, a click that refines a
 * selection is a quick update, and anything else is still getting ready.
 */
export function aiStageText(stage: AiMaskStage | null, refining: boolean): string {
  if (stage) return t(STAGE_KEYS[stage]);
  return t(refining ? "tools.mask.stage.refining" : "tools.mask.stage.starting");
}

/**
 * True once `busy` has lasted `delay` ms. A refining click comes back in a
 * blink, and a bar that flashes for it reads as a glitch.
 */
export function useShownAfter(busy: boolean, delay = 300): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!busy) {
      setShown(false);
      return;
    }
    const id = window.setTimeout(() => setShown(true), delay);
    return () => window.clearTimeout(id);
  }, [busy, delay]);
  return shown;
}

/** Whole seconds since `busy` turned on, ticking while it stays on. */
function useElapsed(busy: boolean): number {
  const since = useRef(0);
  const [seconds, setSeconds] = useState(0);
  useEffect(() => {
    if (!busy) return;
    since.current = Date.now();
    setSeconds(0);
    const id = window.setInterval(() => setSeconds(Math.floor((Date.now() - since.current) / 1000)), 250);
    return () => window.clearInterval(id);
  }, [busy]);
  return seconds;
}

/**
 * The masks panel's line: the step, the seconds so far, and a sweeping bar.
 * Steps aside while a model downloads, which has its own bar with bytes.
 */
export default function AiMaskProgress({
  busy,
  stage,
  refining,
  className = "",
}: {
  busy: boolean;
  stage: AiMaskStage | null;
  refining: boolean;
  className?: string;
}) {
  const shown = useShownAfter(busy);
  const elapsed = useElapsed(busy);
  const downloading = useStore((s) => Object.keys(s.jobs).some((id) => isDownload(id) && !s.jobs[id]!.error));
  if (!shown || downloading) return null;
  const text = aiStageText(stage, refining);
  return (
    <div className={className} role="status">
      <div className="mb-1 flex min-w-0 items-start gap-2 text-label leading-snug text-muted">
        <span className="min-w-0 flex-1">{text}</span>
        {elapsed > 0 && <span className="shrink-0 tabular-nums text-faint">{formatDuration(elapsed)}</span>}
      </div>
      <ProgressBar pct={null} label={text} />
    </div>
  );
}

/** A small turning ring, for a chip on the photo. */
export function Spinner({ className = "h-3 w-3" }: { className?: string }) {
  return (
    <span
      aria-hidden
      className={`inline-block shrink-0 animate-spin rounded-full border-2 border-white/30 border-t-white ${className}`}
    />
  );
}
