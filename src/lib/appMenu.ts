import { Menu, MenuItem, PredefinedMenuItem, Submenu, type MenuItemOptions } from "@tauri-apps/api/menu";
import { useStore } from "../store";
import { api } from "./api";
import { toast } from "./dialogs";
import { ACTIONS, actionEvent, type ActionId } from "./keymap";
import { FILE_MANAGER, IS_MAC } from "./platform";
import { requestQuit } from "./quitGuard";
import { onLocaleChange, t, type Key } from "../i18n";

/**
 * The Mac menu bar (#native-look, docs/MACOS.md plan 2).
 *
 * Tauri's default gave every Mac user a generic File / Edit / View / Window /
 * Help with nothing of the app's in it. This one carries the app's commands,
 * each showing the key it is bound to — the user's own binding, since it is
 * rebuilt whenever the keymap changes, and worded in the user's language, so
 * it is rebuilt when that changes too.
 *
 * **Keymap actions run through their key's handler.** A menu item sends an
 * `actionEvent` to the page, which the open surface's keydown handler resolves
 * exactly as it would the key: rating in the grid rates the selection, in the
 * Lightbox the photo shown, and an action that does not apply where the user
 * is does nothing, as its key would. The keys themselves stay with the page:
 * WKWebView offers a keystroke to the page before the menu, and the handlers
 * mark what they use.
 *
 * **The Edit menu is AppKit's own.** Its Undo, Cut, Copy, Paste and Select All
 * go to whatever has focus, which is what makes ⌘C and ⌘V work in a text field
 * at all — and ⌘Z still reaches the app's undo first outside one.
 *
 * Windows has no menu bar in this app, and none is added there.
 */
export function installAppMenu(): void {
  if (!IS_MAC || !("__TAURI_INTERNALS__" in window)) return;
  let built: Record<ActionId, string> | null = null;
  // A change of language rebuilds it even though the keymap is the same one.
  const build = (force = false) => {
    const keymap = useStore.getState().keymap;
    if (!force && keymap === built) return;
    built = keymap;
    void buildMenu(keymap)
      .then((menu) => menu.setAsAppMenu())
      .catch((e) => console.warn("menu bar unavailable:", e));
  };
  build();
  useStore.subscribe(() => build());
  onLocaleChange(() => build(true));
}

/** Runs a keymap action as if its key had been pressed where the user is. */
function run(id: ActionId) {
  document.body.dispatchEvent(actionEvent(id));
}

/**
 * Focuses the search box once it is there. Clearing a search, or switching to
 * Browse, mounts it a render or two later — and one animation frame was not
 * always enough — so this looks for it for up to half a second.
 */
function focusSearch(tries = 30) {
  const input = document.querySelector<HTMLInputElement>('[data-tour="search"] input');
  if (input) input.focus();
  else if (tries > 0) requestAnimationFrame(() => focusSearch(tries - 1));
}

/** The photo a single-photo command acts on in the library. */
function subjectPhoto() {
  const s = useStore.getState();
  const id = s.focusId ?? [...s.selected][0];
  return id == null ? null : (s.photos.find((p) => p.id === id) ?? null);
}

/**
 * The menu accelerator for a keymap combo, or none. Modifier combos only: a
 * bare key such as a rating's "3" would also fire from the menu while it is
 * being typed into a text field, so those are listed in Keyboard Shortcuts
 * instead.
 */
export function accelerator(combo: string): string | undefined {
  const parts = combo.split("+");
  const key = parts.pop() ?? "";
  if (!parts.some((p) => p === "mod" || p === "alt")) return undefined;
  const KEYS: Record<string, string> = {
    arrowleft: "Left", arrowright: "Right", arrowup: "Up", arrowdown: "Down",
    escape: "Escape", enter: "Enter", space: "Space", "\\": "Backslash",
    "[": "BracketLeft", "]": "BracketRight", ",": "Comma", ".": "Period",
    "/": "Slash", "-": "Minus", "=": "Equal", ";": "Semicolon", "'": "Quote", "`": "Backquote",
  };
  const name = KEYS[key] ?? (/^[a-z0-9]$/.test(key) ? key.toUpperCase() : null);
  if (!name) return undefined;
  const mods = parts.map((p) => (p === "mod" ? "CmdOrCtrl" : p === "shift" ? "Shift" : "Alt"));
  return [...mods, name].join("+");
}

