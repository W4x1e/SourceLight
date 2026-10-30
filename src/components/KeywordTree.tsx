import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { sentenceCase } from "../lib/format";
import { formatNumber, t } from "../i18n";
import { tagFromLocal, tagName } from "../i18n/tags";
import { createPortal } from "react-dom";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { confirm, prompt } from "../lib/dialogs";
import { useEscape } from "../lib/useEscape";
import {
  buildKeywordTree,
  findInTree,
  flatten,
  inBranch,
  parentsOf,
  pathOf,
  selectionCounts,
  type KeywordRow,
} from "../lib/keywordTree";
import { facetClick } from "../lib/facetClick";

/** Top-level keywords shown before "Show more", as every facet list does. */
const LIMIT = 8;
/** Indent per level, px. */
const INDENT = 12;
/** How far the pointer must travel before a press becomes a drag, px. */
const DRAG_SLOP = 5;
/** The drop target meaning "the top of the tree". No keyword's name can hold a
 *  `|` (it separates a path's levels), so this can't be one. */
const TOP = "|top";

type Menu = { name: string; x: number; y: number };

/**
 * The Keywords section of the filter rail: subject keywords as a tree.
 *
 * Clicking a keyword filters to it **and everything filed beneath it**, the
 * counts following the branch. The tree is the user's to arrange at any time —
 * drag a keyword onto another to file it there, or onto "Top level" — and to
 * manage from each row's menu (right-click, or the ⋯ button): new keyword
 * inside, rename (to an existing name, a merge), move, delete. With photos
 * selected, a box on each row shows which of them have it, and puts it on or
 * takes it off. Alt-click leaves a keyword's photos out instead, and
 * with two or more chosen, a switch says whether photos need any or all.
 *
 * Dragging is done with pointer events, not HTML drag and drop: the webview
 * hands HTML drags to the window's file-drop handler on Windows, so they never
 * reach the page.
 */
