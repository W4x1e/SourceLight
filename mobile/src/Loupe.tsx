import { useEffect, useRef, useState } from "react";
import { t } from "../../src/i18n";
import { Icon } from "../../src/lib/icons";
import { IconButton } from "./ui";

export type Verdict = "pick" | "reject";

/** How far a finger must travel sideways, in px, to turn the page. */
const SWIPE = 60;
/** How far a finger must pull the photo down, in px, to close it. */
const DISMISS = 110;
const MAX_ZOOM = 4;
/** A double tap zooms to this, about where it landed. */
const TAP_ZOOM = 2.5;

type Point = { x: number; y: number };
const ORIGIN: Point = { x: 0, y: 0 };

/**
 * One photo, to judge: swipe for the next, pinch or double-tap to zoom, and
 * Pick or Reject. Full screen on a phone (Back at the top left, or pull the
 * photo down), with the controls beside the photo when the phone is on its
 * side; the right-hand pane on a tablet, with Close (back to no photo) and
 * Expand (full screen) at the top right. Edit sits in the bottom bar with
 * Reject and Pick, as the one key on it. Zooming keeps the spot under the
 * fingers (or the double tap) where it was, so a face can be checked for
 * focus, and the photo can't be pushed off the screen. The thumbnail shows
 * at once; the larger preview fades in over it. Without `onVerdict`, as for
 * the phone's own photos, there is nothing to judge and the buttons are left
 * out.
 */
