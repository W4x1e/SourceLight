import GroupedList from "./GroupedList";
import { useWindowWidth } from "../lib/useWindowWidth";
import { sentenceCase } from "../lib/format";
import { useEffect, useRef, useState } from "react";
import { EXTRA_SECTIONS, useStore } from "../store";
import { nextStep, progressLine } from "../lib/shoots";
import { folderName } from "./LibraryMenu";
import { Icon, type IconName } from "../lib/icons";
import { confirm, prompt, toast } from "../lib/dialogs";
import { api } from "../lib/api";
import { useFlip } from "../lib/useFlip";
import DatePicker from "./DatePicker";
import { periodLabel } from "../lib/timeline";
import { formatCapture } from "../lib/captureTime";
import KeywordTree from "./KeywordTree";
import {
  APERTURE_RANGES,
  COLOR_LABELS,
  FACE_COUNTS,
  PALETTE,
  FILE_KINDS,
  SHAPES,
  FOCAL_RANGES,
  ISO_RANGES,
  isFilterEmpty,
  normaliseFilter,
  type FacetCount,
  type PhotoFilter,
} from "../types";
import { comboToLabel } from "../lib/keymap";
import { formatNumber, intlLocale, t, tx, type Key } from "../i18n";
import { localPlaceName } from "../lib/countries";
import { conditionName } from "../lib/content";
import { tagName } from "../i18n/tags";
import { facetClick } from "../lib/facetClick";

/** Five stars down to one, then unrated: the order someone looks for them in. */
const RATING_OPTIONS = [5, 4, 3, 2, 1, 0];

/** How many facets are narrowing the library, for the collapsed rail's badge. */
function activeFilterCount(f: PhotoFilter): number {
  return (
    (f.dateFrom || f.dateTo ? 1 : 0) +
    f.genres.length +
    f.subjects.length +
    f.conditions.length +
    (f.excludeSubjects?.length ?? 0) +
    (f.excludeGenres?.length ?? 0) +
    (f.excludeConditions?.length ?? 0) +
    f.placeIds.length +
    (f.ratings?.length ?? 0) +
    (f.minRating ? 1 : 0) +
    f.flags.length +
    f.colorLabels.length +
    (f.collectionId !== null ? 1 : 0) +
    (f.personId != null ? 1 : 0) +
    (f.personIds?.length ?? 0) +
    gearCount(f) +
    kindCount(f) +
    statusCount(f) +
    (f.folderIds?.length ?? 0)
  );
}

/** How many camera, lens and exposure choices are made, for the Camera badge. */
function gearCount(f: PhotoFilter): number {
  return (
    (f.cameras?.length ?? 0) +
    (f.lenses?.length ?? 0) +
    (f.focalRanges?.length ?? 0) +
    (f.isoRanges?.length ?? 0) +
    (f.apertureRanges?.length ?? 0)
  );
}

/** How many file type, shape and face choices are made. */
function kindCount(f: PhotoFilter): number {
  return (f.fileKinds?.length ?? 0) + (f.shapes?.length ?? 0) + (f.faceCounts?.length ?? 0);
}

/** How many status toggles are on, for the Status badge. */
function statusCount(f: PhotoFilter): number {
  return [f.untaggedOnly, f.missingOnly, f.editedOnly, f.copiesOnly, f.facesOnly, f.locatedOnly, f.unplacedOnly].filter(
    Boolean,
  ).length;
}

/** A range facet's rows: its options in their own order, with counts, the
 *  empty ones left out unless chosen (so a choice can always be taken back). */
function rangeItems(
  // Worded by key (exposure), or already worded (file kind, shape, faces).
  ranges: { key: string; labelKey?: Key; label?: string }[],
  counts: FacetCount[] | undefined,
  chosen: string[],
): { key: string; label: string; count: number }[] {
  return ranges
    .map((r) => ({
      key: r.key,
      label: r.labelKey ? t(r.labelKey) : (r.label ?? r.key),
      count: counts?.find((c) => c.value === r.key)?.count ?? 0,
    }))
    .filter((r) => r.count > 0 || chosen.includes(r.key));
}

/** A small heading inside a section that holds several lists. */
function SubLabel({ children }: { children: React.ReactNode }) {
  return <div className="px-2 pb-0.5 pt-2 text-label text-faint first:pt-1">{children}</div>;
}

const FLAG_OPTIONS: { value: number; label: Key; icon: IconName }[] = [
  { value: 1, label: "filters.flag.picks", icon: "flag" },
  { value: -1, label: "filters.flag.rejects", icon: "flag-off" },
  { value: 0, label: "filters.flag.unflagged", icon: "minus" },
];

/** A numeric facet's count, from the backend's string-keyed list. */
function countOf(list: { value: string; count: number }[] | undefined, value: number): number {
  return list?.find((f) => f.value === String(value))?.count ?? 0;
}

/** Toggles a value in/out of a facet array (OR semantics within a facet). */
function toggle<T>(list: T[], value: T): T[] {
  return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
}


/**
 * The multi-select marker on every facet row. No resting box — an empty row is
 * just its label and count, so a rail of unset facets reads as a quiet list
 * rather than a column of empty checkboxes. Selection is carried by the row's
 * wash (`facet-row-active`) plus a borderless steel tick here, matching the flat
 * "filled-on-state" language of the buttons. The 14px slot is always reserved so
 * toggling a facet never shifts its label.
 */
