import { toggleFullscreen } from "../lib/fullscreen";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Person, Place } from "../bindings";
import { api } from "../lib/api";
import { ACTIONS, actionEvent, comboToLabel, resolveAction, type ActionId, type Context } from "../lib/keymap";
import { rankCommands, type Command } from "../lib/palette";
import { t, type Key } from "../i18n";
import { countryName, localPlaceName } from "../lib/countries";
import { developShown, useStore, type AppMode } from "../store";
import { EMPTY_FILTER, isFilterEmpty, normaliseFilter } from "../types";
import { useHeld, usePresence } from "../lib/usePresence";
import { scrimOpen } from "../lib/useEscape";

/**
 * The command palette, Ctrl+K, and the keys that work anywhere.
 *
 * Four workspaces, Develop, Activity and Settings hold dozens of actions, and
 * many were reachable only from one panel or menu. Here every one is a few
 * letters away — the app's commands, the keyboard actions that apply where
 * you are (with the key each is bound to, so the palette teaches them), and
 * the people, places, collections and saved views to jump to.
 *
 * Keymap actions run as their key would (`actionEvent`), through whichever
 * surface is open, so a command here never does something its key wouldn't.
 */
export default function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [people, setPeople] = useState<Person[]>([]);
  const [places, setPlaces] = useState<Place[]>([]);
  const listRef = useRef<HTMLDivElement>(null);

  // The keys that work anywhere: the palette, the workspaces, search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      const action = resolveAction(s.keymap, "global", e);
      if (!action) return;
      if (action === "open-palette" && open) {
        e.preventDefault();
        setOpen(false);
        return;
      }
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
      if (scrimOpen()) return;
      e.preventDefault();
      runGlobal(action, () => setOpen(true));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  // Fresh names each time it opens: People and Places change elsewhere.
  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActive(0);
    let live = true;
    void api
      .listPeople()
      .then((p) => live && setPeople(p))
      .catch(() => {});
    void api
      .listPlaces()
      .then((p) => live && setPlaces(p))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [open]);

  // Escape closes it — its own listener, not `useEscape`, which stands
  // aside for any open `.dialog-scrim`, and the palette is one: it never
  // closed on Escape. Nothing opens over it, so it always owns the key.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      setOpen(false);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);

  // Held as the palette fades out, so its list doesn't empty on the way.
  const built = useMemo(() => (open ? buildCommands(people, places) : null), [open, people, places]);
  const commands = useHeld(built) ?? [];
  const shown = useMemo(() => rankCommands(commands, query), [commands, query]);
  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  // Mounted a moment past closing, for the exit (see usePresence).
  const presence = usePresence(open);
  if (!presence.mounted) return null;

  const run = (c: Command | undefined) => {
    if (!c) return;
    setOpen(false);
    // After the palette is gone, so a key-driven action sees the surface
    // under it, not the palette. A timeout, not an animation frame: a frame
    // never comes while the window isn't drawing.
    setTimeout(() => c.run(), 0);
  };

  return (
    <div
      data-closing={presence.closing || undefined}
      className="dialog-scrim anim-scrim fixed inset-0 z-[70] flex justify-center bg-scrim pt-[12vh]"
      onClick={(e) => e.target === e.currentTarget && setOpen(false)}
    >
      <div
        role="dialog"
        aria-label={t("shell.palette.title")}
        className="anim-card flex max-h-[60vh] w-[560px] max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <input
          autoFocus
          className="border-b border-line bg-transparent px-4 py-3 text-body text-ink outline-none placeholder:text-faint"
          placeholder={t("shell.palette.placeholder")}
          aria-label={t("shell.palette.inputLabel")}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") setActive((i) => Math.min(i + 1, shown.length - 1));
            else if (e.key === "ArrowUp") setActive((i) => Math.max(i - 1, 0));
            else if (e.key === "Enter") run(shown[active]);
            else return;
            e.preventDefault();
          }}
        />
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto py-1" role="listbox">
          {shown.length === 0 && <div className="px-4 py-3 text-caption text-faint">{t("shell.palette.empty")}</div>}
          {shown.map((c, i) => (
            <button
              key={c.id}
              data-index={i}
              role="option"
              aria-selected={i === active}
              onMouseMove={() => setActive(i)}
              onClick={() => run(c)}
              className={`flex w-full items-center gap-3 px-4 py-1.5 text-left text-caption ${
                i === active ? "bg-hover text-ink" : "text-muted"
              }`}
            >
              <span className="min-w-0 flex-1 truncate">{c.label}</span>
              <span className="shrink-0 text-label text-faint">{c.group}</span>
              {c.keys && (
                <kbd className="shrink-0 rounded-sm border border-line px-1.5 font-mono text-label text-faint">
                  {c.keys}
                </kbd>
              )}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

/** Looks for the search box for up to half a second: it mounts a render or
 *  two after switching to Browse. */
function focusSearch(tries = 30) {
  const input = document.querySelector<HTMLInputElement>('[data-tour="search"] input');
  if (input) input.focus();
  else if (tries > 0) requestAnimationFrame(() => focusSearch(tries - 1));
}

function goTo(mode: AppMode) {
  const s = useStore.getState();
  // A workspace is under whatever is open over it; take that away first.
  if (developShown(s)) void s.closeDevelop();
  if (s.compare) s.closeCompare();
  if (s.lightboxIndex !== null) s.openLightbox(null);
  s.setMode(mode);
}

/** What a global key does. */
function runGlobal(action: ActionId, openPalette: () => void) {
  const s = useStore.getState();
  switch (action) {
    case "open-palette":
      openPalette();
      break;
    case "mode-browse":
      goTo("browse");
      break;
    case "mode-cull":
      goTo("cull");
      break;
    case "mode-places":
      goTo("places");
      break;
    case "mode-people":
      goTo("people");
      break;
    case "focus-search":
      goTo("browse");
      if (s.searchResults) s.clearSearch();
      focusSearch();
      break;
    case "add-folder":
      void s.addLibraryFolder();
      break;
    case "import-card":
      s.setShowImport(true);
      break;
    case "export-selected":
      if (s.selected.size > 0) void s.exportSelection();
      break;
    case "open-settings":
      s.setShowSettings(true);
      break;
    case "show-activity":
      s.setShowActivity(!s.showActivity);
      break;
    case "close-view":
      // Whatever is open closes the way Escape closes it.
      document.body.dispatchEvent(actionEvent("close-overlay"));
      break;
    case "toggle-fullscreen":
      void toggleFullscreen();
      break;
  }
}

/** Where the keyboard is: which keymap actions apply right now. */
function currentContext(): Context {
  const s = useStore.getState();
  if (developShown(s)) return "develop";
  if (s.compare) return "compare";
  if (s.lightboxIndex !== null) return "lightbox";
  if (s.mode === "cull") return "cull";
  if (s.mode === "people") return "people";
  return "library";
}

function buildCommands(people: Person[], places: Place[]): Command[] {
  const s = useStore.getState();
  const keys = (id: ActionId) => (s.keymap[id] ? comboToLabel(s.keymap[id]) : undefined);
  const out: Command[] = [];
  const app = (id: string, label: Key, run: () => void, extra: Partial<Command> = {}) =>
    out.push({ id, label: t(label), group: t("shell.keymap.group.app"), run, ...extra });
  /** A command's other names: the catalogue holds them as one comma-separated list. */
  const words = (key: Key) => t(key).split(",").map((w) => w.trim());

  for (const id of ["mode-browse", "mode-cull", "mode-places", "mode-people", "focus-search", "toggle-fullscreen"] as const) {
    const def = ACTIONS.find((a) => a.id === id)!;
    app(id, def.label, () => runGlobal(id, () => {}), { keys: keys(id) });
  }
  app("settings", "shell.mode.settings", () => s.setShowSettings(true), { aliases: words("shell.palette.aliases.settings") });
  app("settings-library", "shell.palette.settingsLibrary", () => s.openSettings("library"));
  app("settings-ai", "shell.palette.settingsAi", () => s.openSettings("ai"));
  app("settings-export", "shell.palette.settingsExport", () => s.openSettings("export"));
  app("settings-about", "shell.palette.settingsAbout", () => s.openSettings("about"));
  app("shortcuts", "shell.shortcuts.title", () => s.setShowShortcuts(true), { aliases: words("shell.palette.aliases.shortcuts") });
  app("activity", "shell.mode.activity", () => s.setShowActivity(true), { aliases: words("shell.palette.aliases.activity") });
  app("tour", "shell.palette.tour", () => s.startTour(), { aliases: words("shell.palette.aliases.tour") });
  app("add-folder", "shell.keymap.action.addFolder", () => void s.addLibraryFolder());
  app("import-card", "shell.keymap.action.importCard", () => s.setShowImport(true));
  app("insights", "shell.palette.insights", () => s.setShowInsights(true), {
    aliases: words("shell.palette.aliases.insights"),
  });
  app("folder-keywords", "shell.palette.folderKeywords", () => void s.addFolderKeywords(null, null), {
    aliases: words("shell.palette.aliases.folderKeywords"),
  });
  app("folder-keywords-off", "shell.palette.folderKeywordsOff", () => void s.removeFolderKeywords(null));
  const view = s.searchResults ?? s.photos;
  if (view.length > 0) {
    app("slideshow", "shell.palette.slideshow", () => s.playSlideshow(view), {
      aliases: words("shell.palette.aliases.slideshow"),
    });
  }
  /** A command whose label carries a value, so is worded already. */
  const worded = (id: string, label: string, run: () => void, extra: Partial<Command> = {}) =>
    out.push({ id, label, group: t("shell.keymap.group.app"), run, ...extra });
  if (s.selected.size > 1) {
    worded("slideshow-selected", t("shell.palette.slideshowSelected", { count: s.selected.size }), () =>
      s.playSlideshow(view.filter((p) => s.selected.has(p.id))),
    );
  }
  if (s.selected.size > 0) {
    worded("export", t("shell.palette.exportSelected", { count: s.selected.size }), () => void s.exportSelection());
    if (s.selected.size === 1) {
      const id = [...s.selected][0]!;
      const p = (s.searchResults ?? s.photos).find((x) => x.id === id);
      if (p) app("copy-meta", "shell.palette.copyMeta", () => s.copyMetaFrom(p.id, p.filename));
    }
    if (s.copiedFrom) {
      const f = s.copiedFrom;
      worded("paste-location", t("shell.palette.pasteLocation", { name: f.filename }), () => void s.pasteLocation([...s.selected]));
      worded("paste-keywords", t("shell.palette.pasteKeywords", { name: f.filename }), () => void s.pasteKeywords([...s.selected]));
    }
    worded("capture-time", t("shell.palette.captureTime", { count: s.selected.size }), () => s.setCaptureTimeFor([...s.selected]), {
      aliases: words("shell.palette.aliases.captureTime"),
    });
  }
  if (!isFilterEmpty(s.filter)) app("clear-filters", "shell.palette.clearFilters", () => s.clearFilter());
  if ((s.searchResults ?? s.photos).length >= 12) {
    app("highlights", "shell.palette.highlights", () => void s.makeHighlights(24, t("shell.palette.thisView")), {
      aliases: words("shell.palette.aliases.highlights"),
    });
  }

  // The keyboard's actions that apply where you are, each with its key.
  const here = currentContext();
  for (const a of ACTIONS) {
    if (!a.contexts.includes(here)) continue;
    out.push({
      id: `key:${a.id}`,
      label: t(a.label),
      group: t(a.group),
      keys: keys(a.id),
      run: () => document.body.dispatchEvent(actionEvent(a.id)),
    });
  }

  const show = (filter: typeof EMPTY_FILTER) => {
    goTo("browse");
    s.setFilter({ ...filter, sort: s.filter.sort ?? "newest" });
  };
  for (const p of people) {
    if (!p.name || p.hidden) continue;
    out.push({
      id: `person:${p.id}`,
      label: t("shell.palette.photosOf", { name: p.name }),
      // Two people can share a name; how many photos tells them apart.
      group: t("shell.palette.groupPerson", { count: p.count }),
      aliases: [p.name],
      run: () => show({ ...EMPTY_FILTER, personIds: [p.id] }),
    });
  }
  for (const pl of places) {
    out.push({
      id: `place:${pl.id}`,
      label: t("shell.palette.photosIn", { name: localPlaceName(pl.name, pl.country) }),
      group: t("shell.palette.groupPlace"),
      aliases: [pl.name, localPlaceName(pl.name, pl.country), ...(pl.country ? [pl.country, countryName(pl.country)] : [])],
      run: () => show({ ...EMPTY_FILTER, placeIds: [pl.id] }),
    });
  }
  for (const c of s.collections) {
    out.push({
      id: `collection:${c.id}`,
      label: c.name,
      group: t("shell.palette.groupCollection"),
      run: () => show({ ...EMPTY_FILTER, collectionId: c.id }),
    });
  }
  for (const v of s.smartCollections) {
    out.push({
      id: `view:${v.id}`,
      label: v.name,
      group: t("shell.palette.groupSavedView"),
      run: () => show(normaliseFilter(v.filter)),
    });
  }
  return out;
}