export default function Loupe({
  index,
  count,
  thumb,
  preview,
  fallback,
  verdict,
  rating,
  onIndex,
  onVerdict,
  onRating,
  onClose,
  pane,
  onEdit,
  actions,
  note,
  title,
  subtitle,
}: {
  index: number;
  count: number;
  thumb: string;
  /** None for the test grid's stand-ins. */
  preview: string | null;
  /** Shown instead if `preview` can't be had. */
  fallback?: string;
  verdict?: Verdict | null;
  /** 0 to 5, with `onRating`. */
  rating?: number;
  onIndex: (i: number) => void;
  onVerdict?: (v: Verdict | null) => void;
  /** Tapping the photo's own rating takes it off. */
  onRating?: (r: number) => void;
  /** Full screen: Back, and pulling the photo down. */
  onClose?: () => void;
  /** Beside the grid on a tablet: back to no photo, or full screen. */
  pane?: { onClose: () => void; onExpand: () => void };
  /** Edit, when this photo can be edited here. */
  onEdit?: () => void;
  /** Buttons for the bottom bar when there is nothing to judge. */
  actions?: React.ReactNode;
  /** A line about this photo above the bottom bar. */
  note?: string | null;
  /** What the photo is, for the top bar: when it was taken, its file name. */
  title?: string | null;
  subtitle?: string | null;
}) {
  // The zoom and where the zoomed photo sits, from its centred place; kept in
  // a ref as well, as pointer events can come faster than renders.
  const [view, setView] = useState({ zoom: 1, pan: ORIGIN });
  const shown = useRef(view);
  const show = (zoom: number, pan: Point) => {
    shown.current = { zoom, pan };
    setView(shown.current);
  };
  // An unzoomed photo following the finger: sideways to turn the page, down
  // to close.
  const [drag, setDrag] = useState(ORIGIN);
  const area = useRef<HTMLDivElement>(null);
  /** The photo's width over its height, once an image of it is in. */
  const aspect = useRef<number | null>(null);
  const pointers = useRef(new Map<number, Point>());
  // A gesture, restarted whenever a finger comes or goes: the view and the
  // fingers' middle and spread when it started, which way an unzoomed drag
  // is going, and how far it has gone.
  const gesture = useRef<{ zoom: number; pan: Point; at: Point; spread: number; axis: "x" | "y" | null; drag: Point } | null>(null);
  const moved = useRef(false);
  const lastTap = useRef(0);
  const [sharp, setSharp] = useState(false);
  const [failed, setFailed] = useState(false);

  // A new photo starts unzoomed, on its thumbnail.
  useEffect(() => {
    setSharp(false);
    setFailed(false);
    aspect.current = null;
    shown.current = { zoom: 1, pan: ORIGIN };
    setView(shown.current);
    setDrag(ORIGIN);
  }, [index]);

  const go = (delta: number) => {
    const next = index + delta;
    if (next >= 0 && next < count) onIndex(next);
  };

  /** A point on screen, from the middle of the photo's area. */
  const local = (p: Point): Point => {
    const r = area.current!.getBoundingClientRect();
    return { x: p.x - r.left - r.width / 2, y: p.y - r.top - r.height / 2 };
  };

  /** `pan` kept to where the photo at `zoom` still covers the area it can:
   *  an edge stops at the area's edge, and a side narrower than the area
   *  stays centred. */
  const within = (zoom: number, pan: Point): Point => {
    const r = area.current?.getBoundingClientRect();
    if (!r) return pan;
    const a = aspect.current ?? r.width / r.height;
    const width = Math.min(r.width, r.height * a);
    const x = Math.max(0, (width * zoom - r.width) / 2);
    const y = Math.max(0, ((width / a) * zoom - r.height) / 2);
    return { x: Math.min(x, Math.max(-x, pan.x)), y: Math.min(y, Math.max(-y, pan.y)) };
  };

  /** `zoom`, with the photo's spot at `at` (from the middle) staying there,
   *  starting from `from`. */
  const zoomAbout = (zoom: number, at: Point, from: { zoom: number; pan: Point }, to: Point = at) => {
    const spot = { x: (at.x - from.pan.x) / from.zoom, y: (at.y - from.pan.y) / from.zoom };
    show(zoom, within(zoom, { x: to.x - zoom * spot.x, y: to.y - zoom * spot.y }));
  };

  const fingers = () => {
    const [a, b] = [...pointers.current.values()];
    if (!a) return { at: ORIGIN, spread: 0 };
    if (!b) return { at: local(a), spread: 0 };
    return { at: local({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }), spread: Math.hypot(a.x - b.x, a.y - b.y) };
  };

  const begin = () => {
    const { at, spread } = fingers();
    gesture.current = { ...shown.current, at, spread, axis: null, drag: ORIGIN };
  };

  const onDown = (e: React.PointerEvent) => {
    (e.target as Element).setPointerCapture(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size === 1) moved.current = false;
    begin();
  };

  const onMove = (e: React.PointerEvent) => {
    const g = gesture.current;
    if (!pointers.current.has(e.pointerId) || !g) return;
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const { at, spread } = fingers();
    const dx = at.x - g.at.x;
    const dy = at.y - g.at.y;
    if (Math.hypot(dx, dy) > 10) moved.current = true;
    if (pointers.current.size === 2 && g.spread > 0) {
      // Pinch, and move both fingers to pan as it zooms.
      moved.current = true;
      zoomAbout(Math.min(MAX_ZOOM, Math.max(1, (g.zoom * spread) / g.spread)), g.at, g, at);
    } else if (pointers.current.size === 1) {
      if (shown.current.zoom > 1) show(shown.current.zoom, within(shown.current.zoom, { x: g.pan.x + dx, y: g.pan.y + dy }));
      else {
        if (!g.axis && moved.current) g.axis = Math.abs(dx) >= Math.abs(dy) || !onClose ? "x" : "y";
        g.drag = g.axis === "x" ? { x: dx, y: 0 } : g.axis === "y" ? { x: 0, y: Math.max(0, dy) } : ORIGIN;
        setDrag(g.drag);
      }
    }
  };

  const onUp = (e: React.PointerEvent) => {
    const g = gesture.current;
    pointers.current.delete(e.pointerId);
    // A finger still down carries on from where things are.
    if (pointers.current.size > 0) return begin();
    if (!g) return;
    gesture.current = null;
    setDrag(ORIGIN);
    const { zoom } = shown.current;
    if (zoom <= 1.02) {
      show(1, ORIGIN);
      if (g.axis === "x" && Math.abs(g.drag.x) > SWIPE) go(g.drag.x < 0 ? 1 : -1);
      if (g.axis === "y" && g.drag.y > DISMISS) onClose?.();
    }
    // Double tap: zoom in there, or back to fit.
    if (!moved.current) {
      const now = performance.now();
      if (now - lastTap.current < 300) {
        lastTap.current = 0;
        if (zoom > 1) show(1, ORIGIN);
        else zoomAbout(TAP_ZOOM, local({ x: e.clientX, y: e.clientY }), shown.current);
      } else lastTap.current = now;
    }
  };

  const fit = (e: React.SyntheticEvent<HTMLImageElement>) => {
    const img = e.currentTarget;
    if (img.naturalWidth && img.naturalHeight) aspect.current = img.naturalWidth / img.naturalHeight;
  };
  // Pulled down, the photo shrinks a little as it goes.
  const shrink = Math.max(0.7, 1 - drag.y / 1200);

  const toggle = (v: Verdict) => onVerdict?.(verdict === v ? null : v);

  return (
    <div className="flex h-full flex-col bg-black short:flex-row short:pt-[var(--safe-top)]">
      <div className="contents short:order-2 short:flex short:w-80 short:shrink-0 short:flex-col short:overflow-y-auto">
        <div className="order-1 flex min-h-14 items-center gap-1 px-1 pt-[var(--safe-top)] short:pt-0">
          {onClose ? <IconButton icon="back" label={t("common.back")} onClick={onClose} /> : <span className="w-3" />}
          <div className="min-w-0 flex-1 px-1">
            {title && <div className="truncate text-body text-ink">{title}</div>}
            <div className="truncate font-mono text-label tabular-nums text-faint">
              {t("mobile.loupe.position", { index: index + 1, count })}
              {subtitle ? ` · ${subtitle}` : ""}
            </div>
          </div>
          {pane && (
            <>
              <IconButton icon="expand" label={t("mobile.loupe.expand")} onClick={pane.onExpand} />
              <IconButton icon="close" label={t("mobile.loupe.close")} onClick={pane.onClose} />
            </>
          )}
        </div>
        {/* The controls sit at the foot of the side column. */}
        <div className="order-3 hidden short:block short:flex-1" />
        {note && <p className="order-3 px-4 pt-3 text-center text-label text-muted">{note}</p>}
        {!onVerdict &&
          (actions ? (
            <div className={`order-3 flex gap-2 ${BAR}`}>{actions}</div>
          ) : (
            <div className="order-3 pb-[var(--safe-bottom)]" />
          ))}
        {onRating && (
          <div className="order-3 flex justify-center pt-2" role="group" aria-label={t("mobile.loupe.rating")}>
            {[1, 2, 3, 4, 5].map((n) => (
              <button
                key={n}
                className={`flex min-h-12 min-w-12 items-center justify-center ${n <= (rating ?? 0) ? "text-ink" : "text-dim"}`}
                aria-label={t("mobile.loupe.stars", { count: n })}
                aria-pressed={n === rating}
                onClick={() => onRating(n === rating ? 0 : n)}
              >
                <Icon name="star" size={28} filled={n <= (rating ?? 0)} />
              </button>
            ))}
          </div>
        )}
        {onVerdict && (
          <div className={`order-3 flex gap-2 ${BAR}`}>
            {/* The verdicts in the desktop's colours: red for a reject, green
                for a pick, each as a tint with its border, never a solid fill. */}
            <button
              className={`flex min-h-12 flex-1 items-center justify-center gap-2 rounded-md border text-body ${
                verdict === "reject" ? "border-danger/40 bg-danger/10 text-danger-ink" : "border-transparent bg-raised text-ink"
              }`}
              aria-pressed={verdict === "reject"}
              onClick={() => toggle("reject")}
            >
              <Icon name="ban" size={20} />
              {t("mobile.loupe.reject")}
            </button>
            <button
              className={`flex min-h-12 flex-1 items-center justify-center gap-2 rounded-md border text-body ${
                verdict === "pick" ? "border-positive/40 bg-positive/10 text-positive" : "border-transparent bg-raised text-ink"
              }`}
              aria-pressed={verdict === "pick"}
              onClick={() => toggle("pick")}
            >
              <Icon name="check" size={20} />
              {t("mobile.loupe.pick")}
            </button>
            {onEdit && <EditButton onClick={onEdit} />}
          </div>
        )}
      </div>
      <div
        ref={area}
        className="relative order-2 min-h-0 flex-1 touch-none overflow-hidden short:order-1"
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
      >
        <div
          className={`absolute inset-0 ${verdict === "reject" ? "opacity-40" : ""}`}
          style={{
            transform: `translate(${view.pan.x + drag.x}px, ${view.pan.y + drag.y}px) scale(${view.zoom * shrink})`,
            transition: gesture.current ? "none" : "transform 160ms ease-out",
          }}
        >
          <img
            src={thumb}
            alt=""
            draggable={false}
            onLoad={fit}
            className="absolute inset-0 h-full w-full select-none object-contain"
          />
          {preview && (
            <img
              key={failed && fallback ? fallback : preview}
              src={failed && fallback ? fallback : preview}
              alt=""
              draggable={false}
              onLoad={(e) => {
                fit(e);
                setSharp(true);
              }}
              onError={() => setFailed(true)}
              className={`absolute inset-0 h-full w-full select-none object-contain transition-opacity duration-150 ${sharp ? "opacity-100" : "opacity-0"}`}
            />
          )}
        </div>
      </div>
    </div>
  );
}

/** Edit, as the bottom bar's one key: the sliders, the usual sign for
 *  editing a photo, rather than a pencil (which reads as rename or markup). */
/** The bar of buttons under the photo: across a phone, but no wider than
 *  a phone's on a tablet's wide pane. */
const BAR = "mx-auto w-full max-w-xl px-4 pt-3 pb-[calc(var(--safe-bottom)+0.75rem)]";

export function EditButton({ onClick }: { onClick: () => void }) {
  return (
    <button className="btn-accent min-h-12 flex-1 gap-2 text-body" onClick={onClick}>
      <Icon name="adjust" size={20} />
      {t("mobile.edit.open")}
    </button>
  );
}
