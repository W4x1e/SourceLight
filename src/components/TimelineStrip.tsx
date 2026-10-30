import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { sameFrames, useStable } from "../lib/useStable";
import { formatDate, formatNumber, t, tx } from "../i18n";
import { api } from "../lib/api";
import { useStore, viewFilter } from "../store";
import { Icon } from "../lib/icons";
import { dayWindow, fillPeriods, inRange, periodLabel, periodRange } from "../lib/timeline";
import type { TimelineBar } from "../bindings";

const BARS_H = 30;

/**
 * The library's shape over time, above the grid: photos in view per
 * month — per day once a range of three months or less is chosen. Click a bar
 * to go to it in the grid; drag across bars to choose a date range; double-
 * click to clear it. The chosen range doesn't narrow the bars, so the rest of
 * the library stays in sight around it. Hidden for a content search, whose
 * results are in order of relevance, not time.
 */
export default function TimelineStrip() {
  const committed = useStore((s) => s.committedFilter);
  const photos = useStore((s) => s.photos);
  const searching = useStore((s) => s.searchResults !== null || !!s.committedFilter.semantic);
  const collapsed = useStore((s) => s.timelineCollapsed);
  const setCollapsed = useStore((s) => s.setTimelineCollapsed);
  const jump = useStore((s) => s.jumpGridTo);

  const from = committed.dateFrom ?? null;
  const to = committed.dateTo ?? null;
  const days = dayWindow(from, to);
  // Counted again when the photos change, not when a verdict lands,
  // unless the filter is on verdicts, where a rating moves a photo in or out.
  const onVerdicts =
    committed.minRating != null ||
    (committed.ratings?.length ?? 0) > 0 ||
    committed.flags.length > 0 ||
    committed.colorLabels.length > 0;
  const frames = useStable(photos, sameFrames);
  const counted = onVerdicts ? photos : frames;
  const [raw, setRaw] = useState<TimelineBar[]>([]);
  useEffect(() => {
    if (searching || collapsed) return;
    let live = true;
    api
      .timeline(viewFilter(committed), days?.from ?? null, days?.to ?? null)
      .then((b) => live && setRaw(b))
      .catch(() => live && setRaw([]));
    return () => {
      live = false;
    };
    // The photos too: a delete, or under a filter on verdicts a rating,
    // changes the counts under the same filter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [committed, counted, searching, collapsed, days?.from, days?.to]);
  const bars = useMemo(() => fillPeriods(raw, days), [raw, days?.from, days?.to]); // eslint-disable-line react-hooks/exhaustive-deps
  const max = Math.max(1, ...bars.map((b) => b.count));

  const boxRef = useRef<HTMLDivElement>(null);
  const keysId = useId();
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = boxRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => e && setWidth(e.contentRect.width));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
    // The box exists only once there are bars to draw.
  }, [collapsed, searching, bars.length > 0]);

  const [hover, setHover] = useState<number | null>(null);
  const [drag, setDrag] = useState<{ start: number; end: number } | null>(null);
  // The keyboard's place on the strip, shown as the hover is; with
  // Shift it marks out a range from where Shift was first held.
  const [cursor, setCursor] = useState<number | null>(null);
  const barW = bars.length > 0 ? width / bars.length : 0;
  const at = (clientX: number) => {
    const r = boxRef.current!.getBoundingClientRect();
    return Math.max(0, Math.min(bars.length - 1, Math.floor((clientX - r.left) / Math.max(barW, 1e-6))));
  };

  if (searching) return null;
  if (collapsed) {
    return (
      <button
        onClick={() => setCollapsed(false)}
        className="flex h-5 shrink-0 items-center gap-1 border-b border-line/60 px-3 text-caption text-faint hover:text-muted"
        title={t("library.timeline.show")}
      >
        <Icon name="calendar" size={12} /> {t("library.timeline.title")}
      </button>
    );
  }
  if (bars.length === 0) return null;

  const applyRange = (d: { start: number; end: number }) => {
    const [a, b] = [Math.min(d.start, d.end), Math.max(d.start, d.end)];
    if (a === b) {
      if (bars[a]!.count > 0) jump(bars[a]!.period);
      return;
    }
    const s = useStore.getState();
    s.setFilter({
      ...s.filter,
      dateFrom: periodRange(bars[a]!.period).from,
      dateTo: periodRange(bars[b]!.period).to,
    });
  };
  const clearRange = () => {
    const s = useStore.getState();
    if (s.filter.dateFrom || s.filter.dateTo) s.setFilter({ ...s.filter, dateFrom: null, dateTo: null });
  };
  /** ← → step a period, Home/End to the ends; Shift marks a range; Enter goes
   *  there or applies the range; Escape lets a range go; Delete clears dates. */
  const onKey = (e: React.KeyboardEvent) => {
    const last = bars.length - 1;
    const here = cursor ?? Math.max(0, bars.findIndex((b) => b.count > 0));
    const to = (i: number) => {
      const next = Math.max(0, Math.min(last, i));
      if (e.shiftKey) setDrag({ start: drag?.start ?? here, end: next });
      else setDrag(null);
      setCursor(next);
      setHover(next);
    };
    switch (e.key) {
      case "ArrowLeft": to(here - 1); break;
      case "ArrowRight": to(here + 1); break;
      case "Home": to(0); break;
      case "End": to(last); break;
      case "Enter":
        applyRange(drag ?? { start: here, end: here });
        setDrag(null);
        break;
      case "Escape":
        if (!drag) return;
        setDrag(null);
        break;
      case "Delete":
      case "Backspace":
        clearRange();
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  const chosen = (i: number) =>
    drag ? i >= Math.min(drag.start, drag.end) && i <= Math.max(drag.start, drag.end) : inRange(bars[i]!.period, from, to);
  const ranged = !!drag || !!from || !!to;
  const shown = hover !== null ? bars[hover] : null;
  // Year marks under the months; month marks under the days.
  const marks = bars
    .map((b, i) => ({ b, i }))
    .filter(({ b, i }) => (days ? b.period.endsWith("-01") : b.period.endsWith("-01") || i === 0));
  // A month's short name in the interface's language, or the year.
  const markLabel = (p: string) =>
    days ? formatDate(Date.UTC(2000, Number(p.slice(5, 7)) - 1, 1), { month: "short", timeZone: "UTC" }) : p.slice(0, 4);
  const spacedMarks = marks.filter((m, k) => k === 0 || (m.i - marks[k - 1]!.i) * barW >= 34);

  return (
    <div className="shrink-0 border-b border-line/60 px-3 pb-1 pt-1.5">
      <div className="mb-0.5 flex h-4 items-center gap-2 text-caption text-faint">
        {/* The count is the readout, in mono; the date stays in the face the
            hint beside it is in, so hovering doesn't swap the line's font. */}
        <span className="min-w-0 truncate text-muted">
          {shown ? (
            <>
              {periodLabel(shown.period)} ·{" "}
              {tx("common.photos", { count: <span className="tabular-nums">{formatNumber(shown.count)}</span> }, shown.count)}
            </>
          ) : days ? (
            t("library.timeline.byDay")
          ) : (
            t("library.timeline.hint")
          )}
        </span>
        <button
          onClick={() => setCollapsed(true)}
          className="ml-auto hover:text-muted"
          title={t("library.timeline.hide")}
          aria-label={t("library.timeline.hide")}
        >
          <Icon name="chevron" size={12} className="rotate-180" />
        </button>
      </div>
      <div
        ref={boxRef}
        role="slider"
        tabIndex={0}
        aria-label={t("library.timeline.label")}
        aria-valuemin={0}
        aria-valuemax={bars.length - 1}
        aria-valuenow={cursor ?? undefined}
        aria-valuetext={shown ? `${periodLabel(shown.period)}, ${t("common.photos", { count: shown.count })}` : undefined}
        aria-describedby={keysId}
        onKeyDown={onKey}
        onFocus={() => {
          const i = cursor ?? Math.max(0, bars.findIndex((b) => b.count > 0));
          setCursor(i);
          setHover(i);
        }}
        onBlur={() => {
          setHover(null);
          setDrag(null);
        }}
        // The app's focus ring: a `role="slider"` div isn't one of the
        // elements styles.css gives it to, so Tab landed here unmarked.
        className="relative cursor-pointer select-none rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus focus-visible:outline-solid"
        style={{ height: BARS_H + 12 }}
        onPointerMove={(e) => {
          const i = at(e.clientX);
          setHover(i);
          if (drag) setDrag({ ...drag, end: i });
        }}
        onPointerLeave={() => !drag && setHover(null)}
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          const i = at(e.clientX);
          setDrag({ start: i, end: i });
        }}
        onPointerUp={() => {
          const d = drag;
          setDrag(null);
          if (d) applyRange(d);
        }}
        onDoubleClick={clearRange}
      >
        <svg width={width} height={BARS_H + 12} className="block">
          {bars.map((b, i) => {
            if (b.count === 0) return null;
            // Square-root heights, so a quiet month still shows beside a
            // wedding's thousand frames.
            const h = Math.max(2, Math.sqrt(b.count / max) * BARS_H);
            const on = !ranged || chosen(i);
            return (
              <rect
                key={b.period}
                x={i * barW + (barW > 3 ? 0.5 : 0)}
                y={BARS_H - h}
                width={Math.max(1, barW - (barW > 3 ? 1 : 0))}
                height={h}
                style={{
                  fill: `var(--color-${i === hover ? "ink" : !ranged ? "faint" : on ? "muted" : "dim"})`,
                  opacity: on ? 1 : 0.6,
                }}
              />
            );
          })}
          {spacedMarks.map(({ b, i }) => (
            <text
              key={`t${b.period}`}
              x={i * barW + 1}
              y={BARS_H + 10}
              style={{ fill: "var(--color-faint)", fontSize: "var(--text-label)" }}
            >
              {markLabel(b.period)}
            </text>
          ))}
        </svg>
      </div>
      <p id={keysId} className="sr-only">
        {t("library.timeline.keys")}
      </p>
    </div>
  );
}
