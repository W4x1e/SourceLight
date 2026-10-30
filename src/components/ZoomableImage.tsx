import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { t } from "../i18n";
import type { GridKind, PeakColor } from "../lib/composition";
import { IS_MAC } from "../lib/platform";
import { wheelIntent } from "../lib/wheel";
import CompositionGrid from "./CompositionGrid";
import FocusPeaking, { type PeakingStatus } from "./FocusPeaking";

/**
 * A photo you can zoom into and push around.
 *
 * Culling is mostly a question about focus, and a frame scaled to fit answers
 * it badly — every photo looks sharp at 20%. This is the control that lets
 * someone actually decide, and it is shared by the Cull loupe and the Lightbox
 * so the gesture means the same thing in both.
 *
 * **Where the pixels come from.** Fitted, the source starts as the cached
 * 1600px preview. That is plenty for composition on a small window, but on a
 * large or high-DPI screen the browser stretches it, and every frame looks
 * soft, which is the one thing a cull must not suggest. So once a photo has
 * been on screen a moment, its full-resolution pixels are fetched, and
 * swapped in while fitted too if the screen would show more than the preview
 * has. The first zoom on a photo fetches them if that hasn't happened yet.
 * Never on arrival: that fetch is a file read at best and a full sensor
 * decode at worst, and paging through a burst must not pay it per frame.
 * `zoomSourceLabel` tells the person which of the two they are looking at,
 * because a soft-looking preview pixel and a soft sensor pixel mean very
 * different things.
 *
 * Zoom is anchored at the cursor, because zooming to the centre means hunting
 * for the eye you were looking at, every time.
 */

/** How far in the zoom goes at least, and how far past 1:1 it goes beyond that. */
const MAX_SCALE = 4;
const PAST_ONE_TO_ONE = 2;
/** How long a photo stays on screen before its full resolution is fetched ahead of a zoom. */
const PREFETCH_AFTER_MS = 700;

/** Which pixels are on screen. */
export type ZoomSource = "preview" | "loading" | "full";

/** The badge text for a zoomed view. */
export function zoomSourceLabel(source: ZoomSource): string {
  switch (source) {
    case "full":
      return t("viewer.zoom.full");
    case "loading":
      return t("viewer.zoom.loading");
    default:
      return t("viewer.zoom.preview");
  }
}

/**
 * How many device pixels wide `img` would be drawn when fitted into `box`: the
 * resolution at which a source stops looking soft on this screen.
 */
export function fittedDevicePixels(
  box: { clientWidth: number; clientHeight: number } | null,
  img: { naturalWidth: number; naturalHeight: number },
  dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1,
): number {
  if (!box || !img.naturalWidth || !img.naturalHeight) return 0;
  const aspect = img.naturalWidth / img.naturalHeight;
  const width = Math.min(box.clientWidth, box.clientHeight * aspect, img.naturalWidth);
  return width * dpr;
}

/** Scale (1 = fitted) and the pan, in screen pixels from the box centre. */
export interface ZoomView {
  scale: number;
  x: number;
  y: number;
}
type View = ZoomView;

export const FITTED: View = { scale: 1, x: 0, y: 0 };

/**
 * `view` rescaled to `to`, keeping the point `(dx, dy)` from the box centre
 * where it is on screen — the cursor for a wheel or double-click, the centre
 * (0, 0) otherwise. Pure, so React may call it as often as it likes.
 */
export function zoomView(view: View, to: number, dx: number, dy: number): View {
  if (to === view.scale) return view;
  if (to === 1) return FITTED;
  const k = to / view.scale;
  return { scale: to, x: dx - (dx - view.x) * k, y: dy - (dy - view.y) * k };
}

