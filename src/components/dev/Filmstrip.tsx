import { useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { fileUrl } from "../../lib/api";
import { useStore } from "../../store";

/** A thumb is 3:2 at 72px high, with 6px between them. */
const THUMB_H = 72;
const THUMB_W = 108;
const GAP = 6;

/**
 * The shoot, along the bottom of Develop, so the next frame is one click away.
 *
 * Reads the same filtered photo list the library grid shows and opens the
 * clicked one straight into develop — `openDevelop` swaps the resident photo
 * without leaving the module. The current frame scrolls itself into view when
 * it changes, including when the change came from elsewhere (a keyboard step).
 *
 * Virtualised, like Cull's strip: the filter can hold 20,000 photos,
 * and every one used to be a button with an image in the DOM.
 */
export default function Filmstrip() {
  const photos = useStore((s) => s.photos);
  // The photo asked for, not only the one on screen: a click on a frame
  // lights it at once, while its raw is still being opened (as
  // the arrows already step from it).
  const currentId = useStore((s) => s.developWanted ?? s.develop?.photoId ?? null);
  const openDevelop = useStore((s) => s.openDevelop);

  const stripRef = useRef<HTMLDivElement>(null);
  const strip = useVirtualizer({
    horizontal: true,
    count: photos.length,
    getScrollElement: () => stripRef.current,
    estimateSize: () => THUMB_W + GAP,
    overscan: 8,
  });

  // A virtualised strip has refs only for frames already drawn, so the
  // current one is brought into view by index rather than by element.
  const current = photos.findIndex((p) => p.id === currentId);
  useEffect(() => {
    if (current >= 0) strip.scrollToIndex(current, { align: "center" });
  }, [current, strip]);

  // There's no scrollbar to drag, so a mouse wheel scrolls the strip sideways:
  // a vertical wheel over a sideways list otherwise does nothing. A touchpad's
  // sideways swipe already scrolls it and is left alone. Bound natively, as
  // React's wheel listener is passive and can't stop the page taking it too.
  const hasPhotos = photos.length > 0;
  useEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
      e.preventDefault();
      const unit = e.deltaMode === 1 ? THUMB_W + GAP : e.deltaMode === 2 ? el.clientWidth : 1;
      el.scrollLeft += e.deltaY * unit;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [hasPhotos]);

  if (!hasPhotos) return null;

  return (
    <div
      ref={stripRef}
      className="no-scrollbar flex h-[92px] shrink-0 items-center overflow-x-auto overflow-y-hidden border-t border-line bg-base px-2.5 py-2"
    >
      <div style={{ width: strip.getTotalSize(), height: THUMB_H, position: "relative", flexShrink: 0 }}>
        {strip.getVirtualItems().map((v) => {
          const p = photos[v.index];
          if (!p) return null;
          const active = p.id === currentId;
          return (
            <button
              key={p.id}
              onClick={() => void openDevelop(p.id)}
              // One stop in the Tab order, the frame being edited: every
              // frame was one, so Tab walked the whole shoot — thousands of
              // stops — before it reached the sliders. The arrows step
              // between photos from there, as they do anywhere in Develop.
              tabIndex={active ? 0 : -1}
              aria-current={active ? "true" : undefined}
              title={p.filename}
              className={`absolute top-0 overflow-hidden rounded-sm border-2 transition-opacity ${
                active ? "border-accent opacity-100" : "border-transparent opacity-70 hover:opacity-100"
              }`}
              style={{ left: v.start, width: THUMB_W, height: THUMB_H }}
            >
              {p.thumbPath ? (
                <img
                  src={fileUrl(p.thumbPath)}
                  alt={p.filename}
                  loading="lazy"
                  draggable={false}
                  className="h-full w-full object-cover"
                />
              ) : (
                <div className="grid h-full w-full place-items-center bg-raised text-faint">?</div>
              )}
              {p.rating > 0 && (
                <div className="pointer-events-none absolute bottom-0.5 left-1 text-label text-accent drop-shadow">
                  {"★".repeat(p.rating)}
                </div>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}
