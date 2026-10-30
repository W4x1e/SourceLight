import type { StoreApi } from "zustand";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import type { ExtraFacets, FacetGroups, LibraryFolder, LibraryFolders } from "../bindings";
import { confirm, toast } from "../lib/dialogs";
import { t } from "../i18n";
import { learnTagNames } from "../lib/tagNames";
import { translateCoreError } from "../lib/coreText";
import { EMPTY_FILTER, type Facets, type Photo, type PhotoFilter, type Progress, isFilterEmpty, sortIsByDate } from "../types";
import type { Slice, State } from "./types";

/**
 * The library: its folders and their scans, the filter, and the photos and
 * counts it shows.
 */

// The library is small enough (thousands, not millions) to hold the filtered
// set in memory. That buys instant scrolling and an honest scrollbar, which
// paging would not. Exported so the header can tell the user when a larger
// library is being truncated to this cap.
export const PAGE = 20000;

/** Typing in the search box must not fire a query per keystroke. */
const FILTER_DEBOUNCE_MS = 180;

/** Tail of the scan chain — see `runScan`. Scans run one at a time. */
let scanQueue: Promise<void> = Promise.resolve();

/** How many photos the previous scan found missing, so a launch scan only
 *  speaks up — and only reloads the grid — when that number changes. */
let lastMissing = 0;

/** The reason last shown for photos that couldn't be read, so every launch
 *  scan does not repeat the same failure. */
let lastFailure: string | null = null;

/**
 * One scan pass over `targets`, the library folders to walk. Split out of
 * `runScan` so the queueing there stays readable; this is the part that talks
 * to the backend. Folders are scanned one after another and reported once, so
 * a library of five folders does not raise five toasts.
 */
async function scanOnce(
  set: (partial: Partial<State>) => void,
  get: () => State,
  targets: string[],
): Promise<void> {
  let added = 0;
  let relinked = 0;
  let missing = 0;
  let failed = 0;
  let firstFailure: string | null = null;
  // A first scan shows photos as they are read: the grid sat empty
  // saying the scan "found nothing" until every photo was in, while the tour
  // played over it. Only from empty, so a launch rescan doesn't churn.
  const fromEmpty = get().photos.length === 0;
  const trickle = fromEmpty ? setInterval(() => void get().reload(), 3000) : null;
  for (const target of targets) {
    set({ scanning: { phase: "scan", current: 0, total: 0, message: t("store.scan.starting") } });
    try {
      const result = await api.scanLibrary(target);
      added += result.added;
      relinked += result.relinked;
      missing += result.missing;
      failed += result.failed;
      firstFailure ??= result.firstFailure;
      console.info("scan complete", target, result);
    } catch (e) {
      console.error("scan failed", target, e);
      set({ error: String(e) });
    }
  }
  if (trickle) clearInterval(trickle);
  set({ scanning: null });
  const missingChanged = missing !== lastMissing;
  lastMissing = missing;
  void get().refreshFolders();
  // Only re-query when the index actually changed. This runs on every launch,
  // and a scan that finds nothing should not flash the grid through
  // "Loading…" for no reason.
  if (added > 0 || relinked > 0 || missingChanged) {
    await get().reload();
  }
  if (added > 0) {
    toast(t("store.scan.added", { count: added }), "success");
    // New photos are unsearchable, untagged and faceless until something
    // analyses them — so something does.
    void get().analyseNewPhotos();
  }
  // A folder renamed or moved outside the app: say that nothing was
  // lost, because the user will have half-expected it to be.
  if (relinked > 0) {
    toast(t("store.scan.relinked", { count: relinked }), "success");
  }
  if (missingChanged && missing > 0) {
    toast(t("store.scan.missing", { count: missing }), "info");
  }
  // Photos that couldn't be read: say why, once. Failures usually share a
  // cause (HEIC without Windows' codec), and the reason says how to fix it.
  if (failed > 0 && firstFailure && firstFailure !== lastFailure) {
    toast(t("store.scan.failed", { count: failed, reason: translateCoreError(firstFailure) }), "error");
  }
  lastFailure = firstFailure;
}

/**
 * The filter-rail sections whose counts are optional, by the group of
 * `get_extra_facets` they need. Each costs a pass over the photos the base
 * counts don't, so it is only counted while its section is open — and they all
 * start closed. The labels must match the rail's `Section` labels.
 */
export const EXTRA_SECTIONS: Record<keyof FacetGroups, string> = {
  gear: "Camera",
  people: "People",
  status: "Status",
  kind: "File & shape",
  palette: "Palette",
};

