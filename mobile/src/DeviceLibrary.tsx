import { useCallback, useEffect, useRef, useState } from "react";
import { formatNumber, t } from "../../src/i18n";
import { Icon } from "../../src/lib/icons";
import {
  api,
  devicePreviewUrl,
  deviceThumbUrl,
  type DeviceAccess,
  type DeviceAdd,
  type DeviceAlbum,
  type DevicePhoto,
  type LinkStatus,
} from "./api";
import { monthKey, monthLabel, takenText } from "./dates";
import { backPast, gonePast, pushStep } from "./back";
import Editor from "./Editor";
import Grid from "./Grid";
import { SyncButton, useLink } from "./link";
import Loupe, { EditButton } from "./Loupe";
import { Choice, Dropdown, EmptyState, IconButton, ScreenTitle, useScreen } from "./ui";

/**
 * On this phone (docs/MOBILE.md section 4): the phone's own photos, apart
 * from the synced library. Read where they are; nothing leaves the phone.
 * Browsed by album, opened, and edited (`Editor`), with the edits kept as
 * settings and copies saved to the gallery. With a computer paired, photos
 * can be added to the library: chosen ones wait for the next sync, then go,
 * edits and all, and become library photos. Select, or a long press on a
 * photo, starts choosing.
 *
 * The large title is the album, and opens a menu of the others; beside it,
 * the order. As in the library, it scrolls away with the grid.
 */
