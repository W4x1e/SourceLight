import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { t } from "../../src/i18n";
import { Icon } from "../../src/lib/icons";
import type { Verdict } from "./Loupe";

/** How far, in px, the grid must be pulled down to sync, and the most it
 *  follows the finger. */
const PULL = { at: 64, most: 110 };

/** A tile is never narrower than this: three across on a phone, more on a tablet. */
const MIN_TILE = 120;
/** A month heading's height. */
const HEADING = 52;
/** Tiles across beyond those that fit at `MIN_TILE`: spreading two fingers
 *  takes them away (down to one fewer), pinching adds them (up to three
 *  more). */
const DENSITY = { least: -1, most: 3 };
const DENSITY_KEY = "sourcelight.gridDensity";

function savedDensity(): number {
  try {
    const v = Number(localStorage.getItem(DENSITY_KEY));
    return Number.isInteger(v) ? Math.min(DENSITY.most, Math.max(DENSITY.least, v)) : 0;
  } catch {
    return 0;
  }
}

/** A row of the grid: a heading, or tiles `start` to `end` (exclusive). */
type Row = { heading: string } | { start: number; end: number };

/** A small white-on-black label over a tile (DESIGN.md's over-image rule). */
const CHIP = "pointer-events-none absolute flex h-6 items-center gap-1 rounded-sm bg-chip px-1.5 text-label text-white";

/**
 * A virtualised grid of square tiles, for either library: only the rows on
 * screen exist. A tile whose image fails stays empty. With `sectionAt`, the
 * tiles fall under headings (the month they were taken), as on the desktop.
 * Pinch to fit more or fewer across; the photo under the fingers stays put.
 * A `header` (the screen's large title) scrolls away above the tiles.
 * With `onRefresh`, pulling the grid down from its top syncs, as Mail and
 * Photos do; `refreshing` keeps saying so until it's done.
 */
