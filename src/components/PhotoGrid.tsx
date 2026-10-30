import { periodRange } from "../lib/timeline";
import { sameFrames, useStable } from "../lib/useStable";
import { tileName } from "../lib/tileName";
import { sentenceCase } from "../lib/format";
import { overlayOpen } from "../lib/useEscape";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api, fileUrl } from "../lib/api";
import { toast } from "../lib/dialogs";
import { buildStackCells, type GridCell } from "../lib/stacks";
import {
  buildGridRows,
  cellRowIndex,
  DAY_ROW_H,
  MONTH_ROW_H,
  monthLabelAt,
} from "../lib/gridRows";
import { useEntranceWave } from "../lib/useEntranceWave";
import { developShown, useStore } from "../store";
import { formatNumber, getLocale, t } from "../i18n";
import { tagName } from "../i18n/tags";
import { localPlaceName } from "../lib/countries";
import { Icon } from "../lib/icons";
import ProgressBar from "./ProgressBar";
import { ColorLabelDot, FlagBadge, RatingBadge } from "./PhotoBadges";
import { isFilterEmpty, sortIsByDate, type Photo } from "../types";
import { useEditedPath } from "../lib/useEditedPath";

const GAP = 6;

export default function PhotoGrid() {
  // Content-search results take over the grid while a search is active.
  const photos = useStore((s) => s.searchResults ?? s.photos);
  const selected = useStore((s) => s.selected);
  const focusId = useStore((s) => s.focusId);
  const filter = useStore((s) => s.committedFilter);
  const scanning = useStore((s) => s.scanning);
  const runScan = useStore((s) => s.runScan);
  const addFolder = useStore((s) => s.addLibraryFolder);
  const cellSize = useStore((s) => s.cellSize);
  const onSelect = useStore((s) => s.select);
  const openPhotoMenu = useStore((s) => s.openPhotoMenu);

  /** Right-click (or the menu key) on a photo: act on the selection, first
   *  making this photo the selection if it is not part of it. */
  const menuFor = (id: number, x: number, y: number) => {
    const sel = useStore.getState().selected;
    const ids = sel.has(id) ? [...sel] : [id];
    if (!sel.has(id)) onSelect(id, "single");
    openPhotoMenu(x, y, ids, "browse");
  };
  const onOpen = useStore((s) => s.openLightbox);
  // A collection shown in its own order is arranged by dragging; a
  // drag with Alt held still hands the files out, as everywhere else.
  const arranging = useStore(
    (s) => s.committedFilter.collectionId != null && s.committedFilter.sort === "custom" && s.searchResults === null,
  );
  const [arrange, setArrange] = useState<{ ids: number[]; over: number | null; after: boolean } | null>(null);
  const beginArrange = (ids: number[]) => {
    let state = { ids, over: null as number | null, after: false };
    setArrange(state);
    const prevCursor = document.body.style.cursor;
    document.body.style.cursor = "grabbing";
    const onMove = (e: PointerEvent) => {
      const el = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-photo-id]");
      if (!el) return;
      const r = el.getBoundingClientRect();
      state = { ids, over: Number(el.dataset.photoId), after: e.clientX > r.left + r.width / 2 };
      setArrange(state);
    };
    const end = (drop: boolean) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("keydown", onKey, true);
      document.body.style.cursor = prevCursor;
      setArrange(null);
      if (!drop || state.over === null || ids.includes(state.over)) return;
      // In the order they are shown, and before whatever follows the drop
      // point once they are lifted out.
      const order = photos.map((x) => x.id);
      const moving = order.filter((id) => ids.includes(id));
      const rest = order.filter((id) => !ids.includes(id));
      const at = rest.indexOf(state.over) + (state.after ? 1 : 0);
      void useStore.getState().arrangeCollection(moving, rest[at] ?? null);
    };
    const onUp = () => end(true);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      end(false);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("keydown", onKey, true);
  };
  const stacking = useStore((s) => s.stacking);
  const stackGroups = useStore((s) => s.stackGroups);
  const expandedStacks = useStore((s) => s.expandedStacks);
  const toggleStack = useStore((s) => s.toggleStack);
  const groupByDate = useStore((s) => s.groupByDate);
  // Semantic results come back by relevance, not date, and a sort by rating or
  // name has nothing to do with time either: headings over them would
  // split the grid at arbitrary places.
  const dateOrdered = useStore(
    (s) => s.searchResults === null && !s.committedFilter.semantic && sortIsByDate(s.committedFilter.sort),
  );

  const parentRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);

  // Play the staggered entrance only when the displayed set actually changes —
  // a filter applied/cleared or a search run, not a rating — and switch it back
  // off once the wave has finished, so scrolling (which recycles cells) never
  // re-triggers it.
  const animating = useEntranceWave(photos, 700);

  // Bursts collapse to one cell each when stacking is on; otherwise every photo
  // is its own cell. Either way the grid renders a flat list of cells.
  //
  // Built from the photos as laid out: a rating, flag or label leaves
  // the cells, rows and their measurements as they were, and each cell looks
  // its photo up as it is now when it draws. Rebuilding 20,000 cells and every
  // date row on each verdict cost ~15 ms a key before anything drew.
  const layout = useStable(photos, sameFrames);
  const current = useMemo(() => new Map(photos.map((p) => [p.id, p])), [photos]);
  const cells: GridCell[] = useMemo(
    () =>
      stacking && stackGroups
        ? buildStackCells(layout, stackGroups, expandedStacks)
        : layout.map((p) => ({ photo: p })),
    [layout, stacking, stackGroups, expandedStacks],
  );

  // The lightbox indexes into the displayed photo list, so a cell maps back to
  // its photo's position there (stacking reorders and hides cells, so the cell
  // index is not the photo index).
  const indexById = useMemo(() => {
    const m = new Map<number, number>();
    layout.forEach((p, i) => m.set(p.id, i));
    return m;
  }, [layout]);

  // Column count is derived from the measured width so the grid reflows with
  // the window and the thumbnail-size slider.
  //
  // Per element, not once: the empty state and the grid are separate elements
  // (keyed apart, below), and an observer left on the one that went away read
  // a width of 0 — every photo row collapsed to nothing when a search that
  // found none was cleared.
  const empty = photos.length === 0;
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      if (entry) setWidth(entry.contentRect.width);
    });
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, [empty]);

  const cols = Math.max(1, Math.floor((width + GAP) / (cellSize + GAP)));
  const cellW = cols > 0 ? (width - GAP * (cols - 1)) / cols : cellSize;
  const rowHeight = cellW + GAP;

  // The grid is no longer "cells sliced by index": with date headings between
  // them, rows are a described list of varying heights, and a cell's position
  // in the flat list no longer follows from its row number.
  const grouped = groupByDate && dateOrdered;
  // The headings are worded dates, so a change of language rebuilds them.
  const locale = getLocale();
  const rows = useMemo(() => buildGridRows(cells, cols, grouped), [cells, cols, grouped, locale]);
  const rowCount = rows.length;
  // Headings are presentation, so the grid's rows are the photo rows alone:
  // counting the headings made aria-rowcount and -rowindex disagree
  // with what a screen reader can move through.
  const photoRows = useMemo(() => {
    const index: number[] = [];
    let n = 0;
    for (const r of rows) index.push(r.kind === "photos" ? ++n : 0);
    return { index, count: n };
  }, [rows]);
  const rowOfCell = useMemo(() => cellRowIndex(rows), [rows]);

  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: (i) => {
      const row = rows[i];
      if (!row) return rowHeight;
      return row.kind === "month" ? MONTH_ROW_H : row.kind === "day" ? DAY_ROW_H : rowHeight;
    },
    overscan: 4,
  });

  // The timeline strip asked for a month or a day: scroll to its
  // heading, or its first photo. When the grid isn't in date order, or the
  // period isn't among the photos loaded, show that period instead.
  const gridJump = useStore((s) => s.gridJump);
  useEffect(() => {
    if (!gridJump) return;
    const { period } = gridJump;
    const heading = rows.findIndex((r) => r.key === (period.length === 7 ? `m${period}` : `d${period}`));
    const cell = dateOrdered ? cells.findIndex((c) => c.photo.captureDate?.startsWith(period)) : -1;
    const row = heading >= 0 ? heading : cell >= 0 ? rowOfCell.get(cell) : undefined;
    if (row !== undefined) {
      rowVirtualizer.scrollToIndex(row, { align: "start" });
      return;
    }
    const { from, to } = periodRange(period);
    const s = useStore.getState();
    s.setFilter({ ...s.filter, dateFrom: from, dateTo: to });
    // Only a new request moves the grid, not a reflow.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gridJump]);

  // Row heights change when the size slider moves, and the whole row list
  // changes when grouping or the column count does; re-measure so offsets stay
  // right rather than accumulating from a stale estimate.
  useEffect(() => {
    rowVirtualizer.measure();
  }, [rowHeight, cols, rows, rowVirtualizer]);

  // Scroll position survives a mode switch. The grid unmounts whenever you visit
  // Cull/Places/People, and "Review N picks" hands you straight back here — so
  // without this you restarted at the top of the library every round trip.
  // Restored once the container has a width (and therefore a scrollable height).
  const restoredScroll = useRef(false);
  useEffect(() => {
    const el = parentRef.current;
    if (!el || restoredScroll.current || width === 0 || rowCount === 0) return;
    restoredScroll.current = true;
    el.scrollTop = useStore.getState().gridScrollTop;
  }, [width, rowCount]);
  useEffect(() => {
    const el = parentRef.current;
    return () => {
      if (el) useStore.getState().setGridScrollTop(el.scrollTop);
    };
  }, []);

  // --- Keyboard navigation -------------------------------------------------
  //
  // The grid was mouse-only: cells were plain divs with an onClick, so they
  // could not be focused, selected or opened from the keyboard at all — even
  // though the rate/flag/label shortcuts work here. That made those shortcuts
  // half-usable, since you had to reach for the mouse to move between photos.
  //
  // Traversal is handled here rather than through `keymap`, which is for photo
  // *actions* (rate, flag, develop) and only carries a 1-D prev/next. A grid
  // needs 2-D movement plus Home/End/Page keys, and arrow-key traversal inside a grid
  // widget is a structural convention rather than a preference to rebind.

  /** Index of the focused photo among the *cells* (stacking hides some). */
  const focusIndex = useMemo(
    () => (focusId === null ? -1 : cells.findIndex((c) => c.photo.id === focusId)),
    [cells, focusId],
  );

  const focusedCellRef = useRef<HTMLDivElement>(null);

  const moveTo = (target: number, extend: boolean) => {
    const cell = cells[Math.min(cells.length - 1, Math.max(0, target))];
    if (cell) onSelect(cell.photo.id, extend ? "range" : "single");
  };

  /**
   * Up/down, one *photo* row at a time.
   *
   * Not `± cols` any more: a day's last row is often short, and headings sit
   * between rows, so a fixed stride walks off the end of a short row or lands
   * on a heading. Walk the row list instead, keeping the column where it fits.
   */
  const moveVertically = (dir: 1 | -1, from: number, extend: boolean, by = 1) => {
    const start = rowOfCell.get(from);
    if (start === undefined) {
      moveTo(from + dir * cols * by, extend);
      return;
    }
    const row = rows[start];
    const col = row?.kind === "photos" ? from - row.firstIndex : 0;
    // Walk `by` photo rows, stopping at the last one there is.
    let landed: number | null = null;
    let left = by;
    for (let i = start + dir; i >= 0 && i < rows.length && left > 0; i += dir) {
      const next = rows[i];
      if (next?.kind !== "photos") continue;
      landed = next.firstIndex + Math.min(col, next.cells.length - 1);
      left -= 1;
    }
    if (landed !== null) moveTo(landed, extend);
  };

  /** How many photo rows fit on screen: Page Up/Down's stride. */
  const pageRows = () =>
    Math.max(1, Math.floor((parentRef.current?.clientHeight ?? rowHeight) / rowHeight) - 1);

  const onGridKey = (e: React.KeyboardEvent) => {
    const el = e.target as HTMLElement | null;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
    // The grid is behind the Lightbox or a dialog then: its arrows moved the
    // hidden selection while the Lightbox stayed put.
    // Nor behind Compare or Develop, where focus can come back to the grid
    // as a view closes over another.
    const st = useStore.getState();
    if (st.lightboxIndex !== null || st.compare || developShown(st) || overlayOpen()) return;
    // Anything before the first move starts from the top-left cell.
    const from = focusIndex < 0 ? 0 : focusIndex;
    switch (e.key) {
      case "ArrowRight": moveTo(from + 1, e.shiftKey); break;
      case "ArrowLeft": moveTo(from - 1, e.shiftKey); break;
      case "ArrowDown": moveVertically(1, from, e.shiftKey); break;
      case "ArrowUp": moveVertically(-1, from, e.shiftKey); break;
      // A screenful at a time, as every list on the desktop does.
      case "PageDown": moveVertically(1, from, e.shiftKey, pageRows()); break;
      case "PageUp": moveVertically(-1, from, e.shiftKey, pageRows()); break;
      case "Home": moveTo(0, e.shiftKey); break;
      case "End": moveTo(cells.length - 1, e.shiftKey); break;
      case "Enter":
      case " ": {
        const cell = cells[from];
        if (cell) onOpen(indexById.get(cell.photo.id) ?? 0);
        break;
      }
      // The keyboard's way into the right-click menu: the menu key, or
      // Shift+F10, anchored under the focused photo.
      case "ContextMenu":
      case "F10": {
        if (e.key === "F10" && !e.shiftKey) return;
        const cell = cells[from];
        const rect = (e.target as HTMLElement).getBoundingClientRect();
        if (cell) menuFor(cell.photo.id, rect.left + rect.width / 2, rect.top + rect.height / 2);
        break;
      }
      default:
        return;
    }
    e.preventDefault();
    // The focused grid owns its traversal keys. Without this they would also
    // reach BrowseWorkspace's window listener, so a user who rebound an action
    // onto an arrow key would move *and* fire the action on one press.
    e.stopPropagation();
  };

  // Follow the focused cell: scroll its row in, then move DOM focus to it — but
  // only when focus is already inside the grid, so clicking a photo and then
  // typing in the search box doesn't yank focus back. The rAF gives the
  // virtualiser a frame to render the row before we reach for its node.
  useEffect(() => {
    const root = parentRef.current;
    if (!root || focusIndex < 0) return;
    // Which row holds this cell is a lookup now, not `index / cols` — headings
    // shift every row below them, so the arithmetic would land short.
    const row = rowOfCell.get(focusIndex);
    if (row === undefined) return;
    rowVirtualizer.scrollToIndex(row, { align: "auto" });
    if (!root.contains(document.activeElement)) return;
    const raf = requestAnimationFrame(() => focusedCellRef.current?.focus({ preventScroll: true }));
    return () => cancelAnimationFrame(raf);
  }, [focusIndex, rowOfCell, rowVirtualizer]);

  if (photos.length === 0 && scanning && isFilterEmpty(filter)) {
    // A scan into an empty library (a first folder): the photos trickle in as
    // they are read, but until the first of them lands the state below said
    // "No photos were found. Add a folder…" to someone who just had.
    const known = scanning.total > 0;
    return (
      <div key="empty" ref={parentRef} className="grid flex-1 place-items-center overflow-auto text-center">
        <div className="w-72 max-w-full">
          <div className="mb-3 flex justify-center text-faint opacity-60">
            <Icon name="import" size={30} strokeWidth={1.4} />
          </div>
          <div className="text-title text-muted">{t("library.grid.readingTitle")}</div>
          <div className="mt-1 text-caption leading-relaxed text-faint">{t("library.grid.readingBody")}</div>
          <ProgressBar
            pct={known ? (scanning.current / scanning.total) * 100 : null}
            label={t("library.browse.scanningBar")}
            className="mx-auto mt-4 h-1 w-48 overflow-hidden rounded-full bg-line"
          />
          <div className="mt-2 text-label text-faint" aria-live="polite">
            {known ? (
              <span className="tabular-nums">{t("common.progress", { current: scanning.current, total: scanning.total })}</span>
            ) : (
              t("library.browse.scanningStart")
            )}
          </div>
        </div>
      </div>
    );
  }

  if (photos.length === 0) {
    // One message used to cover three different situations, and was wrong in
    // two of them: an empty library has no filter to clear, and a content query
    // lives in the top bar rather than the left rail.
    const state = filter.semantic
      ? {
          icon: "search" as const,
          title: t("library.grid.noMatchTitle"),
          body: t("library.grid.noMatchBody"),
        }
      : !isFilterEmpty(filter)
        ? {
            icon: "search" as const,
            title: t("library.grid.noFilterMatchTitle"),
            body: t("library.grid.noFilterMatchBody"),
          }
        : {
            icon: "import" as const,
            title: t("library.grid.emptyTitle"),
            body: t("library.grid.emptyBody"),
          };
    return (
      // Keyed apart from the grid below, so switching between them replaces
      // the element rather than letting React reuse the grid's. Reused, it
      // kept day headings from the photos just filtered away and lost the
      // `relative` they are positioned against: on a Mac they were drawn at
      // the window's left edge, over the mode rail, until the next scroll.
      <div
        key="empty"
        ref={parentRef}
        className="grid flex-1 place-items-center overflow-auto text-center"
      >
        <div className="max-w-xs">
          <div className="mb-3 flex justify-center text-faint opacity-60">
            <Icon name={state.icon} size={30} strokeWidth={1.4} />
          </div>
          <div className="text-title text-muted">{state.title}</div>
          <div className="mt-1 text-caption leading-relaxed text-faint">{state.body}</div>
          {/* The body offers both, so the buttons do: it said "Add a folder"
              and had only Scan. */}
          {isFilterEmpty(filter) && (
            <div className="mt-3 flex flex-wrap justify-center gap-2">
              <button onClick={() => void addFolder()} className="btn-solid">
                <Icon name="plus" size={14} />
                {t("library.libraryMenu.addFolder")}
              </button>
              <button onClick={() => void runScan()} disabled={scanning !== null} className="btn-solid">
                <Icon name="import" size={14} />
                {t("library.grid.scanForNew")}
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  const virtualRows = rowVirtualizer.getVirtualItems();

  // The grid's single tab stop. It follows the focused photo, but falls back to
  // the first rendered cell when that photo has been scrolled outside the
  // virtual window — otherwise no cell would carry `tabIndex={0}` and Tab would
  // skip the grid altogether. Both ends come from the rendered rows themselves,
  // since a row index no longer maps to a cell index by multiplication.
  const renderedTiles = virtualRows
    .map((r) => rows[r.index])
    .filter((r) => r?.kind === "photos");
  const firstRendered = renderedTiles[0]?.firstIndex ?? 0;
  const lastTiles = renderedTiles[renderedTiles.length - 1];
  const lastRendered = lastTiles ? lastTiles.firstIndex + lastTiles.cells.length - 1 : 0;
  const tabStopIndex =
    focusIndex >= firstRendered && focusIndex <= lastRendered ? focusIndex : firstRendered;

  // The month the top of the viewport is inside, pinned so scrolling deep into
  // a long shoot never loses track of when you are.
  //
  // Taken from the row the top edge actually sits in — not `firstRow`, which
  // includes the overscan rows already scrolled past and would name the month
  // above the one on screen. Suppressed while that month's own heading is at
  // the top edge, where the bar would only repeat the heading beneath it.
  const scrollOffset = rowVirtualizer.scrollOffset ?? 0;
  const topVirtual = virtualRows.find((v) => v.start + v.size > scrollOffset) ?? virtualRows[0];
  const topRow = topVirtual ? rows[topVirtual.index] : undefined;
  const stickyMonth =
    grouped && topVirtual && topRow && topRow.kind !== "month"
      ? monthLabelAt(rows, topVirtual.index)
      : null;

  return (
    <div
      key="grid"
      ref={parentRef}
      role="grid"
      aria-label={t("library.grid.label")}
      aria-rowcount={photoRows.count}
      aria-colcount={cols}
      onKeyDown={onGridKey}
      className="relative flex-1 overflow-y-auto overflow-x-hidden p-2"
    >
      {stickyMonth && (
        // A solid band, not a fade to transparent: the fade left the label
        // sitting on the photo beneath it.
        //
        // Pinned at -8px, and zero-height with the band drawn inside it. A
        // sticky element sticks inside its scroller's padding, so `top-0` held
        // it 8px (this grid's `p-2`) below the top edge, and photos scrolled
        // past in the gap above it. In flow, it also pushed the grid down by
        // its own height whenever it appeared.
        <div className="pointer-events-none sticky -top-2 z-10 -mx-2 h-0">
          <div className="absolute inset-x-0 top-0 bg-base/95 px-3 pb-2 pt-2 backdrop-blur-sm">
            <span className="font-display text-caption font-medium tracking-tight text-faint">
              {stickyMonth}
            </span>
          </div>
        </div>
      )}
      <div
        style={{ height: rowVirtualizer.getTotalSize(), position: "relative" }}
      >
        {virtualRows.map((virtual) => {
          const row = rows[virtual.index];
          if (!row) return null;

          const place = {
            position: "absolute" as const,
            top: 0,
            left: 0,
            width: "100%",
            transform: `translateY(${virtual.start}px)`,
          };

          // Headings are `presentation`, not grid rows: they carry no cells, so
          // announcing them as rows would make the row count disagree with what
          // a screen reader can actually move through.
          if (row.kind === "month") {
            return (
              <div
                key={row.key}
                role="presentation"
                style={{ ...place, height: MONTH_ROW_H }}
                className="flex items-end px-1 pb-2 pt-4"
              >
                <h2 className="font-display text-display text-ink">
                  {row.label}
                </h2>
              </div>
            );
          }
          if (row.kind === "day") {
            return (
              <div
                key={row.key}
                role="presentation"
                style={{ ...place, height: DAY_ROW_H }}
                className="flex items-end gap-2 px-1 pb-1.5"
              >
                <span className="text-caption font-medium text-muted">{row.label}</span>
                <span className="text-label tabular-nums text-faint">
                  {formatNumber(row.count)}
                </span>
              </div>
            );
          }

          const start = row.firstIndex;
          const items = row.cells;
          return (
            <div
              key={row.key}
              role="row"
              aria-rowindex={photoRows.index[virtual.index]}
              style={{
                ...place,
                height: rowHeight,
                display: "flex",
                gap: GAP,
              }}
            >
              {items.map((cell, ci) => {
                const p = current.get(cell.photo.id) ?? cell.photo;
                const isSelected = selected.has(p.id);
                const stack = cell.stack;
                // The head of a collapsed stack, or any frame in an expanded
                // one, gets an accent ring so the burst reads as a group.
                const stacked = stack && (stack.head ? !stack.expanded : true);
                // Stagger the entrance across the viewport (row-major), capped so
                // a dense grid still finishes its wave inside the animating window.
                const delay = animating
                  ? Math.min((start - firstRendered + ci) * 6, 120)
                  : 0;
                const cellIndex = start + ci;
                // Roving tabindex: exactly one cell is in the tab order, so Tab
                // steps into the grid and out of it rather than through 20,000
                // photos. A `gridcell` rather than a `button` because the stack
                // badge is a real button nested inside it.
                return (
                  <div
                    key={p.id}
                    ref={cellIndex === focusIndex ? focusedCellRef : undefined}
                    role="gridcell"
                    aria-colindex={ci + 1}
                    aria-selected={isSelected}
                    // Its state as well as its name: stars, verdict,
                    // label, missing, edited — what the badges show the eye.
                    aria-label={tileName(p)}
                    tabIndex={cellIndex === tabStopIndex ? 0 : -1}
                    onClick={(e) =>
                      onSelect(
                        p.id,
                        e.shiftKey ? "range" : e.ctrlKey || e.metaKey ? "toggle" : "single",
                      )
                    }
                    onDoubleClick={() => onOpen(indexById.get(p.id) ?? 0)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      menuFor(p.id, e.clientX, e.clientY);
                    }}
                    // Out to Finder, Mail or an editor as the original files —
                    // the selection if this photo is in it, else this photo.
                    // The webview's own drag would carry a thumbnail image;
                    // the native one carries the files, always as copies.
                    draggable
                    data-photo-id={p.id}
                    onDragStart={(e) => {
                      e.preventDefault();
                      const ids = isSelected ? [...selected] : [p.id];
                      if (arranging && !e.altKey) {
                        beginArrange(ids);
                        return;
                      }
                      api.dragPhotosOut(ids).catch((err) => toast(String(err), "error"));
                    }}
                    style={{
                      width: cellW,
                      height: cellW,
                      animationDelay: animating ? `${delay}ms` : undefined,
                    }}
                    className={`group relative overflow-hidden rounded-sm bg-raised ring-offset-1 ring-offset-base transition-shadow ${
                      animating ? "grid-cell-in" : ""
                    } ${
                      isSelected
                        ? "ring-2 ring-accent"
                        : stacked
                          ? "ring-1 ring-accent/45"
                          : "hover:ring-1 hover:ring-line"
                    }`}
                  >
                    {arrange?.over === p.id && !arrange.ids.includes(p.id) && (
                      <span
                        aria-hidden
                        className="pointer-events-none absolute inset-y-0 z-10 w-1 bg-accent"
                        style={arrange.after ? { right: 0 } : { left: 0 }}
                      />
                    )}
                    {arrange?.ids.includes(p.id) && (
                      <span aria-hidden className="pointer-events-none absolute inset-0 z-10 bg-base/50" />
                    )}
                    <TileImage photo={p} />

                    {/* Metadata only on hover — the grid should read as photos, not chrome.
                        Lifted over the bottom badges when there are any:
                        they covered the chips, leaving "ldlife" for "Wildlife". */}
                    <div
                      className={`pointer-events-none absolute inset-x-0 bottom-0 flex items-end gap-1 bg-gradient-to-t from-black/80 to-transparent p-1.5 opacity-0 transition-opacity group-hover:opacity-100 ${
                        p.missing || p.variant > 0 || p.edited || stack?.head ? "pb-7" : ""
                      }`}
                    >
                      {p.genre && (
                        <span className="rounded-sm bg-chip px-1 text-label text-ink backdrop-blur">
                          {sentenceCase(tagName(p.genre))}
                        </span>
                      )}
                      {p.place && (
                        <span className="truncate rounded-sm bg-chip px-1 text-label text-ink backdrop-blur">
                          {localPlaceName(p.place)}
                        </span>
                      )}
                    </div>

                    {/* Flag + rating, stacked top-left so a cull reads at a glance. */}
                    <div className="absolute left-1 top-1 flex flex-col items-start gap-0.5">
                      <FlagBadge flag={p.flag} />
                      <RatingBadge rating={p.rating} />
                      {/* Colour label sits with the other state markers so the
                          hover tag chips at the bottom never ride over it. */}
                      <ColorLabelDot label={p.colorLabel} className="mt-0.5" />
                    </div>

                    {!p.tagged && (
                      <div
                        className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-accent/70"
                        title={t("library.grid.notTagged")}
                      />
                    )}

                    {/* The file is not where it was. The row and its work
                        are kept, so it stays in the grid — but it has to say why
                        it will not open, rather than fail when it is clicked. */}
                    {p.missing && (
                      <div
                        className="absolute inset-x-1 bottom-1 flex items-center gap-1 rounded-sm bg-chip px-1.5 py-0.5 text-label text-warning backdrop-blur"
                        title={t("library.grid.missingTooltip", { path: p.path })}
                      >
                        <Icon name="warn" size={10} className="shrink-0" />
                        <span className="truncate">{t("library.grid.fileMissing")}</span>
                      </div>
                    )}

                    {/* A copy sits next to the photo it came from, so it has to
                        say which of the two it is — otherwise the grid just
                        looks like it has duplicates in it. An edited original
                        gets the same badge without the name. */}
                    {!p.missing && (p.variant > 0 || p.edited) && (
                      <div
                        className="absolute bottom-1 left-1 flex items-center gap-1 rounded-sm bg-chip px-1 py-0.5 text-label text-ink backdrop-blur"
                        title={
                          p.variant > 0
                            ? p.variantName
                              ? t("library.grid.copyNamedTooltip", { name: p.variantName })
                              : t("library.grid.copyTooltip")
                            : t("library.grid.editedTooltip")
                        }
                      >
                        <Icon name="adjust" size={10} />
                        {p.variant > 0 && (
                          <span className="max-w-24 truncate">{p.variantName ?? t("library.grid.copy")}</span>
                        )}
                      </div>
                    )}

                    {/* Stack badge on the representative: click to expand/collapse
                        the burst. Kept off the top-left flag stack and the
                        bottom-left colour dot. */}
                    {stack?.head && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          toggleStack(stack.id);
                        }}
                        className="absolute bottom-1 right-1 flex items-center gap-0.5 rounded-sm bg-chip px-1 py-0.5 text-label font-medium text-ink backdrop-blur transition-colors hover:bg-chip-hover"
                        title={
                          stack.expanded
                            ? t("library.grid.collapseBurst")
                            : t("library.grid.expandBurst", { count: stack.count })
                        }
                      >
                        <Icon name="chevron" size={11} className={`transition-transform ${stack.expanded ? "" : "-rotate-90"}`} />
                        <Icon name="layers" size={11} />
                        <span className="leading-none tabular-nums">{stack.count}</span>
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** A tile's picture: the photo as edited once rendered, else the camera's thumbnail. */
function TileImage({ photo }: { photo: Photo }) {
  const src = useEditedPath(photo) ?? photo.thumbPath;
  return src ? (
    <img
      src={fileUrl(src)}
      alt={photo.filename}
      loading="lazy"
      draggable={false}
      className="h-full w-full object-cover"
    />
  ) : (
    <div className="grid h-full place-items-center text-faint">?</div>
  );
}
