import { useEffect, useState } from "react";
import { sentenceCase } from "../lib/format";
import { tagName } from "../i18n/tags";
import { periodLabel } from "../lib/timeline";
import { api } from "../lib/api";
import { useStore, viewFilter } from "../store";
import { Icon } from "../lib/icons";
import { useEscape } from "../lib/useEscape";
import { focalRangeOf, keeperShare, most, patchFor, type InsightKind } from "../lib/insights";
import { APERTURE_RANGES, FOCAL_RANGES, ISO_RANGES } from "../types";
import { formatDate, formatNumber, t, type Key } from "../i18n";
import type { InsightCount, Insights, LensKeywords, PhotoFilter } from "../bindings";
import { usePresence } from "../lib/usePresence";

/** A day, in the interface's language: it was English whatever the language. */
const dayLabel = (day: string) => periodLabel(day);

/** A month's short name, in the interface's language. */
const monthName = (m: number) => formatDate(Date.UTC(2000, m - 1, 1), { month: "short", timeZone: "UTC" });

/** A range's name: worded when shown, so it follows the language. */
const rangeLabel = (table: { key: string; labelKey?: Key; label?: string }[]) => (key: string) => {
  const r = table.find((x) => x.key === key);
  return r?.labelKey ? t(r.labelKey) : (r?.label ?? key);
};

/** A photo count, and the keepers' part of it, as a tooltip. */
const tip = (label: string, c: InsightCount) =>
  `${label} · ${t("viewer.insights.photos", { count: c.count })}` +
  (c.count > 0 ? ` · ${t("viewer.insights.keepers", { share: keeperShare(c) })}` : "");

/** Upright bars: time, and the settings. The keepers are the bright part. */
function Columns({
  items,
  label,
  onPick,
  height = 96,
}: {
  items: InsightCount[];
  label: (key: string) => string;
  onPick?: (key: string) => void;
  height?: number;
}) {
  const max = most(items);
  if (items.length === 0) return <p className="text-caption text-faint">{t("viewer.insights.nothing")}</p>;
  return (
    <div className="flex items-end gap-1" style={{ height: height + 34 }}>
      {items.map((c) => {
        const h = Math.max(2, (c.count / max) * height);
        const k = c.count > 0 ? (c.keepers / c.count) * h : 0;
        return (
          <button
            key={c.key}
            onClick={onPick ? () => onPick(c.key) : undefined}
            disabled={!onPick}
            title={tip(label(c.key), c)}
            className="group flex min-w-0 flex-1 flex-col items-center gap-1 disabled:cursor-default"
          >
            <span className="text-label tabular-nums text-faint opacity-0 group-hover:opacity-100">
              {formatNumber(c.count)}
            </span>
            <span
              className="relative w-full max-w-10 overflow-hidden rounded-t-sm bg-dim transition-colors group-enabled:group-hover:bg-muted"
              style={{ height: h }}
            >
              <span className="absolute inset-x-0 bottom-0 bg-muted group-enabled:group-hover:bg-ink" style={{ height: k }} />
            </span>
            <span className="w-full truncate text-center text-label text-faint">{label(c.key)}</span>
          </button>
        );
      })}
    </div>
  );
}

/** Lying bars, for names: cameras, lenses, days. */
function Rows({
  items,
  label = (k) => k,
  onPick,
  limit = 8,
}: {
  items: InsightCount[];
  label?: (key: string) => string;
  onPick: (key: string) => void;
  limit?: number;
}) {
  const shown = items.slice(0, limit);
  const max = most(shown);
  if (shown.length === 0) return <p className="text-caption text-faint">{t("viewer.insights.nothing")}</p>;
  return (
    <div className="flex flex-col gap-1">
      {shown.map((c) => (
        <button
          key={c.key}
          onClick={() => onPick(c.key)}
          title={tip(label(c.key), c)}
          className="group grid grid-cols-[minmax(0,11rem)_1fr_3rem_2.5rem] items-center gap-2 rounded-sm px-1 py-0.5 text-left hover:bg-hover"
        >
          <span className="truncate text-caption text-muted group-hover:text-ink">{label(c.key)}</span>
          <span className="relative h-2.5 overflow-hidden rounded-sm bg-raised">
            <span className="absolute inset-y-0 left-0 bg-dim" style={{ width: `${(c.count / max) * 100}%` }} />
            <span className="absolute inset-y-0 left-0 bg-muted" style={{ width: `${(c.keepers / max) * 100}%` }} />
          </span>
          <span className="text-right text-label tabular-nums text-faint">{formatNumber(c.count)}</span>
          <span className="text-right text-label tabular-nums text-faint">{keeperShare(c)}</span>
        </button>
      ))}
    </div>
  );
}

function Section({ title, note, children, wide }: { title: string; note?: string; children: React.ReactNode; wide?: boolean }) {
  return (
    <section className={`rounded-md bg-raised/50 p-3.5 ${wide ? "md:col-span-2" : ""}`}>
      <div className="mb-2.5 flex items-baseline gap-2">
        <h3 className="text-caption font-semibold text-ink">{title}</h3>
        {note && <span className="text-label text-faint">{note}</span>}
      </div>
      {children}
    </section>
  );
}

/**
 * What you shoot, with what, and what works: the photos in view — the
 * current filter, so "all of 2025" or "the Karoo trip" — counted by when, by
 * camera and lens, by settings, and by what each lens took. Keepers (picked,
 * or three stars and up) are the bright part of each bar. Any bar applies
 * itself as a filter and goes back to the grid.
 */
