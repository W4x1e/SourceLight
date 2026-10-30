import { useEffect, useState, type RefObject } from "react";
import { formatNumber, t } from "../../src/i18n";

/** The drawing's size, in its own units. */
const W = 300;
const H = 64;
/** Bins per channel. */
const BINS = 64;
/** The width the frame is read at: plenty for a histogram, and quick. */
const SAMPLE = 192;

type Reading = { r: number[]; g: number[]; b: number[]; shadowClip: number; highlightClip: number };

/** Counts a frame's pixels into bins per channel, and the share crushed to
 *  black or blown to white. */
function read(canvas: HTMLCanvasElement): Reading | null {
  if (!canvas.width || !canvas.height) return null;
  const w = Math.min(SAMPLE, canvas.width);
  const h = Math.max(1, Math.round((canvas.height * w) / canvas.width));
  const probe = document.createElement("canvas");
  Object.assign(probe, { width: w, height: h });
  const ctx = probe.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  ctx.drawImage(canvas, 0, 0, w, h);
  const px = ctx.getImageData(0, 0, w, h).data;
  const r = new Array<number>(BINS).fill(0);
  const g = new Array<number>(BINS).fill(0);
  const b = new Array<number>(BINS).fill(0);
  let dark = 0;
  let bright = 0;
  const shift = Math.log2(256 / BINS);
  for (let i = 0; i < px.length; i += 4) {
    const [pr, pg, pb] = [px[i]!, px[i + 1]!, px[i + 2]!];
    r[pr >> shift]! += 1;
    g[pg >> shift]! += 1;
    b[pb >> shift]! += 1;
    if (pr <= 2 && pg <= 2 && pb <= 2) dark += 1;
    if (pr >= 253 || pg >= 253 || pb >= 253) bright += 1;
  }
  const n = px.length / 4;
  return { r, g, b, shadowClip: (dark / n) * 100, highlightClip: (bright / n) * 100 };
}

function polygon(bins: number[]): string {
  // The end bins left out of the scale: pure black and white spike and would
  // flatten everything else, as on the desktop.
  const max = Math.max(...bins.slice(1, -1), 1);
  const pts = bins.map((v, i) => `${((i / (bins.length - 1)) * W).toFixed(1)},${(H - Math.min(1, v / max) * H).toFixed(1)}`);
  return `0,${H} ${pts.join(" ")} ${W},${H}`;
}

const clip = (v: number) => formatNumber(v, { minimumFractionDigits: 1, maximumFractionDigits: 1 });

/**
 * The desktop's RGB histogram (components/dev/Histogram.tsx) and clipping
 * readouts, read from the editor's own frame. Unlike the desktop's preview,
 * the phone's frames are drawn into a canvas from a blob, so their pixels can
 * be read here. Read again each time `frame` changes: a sharp frame landed.
 */
export default function Histogram({ canvas, frame }: { canvas: RefObject<HTMLCanvasElement | null>; frame: number }) {
  const [reading, setReading] = useState<Reading | null>(null);
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    try {
      const next = read(c);
      if (next) setReading(next);
    } catch {
      // A histogram that can't be read leaves the last one up.
    }
  }, [canvas, frame]);
  return (
    <div className="mb-1">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="h-14 w-full rounded-sm bg-black/30" aria-hidden>
        {reading && (
          <>
            <polygon points={polygon(reading.b)} fill="rgba(91,141,217,.5)" />
            <polygon points={polygon(reading.g)} fill="rgba(111,191,115,.5)" />
            <polygon points={polygon(reading.r)} fill="rgba(224,98,91,.5)" />
          </>
        )}
      </svg>
      <div className="mt-1 flex justify-between font-mono text-label tabular-nums text-faint">
        <span>{t("develop.histogram.shadowClip", { percent: clip(reading?.shadowClip ?? 0) })}</span>
        <span>{t("develop.histogram.highlightClip", { percent: clip(reading?.highlightClip ?? 0) })}</span>
      </div>
    </div>
  );
}
