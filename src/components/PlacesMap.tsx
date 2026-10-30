import { useEffect, useMemo, useRef, useState } from "react";
import type { Place } from "../bindings";
import { api, fileUrl } from "../lib/api";
import { Icon } from "../lib/icons";
import { WORLD_PATH } from "../lib/worldMap";
import { formatNumber, t, tx } from "../i18n";
import { localPlaceName } from "../lib/countries";

/**
 * The places on a map — offline: the land is one Natural Earth path in
 * an equirectangular box, so nothing is fetched and nothing leaves the
 * machine. Each place with a position is a dot sized by its photo count, and
 * the view fits itself to where the photos are rather than always showing the
 * whole world. Click a dot to see its photos.
 *
 * Places named from trips — a camera with no GPS — have no position of their
 * own, and used to be listed but never drawn, so a library shot entirely on
 * one had an empty map with nothing to do about it. They now wait in a tray on
 * the map: pick one and click where it is. Any pin can be dragged to move it,
 * and the map zooms (wheel) and pans (drag) so a pin can go down on a town
 * rather than a continent.
 */

/** The smallest span the view zooms to, in degrees — closer and the 1:110m
 *  coastline turns to jagged lines. */
const MIN_SPAN = 24;

/** Pointer travel, in screen pixels, before a press becomes a drag. Below it a
 *  press is a click, so a slightly shaky click on a dot still opens it. */
const DRAG_SLOP = 4;

type View = { x: number; y: number; w: number; h: number };
type Located = Place & { lat: number; lon: number };

/** Keeps a view inside the world and at the map's 2:1 shape. */
export function clampView(v: View): View {
  const w = Math.min(360, Math.max(MIN_SPAN, v.w));
  const h = w / 2;
  return {
    x: Math.min(Math.max(v.x, 0), 360 - w),
    y: Math.min(Math.max(v.y, 0), 180 - h),
    w,
    h,
  };
}

/** A point in map units (equirectangular, 0–360 × 0–180) as a position. */
export function toLatLon(x: number, y: number): { lat: number; lon: number } {
  return {
    lat: Math.min(90, Math.max(-90, 90 - y)),
    lon: Math.min(180, Math.max(-180, x - 180)),
  };
}

/** Fit the view to the dots, with a margin, keeping the 2:1 shape. */
function fitTo(located: Located[]): View {
  if (located.length === 0) return { x: 0, y: 0, w: 360, h: 180 };
  const xs = located.map((p) => p.lon + 180);
  const ys = located.map((p) => 90 - p.lat);
  const [x0, x1] = [Math.min(...xs), Math.max(...xs)];
  const [y0, y1] = [Math.min(...ys), Math.max(...ys)];
  let w = Math.max(MIN_SPAN, (x1 - x0) * 1.4);
  let h = Math.max(MIN_SPAN / 2, (y1 - y0) * 1.4);
  if (w / h > 2) h = w / 2;
  else w = h * 2;
  return clampView({ x: (x0 + x1) / 2 - w / 2, y: (y0 + y1) / 2 - h / 2, w, h });
}

