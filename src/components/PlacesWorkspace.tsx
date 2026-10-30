import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFitsOneLine } from "../lib/useFitsOneLine";
import { api, fileUrl } from "../lib/api";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { confirm, prompt, toast } from "../lib/dialogs";
import { EMPTY_FILTER, type PhotoFilter, type TripCluster } from "../types";
import type { Place } from "../bindings";
import { open } from "@tauri-apps/plugin-dialog";
import GpxDialog from "./GpxDialog";
import PlacesMap from "./PlacesMap";
import PhotoSet from "./PhotoSet";
import { formatDate, formatNumber, intlLocale, t, type Key } from "../i18n";
import { countryName, localPlaceName } from "../lib/countries";

/**
 * Places — named two ways. Photos that carry a GPS position (phones, drones,
 * app-linked bodies) are placed by themselves, offline, and drawn on the map.
 * Cameras with no GPS, like the a6400, are placed by trip: rather than
 * tag hundreds of photos, the shooter names a handful of runs of photos
 * separated by a gap in shooting time. The named places are on the left; the
 * centre switches between trips and the map, and opening a place or a trip
 * shows its photos there too, rather than sending you off to Browse.
 *
 * The centre used to be a 768px column of 56px rows floating in an unbounded
 * pane — a 300-photo trip represented by one thumbnail and a date range in
 * 12px grey, with the rest of the screen empty. Naming a trip means
 * *recognising* it first, and one frame of its first minute is not enough to do
 * that from. So each trip is now a contact sheet across its whole span, sized
 * to fill the pane it lives in.
 */

/** A month's short name in the active language ("Aug"), from a 0-based month. */
const monthName = (m: number) => formatDate(new Date(Date.UTC(2000, m, 1)), { month: "short", timeZone: "UTC" });

/** The date part of a `YYYY-MM-DD HH:MM:SS` capture stamp, as [y, m, d]. */
function parts(stamp: string): [number, number, number] {
  const [y, m, d] = stamp.slice(0, 10).split("-").map(Number);
  return [y ?? 0, (m ?? 1) - 1, d ?? 1];
}

/**
 * A trip's span, the way a person would say it: "12–15 Aug 2025", "30 Aug –
 * 2 Sep 2025", "6 Sep 2026". The raw `2025-08-12 → 2025-08-15` it replaced made
 * every trip look the same at a glance.
 */
export function tripDates(start: string, end: string): string {
  const [sy, sm, sd] = parts(start);
  const [ey, em, ed] = parts(end);
  // Days and years go in as strings so they are not grouped like counts.
  const [day, startDay, endDay, year, startYear, endYear] = [sd, sd, ed, sy, sy, ey].map(String) as [
    string, string, string, string, string, string,
  ];
  const [month, startMonth, endMonth] = [monthName(sm), monthName(sm), monthName(em)] as [string, string, string];
  if (sy === ey && sm === em && sd === ed) return t("places.trip.datesOneDay", { day, month, year });
  if (sy === ey && sm === em) return t("places.trip.datesOneMonth", { startDay, endDay, month, year });
  if (sy === ey) return t("places.trip.datesOneYear", { startDay, startMonth, endDay, endMonth, year });
  return t("places.trip.datesAcrossYears", { startDay, startMonth, startYear, endDay, endMonth, endYear });
}

/** Whole days a trip spans, inclusive — "4 days" reads better than a gap. */
export function tripLength(start: string, end: string): string {
  const a = Date.UTC(...parts(start));
  const b = Date.UTC(...parts(end));
  const days = Math.round((b - a) / 86_400_000) + 1;
  return t("places.trip.length", { count: Math.max(1, days) });
}

const photos = (n: number) => t("common.photos", { count: n });

/** What naming a trip did — including the photos it deliberately left alone,
 *  so "Placed 12" on a 14-photo trip is not a mystery. */
export function namedMessage(name: string, placed: number, kept: number, keptPlaced = 0): string {
  const parts = [t("places.trip.placedNamed", { count: placed, name })];
  if (kept > 0) parts.push(t("places.trip.keptOwnGps", { count: kept }));
  // Places set by hand are kept, and the toast says so.
  if (keptPlaced > 0) parts.push(t("places.trip.keptOwnPlace", { count: keptPlaced }));
  return parts.join(" ");
}

/** The places in a trip other than its name, as "Edinburgh" or "Edinburgh +
 *  2 more" — a trip is often in more than one. */