export default function DeviceLibrary({
  active,
  paired,
  platform,
  toTop,
}: {
  active: boolean;
  paired: boolean;
  /** Where to change the photo permission differs between Android and iOS. */
  platform: LinkStatus["platform"];
  /** Bumped to scroll back to the top. */
  toTop: number;
}) {
  const { syncNow } = useLink();
  /** Null while asking; "phoneOnly" when the app runs on a computer. */
  const [access, setAccess] = useState<DeviceAccess | "phoneOnly" | null>(null);
  /** Asked this session and still refused: Android won't ask again. */
  const [refused, setRefused] = useState(false);
  const [albums, setAlbums] = useState<DeviceAlbum[]>([]);
  const [album, setAlbum] = useState<string | null>(null);
  const [newestFirst, setNewestFirst] = useState(true);
  // A menu dropping from the title or the order button, and from where.
  const [menu, setMenu] = useState<{ kind: "album" | "order"; anchor: DOMRect } | null>(null);
  // The large title has scrolled away, so the top bar shows it.
  const [titleHidden, setTitleHidden] = useState(false);
  const [photos, setPhotos] = useState<DevicePhoto[] | null>(null);
  // Every photo the phone lets the app see, whichever album is showing:
  // counted when all of them are listed. Android's albums add up to it, but
  // iOS's overlap and leave photos out.
  const [allowedCount, setAllowedCount] = useState(0);
  const sectionAt = useCallback(
    (i: number) => {
      const taken = photos?.[i]?.[1];
      return monthKey(taken ? new Date(taken) : null);
    },
    [photos],
  );
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  /** The photo being edited. */
  const [editing, setEditing] = useState<DevicePhoto | null>(null);
  const [edited, setEdited] = useState<Set<number>>(() => new Set());
  /** Add to library, by phone photo: waiting, added, or not added. */
  const [adds, setAdds] = useState<Map<number, DeviceAdd>>(() => new Map());
  /** Choosing photos to add: the ones chosen; null when not choosing. */
  const [choosing, setChoosing] = useState<Set<number> | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const allowed = access === "all" || access === "some";
  /** The phone has answered once: a later failure is an error, not a computer. */
  const answered = useRef(false);

  // What the phone allows now, and its photos if it allows any. Run on
  // opening the tab and on coming back to the app: there may be new photos,
  // or the person may have changed the permission in Settings.
  const refresh = useCallback(async () => {
    try {
      const now = await api.deviceAccess();
      answered.current = true;
      setAccess(now);
      if (now === "none") return;
      const [list, found, marked, added] = await Promise.all([
        api.devicePhotos(album),
        api.deviceAlbums(),
        api.editEdited().catch(() => [] as number[]),
        api.deviceAdds().catch(() => [] as DeviceAdd[]),
      ]);
      setPhotos(newestFirst ? list : list.reverse());
      if (album === null) setAllowedCount(list.length);
      setAlbums(found);
      setEdited(new Set(marked));
      setAdds(new Map(added.map((a) => [a.photo, a])));
      setError(null);
    } catch (e) {
      if (!answered.current) setAccess("phoneOnly");
      else setError(String(e));
    }
  }, [album, newestFirst]);

  useEffect(() => {
    if (!active) return;
    void refresh();
    const onVisible = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
    // `refresh` changes with the album, which reloads, as it should.
  }, [active, refresh]);

  // While photos wait for the library, their marks follow the syncs.
  const waitingCount = [...adds.values()].filter((a) => a.libraryId === null && a.error === null).length;
  useEffect(() => {
    if (!active || waitingCount === 0) return;
    const timer = window.setInterval(() => {
      api
        .deviceAdds()
        .then((added) => setAdds(new Map(added.map((a) => [a.photo, a]))))
        .catch(() => {});
    }, 3000);
    return () => window.clearInterval(timer);
  }, [active, waitingCount]);

  // When photos that were waiting go, say how it went, in place of "will be
  // added".
  const wasWaiting = useRef<Set<number>>(new Set());
  useEffect(() => {
    const now = new Set([...adds.values()].filter((a) => a.libraryId === null && a.error === null).map((a) => a.photo));
    const gone = [...wasWaiting.current].filter((id) => !now.has(id));
    const done = gone.filter((id) => adds.get(id)?.libraryId != null).length;
    const failed = gone.filter((id) => adds.get(id)?.error).length;
    if (failed > 0) setNotice(t("mobile.add.someFailed", { count: failed }));
    else if (done > 0) setNotice(t("mobile.add.done", { count: done }));
    wasWaiting.current = now;
  }, [adds]);

  const addToLibrary = async (ids: number[]) => {
    try {
      const queued = await api.deviceAdd(ids);
      setChoosing(null);
      if (queued > 0) setNotice(t("mobile.add.queued", { count: queued }));
      setAdds(new Map((await api.deviceAdds()).map((a) => [a.photo, a])));
      // They go with a sync: one now, rather than within ten seconds.
      if (queued > 0) void syncNow();
    } catch (e) {
      setError(String(e));
    }
  };

  /** What a tile's corner says: in the library, on its way, or edited. */
  const markFor = (id: number): string | null => {
    const a = adds.get(id);
    if (a?.libraryId != null) return t("mobile.add.inLibraryMark");
    if (a && a.error) return t("mobile.add.failedMark");
    if (a) return t("mobile.add.waitingMark");
    return edited.has(id) ? t("mobile.edit.mark") : null;
  };

  const ask = async () => {
    try {
      const now = await api.deviceRequestAccess();
      setRefused(now === "none");
      setAccess(now);
      if (now !== "none") void refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const { width, twoPane } = useScreen();

  // On a tablet, the photo beside the grid can go full screen, and back.
  const [expanded, setExpanded] = useState(false);
  const full = !twoPane || expanded;

  // Back closes the loupe (full screen, or the tablet's pane back to no
  // photo), as in the library; so do Close and Esc. Each takes a Back step
  // (`back.ts`), as do the tablet's full screen and the menus.
  const loupeAt = useRef<number | null>(null);
  const fullAt = useRef<number | null>(null);
  const menuAt = useRef<number | null>(null);
  const openLoupe = (i: number) => {
    if (loupeAt.current === null) loupeAt.current = pushStep("deviceLoupe");
    // Edit is a tap away: the GPU gets ready meanwhile.
    void api.editWarm().catch(() => {});
    setOpen(i);
  };
  const expand = () => {
    fullAt.current = pushStep("deviceLoupeFull");
    setExpanded(true);
  };
  // A tablet turned upright shows the photo full screen anyway, so its
  // full-screen step goes. Not under the editor, whose step is above it.
  useEffect(() => {
    if (!twoPane && fullAt.current !== null && editing === null) backPast(fullAt.current);
    // Only when the layout changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [twoPane]);
  // Editing opens over the loupe, with its own Back step.
  const openEditor = (p: DevicePhoto) => {
    setEditing(p);
  };
  // A menu takes a Back step too.
  const openMenu = (kind: "album" | "order", anchor: DOMRect) => {
    menuAt.current = pushStep("deviceMenu");
    setMenu({ kind, anchor });
  };
  // A choice closes the menu and lets go of the photo, in one step back.
  const chooseAlbum = (next: string | null) => {
    setAlbum(next);
    backPast(loupeAt.current ?? menuAt.current!);
  };
  const chooseOrder = (newest: boolean) => {
    setNewestFirst(newest);
    backPast(loupeAt.current ?? menuAt.current!);
  };
  const editingNow = useRef(editing);
  editingNow.current = editing;
  useEffect(() => {
    const onPop = () => {
      // The editor takes care of its own Back step, and says when it has
      // gone (its onClose).
      if (editingNow.current) return;
      // Whatever Back has gone past closes.
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

  const count = photos?.length ?? 0;
  // With a keyboard: the arrows step through the photos, and Esc closes.
  useEffect(() => {
    if (open === null || editing !== null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") history.back();
      else if (e.key === "ArrowRight" && open < count - 1) setOpen(open + 1);
      else if (e.key === "ArrowLeft" && open > 0) setOpen(open - 1);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, editing, count]);
  const loupe =
    photos && open !== null && open < count ? (
      <Loupe
        index={open}
        count={count}
        thumb={deviceThumbUrl(photos[open]!)}
        preview={devicePreviewUrl(photos[open]!)}
        onIndex={setOpen}
        onClose={full ? () => history.back() : undefined}
        pane={full ? undefined : { onClose: () => history.back(), onExpand: expand }}
        title={photos[open]![1] ? takenText(new Date(photos[open]![1])) : null}
        actions={<LoupeActions photo={photos[open]!} add={adds.get(photos[open]![0])} paired={paired} onEdit={openEditor} onAdd={(id) => void addToLibrary([id])} />}
      />
    ) : null;

  const title = album === null ? t("mobile.device.title") : albums.find((a) => a.id === album)?.name || "?";
  const showsGrid = allowed && photos && count > 0;
  // Above the grid, scrolling with it: the album, its order and its size.
  const titleBlock = (
    <div className="pt-1 pr-1 pb-3 pl-4">
      <div className="flex min-h-12 items-center gap-1">
        <ScreenTitle
          title={title}
          onOpen={allowed && albums.length > 1 && !choosing ? (anchor) => openMenu("album", anchor) : undefined}
        />
        <div className="flex-1" />
        {allowed && photos && count > 1 && (
          <IconButton
            icon="sort"
            label={t("mobile.order.label")}
            onClick={(e) => openMenu("order", e.currentTarget.getBoundingClientRect())}
          />
        )}
      </div>
      {showsGrid && <div className="text-caption text-muted">{t("mobile.library.count", { count })}</div>}
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
          {showsGrid && paired && !choosing && (
            <button className="btn-ghost min-h-12 shrink-0 px-3 text-body text-ink" onClick={() => setChoosing(new Set())}>
              {t("mobile.add.select")}
            </button>
          )}
          <SyncButton />
        </div>
      </header>

      {choosing && (
        <div className="flex items-center gap-2 border-b border-line bg-raised px-4 py-2">
          <p className="min-w-0 flex-1 text-caption text-ink">{t("mobile.add.chosen", { count: choosing.size })}</p>
          <button className="btn-ghost min-h-12 shrink-0 text-body text-muted" onClick={() => setChoosing(null)}>
            {t("mobile.add.cancel")}
          </button>
          <button
            className="btn-accent min-h-12 shrink-0 px-4 text-body"
            disabled={choosing.size === 0}
            onClick={() => void addToLibrary([...choosing])}
          >
            {t("mobile.add.add")}
          </button>
        </div>
      )}
      {notice && (
        <p role="status" className="border-b border-line px-4 py-2 text-label text-ink" onClick={() => setNotice(null)}>
          {notice}
        </p>
      )}
      {access === "some" && (
        <div className="flex items-center gap-3 border-b border-line bg-raised px-4 py-2">
          <p className="min-w-0 flex-1 text-label text-muted">{t("mobile.device.some", { count: allowedCount })}</p>
          <button className="btn-ghost min-h-12 shrink-0 text-ink" onClick={() => void ask()}>
            {t("mobile.device.chooseMore")}
          </button>
        </div>
      )}
      {error && <p className="px-4 py-2 text-label text-danger">{error}</p>}
      {!showsGrid && titleBlock}

      {access === "phoneOnly" && <EmptyState icon="phone" title={t("mobile.device.phoneOnly")} />}

      {access === "none" && (
        <EmptyState icon="phone" title={t("mobile.device.askTitle")} hint={t("mobile.device.askBody")}>
          <button className="btn-accent mt-4 min-h-12 px-6 text-body" onClick={() => void ask()}>
            {t("mobile.device.allow")}
          </button>
          {refused && (
            <p className="mt-4 max-w-xs text-caption leading-relaxed text-muted">
              {t(platform === "ios" ? "mobile.device.refusedIos" : "mobile.device.refused")}
            </p>
          )}
        </EmptyState>
      )}

      {allowed && photos && count === 0 && <EmptyState icon="phone" title={t("mobile.device.empty")} />}
      {showsGrid && (
        <div className="flex min-h-0 flex-1">
          <Grid
            width={twoPane ? Math.round(width * 0.42) : width}
            count={count}
            url={(i) => deviceThumbUrl(photos[i]!)}
            verdictAt={() => null}
            selected={choosing ? null : open}
            onOpen={(i) => {
              if (!choosing) return openLoupe(i);
              const id = photos[i]![0];
              const next = new Set(choosing);
              if (next.has(id)) next.delete(id);
              else next.add(id);
              setChoosing(next);
            }}
            meter={false}
            markAt={(i) => markFor(photos[i]![0])}
            sectionAt={sectionAt}
            sectionLabel={monthLabel}
            chosenAt={choosing ? (i) => choosing.has(photos[i]![0]) : undefined}
            onLongPress={paired && !choosing ? (i) => setChoosing(new Set([photos[i]![0]])) : undefined}
            header={titleBlock}
            onHeaderHidden={setTitleHidden}
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
      {menu?.kind === "album" && (
        <Dropdown anchor={menu.anchor} onClose={() => history.back()}>
          <Choice
            label={t("mobile.device.allPhotos")}
            detail={formatNumber(allowedCount)}
            selected={album === null}
            onClick={() => chooseAlbum(null)}
          />
          {albums.map((a) => (
            <Choice
              key={a.id}
              label={a.name || "?"}
              detail={formatNumber(a.count)}
              selected={album === a.id}
              onClick={() => chooseAlbum(a.id)}
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
      {editing && (
        <div className="fixed inset-0 z-50">
          <Editor
            source={{ kind: "device", photo: editing }}
            preview={devicePreviewUrl(editing)}
            onClose={() => {
              setEditing(null);
              void refresh();
            }}
            onSaved={() => void refresh()}
          />
        </div>
      )}
    </div>
  );
}

/**
 * The loupe's bar for a phone photo: Edit, and Add to library with a computer
 * paired. Once added, the photo's edits belong to the library copy, so it
 * says where it is instead.
 */
function LoupeActions({
  photo,
  add,
  paired,
  onEdit,
  onAdd,
}: {
  photo: DevicePhoto;
  add: DeviceAdd | undefined;
  paired: boolean;
  onEdit: (p: DevicePhoto) => void;
  onAdd: (id: number) => void;
}) {
  if (add?.libraryId != null) {
    return <p className="flex-1 py-3 text-center text-caption text-muted">{t("mobile.add.inLibrary")}</p>;
  }
  const waiting = add && !add.error;
  const raw = Boolean(photo[5]);
  return (
    <>
      {paired &&
        (waiting ? (
          <p className="flex-1 py-3 text-center text-caption text-muted">{t("mobile.add.waiting")}</p>
        ) : (
          <button className="btn-solid min-h-12 flex-1 gap-2 text-body" onClick={() => onAdd(photo[0])} title={add?.error ?? undefined}>
            <Icon name="import" size={20} />
            {add?.error ? t("mobile.add.retry") : t("mobile.add.add")}
          </button>
        ))}
      {raw ? (
        <p className="flex-1 py-3 text-center text-caption text-muted">{t("mobile.edit.rawNotYet")}</p>
      ) : (
        <EditButton onClick={() => onEdit(photo)} />
      )}
    </>
  );
}