export interface ZoomableImageProps {
  /** Undefined while a photo has neither preview nor thumbnail yet. */
  src: string | undefined;
  alt: string;
  /** Changes reset the view — a new photo starts fitted, never mid-pan. */
  resetKey: string | number;
  /** Reports fit/zoomed so a parent can render a toolbar or a badge. */
  onZoomChange?: (zoomed: boolean) => void;
  /** Full-resolution pixels, fetched on the first zoom. A failed or empty
   *  (204) response keeps the preview. */
  fullSrc?: string;
  /** Reports which pixels are showing, for the badge. */
  onSourceChange?: (source: ZoomSource) => void;
  className?: string;
  /**
   * The view, when a parent holds it — Compare shares one across every frame
   * of a burst, so zooming or panning one does the same to all. Frames
   * of a burst are the same size in same-size tiles, so a scale and a pan in
   * tile pixels land on the same spot of each. Left out, the view is this
   * component's own.
   */
  view?: ZoomView;
  onViewChange?: (view: ZoomView) => void;
  /** Focus peaking over the photo, or none. */
  peaking?: { color: PeakColor; sensitivity: number } | null;
  onPeakingStatus?: (status: PeakingStatus) => void;
  /** A composition grid over the photo. */
  grid?: GridKind;
  /** Keeps the zoom and pan when `resetKey` changes, so checking focus
   *  through a burst is one zoom, not one a frame. */
  keepView?: boolean;
}

export interface ZoomableImageHandle {
  toggle: () => void;
  reset: () => void;
}