/** Which optional counts the rail needs, from its sections' open state. */
export function extraGroups(sections: Record<string, boolean>): FacetGroups {
  return {
    gear: sections[EXTRA_SECTIONS.gear] === true,
    people: sections[EXTRA_SECTIONS.people] === true,
    status: sections[EXTRA_SECTIONS.status] === true,
    kind: sections[EXTRA_SECTIONS.kind] === true,
    palette: sections[EXTRA_SECTIONS.palette] === true,
  };
}

/** Orders `refreshExtraFacets` responses: only the latest request's lands. */
let extraSeq = 0;

/** Orders `reload` responses the same way. The photo and facet queries run off
 *  the main thread, side by side, so a slow one (a text search over a large
 *  library) can finish after the quick one asked for after it. */
let reloadSeq = 0;
/** This page's number for its grid loads, new each time it loads, so
 *  the catalogue can tell a reloaded page's first load from an old one. */
const VIEW_PAGE = Math.floor(Math.random() * 2 ** 32);

let filterTimer: ReturnType<typeof setTimeout> | undefined;

/** `viewFilter` against a given state, for the slices: they make up the
 *  store, so they cannot import it. */
export function viewFilterFor(s: Pick<State, "showAiKeywords">, f: PhotoFilter): PhotoFilter {
  return { ...f, ownKeywords: !s.showAiKeywords };
}

/** After a filter change: keeps the selected photos still shown, and says how
 *  many it let go. */
function keepShownSelection(store: StoreApi<State>, before: Set<number>) {
  if (before.size === 0) return;
  const st = store.getState();
  const shown = new Set((st.searchResults ?? st.photos).map((p) => p.id));
  const kept = [...before].filter((id) => shown.has(id));
  const dropped = before.size - kept.length;
  if (dropped === 0) return;
  store.setState({
    selected: new Set(kept),
    focusId: st.focusId !== null && shown.has(st.focusId) ? st.focusId : (kept[0] ?? null),
    anchorId: st.anchorId !== null && shown.has(st.anchorId) ? st.anchorId : null,
  });
  toast(t("store.selection.narrowed", { count: dropped }));
}

export interface LibrarySlice {
  /** The library's folders; null until first loaded. */
  libraryFolders: LibraryFolders | null;
  photos: Photo[];
  facets: Facets | null;
  /** The rail's optional counts — camera and lens, people, status —
   *  for whichever of their sections are open; `null` until first counted. */
  extraFacets: ExtraFacets | null;
  /** Re-counts `extraFacets` for the open sections, under the committed filter. */
  refreshExtraFacets: () => Promise<void>;
  loading: boolean;

  /** What the controls show — updates on every keystroke. */
  filter: PhotoFilter;
  /** What queries actually run against — debounced behind `filter`. */
  committedFilter: PhotoFilter;

  scanning: Progress | null;

  setFilter: (f: PhotoFilter) => void;
  clearFilter: () => void;

  setScanning: (p: Progress | null) => void;
  /** A `scan-progress` event: taken only while a scan is in flight (see App). */
  scanProgress: (p: Progress) => void;

  init: () => Promise<void>;
  reload: () => Promise<void>;
  /**
   * After a change to these photos alone: their rows fetched again and patched
   * into the page, and the facets counted again, instead of the whole
   * 20,000-row page. Falls back to `reload` where a change could move a photo
   * into or out of the view or along it: under any filter or a sort other than
   * by date, or for a photo not on the page.
   */
  refreshPhotos: (ids: number[]) => Promise<void>;
  refreshFolders: () => Promise<void>;
  /** Picks a folder, adds it to the library and scans it. */
  /** Adds `path`, or a folder chosen in the picker when there is none. */
  addLibraryFolder: (path?: string) => Promise<void>;
  /** Confirms, then takes a folder out of the library, forgetting its photos. */
  removeLibraryFolder: (folder: LibraryFolder) => Promise<void>;
  /** "Locate folder…": picks where a moved folder is now, and rescans it. */
  locateLibraryFolder: (folder: LibraryFolder) => Promise<void>;
  /** Confirms, then forgets photos that belong to no library folder. */
  forgetUnfiledPhotos: () => Promise<void>;
  /** Scans one folder, or every folder that is reachable. */
  runScan: (root?: string) => Promise<void>;
}