export default function Grid({
  width,
  count,
  url,
  verdictAt,
  selected,
  onOpen,
  meter,
  markAt,
  chosenAt,
  ratingAt,
  editedAt,
  sectionAt,
  sectionLabel,
  header,
  onHeaderHidden,
  onLongPress,
  onRefresh,
  refreshing = false,
  toTop = 0,
}: {
  width: number;
  count: number;
  url: (i: number) => string;
  verdictAt: (i: number) => Verdict | null;
  selected: number | null;
  onOpen: (i: number) => void;
  meter: boolean;
  /** A word in the tile's corner, such as Edited. */
  markAt?: (i: number) => string | null;
  /** Tiles chosen for an action, while choosing. */
  chosenAt?: (i: number) => boolean;
  /** Stars in the tile's corner, 0 for none. */
  ratingAt?: (i: number) => number;
  /** Shows a pencil: the photo has edits. */
  editedAt?: (i: number) => boolean;
  /** Which heading tile `i` falls under, as a cheap key; a run of tiles with
   *  one key shares a heading. `sectionLabel` words a key. */
  sectionAt?: (i: number) => string;
  sectionLabel?: (key: string) => string;
  /** Above the tiles, scrolling with them. */
  header?: ReactNode;
  /** Told when the header has scrolled out of sight, and back. */
  onHeaderHidden?: (hidden: boolean) => void;
  /** A long press on a tile. */
  onLongPress?: (i: number) => void;
  /** Pulled down from the top: sync. */
  onRefresh?: () => void;
  refreshing?: boolean;
  /** Bumped to scroll back to the top. */
  toTop?: number;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (toTop > 0) scroller.current?.scrollTo({ top: 0, behavior: "smooth" });
  }, [toTop]);
  // Pulling down from the top: where the finger started, and how far the
  // pull has come, at half the finger's travel.
  const pullFrom = useRef<number | null>(null);
  const [pull, setPull] = useState(0);
  const pullStart = (e: React.TouchEvent) => {
    const el = scroller.current;
    pullFrom.current = onRefresh && el && e.touches.length === 1 && el.scrollTop <= 0 ? e.touches[0]!.clientY : null;
  };
  const pullMove = (e: React.TouchEvent) => {
    const el = scroller.current;
    if (pullFrom.current === null || !el || e.touches.length !== 1) return;
    const dy = e.touches[0]!.clientY - pullFrom.current;
    if (el.scrollTop > 0 && dy <= 0) {
      pullFrom.current = null;
      return setPull(0);
    }
    setPull(Math.min(PULL.most, Math.max(0, dy * 0.5)));
  };
  const pullEnd = () => {
    if (pull >= PULL.at) onRefresh?.();
    pullFrom.current = null;
    setPull(0);
  };
  // The content follows the pull down, and leaves room for the word above
  // it while the sync runs.
  const lowered = refreshing ? 48 : pull;
  const top = useRef<HTMLDivElement>(null);
  const hasHeader = header !== undefined;
  // The header's height: the tiles start below it.
  const [above, setAbove] = useState(0);
  useEffect(() => {
    const el = top.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setAbove(el.offsetHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, [hasHeader]);
  const hidden = useRef(onHeaderHidden);
  hidden.current = onHeaderHidden;
  useEffect(() => {
    const el = top.current;
    const root = scroller.current;
    if (!el || !root) return;
    const observer = new IntersectionObserver(([entry]) => hidden.current?.(!entry!.isIntersecting), { root });
    observer.observe(el);
    return () => {
      observer.disconnect();
      hidden.current?.(false);
    };
  }, [hasHeader]);
  // Laid out from the scroller's own width, which a scrollbar or a border
  // can make narrower than the space it was given.
  const [inner, setInner] = useState(width);
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setInner(el.clientWidth));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  const [density, setDensity] = useState(savedDensity);
  useEffect(() => {
    try {
      localStorage.setItem(DENSITY_KEY, String(density));
    } catch {
      // Kept for this run only.
    }
  }, [density]);
  const fit = Math.max(3, Math.floor(inner / MIN_TILE));
  const cols = fit + density;
  const tile = inner / cols;
  const rows = useMemo(() => {
    const out: Row[] = [];
    let i = 0;
    while (i < count) {
      let end = count;
      if (sectionAt) {
        const key = sectionAt(i);
        out.push({ heading: key });
        end = i + 1;
        while (end < count && sectionAt(end) === key) end++;
      }
      for (let s = i; s < end; s += cols) out.push({ start: s, end: Math.min(s + cols, end) });
      i = end;
    }
    return out;
  }, [count, cols, sectionAt]);
  const size = useCallback((r: number) => ("heading" in rows[r]! ? HEADING : tile), [rows, tile]);
  // The virtualiser keeps the sizes it has worked out until the item keys
  // change, so they change with the layout: the rows are then placed afresh
  // in the same render, with nothing stale for a frame.
  const itemKey = useCallback((r: number) => `${tile}:${r}`, [rows, tile]);
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: size,
    getItemKey: itemKey,
    scrollMargin: above,
    overscan: 4,
  });

  // A pinch scales the grid as the fingers move and lays it out afresh when
  // they lift. Laying it out under the fingers would swap the tiles they
  // touched for new ones, and the webview stops reporting a touch whose
  // tile has gone. `pinch` holds how far apart the fingers started, and the
  // photo between them with how far down the screen it was, and whether the
  // fingers have lifted; `squeeze` the scale, and where in the grid it is
  // about.
  const pinch = useRef<{ spread: number; index: number; y: number; scale: number; done: boolean } | null>(null);
  const [squeeze, setSqueeze] = useState<{ scale: number; x: number; y: number } | null>(null);
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const spread = (t: TouchList) => Math.hypot(t[0]!.clientX - t[1]!.clientX, t[0]!.clientY - t[1]!.clientY);
    const middle = (t: TouchList) => {
      const rect = el.getBoundingClientRect();
      return { x: (t[0]!.clientX + t[1]!.clientX) / 2 - rect.left, y: (t[0]!.clientY + t[1]!.clientY) / 2 - rect.top };
    };
    // The tile at `x`, `y` down the whole grid; for a heading, the first
    // tile under it.
    const tileAt = (x: number, y: number) => {
      let top = 0;
      for (let k = 0; k < rows.length; k++) {
        const r = rows[k]!;
        top += size(k);
        if (top <= y) continue;
        if ("heading" in r) return k + 1 < rows.length ? (rows[k + 1] as { start: number }).start : null;
        return Math.min(r.end - 1, r.start + Math.floor(x / tile));
      }
      return null;
    };
    const onStart = (e: TouchEvent) => {
      if (e.touches.length !== 2) return;
      const m = middle(e.touches);
      const index = tileAt(m.x, el.scrollTop + m.y - above);
      if (index === null) return;
      pinch.current = { spread: spread(e.touches), index, y: m.y, scale: 1, done: false };
      setSqueeze({ scale: 1, x: m.x, y: el.scrollTop + m.y - above });
    };
    const onMove = (e: TouchEvent) => {
      const p = pinch.current;
      if (!p || e.touches.length !== 2) return;
      if (e.cancelable) e.preventDefault();
      // No further than the most and fewest tiles across.
      p.scale = Math.min(cols / (fit + DENSITY.least), Math.max(cols / (fit + DENSITY.most), spread(e.touches) / p.spread));
      setSqueeze((s) => s && { ...s, scale: p.scale });
    };
    const onEnd = (e: TouchEvent) => {
      const p = pinch.current;
      if (!p || e.touches.length >= 2) return;
      setSqueeze(null);
      const next = Math.min(DENSITY.most, Math.max(DENSITY.least, Math.round(cols / p.scale) - fit));
      if (next === density) pinch.current = null;
      else {
        p.done = true;
        setDensity(next);
      }
    };
    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: false });
    el.addEventListener("touchend", onEnd);
    el.addEventListener("touchcancel", onEnd);
    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
      el.removeEventListener("touchcancel", onEnd);
    };
  }, [rows, size, tile, cols, fit, density, above]);
  // Once laid out afresh, scroll so the photo that was between the fingers
  // is still there.
  useLayoutEffect(() => {
    const p = pinch.current;
    const el = scroller.current;
    if (!p?.done || !el) return;
    pinch.current = null;
    let y = 0;
    for (let k = 0; k < rows.length; k++) {
      const r = rows[k]!;
      if ("start" in r && p.index >= r.start && p.index < r.end) {
        el.scrollTop = above + y + tile / 2 - p.y;
        return;
      }
      y += size(k);
    }
  }, [rows, size, tile, above]);

  return (
    <div className="relative min-w-0 shrink-0" style={{ width }}>
      {(pull > 8 || refreshing) && (
        <div
          role="status"
          className="pointer-events-none absolute inset-x-0 top-2 z-10 flex justify-center"
          style={{ opacity: refreshing ? 1 : Math.min(1, pull / PULL.at) }}
        >
          <span className="flex items-center gap-2 rounded-md bg-raised px-3 py-1.5 text-caption text-ink shadow-2xl">
            <span
              className={`flex ${refreshing ? "spin" : ""}`}
              style={refreshing ? undefined : { transform: `rotate(${(pull / PULL.at) * 270}deg)` }}
            >
              <Icon name="sync" size={16} />
            </span>
            {refreshing ? t("mobile.sync.syncing") : pull >= PULL.at ? t("mobile.sync.release") : t("mobile.sync.pull")}
          </span>
        </div>
      )}
      {/* pan-y: the webview scrolls the grid but leaves a pinch to it. */}
      <div
        ref={scroller}
        className="h-full touch-pan-y overflow-x-hidden overflow-y-auto overscroll-contain pb-[var(--grid-bottom,var(--safe-bottom))]"
        onTouchStart={pullStart}
        onTouchMove={pullMove}
        onTouchEnd={pullEnd}
        onTouchCancel={pullEnd}
      >
        <div
          style={{
            transform: lowered > 0 ? `translateY(${lowered}px)` : undefined,
            transition: pullFrom.current !== null ? "none" : "transform 200ms ease-out",
          }}
        >
        {hasHeader && <div ref={top}>{header}</div>}
        <div
          style={{
            height: virtual.getTotalSize(),
            position: "relative",
            ...(squeeze && { transform: `scale(${squeeze.scale})`, transformOrigin: `${squeeze.x}px ${squeeze.y}px` }),
          }}
        >
          {virtual.getVirtualItems().map((row) => {
            const r = rows[row.index]!;
            if ("heading" in r)
              return (
                <div
                  key={row.key}
                  className="absolute left-0 flex items-end px-3 pb-2 font-display text-title text-ink"
                  style={{ top: row.start - above, height: HEADING, width: inner }}
                >
                  {sectionLabel ? sectionLabel(r.heading) : r.heading}
                </div>
              );
            return (
            <div key={row.key} className="absolute left-0 flex" style={{ top: row.start - above, height: tile, width: inner }}>
              {Array.from({ length: r.end - r.start }, (_, c) => {
                const i = r.start + c;
                const v = verdictAt(i);
                const stars = ratingAt?.(i) ?? 0;
                return (
                  <button
                    key={i}
                    className={`relative bg-raised p-px ${selected === i ? "outline outline-2 -outline-offset-2 outline-accent" : ""}`}
                    style={{ width: tile, height: tile }}
                    onClick={() => onOpen(i)}
                    // A long press is a context menu to the webview.
                    onContextMenu={(e) => {
                      e.preventDefault();
                      onLongPress?.(i);
                    }}
                  >
                    <img
                      src={url(i)}
                      className={`h-full w-full object-cover ${v === "reject" ? "opacity-35" : ""}`}
                      decoding="async"
                      draggable={false}
                      alt=""
                      // A photo the computer has no thumbnail for yet, or one
                      // not fetched while offline, stays an empty tile.
                      onError={(e) => (e.currentTarget.style.visibility = "hidden")}
                    />
                    {chosenAt?.(i) && (
                      <span className="pointer-events-none absolute inset-0 border-2 border-accent">
                        <span className="absolute right-1 top-1 flex h-6 w-6 items-center justify-center rounded-sm bg-accent text-black">
                          <Icon name="check" size={16} strokeWidth={2.2} />
                        </span>
                      </span>
                    )}
                    {markAt?.(i) && <span className={`${CHIP} bottom-1 left-1`}>{markAt(i)}</span>}
                    {editedAt?.(i) && (
                      <span className={`${CHIP} bottom-1 left-1`} aria-label={t("mobile.edit.mark")}>
                        <Icon name="pencil" size={14} />
                      </span>
                    )}
                    {stars > 0 && (
                      <span className={`${CHIP} bottom-1 right-1 font-mono tabular-nums`} aria-label={t("mobile.loupe.stars", { count: stars })}>
                        <Icon name="star" size={12} filled />
                        {stars}
                      </span>
                    )}
                    {v && (
                      <span className={`${CHIP} right-1 top-1`} aria-label={t(v === "pick" ? "mobile.loupe.pick" : "mobile.loupe.reject")}>
                        <Icon name={v === "pick" ? "check" : "ban"} size={14} />
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
            );
          })}
        </div>
        </div>
      </div>
      {meter && <FpsMeter target={scroller} count={count} />}
    </div>
  );
}

/**
 * Frames per second while the grid scrolls, and the lowest second seen: the
 * spike's pass mark is smooth fling scrolling on a mid-range Android phone.
 * Tap it to reset the lowest.
 */
function FpsMeter({ target, count }: { target: React.RefObject<HTMLDivElement | null>; count: number }) {
  const [fps, setFps] = useState<number | null>(null);
  const [low, setLow] = useState<number | null>(null);

  useEffect(() => {
    const el = target.current;
    if (!el) return;
    let frames = 0;
    let since = performance.now();
    let scrolling = false;
    let raf = 0;
    let idle = 0;
    const tick = (now: number) => {
      frames += 1;
      if (now - since >= 500) {
        const value = Math.round((frames * 1000) / (now - since));
        setFps(value);
        if (scrolling) setLow((l) => (l === null ? value : Math.min(l, value)));
        frames = 0;
        since = now;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    const onScroll = () => {
      scrolling = true;
      window.clearTimeout(idle);
      idle = window.setTimeout(() => (scrolling = false), 300);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      cancelAnimationFrame(raf);
      el.removeEventListener("scroll", onScroll);
    };
  }, [target]);

  return (
    <button
      className="absolute bottom-[calc(var(--safe-bottom)+0.5rem)] left-2 rounded-sm bg-black/70 px-2 py-1 font-mono text-label text-ink"
      onClick={() => setLow(null)}
    >
      {t("mobile.grid.count", { count })}
      {fps !== null && <span className="block">{t("mobile.grid.fps", { fps, low: low ?? "–" })}</span>}
    </button>
  );
}