export default function KeywordTree() {
  const nodes = useStore((s) => s.keywords);
  const facets = useStore((s) => s.facets);
  const filter = useStore((s) => s.filter);
  const setFilter = useStore((s) => s.setFilter);
  const selected = useStore((s) => s.selected);
  const shownPhotos = useStore((s) => s.searchResults ?? s.photos);
  const railExpanded = useStore((s) => s.railExpanded);
  const setRailExpanded = useStore((s) => s.setRailExpanded);
  const createKeyword = useStore((s) => s.createKeyword);
  const moveKeyword = useStore((s) => s.moveKeyword);
  const renameKeyword = useStore((s) => s.renameKeyword);
  const deleteKeyword = useStore((s) => s.deleteKeyword);
  const setKeywordOn = useStore((s) => s.setKeywordOn);

  const [query, setQuery] = useState("");
  const [menu, setMenu] = useState<Menu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);

  const chosen = filter.subjects;
  const excluded = useMemo(() => filter.excludeSubjects ?? [], [filter.excludeSubjects]);
  // Left-out keywords stay in the tree, so they can be let back in.
  const marked = useMemo(() => [...chosen, ...excluded], [chosen, excluded]);
  const parents = useMemo(() => parentsOf(nodes), [nodes]);
  const tree = useMemo(() => buildKeywordTree(nodes, facets?.subjects ?? [], marked), [nodes, facets, marked]);
  const found = useMemo(() => findInTree(tree, query), [tree, query]);
  const searching = query.trim() !== "";
  const showAll = railExpanded.subjects ?? false;
  const roots = searching || showAll ? found : found.slice(0, LIMIT);
  // A branch is open when opened, while searching, or while it holds a
  // chosen keyword — a choice is never hidden inside a closed branch.
  const holdsChoice = useMemo(() => {
    const out = new Set<string>();
    for (const c of marked) for (const p of pathOf(parents, c).slice(0, -1)) out.add(p);
    return out;
  }, [marked, parents]);
  const isOpen = (name: string) => searching || holdsChoice.has(name) || (railExpanded[`kw:${name}`] ?? false);
  const rows = flatten(roots, isOpen);

  // What the selection has, for the assign boxes.
  const selection = useMemo(() => shownPhotos.filter((p) => selected.has(p.id)), [shownPhotos, selected]);
  const have = useMemo(() => selectionCounts(selection), [selection]);
  const selIds = selection.map((p) => p.id);

  const known = (name: string) => nodes.some((n) => n.name === name) || tree.some((r) => r.name === name);

  // A click chooses a keyword; Alt-click leaves it out; any click on one left
  // out lets it back in.
  const clickKeyword = (name: string, alt: boolean) => {
    const next = facetClick(chosen, excluded, name, alt);
    setFilter({ ...filter, subjects: next.chosen, excludeSubjects: next.excluded });
  };
  const matchAll = filter.subjectsAll ?? false;

  // --- Actions, shared by the row menu and the section's own buttons -------

  const newKeyword = async (parent: string | null) => {
    const name = await prompt({
      title: parent ? t("filters.kw.newInsideTitle", { name: tagName(parent) }) : t("filters.kw.newTitle"),
      placeholder: t("filters.kw.placeholder"),
      confirmLabel: t("filters.collection.create"),
    });
    if (!name) return;
    if (await createKeyword(name, parent)) {
      if (parent) setRailExpanded(`kw:${parent}`, true);
    }
  };

  const rename = async (name: string) => {
    const next = await prompt({
      title: t("filters.kw.renameTitle", { name: tagName(name) }),
      defaultValue: name,
      confirmLabel: t("common.rename"),
    });
    if (!next || next.trim().toLowerCase() === name) return;
    const target = next.trim().toLowerCase();
    if (known(target)) {
      const ok = await confirm({
        title: t("filters.kw.mergeTitle", { name: tagName(name), target: tagName(target) }),
        message: t("filters.kw.mergeMessage", { name: tagName(name), target: tagName(target) }),
        confirmLabel: t("filters.kw.merge"),
      });
      if (!ok) return;
    }
    await renameKeyword(name, next);
  };

  const moveInto = async (name: string) => {
    const where = await prompt({
      title: t("filters.kw.moveTitle", { name: tagName(name) }),
      message: t("filters.kw.moveMessage"),
      placeholder: t("filters.kw.placeholder"),
      confirmLabel: t("filters.kw.move"),
    });
    // A keyword typed in the interface's language files under the stored one.
    if (where) await moveKeyword(name, tagFromLocal(where.trim()) ?? where.trim().toLowerCase());
  };

  const remove = async (name: string) => {
    const ok = await confirm({
      title: t("filters.kw.deleteTitle", { name: tagName(name) }),
      message: t("filters.kw.deleteMessage"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (ok) await deleteKeyword(name);
  };

  // --- Dragging a keyword onto another ------------------------------------

  const press = useRef<{
    name: string;
    x: number;
    y: number;
    moved: boolean;
  } | null>(null);
  const [drag, setDrag] = useState<{
    name: string;
    x: number;
    y: number;
    over: string | null;
  } | null>(null);
  // The click that ends a drag must not also toggle the filter.
  const swallowClick = useRef(false);

  const dropOK = (dragged: string, over: string | null) =>
    over !== null &&
    (over === TOP
      ? parents.has(dragged)
      : over !== dragged && !inBranch(parents, over, dragged) && parents.get(dragged) !== over);

  useEffect(() => {
    const target = (x: number, y: number): string | null => {
      const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-kw-drop]");
      return el?.dataset.kwDrop ?? null;
    };
    const onMove = (e: PointerEvent) => {
      const p = press.current;
      if (!p) return;
      if (!p.moved && Math.hypot(e.clientX - p.x, e.clientY - p.y) < DRAG_SLOP) return;
      p.moved = true;
      setDrag({
        name: p.name,
        x: e.clientX,
        y: e.clientY,
        over: target(e.clientX, e.clientY),
      });
    };
    const onUp = (e: PointerEvent) => {
      const p = press.current;
      press.current = null;
      if (!p?.moved) return;
      swallowClick.current = true;
      window.setTimeout(() => (swallowClick.current = false), 0);
      setDrag(null);
      const over = target(e.clientX, e.clientY);
      if (dropOK(p.name, over)) void moveKeyword(p.name, over === TOP ? null : over);
    };
    const onCancel = () => {
      press.current = null;
      setDrag(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
    };
  });

  // --- Rows ----------------------------------------------------------------

  const renderRow = ({ row, depth }: { row: KeywordRow; depth: number }) => {
    const on = chosen.includes(row.name);
    const out = excluded.includes(row.name);
    const branch = row.children.length > 0;
    const open = isOpen(row.name);
    const n = have.get(row.name) ?? 0;
    const all = selIds.length > 0 && n === selIds.length;
    const target = drag !== null && drag.over === row.name;
    const valid = target && dropOK(drag.name, row.name);
    return (
      <div
        key={row.name}
        role="treeitem"
        aria-level={depth + 1}
        aria-expanded={branch ? open : undefined}
        aria-selected={on}
        data-kw-drop={row.name}
        className={`group/kw relative flex items-center rounded-sm ${
          target ? (valid ? "bg-accent-dim/40 ring-1 ring-accent" : "") : ""
        } ${drag?.name === row.name ? "opacity-50" : ""}`}
        style={{ paddingLeft: depth * INDENT }}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ name: row.name, x: e.clientX, y: e.clientY });
        }}
      >
        {branch ? (
          <button
            onClick={() => setRailExpanded(`kw:${row.name}`, !open)}
            className="grid h-6 w-4 shrink-0 place-items-center text-faint hover:text-ink"
            aria-label={open ? t("filters.kw.close", { name: tagName(row.name) }) : t("filters.kw.open", { name: tagName(row.name) })}
          >
            <Icon name="chevron" size={11} className={`transition-transform ${open ? "" : "-rotate-90"}`} />
          </button>
        ) : (
          <span className="w-4 shrink-0" />
        )}
        <button
          onPointerDown={(e) => {
            if (e.button === 0)
              press.current = {
                name: row.name,
                x: e.clientX,
                y: e.clientY,
                moved: false,
              };
          }}
          onClick={(e) => {
            if (!swallowClick.current) clickKeyword(row.name, e.altKey);
          }}
          className={`facet-row min-w-0 flex-1 pl-1 group-hover/kw:bg-hover group-hover/kw:text-ink ${on || out ? "facet-row-active" : ""}`}
          title={`${pathOf(parents, row.name).map(tagName).join(" › ")} — ${
            out ? t("filters.facet.excludedTitle") : t("filters.facet.excludeTitle")
          }`}
        >
          <span
            className={`grid h-3.5 w-3.5 shrink-0 place-items-center ${out ? "text-danger" : "text-accent"}`}
          >
            {out ? (
              <Icon name="ban" size={12} strokeWidth={2.2} />
            ) : (
              on && <Icon name="check" size={13} strokeWidth={2.5} />
            )}
          </span>
          <span
            className={`truncate ${out ? "text-faint line-through" : row.count === 0 && !on ? "text-faint" : ""}`}
          >
            {sentenceCase(tagName(row.name))}
          </span>
          <span className="ml-auto shrink-0 text-label tabular-nums text-faint group-hover/kw:invisible">
            {out ? "–" : formatNumber(row.count)}
          </span>
        </button>
        {selIds.length > 0 && (
          <button
            onClick={() => void setKeywordOn(selIds, row.name, !all)}
            className={`grid h-6 w-5 shrink-0 place-items-center transition-opacity ${
              // Shown whenever there is a selection, not only on hover:
              // reaching for the keyword's name to tag photos filtered by it.
              n > 0 ? "text-accent" : "text-faint"
            }`}
            title={
              all
                ? t("filters.kw.takeOffTitle", { name: tagName(row.name), count: selIds.length })
                : t("filters.kw.putOnTitle", { name: tagName(row.name), count: selIds.length })
            }
            aria-label={
              all
                ? t("filters.kw.takeOffAria", { name: tagName(row.name) })
                : t("filters.kw.putOnAria", { name: tagName(row.name) })
            }
            aria-pressed={all ? true : n > 0 ? "mixed" : false}
          >
            <span
              className={`grid h-3 w-3 place-items-center rounded-[3px] border ${
                n > 0 ? "border-accent bg-accent text-on-accent" : "border-current"
              }`}
            >
              {all ? (
                <Icon name="check" size={9} strokeWidth={3} />
              ) : n > 0 ? (
                <Icon name="minus" size={9} strokeWidth={3} />
              ) : null}
            </span>
          </button>
        )}
        <button
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setMenu({ name: row.name, x: r.left, y: r.bottom });
          }}
          // Over the count while the row is hovered, not beside it: holding
          // its place at rest set every keyword's count 20px in from the
          // other facets'. Left of the tick box when there is a selection.
          className={`absolute top-1/2 grid h-6 w-7 -translate-y-1/2 place-items-center rounded-sm bg-hover text-faint opacity-0 transition-opacity hover:text-ink focus-visible:opacity-100 group-hover/kw:opacity-100 ${
            selIds.length > 0 ? "right-5" : "right-0"
          }`}
          title={t("filters.kw.actions")}
          aria-label={t("filters.kw.actionsFor", { name: tagName(row.name) })}
        >
          <Icon name="more" size={13} />
        </button>
      </div>
    );
  };

  const total = tree.length;

  return (
    <div role="tree" aria-label={t("filters.section.keywords")}>
      {(total > LIMIT || searching) && (
        <div className="px-1 pb-1">
          <input
            className="input h-7 w-full text-caption"
            placeholder={t("filters.kw.find")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && query) {
                e.stopPropagation();
                setQuery("");
              }
            }}
            aria-label={t("filters.kw.findLabel")}
          />
        </div>
      )}
      {chosen.length > 1 && (
        // How the chosen keywords combine: photos with any of them, or
        // only those with every one.
        <div className="flex items-center gap-2 px-2 pb-1 text-label text-faint" role="group" aria-label={t("filters.kw.match")}>
          <span>{t("filters.kw.photosWith")}</span>
          <div className="flex overflow-hidden rounded-sm border border-line">
            {([false, true] as const).map((all) => (
              <button
                key={String(all)}
                onClick={() => setFilter({ ...filter, subjectsAll: all })}
                className={`px-1.5 py-0.5 ${matchAll === all ? "bg-accent-dim/40 text-ink" : "hover:bg-hover hover:text-ink"}`}
                aria-pressed={matchAll === all}
              >
                {all ? t("filters.kw.all") : t("filters.kw.any")}
              </button>
            ))}
          </div>
          <span>{t("filters.kw.ofThem")}</span>
        </div>
      )}
      {rows.length === 0 ? (
        <div className="px-2 py-1 text-caption leading-relaxed text-faint">
          {searching ? t("filters.kw.noMatch") : t("filters.kw.noneYet")}
        </div>
      ) : (
        rows.map(renderRow)
      )}
      {!searching && found.length > LIMIT && (
        <button
          onClick={() => setRailExpanded("subjects", !showAll)}
          className="px-2 py-1 text-caption text-accent hover:underline"
        >
          {showAll ? t("filters.kw.showLess") : t("filters.kw.showMore", { count: found.length - LIMIT })}
        </button>
      )}
      {drag && parents.has(drag.name) ? (
        <div
          data-kw-drop={TOP}
          className={`mx-1 mt-1 rounded-sm border border-dashed px-2 py-1.5 text-caption ${
            drag.over === TOP ? "border-accent bg-accent-dim/40 text-ink" : "border-line text-faint"
          }`}
        >
          {t("filters.kw.dropTop")}
        </div>
      ) : (
        <button
          onClick={() => void newKeyword(null)}
          className="block px-2 py-1 text-caption text-accent hover:underline"
        >
          {t("filters.kw.newButton")}
        </button>
      )}
      {drag &&
        createPortal(
          // What is being carried, beside the pointer. Portalled out of the
          // rail: its sections animate with transforms, which would move a
          // fixed element along with them.
          <div
            className="pointer-events-none fixed z-[70] rounded-sm bg-raised px-2 py-0.5 text-caption text-ink shadow-lg"
            style={{ left: drag.x + 12, top: drag.y + 8 }}
          >
            {sentenceCase(tagName(drag.name))}
          </div>,
          document.body,
        )}
      {menu &&
        createPortal(
          <KeywordMenu
            menu={menu}
            filed={parents.has(menu.name)}
            leftOut={excluded.includes(menu.name)}
            selectionSize={selIds.length}
            selectionHas={(have.get(menu.name) ?? 0) > 0}
            close={closeMenu}
            actions={{
              newInside: () => void newKeyword(menu.name),
              rename: () => void rename(menu.name),
              moveInto: () => void moveInto(menu.name),
              moveTop: () => void moveKeyword(menu.name, null),
              remove: () => void remove(menu.name),
              putOn: () => void setKeywordOn(selIds, menu.name, true),
              takeOff: () => void setKeywordOn(selIds, menu.name, false),
              leaveOut: () => clickKeyword(menu.name, true),
            }}
          />,
          document.body,
        )}
    </div>
  );
}