export default function InsightsView() {
  const show = useStore((s) => s.showInsights);
  const setShow = useStore((s) => s.setShowInsights);
  const committed = useStore((s) => s.committedFilter);
  const [data, setData] = useState<Insights | null>(null);
  const [kw, setKw] = useState<LensKeywords[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEscape(show, () => setShow(false));

  useEffect(() => {
    if (!show) return;
    let live = true;
    const filter = viewFilter(committed);
    setData(null);
    setKw(null);
    setError(null);
    api
      .insights(filter)
      .then((d) => {
        if (!live) return;
        setData(d);
        // What each lens took reads every keyword: asked for once the rest shows.
        return api.lensKeywords(filter, d.lensDetail.map((l) => l.lens)).then((k) => live && setKw(k));
      })
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [show, committed]);

  // Mounted a moment past closing, for the exit (see usePresence).
  const presence = usePresence(show);
  if (!presence.mounted) return null;

  const pick = (kind: InsightKind, key: string, more: Partial<PhotoFilter> = {}) => {
    const s = useStore.getState();
    s.setFilter({ ...s.filter, ...patchFor(kind, key), ...more });
    s.setMode("browse");
    setShow(false);
  };
  return (
    <div
      data-closing={presence.closing || undefined}
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-6"
      onClick={(e) => e.target === e.currentTarget && setShow(false)}
    >
      <div
        role="dialog"
        aria-label={t("viewer.insights.title")}
        className="anim-card flex h-[min(900px,calc(100vh-3rem))] max-h-[calc(100vh-3rem)] w-full max-w-5xl flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-5 py-3.5">
          <div className="min-w-0">
            <h2 className="font-display text-display text-ink">{t("viewer.insights.title")}</h2>
            <p className="text-caption text-faint">
              {data
                ? t("viewer.insights.summary", {
                    count: data.total,
                    share: keeperShare({ count: data.total, keepers: data.keepers }),
                  })
                : t("viewer.insights.counting")}
            </p>
          </div>
          <button onClick={() => setShow(false)} className="icon-chip ml-auto" aria-label={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {error && <p className="text-caption text-danger">{error}</p>}
          {data && (
            <div className="grid gap-3 md:grid-cols-2">
              <Section title={t("viewer.insights.years")}>
                <Columns items={data.years} label={(k) => k} onPick={(k) => pick("year", k)} />
              </Section>
              {/* The same two columns as Cameras and Lenses, so the same key. */}
              <Section title={t("viewer.insights.busiestDays")} note={t("viewer.insights.rowsNote")}>
                <Rows items={data.busiestDays} label={dayLabel} onPick={(k) => pick("day", k)} limit={6} />
              </Section>
              <Section title={t("viewer.insights.months")} wide>
                <Columns
                  items={data.months}
                  height={64}
                  label={(k) => {
                    const [y, m] = k.split("-");
                    return m === "01" || data.months.length <= 14 ? `${monthName(Number(m))} ${y!.slice(2)}` : "";
                  }}
                  onPick={(k) => pick("month", k)}
                />
              </Section>
              <Section title={t("viewer.insights.cameras")} note={t("viewer.insights.rowsNote")}>
                <Rows items={data.cameras} onPick={(k) => pick("camera", k)} />
              </Section>
              <Section title={t("viewer.insights.lenses")} note={t("viewer.insights.rowsNote")}>
                <Rows items={data.lenses} onPick={(k) => pick("lens", k)} />
              </Section>
              <Section title={t("viewer.insights.focal")}>
                <Columns items={data.focal} label={rangeLabel(FOCAL_RANGES)} onPick={(k) => pick("focal", k)} />
              </Section>
              <Section title={t("viewer.insights.aperture")}>
                <Columns items={data.aperture} label={rangeLabel(APERTURE_RANGES)} onPick={(k) => pick("aperture", k)} />
              </Section>
              <Section title="ISO">
                <Columns items={data.iso} label={rangeLabel(ISO_RANGES)} onPick={(k) => pick("iso", k)} />
              </Section>
              <Section title={t("viewer.insights.shutter")} note={t("viewer.insights.shutterNote")}>
                <Columns items={data.shutter.slice(0, 16)} label={(k) => k} />
              </Section>
              {data.lensDetail.map((l) => {
                const k = kw?.find((x) => x.lens === l.lens);
                return (
                  <Section key={l.lens} title={l.lens} note={l.focals.length > 1 ? t("viewer.insights.usedAt") : undefined}>
                    {l.focals.length > 1 && (
                      <Columns
                        items={l.focals.length > 24 ? l.focals.filter((f) => f.count > 0).slice(0, 24) : l.focals}
                        height={56}
                        label={(mm) => `${mm}`}
                        onPick={(mm) => pick("lens", l.lens, { focalRanges: [focalRangeOf(Number(mm))] })}
                      />
                    )}
                    <div className="mt-2 flex flex-wrap gap-1">
                      {!k && <span className="text-label text-faint">{t("viewer.insights.readingKeywords")}</span>}
                      {k?.genres.map((g) => (
                        <button
                          key={`g${g.key}`}
                          onClick={() => pick("lens", l.lens, { genres: [g.key] })}
                          className="chip hover:border-line-hover"
                          title={tip(tagName(g.key), g)}
                        >
                          {sentenceCase(tagName(g.key))}
                        </button>
                      ))}
                      {k?.subjects.map((s) => (
                        <button
                          key={`s${s.key}`}
                          onClick={() => pick("lens", l.lens, { subjects: [s.key] })}
                          className="chip hover:border-line-hover"
                          title={tip(tagName(s.key), s)}
                        >
                          {sentenceCase(tagName(s.key))}
                          <span className="ml-1 tabular-nums text-faint">{s.count}</span>
                        </button>
                      ))}
                      {k && k.genres.length + k.subjects.length === 0 && (
                        <span className="text-label text-faint">{t("viewer.insights.noKeywords")}</span>
                      )}
                    </div>
                  </Section>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
