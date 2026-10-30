import { useEffect, useMemo, useState } from "react";
import { sentenceCase } from "../lib/format";
import { PAGE, useStore } from "../store";
import { api } from "../lib/api";
import { Icon } from "../lib/icons";
import { prompt } from "../lib/dialogs";
import { folderName } from "./LibraryMenu";
import { formatNumber, getLocale, t, tx, type Key } from "../i18n";
import { countryName, localPlaceName } from "../lib/countries";
import { conditionName } from "../lib/content";
import { tagName } from "../i18n/tags";
import {
  APERTURE_RANGES,
  COLOR_LABELS,
  FACE_COUNTS,
  PALETTE,
  FILE_KINDS,
  FOCAL_RANGES,
  SHAPES,
  ISO_RANGES,
  SORTS,
  isFilterEmpty,
  type PhotoFilter,
} from "../types";
import type { PhotoSort } from "../bindings";
import type { Part } from "../lib/queryReading";

/** What each kind of reading is called, for "“Paris” as a person?". */
const KIND_WORDS: Record<Part["kind"], Key> = {
  person: "library.header.kind.person",
  place: "library.header.kind.place",
  date: "library.header.kind.date",
  genre: "library.header.kind.genre",
  condition: "library.header.kind.condition",
  camera: "library.header.kind.camera",
  lens: "library.header.kind.lens",
  exclude: "library.header.kind.exclude",
};

/** The on/off filters that get a chip, by the words the chip carries. */
const TOGGLE_CHIPS: [keyof PhotoFilter, Key][] = [
  ["editedOnly", "library.header.chip.edited"],
  ["copiesOnly", "library.header.chip.virtualCopies"],
  ["facesOnly", "library.header.chip.withFaces"],
  ["locatedOnly", "library.header.chip.hasLocation"],
  ["unplacedOnly", "library.header.chip.noPlace"],
  ["missingOnly", "library.header.chip.fileMissing"],
];

/**
 * The one header that owns "what am I looking at right now": the result count,
 * the active-filter chips, and Save-as-view. Previously this state was spread
 * across the top bar, the rail and an inline chip strip in App; here it lives in
 * a single line above the grid.
 */
