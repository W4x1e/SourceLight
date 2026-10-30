import TimelineStrip from "./TimelineStrip";
import ProgressBar from "./ProgressBar";
import { isTextEntry, overlayOpen } from "../lib/useEscape";
import { useEffect } from "react";
import { textIsSelected } from "../lib/copyImage";
import { developShown, useStore } from "../store";
import { resolveAction } from "../lib/keymap";
import { formatNumber, t, tx } from "../i18n";
import TopBar from "./TopBar";
import FilterRail from "./FilterRail";
import ContextHeader from "./ContextHeader";
import PhotoGrid from "./PhotoGrid";
import PersonPickBanner from "./PersonPickBanner";
import SelectionPanel from "./SelectionPanel";

/**
 * The Browse workspace: the filter rail, the query header, the photo grid, the
 * contextual selection panel, and a slim status footer. This is the extraction
 * of what App used to render inline; App is now just the mode router.
 */
export default function BrowseWorkspace() {
  const scanning = useStore((s) => s.scanning);
  const tagging = useStore((s) => s.tagging);
  const selected = useStore((s) => s.selected);

  // The grid's cull shortcuts, resolved through the editable keymap ("library"
  // context). Active only with a selection, focus outside a field, and none of
  // the photo-scoped overlays up (they own their own keys).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      if (developShown(s) || s.compare || s.lightboxIndex !== null) return;
      // Not under a menu, dialog or panel: P/X/1-5 rated the selection
      // behind the batch Export dialog.
      if (overlayOpen()) return;
      if (isTextEntry(e.target)) return;
      const action = resolveAction(s.keymap, "library", e);
      if (!action) return;
      // Undo needs no selection: it takes back whatever was changed last.
      if (action === "undo-verdict") {
        e.preventDefault();
        void s.undoVerdict();
        return;
      }
      if (action === "toggle-filters") {
        e.preventDefault();
        s.setRailCollapsed(!s.railCollapsed);
        return;
      }
      if (action === "select-all") {
        e.preventDefault();
        s.selectPhotos((s.searchResults ?? s.photos).map((p) => p.id));
        return;
      }
      // Only with something to clear: an Escape that does nothing should
      // stay free for whatever else might want it.
      if (action === "clear-selection") {
        if (s.selected.size === 0) return;
        e.preventDefault();
        s.clearSelection();
        return;
      }
      if (s.selected.size === 0) return;
      if (action === "copy-image") {
        // Selected text copies as text; several photos copy the focused one.
        if (textIsSelected()) return;
        const one = s.focusId ?? [...s.selected][0];
        if (one != null) void s.copyImage(one);
      } else if (action.startsWith("rate-")) void s.rateSelection(Number(action.slice(5)));
      else if (action === "flag-pick") void s.flagSelection(1);
      else if (action === "flag-reject") void s.flagSelection(-1);
      else if (action === "flag-none") void s.flagSelection(0);
      else if (action === "label-none") void s.colorLabelSelection(0);
      else if (action.startsWith("label-")) void s.colorLabelSelection(Number(action.slice(6)));
      else if (action === "open-develop") {
        const first = s.focusId ?? [...s.selected][0];
        if (first != null) void s.openDevelop(first);
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <TopBar />
      <div className="flex min-h-0 flex-1">
        <FilterRail />

        <main className="flex min-w-0 flex-1 flex-col">
          <PersonPickBanner />
          <ContextHeader />
          <TimelineStrip />
          <PhotoGrid />

          <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-line bg-panel px-3 text-caption text-faint">
          {/* The count lives in the context header above the grid; saying it
              twice was noise. The footer keeps what the header doesn't:
              the selection, and background progress. */}
          <span>
            {selected.size > 0 ? (
              <>
                {tx("common.selected", { count: <span className="tabular-nums">{formatNumber(selected.size)}</span> }, selected.size)}
              </>
            ) : (
              t("library.browse.nothingSelected")
            )}
          </span>
          {(scanning || tagging) && (
            <div className="ml-auto flex items-center gap-2">
              <span>
                {/* Not "Scanning 0/0" while the folders are still being walked. */}
                {scanning && scanning.total === 0
                  ? t("library.browse.scanningStart")
                  : t(scanning ? "library.browse.scanning" : "library.browse.tagging", {
                      current: (scanning ?? tagging)?.current ?? 0,
                      total: (scanning ?? tagging)?.total ?? 0,
                    })}
              </span>
              <ProgressBar
                pct={
                  scanning && scanning.total === 0
                    ? null
                    : (((scanning ?? tagging)?.current ?? 0) / Math.max(1, (scanning ?? tagging)?.total ?? 1)) * 100
                }
                label={t(scanning ? "library.browse.scanningBar" : "library.browse.taggingBar")}
                className="h-1 w-32 overflow-hidden rounded-full bg-line"
                barClassName="h-full bg-accent transition-[width]"
              />
            </div>
          )}
        </footer>
        </main>

        <SelectionPanel />
      </div>
    </div>
  );
}
