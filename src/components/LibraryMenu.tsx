import { useEffect, useRef, useState } from "react";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { useEscape } from "../lib/useEscape";
import { usePresence } from "../lib/usePresence";
import type { LibraryFolder } from "../bindings";
import { formatNumber, t, tx } from "../i18n";

/** The last part of a path — what a folder is called. */
export const folderName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/**
 * The library's folders, from the top bar.
 *
 * The button here used to open a folder picker straight away and *replace*
 * the library — but the old folder's photos stayed in the catalogue and were
 * never rescanned, so it read as "switch" and behaved as a half-hearted "add".
 * Now a library is a list of folders: each says how many photos it holds and
 * whether any are missing, and can be rescanned, located after a move, or
 * removed. Adding one never replaces anything.
 */
/**
 * The folder list itself — each folder with its counts and actions, the note
 * about photos outside every folder, and Add folder. Shared by the top-bar
 * menu and Settings → Library, so the two can never disagree. `onAct` runs
 * before each action; the menu uses it to close itself.
 */
export function FolderList({ onAct = () => {} }: { onAct?: () => void }) {
  const lib = useStore((s) => s.libraryFolders);
  const scanning = useStore((s) => s.scanning);
  const addFolder = useStore((s) => s.addLibraryFolder);
  const removeFolder = useStore((s) => s.removeLibraryFolder);
  const locateFolder = useStore((s) => s.locateLibraryFolder);
  const forgetUnfiled = useStore((s) => s.forgetUnfiledPhotos);
  const runScan = useStore((s) => s.runScan);
  const setShowImport = useStore((s) => s.setShowImport);
  const addFolderKeywords = useStore((s) => s.addFolderKeywords);

  const folders = lib?.folders ?? [];

  // Every action runs `onAct` first: most open a dialog or a picker, which
  // should not sit under a popover.
  const act = (fn: () => unknown) => () => {
    onAct();
    void fn();
  };

  const row = (f: LibraryFolder) => (
    <li key={f.id} className="px-3 py-2">
      <div className="flex items-center gap-2">
        <Icon name="folder" size={14} className="shrink-0 text-faint" />
        <span className="min-w-0 flex-1 truncate text-caption font-medium text-ink" title={f.path}>
          {folderName(f.path)}
        </span>
        <span className="shrink-0 font-mono text-label tabular-nums text-faint">
          {formatNumber(f.photos)}
        </span>
      </div>
      <div className="ml-6 truncate text-label text-faint" title={f.path}>
        {f.path}
      </div>
      {(!f.online || f.missing > 0) && (
        <div className="ml-6 mt-0.5 flex items-center gap-1 text-label text-warning">
          <Icon name="warn" size={11} className="shrink-0" />
          {!f.online
            ? t("library.libraryMenu.notFound")
            : t("library.libraryMenu.missing", { count: f.missing })}
        </div>
      )}
      {/* 18px, so each action's label starts on the path's edge above it
          (the ghost buttons carry 6px of padding). */}
      <div className="ml-[18px] mt-1 flex flex-wrap gap-1">
        {f.online ? (
          <button
            onClick={act(() => runScan(f.path))}
            disabled={scanning !== null}
            className="btn-ghost px-1.5 py-0.5 text-label"
          >
            {t("library.libraryMenu.rescan")}
          </button>
        ) : null}
        {(!f.online || f.missing > 0) && (
          <button onClick={act(() => locateFolder(f))} className="btn-ghost px-1.5 py-0.5 text-label">
            {t("library.libraryMenu.locate")}
          </button>
        )}
        {/* "2024-05 Karoo trip/" is a keyword the library already has. */}
        <button
          onClick={act(() => addFolderKeywords(f.id, null))}
          className="btn-ghost px-1.5 py-0.5 text-label"
          title={t("library.libraryMenu.folderKeywordsTitle")}
        >
          {t("library.libraryMenu.folderKeywords")}
        </button>
        <button onClick={act(() => removeFolder(f))} className="btn-ghost px-1.5 py-0.5 text-label">
          {t("library.libraryMenu.removeFolder")}
        </button>
      </div>
    </li>
  );

  return (
    <>
      {folders.length === 0 ? (
        <p className="px-3 py-2 text-caption text-faint">{t("library.libraryMenu.noFolders")}</p>
      ) : (
        <ul className="max-h-80 divide-y divide-line overflow-y-auto">{folders.map(row)}</ul>
      )}

      {lib && lib.unfiled > 0 && (
        <div className="mx-3 my-1 rounded-sm bg-base px-2.5 py-2 text-label text-faint">
          {tx("library.libraryMenu.unfiled", {
            count: lib.unfiled,
            forget: (
              <button onClick={act(forgetUnfiled)} className="text-muted underline hover:text-ink">
                {t("library.libraryMenu.unfiledForget", { count: lib.unfiled })}
              </button>
            ),
          })}
        </div>
      )}

      {/* Wraps rather than running out of the popover: "Alle neu scannen"
          spilled 105px past its edge in German. */}
      <div className="mt-1 flex flex-wrap items-center gap-2 border-t border-line px-3 pb-1.5 pt-2">
        <button onClick={act(addFolder)} className="btn-solid text-caption">
          <Icon name="plus" size={13} />
          {t("library.libraryMenu.addFolder")}
        </button>
        {/* From a card: copied into a folder here, then scanned. */}
        <button
          onClick={act(() => setShowImport(true))}
          className="btn-solid text-caption"
          disabled={folders.length === 0}
          title={folders.length === 0 ? t("library.libraryMenu.importNeedsFolder") : t("library.libraryMenu.importTooltip")}
        >
          <Icon name="import" size={13} />
          {t("library.libraryMenu.import")}
        </button>
        {folders.length > 1 && (
          <button
            onClick={act(() => runScan())}
            disabled={scanning !== null}
            className="btn-ghost ml-auto text-caption"
          >
            <Icon name="sync" size={13} />
            {t("library.libraryMenu.rescanAll")}
          </button>
        )}
      </div>
    </>
  );
}