export default function ContextHeader() {
  const filter = useStore((s) => s.filter);
  const facets = useStore((s) => s.facets);
  const collections = useStore((s) => s.collections);
  const photoCount = useStore((s) => s.photos.length);
  const searchResults = useStore((s) => s.searchResults);
  const loading = useStore((s) => s.loading);
  const setFilter = useStore((s) => s.setFilter);
  const clearFilter = useStore((s) => s.clearFilter);
  const saveCurrentAsSmart = useStore((s) => s.saveCurrentAsSmart);
  const queryReading = useStore((s) => s.queryReading);
  const shoots = useStore((s) => s.shoots);
  const searchAsTyped = useStore((s) => s.searchAsTyped);
  const takeAlternative = useStore((s) => s.takeAlternative);

  const libraryFolders = useStore((s) => s.libraryFolders);

  // The people chips resolve their own names: the rail's People counts carry
  // only named, visible people, and are only counted while that section is
  // open — but People can jump into Browse with anyone. Re-read whenever the
  // chosen people change — Browse unmounts when you visit People, so a rename
  // over there is picked up on the way back.
  const personIds = useMemo(
    () => [...new Set([...(filter.personId != null ? [filter.personId] : []), ...(filter.personIds ?? [])])],
    [filter.personId, filter.personIds],
  );
  const personKey = personIds.join(",");
  const [personNames, setPersonNames] = useState<Map<number, string | null>>(new Map());
  useEffect(() => {
    if (!personKey) {
      setPersonNames(new Map());
      return;
    }
    let live = true;
    void api
      .listPeople()
      .then((people) => {
        if (live) setPersonNames(new Map(people.map((p) => [p.id, p.name])));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [personKey]);

  // The chips hold words, so a change of language has to rebuild them.
  const locale = getLocale();
  const chips = useMemo(() => {
    // `asIs`: shown as spelt. Tags are stored lower-case and read better
    // capitalised; a camera, a lens, "24 mm" or a person's name does not.
    const out: { label: string; clear: () => void; asIs?: boolean }[] = [];
    // What a search was read as names its own chips: "Summer 2025"
    // rather than two dates, and a country as one chip, not one per place.
    const read = queryReading?.applied;
    const readDate =
      read?.date && read.date.from === filter.dateFrom && read.date.to === filter.dateTo ? read.date.label : null;
    if (filter.dateFrom || filter.dateTo)
      out.push({
        label: readDate ?? t("library.header.chip.dateRange", { from: filter.dateFrom ?? "…", to: filter.dateTo ?? "…" }),
        asIs: readDate !== null,
        clear: () => setFilter({ ...filter, dateFrom: null, dateTo: null }),
      });
    filter.genres.forEach((g) =>
      out.push({ label: tagName(g), clear: () => setFilter({ ...filter, genres: filter.genres.filter((x) => x !== g) }) }),
    );
    filter.subjects.forEach((s) =>
      out.push({ label: tagName(s), clear: () => setFilter({ ...filter, subjects: filter.subjects.filter((x) => x !== s) }) }),
    );
    filter.conditions.forEach((c) =>
      out.push({
        label: conditionName(c),
        clear: () => setFilter({ ...filter, conditions: filter.conditions.filter((x) => x !== c) }),
      }),
    );
    // Left out: "not clouds".
    const notChips: [keyof PhotoFilter & ("excludeGenres" | "excludeSubjects" | "excludeConditions"), string[]][] = [
      ["excludeGenres", filter.excludeGenres ?? []],
      ["excludeSubjects", filter.excludeSubjects ?? []],
      ["excludeConditions", filter.excludeConditions ?? []],
    ];
    for (const [key, list] of notChips)
      list.forEach((v) =>
        out.push({
          // In the language shown: it read "Not clouds" beside "Wolken".
          label: t("library.header.chip.not", { name: key === "excludeConditions" ? conditionName(v) : tagName(v) }),
          clear: () => setFilter({ ...filter, [key]: list.filter((x) => x !== v) }),
        }),
      );
    const grouped = new Set<number>();
    for (const g of read?.groups ?? []) {
      if (!g.placeIds.every((id) => filter.placeIds.includes(id))) continue;
      g.placeIds.forEach((id) => grouped.add(id));
      out.push({
        label: countryName(g.label),
        asIs: true,
        clear: () => setFilter({ ...filter, placeIds: filter.placeIds.filter((x) => !g.placeIds.includes(x)) }),
      });
    }
    filter.placeIds.forEach((id) => {
      if (grouped.has(id)) return;
      const place = facets?.places.find((p) => p.id === id);
      // A place a search named may have no photos under the rest of the
      // filter, so no facet row to take its name from.
      const named = queryReading?.reading.parts.find((p) => p.kind === "place" && p.ids.includes(id));
      out.push({
        label: (place ? localPlaceName(place.name) : undefined) ?? named?.label ?? t("library.header.chip.placeId", { id: String(id) }),
        asIs: true,
        clear: () => setFilter({ ...filter, placeIds: filter.placeIds.filter((x) => x !== id) }),
      });
    });
    if (filter.minRating)
      out.push({ label: t("library.header.chip.ratingAtLeast", { rating: String(filter.minRating) }), clear: () => setFilter({ ...filter, minRating: null }) });
    // One chip for the rating set, not one per star: "★ 3, 5" reads as a
    // single choice, which is how it was made.
    const ratings = filter.ratings ?? [];
    if (ratings.length > 0) {
      const stars = [...ratings].sort((a, b) => b - a).map((r) => (r === 0 ? t("library.header.chip.unrated") : String(r)));
      out.push({ label: t("library.header.chip.ratings", { ratings: stars.join(", ") }), clear: () => setFilter({ ...filter, ratings: [] }) });
    }
    if (filter.untaggedOnly)
      out.push({ label: t("library.header.chip.untagged"), clear: () => setFilter({ ...filter, untaggedOnly: false }) });
    filter.flags.forEach((fl) =>
      out.push({
        label: t(fl === 1 ? "library.header.chip.picks" : fl === -1 ? "library.header.chip.rejects" : "library.header.chip.unflagged"),
        clear: () => setFilter({ ...filter, flags: filter.flags.filter((x) => x !== fl) }),
      }),
    );
    filter.colorLabels.forEach((v) => {
      const c = COLOR_LABELS.find((x) => x.value === v);
      out.push({
        label: c ? t(c.nameKey) : t("library.header.chip.labelId", { id: String(v) }),
        clear: () => setFilter({ ...filter, colorLabels: filter.colorLabels.filter((x) => x !== v) }),
      });
    });
    if (filter.collectionId) {
      const coll = collections.find((c) => c.id === filter.collectionId);
      out.push({ label: coll?.name ?? t("library.header.chip.collection"), clear: () => setFilter({ ...filter, collectionId: null }) });
    }
    // People jumps into Browse with a person set. Without a chip it is a
    // filter with no on-screen cause and no way out.
    personIds.forEach((id) =>
      out.push({
        label: personNames.get(id) ?? t("library.header.chip.unnamedPerson"),
        asIs: true,
        clear: () =>
          setFilter({
            ...filter,
            personId: filter.personId === id ? null : filter.personId,
            personIds: (filter.personIds ?? []).filter((x) => x !== id),
          }),
      }),
    );
    // Camera and exposure.
    const list = (key: "cameras" | "lenses", values: string[] | undefined) =>
      (values ?? []).forEach((v) =>
        out.push({
          label: v,
          asIs: true,
          clear: () => setFilter({ ...filter, [key]: (filter[key] ?? []).filter((x) => x !== v) }),
        }),
      );
    list("cameras", filter.cameras);
    list("lenses", filter.lenses);
    const ranges = (
      key: "focalRanges" | "isoRanges" | "apertureRanges" | "fileKinds" | "shapes" | "faceCounts",
      // Worded by key (exposure) or already worded (main's newer tables).
      table: { key: string; labelKey?: Key; label?: string }[],
    ) =>
      (filter[key] ?? []).forEach((k) =>
        out.push({
          label: ((r) => (r?.labelKey ? t(r.labelKey) : (r?.label ?? k)))(table.find((r) => r.key === k)),
          asIs: true,
          clear: () => setFilter({ ...filter, [key]: (filter[key] ?? []).filter((x) => x !== k) }),
        }),
      );
    ranges("focalRanges", FOCAL_RANGES);
    ranges("apertureRanges", APERTURE_RANGES);
    ranges("isoRanges", ISO_RANGES);
    ranges("fileKinds", FILE_KINDS);
    ranges("shapes", SHAPES);
    ranges("faceCounts", FACE_COUNTS);
    (filter.palette ?? []).forEach((k) =>
      out.push({
        label: ((c) => (c ? t(c.labelKey) : k))(PALETTE.find((c) => c.key === k)),
        asIs: true,
        clear: () => setFilter({ ...filter, palette: (filter.palette ?? []).filter((x) => x !== k) }),
      }),
    );
    for (const [key, word] of TOGGLE_CHIPS) {
      if (filter[key]) out.push({ label: t(word), clear: () => setFilter({ ...filter, [key]: false }) });
    }
    if (filter.shootId != null) {
      const shoot = shoots.find((s) => s.id === filter.shootId);
      out.push({
        label: shoot ? shoot.name : t("library.header.chip.shoot"),
        asIs: true,
        clear: () => setFilter({ ...filter, shootId: null }),
      });
    }
    // A folder choice had no chip, so it narrowed the grid invisibly.
    (filter.folderIds ?? []).forEach((id) => {
      const fo = libraryFolders?.folders.find((x) => x.id === id);
      out.push({
        label: fo ? folderName(fo.path) : t("library.header.chip.folder"),
        asIs: true,
        clear: () => setFilter({ ...filter, folderIds: (filter.folderIds ?? []).filter((x) => x !== id) }),
      });
    });
    // `filter.semantic` deliberately gets no chip: the top bar replaces the
    // search box with a "Content: …" banner that already shows and clears it.
    // It still counts in `isFilterEmpty`, so Clear-all and Save-as-view apply.
    return out;
  }, [filter, facets, collections, setFilter, personIds, personNames, libraryFolders, queryReading, shoots, locale]);

  const empty = isFilterEmpty(filter);
  const total = facets?.total ?? 0;
  const shown = searchResults ? searchResults.length : photoCount;

  const saveView = async () => {
    const name = await prompt({
      title: t("library.header.saveView.title"),
      message: t("library.header.saveView.message"),
      placeholder: t("library.header.saveView.placeholder"),
      confirmLabel: t("common.save"),
    });
    if (name) void saveCurrentAsSmart(name);
  };

  const askHighlights = async () => {
    const answer = await prompt({
      title: t("library.header.highlights.title", { count: shown }),
      message: t("library.header.highlights.message"),
      defaultValue: String(Math.min(24, Math.max(6, Math.round(shown / 10)))),
      confirmLabel: t("library.header.highlights.choose"),
    });
    const n = Number(answer);
    if (!answer || !Number.isFinite(n) || n < 1) return;
    const label = chips.map((c) => c.label).slice(0, 2).join(", ") || t("library.header.highlights.wholeLibrary");
    await useStore.getState().makeHighlights(Math.min(Math.round(n), 500), label);
  };

  return (
    <div className="flex min-h-[42px] flex-wrap items-center gap-2 border-b border-line bg-panel/60 px-4 py-1.5">
      {/* The number is a readout (mono); the word is not. */}
      <span className="text-body font-semibold text-ink">
        {loading
          ? t("common.loading")
          : tx("common.photos", { count: <span className="tabular-nums">{formatNumber(shown)}</span> }, shown)}
      </span>
      {!empty && !searchResults && total > 0 && (
        <span className="text-caption text-faint">
          {tx("library.header.ofTotal", { total: <span className="tabular-nums">{formatNumber(total)}</span> })}
        </span>
      )}

      {/* The filtered set is held in memory and capped at PAGE; say so when a
          larger library is being truncated, so the count isn't silently short. */}
      {!searchResults && photoCount >= PAGE && total > photoCount && (
        <span
          className="text-caption text-warning"
          title={t("library.header.truncatedTooltip", { count: PAGE })}
        >
          {t("library.header.truncated", { count: PAGE })}
        </span>
      )}

      {chips.map((c, i) => (
        <span key={i} className="chip">
          {c.asIs ? c.label : sentenceCase(c.label)}
          <button onClick={c.clear} className="chip-x" aria-label={t("library.header.removeChip", { label: c.label })}>
            <Icon name="close" size={11} strokeWidth={2.4} />
          </button>
        </span>
      ))}

      {/* The other reading of a word that reads two ways, and the whole query
          searched as typed — each one click from a guess that was wrong. */}
      {queryReading?.reading.alternatives.map((a, i) => (
        <button
          key={`${a.chosen.text}-${i}`}
          onClick={() => takeAlternative(i)}
          className="rounded-sm px-2 py-1 text-caption text-muted transition-colors hover:bg-hover hover:text-ink"
          title={t("library.header.altTooltip", {
            text: a.chosen.text,
            other: t(KIND_WORDS[a.other.kind]),
            chosen: t(KIND_WORDS[a.chosen.kind]),
          })}
        >
          {t("library.header.altButton", { text: a.chosen.text, other: t(KIND_WORDS[a.other.kind]) })}
        </button>
      ))}
      {queryReading && (
        <button
          onClick={() => void searchAsTyped()}
          className="rounded-sm px-2 py-1 text-caption text-muted transition-colors hover:bg-hover hover:text-ink"
          title={t("library.header.asTypedTooltip", { query: queryReading.raw })}
        >
          {t("library.header.asTyped")}
        </button>
      )}

      {!empty && (
        <button
          onClick={clearFilter}
          className="rounded-sm px-2 py-1 text-caption text-accent transition-colors hover:bg-hover hover:underline"
        >
          {t("library.header.clearAll")}
        </button>
      )}

      <div className="flex-1" />

      {/* The best of what's in view: a trip, a person, a shoot, a month. */}
      {!loading && shown >= 12 && (
        <button
          onClick={() => void askHighlights()}
          className="flex items-center gap-1.5 rounded-sm px-2 py-1 text-caption text-muted transition-colors hover:bg-hover hover:text-ink"
          title={t("library.header.highlights.tooltip")}
        >
          <Icon name="star" size={13} />
          {t("library.header.highlights.button")}
        </button>
      )}

      {/* The order. A content search is ranked by how well each photo
          matches, which no other order should overrule. */}
      {filter.semantic ? (
        <span className="text-caption text-faint">{t("library.header.byRelevance")}</span>
      ) : (
        <label className="flex items-center gap-1.5 text-caption text-faint">
          {t("library.header.sort")}
          <select
            className="rounded-sm border border-line bg-base px-1.5 py-1 text-caption text-ink outline-none"
            value={filter.sort === "custom" && filter.collectionId == null ? "newest" : (filter.sort ?? "newest")}
            onChange={(e) => {
              e.currentTarget.blur();
              setFilter({ ...filter, sort: e.target.value as PhotoSort });
            }}
            aria-label={t("library.header.sortLabel")}
          >
            {SORTS.filter((o) => o.value !== "custom" || filter.collectionId != null).map((o) => (
              <option key={o.value} value={o.value}>
                {t(o.labelKey)}
              </option>
            ))}
          </select>
        </label>
      )}

      {!empty && (
        <button
          onClick={() => void saveView()}
          className="btn-ghost text-caption text-muted"
          title={t("library.header.saveView.tooltip")}
        >
          <Icon name="bookmark" size={13} />
          {t("library.header.saveView.button")}
        </button>
      )}
    </div>
  );
}
