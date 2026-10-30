import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStore } from "../store";
import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { batchSucceeded, sidecarReadSummary, sidecarSummary } from "../lib/export";
import { Icon, type IconName } from "../lib/icons";
import { useEscape } from "../lib/useEscape";
import { usePresence } from "../lib/usePresence";
import { FILE_MANAGER, IS_MAC, trashName } from "../lib/platform";
import { t } from "../i18n";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import type { Photo } from "../types";

/**
 * The right-click menu on a photo.
 *
 * The app suppresses the webview's own context menu so it reads as a desktop
 * tool rather than a web page — and until this there was nothing in its place,
 * so right-clicking a photo, the first thing a desktop user tries, did nothing.
 * There was also no way at all to find a photo's file on disk.
 *
 * In Browse it acts on the selection: right-clicking a photo outside it selects
 * that photo first, as Explorer and other editors do, so the menu and the selection
 * panel always agree about what they will change. In Cull it acts on the one
 * frame, whose verdicts belong to the keyboard.
 *
 * **On a Mac it is a real menu.** The items are one list (`entries`) drawn two
 * ways: as this HTML menu on Windows, and on a Mac as an `NSMenu` — keyboard
 * navigation, type-to-select, the system's look and VoiceOver, all of which a
 * drawn menu only imitates. The star row becomes a Rating submenu there, with
 * a check against the rating the photos share.
 */

function Item({
  icon,
  label,
  onClick,
  danger = false,
}: {
  icon?: IconName;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      role="menuitem"
      onClick={onClick}
      className={`flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-caption hover:bg-hover focus:bg-hover focus:outline-none ${
        danger ? "text-danger-ink" : "text-ink"
      }`}
    >
      <span className="grid w-4 shrink-0 place-items-center text-faint">
        {icon && <Icon name={icon} size={14} />}
      </span>
      {label}
    </button>
  );
}

const Divider = () => <div className="my-1 h-px bg-line" role="separator" />;


/** One line of the menu, in the order it is drawn. */
type Entry =
  | { kind: "item"; label: string; icon?: IconName; danger?: boolean; act: () => unknown }
  | { kind: "rating"; current: number | null }
  | { kind: "sep" };