export default function ZoomableImage({
  src,
  alt,
  resetKey,
  onZoomChange,
  fullSrc,
  onSourceChange,
  className,
  controlRef,
  view: sharedView,
  onViewChange,
  peaking,
  onPeakingStatus,
  grid = "off",
  keepView = false,
}: ZoomableImageProps & { controlRef?: React.MutableRefObject<ZoomableImageHandle | null> }) {
  // scale 1 means "fitted"; anything above it is a multiple of the fitted size.
  //
  // Scale and pan are one state value, changed by pure updaters. They used to
  // be two, with the offset set from *inside* the scale's updater — impure, so
  // React's development double-invoke applied every zoom's pan twice and the
  // point under the cursor drifted away as you zoomed.
  //
  // Updates are computed here from the latest view rather than inside a React
  // updater — still pure, so nothing is applied twice — because a shared view
  // lives in the parent and has to be *reported* rather than set. `viewRef`
  // moves on the moment a change is made, so wheel steps between two renders
  // build on each other instead of each starting from the same stale view.
  const [ownView, setOwnView] = useState<View>(FITTED);
  const view = sharedView ?? ownView;
  const { scale } = view;
  const offset = { x: view.x, y: view.y };
  // The committed view, for handlers that decide what to do from it.
  const viewRef = useRef(view);
  viewRef.current = view;
  const report = useRef(onViewChange);
  report.current = onViewChange;
  const shared = sharedView !== undefined;
  const sharedRef = useRef(shared);
  sharedRef.current = shared;
  const setView = useCallback((next: View | ((v: View) => View)) => {
    const was = viewRef.current;
    const v = typeof next === "function" ? next(was) : next;
    if (v.scale === was.scale && v.x === was.x && v.y === was.y) return;
    viewRef.current = v;
    report.current?.(v);
    if (!sharedRef.current) setOwnView(v);
  }, []);
  // Where the last zoom was anchored, from the box centre: the full-resolution
  // swap re-zooms about the same point, so what was double-clicked stays put.
  const anchor = useRef({ dx: 0, dy: 0 });
  const boxRef = useRef<HTMLDivElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const dragging = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null);
  // "idle" until the first zoom asks for it; "failed" covers a 204 as well as
  // an error — either way the preview is what there is.
  const [full, setFull] = useState<"idle" | "loading" | "ready" | "failed">("idle");
  // Set when the view was zoomed to exactly 1:1, so the arrival of the full
  // source can move it to the *new* 1:1 rather than leaving it at the old one.
  const atOneToOne = useRef(false);
  // How much the browser shrank the shown image to fit; past this, zooming
  // magnifies its pixels rather than revealing more of them.
  const [nativeRatio, setNativeRatio] = useState(1);

  // Where the image is laid out in the box, before its zoom transform: the
  // overlays sit exactly there and take the same transform, so they stay on
  // the photo through every zoom and pan.
  const [layout, setLayout] = useState<{ left: number; top: number; width: number; height: number } | null>(
    null,
  );
  const overlaid = !!peaking || grid !== "off";
  const measure = useCallback(() => {
    const img = imgRef.current;
    if (!img || !img.offsetWidth) return setLayout(null);
    setLayout((was) => {
      const now = { left: img.offsetLeft, top: img.offsetTop, width: img.offsetWidth, height: img.offsetHeight };
      return was && was.left === now.left && was.top === now.top && was.width === now.width && was.height === now.height
        ? was
        : now;
    });
  }, []);
  useLayoutEffect(() => {
    if (!overlaid) return;
    measure();
    const observer = new ResizeObserver(measure);
    if (imgRef.current) observer.observe(imgRef.current);
    if (boxRef.current) observer.observe(boxRef.current);
    return () => observer.disconnect();
  }, [overlaid, measure]);

  const reset = useCallback(() => {
    setView(FITTED);
    atOneToOne.current = false;
  }, [setView]);

  // A new photo starts fitted — unless asked to keep the view, for stepping
  // through a burst at the same spot. Frames of a burst share their
  // framing, so the same scale and pan land on the same place in each.
  const keepRef = useRef(keepView);
  keepRef.current = keepView;
  useEffect(() => {
    if (!keepRef.current || viewRef.current.scale <= 1) reset();
    setFull("idle");
  }, [resetKey, reset]);

  const zoomed = scale > 1;

  // Fetch the full-resolution source on the first zoom, off-DOM, and swap it
  // in only once it has decoded — so there is never a blank frame mid-swap.
  //
  // Tracked by photo in a ref, not by `full`: an effect depending on the state
  // it sets would re-run on "loading" and its cleanup would cancel the very
  // load it had just started. A load that lands after the photo has changed is
  // ignored — the backend has cached it, which is all it is still good for.
  const requested = useRef<string | number | null>(null);
  useEffect(() => {
    if (!zoomed || !fullSrc || requested.current === resetKey) return;
    const key = resetKey;
    requested.current = key;
    setFull("loading");
    const img = new Image();
    img.onload = () => {
      if (requested.current !== key) return;
      const shown = imgRef.current?.naturalWidth ?? 0;
      setFull(img.naturalWidth > shown ? "ready" : "failed");
    };
    img.onerror = () => {
      if (requested.current === key) setFull("failed");
    };
    img.src = fullSrc;
  }, [zoomed, fullSrc, resetKey]);

  // Fetch the full pixels once a photo has been on screen a moment, so the
  // first zoom does not wait for them. On cameras with a small embedded preview
  // (Sony, Panasonic) they are a sensor decode: a second or two, which is
  // exactly the pause someone zooming in to check focus noticed. Paging quickly
  // through a burst never reaches the timer, so only a photo someone is
  // actually looking at costs a decode.
  //
  // Swapped in while fitted only when the screen needs them: when the box, in
  // device pixels, is wider than the preview. On a laptop panel the preview is
  // already enough, and keeping it avoids a pointless swap.
  const currentKey = useRef(resetKey);
  currentKey.current = resetKey;
  useEffect(() => {
    if (!fullSrc || zoomed) return;
    const key = resetKey;
    const timer = window.setTimeout(() => {
      if (requested.current === key) return;
      const img = new Image();
      img.onload = () => {
        if (currentKey.current !== key || requested.current === key) return;
        const shown = imgRef.current;
        if (!shown || img.naturalWidth <= shown.naturalWidth) return;
        if (shown.naturalWidth >= fittedDevicePixels(boxRef.current, img) * 0.95) return;
        requested.current = key;
        setFull("ready");
      };
      // Marked as a prefetch: one still waiting its turn when the
      // photo has changed is dropped rather than rendered.
      img.src = `${fullSrc}${fullSrc.includes("?") ? "&" : "?"}prefetch=1`;
    }, PREFETCH_AFTER_MS);
    return () => window.clearTimeout(timer);
  }, [fullSrc, resetKey, zoomed]);

  useEffect(() => {
    onSourceChange?.(full === "ready" ? "full" : full === "loading" ? "loading" : "preview");
  }, [full, onSourceChange]);

  useEffect(() => {
    onZoomChange?.(scale > 1);
  }, [scale, onZoomChange]);

  /**
   * The scale at which one image pixel covers one screen pixel.
   *
   * `naturalWidth / clientWidth` is exactly that ratio: how much the browser
   * shrank the image to fit. Computed on demand rather than stored, because it
   * changes whenever the window resizes.
   */
  const ratio = useCallback(() => {
    const img = imgRef.current;
    if (!img || !img.clientWidth) return 2;
    return img.naturalWidth / img.clientWidth;
  }, []);

  /** A full-resolution source goes further than the preview can. */
  const maxScale = useCallback(() => Math.max(MAX_SCALE, ratio() * PAST_ONE_TO_ONE), [ratio]);

  const oneToOneScale = useCallback(
    () => Math.max(1, Math.min(maxScale(), ratio())),
    [maxScale, ratio],
  );

  /**
   * Keeps the point under `client` still while the scale changes around it —
   * or the centre of the view, when there is no pointer (a shortcut, or the
   * full source arriving).
   */
  const zoomAt = useCallback(
    (next: number, clientX?: number, clientY?: number) => {
      const box = boxRef.current;
      const to = Math.max(1, Math.min(maxScale(), next));
      let dx = 0;
      let dy = 0;
      if (box && clientX !== undefined && clientY !== undefined) {
        const r = box.getBoundingClientRect();
        // Distance from the centre, which is what the transform scales about.
        dx = clientX - (r.left + r.width / 2);
        dy = clientY - (r.top + r.height / 2);
      }
      anchor.current = { dx, dy };
      setView((v) => zoomView(v, to, dx, dy));
    },
    [maxScale, setView],
  );

  const toggle = useCallback(() => {
    anchor.current = { dx: 0, dy: 0 };
    if (viewRef.current.scale > 1) {
      atOneToOne.current = false;
      setView(FITTED);
    } else {
      atOneToOne.current = true;
      setView({ scale: oneToOneScale(), x: 0, y: 0 });
    }
  }, [oneToOneScale, setView]);

  /** Runs on every load of the shown image — the preview, then the full one. */
  const onImageLoad = () => {
    setNativeRatio(ratio());
    measure();
    // Someone who asked for 1:1 before the full pixels arrived asked for real
    // 1:1; move them there, about the same point they zoomed in on.
    if (full === "ready" && atOneToOne.current) {
      const to = oneToOneScale();
      const { dx, dy } = anchor.current;
      setView((v) => zoomView(v, to, dx, dy));
    }
  };

  // Let the parent drive the same two actions from a toolbar or a shortcut,
  // rather than duplicating the state up there.
  useEffect(() => {
    if (controlRef) controlRef.current = { toggle, reset };
  }, [controlRef, toggle, reset]);

  // A Mac's trackpad is not a mouse wheel. Its two-finger scroll arrives as a
  // stream of small wheel events, and zooming 15% on each made a scroll over
  // a photo zoom it wildly. So on a Mac, as in Preview and Photos: pinch zooms
  // (WebKit's gesture events, below), ⌘-scroll zooms smoothly — and so does
  // ctrl+wheel, which is how some engines report a pinch — and a plain scroll
  // pans a zoomed photo and leaves a fitted one alone.
  useEffect(() => {
    const box = boxRef.current;
    if (!box || !IS_MAC) return;
    let start = 1;
    type Gesture = Event & { scale: number; clientX: number; clientY: number };
    const onStart = (e: Event) => {
      e.preventDefault();
      start = viewRef.current.scale;
    };
    const onChange = (e: Event) => {
      e.preventDefault();
      const g = e as Gesture;
      atOneToOne.current = false;
      zoomAt(start * g.scale, g.clientX, g.clientY);
    };
    const onEnd = (e: Event) => e.preventDefault();
    box.addEventListener("gesturestart", onStart);
    box.addEventListener("gesturechange", onChange);
    box.addEventListener("gestureend", onEnd);
    return () => {
      box.removeEventListener("gesturestart", onStart);
      box.removeEventListener("gesturechange", onChange);
      box.removeEventListener("gestureend", onEnd);
    };
  }, [zoomAt]);

  const onWheel = (e: React.WheelEvent) => {
    if (IS_MAC) {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        atOneToOne.current = false;
        zoomAt(viewRef.current.scale * Math.exp(-e.deltaY * 0.01), e.clientX, e.clientY);
      } else if (viewRef.current.scale > 1) {
        e.preventDefault();
        setView((v) => ({ ...v, x: v.x - e.deltaX, y: v.y - e.deltaY }));
      }
      return;
    }
    // Windows: a notch, a touchpad scroll and a pinch all arrive as `wheel`
    // (see lib/wheel.ts), and only a notch should zoom a step.
    const intent = wheelIntent(e, scale > 1);
    if (intent.kind === "none") return;
    if (intent.kind === "pan") {
      e.preventDefault();
      setView((v) => ({ ...v, x: v.x - intent.dx, y: v.y - intent.dy }));
      return;
    }
    // Only claim a notch while zoomed or when zooming in: otherwise a scroll
    // over a fitted photo should still reach whatever is behind it.
    if (intent.kind === "step" && scale === 1 && !intent.zoomIn) return;
    e.preventDefault();
    atOneToOne.current = false;
    const factor = intent.kind === "smooth" ? intent.factor : intent.zoomIn ? 1.15 : 1 / 1.15;
    zoomAt(scale * factor, e.clientX, e.clientY);
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (scale === 1) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragging.current = { x: e.clientX, y: e.clientY, ox: offset.x, oy: offset.y };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = dragging.current;
    if (!d) return;
    setView((v) => ({ ...v, x: d.ox + (e.clientX - d.x), y: d.oy + (e.clientY - d.y) }));
  };

  const endDrag = (e: React.PointerEvent) => {
    if (!dragging.current) return;
    dragging.current = null;
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
  };

  const shown = full === "ready" ? fullSrc : src;
  const transform = `translate(${offset.x}px, ${offset.y}px) scale(${scale})`;
  const transition = dragging.current ? "none" : "transform 120ms ease-out";
  const pixelated = scale > nativeRatio + 0.01;
  const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;

  return (
    // `self-stretch` is what makes the image fit. The Cull and Lightbox stages
    // centre their children, so without it this box was only as tall as the
    // image. The image's `max-h-full` was then a percentage of itself and
    // limited nothing: portrait frames, and landscape ones in a window that
    // isn't maximised, ran off the bottom. Stretched, the box takes the
    // stage's height, and the percentage has a real height to resolve against.
    <div
      ref={boxRef}
      className={`relative flex min-h-0 min-w-0 flex-1 items-center justify-center self-stretch overflow-hidden ${className ?? ""}`}
      onWheel={onWheel}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDoubleClick={(e) => {
        // Double-click zooms to where you double-clicked, which is the whole
        // point — you are pointing at the thing you want to inspect.
        if (zoomed) toggle();
        else {
          atOneToOne.current = true;
          zoomAt(oneToOneScale(), e.clientX, e.clientY);
        }
      }}
      style={{ cursor: zoomed ? (dragging.current ? "grabbing" : "grab") : "zoom-in" }}
    >
      <img
        ref={imgRef}
        src={shown}
        alt={alt}
        draggable={false}
        onLoad={onImageLoad}
        className="max-h-full max-w-full select-none object-contain shadow-2xl"
        style={{
          transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})`,
          // Nearest-neighbour once past 1:1, so what is on screen is the
          // source's own pixels rather than the browser's guess at them —
          // smoothing here would make a soft frame look acceptable. Not before
          // 1:1: a full-resolution source is still being *reduced* there, and
          // nearest-neighbour reduction aliases.
          imageRendering: pixelated ? "pixelated" : "auto",
          transition,
        }}
      />
      {overlaid && layout && (
        <div
          className="pointer-events-none absolute"
          style={{
            left: layout.left,
            top: layout.top,
            width: layout.width,
            height: layout.height,
            transform,
            transition,
            imageRendering: pixelated ? "pixelated" : "auto",
          }}
        >
          {peaking && (
            <FocusPeaking
              src={shown}
              color={peaking.color}
              sensitivity={peaking.sensitivity}
              shownWidth={layout.width * dpr * scale}
              onStatus={onPeakingStatus}
            />
          )}
          <CompositionGrid kind={grid} thickness={1 / scale} />
        </div>
      )}
    </div>
  );
}
