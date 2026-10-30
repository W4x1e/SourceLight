import { useCallback, useEffect, useRef, useState } from "react";
import { formatDate, t } from "../../src/i18n";
import { SyncButton } from "./link";
import { api, type Tracks as TrackList } from "./api";

/**
 * The Tracks tab, for shoot tracks (docs/MOBILE.md): record where you are
 * while shooting, so the computer can place photos from a camera without
 * GPS. Recording carries on with the screen off; stopping keeps the track
 * and sends it to the computer, which offers to place the shoot's photos
 * from it.
 */
export default function Tracks({ active, toTop }: { active: boolean; toTop: number }) {
  const scroller = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (toTop > 0) scroller.current?.scrollTo({ top: 0, behavior: "smooth" });
  }, [toTop]);
  const [state, setState] = useState<TrackList | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .tracks()
      .then(setState)
      .catch((e) => setError(String(e)));
  }, []);
  // Each time the tab is opened: a track may have gone with a sync since.
  useEffect(() => {
    if (active) refresh();
  }, [active, refresh]);

  // While recording, the point count and accuracy move: keep them current.
  const recording = state?.status.recording ?? false;
  useEffect(() => {
    if (!recording) return;
    const timer = window.setInterval(refresh, 2000);
    return () => window.clearInterval(timer);
  }, [recording, refresh]);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.trackStart();
      refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    setError(null);
    try {
      // Named for when it ends, in the phone's own time and language.
      setState(await api.trackStop(formatDate(new Date(), { dateStyle: "medium", timeStyle: "short" })));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const status = state?.status;
  const line = !status?.recording
    ? t("mobile.track.idle")
    : status.points === 0
      ? t("mobile.track.waitingFix")
      : t("mobile.track.recording", { count: status.points });

  return (
    <div className="flex h-full flex-col bg-base">
      <header className="flex h-14 shrink-0 items-center justify-end px-1 pt-[var(--safe-top)] box-content">
        <SyncButton />
      </header>
      <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto px-4 pb-5">
        <h1 className="py-1.5 font-display text-headline text-ink">{t("mobile.track.title")}</h1>
        <p className="mt-2 text-body leading-relaxed text-muted">{t("mobile.track.blurb")}</p>

        <div className="mt-5 rounded-md bg-raised p-4">
          <div className="text-body text-ink">{line}</div>
          {status?.recording && status.points > 0 && (
            <div className="mt-0.5 text-label text-faint">
              {t("mobile.track.accuracy", { metres: Math.round(status.lastAccuracy) })}
            </div>
          )}
          <button
            className={`mt-4 w-full py-3 text-body ${recording ? "btn-solid btn-danger" : "btn-accent"}`}
            disabled={busy || state === null}
            onClick={() => void (recording ? stop() : start())}
          >
            {recording ? t("mobile.track.stop") : t("mobile.track.start")}
          </button>
          {error && <p className="mt-3 text-caption text-danger">{error}</p>}
        </div>

        <ul className="mt-5 space-y-1">
          {state && state.tracks.length === 0 && <li className="text-label text-faint">{t("mobile.track.none")}</li>}
          {state?.tracks.map((tr) => (
            <li key={tr.id} className="py-2">
              <div className="text-body text-ink">{tr.name}</div>
              <div className="text-label text-faint">
                {t("mobile.track.points", { count: tr.points })}
                {" · "}
                {t("mobile.track.minutes", { count: Math.max(1, Math.round((tr.endedMs - tr.startedMs) / 60_000)) })}
                {" · "}
                {tr.sent ? t("mobile.track.sent") : t("mobile.track.waiting")}
              </div>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