export function otherPlaces(c: Pick<TripCluster, "places" | "existingPlace">): string | null {
  const rest = c.places.filter((p) => p.name !== c.existingPlace);
  if (rest.length === 0) return null;
  const first = localPlaceName(rest[0]!.name);
  return rest.length === 1 ? first : t("places.trip.morePlaces", { name: first, count: rest.length - 1 });
}

/** A trip's own GPS says where it was: the place its phone shots are in, for a
 *  trip not yet named that also has frames without GPS to place there. */
export function tripSuggestion(
  c: Pick<TripCluster, "existingPlace" | "places" | "located" | "count">,
): string | null {
  if (c.existingPlace || c.located === 0 || c.located >= c.count) return null;
  return c.places[0]?.name ?? null;
}

/** A place's name without its country — "Edinburgh, United Kingdom" is
 *  "Edinburgh" under a United Kingdom heading. With no country given, what
 *  follows the last comma is taken to be one. */
export function shortName(name: string, country?: string | null): string {
  if (country) {
    const suffix = `, ${country}`;
    return name.toLowerCase().endsWith(suffix.toLowerCase()) ? name.slice(0, -suffix.length) : name;
  }
  const at = name.lastIndexOf(", ");
  return at > 0 ? name.slice(0, at) : name;
}

/** Places grouped by country, most photographed first, with the places whose
 *  country isn't known last. Order within a group is kept. */
export function groupByCountry<P extends Pick<Place, "country" | "count">>(
  places: P[],
): { country: string | null; count: number; places: P[] }[] {
  const groups = new Map<string | null, P[]>();
  for (const p of places) groups.set(p.country, [...(groups.get(p.country) ?? []), p]);
  return [...groups]
    .map(([country, ps]) => ({ country, places: ps, count: ps.reduce((n, p) => n + p.count, 0) }))
    .sort(
      (a, b) =>
        Number(a.country === null) - Number(b.country === null) ||
        b.count - a.count ||
        (a.country ?? "").localeCompare(b.country ?? "", intlLocale()),
    );
}

const GAPS: { value: number; label: Key; count: number }[] = [
  { value: 4, label: "places.gap.hours", count: 4 },
  { value: 12, label: "places.gap.hours", count: 12 },
  { value: 24, label: "places.gap.days", count: 1 },
  { value: 72, label: "places.gap.days", count: 3 },
];

/** A trip's identity across re-fetches: its span. Two trips never share one,
 *  because clusters are cut at gaps in time. */
export function tripKey(c: Pick<TripCluster, "startDate" | "endDate">): string {
  return `${c.startDate}|${c.endDate}`;
}

