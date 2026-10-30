import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { formatNumber, t, type Key } from "../../src/i18n";
import { applyReading, readQuery, type Vocabulary } from "../../src/lib/queryReading";
import { EMPTY_FILTER, type PhotoFilter } from "../../src/types";
import {
  api,
  EDIT_COMPUTER,
  EDIT_RAW,
  editedUrl,
  previewUrl,
  standinUrl,
  thumbUrl,
  type SyncSummary,
  type Tile,
} from "./api";
import { catalogueDate, monthKey, monthLabel, takenText } from "./dates";
import { backPast, gonePast, pushStep } from "./back";
import Editor from "./Editor";
import Grid from "./Grid";
import { SyncButton, syncedText, useLink, type Conn } from "./link";
import Loupe, { type Verdict } from "./Loupe";
import { Choice, Dropdown, EmptyState, IconButton, ScreenTitle, StatusDot, useScreen } from "./ui";

/** The scroll test's size, when no computer is paired (thumbs.rs). */
const STANDINS = 20_000;

/** What the library can show, from the menu under its title. */
const VIEWS = ["all", "picks", "unflagged", "rejects", "rated", "edited"] as const;
type View = (typeof VIEWS)[number];
const IN_VIEW: Record<View, (tile: Tile) => boolean> = {
  all: () => true,
  picks: (tile) => tile[1] > 0,
  unflagged: (tile) => tile[1] === 0,
  rejects: (tile) => tile[1] < 0,
  rated: (tile) => tile[2] > 0,
  edited: (tile) => tile[3],
};
const VIEW_LABEL: Record<View, Key> = {
  all: "mobile.view.all",
  picks: "filters.flag.picks",
  unflagged: "filters.flag.unflagged",
  rejects: "filters.flag.rejects",
  rated: "mobile.view.rated",
  edited: "filters.status.edited",
};

const verdictOf = (flag: number): Verdict | null => (flag > 0 ? "pick" : flag < 0 ? "reject" : null);
const flagOf = (v: Verdict | null) => (v === "pick" ? 1 : v === "reject" ? -1 : 0);

/**
 * The library on the phone (docs/MOBILE.md): the phone's copy of the paired
 * computer's photos, newest first, shown at once from what is already here so
 * it works offline, then brought up to date over the link. Picks and rejects
 * are kept on the phone and sent to the computer.
 *
 * Non-raw photos can be edited here too (docs/MOBILE.md section 4, step 6):
 * Edit opens the photo's preview in the editor, and the edits go to the
 * computer as they are made, or with the next sync.
 *
 * The large title above the grid says what it shows, and opens a menu of
 * the others: picks, rejects, unflagged, rated or edited photos. Beside it,
 * the order: newest or oldest first. The title scrolls away with the grid,
 * and the bar above, with the app's menu and Search, takes it over.
 *
 * Unpaired, it says so and offers pairing. The app's menu then has the
 * scroll test (`scrollTest`): a grid of stand-ins, so scrolling can be
 * measured on a phone before anything is paired.
 */