/** What the menu offers for `photos`, opened in `context`. Both renderers draw this. */
function entries(photos: Photo[], ids: number[], context: "browse" | "cull"): Entry[] {
  const s = useStore.getState();
  const one = photos.length === 1 ? photos[0] : null;
  const rejects = photos.filter((p) => p.flag === -1);
  const out: Entry[] = [];
  if (one) {
    out.push(
      { kind: "item", icon: "adjust", label: t("library.photoMenu.openInDevelop"), act: () => s.openDevelop(one.id) },
      // From Cull, in Browse: the lookalikes replaced the set being
      // culled, with nothing to say so and no way back inside Cull.
      {
        kind: "item",
        icon: "compare",
        label: t("library.photoMenu.findSimilar"),
        act: () => {
          if (context === "cull") s.setMode("browse");
          return s.findSimilar(one.id, one.filename);
        },
      },
      // By colour alone, whatever the photos show.
      {
        kind: "item",
        icon: "compare",
        label: t("library.photoMenu.findSimilarColours"),
        act: () => {
          if (context === "cull") s.setMode("browse");
          return s.findSimilarColours(one.id, one.filename);
        },
      },
      {
        kind: "item",
        icon: "folder",
        label: t("common.showInFileManager", { fileManager: FILE_MANAGER }),
        act: () => api.showInFolder(one.id).catch((e) => toast(String(e), "error")),
      },
      // The photo itself, with its edits, to paste into a message or a
      // document.
      { kind: "item", icon: "copy", label: t("shell.keymap.action.copyImage"), act: () => s.copyImage(one.id) },
      {
        kind: "item",
        icon: "copy",
        label: t("library.photoMenu.copyPath"),
        act: () =>
          navigator.clipboard
            .writeText(one.path)
            .then(() => toast(t("library.photoMenu.pathCopied"), "success"))
            .catch(() => toast(t("library.photoMenu.pathCopyFailed"), "error")),
      },
    );
  }
  // Two or more in Browse: Compare, the selection panel's first action for
  // a set, was missing from the menu that acts on the same set.
  if (!one && context === "browse" && photos.length > 1) {
    out.push({ kind: "item", icon: "compare", label: t("library.selection.compare"), act: () => s.compareSelection() });
  }
  // A slideshow: of the selection, in the grid's order, or of the
  // whole view from this photo.
  out.push({
    kind: "item",
    icon: "play",
    label: one
      ? t("library.photoMenu.slideshowFromHere")
      : t("shell.palette.slideshowSelected", { count: photos.length }),
    act: () => {
      // Cull's strip when it opened the menu: from Picks, the picks.
      const view = s.photoMenu?.set ?? s.searchResults ?? s.photos;
      if (one) s.playSlideshow(view, Math.max(0, view.findIndex((p) => p.id === one.id)));
      else {
        const chosen = new Set(ids);
        s.playSlideshow(view.filter((p) => chosen.has(p.id)));
      }
    },
  });
  if (context === "browse") {
    const ratings = new Set(photos.map((p) => p.rating ?? 0));
    if (one) out.push({ kind: "sep" });
    out.push(
      { kind: "rating", current: ratings.size === 1 ? ([...ratings][0] ?? null) : null },
      { kind: "item", icon: "flag", label: t("library.photoMenu.pick"), act: () => s.flagSelection(1) },
      { kind: "item", icon: "flag-off", label: t("library.photoMenu.reject"), act: () => s.flagSelection(-1) },
      { kind: "sep" },
      {
        kind: "item",
        label: t("library.photoMenu.export", { count: photos.length }),
        act: () => s.exportSelection(),
      },
      // A camera clock on the wrong time, or a scan with none.
      { kind: "item", icon: "clock", label: t("library.captureTime.open"), act: () => s.setCaptureTimeFor(ids) },
      // One photo's place and keywords, for the rest of a set.
      ...(one
        ? [{ kind: "item" as const, icon: "copy" as const, label: t("shell.palette.copyMeta"), act: () => s.copyMetaFrom(one.id, one.filename) }]
        : []),
      ...(s.copiedFrom && !(one && one.id === s.copiedFrom.id)
        ? [
            {
              kind: "item" as const,
              icon: "pin" as const,
              label: t("library.photoMenu.pasteLocation", { name: s.copiedFrom.filename }),
              act: () => s.pasteLocation(ids),
            },
            {
              kind: "item" as const,
              label: t("library.photoMenu.pasteKeywords", { name: s.copiedFrom.filename }),
              act: () => s.pasteKeywords(ids),
            },
          ]
        : []),
      // Sidecars for just these, where Activity only does the lot.
      {
        kind: "item",
        label: t("library.photoMenu.writeXmp", { count: photos.length }),
        act: () =>
          s
            .exportXmp(ids)
            .then((o) => toast(sidecarSummary(o), batchSucceeded(o) ? "success" : "error"))
            .catch((e) => toast(String(e), "error")),
      },
      {
        kind: "item",
        label: t("library.photoMenu.readXmp", { count: photos.length }),
        act: () =>
          s
            .importXmp(ids)
            .then((r) => toast(sidecarReadSummary(r), r.read > 0 ? "success" : "info"))
            .catch((e) => toast(String(e), "error")),
      },
    );
  }
  if (rejects.length > 0 && rejects.length === photos.length) {
    out.push(
      { kind: "sep" },
      {
        kind: "item",
        danger: true,
        label: t("library.photoMenu.moveRejects", { count: rejects.length, trash: trashName() }),
        act: () => s.binRejects(rejects.map((p) => p.id)),
      },
    );
  }
  if (context === "browse") {
    if (rejects.length !== photos.length) out.push({ kind: "sep" });
    out.push({
      kind: "item",
      icon: "trash",
      danger: true,
      label: t("library.photoMenu.delete", { count: photos.length }),
      act: () => s.deletePhotos(photos),
    });
  }
  return out;
}

/** The menu as an `NSMenu`, shown at the pointer. */
async function popUpNative(list: Entry[], rate: (n: number) => unknown): Promise<void> {
  const items = await Promise.all(
    list.map(async (e) => {
      if (e.kind === "sep") return PredefinedMenuItem.new({ item: "Separator" });
      if (e.kind === "item") return MenuItem.new({ text: e.label, action: () => void e.act() });
      return Submenu.new({
        text: t("library.photoMenu.rating"),
        items: await Promise.all(
          [5, 4, 3, 2, 1].map((n) =>
            CheckMenuItem.new({ text: "★".repeat(n), checked: e.current === n, action: () => void rate(n) }),
          ),
        ),
      });
    }),
  );
  const menu = await Menu.new({ items });
  await menu.popup();
}

