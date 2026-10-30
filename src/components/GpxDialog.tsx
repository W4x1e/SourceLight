import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { Icon } from "../lib/icons";
import { useEscape } from "../lib/useEscape";
import type { GpxPreview } from "../bindings";
import { formatNumber, t, tx, type Key } from "../i18n";

/**
 * Place photos from a GPX track — for cameras with no GPS. The phone
 * or watch in a pocket knew where it was; the camera knew when each frame was
 * taken. Matching the two needs one thing the user may not know offhand: how
 * far the camera's clock is from UTC. So the dialog opens on the offset that
 * places the most photos, shows what it would do by day, and writes nothing
 * until asked. Places already set by hand are kept.
 */

/** "UTC+01:00" for 60. */
function offsetLabel(m: number): string {
  const sign = m < 0 ? "−" : "+";
  const a = Math.abs(m);
  return `UTC${sign}${String(Math.floor(a / 60)).padStart(2, "0")}:${String(a % 60).padStart(2, "0")}`;
}

/** The range of real time zones, as the backend guesses them. */
const MIN_OFFSET = -12 * 60;
const MAX_OFFSET = 14 * 60;

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export default function GpxDialog({
  paths,
  onClose,
  onDone,
}: {
  paths: string[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [offset, setOffset] = useState<number | null>(null);
  const [guessed, setGuessed] = useState<number | null>(null);
  const [preview, setPreview] = useState<GpxPreview | null>(null);
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);

  const close = useCallback(() => {
    if (!running) onClose();
  }, [running, onClose]);
  useEscape(true, close);

  // First load guesses the offset; changing it asks again with that offset.
  useEffect(() => {
    let live = true;
    setLoading(true);
    api
      .gpxPreview(paths, offset)
      .then((p) => {
        if (!live) return;
        setPreview(p);
        if (offset === null) {
          setGuessed(p.offsetMinutes);
          setOffset(p.offsetMinutes);
        }
      })
      .catch((e) => {
        toast(String(e), "error");
        onClose();
      })
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [paths, offset]); // eslint-disable-line react-hooks/exhaustive-deps

  const apply = async () => {
    if (offset === null) return;
    setRunning(true);
    try {
      const out = await api.gpxApply(paths, offset);
      toast(
        out.placed < out.matched
          ? t("places.gpx.placedWithTowns", { count: out.matched, placed: out.placed })
          : t("places.gpx.placed", { count: out.matched }),
        "success",
      );
      onDone();
    } catch (e) {
      toast(String(e), "error");
      setRunning(false);
    }
  };

  const maxDay = useMemo(() => Math.max(1, ...(preview?.days.map((d) => d.count) ?? [])), [preview]);
  const nothing = preview !== null && preview.points === 0;

  return (
    <div
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      onClick={(e) => e.target === e.currentTarget && close()}
    >
      <div
        role="dialog"
        aria-label={t("places.gpx.dialogLabel")}
        className="anim-card flex max-h-[min(640px,calc(100vh-4rem))] w-full max-w-md flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-4 py-3.5">
          <div className="min-w-0">
            <h2 className="font-display text-display text-ink">{t("places.gpx.title")}</h2>
            <p className="truncate text-caption text-faint" title={paths.join("\n")}>
              {paths.length === 1 ? fileName(paths[0]!) : t("places.gpx.tracks", { count: paths.length })}
              {preview && ` · ${t("places.gpx.points", { count: preview.points })}`}
            </p>
          </div>
          <button onClick={close} className="icon-chip ml-auto shrink-0" title={t("common.close")} disabled={running}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex-1 space-y-4 overflow-y-auto px-4 py-4">
          {nothing ? (
            <p className="text-caption leading-relaxed text-muted">
              {paths.length === 1 ? t("places.gpx.noPointsFile") : t("places.gpx.noPointsFiles")}
            </p>
          ) : (
            <>
              <div>
                <div className="section-label mb-1.5 px-0">{t("places.gpx.cameraClock")}</div>
                {/* Steppers, not a native <select>: the webview fires change
                    events on native dropdowns unreliably (see AddToCollection). */}
                <div className="flex items-center gap-1">
                  {(
                    [
                      [-60, "places.gpx.hourEarlier", "places.gpx.hourEarlierShort"],
                      [-15, "places.gpx.quarterEarlier", "places.gpx.quarterEarlierShort"],
                    ] as [number, Key, Key][]
                  ).map(([d, hint, label]) => (
                    <button
                      key={d}
                      className="btn-ghost px-2 font-mono text-caption tabular-nums"
                      disabled={offset === null || running || offset + d < MIN_OFFSET}
                      onClick={() => setOffset((o) => (o ?? 0) + d)}
                      title={t(hint)}
                    >
                      {t(label)}
                    </button>
                  ))}
                  <div className="flex-1 text-center">
                    <span className="font-mono text-body tabular-nums text-ink">
                      {offset === null ? "…" : offsetLabel(offset)}
                    </span>
                    {offset !== null && offset === guessed && (preview?.matched ?? 0) > 0 && (
                      <span className="ml-1.5 text-label text-faint">{t("places.gpx.bestMatch")}</span>
                    )}
                  </div>
                  {(
                    [
                      [15, "places.gpx.quarterLater", "places.gpx.quarterLaterShort"],
                      [60, "places.gpx.hourLater", "places.gpx.hourLaterShort"],
                    ] as [number, Key, Key][]
                  ).map(([d, hint, label]) => (
                    <button
                      key={d}
                      className="btn-ghost px-2 font-mono text-caption tabular-nums"
                      disabled={offset === null || running || offset + d > MAX_OFFSET}
                      onClick={() => setOffset((o) => (o ?? 0) + d)}
                      title={t(hint)}
                    >
                      {t(label)}
                    </button>
                  ))}
                </div>
                {guessed !== null && offset !== guessed && (
                  <button className="mt-1 text-label text-muted underline" onClick={() => setOffset(guessed)}>
                    {t("places.gpx.backToBest", { offset: offsetLabel(guessed) })}
                  </button>
                )}
                <p className="mt-1.5 text-label leading-relaxed text-faint">
                  {t("places.gpx.clockExplain")}
                </p>
              </div>

              <div>
                <div className="flex items-baseline justify-between">
                  <div className="section-label px-0">{t("places.gpx.wouldPlace")}</div>
                  {preview && (
                    <div className="text-caption text-faint">
                      {tx(
                        "places.gpx.wouldPlaceOf",
                        {
                          matched: <span className="font-mono tabular-nums text-ink">{formatNumber(preview.matched)}</span>,
                          count: preview.candidates,
                        },
                        preview.candidates,
                      )}
                    </div>
                  )}
                </div>
                <div className={`mt-2 space-y-1 ${loading ? "opacity-50" : ""}`}>
                  {preview?.days.map((d) => (
                    <div key={d.date} className="flex items-center gap-2 text-caption">
                      <span className="w-24 shrink-0 font-mono tabular-nums text-muted">{d.date}</span>
                      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-base">
                        <div
                          className="h-full rounded-full bg-accent"
                          style={{ width: `${(d.count / maxDay) * 100}%` }}
                        />
                      </div>
                      <span className="w-12 text-right font-mono tabular-nums text-faint">
                        {formatNumber(d.count)}
                      </span>
                    </div>
                  ))}
                  {preview && preview.matched === 0 && (
                    <p className="text-caption leading-relaxed text-faint">
                      {t("places.gpx.noMatches")}
                    </p>
                  )}
                </div>
              </div>
            </>
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-line px-4 py-3">
          <button onClick={close} className="btn-ghost ml-auto" disabled={running}>
            {t("common.cancel")}
          </button>
          <button
            onClick={() => void apply()}
            className="btn-accent"
            disabled={loading || running || !preview || preview.matched === 0}
          >
            {running
              ? t("places.gpx.placing")
              : preview && preview.matched > 0
                ? t("places.gpx.placeCount", { count: preview.matched })
                : t("places.gpx.place")}
          </button>
        </div>
      </div>
    </div>
  );
}