export function CheckBox({ active, excluded = false }: { active: boolean; excluded?: boolean }) {
  return (
    <span className={`grid h-3.5 w-3.5 shrink-0 place-items-center ${excluded ? "text-danger" : "text-accent"}`}>
      {excluded ? (
        <Icon name="ban" size={12} strokeWidth={2.2} />
      ) : (
        active && <Icon name="check" size={13} strokeWidth={2.5} />
      )}
    </span>
  );
}

function Section({
  id,
  label,
  children,
  defaultOpen = true,
  badge,
}: {
  /** Stable key for the remembered open state: the English name, whatever the language. */
  id: string;
  label: string;
  children: React.ReactNode;
  defaultOpen?: boolean;
  badge?: number;
}) {
  // Open/closed lives in the store, not here: the rail unmounts on every mode
  // switch, so a section you opened re-collapsed the moment you visited Cull.
  // An absent entry means "still at this section's own default".
  const stored = useStore((s) => s.railSections[id]);
  const setRailSection = useStore((s) => s.setRailSection);
  const open = stored ?? defaultOpen;
  const setOpen = (v: boolean) => setRailSection(id, v);
  return (
    <div data-flip={`sec:${id}`} className="border-b border-line/60 py-2">
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-1.5 px-2 pb-1 text-left"
        aria-expanded={open}
      >
        <Icon
          name="chevron"
          size={12}
          className={`text-faint transition-transform ${open ? "" : "-rotate-90"}`}
        />
        <span className="text-caption font-semibold text-muted">
          {label}
        </span>
        {badge ? (
          <span className="ml-auto rounded-sm bg-accent px-1.5 text-label font-semibold tabular-nums text-on-accent">
            {badge}
          </span>
        ) : null}
      </button>
      {/* Open and close by height, so the sections below slide rather than
          jump. Closed content stays mounted but `inert`, out of the tab order
          and the accessibility tree. */}
      <div
        className={`grid transition-[grid-template-rows] duration-200 ease-out ${
          open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"
        }`}
        inert={!open}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="px-1">{children}</div>
        </div>
      </div>
    </div>
  );
}

/** One row of a facet list, keyed independently of its label — places are
 *  toggled by id, the tag facets by their own text. */
interface FacetItem {
  key: string;
  label: string;
  count: number;
}

const asItems = (counts: FacetCount[]): FacetItem[] =>
  counts.map((c) => ({ key: c.value, label: c.value, count: c.count }));

/** A list with the values left out kept on it. Their photos are gone
 *  from every count, so without this a value would vanish the moment it was
 *  excluded, leaving nothing to click to let it back in. */
const withChosen = (items: FacetItem[], kept: string[]): FacetItem[] => [
  ...items,
  ...kept.filter((k) => !items.some((it) => it.key === k)).map((k) => ({ key: k, label: k, count: 0 })),
];

/**
 * Facet list with a "show more" tail so long lists don't dominate the rail.
 *
 * Generic over the key so Place can use it too. Place used to render its own
 * rows — the only facet without a cap, so a library with many named trips got an
 * unbounded section, and it missed out on the remembered expansion as well.
 */
function FacetList({
  id,
  items,
  active,
  onToggle,
  excluded,
  onClickAlt,
  capitalise = true,
  empty,
}: {
  /** Stable key for remembering the expanded state across a mode switch. */
  id: string;
  items: FacetItem[];
  active: (key: string) => boolean;
  onToggle: (key: string) => void;
  /** Values the filter leaves out, drawn struck through. */
  excluded?: (key: string) => boolean;
  /** Alt-click, where a list can leave values out. */
  onClickAlt?: (key: string) => void;
  capitalise?: boolean;
  empty?: React.ReactNode;
}) {
  const expanded = useStore((s) => s.railExpanded[id] ?? false);
  const setRailExpanded = useStore((s) => s.setRailExpanded);
  const setExpanded = (v: boolean) => setRailExpanded(id, v);
  const LIMIT = 8;

  if (items.length === 0) {
    return <div className="px-2 py-1 text-caption leading-relaxed text-faint">{empty ?? t("filters.facet.noneYet")}</div>;
  }

  const shown = expanded ? items : items.slice(0, LIMIT);

  return (
    <div>
      {shown.map((it) => {
        const on = active(it.key);
        const out = excluded?.(it.key) ?? false;
        return (
          <button
            key={it.key}
            onClick={(e) => (onClickAlt && (e.altKey || out) ? onClickAlt(it.key) : onToggle(it.key))}
            className={`facet-row ${on || out ? "facet-row-active" : ""}`}
            title={onClickAlt ? (out ? t("filters.facet.excludedTitle") : t("filters.facet.excludeTitle")) : undefined}
            aria-pressed={out ? "mixed" : on}
          >
            <CheckBox active={on} excluded={out} />
            {/* The whole name on hover, where a long one is cut short —
                except where the row's own tooltip teaches Alt-click. */}
            <span className={`truncate ${out ? "text-faint line-through" : ""}`} title={onClickAlt ? undefined : it.label}>
              {capitalise ? sentenceCase(it.label) : it.label}
            </span>
            <span className="ml-auto shrink-0 text-label tabular-nums text-faint">{out ? "–" : formatNumber(it.count)}</span>
          </button>
        );
      })}
      {items.length > LIMIT && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="px-2 py-1 text-caption text-accent hover:underline"
        >
          {expanded ? t("filters.facet.showLess") : t("filters.facet.showMore", { count: items.length - LIMIT })}
        </button>
      )}
    </div>
  );
}