function KeywordMenu({
  menu,
  filed,
  leftOut,
  selectionSize,
  selectionHas,
  close,
  actions,
}: {
  menu: Menu;
  filed: boolean;
  leftOut: boolean;
  selectionSize: number;
  selectionHas: boolean;
  close: () => void;
  actions: Record<
    "newInside" | "rename" | "moveInto" | "moveTop" | "remove" | "putOn" | "takeOff" | "leaveOut",
    () => void
  >;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  useEscape(true, close);

  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("wheel", close, { passive: true });
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("wheel", close);
      window.removeEventListener("resize", close);
    };
  }, [close]);

  // Kept inside the window, as the photo menu is.
  useLayoutEffect(() => {
    if (!ref.current) return;
    const r = ref.current.getBoundingClientRect();
    const left = menu.x + r.width > window.innerWidth - 8 ? Math.max(8, menu.x - r.width) : menu.x;
    const top = menu.y + r.height > window.innerHeight - 8 ? Math.max(8, menu.y - r.height) : menu.y;
    setPos({ left, top });
  }, [menu]);

  useEffect(() => {
    if (pos) ref.current?.querySelector<HTMLButtonElement>("[role=menuitem]")?.focus();
  }, [pos]);

  const items: ({ label: string; act: () => void; danger?: boolean } | "sep")[] = [
    ...(selectionSize > 0
      ? [
          { label: t("filters.kw.menuPutOn", { count: selectionSize }), act: actions.putOn },
          ...(selectionHas ? [{ label: t("filters.kw.menuTakeOff", { count: selectionSize }), act: actions.takeOff }] : []),
          "sep" as const,
        ]
      : []),
    { label: leftOut ? t("filters.kw.menuLetBack") : t("filters.kw.menuLeaveOut"), act: actions.leaveOut },
    "sep",
    { label: t("filters.kw.menuNewInside"), act: actions.newInside },
    { label: t("filters.kw.menuRename"), act: actions.rename },
    { label: t("filters.kw.menuMoveInto"), act: actions.moveInto },
    ...(filed ? [{ label: t("filters.kw.menuMoveTop"), act: actions.moveTop }] : []),
    "sep",
    { label: t("filters.kw.menuDelete"), act: actions.remove, danger: true },
  ];

  return (
    <div
      ref={ref}
      role="menu"
      // An overlay, as the photo menu is: the grid's rating and flag
      // keys stand aside while it is up rather than acting on the selection.
      data-overlay
      aria-label={t("filters.kw.menuLabel", { name: tagName(menu.name) })}
      className="anim-pop fixed z-[60] min-w-48 rounded-md bg-raised py-1 shadow-2xl"
      style={{
        left: pos?.left ?? menu.x,
        top: pos?.top ?? menu.y,
        visibility: pos ? "visible" : "hidden",
      }}
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
        e.preventDefault();
        const list = [...(ref.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]") ?? [])];
        const i = list.indexOf(document.activeElement as HTMLButtonElement);
        const next = e.key === "ArrowDown" ? (i + 1) % list.length : (i - 1 + list.length) % list.length;
        list[next]?.focus();
      }}
    >
      {items.map((it, i) =>
        it === "sep" ? (
          <div key={i} className="my-1 h-px bg-line" role="separator" />
        ) : (
          <button
            key={i}
            role="menuitem"
            onClick={() => {
              close();
              it.act();
            }}
            className={`flex w-full items-center px-3 py-1.5 text-left text-caption hover:bg-hover focus:bg-hover focus:outline-none ${
              it.danger ? "text-danger-ink" : "text-ink"
            }`}
          >
            {it.label}
          </button>
        ),
      )}
    </div>
  );
}
