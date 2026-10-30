import { useEffect, useRef, useState } from "react";
import { Icon } from "../lib/icons";
import { formatDate, t } from "../i18n";
import { monthGrid, parseISO, shiftMonth, toISO } from "../lib/calendar";
import { useEscape } from "../lib/useEscape";

/**
 * A small calendar date picker for the date filter — a theme-consistent popover
 * instead of the native `<input type="date">`, whose control chrome doesn't match
 * the dark UI and renders inconsistently in the webview.
 *
 * Value and onChange speak `YYYY-MM-DD` (or null), the shape the filter stores.
 */

// A Sunday, to name the weekday columns from in the active language.
const SUNDAY = new Date(2026, 0, 4);

/** Popover size, matching `w-56` and the six-week calendar plus its chrome. Used
 *  to keep the menu inside the viewport when it is flipped or shifted. */
const MENU_W = 224;
const MENU_H = 268;

interface Props {
  value: string | null;
  onChange: (value: string | null) => void;
  placeholder?: string;
}

export default function DatePicker({ value, onChange, placeholder }: Props) {
  const [open, setOpen] = useState(false);
  // Fixed viewport coordinates for the popover. The menu is positioned relative
  // to the viewport, not the button, so it can escape the filter rail's
  // `overflow-y-auto` clip (which would otherwise cut its right edge off against
  // the grid and add a horizontal scrollbar to the rail).
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  const selected = parseISO(value);
  // The month on screen. Opens on the selected date's month, else the current one.
  const [view, setView] = useState(() => {
    if (selected) return { year: selected.year, month0: selected.month0 };
    const now = new Date();
    return { year: now.getFullYear(), month0: now.getMonth() };
  });

  // Anchor the popover under the button, flipping above / shifting left so it
  // always stays on screen.
  const place = () => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const margin = 8;
    const left = Math.max(margin, Math.min(r.left, window.innerWidth - MENU_W - margin));
    const below = r.bottom + 4;
    const top =
      below + MENU_H > window.innerHeight - margin
        ? Math.max(margin, r.top - 4 - MENU_H)
        : below;
    setPos({ top, left });
  };

  // On open, jump the view to the selected date, place the menu, and keep it
  // anchored while the rail scrolls or the window resizes under it.
  useEffect(() => {
    if (!open) return;
    if (selected) setView({ year: selected.year, month0: selected.month0 });
    place();
    const reposition = () => place();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Dismiss on outside click, or on Escape as the innermost overlay — so the
  // key closes the calendar and stops there, rather than going on to clear
  // the grid's selection behind it.
  useEscape(open, () => setOpen(false), ref);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const cells = monthGrid(view.year, view.month0);
  const isSelected = (d: number) =>
    selected != null &&
    selected.year === view.year &&
    selected.month0 === view.month0 &&
    selected.day === d;

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() =>
          setOpen((o) => {
            if (!o) place();
            return !o;
          })
        }
        className="input flex items-center justify-between gap-2"
      >
        <span className={value ? "text-ink" : "text-faint"}>{value ?? placeholder ?? t("filters.datePicker.anyDate")}</span>
        <Icon name="calendar" size={13} className="shrink-0 text-faint" />
      </button>

      {open && pos && (
        <div
          data-overlay
          className="anim-pop fixed z-50 w-56 rounded-md border border-line bg-raised p-2 shadow-2xl"
          style={{ top: pos.top, left: pos.left }}
        >
          <div className="mb-1.5 flex items-center justify-between">
            <button
              onClick={() => setView((v) => shiftMonth(v.year, v.month0, -1))}
              className="rounded-sm p-1 text-faint hover:bg-hover hover:text-ink"
              title={t("filters.datePicker.previousMonth")}
            >
              <Icon name="chevron" size={13} className="rotate-90" />
            </button>
            <span className="text-caption font-medium text-ink">
              {formatDate(new Date(view.year, view.month0, 1), { month: "long", year: "numeric" })}
            </span>
            <button
              onClick={() => setView((v) => shiftMonth(v.year, v.month0, 1))}
              className="rounded-sm p-1 text-faint hover:bg-hover hover:text-ink"
              title={t("filters.datePicker.nextMonth")}
            >
              <Icon name="chevron" size={13} className="-rotate-90" />
            </button>
          </div>

          <div className="mb-0.5 grid grid-cols-7 text-center text-label font-medium text-faint">
            {Array.from({ length: 7 }, (_, i) => (
              <div key={i} className="py-0.5">
                {formatDate(new Date(2026, 0, SUNDAY.getDate() + i), { weekday: "narrow" })}
              </div>
            ))}
          </div>
          <div className="grid grid-cols-7 gap-0.5">
            {cells.map((d, i) =>
              d === null ? (
                <div key={i} />
              ) : (
                <button
                  key={i}
                  onClick={() => {
                    onChange(toISO(view.year, view.month0, d));
                    setOpen(false);
                  }}
                  className={`rounded-sm py-1 text-caption tabular-nums transition-colors ${
                    isSelected(d)
                      ? "bg-accent font-semibold text-on-accent"
                      : "text-muted hover:bg-hover hover:text-ink"
                  }`}
                >
                  {d}
                </button>
              ),
            )}
          </div>

          {value && (
            <div className="mt-1.5 border-t border-line/60 pt-1.5 text-right">
              <button
                onClick={() => {
                  onChange(null);
                  setOpen(false);
                }}
                className="text-label text-faint hover:text-ink"
              >
                {t("common.clear")}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
