import { type ReactNode } from "react";
import { Icon } from "../../lib/icons";
import { inSentence, t } from "../../i18n";

interface Props {
  title: string;
  open: boolean;
  onToggle: () => void;
  /** Shows a dot and a reset control. */
  modified?: boolean;
  onReset?: () => void;
  children: ReactNode;
}

/**
 * One collapsible group in the adjustment column.
 *
 * The open state is owned by the parent rather than held here, because "one
 * at a time" has to close the others when one opens — a decision no single
 * panel can make about its siblings.
 *
 * The heading is the group's name at title size, a step above the slider
 * labels under it, so the column reads as sections rather than as one long
 * list of equal-weight words. The toggle is a real button (it used to be a
 * clickable div, which the keyboard could not reach), and Reset sits beside
 * it rather than inside it. Reset shows on hover or keyboard focus, where it
 * used to be an invisible lowercase word only a mouse could find.
 */
export default function CollapsiblePanel({
  title,
  open,
  onToggle,
  modified,
  onReset,
  children,
}: Props) {
  return (
    <section className="border-b border-line">
      <div className="group flex h-9 items-center pr-2 hover:bg-hover">
        <button
          onClick={onToggle}
          aria-expanded={open}
          className="flex h-full min-w-0 flex-1 items-center gap-2 pl-3 text-left focus-visible:outline-offset-[-2px]"
        >
          <Icon
            name="chevron"
            size={13}
            className={`shrink-0 text-faint transition-transform ${open ? "" : "-rotate-90"}`}
          />
          <span className="truncate text-title font-semibold text-ink">{title}</span>
          {modified && (
            <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" title={t("develop.panel.changed")} />
          )}
        </button>
        {modified && onReset && (
          <button
            onClick={onReset}
            className="rounded-sm px-1.5 py-0.5 text-label text-faint opacity-0 transition-opacity hover:bg-raised hover:text-ink focus-visible:opacity-100 group-hover:opacity-100"
            title={t("develop.panel.resetTitle", { name: inSentence(title) })}
          >
            {t("develop.panel.reset")}
          </button>
        )}
      </div>
      {open && <div className="px-3 pb-3 pt-0.5">{children}</div>}
    </section>
  );
}
