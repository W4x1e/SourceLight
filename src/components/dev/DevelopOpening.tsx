import { useEffect, useRef } from "react";
import { t } from "../../i18n";
import { Icon } from "../../lib/icons";
import { resolveAction } from "../../lib/keymap";
import { isTextEntry, overlayOpen } from "../../lib/useEscape";
import { useWindowWidth } from "../../lib/useWindowWidth";
import { useStore } from "../../store";
import { Spinner, useShownAfter } from "./AiMaskProgress";
import Filmstrip from "./Filmstrip";
import { railFit, savedRailWidth } from "./railWidth";

/**
 * Develop while its first photo opens: decoding a raw takes a second or more
 * (several in a debug build), and Develop used to appear only once it had,
 * with nothing on screen in the meantime to say D had been pressed.
 *
 * Laid out as Develop is — the header with the way back and the file's name,
 * the canvas, the view bar, the filmstrip, the rail — so nothing moves when
 * the photo arrives, and saying "Opening …" on the canvas as stepping to
 * another photo in Develop already does. Escape, or Library, goes back; the
 * arrows step on, as they will in Develop.
 */
export default function DevelopOpening() {
  const name = useStore((s) => {
    const id = s.developWanted;
    return (s.searchResults ?? s.photos).find((p) => p.id === id)?.filename ?? t("store.develop.thisPhoto");
  });
  const close = useStore((s) => s.closeDevelop);
  const railW = railFit(savedRailWidth(), useWindowWidth(), false);
  // A fast open is over before a chip could be read: said only once it isn't.
  const slow = useShownAfter(true, 250);

  // The keyboard comes here, as it will to Develop: left on the grid cell
  // that opened it, the arrows moved the library's selection underneath.
  const shellRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    shellRef.current?.focus({ preventScroll: true });
    return () => {
      if (opener && opener !== document.body && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (overlayOpen() || isTextEntry(e.target)) return;
      const s = useStore.getState();
      const action = resolveAction(s.keymap, "develop", e);
      if (action === "close-overlay") void s.closeDevelop();
      else if (action === "prev-photo" || action === "next-photo") {
        // From the photo asked for, as Develop steps.
        const i = s.photos.findIndex((p) => p.id === s.developWanted);
        const next = s.photos[i + (action === "next-photo" ? 1 : -1)];
        if (i >= 0 && next) void s.openDevelop(next.id);
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div ref={shellRef} tabIndex={-1} className="develop-shell fixed inset-0 z-50 flex flex-col bg-base outline-none">
      <header
        className="titlebar grid h-11 shrink-0 grid-cols-[minmax(max-content,1fr)_minmax(0,auto)_minmax(max-content,1fr)] items-center gap-3 border-b border-line bg-panel px-3"
        data-tauri-drag-region="deep"
      >
        <div className="flex min-w-0 items-center gap-1">
          <button
            onClick={() => void close()}
            className="btn-ghost gap-1 px-2 text-muted hover:text-ink"
            title={t("develop.header.backToLibrary")}
          >
            <Icon name="chevron" size={14} className="rotate-90" />
            {t("develop.header.library")}
          </button>
        </div>
        <div className="flex min-w-0 items-baseline justify-center gap-2" data-tauri-drag-region>
          <span className="truncate text-caption font-medium text-ink">{name}</span>
        </div>
        <div />
      </header>

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <main className="develop-canvas relative min-h-0 min-w-0 flex-1 overflow-hidden bg-canvas-void">
            {slow && (
              <div
                role="status"
                className="loupe-chip pointer-events-none absolute left-1/2 top-3 z-20 flex max-w-[80%] -translate-x-1/2 items-center gap-2 px-2.5 py-1 text-label text-white/85"
              >
                <Spinner />
                <span className="truncate">{t("develop.view.opening", { name })}</span>
              </div>
            )}
          </main>
          {/* Where the view bar will be, so the photo lands at its own size. */}
          <div className="min-h-10 shrink-0 border-t border-line bg-panel" />
          <Filmstrip />
        </div>
        <aside style={{ width: railW }} className="shrink-0 border-l border-line bg-panel" />
      </div>
    </div>
  );
}
