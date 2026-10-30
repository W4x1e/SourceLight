import { useEffect, useRef, useState } from "react";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { prompt, toast } from "../lib/dialogs";
import { useEscape } from "../lib/useEscape";
import { formatNumber, t } from "../i18n";

/**
 * Add the current selection to a collection.
 *
 * A themed popover rather than a native `<select>`: the webview renders and fires
 * change events on native form dropdowns inconsistently (the same reason the date
 * filter is a custom `DatePicker`), which is why picking a collection often did
 * nothing. This owns its open state, closes on outside-click, and briefly confirms
 * the add so the action isn't silent.
 */
export default function AddToCollection() {
  const collections = useStore((s) => s.collections);
  const createCollection = useStore((s) => s.createCollection);
  const addSelectionToCollection = useStore((s) => s.addSelectionToCollection);

  const [open, setOpen] = useState(false);
  const [added, setAdded] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [open]);
  // Escape closes the list and goes no further: it had no Escape at all, and
  // the key went on to clear the selection the list was about to act on.
  useEscape(open, () => setOpen(false), ref);

  const add = async (id: number, name: string) => {
    setOpen(false);
    try {
      await addSelectionToCollection(id);
    } catch (e) {
      // It failed with no word, and "Added to …" still showed.
      toast(String(e).replace(/^Error: /, ""), "error");
      return;
    }
    setAdded(name);
    window.setTimeout(() => setAdded(null), 1800);
  };

  const makeNew = async () => {
    setOpen(false);
    const name = await prompt({ title: t("library.addToCollection.newTitle"),
      placeholder: t("library.addToCollection.namePlaceholder"),
      confirmLabel: t("library.addToCollection.create"), });
    if (!name) return;
    const id = await createCollection(name);
    if (id) await add(id, name);
  };

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="btn-solid gap-1.5 text-caption"
        title={t("library.addToCollection.tooltip")}
      >
        {added ? t("library.addToCollection.added", { name: added }) : t("library.addToCollection.button")}
        <Icon name="chevron" size={12} className="text-faint" />
      </button>
      {open && (
        <div
          data-overlay
          className="anim-pop pop-up absolute bottom-full left-0 z-20 mb-1 max-h-64 w-52 overflow-y-auto rounded-md border border-line bg-raised py-1 shadow-2xl"
        >
          {collections.length === 0 && (
            <div className="px-3 py-1.5 text-caption text-faint">{t("library.addToCollection.none")}</div>
          )}
          {collections.map((c) => (
            <button
              key={c.id}
              onClick={() => void add(c.id, c.name)}
              className="flex w-full items-center justify-between gap-2 px-3 py-1.5 text-left text-caption text-muted hover:bg-hover hover:text-ink"
            >
              <span className="truncate">{c.name}</span>
              <span className="shrink-0 text-label tabular-nums text-faint">{formatNumber(c.count)}</span>
            </button>
          ))}
          <div className="my-1 border-t border-line/60" />
          <button
            onClick={() => void makeNew()}
            className="w-full px-3 py-1.5 text-left text-caption text-accent hover:bg-hover"
          >
            {t("library.addToCollection.newItem")}
          </button>
        </div>
      )}
    </div>
  );
}