export default function PlacesMap({
  places,
  onPick,
  onPlace,
}: {
  places: Place[];
  onPick: (id: number) => void;
  /** Pins a place at a position, or moves its pin. */
  onPlace: (id: number, lat: number, lon: number) => Promise<void>;
}) {
  const located = useMemo(
    () => places.filter((p): p is Located => p.lat != null && p.lon != null),
    [places],
  );
  const unplaced = useMemo(() => places.filter((p) => p.lat == null || p.lon == null), [places]);
  const [hover, setHover] = useState<number | null>(null);
  // A few photos from each hovered place, fetched the first time it is
  // hovered rather than shipped with every place. Dropped when the places
  // change, since a place's photos may have too.
  const [thumbs, setThumbs] = useState<Record<number, string[]>>({});
  useEffect(() => setThumbs({}), [places]);
  useEffect(() => {
    if (hover == null || hover in thumbs) return;
    const id = hover;
    void api
      .placeThumbs(id)
      .then((paths) => setThumbs((s) => ({ ...s, [id]: paths })))
      .catch(() => setThumbs((s) => ({ ...s, [id]: [] })));
  }, [hover, thumbs]);
  // The place waiting for a click on the map.
  const [placing, setPlacing] = useState<number | null>(null);
  const placingPlace = unplaced.find((p) => p.id === placing);
  useEffect(() => {
    if (placing != null && !placingPlace) setPlacing(null);
  }, [placing, placingPlace]);

  // Fitted to the dots until the user zooms, pans or pins something — then it
  // is theirs, so a new pin does not yank the map out from under the pointer.
  const fit = useMemo(() => fitTo(located), [located]);
  const [own, setOwn] = useState<View | null>(null);
  const view = own ?? fit;

  // A dot being dragged to a new position, drawn there until it is saved.
  const [moving, setMoving] = useState<{ id: number; x: number; y: number } | null>(null);
  const press = useRef<{
    kind: "pan" | "dot";
    id?: number;
    sx: number;
    sy: number;
    view: View;
    dragged: boolean;
  } | null>(null);

  const svg = useRef<SVGSVGElement>(null);

  /** A screen point in map units — the SVG's own transform undoes the
   *  letterboxing `meet` adds. */
  const toMap = (clientX: number, clientY: number) => {
    const el = svg.current;
    const m = el?.getScreenCTM();
    if (!el || !m) return null;
    const p = new DOMPoint(clientX, clientY).matrixTransform(m.inverse());
    return { x: p.x, y: p.y };
  };
  /** Map units per screen pixel at the current view. */
  const unitsPerPx = () => {
    const r = svg.current?.getBoundingClientRect();
    if (!r || r.width === 0 || r.height === 0) return 1;
    return 1 / Math.min(r.width / view.w, r.height / view.h);
  };

  const zoomAt = (factor: number, at?: { x: number; y: number }) =>
    setOwn((cur) => {
      const v = cur ?? fit;
      const w = Math.min(360, Math.max(MIN_SPAN, v.w * factor));
      const k = w / v.w;
      const px = at?.x ?? v.x + v.w / 2;
      const py = at?.y ?? v.y + v.h / 2;
      return clampView({ x: px - (px - v.x) * k, y: py - (py - v.y) * k, w, h: w / 2 });
    });

  // Wheel zoom, around the pointer. A native listener: React's is passive, and
  // the wheel would otherwise also scroll whatever holds the map.
  useEffect(() => {
    const el = svg.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const at = toMap(e.clientX, e.clientY) ?? undefined;
      zoomAt(e.deltaY > 0 ? 1.25 : 0.8, at);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });

  useEffect(() => {
    if (placing == null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setPlacing(null);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [placing]);

  const place = async (id: number, x: number, y: number) => {
    setOwn((cur) => cur ?? fit);
    const { lat, lon } = toLatLon(x, y);
    await onPlace(id, lat, lon);
  };

  const onPointerDown = (e: React.PointerEvent, dot?: number) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
    press.current = {
      kind: dot != null ? "dot" : "pan",
      id: dot,
      sx: e.clientX,
      sy: e.clientY,
      view,
      dragged: false,
    };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const p = press.current;
    if (!p) return;
    const dx = e.clientX - p.sx;
    const dy = e.clientY - p.sy;
    if (!p.dragged && Math.hypot(dx, dy) < DRAG_SLOP) return;
    p.dragged = true;
    if (p.kind === "pan") {
      const u = unitsPerPx();
      setOwn(clampView({ ...p.view, x: p.view.x - dx * u, y: p.view.y - dy * u }));
    } else if (p.id != null) {
      const at = toMap(e.clientX, e.clientY);
      if (at) setMoving({ id: p.id, x: at.x, y: at.y });
    }
  };

  const onPointerUp = (e: React.PointerEvent) => {
    const p = press.current;
    press.current = null;
    if (!p) return;
    if (p.kind === "dot" && p.id != null) {
      if (!p.dragged) {
        onPick(p.id);
        return;
      }
      const at = toMap(e.clientX, e.clientY);
      if (at) void place(p.id, at.x, at.y).finally(() => setMoving(null));
      else setMoving(null);
      return;
    }
    // A click, not a pan, on the map while a place waits for one: pin it.
    if (!p.dragged && placing != null) {
      const at = toMap(e.clientX, e.clientY);
      const id = placing;
      setPlacing(null);
      if (at) void place(id, at.x, at.y);
    }
  };

  const max = Math.max(1, ...located.map((p) => p.count));
  // Dot radius in view units, scaled with the zoom so dots stay a similar size
  // on screen, and by area (sqrt) with the count.
  const radius = (count: number) => (view.w / 360) * (1.2 + 3.2 * Math.sqrt(count / max));

  const hovered = located.find((p) => p.id === hover);
  const dragging = press.current?.dragged ?? false;

  return (
    <div className="relative h-full w-full select-none">
      <svg
        ref={svg}
        viewBox={`${view.x} ${view.y} ${view.w} ${view.h}`}
        preserveAspectRatio="xMidYMid meet"
        className={`h-full w-full touch-none ${placing != null ? "cursor-crosshair" : dragging ? "cursor-grabbing" : "cursor-grab"}`}
        role="img"
        aria-label={t("places.map.ariaLabel", { count: located.length })}
        onPointerDown={(e) => onPointerDown(e)}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => {
          press.current = null;
          setMoving(null);
        }}
      >
        <rect x={0} y={0} width={360} height={180} fill="var(--color-canvas-void)" />
        <path d={WORLD_PATH} fill="var(--color-raised)" stroke="var(--color-line-hover)" strokeWidth={view.w / 1200} />
        {located.map((p) => {
          const m = moving?.id === p.id ? moving : null;
          return (
            <circle
              key={p.id}
              cx={m ? m.x : p.lon + 180}
              cy={m ? m.y : 90 - p.lat}
              r={radius(p.count)}
              fill="var(--color-accent)"
              fillOpacity={hover === p.id || m ? 1 : 0.6}
              stroke="var(--color-base)"
              strokeWidth={view.w / 900}
              className={placing != null ? "pointer-events-none" : "cursor-pointer"}
              onPointerDown={(e) => onPointerDown(e, p.id)}
              onMouseEnter={() => setHover(p.id)}
              onMouseLeave={() => setHover(null)}
            >
              <title>{t("places.map.dotTitle", { name: localPlaceName(p.name, p.country), photos: t("common.photos", { count: p.count }) })}</title>
            </circle>
          );
        })}
      </svg>

      {hovered && placing == null && (
        <div className="pointer-events-none absolute left-3 top-3 max-w-[18rem] overflow-hidden rounded-md bg-chip text-caption text-ink backdrop-blur">
          {(thumbs[hovered.id]?.length ?? 0) > 0 && (
            <div className="flex gap-px">
              {thumbs[hovered.id]!.map((thumb) => (
                <img key={thumb} src={fileUrl(thumb)} alt="" draggable={false} className="h-12 w-[4.5rem] object-cover" />
              ))}
            </div>
          )}
          <div className="px-2 py-1">
            {localPlaceName(hovered.name, hovered.country)} · <span className="font-mono tabular-nums">{formatNumber(hovered.count)}</span>
          </div>
        </div>
      )}

      {placingPlace && (
        <div className="pointer-events-none absolute left-1/2 top-3 -translate-x-1/2 rounded-md bg-chip px-3 py-1.5 text-caption text-ink backdrop-blur">
          {tx("places.map.placingPrompt", {
            name: <span className="font-semibold">{placingPlace.name}</span>,
            cancel: <span className="text-faint">{t("places.map.placingCancel")}</span>,
          })}
        </div>
      )}

      {places.length === 0 && (
        <div className="pointer-events-none absolute inset-0 grid place-items-center p-8">
          <div className="max-w-sm rounded-lg bg-chip p-4 text-center backdrop-blur">
            <div className="text-title text-ink">{t("places.map.emptyTitle")}</div>
            <div className="mt-1 text-caption leading-relaxed text-muted">
              {t("places.map.emptyBody")}
            </div>
          </div>
        </div>
      )}

      {unplaced.length > 0 && (
        <div className="absolute right-3 top-3 flex max-h-[calc(100%-3rem)] w-60 flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl">
          <div className="border-b border-line px-3 py-2">
            <div className="text-caption font-semibold text-ink">{t("places.map.unplacedTitle")}</div>
            <div className="mt-0.5 text-label leading-snug text-faint">
              {t("places.map.unplacedBody")}
            </div>
          </div>
          <div className="min-h-0 overflow-y-auto p-1">
            {unplaced.map((p) => (
              <button
                key={p.id}
                onClick={() => setPlacing(placing === p.id ? null : p.id)}
                className={`facet-row ${placing === p.id ? "facet-row-active" : ""}`}
                title={placing === p.id ? t("common.cancel") : t("places.map.pin", { name: localPlaceName(p.name, p.country) })}
              >
                <Icon name="pin" size={13} className={`shrink-0 ${placing === p.id ? "text-accent" : "text-faint"}`} />
                <span className="truncate">{localPlaceName(p.name, p.country)}</span>
                <span className="ml-auto shrink-0 text-label tabular-nums text-faint">{p.count}</span>
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="absolute bottom-3 left-3 flex flex-col overflow-hidden rounded-md border border-line bg-panel">
        <button className="grid h-7 w-7 place-items-center text-muted hover:bg-hover hover:text-ink" title={t("places.map.zoomIn")} onClick={() => zoomAt(0.8)}>
          <Icon name="plus" size={14} />
        </button>
        <button className="grid h-7 w-7 place-items-center border-t border-line text-muted hover:bg-hover hover:text-ink" title={t("places.map.zoomOut")} onClick={() => zoomAt(1.25)}>
          <Icon name="minus" size={14} />
        </button>
        <button
          className="grid h-7 w-7 place-items-center border-t border-line text-label font-semibold text-muted hover:bg-hover hover:text-ink disabled:opacity-40"
          title={t("places.map.fitHint")}
          disabled={own == null}
          onClick={() => setOwn(null)}
        >
          {t("places.map.fit")}
        </button>
      </div>

      <div className="pointer-events-none absolute bottom-2 right-3 text-label text-faint">
        {t("places.map.credit")}
      </div>
    </div>
  );
}