async function buildMenu(keymap: Record<ActionId, string>): Promise<Menu> {
  const item = (opts: MenuItemOptions) => MenuItem.new(opts);
  const action = (id: ActionId, text?: Key, { showKey = true } = {}) => {
    const def = ACTIONS.find((a) => a.id === id)!;
    const key = showKey ? accelerator(keymap[id] ?? "") : undefined;
    return item({ text: t(text ?? def.label), accelerator: key, action: () => run(id) });
  };
  const store = () => useStore.getState();
  const sep = () => PredefinedMenuItem.new({ item: "Separator" });

  const app = await Submenu.new({
    text: "SourceLight",
    items: [
      await item({ text: t("shell.menu.about"), action: () => store().openSettings("about") }),
      await sep(),
      await item({ text: t("shell.menu.settings"), accelerator: "CmdOrCtrl+,", action: () => store().setShowSettings(true) }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Services" }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Hide" }),
      await PredefinedMenuItem.new({ item: "HideOthers" }),
      await PredefinedMenuItem.new({ item: "ShowAll" }),
      await sep(),
      // Not AppKit's Quit: that terminates without the runtime hearing of it,
      // so it could not ask before cutting off an export (lib/quitGuard.ts).
      await item({ text: t("shell.menu.quit"), accelerator: "CmdOrCtrl+Q", action: () => void requestQuit() }),
    ],
  });

  const file = await Submenu.new({
    text: t("shell.menu.file"),
    items: [
      await item({
        text: t("shell.menu.addFolder"),
        accelerator: "CmdOrCtrl+O",
        action: () => void store().addLibraryFolder(),
      }),
      await item({
        text: t("shell.menu.importCard"),
        accelerator: "CmdOrCtrl+Shift+I",
        action: () => store().setShowImport(true),
      }),
      await sep(),
      await item({
        text: t("shell.menu.exportSelected"),
        accelerator: "CmdOrCtrl+Shift+E",
        action: () => void store().exportSelection(),
      }),
      await item({
        text: t("common.showInFileManager", { fileManager: FILE_MANAGER }),
        action: () => {
          const photo = subjectPhoto();
          if (photo) api.showInFolder(photo.id).catch((e) => toast(String(e), "error"));
        },
      }),
      await sep(),
      // ⌘W backs out of what is open — Develop, the Lightbox, Compare — as a
      // window's Close would, rather than closing the only window.
      await item({ text: t("common.close"), accelerator: "CmdOrCtrl+W", action: () => run("close-overlay") }),
    ],
  });

  const edit = await Submenu.new({
    text: t("common.edit"),
    items: [
      await PredefinedMenuItem.new({ item: "Undo" }),
      await PredefinedMenuItem.new({ item: "Redo" }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Cut" }),
      await PredefinedMenuItem.new({ item: "Copy" }),
      await PredefinedMenuItem.new({ item: "Paste" }),
      await PredefinedMenuItem.new({ item: "SelectAll" }),
      await sep(),
      await item({
        text: t("shell.menu.find"),
        accelerator: "CmdOrCtrl+F",
        action: () => {
          const s = store();
          s.setMode("browse");
          // A search in force shows as a banner in the box's place: clear it,
          // as ⌘F starts a new search in Finder, so there is a box to type in.
          if (s.searchResults) s.clearSearch();
          if (s.committedFilter.semantic) s.setFilter({ ...s.filter, semantic: null });
          focusSearch();
        },
      }),
    ],
  });

  const workspace = (text: Key, mode: "browse" | "cull" | "places" | "people", n: number) =>
    item({ text: t(text), accelerator: `CmdOrCtrl+${n}`, action: () => store().setMode(mode) });
  const view = await Submenu.new({
    text: t("shell.menu.view"),
    items: [
      await workspace("shell.mode.browse", "browse", 1),
      await workspace("shell.mode.cull", "cull", 2),
      await workspace("shell.mode.places", "places", 3),
      await workspace("shell.mode.people", "people", 4),
      await sep(),
      await action("toggle-filters", "shell.menu.toggleFilters"),
      await action("toggle-zoom"),
      await item({
        text: t("shell.mode.activity"),
        accelerator: "CmdOrCtrl+Alt+A",
        action: () => store().setShowActivity(!store().showActivity),
      }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Fullscreen" }),
    ],
  });

  const photo = await Submenu.new({
    text: t("shell.menu.photo"),
    items: [
      await Submenu.new({
        text: t("shell.menu.rating"),
        items: await Promise.all(
          (["rate-5", "rate-4", "rate-3", "rate-2", "rate-1", "rate-0"] as const).map((id) => action(id)),
        ),
      }),
      await Submenu.new({
        text: t("shell.menu.flag"),
        items: await Promise.all((["flag-pick", "flag-reject", "flag-none"] as const).map((id) => action(id))),
      }),
      await Submenu.new({
        text: t("shell.menu.colourLabel"),
        items: await Promise.all(
          (["label-1", "label-2", "label-3", "label-4", "label-5", "label-none"] as const).map((id) =>
            action(id),
          ),
        ),
      }),
      // ⌘Z by default, which Edit → Undo already shows; one key, one item.
      await action("undo-verdict", undefined, { showKey: false }),
      await sep(),
      await action("open-develop"),
      await item({
        text: t("shell.menu.findSimilar"),
        action: () => {
          const p = subjectPhoto();
          if (p) void store().findSimilar(p.id, p.filename);
        },
      }),
      await sep(),
      await action("prev-photo"),
      await action("next-photo"),
    ],
  });

  const develop = await Submenu.new({
    text: t("shell.mode.develop"),
    items: [
      await action("develop-before-after"),
      await action("develop-crop"),
      await action("develop-mask"),
      await sep(),
      await action("develop-save"),
      await action("develop-reset"),
    ],
  });

  const windowMenu = await Submenu.new({
    text: t("shell.menu.window"),
    items: [
      await PredefinedMenuItem.new({ item: "Minimize" }),
      await PredefinedMenuItem.new({ item: "Maximize" }),
    ],
  });
  await windowMenu.setAsWindowsMenuForNSApp();

  const help = await Submenu.new({
    text: t("shell.menu.help"),
    items: [
      await item({
        text: t("shell.menu.keyboardShortcuts"),
        accelerator: "CmdOrCtrl+/",
        action: () => store().setShowShortcuts(true),
      }),
    ],
  });
  await help.setAsHelpMenuForNSApp();

  return Menu.new({ items: [app, file, edit, view, photo, develop, windowMenu, help] });
}