export default function Library({
  desktop,
  onPair,
  scrollTest,
  toTop,
}: {
  /** Null when no computer is paired. */
  desktop: string | null;
  onPair: () => void;
  scrollTest: boolean;
  /** Bumped to scroll back to the top. */
  toTop: number;
}) {
  const real = desktop !== null;
  // The link is the app's (`LinkProvider`): how it stands, and its syncs.
  const { conn, lastSynced, syncNow, onSynced } = useLink();
  const [tiles, setTiles] = useState<Tile[] | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  // What the grid shows, and in which order.
  const [view, setView] = useState<View>("all");
  const [newestFirst, setNewestFirst] = useState(true);
  // A menu dropping from the title or the order button, and from where.
  const [menu, setMenu] = useState<{ kind: "view" | "order"; anchor: DOMRect } | null>(null);
  // While a photo is open, a view keeps the photos it had when it opened,
  // so picking one under Unflagged doesn't take it away mid-look.
  const [kept, setKept] = useState<Set<number> | null>(null);
  // The large title has scrolled away, so the top bar shows it.
  const [titleHidden, setTitleHidden] = useState(false);
  // A search: what was asked, and the photos found (null while none is on).
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<{ query: string; ids: number[]; ranked: boolean } | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  // The library photo being edited.
  const [editing, setEditing] = useState<number | null>(null);
  const [standinVerdicts, setStandinVerdicts] = useState<Map<number, Verdict>>(() => new Map());
  // Photos sent from the computer: said for a while after they land.
  const [notice, setNotice] = useState<{ text: string; failed: boolean } | null>(null);
  const noticeTimer = useRef(0);
  const noteReceived = useCallback((s: SyncSummary) => {
    let next: { text: string; failed: boolean } | null = null;
    if (s.receiveFailed) next = { text: t("mobile.receive.failed", { error: s.receiveFailed }), failed: true };
    else if (s.editsCopied > 0) next = { text: t("mobile.libraryEdit.copied", { count: s.editsCopied }), failed: false };
    else if (s.received > 0) next = { text: t("mobile.receive.saved", { count: s.received }), failed: false };
    else if (s.added > 0) next = { text: t("mobile.add.done", { count: s.added }), failed: false };
    if (!next) return;
    setNotice(next);
    window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(null), 8000);
  }, []);

  const loadTiles = useCallback(async () => {
    if (real) setTiles(await api.photos());
  }, [real]);

  // What the phone already has first, then each sync's changes; the first
  // sync after opening loads them whatever it says.
  useEffect(() => {
    void loadTiles();
  }, [loadTiles]);
  const synced = useRef(false);
  useEffect(
    () =>
      onSynced((s) => {
        noteReceived(s);
        if (s.changed > 0 || !synced.current) void loadTiles();
        synced.current = true;
      }),
    [onSynced, noteReceived, loadTiles],
  );
  // Pulled down: a sync now, said over the grid until it's done.
  const [pulling, setPulling] = useState(false);
  const pullSync = async () => {
    setPulling(true);
    await syncNow();
    setPulling(false);
  };

  const { width, twoPane } = useScreen();

  // On a tablet, the photo beside the grid can go full screen, and back.
  const [expanded, setExpanded] = useState(false);
  const full = !twoPane || expanded;

  // Android's Back gesture closes the loupe (full screen, or the tablet's
  // pane back to no photo) rather than leaving the app: opening it adds a
  // Back step (`back.ts`), and Back pops it. So do Close and Esc. The
  // tablet's full screen and the menus take a step each too.
  const loupeAt = useRef<number | null>(null);
  const fullAt = useRef<number | null>(null);
  const menuAt = useRef<number | null>(null);
  const openLoupe = (i: number) => {
    if (loupeAt.current === null) {
      loupeAt.current = pushStep("loupe");
      if (real && shown) setKept(new Set(shown.map((tile) => tile[0])));
    }
    // Edit is a tap away: the GPU gets ready meanwhile.
    if (real) void api.editWarm().catch(() => {});
    setOpen(i);
  };
  useEffect(() => {
    if (open === null) setKept(null);
  }, [open]);
  const closeLoupe = () => history.back();
  /** Lets go of the photo as Back would, for a new view or search. */
  const letGo = () => {
    if (loupeAt.current !== null) backPast(loupeAt.current);
  };
  const expand = () => {
    fullAt.current = pushStep("loupeFull");
    setExpanded(true);
  };
  // A tablet turned upright shows the photo full screen anyway, so its
  // full-screen step goes, and Back from there lets go of the photo. Not
  // under the editor, whose own step is above it.
  useEffect(() => {
    if (!twoPane && fullAt.current !== null && editing === null) backPast(fullAt.current);
    // Only when the layout changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [twoPane]);
  const openMenu = (kind: "view" | "order", anchor: DOMRect) => {
    menuAt.current = pushStep("libraryMenu");
    setMenu({ kind, anchor });
  };
  // A choice closes the menu and lets go of the photo, in one step back.
  const chooseView = (next: View) => {
    setView(next);
    backPast(loupeAt.current ?? menuAt.current!);
  };
  const chooseOrder = (newest: boolean) => {
    setNewestFirst(newest);
    backPast(loupeAt.current ?? menuAt.current!);
  };
  // Editing opens over the loupe, with its own Back step.
  const openEditor = (id: number) => {
    setEditing(id);
  };
  const editingNow = useRef(editing);
  editingNow.current = editing;
  useEffect(() => {
    const onPop = () => {
      // The editor takes care of its own Back step, and says when it has
      // gone (its onClose).
      if (editingNow.current !== null) return;
      // Whatever Back has gone past closes: a menu, a tablet's full screen
      // back beside the grid, the photo.
      if (gonePast(menuAt.current)) {
        menuAt.current = null;
        setMenu(null);
      }
      if (gonePast(fullAt.current)) {
        fullAt.current = null;
        setExpanded(false);
      }
      if (gonePast(loupeAt.current)) {
        loupeAt.current = null;
        setOpen(null);
      }
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  // What the grid shows: the whole library, or what a search found, best
  // first when it ranks them, otherwise in the library's order; then only
  // the view's photos, newest or oldest first.
  const shown = useMemo(() => {
    if (!tiles) return tiles;
    let list = tiles;
    if (found && found.ranked) {
      const byId = new Map(tiles.map((tile) => [tile[0], tile]));
      list = found.ids.map((id) => byId.get(id)).filter((tile): tile is Tile => tile !== undefined);
    } else if (found) {
      const hit = new Set(found.ids);
      list = tiles.filter((tile) => hit.has(tile[0]));
    }
    if (view !== "all") list = list.filter((tile) => kept?.has(tile[0]) || IN_VIEW[view](tile));
    if (!newestFirst && !found?.ranked) list = [...list].reverse();
    return list;
  }, [tiles, found, view, newestFirst, kept]);
  // How many photos each view has, for its menu.
  const viewCounts = useMemo(() => {
    const counts = Object.fromEntries(VIEWS.map((v) => [v, 0])) as Record<View, number>;
    for (const tile of tiles ?? []) for (const v of VIEWS) if (IN_VIEW[v](tile)) counts[v] += 1;
    return counts;
  }, [tiles]);

  const search = async () => {
    const q = query.trim();
    if (!q) return;
    setSearching(true);
    setSearchError(null);
    try {
      // Read the query as the computer's search box does: people, places,
      // dates and gear become the filter, and the rest searches what is in
      // the photos.
      const vocab = JSON.parse(await api.libraryVocabulary()) as Vocabulary;
      const reading = readQuery(q, vocab, new Date());
      let filter: PhotoFilter = { ...EMPTY_FILTER };
      let content = q;
      if (reading.parts.length > 0) {
        filter = applyReading(filter, reading).filter;
        content = reading.rest;
      }
      if (content.trim()) filter = { ...filter, semantic: content.trim() };
      const result = await api.librarySearch(filter);
      letGo();
      setFound({ query: q, ...result });
    } catch (e) {
      const code = String(e);
      setSearchError(
        code === "needsComputer"
          ? t("mobile.search.needsComputer")
          : code === "updateComputer"
            ? t("mobile.libraryEdit.updateComputer")
            : t("mobile.search.failed"),
      );
    } finally {
      setSearching(false);
    }
  };
  const endSearch = () => {
    setSearchOpen(false);
    setFound(null);
    setQuery("");
    setSearchError(null);
    letGo();
  };

  const count = real ? (shown?.length ?? 0) : scrollTest ? STANDINS : 0;
  const idAt = (i: number) => (real ? shown![i]![0] : i);
  const verdictAt = (i: number): Verdict | null =>
    real ? verdictOf(shown![i]![1]) : (standinVerdicts.get(i) ?? null);

  const setVerdict = (i: number, v: Verdict | null) => {
    if (!real) {
      setStandinVerdicts((m) => {
        const next = new Map(m);
        if (v) next.set(i, v);
        else next.delete(i);
        return next;
      });
      return;
    }
    const id = shown![i]![0];
    setTiles((all) => all!.map((tile): Tile => (tile[0] === id ? [tile[0], flagOf(v), ...(tile.slice(2) as [number, boolean, number, string | null, string | null])] : tile)));
    void api.setFlag(id, flagOf(v));
  };

  const setRating = (i: number, rating: number) => {
    const id = shown![i]![0];
    setTiles((all) => all!.map((tile): Tile => (tile[0] === id ? [tile[0], tile[1], rating, ...(tile.slice(3) as [boolean, number, string | null, string | null])] : tile)));
    void api.setRating(id, rating);
  };

  // A raw, or a photo whose edits only the computer can show, has no Edit:
  // a word says why instead. The editor itself says anything else.
  const openEditOn = real && open !== null && open < count ? shown![open]![4] : null;
  const editable = openEditOn !== null && openEditOn !== EDIT_RAW && openEditOn !== EDIT_COMPUTER;
  const editNote =
    openEditOn === EDIT_RAW
      ? t("mobile.libraryEdit.raw")
      : openEditOn === EDIT_COMPUTER
        ? t("mobile.libraryEdit.computerOnly")
        : null;

  // With a keyboard (a tablet's): the arrows step through the photos, and
  // Esc closes the one open.
  useEffect(() => {
    if (open === null || editing !== null) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement | null)?.closest("input, textarea")) return;
      if (e.key === "Escape") history.back();
      else if (e.key === "ArrowRight" && open < count - 1) setOpen(open + 1);
      else if (e.key === "ArrowLeft" && open > 0) setOpen(open - 1);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, editing, count]);

  // The open photo's file name, for the loupe's top bar.
  const openId = real && open !== null && open < count ? idAt(open) : null;
  const [openName, setOpenName] = useState<string | null>(null);
  useEffect(() => {
    setOpenName(null);
    if (openId === null) return;
    let gone = false;
    void api
      .libraryPhotoName(openId)
      .then((name) => !gone && setOpenName(name))
      .catch(() => {});
    return () => {
      gone = true;
    };
  }, [openId]);
  const openTaken = real && open !== null && open < count ? catalogueDate(shown![open]![6]) : null;

  // Month headings, as the desktop's grid has; not for a search ranked by
  // how well photos match, which has no order by date.
  const sectionAt = useCallback((i: number) => monthKey(catalogueDate(shown![i]![6])), [shown]);
  const sections = real && !found?.ranked;

  const loupe =
    open !== null && open < count ? (
      <Loupe
        index={open}
        count={count}
        thumb={real ? thumbUrl(idAt(open)) : standinUrl(open)}
        // An edited photo shows as edited, raws as developed on the computer;
        // the plain preview if that can't be had.
        preview={real ? (shown![open]![5] ? editedUrl(idAt(open), shown![open]![5]!) : previewUrl(idAt(open))) : null}
        fallback={real ? previewUrl(idAt(open)) : undefined}
        verdict={verdictAt(open)}
        onIndex={setOpen}
        onVerdict={(v) => setVerdict(open, v)}
        rating={real ? shown![open]![2] : undefined}
        onRating={real ? (r) => setRating(open, r) : undefined}
        onClose={full ? closeLoupe : undefined}
        pane={full ? undefined : { onClose: closeLoupe, onExpand: expand }}
        onEdit={editable ? () => openEditor(idAt(open)) : undefined}
        note={editNote}
        title={openTaken ? takenText(openTaken) : null}
        subtitle={openName}
      />
    ) : null;

  const title = real ? t(VIEW_LABEL[view]) : scrollTest ? t("mobile.scrollTest.title") : t("mobile.tabs.library");
  // Above the grid, scrolling with it: what it shows, its order, and the
  // link's state.
  const titleBlock = (
    <div className="pt-1 pr-1 pb-3 pl-4">
      <div className="flex min-h-12 items-center gap-1">
        <ScreenTitle title={title} onOpen={real ? (anchor) => openMenu("view", anchor) : undefined} />
        <div className="flex-1" />
        {real && !found?.ranked && (
          <IconButton
            icon="sort"
            label={t("mobile.order.label")}
            onClick={(e) => openMenu("order", e.currentTarget.getBoundingClientRect())}
          />
        )}
      </div>
      {real && <StatusLine conn={conn} count={shown?.length ?? null} lastSynced={lastSynced} />}
    </div>
  );

  return (
    <div className="flex h-full flex-col bg-base">
      <header
        className={`shrink-0 border-b bg-base pt-[var(--safe-top)] ${titleHidden ? "border-line" : "border-transparent"}`}
      >
        <div className="flex h-14 items-center gap-1 pr-1 pl-4">
          <div
            aria-hidden={!titleHidden}
            className={`min-w-0 flex-1 truncate font-display text-title text-ink transition-opacity duration-150 ${titleHidden ? "opacity-100" : "opacity-0"}`}
          >
            {title}
          </div>
          {real && !searchOpen && (
            <IconButton icon="search" label={t("mobile.search.open")} onClick={() => setSearchOpen(true)} />
          )}
          {scrollTest ? <IconButton icon="close" label={t("common.close")} onClick={() => history.back()} /> : <SyncButton />}
        </div>
      </header>
      {/* Can't reach the computer: one quiet line, as the sync button says
          too; the sheet has why and what to do. */}
      {real && conn.state === "down" && (
        <div className="flex min-h-12 items-center gap-2 border-b border-line bg-panel pr-1 pl-4">
          <p className="min-w-0 flex-1 truncate text-caption text-muted">
            {t("mobile.link.offline", { name: desktop })}
            {lastSynced !== null && <span className="text-faint"> · {syncedText(lastSynced)}</span>}
          </p>
          <button className="btn-ghost min-h-12 shrink-0 px-3 text-caption text-ink" title={conn.error} onClick={() => void syncNow()}>
            {t("mobile.link.retry")}
          </button>
        </div>
      )}
      {searchOpen && (
        <div className="border-b border-line bg-raised px-4 py-2">
          <form
            className="flex items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void search();
            }}
          >
            <input
              className="min-h-12 min-w-0 flex-1 rounded-md bg-base px-3 text-body text-ink"
              type="search"
              enterKeyHint="search"
              value={query}
              placeholder={t("mobile.search.placeholder")}
              aria-label={t("mobile.search.open")}
              autoFocus
              onChange={(e) => setQuery(e.target.value)}
            />
            <button className="btn-accent min-h-12 shrink-0 px-4 text-body" disabled={searching || !query.trim()} type="submit">
              {searching ? t("mobile.search.searching") : t("mobile.search.go")}
            </button>
            <IconButton icon="close" label={t("common.close")} onClick={endSearch} />
          </form>
          {searchError ? (
            <p className="pt-2 text-label text-danger">{searchError}</p>
          ) : (
            found &&
            found.ids.length > 0 && (
              <p className="pt-2 text-label text-muted">
                {t("mobile.search.count", { count: found.ids.length, query: found.query })}
              </p>
            )
          )}
        </div>
      )}
      {notice && (
        <div
          role="status"
          className={`border-b border-line bg-raised px-4 py-2 text-label ${notice.failed ? "text-danger" : "text-ink"}`}
        >
          {notice.text}
        </div>
      )}
      {!real && !scrollTest ? (
        <>
          {titleBlock}
          <EmptyState icon="grid" title={t("mobile.library.unpairedTitle")} hint={t("mobile.library.unpairedHint")}>
            <button className="btn-accent mt-4 min-h-12 px-6 text-body" onClick={onPair}>
              {t("mobile.pair.submit")}
            </button>
          </EmptyState>
        </>
      ) : real && tiles !== null && count === 0 ? (
        <>
          {titleBlock}
          {found && found.ids.length === 0 ? (
            <EmptyState icon="search" title={t("mobile.search.none", { query: found.query })} />
          ) : found || view !== "all" ? (
            <EmptyState icon="filter" title={t("mobile.view.empty")} />
          ) : conn.state === "up" ? (
            <EmptyState icon="folder" title={t("mobile.library.emptyTitle")} hint={t("mobile.library.empty")} />
          ) : (
            <EmptyState icon="grid" title={t("mobile.library.notCopied")} />
          )}
        </>
      ) : real && tiles === null ? (
        <>
          {titleBlock}
          <div className="flex-1" />
        </>
      ) : (
        <div className="flex min-h-0 flex-1">
          <Grid
            width={twoPane ? Math.round(width * 0.42) : width}
            count={count}
            url={(i) => (real ? thumbUrl(idAt(i)) : standinUrl(i))}
            verdictAt={verdictAt}
            selected={open}
            onOpen={openLoupe}
            meter={!real}
            editedAt={real ? (i) => shown![i]![3] : undefined}
            ratingAt={real ? (i) => shown![i]![2] : undefined}
            sectionAt={sections ? sectionAt : undefined}
            sectionLabel={monthLabel}
            header={titleBlock}
            onHeaderHidden={setTitleHidden}
            onRefresh={real ? () => void pullSync() : undefined}
            refreshing={pulling}
            toTop={toTop}
          />
          {twoPane && (
            <div className="flex min-w-0 flex-1 flex-col border-l border-line bg-black">
              {!expanded && loupe ? loupe : <EmptyState icon="loupe" title={t("mobile.loupe.choose")} />}
            </div>
          )}
        </div>
      )}
      {full && loupe && <div className="fixed inset-0 z-40">{loupe}</div>}
      {editing !== null && (
        <div className="fixed inset-0 z-50">
          <Editor
            source={{ kind: "library", id: editing }}
            preview={previewUrl(editing)}
            onClose={() => {
              setEditing(null);
              void loadTiles();
            }}
          />
        </div>
      )}
      {menu?.kind === "view" && (
        <Dropdown anchor={menu.anchor} onClose={() => history.back()}>
          {VIEWS.map((v) => (
            <Choice
              key={v}
              label={t(VIEW_LABEL[v])}
              detail={formatNumber(viewCounts[v])}
              selected={view === v}
              onClick={() => chooseView(v)}
            />
          ))}
        </Dropdown>
      )}
      {menu?.kind === "order" && (
        <Dropdown anchor={menu.anchor} align="right" onClose={() => history.back()}>
          <Choice label={t("store.types.sortNewest")} selected={newestFirst} onClick={() => chooseOrder(true)} />
          <Choice label={t("store.types.sortOldest")} selected={!newestFirst} onClick={() => chooseOrder(false)} />
        </Dropdown>
      )}
    </div>
  );
}

/** The header's second line: a status light and the library's size, and
 *  when it last synced, or what the link is doing. Away from home it says it
 *  is going through the relay. Why it can't connect is in the sync sheet. */
function StatusLine({ conn, count, lastSynced }: { conn: Conn; count: number | null; lastSynced: number | null }) {
  const photos = count !== null ? t("mobile.library.count", { count }) : "";
  const when = lastSynced !== null ? syncedText(lastSynced) : null;
  const [tone, text] =
    conn.state === "connecting"
      ? (["warning", t("mobile.link.connecting")] as const)
      : conn.state === "syncing"
        ? (["warning", photos ? `${photos} · ${t("mobile.sync.syncing")}` : t("mobile.library.syncing")] as const)
        : conn.state === "up"
          ? ([
              "positive",
              [photos, conn.relayed ? t("mobile.link.viaRelay") : when].filter(Boolean).join(" · "),
            ] as const)
          : (["danger", photos || t("mobile.link.notConnected")] as const);
  return (
    <div className="flex items-center gap-2 text-caption text-muted" role="status">
      <StatusDot tone={tone} />
      <span className="truncate">{text}</span>
    </div>
  );
}