export default function FilterRail() {
  const facets = useStore((s) => s.facets);
  const extra = useStore((s) => s.extraFacets);
  // The list object, not `?.folders ?? []`: a fresh [] per read is a new
  // snapshot every time, and zustand re-renders until React gives up.
  const libraryFolders = useStore((s) => s.libraryFolders);
  const folders = libraryFolders?.folders ?? [];
  const filter = useStore((s) => s.filter);
  const onChange = useStore((s) => s.setFilter);
  const collections = useStore((s) => s.collections);
  const createCollection = useStore((s) => s.createCollection);
  const deleteCollection = useStore((s) => s.deleteCollection);
  const renameCollection = useStore((s) => s.renameCollection);
  const playCollection = useStore((s) => s.playCollection);
  const createCollectionGroup = useStore((s) => s.createCollectionGroup);
  const shoots = useStore((s) => s.shoots);
  const continueShoot = useStore((s) => s.continueShoot);
  const importForShoot = useStore((s) => s.importForShoot);
  const loadShoots = useStore((s) => s.loadShoots);
  const forgetShoot = useStore((s) => s.forgetShoot);
  const smartCollections = useStore((s) => s.smartCollections);
  const saveCurrentAsSmart = useStore((s) => s.saveCurrentAsSmart);
  const deleteSmartCollection = useStore((s) => s.deleteSmartCollection);
  const clearFilter = useStore((s) => s.clearFilter);

  const set = (patch: Partial<PhotoFilter>) => onChange({ ...filter, ...patch });
  const empty = isFilterEmpty(filter);
  const ratings = filter.ratings ?? [];
  const folded = useStore((s) => s.railCollapsed);
  const setFolded = useStore((s) => s.setRailCollapsed);
  // On a narrow window with the selection panel open, the rail folds to its
  // strip unless opened again: rail, panel and grid together left the
  // grid one 280px column wide. Not remembered: it is the window's doing.
  const narrow = useWindowWidth() < 1180;
  const selecting = useStore((s) => s.selected.size > 0);
  const crowded = narrow && selecting;
  const [openAnyway, setOpenAnyway] = useState(false);
  useEffect(() => {
    if (!crowded) setOpenAnyway(false);
  }, [crowded]);
  const collapsed = folded || (crowded && !openAnyway);
  const setCollapsed = (v: boolean) => {
    if (!v && crowded) setOpenAnyway(true);
    setFolded(v);
  };
  const toggleKey = comboToLabel(useStore((s) => s.keymap["toggle-filters"]));

  // When the filter changes, the recomputed counts resize and reorder the facet
  // lists; FLIP the section blocks so the rail settles instead of jumping.
  const scrollRef = useRef<HTMLDivElement>(null);
  useFlip(scrollRef, facets);

  return (
    // One element whose width animates. Each state's content keeps its own
    // fixed width inside and is clipped, so the full rail never reflows into
    // 40px on its way in or out.
    <aside
      data-tour="filter-rail"
      className={`flex shrink-0 overflow-hidden border-r border-line bg-panel transition-[width] duration-200 ease-out ${
        collapsed ? "w-10" : "w-[var(--panel-w)]"
      }`}
    >
      {collapsed ? (
        <div className="flex w-10 shrink-0 flex-col items-center gap-2 py-1.5">
          <button
            onClick={() => setCollapsed(false)}
            className="relative grid h-7 w-7 place-items-center rounded-md text-muted hover:bg-hover hover:text-ink"
            title={t("filters.rail.showTitle", { key: toggleKey })}
            aria-label={t("filters.rail.show")}
            aria-expanded={false}
          >
            <Icon name="filter" size={15} />
            {!empty && (
              <span className="absolute -right-1 -top-1 min-w-4 rounded-full bg-accent px-1 text-center text-label font-semibold leading-4 tabular-nums text-on-accent">
                {activeFilterCount(filter)}
              </span>
            )}
          </button>
        </div>
      ) : (
      <div className="flex w-[var(--panel-w)] shrink-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-line pl-3 pr-1.5">
        <span className="text-caption font-semibold text-muted">{t("filters.rail.title")}</span>
        <button
          onClick={clearFilter}
          disabled={empty}
          className="ml-auto rounded-sm px-1.5 py-0.5 text-caption text-muted transition-colors
                     hover:bg-hover hover:text-ink disabled:pointer-events-none disabled:opacity-40"
        >
          {t("filters.rail.clearAll")}
        </button>
        <button
          onClick={() => setCollapsed(true)}
          className="grid h-6 w-6 place-items-center rounded-sm text-faint hover:bg-hover hover:text-ink"
          title={t("filters.rail.hideTitle", { key: toggleKey })}
          aria-label={t("filters.rail.hide")}
          aria-expanded
        >
          <Icon name="chevrons" size={14} />
        </button>
      </div>
      <div ref={scrollRef} className="flex-1 overflow-y-auto pb-6">
        <Section id="Date" label={t("filters.section.date")} badge={filter.dateFrom || filter.dateTo ? 1 : 0}>
          <div className="flex flex-col gap-1.5 px-1 py-1">
            <div className="flex items-center gap-2">
              <span className="w-8 shrink-0 text-label text-faint">{t("filters.date.from")}</span>
              <div className="flex-1">
                <DatePicker
                  value={filter.dateFrom}
                  onChange={(v) => set({ dateFrom: v })}
                  placeholder={t("filters.date.any")}
                />
              </div>
            </div>
            <div className="flex items-center gap-2">
              <span className="w-8 shrink-0 text-label text-faint">{t("filters.date.to")}</span>
              <div className="flex-1">
                <DatePicker
                  value={filter.dateTo}
                  onChange={(v) => set({ dateTo: v })}
                  placeholder={t("filters.date.any")}
                />
              </div>
            </div>
            {facets?.dateMin && (
              <div className="px-1 text-label text-faint">
                {/* Each date kept whole: French broke "2025-12-27" at a hyphen. */}
                {tx("filters.date.spans", {
                  from: <span className="whitespace-nowrap">{facets.dateMin.slice(0, 10)}</span>,
                  to: <span className="whitespace-nowrap">{facets.dateMax?.slice(0, 10) ?? ""}</span>,
                })}
              </div>
            )}
          </div>
        </Section>

        <Section id="Genre" label={t("filters.section.genre")} badge={filter.genres.length + (filter.excludeGenres?.length ?? 0)}>
          <FacetList
            id="genres"
            items={withChosen(asItems(facets?.genres ?? []), filter.excludeGenres ?? []).map((i) => ({
              ...i,
              label: tagName(i.label),
            }))}
            active={(v) => filter.genres.includes(v)}
            onToggle={(v) => set({ genres: toggle(filter.genres, v) })}
            excluded={(v) => (filter.excludeGenres ?? []).includes(v)}
            onClickAlt={(v) => {
              const next = facetClick(filter.genres, filter.excludeGenres ?? [], v, true);
              set({ genres: next.chosen, excludeGenres: next.excluded });
            }}
          />
        </Section>

        <Section id="Place" label={t("filters.section.place")} badge={filter.placeIds.length}>
          <FacetList
            id="places"
            items={(facets?.places ?? []).map((p) => ({
              key: String(p.id),
              label: localPlaceName(p.name),
              count: p.count,
            }))}
            active={(k) => filter.placeIds.includes(Number(k))}
            onToggle={(k) => set({ placeIds: toggle(filter.placeIds, Number(k)) })}
            capitalise={false}
            empty={t("filters.place.empty")}
          />
        </Section>

        {/* Named people. Several at once means "all of them in the
            photo" — who is together — not "any of them", unlike every other
            list here. Counted only while open; see EXTRA_SECTIONS. */}
        <Section id={EXTRA_SECTIONS.people} label={t("filters.section.people")} badge={filter.personIds?.length ?? 0} defaultOpen={false}>
          <FacetList
            id="people"
            items={(extra?.people ?? []).map((p) => ({ key: String(p.id), label: p.name, count: p.count }))}
            active={(k) => (filter.personIds ?? []).includes(Number(k))}
            onToggle={(k) => set({ personId: null, personIds: toggle(filter.personIds ?? [], Number(k)) })}
            capitalise={false}
            empty={extra ? t("filters.people.empty") : t("filters.facet.counting")}
          />
          {(filter.personIds?.length ?? 0) > 1 && (
            <div className="px-2 pt-1 text-label text-faint">{t("filters.people.allOfThem")}</div>
          )}
        </Section>

        {/* Subjects as a tree the user files them in: choosing one
            takes in everything beneath it. */}
        <Section id="Keywords" label={t("filters.section.keywords")} badge={filter.subjects.length + (filter.excludeSubjects?.length ?? 0)}>
          <KeywordTree />
        </Section>

        <Section
          id="Conditions"
          label={t("filters.section.conditions")}
          badge={filter.conditions.length + (filter.excludeConditions?.length ?? 0)}
          defaultOpen={false}
        >
          <FacetList
            id="conditions"
            items={withChosen(asItems(facets?.conditions ?? []), filter.excludeConditions ?? []).map((i) => ({
              ...i,
              label: conditionName(i.label),
            }))}
            active={(v) => filter.conditions.includes(v)}
            onToggle={(v) => set({ conditions: toggle(filter.conditions, v) })}
            excluded={(v) => (filter.excludeConditions ?? []).includes(v)}
            onClickAlt={(v) => {
              const next = facetClick(filter.conditions, filter.excludeConditions ?? [], v, true);
              set({ conditions: next.chosen, excludeConditions: next.excluded });
            }}
          />
        </Section>

        {/* Exact ratings, several at once, like every other facet. It was a
            row of stars meaning "at least", so three stars also brought back
            every four and five, which is not what clicking three says. */}
        <Section id="Rating" label={t("filters.section.rating")} badge={ratings.length}>
          {RATING_OPTIONS.map((n) => {
            const active = ratings.includes(n);
            const count = countOf(facets?.ratings, n);
            return (
              <button
                key={n}
                onClick={() => set({ ratings: toggle(ratings, n), minRating: null })}
                className={`facet-row ${active ? "facet-row-active" : ""}`}
                aria-pressed={active}
                aria-label={
                  n === 0
                    ? t("filters.rating.unratedAria", { photos: t("common.photos", { count }) })
                    : t("filters.rating.starsAria", { count: n, photos: t("common.photos", { count }) })
                }
              >
                <CheckBox active={active} />
                {n === 0 ? (
                  <span>{t("filters.rating.unrated")}</span>
                ) : (
                  <span className="tracking-tight" aria-hidden>
                    <span className="text-accent">{"★".repeat(n)}</span>
                    <span className="text-dim">{"★".repeat(5 - n)}</span>
                  </span>
                )}
                <span className="ml-auto shrink-0 text-label tabular-nums text-faint">{formatNumber(count)}</span>
              </button>
            );
          })}
        </Section>

        <Section id="Flags" label={t("filters.section.flags")} badge={filter.flags.length}>
          {FLAG_OPTIONS.map((o) => {
            const active = filter.flags.includes(o.value);
            const count = countOf(facets?.flags, o.value);
            return (
              <button
                key={o.value}
                onClick={() => set({ flags: toggle(filter.flags, o.value) })}
                className={`facet-row ${active ? "facet-row-active" : ""}`}
              >
                <CheckBox active={active} />
                <span className="inline-flex items-center gap-1.5">
                  <Icon name={o.icon} size={13} className="text-faint" />
                  {t(o.label)}
                </span>
                <span className="ml-auto shrink-0 text-label tabular-nums text-faint">
                  {formatNumber(count)}
                </span>
              </button>
            );
          })}
        </Section>

        <Section id="Colour" label={t("filters.section.colour")} badge={filter.colorLabels.length}>
          <div className="flex flex-wrap items-center gap-1.5 px-2 py-1.5">
            {COLOR_LABELS.map((c) => {
              const active = filter.colorLabels.includes(c.value);
              const count = countOf(facets?.colorLabels, c.value);
              return (
                <button
                  key={c.value}
                  onClick={() => set({ colorLabels: toggle(filter.colorLabels, c.value) })}
                  className={`h-5 w-5 rounded-full border transition-transform hover:scale-110 ${
                    active ? "border-ink ring-2 ring-accent" : "border-black/30"
                  }`}
                  style={{ backgroundColor: c.hex }}
                  // A dot has no room for a number, so the count is in the
                  // tooltip and the accessible name — still the facet count,
                  // not the selection count.
                  title={t("filters.colour.title", { name: t(c.nameKey), count })}
                  aria-label={t("filters.colour.aria", { name: t(c.nameKey), photos: t("common.photos", { count }) })}
                />
              );
            })}
          </div>
        </Section>

        {/* Camera and exposure: what EXIF already records about every
            photo. Counted only while open; see EXTRA_SECTIONS. */}
        <Section id={EXTRA_SECTIONS.gear} label={t("filters.section.camera")} badge={gearCount(filter)} defaultOpen={false}>
          {!extra ? (
            <div className="px-2 py-1 text-caption text-faint">{t("filters.facet.counting")}</div>
          ) : (
            <>
              <SubLabel>{t("filters.gear.body")}</SubLabel>
              <FacetList
                id="cameras"
                items={asItems(extra.cameras)}
                active={(v) => (filter.cameras ?? []).includes(v)}
                onToggle={(v) => set({ cameras: toggle(filter.cameras ?? [], v) })}
                capitalise={false}
                empty={t("filters.gear.noCamera")}
              />
              <SubLabel>{t("filters.gear.lens")}</SubLabel>
              <FacetList
                id="lenses"
                items={asItems(extra.lenses)}
                active={(v) => (filter.lenses ?? []).includes(v)}
                onToggle={(v) => set({ lenses: toggle(filter.lenses ?? [], v) })}
                capitalise={false}
                empty={t("filters.gear.noLens")}
              />
              <SubLabel>{t("filters.gear.focalLength")}</SubLabel>
              <FacetList
                id="focal"
                items={rangeItems(FOCAL_RANGES, extra.focalRanges, filter.focalRanges ?? [])}
                active={(k) => (filter.focalRanges ?? []).includes(k)}
                onToggle={(k) => set({ focalRanges: toggle(filter.focalRanges ?? [], k) })}
                capitalise={false}
                empty={t("filters.gear.noneRecorded")}
              />
              <SubLabel>{t("filters.gear.aperture")}</SubLabel>
              <FacetList
                id="aperture"
                items={rangeItems(APERTURE_RANGES, extra.apertureRanges, filter.apertureRanges ?? [])}
                active={(k) => (filter.apertureRanges ?? []).includes(k)}
                onToggle={(k) => set({ apertureRanges: toggle(filter.apertureRanges ?? [], k) })}
                capitalise={false}
                empty={t("filters.gear.noneRecorded")}
              />
              <SubLabel>{t("filters.gear.iso")}</SubLabel>
              <FacetList
                id="iso"
                items={rangeItems(ISO_RANGES, extra.isoRanges, filter.isoRanges ?? [])}
                active={(k) => (filter.isoRanges ?? []).includes(k)}
                onToggle={(k) => set({ isoRanges: toggle(filter.isoRanges ?? [], k) })}
                capitalise={false}
                empty={t("filters.gear.noneRecorded")}
              />
            </>
          )}
        </Section>

        {/* What kind of file, what shape, how many people: solo
            portraits and group shots, panoramas, the phone's HEICs. Counted
            only while open; see EXTRA_SECTIONS. */}
        <Section id={EXTRA_SECTIONS.kind} label={t("filters.section.kind")} badge={kindCount(filter)} defaultOpen={false}>
          {!extra ? (
            <div className="px-2 py-1 text-caption text-faint">{t("filters.facet.counting")}</div>
          ) : (
            <>
              <SubLabel>{t("filters.kind.fileType")}</SubLabel>
              <FacetList
                id="kinds"
                items={rangeItems(FILE_KINDS, extra.fileKinds, filter.fileKinds ?? [])}
                active={(k) => (filter.fileKinds ?? []).includes(k)}
                onToggle={(k) => set({ fileKinds: toggle(filter.fileKinds ?? [], k) })}
                capitalise={false}
                empty={t("filters.kind.noneYet")}
              />
              <SubLabel>{t("filters.kind.shape")}</SubLabel>
              <FacetList
                id="shapes"
                items={rangeItems(SHAPES, extra.shapes, filter.shapes ?? [])}
                active={(k) => (filter.shapes ?? []).includes(k)}
                onToggle={(k) => set({ shapes: toggle(filter.shapes ?? [], k) })}
                capitalise={false}
                empty={t("filters.kind.noneYet")}
              />
              <SubLabel>{t("filters.kind.faces")}</SubLabel>
              <FacetList
                id="facecounts"
                items={rangeItems(FACE_COUNTS, extra.faceCounts, filter.faceCounts ?? [])}
                active={(k) => (filter.faceCounts ?? []).includes(k)}
                onToggle={(k) => set({ faceCounts: toggle(filter.faceCounts ?? [], k) })}
                capitalise={false}
                empty={t("filters.kind.noFacesYet")}
              />
            </>
          )}
        </Section>

        {/* The colours that cover the photos: any of those chosen.
            Counted only while open, like File & shape. */}
        <Section id={EXTRA_SECTIONS.palette} label={t("filters.section.palette")} badge={filter.palette?.length ?? 0} defaultOpen={false}>
          {!extra ? (
            <div className="px-2 py-1 text-caption text-faint">{t("filters.facet.counting")}</div>
          ) : extra.palette.length === 0 && (filter.palette?.length ?? 0) === 0 ? (
            <div className="px-2 py-1 text-caption text-faint">{t("filters.palette.empty")}</div>
          ) : (
            <div className="grid grid-cols-2 gap-x-1 px-1 pb-1">
              {PALETTE.map((c) => {
                const n = extra.palette.find((x) => x.value === c.key)?.count ?? 0;
                const on = (filter.palette ?? []).includes(c.key);
                if (n === 0 && !on) return null;
                return (
                  <button
                    key={c.key}
                    onClick={() => set({ palette: toggle(filter.palette ?? [], c.key) })}
                    className={`facet-row ${on ? "facet-row-active" : ""}`}
                    aria-pressed={on}
                    title={t("filters.palette.title", { name: t(c.labelKey).toLocaleLowerCase(intlLocale()) })}
                  >
                    <span
                      className="h-3 w-3 shrink-0 rounded-full ring-1 ring-line"
                      style={{ background: c.css }}
                    />
                    <span className="truncate">{t(c.labelKey)}</span>
                    <span className="ml-auto shrink-0 text-label tabular-nums text-faint">{formatNumber(n)}</span>
                  </button>
                );
              })}
            </div>
          )}
        </Section>

        {/* Card imports, each with how far it has got and the one thing to do
            next: "where was I?" answered by a button. */}
        {shoots.length > 0 && (
          <Section id="Shoots" label={t("filters.section.shoots")} badge={filter.shootId != null ? 1 : 0} defaultOpen={false}>
            {shoots.map((sh) => {
              const active = filter.shootId === sh.id;
              const next = nextStep(sh);
              return (
                <div key={sh.id} className="group/shoot pb-1.5">
                  <div className="relative flex items-center">
                    <button
                      onClick={() => set({ shootId: active ? null : sh.id })}
                      className={`facet-row min-w-0 flex-1 group-hover/shoot:bg-hover group-hover/shoot:text-ink ${active ? "facet-row-active" : ""}`}
                      // "Sun 20 Sep 2026", as the timeline says a day; a bare
                      // `formatDate` gave English "9/20/2026".
                      title={t("filters.shoot.importedOn", {
                        date: periodLabel(formatCapture(new Date(sh.createdAt)).slice(0, 10)),
                      })}
                    >
                      <span className="grid h-3.5 w-3.5 shrink-0 place-items-center">
                        {active && <span className="h-2 w-2 rounded-full bg-accent" />}
                      </span>
                      <span className="truncate">{sh.name}</span>
                      <span className="ml-auto shrink-0 text-label tabular-nums text-faint group-hover/shoot:invisible">
                        {formatNumber(sh.photos)}
                      </span>
                    </button>
                    <button
                      onClick={() =>
                        void confirm({
                          title: t("filters.shoot.forgetTitle", { name: sh.name }),
                          message: t("filters.shoot.forgetMessage"),
                          confirmLabel: t("filters.shoot.forgetConfirm"),
                        }).then((ok) => ok && void forgetShoot(sh.id))
                      }
                      className="absolute inset-y-0 right-0 rounded-r-sm bg-hover px-1.5 text-faint opacity-0 transition-opacity hover:text-danger focus-visible:opacity-100 group-hover/shoot:opacity-100"
                      title={t("filters.shoot.forgetTooltip")}
                    >
                      <Icon name="close" size={12} />
                    </button>
                  </div>
                  <div className="truncate pl-7 text-label text-faint" title={progressLine(sh)}>
                    {progressLine(sh)}
                  </div>
                  {/* What its import could not do, until it is done:
                      the line a photographer checks before formatting. */}
                  {sh.notCopied > 0 && (
                    <div className="flex items-baseline gap-2 pl-7 text-label">
                      <span className="text-danger">{t("library.shoots.notCopied", { count: sh.notCopied })}</span>
                      <button onClick={() => importForShoot(sh.id)} className="text-accent hover:underline">
                        {t("library.shoots.importRest")}
                      </button>
                    </div>
                  )}
                  {sh.notBackedUp > 0 && (
                    <div className="flex items-baseline gap-2 pl-7 text-label">
                      <span className="text-danger">{t("library.shoots.notBackedUp", { count: sh.notBackedUp })}</span>
                      <button
                        onClick={() =>
                          void api
                            .finishShootBackup(sh.id)
                            .then((b) =>
                              toast(
                                b.failed.length === 0
                                  ? t("transfer.import.backupDone")
                                  : t("transfer.import.backupStillFailed", { count: b.failed.length }),
                                b.failed.length === 0 ? "success" : "error",
                              ),
                            )
                            .catch((e) => toast(String(e), "error"))
                            .finally(() => void loadShoots())
                        }
                        className="text-accent hover:underline"
                      >
                        {t("library.shoots.finishBackup")}
                      </button>
                    </div>
                  )}
                  {next.step !== "done" ? (
                    <button
                      onClick={() => void continueShoot(sh)}
                      className="pl-7 text-label text-accent hover:underline"
                    >
                      {next.label}
                    </button>
                  ) : (
                    <div className="pl-7 text-label text-faint">{next.label}</div>
                  )}
                </div>
              );
            })}
          </Section>
        )}

        <Section id="Collections" label={t("filters.section.collections")} badge={filter.collectionId ? 1 : 0} defaultOpen={false}>
          {collections.length === 0 ? (
            <div className="px-2 py-1 text-caption text-faint">
              {t("filters.collection.none")}
            </div>
          ) : (
            <GroupedList
              items={collections}
              smart={false}
              renderItem={(c) => {
              const active = filter.collectionId === c.id;
              return (
                <div key={c.id} className="group/coll relative flex items-center">
                  <button
                    onClick={() => set({ collectionId: active ? null : c.id })}
                    className={`facet-row min-w-0 flex-1 group-hover/coll:bg-hover group-hover/coll:text-ink ${active ? "facet-row-active" : ""}`}
                    title={c.name}
                  >
                    <span className="grid h-3.5 w-3.5 shrink-0 place-items-center">
                      {active && <span className="h-2 w-2 rounded-full bg-accent" />}
                    </span>
                    <span className="truncate">{c.name}</span>
                    <span className="ml-auto shrink-0 text-label tabular-nums text-faint group-hover/coll:invisible">
                      {formatNumber(c.count)}
                    </span>
                  </button>
                  {/* Over the count while hovered: beside it, the three held
                      every collection's count 60px in from the facets'. */}
                  <div className="absolute inset-y-0 right-0 flex items-center rounded-r-sm bg-hover pl-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover/coll:opacity-100">
                  <button
                    onClick={() => void playCollection(c.id)}
                    className="px-1 text-faint hover:text-ink"
                    title={t("filters.collection.playTitle")}
                    aria-label={t("filters.collection.playAria", { name: c.name })}
                  >
                    <Icon name="play" size={12} />
                  </button>
                  <button
                    onClick={() =>
                      void prompt({
                        title: t("filters.collection.renameTitle"),
                        defaultValue: c.name,
                        placeholder: t("filters.collection.namePlaceholder"),
                        confirmLabel: t("common.rename"),
                      }).then((name) => name && name.trim() !== c.name && void renameCollection(c.id, name))
                    }
                    className="px-1 text-faint hover:text-ink"
                    title={t("filters.collection.renameTitle")}
                    aria-label={t("filters.collection.renameAria", { name: c.name })}
                  >
                    <Icon name="pencil" size={12} />
                  </button>
                  <button
                    onClick={() =>
                      void confirm({
                        title: t("filters.collection.deleteTitle", { name: c.name }),
                        message: t("filters.collection.deleteMessage"),
                        confirmLabel: t("common.delete"),
                        danger: true,
                      }).then((ok) => ok && void deleteCollection(c.id))
                    }
                    className="px-1.5 text-faint hover:text-danger"
                    title={t("filters.collection.deleteTooltip")}
                  >
                    <Icon name="close" size={12} />
                  </button>
                  </div>
                </div>
              );
              }}
            />
          )}
          <button
            onClick={() =>
              void prompt({
                title: t("filters.collection.newTitle"),
                placeholder: t("filters.collection.namePlaceholder"),
                confirmLabel: t("filters.collection.create"),
              }).then(
                (name) => name && void createCollection(name),
              )
            }
            className="px-2 py-1 text-caption text-accent hover:underline"
          >
            {t("filters.collection.newButton")}
          </button>
          <button
            onClick={() =>
              void prompt({
                title: t("filters.group.newTitle"),
                placeholder: t("filters.group.namePlaceholder"),
                confirmLabel: t("filters.collection.create"),
              }).then((name) => name && void createCollectionGroup(name))
            }
            className="px-2 py-1 text-caption text-accent hover:underline"
            title={t("filters.group.newHint")}
          >
            {t("filters.group.newButton")}
          </button>
        </Section>

        <Section id="Smart Collections" label={t("filters.section.smartCollections")} defaultOpen={false}>
          {smartCollections.length === 0 ? (
            <div className="px-2 py-1 text-caption leading-relaxed text-faint">
              {t("filters.smart.hint")}
            </div>
          ) : (
            <GroupedList
              items={smartCollections}
              smart
              renderItem={(sc) => (
              <div key={sc.id} className="group/smart flex items-center">
                <button
                  onClick={() => onChange(normaliseFilter(sc.filter))}
                  className="facet-row flex-1"
                  title={t("filters.smart.applyTooltip")}
                >
                  <Icon name="bookmark" size={13} className="text-faint" />
                  <span className="truncate" title={sc.name}>{sc.name}</span>
                </button>
                <button
                  onClick={() =>
                    void confirm({
                      title: t("filters.smart.deleteTitle", { name: sc.name }),
                      confirmLabel: t("common.delete"),
                      danger: true,
                    }).then((ok) => ok && void deleteSmartCollection(sc.id))
                  }
                  className="px-1.5 text-faint opacity-0 transition-opacity hover:text-danger focus-visible:opacity-100 group-hover/smart:opacity-100"
                  title={t("common.delete")}
                >
                  <Icon name="close" size={12} />
                </button>
              </div>
              )}
            />
          )}
          <button
            disabled={isFilterEmpty(filter)}
            onClick={() =>
              void prompt({
                title: t("filters.smart.saveTitle"),
                placeholder: t("filters.smart.namePlaceholder"),
                confirmLabel: t("common.save"),
              }).then((name) => name && void saveCurrentAsSmart(name))
            }
            className="px-2 py-1 text-caption text-accent hover:underline disabled:cursor-not-allowed disabled:text-faint disabled:no-underline"
            title={isFilterEmpty(filter) ? t("filters.smart.setUpFirst") : t("filters.smart.saveTooltip")}
          >
            {t("filters.smart.saveButton")}
          </button>
        </Section>

        {/* By folder — only when there is more than one to choose. */}
        {folders.length > 1 && (
          <Section id="Folders" label={t("filters.section.folders")} badge={filter.folderIds?.length ?? 0}>
            {folders.map((fo) => {
              const ids = filter.folderIds ?? [];
              const active = ids.includes(fo.id);
              return (
                <button
                  key={fo.id}
                  onClick={() => set({ folderIds: toggle(ids, fo.id) })}
                  className={`facet-row ${active ? "facet-row-active" : ""}`}
                  title={fo.path}
                >
                  <CheckBox active={active} />
                  <span className="min-w-0 truncate">{folderName(fo.path)}</span>
                  <span className="ml-auto shrink-0 text-label tabular-nums text-faint">
                    {formatNumber(countOf(facets?.folders, fo.id))}
                  </span>
                </button>
              );
            })}
          </Section>
        )}

        <Section id={EXTRA_SECTIONS.status} label={t("filters.section.status")} badge={statusCount(filter)} defaultOpen={false}>
          <button
            onClick={() => set({ untaggedOnly: !filter.untaggedOnly })}
            className={`facet-row ${filter.untaggedOnly ? "facet-row-active" : ""}`}
          >
            <CheckBox active={filter.untaggedOnly} />
            <span>{t("filters.status.notTagged")}</span>
            <span className="ml-auto text-label tabular-nums text-faint">
              {formatNumber(facets?.untagged ?? 0)}
            </span>
          </button>
          {/* Files the last scan could not find: shown once there are
              any, so a user can see exactly which photos a drive holds. */}
          {((facets?.missing ?? 0) > 0 || filter.missingOnly) && (
            <button
              onClick={() => set({ missingOnly: !filter.missingOnly })}
              className={`facet-row ${filter.missingOnly ? "facet-row-active" : ""}`}
            >
              <CheckBox active={!!filter.missingOnly} />
              <span className="inline-flex items-center gap-1.5">
                <Icon name="warn" size={13} className="text-warning" />
                {t("filters.status.fileMissing")}
              </span>
              <span className="ml-auto text-label tabular-nums text-faint">
                {formatNumber(facets?.missing ?? 0)}
              </span>
            </button>
          )}
          {/* The status toggles: each narrows on its own, and they combine. */}
          {(
            [
              { key: "editedOnly", label: "filters.status.edited", count: extra?.edited },
              { key: "copiesOnly", label: "filters.status.copies", count: extra?.copies },
              { key: "facesOnly", label: "filters.status.faces", count: extra?.withFaces },
              { key: "locatedOnly", label: "filters.status.located", count: extra?.located },
              { key: "unplacedOnly", label: "filters.status.unplaced", count: extra?.unplaced },
            ] as const
          ).map((tog) => {
            const on = !!filter[tog.key];
            return (
              <button
                key={tog.key}
                onClick={() => set({ [tog.key]: !on })}
                className={`facet-row ${on ? "facet-row-active" : ""}`}
              >
                <CheckBox active={on} />
                <span>{t(tog.label)}</span>
                <span className="ml-auto text-label tabular-nums text-faint">
                  {tog.count != null ? formatNumber(tog.count) : "…"}
                </span>
              </button>
            );
          })}
        </Section>
      </div>
      </div>
      )}
    </aside>
  );
}
