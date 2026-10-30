import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useLatestFrame } from "../../lib/useLatestFrame";
import { t, formatNumber } from "../../i18n";
import type { EditStack, Histogram as HistogramData } from "../../bindings";

/**
 * A live RGB histogram of the current preview, plus clipping readouts.
 *
 * **Computed in Rust** (`api.developHistogram`), not from the preview image:
 * that image reaches the webview over the custom `develop://` scheme, and
 * reading its pixels back through a `<canvas>` either fails the cross-origin
 * load or taints the canvas, so the browser cannot build this. A small proxy
 * render on the Rust side is cheap and always readable.
 *
 * Keyed on `frame`, which the store bumps on every edit — so it refreshes when
 * the photo's look changes but not when the viewer merely pans or zooms, which
 * is correct: a histogram is a property of the image, not the view.
 *
 * And on `draft`, the edit mid-drag: a slider commits only on release, and the
 * histogram used to sit still until then while the photo changed under it. One
 * request at a time, converging on the newest (`useLatestFrame`), so a drag
 * cannot queue them faster than they are made.
 */

const W = 300;
const H = 76;

function polygon(bins: number[]): string {
  // Ignore the two end bins when scaling — pure black and pure white spike and
  // would flatten everything else, as editors' histograms usually do.
  const max = Math.max(...bins.slice(1, -1), 1);
  const n = bins.length;
  const pts = bins.map((v, i) => {
    const x = (i / (n - 1)) * W;
    const y = H - Math.min(1, v / max) * H;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  return `0,${H} ${pts.join(" ")} ${W},${H}`;
}

/** A clipping percentage to one decimal, in the language's own notation. */
const clip = (v: number | null | undefined) =>
  formatNumber(v ?? 0, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
/** The same as a percentage, sign and spacing as the language writes it
 *  ("1.4%", "1,4 %"). */
const clipPct = (v: number | null | undefined) =>
  formatNumber((v ?? 0) / 100, { style: "percent", minimumFractionDigits: 1, maximumFractionDigits: 1 });

export default function Histogram({ frame, draft }: { frame: number; draft: EditStack | null }) {
  const [data, setData] = useState<HistogramData | null>(null);
  const [asked, settled] = useLatestFrame(draft ? `draft ${JSON.stringify(draft)}` : `frame ${frame}`);

  useEffect(() => {
    const draftAsked = asked.startsWith("draft ")
      ? (JSON.parse(asked.slice("draft ".length)) as EditStack)
      : null;
    api
      .developHistogram(draftAsked)
      .then(setData)
      // A failed histogram must never take the panel down — leave the last one up.
      .catch(() => {})
      .finally(settled);
  }, [asked, settled]);

  return (
    <div className="border-b border-line px-3 py-2.5">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="h-[72px] w-full">
        {data && (
          <>
            <polygon points={polygon(data.b)} fill="rgba(91,141,217,.5)" />
            <polygon points={polygon(data.g)} fill="rgba(111,191,115,.5)" />
            <polygon points={polygon(data.r)} fill="rgba(224,98,91,.5)" />
          </>
        )}
      </svg>
      {/* One line whatever the width and language: the name gives way
          (shortened, whole on hover) and the number never does. In Japanese
          at the minimum width each readout wrapped, its number or its dot
          alone on a second line. */}
      <div className="mt-1 flex gap-3 text-label text-faint">
        <span
          className="flex min-w-0 flex-1 items-center gap-1"
          title={t("develop.histogram.shadowClip", { percent: clip(data?.shadowClip) })}
        >
          <span className="inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-[#2b3b6b]" />
          <span className="truncate">{t("develop.histogram.shadow")}</span>
          <span className="shrink-0 tabular-nums">{clipPct(data?.shadowClip)}</span>
        </span>
        <span
          className="flex min-w-0 flex-1 items-center justify-end gap-1"
          title={t("develop.histogram.highlightClip", { percent: clip(data?.highlightClip) })}
        >
          <span className="truncate">{t("develop.histogram.highlight")}</span>
          <span className="shrink-0 tabular-nums">{clipPct(data?.highlightClip)}</span>
          <span className="inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-[#6b5a2b]" />
        </span>
      </div>
    </div>
  );
}
