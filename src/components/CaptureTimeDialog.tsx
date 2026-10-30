import { useEffect, useMemo, useRef, useState } from "react";
import { t, type Key } from "../i18n";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { useModal } from "../lib/useEscape";
import { useHeld, usePresence } from "../lib/usePresence";
import {
  describeCapture,
  earliest,
  formatCapture,
  parseCapture,
  shiftSeconds,
  toInputValue,
  type Shift,
} from "../lib/captureTime";
import type { CaptureTimeChange } from "../bindings";

type Mode = "shift" | "firstTo" | "allTo";

const MODES: { id: Mode; label: Key; blurb: Key }[] = [
  { id: "shift", label: "library.captureTime.shift", blurb: "library.captureTime.shiftBlurb" },
  { id: "firstTo", label: "library.captureTime.firstTo", blurb: "library.captureTime.firstToBlurb" },
  { id: "allTo", label: "library.captureTime.allTo", blurb: "library.captureTime.allToBlurb" },
];

/**
 * Edit capture time: shift a selection's capture times, set the first
 * and move the rest with it, or give them all one time. The files are never
 * changed; the catalogue keeps each file's own time to go back to, and
 * exports carry the corrected one.
 */
export default function CaptureTimeDialog() {
  const openFor = useStore((s) => s.captureTimeFor);
  // The photos it was opened for, held while it fades out after closing.
  const presence = usePresence(openFor !== null);
  const ids = useHeld(openFor) ?? null;
  const close = useStore((s) => s.setCaptureTimeFor);
  const photos = useStore((s) => s.searchResults ?? s.photos);
  const change = useStore((s) => s.changeCaptureTime);
  const revert = useStore((s) => s.revertCaptureTime);

  const chosen = useMemo(() => {
    const wanted = new Set(ids ?? []);
    return photos.filter((p) => wanted.has(p.id));
  }, [photos, ids]);
  const first = useMemo(() => earliest(chosen), [chosen]);
  const undated = chosen.filter((p) => !parseCapture(p.captureDate)).length;
  const edited = chosen.some((p) => p.captureEdited);

  const [mode, setMode] = useState<Mode>("shift");
  const [shift, setShift] = useState<Shift>({ later: false, days: 0, hours: 1, minutes: 0, seconds: 0 });
  const [time, setTime] = useState("");
  const [busy, setBusy] = useState(false);

  // Each opening starts from the photos' own first time.
  useEffect(() => {
    if (!ids) return;
    setMode(first ? "shift" : "allTo");
    setTime(toInputValue(first ?? new Date()));
    setBusy(false);
  }, [ids]); // eslint-disable-line react-hooks/exhaustive-deps

  const dismiss = () => close(null);
  const card = useRef<HTMLDivElement>(null);
  useModal(openFor !== null, card, dismiss);
  if (!presence.mounted || !ids) return null;

  const seconds = shiftSeconds(shift);
  const target = parseCapture(time);
  // What the earliest photo becomes: the one line that says whether this is right.
  const firstAfter =
    mode === "shift"
      ? first && new Date(first.getTime() + seconds * 1000)
      : mode === "firstTo"
        ? target
        : target;
  // Something to do: a shift of more than nothing, or a first time that moves.
  const ready =
    mode === "shift"
      ? seconds !== 0 && !!first
      : mode === "firstTo"
        ? !!target && !!first && target.getTime() !== first.getTime()
        : !!target;

  const apply = async () => {
    const c: CaptureTimeChange =
      mode === "shift"
        ? { kind: "shift", seconds }
        : { kind: mode, time: target ? formatCapture(target) : time };
    setBusy(true);
    if (await change(ids, c)) close(null);
    else setBusy(false);
  };

  const n = ids.length;
  const num = (key: keyof Omit<Shift, "later">, label: string, max: number) => (
    <label className="flex flex-col gap-1 text-label text-faint">
      {label}
      <input
        type="number"
        min={0}
        max={max}
        className="input w-16 tabular-nums"
        value={shift[key]}
        onChange={(e) => setShift({ ...shift, [key]: Number(e.target.value) })}
      />
    </label>
  );

  return (
    <div
      data-closing={presence.closing || undefined}
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      onClick={(e) => e.target === e.currentTarget && dismiss()}
    >
      <div
        ref={card}
        role="dialog"
        aria-modal="true"
        aria-label={t("library.captureTime.title")}
        className="anim-card flex max-h-[min(640px,calc(100vh-4rem))] w-full max-w-md flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-4 py-3.5">
          <div className="min-w-0">
            <h2 className="font-display text-display text-ink">{t("library.captureTime.title")}</h2>
            <p className="text-caption text-faint">
              {t("library.captureTime.photos", { count: n })}
              {" · "}
              {first
                ? t("library.captureTime.earliest", { time: describeCapture(first) })
                : t("library.captureTime.noneTimed")}
            </p>
          </div>
          <button onClick={dismiss} className="icon-chip ml-auto shrink-0" title={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex-1 space-y-4 overflow-y-auto px-4 py-4">
          <div className="flex gap-1.5" role="radiogroup" aria-label={t("library.captureTime.how")}>
            {MODES.map((m) => (
              <button
                key={m.id}
                role="radio"
                aria-checked={mode === m.id}
                disabled={m.id !== "allTo" && !first}
                onClick={() => setMode(m.id)}
                className={`btn-solid ${mode === m.id ? "seg-on" : ""}`}
              >
                {t(m.label)}
              </button>
            ))}
          </div>
          <p className="text-caption text-muted">{t(MODES.find((m) => m.id === mode)!.blurb)}</p>

          {mode === "shift" ? (
            <div className="flex flex-wrap items-end gap-2">
              <div className="flex gap-1" role="radiogroup" aria-label={t("library.captureTime.direction")}>
                {([false, true] as const).map((later) => (
                  <button
                    key={String(later)}
                    role="radio"
                    aria-checked={shift.later === later}
                    onClick={() => setShift({ ...shift, later })}
                    className={`btn-solid ${shift.later === later ? "seg-on" : ""}`}
                  >
                    {later ? t("library.captureTime.later") : t("library.captureTime.earlier")}
                  </button>
                ))}
              </div>
              {num("days", t("library.captureTime.days"), 3650)}
              {num("hours", t("library.captureTime.hours"), 23)}
              {num("minutes", t("library.captureTime.minutes"), 59)}
              {num("seconds", t("library.captureTime.seconds"), 59)}
            </div>
          ) : (
            <label className="flex flex-col gap-1 text-label text-faint">
              {mode === "firstTo" ? t("library.captureTime.firstAt") : t("library.captureTime.allAt")}
              <input
                type="datetime-local"
                step={1}
                className="input w-60"
                value={time}
                onChange={(e) => setTime(e.target.value)}
              />
            </label>
          )}

          <div className="rounded-md bg-raised px-3 py-2 text-caption">
            {mode === "allTo" ? (
              <span className="text-ink">
                {t("library.captureTime.becomes", { count: n, time: describeCapture(target) })}
              </span>
            ) : (
              <span className="text-ink">
                {t("library.captureTime.earliestChange", {
                  from: describeCapture(first),
                  to: describeCapture(firstAfter ?? null),
                })}
              </span>
            )}
            {mode !== "allTo" && undated > 0 && (
              <div className="text-label text-faint">{t("library.captureTime.undated", { count: undated })}</div>
            )}
          </div>

          <p className="text-label leading-relaxed text-faint">{t("library.captureTime.note")}</p>
        </div>

        <div className="flex items-center gap-2 border-t border-line px-4 py-3">
          {edited && (
            <button
              onClick={() => {
                close(null);
                void revert(ids);
              }}
              className="btn-ghost"
              title={t("library.captureTime.revertTitle")}
            >
              {t("library.captureTime.revert")}
            </button>
          )}
          <button onClick={dismiss} className="btn-ghost ml-auto">
            {t("common.cancel")}
          </button>
          <button onClick={() => void apply()} className="btn-accent" disabled={!ready || busy}>
            {t("library.captureTime.apply", { count: n })}
          </button>
        </div>
      </div>
    </div>
  );
}