export default function LibraryMenu() {
  const lib = useStore((s) => s.libraryFolders);
  const refreshFolders = useStore((s) => s.refreshFolders);

  const [open, setOpen] = useState(false);
  const presence = usePresence(open);
  const ref = useRef<HTMLDivElement>(null);
  useEscape(open, () => setOpen(false));

  useEffect(() => {
    if (!open) return;
    void refreshFolders();
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [open, refreshFolders]);

  const folders = lib?.folders ?? [];
  const label =
    folders.length === 0
      ? t("library.libraryMenu.addFolder")
      : folders.length === 1
        ? folderName(folders[0]?.path ?? "")
        : t("library.libraryMenu.folderCount", { count: folders.length });
  const trouble = folders.some((f) => !f.online || f.missing > 0);

  return (
    <div ref={ref} className="relative" data-tour="library-menu">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex h-9 max-w-[240px] items-center gap-2.5 rounded-md bg-raised px-3 hover:bg-hover"
        title={folders.map((f) => f.path).join("\n") || t("library.libraryMenu.addFolderTooltip")}
        aria-expanded={open}
        aria-haspopup="dialog"
      >
        <Icon name="folder" size={16} className="shrink-0 text-muted" />
        <span className="truncate text-caption font-semibold text-ink">{label}</span>
        {trouble && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warning" title={t("library.libraryMenu.needsAttention")} />}
        <Icon name="chevron" size={13} className="shrink-0 text-faint" />
      </button>

      {presence.mounted && (
        <div
          role="dialog"
          aria-label={t("library.libraryMenu.title")}
          // The grid's keys stand aside while it is up.
          data-overlay
          // 352px: at 320 a folder's four actions left "Remove…" alone on a
          // second line in English.
          className="anim-pop absolute left-0 top-full z-30 mt-1 w-[22rem] rounded-md bg-raised py-1 shadow-2xl"
          data-tauri-drag-region="false"
          data-closing={presence.closing || undefined}
        >
          <div className="section-label px-3 pb-1 pt-1.5">{t("library.libraryMenu.title")}</div>
          <FolderList onAct={() => setOpen(false)} />
          <div className="mt-1 border-t border-line/60 pt-1">
            <button
              onClick={() => {
                setOpen(false);
                useStore.getState().setShowInsights(true);
              }}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-caption text-muted hover:bg-hover hover:text-ink"
              title={t("library.libraryMenu.insightsTitle")}
            >
              <Icon name="activity" size={14} />
              {t("library.libraryMenu.insights")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
