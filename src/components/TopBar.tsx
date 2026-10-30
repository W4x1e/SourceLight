import { useState } from "react";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import LibraryMenu from "./LibraryMenu";
import { formatNumber, t, tx } from "../i18n";

/**
 * The top bar, pared back to what belongs at the top of every screen: which
 * library is open, one search, and a couple of view controls. The heavy library
 * jobs (Scan, Tag, XMP) moved to the Activity centre, and duplicate review to
 * Cull — so Search no longer sits at equal weight beside a scan button.
 *
 * **One search box.** It used to have three modes — Text, Content,
 * Similar — and the app's most distinctive feature, describing a photo to find
 * it, sat behind the second one and an Enter nobody knew to press, while the
 * default mode never used it. Now typing filters by filename, tag and place as
 * it always did, and **Enter searches by content** with the same words; the
 * hint in the box says so. Similar was never a way of searching text — it
 * ignored what was typed and acted on the focused photo — so it moved to the
 * photo: the selection panel, the Lightbox and the right-click menu.
 */
export default function TopBar() {
  const filter = useStore((s) => s.filter);
  const cellSize = useStore((s) => s.cellSize);
  const searchResults = useStore((s) => s.searchResults);
  const searchLabel = useStore((s) => s.searchLabel);
  const searchBusy = useStore((s) => s.searchBusy);
  const contentQuery = useStore((s) => s.committedFilter.semantic?.trim() || null);
  const photos = useStore((s) => s.searchResults ?? s.photos);
  const groupByDate = useStore((s) => s.groupByDate);
  const setGroupByDate = useStore((s) => s.setGroupByDate);
  const stacking = useStore((s) => s.stacking);
  const stackingBusy = useStore((s) => s.stackingBusy);
  const hasPhotos = photos.length > 0;

  const onFilterChange = useStore((s) => s.setFilter);
  const onCellSize = useStore((s) => s.setCellSize);
  const runSemanticSearch = useStore((s) => s.runSemanticSearch);
  const clearSearch = useStore((s) => s.clearSearch);
  const toggleStacking = useStore((s) => s.toggleStacking);
  const setShowShortcuts = useStore((s) => s.setShowShortcuts);

  const [focused, setFocused] = useState(false);

  const activeBanner = searchResults
    ? { icon: "compare" as const, label: searchLabel, count: searchResults.length, clear: () => clearSearch() }
    : contentQuery
      ? {
          icon: "search" as const,
          label: t("shell.topBar.contentSearch", { query: contentQuery }),
          count: photos.length,
          clear: () => onFilterChange({ ...filter, semantic: null }),
        }
      : null;

  const typed = (filter.search ?? "").trim();
  const runSearch = () => {
    // Enter promotes what was typed to a content search. The text filter has
    // already been applying as it was typed; this asks what the photos *show*.
    if (typed) void runSemanticSearch(typed);
  };

  return (
    <header
      className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line bg-panel px-4"
      data-tauri-drag-region="deep"
    >
      {/* The library's folders */}
      <LibraryMenu />

      {/* Omni-search */}
      <div className="mx-auto flex min-w-0 max-w-xl flex-1 justify-center">
        <div className="w-full" data-tour="search">
          {activeBanner ? (
            <div className="flex h-9 items-center gap-2 rounded-md bg-raised px-3 text-caption">
              <Icon name={activeBanner.icon} size={14} className="shrink-0 text-muted" />
              <span className="truncate font-medium text-ink">{activeBanner.label}</span>
              <span className="shrink-0 tabular-nums text-faint">{formatNumber(activeBanner.count)}</span>
              <button
                onClick={activeBanner.clear}
                className="ml-auto shrink-0 rounded-sm p-0.5 text-faint hover:text-ink"
                title={t("common.clear")}
                aria-label={t("common.clear")}
              >
                <Icon name="close" size={14} />
              </button>
            </div>
          ) : (
            <div className="flex h-9 items-center gap-2 rounded-md border border-line bg-base pl-3 pr-1.5 focus-within:border-muted">
              <Icon name="search" size={16} className="shrink-0 text-faint" />
              <input
                className="min-w-0 flex-1 bg-transparent text-caption text-ink placeholder:text-faint outline-none"
                style={{ userSelect: "text", WebkitUserSelect: "text" }}
                // A search box is not prose: WebKit's autocorrect offered to
                // "fix" filenames and tags as they were typed.
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                placeholder={t("shell.topBar.searchPlaceholder")}
                aria-label={t("shell.topBar.searchLabel")}
                value={filter.search ?? ""}
                onChange={(e) => onFilterChange({ ...filter, search: e.target.value || null })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") runSearch();
                  if (e.key === "Escape" && filter.search) {
                    e.stopPropagation();
                    onFilterChange({ ...filter, search: null });
                  }
                }}
                onFocus={() => setFocused(true)}
                onBlur={() => setFocused(false)}
              />
              {/* The one hint that makes content search discoverable: shown
                  while there is something to search for. */}
              {typed && focused && (
                <button
                  // mousedown, not click: the input's blur would hide this first.
                  onMouseDown={(e) => {
                    e.preventDefault();
                    runSearch();
                  }}
                  disabled={searchBusy}
                  className="flex shrink-0 items-center gap-1.5 rounded-sm bg-raised px-2 py-0.5 text-label text-muted hover:text-ink"
                  title={t("shell.topBar.searchByContentHint")}
                >
                  {tx("shell.topBar.searchByContent", { enter: <span className="kbd">{t("shell.keymap.key.enter")}</span> })}
                </button>
              )}
            </div>
          )}
        </div>
      </div>

      {/* View controls */}
      <div className="flex shrink-0 items-center gap-1.5">
        <div className="flex items-center gap-1.5 pr-1" title={t("shell.topBar.thumbnailSize")}>
          <Icon name="grid" size={14} className="text-faint" />
          <input
            type="range"
            min={96}
            max={340}
            step={4}
            value={cellSize}
            onChange={(e) => onCellSize(Number(e.target.value))}
            // Named for a screen reader: the tooltip is on the wrapper.
            aria-label={t("shell.topBar.thumbnailSize")}
            className="h-1 w-20 cursor-default accent-[var(--color-accent)]"
          />
        </div>
        <button
          onClick={() => setGroupByDate(!groupByDate)}
          className={`icon-chip ${groupByDate ? "text-accent" : ""}`}
          disabled={!hasPhotos}
          // A toggle, said as one, like Stacking beside it.
          aria-pressed={groupByDate}
          aria-label={t("shell.topBar.dateHeadings")}
          title={
            groupByDate ? t("shell.topBar.dateHeadingsOn") : t("shell.topBar.dateHeadingsOff")
          }
        >
          <Icon name="calendar" size={17} />
        </button>
        <button
          onClick={() => void toggleStacking()}
          aria-pressed={stacking}
          aria-label={t("shell.topBar.stacking")}
          className={`icon-chip ${stacking ? "text-accent" : ""}`}
          disabled={stackingBusy || !hasPhotos}
          title={t("shell.topBar.stacking")}
        >
          <Icon name="layers" size={17} />
        </button>
        <button
          onClick={() => setShowShortcuts(true)}
          className="icon-chip"
          title={t("shell.shortcuts.title")}
          aria-label={t("shell.shortcuts.title")}
        >
          <Icon name="keyboard" size={17} />
        </button>
      </div>
    </header>
  );
}
