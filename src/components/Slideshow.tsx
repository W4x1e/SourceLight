import { useCallback, useEffect, useRef, useState } from "react";
import { formatNumber, t } from "../i18n";
import { useStore } from "../store";
import { fileUrl, fullUrl } from "../lib/api";
import { Icon } from "../lib/icons";
import { resolveAction } from "../lib/keymap";
import { useFocusTrap } from "../lib/useFocusTrap";
import { isFullscreen, setFullscreen } from "../lib/fullscreen";
import { SLIDE_CAPTIONS, captionFor, stepFrom, wantsFull } from "../lib/slideshow";
import type { Photo } from "../types";

/** How long the controls stay after the pointer stops moving. */
const IDLE_MS = 2500;
/** Decoded photos kept ready: the one on show, and the next two each way. */
const KEEP = 5;

type Loaded = { id: number; src: string };

/**
 * A full-screen slideshow of the selection, a collection or the view.
 *
 * Two layers crossfade. The next photo is fetched and *decoded* before the
 * fade starts, so a slow file holds the current one on screen rather than
 * fading into nothing; the next two are fetched ahead. On a screen larger
 * than the cached preview, each photo's full resolution is used, falling back
 * to the preview when a file has none bigger.
 *
 * It owns the keyboard while it plays: Space pauses, ← → step, Esc ends, and
 * the rating, flag and colour-label keys work — a slideshow is often a last
 * look over the picks.
 */
