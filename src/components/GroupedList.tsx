import { useState } from "react";
import { formatNumber, t } from "../i18n";
import { useStore } from "../store";
import { confirm, prompt } from "../lib/dialogs";
import { Icon } from "../lib/icons";

/** What a drag carries between rows of the rail. */
const DRAG_TYPE = "application/x-sourcelight-collection";

/**
 * Collections, or saved views, in their groups: each group a folder
 * that opens and closes, with what is in it indented beneath, then everything
 * at the top level. Drag a row onto a folder to file it there; drag it onto
 * the top level to take it out. Groups are shared by both lists.
 */
export default function GroupedList<T extends { id: number; groupId?: number | null }>({
  items,
  smart,
  renderItem,
}: {
  items: T[];
  /** Saved views rather than collections. */
  smart: boolean;
  renderItem: (item: T) => React.ReactNode;
}) {
  const groups = useStore((s) => s.collectionGroups);
  const setGroup = useStore((s) => s.setCollectionGroup);
  const renameGroup = useStore((s) => s.renameCollectionGroup);
  const deleteGroup = useStore((s) => s.deleteCollectionGroup);
  const [closed, setClosed] = useState<Set<number>>(new Set());
  const [over, setOver] = useState<number | "top" | null>(null);

  const dragged = (e: React.DragEvent): number | null => {
    const raw = e.dataTransfer.getData(DRAG_TYPE);
    const [kind, id] = raw.split(":");
    return kind === (smart ? "smart" : "collection") && id ? Number(id) : null;
  };
  const target = (key: number | "top", group: number | null) => ({
    onDragOver: (e: React.DragEvent) => {
      if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
      e.preventDefault();
      e.stopPropagation();
      setOver(key);
    },
    onDragLeave: () => setOver((o) => (o === key ? null : o)),
    onDrop: (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setOver(null);
      const id = dragged(e);
      if (id !== null) void setGroup(id, smart, group);
    },
  });
  const row = (item: T) => (
    <div
      key={item.id}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_TYPE, `${smart ? "smart" : "collection"}:${item.id}`);
        e.dataTransfer.effectAllowed = "move";
      }}
    >
      {renderItem(item)}
    </div>
  );

  const loose = items.filter((i) => i.groupId == null || !groups.some((g) => g.id === i.groupId));
  return (
    <div {...target("top", null)} className={over === "top" ? "rounded-sm bg-hover/60" : undefined}>
      {groups.map((g) => {
        const inside = items.filter((i) => i.groupId === g.id);
        const open = !closed.has(g.id);
        return (
          <div key={g.id} {...target(g.id, g.id)}>
            <div className={`group/grp relative flex items-center rounded-sm ${over === g.id ? "bg-hover" : ""}`}>
              <button
                onClick={() =>
                  setClosed((c) => {
                    const n = new Set(c);
                    if (n.has(g.id)) n.delete(g.id);
                    else n.add(g.id);
                    return n;
                  })
                }
                className="facet-row min-w-0 flex-1 group-hover/grp:bg-hover group-hover/grp:text-ink"
                aria-expanded={open}
                title={smart ? t("filters.group.dropViews") : t("filters.group.dropCollections")}
              >
                <Icon name="chevron" size={12} className={`shrink-0 text-faint transition-transform ${open ? "" : "-rotate-90"}`} />
                <Icon name="folder" size={13} className="shrink-0 text-faint" />
                <span className="truncate">{g.name}</span>
                <span className="ml-auto shrink-0 text-label tabular-nums text-faint group-hover/grp:invisible">
                  {formatNumber(inside.length)}
                </span>
              </button>
              {/* Over the count while hovered, as on a collection's row, so
                  the counts down the list line up. */}
              <div className="absolute inset-y-0 right-0 flex items-center rounded-r-sm bg-hover pl-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover/grp:opacity-100">
              <button
                onClick={() =>
                  void prompt({ title: t("filters.group.rename"), defaultValue: g.name, confirmLabel: t("common.rename") }).then(
                    (name) => name && name.trim() !== g.name && void renameGroup(g.id, name),
                  )
                }
                className="px-1 text-faint hover:text-ink"
                title={t("filters.group.rename")}
                aria-label={t("filters.group.renameAria", { name: g.name })}
              >
                <Icon name="pencil" size={12} />
              </button>
              <button
                onClick={() =>
                  void confirm({
                    title: t("filters.group.deleteTitle", { name: g.name }),
                    message: t("filters.group.deleteMessage"),
                    confirmLabel: t("filters.group.delete"),
                    danger: true,
                  }).then((ok) => ok && void deleteGroup(g.id))
                }
                className="px-1.5 text-faint hover:text-danger"
                title={t("filters.group.delete")}
                aria-label={t("filters.group.deleteAria", { name: g.name })}
              >
                <Icon name="close" size={12} />
              </button>
              </div>
            </div>
            {open && inside.length > 0 && <div className="pl-4">{inside.map(row)}</div>}
          </div>
        );
      })}
      {loose.map(row)}
    </div>
  );
}