export const createLibrarySlice: Slice<LibrarySlice> = (set, get, store) => ({
  libraryFolders: null,
  photos: [],
  facets: null,
  extraFacets: null,
  refreshExtraFacets: async () => {
    const groups = extraGroups(get().railSections);
    // Any group at all: listing them by name missed one added later.
    if (!Object.values(groups).some(Boolean)) return;
    const mine = ++extraSeq;
    try {
      const extra = await api.getExtraFacets(viewFilterFor(get(), get().committedFilter), groups);
      // A later request (another click, another section opened) wins.
      if (mine === extraSeq) set({ extraFacets: extra });
    } catch (e) {
      console.error("extra facet counts failed", e);
    }
  },
  loading: false,

  filter: EMPTY_FILTER,
  committedFilter: EMPTY_FILTER,

  scanning: null,

  setFilter: (f) => {
    set({ filter: f });
    // Commit — and therefore query — only once typing settles. Reloading
    // from here rather than from a component effect keeps the debounce in
    // one place instead of split across the store and the tree.
    clearTimeout(filterTimer);
    filterTimer = setTimeout(() => {
      set({ committedFilter: get().filter });
      // The selection keeps the photos still shown and drops the rest,
      // saying so. It was cleared on every filter change, so
      // narrowing 60 picked frames by a rating, or clicking a keyword,
      // threw the work away.
      const kept = get().selected;
      // Changing a filter leaves content-search mode — the two describe
      // different sets and showing one under the other's controls confuses.
      get().clearSearch();
      void get()
        .reload()
        .then(() => keepShownSelection(store, kept));
    }, FILTER_DEBOUNCE_MS);
  },

  // The order is a view preference, not a filter: clearing keeps it.
  clearFilter: () => {
    set({ queryReading: null });
    get().setFilter({ ...EMPTY_FILTER, sort: get().filter.sort ?? "newest" });
  },

  setScanning: (p) => set({ scanning: p }),
  // A scan's events and its reply travel separately, so its last events
  // can arrive after `scanOnce` cleared `scanning`. Taking them then left
  // it set for good, and automatic analysis, which waits for the app to be
  // idle, never ran again.
  scanProgress: (p) => {
    if (get().scanning !== null) set({ scanning: p });
  },

  init: async () => {
    await get().refreshFolders();
    void get().refreshAi();
    void get().loadCollections();
    void get().loadSmartCollections();
    void get().loadShoots();
    void get().loadKeywords();
    await get().reload();
    // Pick up anything added to the folder since the last session, so the
    // library is not silently stale until someone remembers to hit Scan.
    //
    // Deliberately not awaited. The grid is already painted from the index
    // above, and a re-scan drops known files on a head-hash before any
    // decode — but that is still an open+read per file, so it must never
    // sit between launch and a usable window. Progress shows in the footer,
    // the Activity panel and the rail pulse like any other background job.
    // Then analysis, whether or not the scan found anything new: a pass cut
    // short by quitting, or a feature turned on since, is picked up here
    // rather than waiting for the next import.
    void get()
      .runScan()
      .then(() => get().analyseNewPhotos());
    // A catalogue backup a day, taken in the background. The catalogue is
    // the only copy of everything decided in it.
    if (get().autoBackup) void api.backupCatalogueIfDue().catch(() => undefined);
  },

  reload: async () => {
    const mine = ++reloadSeq;
    set({ loading: true });
    try {
      const filter = viewFilterFor(get(), get().committedFilter);
      // A newer reload stops this one's queries where they are.
      const request = { page: VIEW_PAGE, seq: mine };
      const [photos, facets] = await Promise.all([
        api.getPhotos(filter, PAGE, 0, request),
        api.getFacets(filter, request),
      ]);
      if (mine !== reloadSeq) return;
      set({ photos, facets });
      void get().refreshExtraFacets();
      // Once per language: names for tags a vision model wrote freely.
      void learnTagNames(facets);
    } catch (e) {
      if (mine !== reloadSeq) return;
      console.error("load failed", e);
      set({ error: String(e) });
    } finally {
      if (mine === reloadSeq) set({ loading: false });
    }
  },

  refreshPhotos: async (ids) => {
    if (ids.length === 0) return;
    const st = get();
    const onPage = new Set(st.photos.map((p) => p.id));
    if (
      !isFilterEmpty(st.committedFilter) ||
      !sortIsByDate(st.committedFilter.sort) ||
      st.searchResults !== null ||
      ids.some((id) => !onPage.has(id))
    ) {
      return st.reload();
    }
    const mine = reloadSeq;
    try {
      const [rows, facets] = await Promise.all([
        api.getPhotosByIds(ids),
        api.getFacets(viewFilterFor(get(), st.committedFilter)),
      ]);
      // A full reload since then has the newer answer.
      if (mine !== reloadSeq) return;
      const byId = new Map(rows.map((p) => [p.id, p]));
      const apply = (p: Photo): Photo => byId.get(p.id) ?? p;
      set({
        photos: get().photos.map(apply),
        lightboxSet: get().lightboxSet?.map(apply) ?? null,
        facets,
      });
      void get().refreshExtraFacets();
    } catch (e) {
      console.error("refresh failed", e);
      await get().reload();
    }
  },

  refreshFolders: async () => {
    try {
      set({ libraryFolders: await api.listLibraryFolders() });
    } catch (e) {
      console.error("listing library folders failed", e);
    }
  },

  addLibraryFolder: async (path) => {
    const dir = path ?? (await open({ directory: true, multiple: false, title: t("store.folder.addTitle") }));
    if (typeof dir !== "string") return;
    // A card's photos catalogued in place are gone, with their ratings
    // and edits, when the card is formatted. Importing copies them
    // first; adding the card itself takes a second, deliberate yes.
    if (await api.folderIsCard(dir).catch(() => false)) {
      const importInstead = await confirm({
        title: t("store.folder.cardTitle"),
        message: t("store.folder.cardMessage"),
        confirmLabel: t("store.folder.cardImport"),
        cancelLabel: t("store.folder.cardNotNow"),
      });
      if (importInstead) {
        get().setShowImport(true);
        return;
      }
      const anyway = await confirm({
        title: t("store.folder.cardAnywayTitle"),
        message: t("store.folder.cardAnywayMessage"),
        confirmLabel: t("store.folder.cardAnyway"),
        danger: true,
      });
      if (!anyway) return;
    }
    try {
      await api.addLibraryFolder(dir);
    } catch (e) {
      toast(String(e), "error");
      return;
    }
    await get().refreshFolders();
    // Photos already catalogued from this folder are filed under it now,
    // so show them before the scan finds the rest.
    await get().reload();
    await get().runScan(dir);
  },

  removeLibraryFolder: async (folder) => {
    const name = folder.path.split(/[\\/]/).pop() || folder.path;
    const ok = await confirm({
      title: t("store.folder.removeTitle", { name }),
      message: t("store.folder.removeMessage", { count: folder.photos }),
      confirmLabel: t("store.folder.removeConfirm"),
      danger: true,
    });
    if (!ok) return;
    try {
      const n = await api.removeLibraryFolder(folder.id);
      toast(t("store.folder.removed", { name, count: n }), "success");
    } catch (e) {
      toast(String(e), "error");
    }
    get().clearSelection();
    await get().refreshFolders();
    await get().reload();
  },

  locateLibraryFolder: async (folder) => {
    const name = folder.path.split(/[\\/]/).pop() || folder.path;
    const dir = await open({ directory: true, multiple: false, title: t("store.folder.locateTitle", { name }) });
    if (typeof dir !== "string") return;
    try {
      await api.relocateLibraryFolder(folder.id, dir);
    } catch (e) {
      toast(String(e), "error");
      return;
    }
    await get().refreshFolders();
    // The rescan unmarks what is really there, and says what is not.
    await get().runScan(dir);
    await get().reload();
  },

  forgetUnfiledPhotos: async () => {
    const n = get().libraryFolders?.unfiled ?? 0;
    const ok = await confirm({
      title: t("store.folder.forgetTitle", { count: n }),
      message: t("store.folder.forgetMessage"),
      confirmLabel: t("store.folder.forgetConfirm"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.forgetUnfiledPhotos();
    } catch (e) {
      toast(String(e), "error");
    }
    await get().refreshFolders();
    await get().reload();
  },

  runScan: async (root) => {
    // Serialise. The launch scan starts unprompted, so adding a folder or a
    // manual re-scan can land on top of it — and `scan_library` opens its
    // own SQLite connection, so two at once would be two writers and two
    // interleaved `scan-progress` streams driving one progress bar.
    const run = scanQueue.then(async () => {
      const folders = get().libraryFolders?.folders ?? [];
      // A folder that is not there (an unplugged drive) is not walked: it
      // would find nothing and mark every photo in it missing, which the
      // folder list already says more precisely.
      const targets = root ? [root] : folders.filter((f) => f.online).map((f) => f.path);
      if (targets.length > 0) await scanOnce(set, get, targets);
    });
    scanQueue = run.catch(() => undefined);
    return run;
  },
});