const NATIVE = IS_MAC && typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export default function PhotoMenu() {
  const menu = useStore((s) => s.photoMenu);
  const close = useStore((s) => s.closePhotoMenu);
  const all = useStore((s) => s.searchResults ?? s.photos);
  const rateSelection = useStore((s) => s.rateSelection);

  // A Mac shows an NSMenu instead, and there is nothing to draw here.
  useEffect(() => {
    if (!NATIVE || !menu) return;
    const byId = new Map(all.map((p) => [p.id, p]));
    const photos = menu.ids
      .map((id) => byId.get(id))
      .filter((p): p is NonNullable<typeof p> => p !== undefined);
    // Cleared first: the menu is modal, and an item's action may open
    // another (the Trash confirmation) that must not find this one still set.
    close();
    popUpNative(entries(photos, menu.ids, menu.context), rateSelection).catch((e) =>
      console.warn("context menu unavailable:", e),
    );
  }, [menu, all, close, rateSelection]);

  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  // The last menu, kept for the exit: the store clears `photoMenu` on close,
  // and the menu still has to be drawn while it fades out.
  const last = useRef(menu);
  if (menu) last.current = menu;
  const { mounted, closing } = usePresence(menu !== null);
  const [hoverStar, setHoverStar] = useState(0);

  useEscape(menu !== null, close);

  // Close on any click elsewhere, and on scroll or resize: a menu left
  // floating over content that has moved points at the wrong photo.
  useEffect(() => {
    if (!menu) return;
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onAway = () => close();
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("wheel", onAway, { passive: true });
    window.addEventListener("resize", onAway);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("wheel", onAway);
      window.removeEventListener("resize", onAway);
    };
  }, [menu, close]);

  // Kept inside the window: flipped up or left when it would run off an edge.
  useLayoutEffect(() => {
    // Closing keeps the position it had, so the exit plays where the menu was.
    if (!menu) return;
    if (!ref.current) {
      setPos(null);
      return;
    }
    const r = ref.current.getBoundingClientRect();
    const left = menu.x + r.width > window.innerWidth - 8 ? Math.max(8, menu.x - r.width) : menu.x;
    const top = menu.y + r.height > window.innerHeight - 8 ? Math.max(8, menu.y - r.height) : menu.y;
    setPos({ left, top });
  }, [menu]);

  // Focus the first item once placed — not before: while it is being measured
  // the menu is `visibility: hidden`, and a hidden element cannot take focus.
  useEffect(() => {
    if (pos) ref.current?.querySelector<HTMLButtonElement>("[role=menuitem]")?.focus();
  }, [pos]);

  const shown = menu ?? (mounted ? last.current : null);
  if (NATIVE || !shown) return null;

  // By id, once: a search per id over the view was 2×10⁸ compares
  // for a select-all of 20k, on every render of the menu.
  const byId = new Map(all.map((p) => [p.id, p]));
  const photos = shown.ids
    .map((id) => byId.get(id))
    .filter((p): p is NonNullable<typeof p> => p !== undefined);
  const one = photos.length === 1 ? photos[0] : null;

  const run = (fn: () => unknown) => () => {
    close();
    void fn();
  };

  return (
    <div
      ref={ref}
      role="menu"
      // An overlay: the workspace's keys stand aside while it is up.
      data-overlay
      aria-label={one ? t("library.photoMenu.actionsFor", { name: one.filename }) : t("library.photoMenu.actionsForMany", { count: photos.length })}
      className="anim-pop fixed z-[60] min-w-52 rounded-md bg-raised py-1 shadow-2xl"
      data-closing={closing || undefined}
      style={{ left: pos?.left ?? shown.x, top: pos?.top ?? shown.y, visibility: pos ? "visible" : "hidden" }}
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        // Arrow keys walk the items, as in any native shown.
        if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
        e.preventDefault();
        const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]") ?? [])];
        const i = items.indexOf(document.activeElement as HTMLButtonElement);
        const next = e.key === "ArrowDown" ? (i + 1) % items.length : (i - 1 + items.length) % items.length;
        items[next]?.focus();
      }}
    >
      {entries(photos, shown.ids, shown.context).map((e, i) =>
        e.kind === "sep" ? (
          <Divider key={i} />
        ) : e.kind === "item" ? (
          <Item key={i} icon={e.icon} label={e.label} danger={e.danger} onClick={run(e.act)} />
        ) : (
          // Verdicts as one row each, like the selection panel: a rating is
          // a single click, not a submenu to aim for.
          <div
            key={i}
            className="flex items-center gap-0.5 px-3 py-1"
            role="group"
            aria-label={t("library.photoMenu.rating")}
            onMouseLeave={() => setHoverStar(0)}
          >
            <span className="w-4 shrink-0" />
            {[1, 2, 3, 4, 5].map((n) => (
              <button
                key={n}
                role="menuitem"
                onClick={run(() => rateSelection(n))}
                onMouseEnter={() => setHoverStar(n)}
                onFocus={() => setHoverStar(n)}
                // The rating the photos share until the pointer is on a
                // star: it drew five empty stars over a four-star photo.
                className={`rounded-sm px-0.5 text-body leading-none focus:outline-none ${
                  n <= (hoverStar || (e.current ?? 0)) ? "text-accent" : "text-dim"
                }`}
                title={t("library.photoMenu.rate", { rating: n })}
                aria-label={t("library.photoMenu.rate", { rating: n })}
              >
                ★
              </button>
            ))}
          </div>
        ),
      )}
    </div>
  );
}
