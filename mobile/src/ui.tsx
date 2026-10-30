import { useEffect, useRef, useState, type ReactNode } from "react";
import { t } from "../../src/i18n";
import { Icon, type IconName } from "../../src/lib/icons";

/**
 * Small pieces the phone's screens share, in the desktop's design
 * (DESIGN.md) at a finger's size: 48 px targets, the same icon set, no hue in
 * the chrome.
 */

/** The loupe sits beside the grid instead of over it on a screen at least
 *  this wide whose shorter side is at least `tablet`: a tablet, not a
 *  phone on its side. The shorter side is the screen's, so a keyboard
 *  coming up doesn't change the layout. */
const TWO_PANE = { width: 900, tablet: 600 };

/** The window's width, and whether the grid and the loupe go side by side;
 *  both follow rotation. */
export function useScreen() {
  const [width, setWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  const tablet = Math.min(window.screen.width, window.screen.height) >= TWO_PANE.tablet;
  return { width, twoPane: tablet && width >= TWO_PANE.width };
}

/** A square icon button with a 48 px target; `label` is what it says to a
 *  screen reader and on a long press. */
export function IconButton({
  icon,
  label,
  onClick,
  disabled = false,
  className = "",
}: {
  icon: IconName;
  label: string;
  /** Given the click, so a button can open a menu under itself. */
  onClick: (e: React.MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <button
      // Never a form's submit: the search bar's Close sits in its form.
      type="button"
      className={`flex h-12 w-12 shrink-0 items-center justify-center rounded-md text-ink active:bg-raised disabled:opacity-40 ${className}`}
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon name={icon} size={22} />
    </button>
  );
}

/** The top bar of a full-screen panel opened over a screen: Back on the
 *  left, then its title. */
export function PanelHeader({ title, onClose }: { title: string; onClose: () => void }) {
  return (
    <div className="flex min-h-14 items-center gap-1 border-b border-line px-1 pt-[var(--safe-top)]">
      <IconButton icon="back" label={t("common.back")} onClick={onClose} />
      <div className="min-w-0 flex-1 truncate px-1 font-display text-title text-ink">{title}</div>
    </div>
  );
}

/** What a screen shows when it has nothing to show, in the middle of the
 *  space: an icon, a line saying so, and maybe a hint and an action. */
export function EmptyState({
  icon,
  title,
  hint,
  children,
}: {
  icon: IconName;
  title: string;
  hint?: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 px-8 pb-12 text-center">
      <Icon name={icon} size={36} strokeWidth={1.4} className="mb-2 text-faint" />
      <p className="text-body text-ink">{title}</p>
      {hint && <p className="max-w-xs text-caption leading-relaxed text-muted">{hint}</p>}
      {children}
    </div>
  );
}

/** A round status light: green when all is well, amber while waiting, red
 *  when something is wrong (the only colours the chrome uses). */
export function StatusDot({ tone }: { tone: "positive" | "warning" | "danger" }) {
  const colour = tone === "positive" ? "bg-positive" : tone === "warning" ? "bg-warning" : "bg-danger";
  return <span aria-hidden className={`inline-block h-2 w-2 shrink-0 rounded-full ${colour}`} />;
}

/** How far, in px, a sheet must be pulled down to close. */
const SHEET_CLOSE = 80;

/** A sheet rising from the bottom over a scrim, as iOS's half-height sheet
 *  and Android's modal bottom sheet: tapping the scrim, or pulling the sheet
 *  down by its grabber, closes it. Whoever opens it gives it a Back step. On
 *  a tablet it keeps a phone's width, centred. */
export function Sheet({ label, onClose, children }: { label: string; onClose: () => void; children: ReactNode }) {
  const from = useRef<number | null>(null);
  const [dy, setDy] = useState(0);
  const end = () => {
    if (dy > SHEET_CLOSE) onClose();
    from.current = null;
    setDy(0);
  };
  return (
    <div className="fixed inset-0 z-50" onClick={onClose}>
      <div aria-hidden className="fade-in absolute inset-0 bg-scrim" />
      <div
        role="dialog"
        aria-label={label}
        className="sheet-in absolute inset-x-0 bottom-0 mx-auto flex max-h-[85%] w-full max-w-lg flex-col rounded-t-md bg-panel pb-[var(--safe-bottom)] shadow-2xl"
        style={dy > 0 ? { transform: `translateY(${dy}px)` } : undefined}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className="flex shrink-0 touch-none justify-center pt-2 pb-2"
          onPointerDown={(e) => {
            (e.target as Element).setPointerCapture(e.pointerId);
            from.current = e.clientY;
          }}
          onPointerMove={(e) => from.current !== null && setDy(Math.max(0, e.clientY - from.current))}
          onPointerUp={end}
          onPointerCancel={end}
        >
          <span aria-hidden className="h-1 w-9 rounded-full bg-dim" />
        </div>
        <div className="min-h-0 overflow-y-auto">{children}</div>
      </div>
    </div>
  );
}

/** A question in a [`Sheet`] before something can't be taken back, as
 *  Discard changes: the destructive choice in the danger triad, and a way
 *  out. */
export function ConfirmSheet({
  title,
  message,
  confirm,
  cancel,
  onConfirm,
  onCancel,
}: {
  title: string;
  message?: string;
  confirm: string;
  cancel: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <Sheet label={title} onClose={onCancel}>
      <div className="px-5 pt-2 pb-5">
        <h2 className="font-display text-title text-ink">{title}</h2>
        {message && <p className="mt-1 text-caption leading-relaxed text-muted">{message}</p>}
        <div className="mt-5 flex flex-col gap-2">
          <button className="btn-solid btn-danger min-h-12 text-body" onClick={onConfirm}>
            {confirm}
          </button>
          <button className="btn-ghost min-h-12 text-body" onClick={onCancel}>
            {cancel}
          </button>
        </div>
      </div>
    </Sheet>
  );
}

/** A row of a menu or [`Sheet`]: an icon and what it does. A destructive
 *  one is in the danger ink. */
export function MenuItem({
  icon,
  label,
  onClick,
  danger = false,
}: {
  icon: IconName;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      className={`flex min-h-14 w-full shrink-0 items-center gap-6 px-5 text-left text-body active:bg-raised ${danger ? "text-danger-ink" : "text-ink"}`}
      onClick={onClick}
    >
      <Icon name={icon} size={22} className={danger ? "" : "text-muted"} />
      {label}
    </button>
  );
}

/** A small heading over a group of [`MenuItem`]s, with a line above it. */
export function MenuHeading({ children }: { children: ReactNode }) {
  return <div className="mt-2 shrink-0 border-t border-line px-5 pt-4 pb-1 text-label text-faint">{children}</div>;
}

/** A list of choices dropping down from the control that opened it, at
 *  `anchor` (its box on screen). Tapping outside closes it. */
export function Dropdown({
  anchor,
  align = "left",
  onClose,
  children,
}: {
  anchor: DOMRect;
  /** Which of the control's edges the list lines up with. */
  align?: "left" | "right";
  onClose: () => void;
  children: ReactNode;
}) {
  const side =
    align === "left" ? { left: Math.max(8, anchor.left) } : { right: Math.max(8, window.innerWidth - anchor.right) };
  return (
    <div className="fixed inset-0 z-50" onClick={onClose}>
      <div
        role="menu"
        className="fade-in absolute max-h-[60vh] max-w-[calc(100vw-1rem)] min-w-56 overflow-y-auto rounded-md border border-line bg-panel py-1 shadow-2xl"
        style={{ top: anchor.bottom + 4, ...side }}
        onClick={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}

/** One choice in a [`Dropdown`], ticked when it is the current one, with a
 *  count or other detail at the end. */
export function Choice({
  label,
  detail,
  selected,
  onClick,
}: {
  label: string;
  detail?: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      role="menuitemradio"
      aria-checked={selected}
      className="flex min-h-12 w-full items-center gap-3 pr-4 pl-3 text-left text-body text-ink active:bg-raised"
      onClick={onClick}
    >
      <span className="flex w-5 shrink-0 justify-center">{selected && <Icon name="check" size={18} />}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {detail && <span className="shrink-0 font-mono text-caption tabular-nums text-faint">{detail}</span>}
    </button>
  );
}

/** A screen's large title. With `onOpen`, it opens a menu of what the
 *  screen shows, and a chevron says so. */
export function ScreenTitle({ title, onOpen }: { title: string; onOpen?: (anchor: DOMRect) => void }) {
  if (!onOpen) return <h1 className="min-w-0 truncate py-1.5 font-display text-headline text-ink">{title}</h1>;
  return (
    <button
      className="-ml-2 flex min-h-12 min-w-0 items-center gap-1 rounded-md px-2 text-ink active:bg-raised"
      aria-haspopup="menu"
      onClick={(e) => onOpen(e.currentTarget.getBoundingClientRect())}
    >
      <span className="truncate font-display text-headline">{title}</span>
      <Icon name="chevron" size={22} className="shrink-0 text-muted" />
    </button>
  );
}