export default function Slideshow() {
  const show = useStore((s) => s.slideshow);
  const options = useStore((s) => s.slideshowOptions);
  const setOptions = useStore((s) => s.setSlideshowOptions);
  const close = useStore((s) => s.closeSlideshow);
  const photos = show?.photos ?? [];

  const [index, setIndex] = useState(0);
  const [paused, setPaused] = useState(false);
  // Two layers, and which is in front: swapped in one update, so the one
  // fading in and the one fading out never disagree.
  const [stage, setStage] = useState<{ layers: [Loaded | null, Loaded | null]; front: 0 | 1 }>({
    layers: [null, null],
    front: 0,
  });
  const { layers, front } = stage;
  const [chrome, setChrome] = useState(true);
  const [settings, setSettings] = useState(false);
  const [ended, setEnded] = useState(false);

  // Photos fetched and decoded, by id, so stepping back is instant too.
  const ready = useRef(new Map<number, Promise<string>>());
  const full = typeof window !== "undefined" && wantsFull(screen.width, screen.height, devicePixelRatio || 1);

  const load = useCallback(
    (p: Photo): Promise<string> => {
      const hit = ready.current.get(p.id);
      if (hit) return hit;
      const preview = fileUrl(p.previewPath ?? p.thumbPath) ?? "";
      const decode = (src: string) => {
        const img = new Image();
        img.src = src;
        return img.decode().then(() => src);
      };
      // A 204 (nothing bigger than the preview) fails to decode: the preview it is.
      const got = (full ? decode(fullUrl(p.id)).catch(() => decode(preview)) : decode(preview)).catch(() => preview);
      ready.current.set(p.id, got);
      return got;
    },
    [full],
  );

  // A new slideshow starts at its first photo, full screen.
  const wasFull = useRef<boolean | null>(null);
  useEffect(() => {
    if (!show) return;
    setIndex(0);
    setPaused(false);
    setEnded(false);
    setStage({ layers: [null, null], front: 0 });
    ready.current.clear();
    void isFullscreen().then((was) => {
      wasFull.current = was;
      if (!was) void setFullscreen(true);
    });
    return () => {
      if (wasFull.current === false) void setFullscreen(false);
      wasFull.current = null;
    };
    // A new list of photos is a new slideshow; a verdict patch is not.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show === null]);

  // Show `index`: once decoded, into the back layer, which then fades in.
  const current = photos[index];
  useEffect(() => {
    if (!current) return;
    let live = true;
    void load(current).then((src) => {
      if (!live) return;
      setStage((st) => {
        if (st.layers[st.front]?.id === current.id) return st;
        const back = st.front === 0 ? 1 : 0;
        const next: [Loaded | null, Loaded | null] = [...st.layers];
        next[back] = { id: current.id, src };
        return { layers: next, front: back };
      });
    });
    // Fetch ahead, and let go of what is far behind.
    for (const d of [1, 2, -1]) {
      const j = stepFrom(index, photos.length, d > 0 ? 1 : -1, options.loop);
      const k = j === null ? null : d === 2 ? stepFrom(j, photos.length, 1, options.loop) : j;
      if (k !== null && photos[k]) void load(photos[k]);
    }
    if (ready.current.size > KEEP * 3) {
      const near = new Set([-2, -1, 0, 1, 2].map((d) => photos[(index + d + photos.length) % photos.length]?.id));
      for (const id of ready.current.keys()) if (!near.has(id)) ready.current.delete(id);
    }
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current?.id]);

  const step = useCallback(
    (dir: 1 | -1) => {
      const j = stepFrom(index, photos.length, dir, options.loop);
      if (j === null) {
        if (dir === 1) {
          setPaused(true);
          setEnded(true);
        }
        return;
      }
      setEnded(false);
      setIndex(j);
    },
    [index, photos.length, options.loop],
  );

  // The clock: each photo stays `seconds` once it is on screen.
  const onScreen = layers[front]?.id === current?.id;
  useEffect(() => {
    if (paused || !onScreen || photos.length < 2) return;
    const t = window.setTimeout(() => step(1), options.seconds * 1000);
    return () => window.clearTimeout(t);
  }, [paused, onScreen, step, options.seconds, photos.length]);

  // The controls show while the pointer moves, and when paused.
  const idle = useRef<number | undefined>(undefined);
  const wake = () => {
    setChrome(true);
    window.clearTimeout(idle.current);
    idle.current = window.setTimeout(() => setChrome(false), IDLE_MS);
  };
  useEffect(() => {
    if (!show) return;
    wake();
    return () => window.clearTimeout(idle.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show === null]);

  // Tab among its own controls only, and focus back where it was after.
  // Before the key handler below, which stops every key it sees.
  const rootRef = useRef<HTMLDivElement>(null);
  useFocusTrap(show !== null, rootRef);

  // It owns the keyboard: caught first, and nothing behind it hears a key.
  const keys = useRef({ step, current, paused });
  keys.current = { step, current, paused };
  useEffect(() => {
    if (!show) return;
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "SELECT")) return;
      e.stopImmediatePropagation();
      const k = keys.current;
      if (e.key === " ") {
        e.preventDefault();
        setPaused(!k.paused);
        setEnded(false);
        wake();
        return;
      }
      const s = useStore.getState();
      const action = resolveAction(s.keymap, "lightbox", e);
      if (!action) return;
      e.preventDefault();
      const id = k.current?.id;
      if (action === "close-overlay") s.closeSlideshow();
      else if (action === "next-photo") k.step(1);
      else if (action === "prev-photo") k.step(-1);
      else if (id === undefined) return;
      else if (action.startsWith("rate-")) void s.compareApply([id], { rating: Number(action.slice(5)) });
      else if (action === "flag-pick") void s.compareApply([id], { flag: 1 });
      else if (action === "flag-reject") void s.compareApply([id], { flag: -1 });
      else if (action === "flag-none") void s.compareApply([id], { flag: 0 });
      else if (action === "label-none") void s.compareApply([id], { colorLabel: 0 });
      else if (action.startsWith("label-")) void s.compareApply([id], { colorLabel: Number(action.slice(6)) });
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show === null]);

  if (!show || !current) return null;
  const caption = captionFor(current, options.caption);
  const fade = `opacity ${options.fade}s ease-in-out`;
  const button = "grid h-9 w-9 place-items-center rounded-sm text-white/80 hover:bg-white/10 hover:text-white";

  return (
    <div
      ref={rootRef}
      tabIndex={-1}
      role="dialog"
      aria-label={t("viewer.slideshow.title")}
      className="anim-view fixed inset-0 z-[70] bg-black outline-none"
      style={{ cursor: chrome ? "default" : "none" }}
      onPointerMove={wake}
    >
      {/* Both always mounted, so each photo fades in from nothing. */}
      {layers.map((l, i) => (
        <img
          key={i}
          src={l?.src || undefined}
          alt=""
          draggable={false}
          className="absolute inset-0 h-full w-full select-none object-contain"
          style={{ opacity: l && front === i ? 1 : 0, transition: fade }}
        />
      ))}

      {caption && (
        <div className="pointer-events-none absolute bottom-6 left-8 text-body text-white/90 [text-shadow:0_1px_3px_rgba(0,0,0,0.9)]">
          {caption}
        </div>
      )}

      {/* The verdict just given, so a key pressed in the dark is seen to land. */}
      <div className="pointer-events-none absolute right-8 top-6 flex items-center gap-2 text-body text-white/85 [text-shadow:0_1px_3px_rgba(0,0,0,0.9)]">
        {current.flag === 1 && <span>{t("viewer.slideshow.pick")}</span>}
        {current.flag === -1 && <span>{t("viewer.slideshow.reject")}</span>}
        {current.rating > 0 && <span>{"★".repeat(current.rating)}</span>}
      </div>

      <div
        className="absolute inset-x-0 bottom-0 flex flex-col items-center gap-2 pb-5 transition-opacity duration-300"
        style={{ opacity: chrome || paused ? 1 : 0, pointerEvents: chrome || paused ? "auto" : "none" }}
      >
        {settings && (
          <div className="flex flex-wrap items-center gap-3 rounded-md bg-black/80 px-4 py-2.5 text-caption text-white/80 backdrop-blur">
            <label className="flex items-center gap-1.5">
              {t("viewer.slideshow.each")}
              <input
                type="number"
                min={1}
                max={60}
                value={options.seconds}
                onChange={(e) => setOptions({ seconds: Math.min(60, Math.max(1, Number(e.target.value) || 1)) })}
                className="w-12 rounded-sm bg-white/10 px-1.5 py-0.5 text-white outline-none"
              />
              {t("viewer.slideshow.seconds")}
            </label>
            <label className="flex items-center gap-1.5">
              {t("viewer.slideshow.fade")}
              <input
                type="range"
                min={0}
                max={3}
                step={0.1}
                value={options.fade}
                onChange={(e) => setOptions({ fade: Number(e.target.value) })}
                className="w-20"
              />
              {formatNumber(options.fade, { minimumFractionDigits: 1, maximumFractionDigits: 1 })}{" "}
              {t("viewer.slideshow.seconds")}
            </label>
            <label className="flex items-center gap-1.5">
              <input type="checkbox" checked={options.shuffle} onChange={(e) => setOptions({ shuffle: e.target.checked })} />
              {t("viewer.slideshow.shuffle")}
            </label>
            <label className="flex items-center gap-1.5">
              <input type="checkbox" checked={options.loop} onChange={(e) => setOptions({ loop: e.target.checked })} />
              {t("viewer.slideshow.loop")}
            </label>
            <select
              value={options.caption}
              onChange={(e) => setOptions({ caption: e.target.value as typeof options.caption })}
              className="rounded-sm bg-white/10 px-1.5 py-0.5 text-white outline-none"
              aria-label={t("viewer.slideshow.caption")}
            >
              {SLIDE_CAPTIONS.map((c) => (
                <option key={c.id} value={c.id} className="bg-black">
                  {t(c.label)}
                </option>
              ))}
            </select>
          </div>
        )}
        <div className="flex items-center gap-1 rounded-md bg-black/70 px-2 py-1 backdrop-blur">
          <button className={button} onClick={() => step(-1)} aria-label={t("viewer.slideshow.previous")} title={t("viewer.slideshow.previousTitle")}>
            <Icon name="chevron" size={18} className="rotate-90" />
          </button>
          <button
            className={button}
            onClick={() => {
              setPaused(!paused);
              setEnded(false);
            }}
            aria-label={paused ? t("viewer.slideshow.play") : t("viewer.slideshow.pause")}
            title={paused ? t("viewer.slideshow.playTitle") : t("viewer.slideshow.pauseTitle")}
          >
            {paused ? (
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
                <path d="M4 2.5v11l9-5.5z" />
              </svg>
            ) : (
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
                <path d="M4 2.5h3v11H4zM9 2.5h3v11H9z" />
              </svg>
            )}
          </button>
          <button className={button} onClick={() => step(1)} aria-label={t("viewer.slideshow.next")} title={t("viewer.slideshow.nextTitle")}>
            <Icon name="chevron" size={18} className="-rotate-90" />
          </button>
          <span className="px-2 text-caption tabular-nums text-white/70">
            {ended ? `${t("viewer.slideshow.theEnd")} · ` : ""}
            {t("common.progress", { current: index + 1, total: photos.length })}
          </span>
          <button
            className={button}
            onClick={() => setSettings((o) => !o)}
            aria-label={t("viewer.slideshow.options")}
            aria-expanded={settings}
            title={t("viewer.slideshow.optionsTitle")}
          >
            <Icon name="settings" size={16} />
          </button>
          <button className={button} onClick={close} aria-label={t("viewer.slideshow.end")} title={t("viewer.slideshow.endTitle")}>
            <Icon name="close" size={16} />
          </button>
        </div>
      </div>
    </div>
  );
}