export default function PlacesWorkspace() {
  const reload = useStore((s) => s.reload);

  const [view, setView] = useState<"trips" | "map">("trips");
  const hintRef = useRef<HTMLDivElement>(null);
  const language = useStore((s) => s.language);
  const hintFits = useFitsOneLine(hintRef, [view, language]);
  // A place's or a trip's photos, shown in the centre in place of the trips or
  // the map. `key` marks which place in the list is open.
  const [viewing, setViewing] = useState<{
    key: string;
    title: string;
    subtitle?: string;
    filter: PhotoFilter;
  } | null>(null);
  const [clusters, setClusters] = useState<TripCluster[] | null>(null);

  // Photos catalogued before positions were read: offered as one pass.
  const [unread, setUnread] = useState<number | null>(null);
  const reading = useStore((s) => s.jobs["locations"]);
  const refreshUnread = useCallback(
    () => void api.locationsStatus().then(setUnread).catch(() => setUnread(null)),
    [],
  );
  useEffect(() => refreshUnread(), [refreshUnread]);
  const readLocations = async () => {
    try {
      const out = await api.readLocations();
      toast(
        out.found > 0
          ? t("places.readLocations.found", { found: out.found, count: out.read, placed: out.placed })
          : t("places.readLocations.none", { count: out.read }),
        "success",
      );
    } catch (e) {
      toast(String(e), "error");
    }
    refreshUnread();
    refreshPlaces();
    await reload();
  };

  // A GPX track places photos from a camera with no GPS.
  const [tracks, setTracks] = useState<string[] | null>(null);
  // One a paired phone sent opens the same dialog (docs/MOBILE.md).
  const phoneTrack = useStore((s) => s.phoneTrack);
  useEffect(() => {
    if (!phoneTrack) return;
    setTracks([phoneTrack]);
    useStore.getState().setPhoneTrack(null);
  }, [phoneTrack]);
  const pickTrack = async () => {
    const picked = await open({
      multiple: true,
      title: t("places.gpx.pickTitle"),
      filters: [{ name: t("places.gpx.pickFilter"), extensions: ["gpx"] }],
    });
    if (picked && picked.length > 0) setTracks(picked);
  };
  const [gapHours, setGapHours] = useState(12);
  // Keyed by the trip, not its position in the list. Naming one
  // trip re-fetches the list and changing the gap reshapes it; keyed by index,
  // a half-typed name in another card then sat against a different trip.
  const [names, setNames] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<string | null>(null);

  // Every named place in the library, not the place *facet*.
  //
  // A facet describes the current query, which is right in the filter rail and
  // wrong here: opening a trip in Browse sets a date filter, and coming back to
  // Places then showed one place out of three with a count of 1 — the facet
  // faithfully describing a query the user had forgotten making.
  const [places, setPlaces] = useState<Place[]>([]);
  const refreshPlaces = useCallback(
    () => void api.listPlaces().then(setPlaces).catch(() => setPlaces([])),
    [],
  );
  useEffect(() => refreshPlaces(), [refreshPlaces]);
  const groups = useMemo(() => groupByCountry(places), [places]);
  // Countries folded away in the list, by name ("" for the unknown ones).
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  useEffect(() => {
    setClusters(null);
    void api.getTripClusters(gapHours).then(setClusters);
  }, [gapHours]);

  /** Names a trip — what was typed into its card, or `suggested`. */
  const assign = async (c: TripCluster, suggested?: string) => {
    const k = tripKey(c);
    const name = (suggested ?? names[k] ?? "").trim();
    if (!name) return;
    setSaving(k);
    try {
      const out = await api.nameTrip(c.photoIds, name);
      setClusters(await api.getTripClusters(gapHours));
      setNames((n) => ({ ...n, [k]: "" }));
      refreshPlaces();
      await reload(); // the grid behind this shows the new place too
      toast(namedMessage(name, out.placed, out.kept, out.keptPlaced), "success");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setSaving(null);
    }
  };

  const viewPlace = (id: number) =>
    setViewing({
      key: `place:${id}`,
      title: localPlaceName(places.find((p) => p.id === id)?.name ?? "") || t("places.list.fallbackTitle"),
      filter: { ...EMPTY_FILTER, placeIds: [id] },
    });

  /** Pin a place to the map, or move its pin. The list is re-read rather than
   *  patched, so the map and the list never disagree about where it is. */
  const pinPlace = async (id: number, lat: number, lon: number) => {
    try {
      await api.setPlacePosition(id, lat, lon);
    } catch (e) {
      toast(String(e), "error");
    }
    await api.listPlaces().then(setPlaces).catch(() => {});
  };

  /** After a place is renamed, merged or deleted: the list, the trip cards
   *  naming it, and the grid behind all show it. */
  const placesChanged = async () => {
    refreshPlaces();
    setClusters(await api.getTripClusters(gapHours).catch(() => null));
    await reload();
  };

  const renamePlace = async (p: Place) => {
    const name = await prompt({ title: t("places.rename.title"), defaultValue: p.name, confirmLabel: t("common.rename") });
    if (!name || name === p.name) return;
    const isOpen = viewing?.key === `place:${p.id}`;
    try {
      const clash = await api.renamePlace(p.id, name);
      if (clash == null) {
        if (isOpen) setViewing({ ...viewing!, title: name });
      } else {
        // Place names are unique, so the same name is the same place — but
        // only if the user says so.
        const other = places.find((o) => o.id === clash);
        const called = other?.name ?? name;
        const merge = await confirm({
          title: t("places.merge.title", { name: called }),
          message: other
            ? t("places.merge.message", { called, theirs: photos(other.count), name: localPlaceName(p.name), mine: photos(p.count) })
            : t("places.merge.messageBare", { called, name: localPlaceName(p.name), mine: photos(p.count) }),
          confirmLabel: t("places.merge.confirm"),
        });
        if (!merge) return;
        await api.mergePlaces([p.id], clash);
        if (isOpen) setViewing(null);
        toast(t("places.merge.done", { name: localPlaceName(p.name), called }), "success");
      }
    } catch (e) {
      toast(String(e), "error");
    }
    await placesChanged();
  };

  const deletePlace = async (p: Place) => {
    const ok = await confirm({
      title: t("places.delete.title", { name: localPlaceName(p.name) }),
      message: t("places.delete.message", { count: p.count }),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deletePlace(p.id);
      if (viewing?.key === `place:${p.id}`) setViewing(null);
    } catch (e) {
      toast(String(e), "error");
    }
    await placesChanged();
  };

  /** Open a trip by its date span, named or not. */
  const viewTrip = (c: TripCluster) =>
    setViewing({
      key: `trip:${tripKey(c)}`,
      title: tripDates(c.startDate, c.endDate),
      subtitle: c.existingPlace ?? undefined,
      filter: { ...EMPTY_FILTER, dateFrom: c.startDate.slice(0, 10), dateTo: c.endDate.slice(0, 10) },
    });


  /** One place in the list; within a country, named without it. */
  const placeRow = (p: Place, country: string | null) => (
          <div key={p.id} className="group/place relative flex items-center">
            <button
              onClick={() => viewPlace(p.id)}
              className={`facet-row min-w-0 flex-1 group-hover/place:bg-hover group-hover/place:text-ink ${viewing?.key === `place:${p.id}` ? "facet-row-active" : ""}`}
              title={t("places.list.show", { photos: photos(p.count), name: localPlaceName(p.name) })}
            >
              <Icon name="pin" size={13} className="shrink-0 text-faint" />
              <span className="truncate">{country ? shortName(p.name, country) : localPlaceName(p.name, p.country)}</span>
              <span className="ml-auto shrink-0 text-label tabular-nums text-faint group-hover/place:invisible">
                {formatNumber(p.count)}
              </span>
            </button>
            {/* Hover actions, like a collection's delete. Merging is a
                rename to a name another place already has. Over the count
                while hovered: beside it, they held each place's count 40px
                in from its country's. */}
            <div className="absolute inset-y-0 right-0 flex items-center rounded-r-sm bg-hover pl-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover/place:opacity-100">
              <button
                onClick={() => void renamePlace(p)}
                className="px-1 text-faint hover:text-ink"
                title={t("places.list.renameHint")}
              >
                <Icon name="pencil" size={12} />
              </button>
              <button
                onClick={() => void deletePlace(p)}
                className="px-1 text-faint hover:text-danger"
                title={t("places.list.delete")}
              >
                <Icon name="close" size={12} />
              </button>
            </div>
          </div>
  );

  return (
    <div className="flex min-h-0 flex-1">
      {/* Named places */}
      <aside className="flex w-[var(--panel-w)] shrink-0 flex-col border-r border-line bg-panel">
        <div
          className="flex h-[52px] shrink-0 items-center gap-2.5 border-b border-line px-4"
          data-tauri-drag-region="deep"
        >
          <Icon name="pin" size={16} className="text-accent" />
          <span className="text-body font-semibold">{t("places.list.heading")}</span>
        </div>
        <div className="flex-1 overflow-y-auto p-2">
          {(unread ?? 0) > 0 && (
            <div className="mx-1 mb-3 rounded-md bg-raised p-2.5">
              <div className="text-label leading-relaxed text-faint">
                {t("places.list.unread", { count: unread! })}
              </div>
              <button
                onClick={() => void readLocations()}
                disabled={!!reading}
                className="btn-solid mt-2 w-full justify-center text-caption"
              >
                {reading
                  ? reading.total > 1
                    ? t("places.list.reading", { current: reading.current, total: reading.total })
                    : t("places.list.readingShort")
                  : t("places.list.readLocations")}
              </button>
            </div>
          )}
          <button
            onClick={() => void pickTrack()}
            className="mx-1 mb-3 flex w-[calc(100%-0.5rem)] items-center gap-2 rounded-md px-2 py-1.5 text-left text-caption text-muted hover:bg-hover hover:text-ink"
            title={t("places.list.gpxHint")}
          >
            <Icon name="trips" size={14} />
            {t("places.list.gpxButton")}
          </button>
          <div className="section-label px-2 pb-1">{t("places.list.named")}</div>
          {places.length === 0 ? (
            <div className="px-2 py-1 text-caption leading-relaxed text-faint">
              {t("places.list.empty")}
            </div>
          ) : groups.length > 1 ? (
            // By country once there is more than one: a library of trips abroad
            // is otherwise one long list of towns.
            groups.map((g) => {
              const key = g.country ?? "";
              const open = !collapsed.has(key);
              return (
                <div key={key} className="mb-1">
                  <button
                    onClick={() =>
                      setCollapsed((prev) => {
                        const next = new Set(prev);
                        if (!next.delete(key)) next.add(key);
                        return next;
                      })
                    }
                    className="flex w-full items-center gap-1.5 rounded-sm px-2 py-1 text-left text-caption font-semibold text-muted hover:bg-hover hover:text-ink"
                    aria-expanded={open}
                  >
                    <Icon name="chevron" size={11} className={`shrink-0 text-faint transition-transform ${open ? "" : "-rotate-90"}`} />
                    <span className="truncate">{g.country ? countryName(g.country) : t("places.list.elsewhere")}</span>
                    <span className="ml-auto shrink-0 text-label font-normal tabular-nums text-faint">{formatNumber(g.count)}</span>
                  </button>
                  {open && <div className="pl-2">{g.places.map((p) => placeRow(p, g.country))}</div>}
                </div>
              );
            })
          ) : (
            places.map((p) => placeRow(p, null))
          )}
          <datalist id="place-names-trips">
            {places.map((p) => (
              <option key={p.id} value={p.name} />
            ))}
          </datalist>
        </div>
      </aside>

      {viewing && (
        <PhotoSet
          title={viewing.title}
          subtitle={viewing.subtitle}
          backLabel={view === "map" ? t("places.view.map") : t("places.view.trips")}
          filter={viewing.filter}
          onBack={() => setViewing(null)}
        />
      )}

      {/* Trip naming. Hidden rather than unmounted while a place or trip is
          open, so Back returns to the same scroll position and the names
          half-typed into other trips. */}
      <main className={`min-w-0 flex-1 flex-col ${viewing ? "hidden" : "flex"}`}>
        <div
          className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line bg-panel px-4"
          data-tauri-drag-region="deep"
        >
          <div className="seg-group shrink-0">
            {(["trips", "map"] as const).map((v) => (
              <button aria-pressed={view === v}
                key={v}
                onClick={() => setView(v)}
                className={`px-3 py-1 text-caption ${view === v ? "seg-on" : "text-muted hover:bg-hover"}`}
              >
                {v === "trips" ? t("places.view.trips") : t("places.view.map")}
              </button>
            ))}
          </div>
          <div className="min-w-0">
            {/* Whole or not at all: cut short at a narrow window it read as
                "Photos grouped by w…". The trips and the map explain
                themselves without it. */}
            <div
              ref={hintRef}
              aria-hidden={!hintFits || undefined}
              className={`truncate text-caption text-faint ${hintFits ? "" : "invisible"}`}
            >
              {view === "trips"
                ? t("places.view.tripsHint")
                : t("places.view.mapHint")}
            </div>
          </div>
          {view === "trips" && (
            <label className="ml-auto flex shrink-0 items-center gap-2 text-caption text-faint">
              {t("places.view.splitAfter")}
              <div className="seg-group">
                {GAPS.map((g) => (
                  <button aria-pressed={gapHours === g.value}
                    key={g.value}
                    onClick={() => setGapHours(g.value)}
                    className={`px-2.5 py-1 text-caption ${gapHours === g.value ? "seg-on" : "text-muted hover:bg-hover"}`}
                  >
                    {t(g.label, { count: g.count })}
                  </button>
                ))}
              </div>
            </label>
          )}
        </div>

        {view === "map" ? (
          <div className="min-h-0 flex-1 bg-canvas-void">
            <PlacesMap places={places} onPick={viewPlace} onPlace={pinPlace} />
          </div>
        ) : (
          <div className="flex-1 overflow-y-auto p-4">
            {clusters === null && (
              <div className="p-8 text-center text-caption text-faint">{t("places.trips.grouping")}</div>
            )}
            {clusters?.length === 0 && (
              <div className="mx-auto max-w-sm p-8 text-center">
                <div className="mb-3 flex justify-center text-faint opacity-60">
                  <Icon name="trips" size={30} strokeWidth={1.4} />
                </div>
                <div className="text-title text-muted">{t("places.trips.emptyTitle")}</div>
                <div className="mt-1 text-caption leading-relaxed text-faint">
                  {t("places.trips.emptyBody")}
                </div>
              </div>
            )}

            <div className="flex flex-col gap-3">
              {clusters?.map((c) => {
                const k = tripKey(c);
                const sheet = c.thumbPaths.length > 0 ? c.thumbPaths : c.thumbPath ? [c.thumbPath] : [];
                return (
                  <section
                    key={k}
                    className="overflow-hidden rounded-lg border border-line bg-panel"
                  >
                    {/* Contact sheet: frames spread across the trip, not its
                        first eight seconds. Clicking opens the whole span.

                        Fixed-size tiles, left-aligned — not `flex-1`. Stretching
                        them to fill the row meant a trip's thumbnails got wider
                        the fewer of them there were, so a one-photo trip became a
                        single 1800px-wide letterbox of somebody's eye. A photo
                        shown in its own proportions is the whole job here. */}
                    {sheet.length > 0 && (
                      <button
                        onClick={() => viewTrip(c)}
                        title={t("places.trips.show", { count: c.count })}
                        className="flex w-full gap-px overflow-hidden bg-line"
                      >
                        {sheet.map((thumb, n) => (
                          <img
                            key={`${thumb}-${n}`}
                            src={fileUrl(thumb)}
                            alt=""
                            draggable={false}
                            className="h-24 w-36 shrink-0 object-cover"
                          />
                        ))}
                      </button>
                    )}

                    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 p-3">
                      {/* A real minimum, not `min-w-0`: with a zero basis the row
                          never wrapped, and at the minimum window the title
                          shrank until the place read "Ca…". Now the rename
                          field drops to its own line instead. */}
                      <div className="min-w-[16rem] flex-1">
                        <div className="truncate text-title font-medium text-ink">
                          {tripDates(c.startDate, c.endDate)}
                          {c.existingPlace && (
                            <span className="ml-2 text-caption font-normal text-accent" title={c.existingPlace}>
                              {c.existingPlace}
                            </span>
                          )}
                          {otherPlaces(c) && (
                            <span
                              className="ml-2 text-caption font-normal text-muted"
                              title={c.places.map((p) => `${localPlaceName(p.name)} · ${photos(p.count)}`).join("\n")}
                            >
                              {c.existingPlace ? t("places.trip.alsoIn", { places: otherPlaces(c)! }) : otherPlaces(c)}
                            </span>
                          )}
                        </div>
                        <div
                          className="mt-0.5 truncate text-caption text-faint"
                          title={[
                            photos(c.count),
                            tripLength(c.startDate, c.endDate),
                            ...(c.cameras.length > 0 ? [c.cameras.join(", ")] : []),
                            ...(c.located > 0 ? [t("places.trips.placedByGps", { count: c.located })] : []),
                          ].join(" · ")}
                        >
                          {photos(c.count)} · {tripLength(c.startDate, c.endDate)}
                          {c.cameras.length > 0 && ` · ${c.cameras.join(", ")}`}
                          {c.located > 0 && ` · ${t("places.trips.placedByGps", { count: c.located })}`}
                        </div>
                      </div>

                      {c.located === c.count ? (
                        <div className="shrink-0 text-caption text-faint">
                          {t("places.trips.allPlacedByGps")}
                        </div>
                      ) : (
                        <div className="flex shrink-0 items-center gap-2">
                          {/* The phone already said where: one click puts the
                              camera's frames there too. */}
                          {tripSuggestion(c) && (
                            <button
                              className="btn-ghost max-w-48 text-accent"
                              disabled={saving === k}
                              onClick={() => void assign(c, tripSuggestion(c)!)}
                              title={t("places.trips.useSuggestionHint", { photos: photos(c.count - c.located), name: localPlaceName(tripSuggestion(c)!) })}
                            >
                              <span className="truncate">{t("places.trips.useSuggestion", { name: shortName(tripSuggestion(c)!) })}</span>
                            </button>
                          )}
                          <input
                            className="input w-56"
                            list="place-names-trips"
                            placeholder={c.existingPlace ?? t("places.trips.namePlaceholder")}
                            value={names[k] ?? ""}
                            onChange={(e) => setNames({ ...names, [k]: e.target.value })}
                            onKeyDown={(e) => e.key === "Enter" && void assign(c)}
                          />
                          <button
                            className="btn-solid"
                            disabled={!(names[k] ?? "").trim() || saving === k}
                            onClick={() => void assign(c)}
                          >
                            {saving === k ? "…" : c.existingPlace ? t("common.rename") : t("places.trips.nameIt")}
                          </button>
                        </div>
                      )}
                    </div>
                  </section>
                );
              })}
            </div>
          </div>
        )}
      </main>
      {tracks && (
        <GpxDialog
          paths={tracks}
          onClose={() => setTracks(null)}
          onDone={() => {
            setTracks(null);
            refreshPlaces();
            void reload();
          }}
        />
      )}
    </div>
  );
}
