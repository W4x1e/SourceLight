import { useEffect, useRef, useState } from "react";
import type { Group } from "../../bindings";
import { COPIED_BY_DEFAULT, DEVELOP_GROUPS } from "../../lib/developGroups";
import { useModal } from "../../lib/useEscape";
import { useHeld, usePresence } from "../../lib/usePresence";
import { t } from "../../i18n";

/** A question the dialog is asking, and how it answers. */
export interface GroupsAsk {
  title: string;
  message?: string;
  confirmLabel: string;
  /** Ask for a name too — a preset's. */
  withName?: boolean;
  resolve: (answer: { groups: Group[]; name: string } | null) => void;
}

/**
 * Which parts of an edit to copy, paste or save as a preset. The crop,
 * masks and spots start unticked: they are drawn on this photo's content.
 */
export default function GroupsDialog({ ask: asked }: { ask: GroupsAsk | null }) {
  // The question, held while the card fades out after it is answered.
  const presence = usePresence(asked !== null);
  const ask = useHeld(asked) ?? null;
  const [chosen, setChosen] = useState<Set<Group>>(new Set(COPIED_BY_DEFAULT));
  const [name, setName] = useState("");
  useEffect(() => {
    if (!ask) return;
    setChosen(new Set(COPIED_BY_DEFAULT));
    setName("");
  }, [ask]);
  // A modal. Escape used to do nothing here: the guard against a
  // confirm on top saw this dialog's own scrim, so Develop closed instead.
  const card = useRef<HTMLFormElement>(null);
  useModal(asked !== null, card, () => ask?.resolve(null));
  if (!presence.mounted || !ask) return null;

  const toggle = (g: Group) =>
    setChosen((prev) => {
      const next = new Set(prev);
      if (next.has(g)) next.delete(g);
      else next.add(g);
      return next;
    });
  const all = chosen.size === DEVELOP_GROUPS.length;
  const ready = chosen.size > 0 && (!ask.withName || name.trim() !== "");
  const confirm = () => {
    if (!ready) return;
    // In panel order, whatever order they were ticked in.
    const groups = DEVELOP_GROUPS.map((g) => g.value).filter((g) => chosen.has(g));
    ask.resolve({ groups, name: name.trim() });
  };

  return (
    <div
      data-closing={presence.closing || undefined}
      className="dialog-scrim anim-scrim fixed inset-0 z-[60] grid place-items-center bg-scrim p-6"
      onClick={(e) => e.target === e.currentTarget && ask.resolve(null)}
    >
      {/* A form, so Enter on a ticked box answers as it does in the name
          field: it did nothing there. */}
      <form
        ref={card}
        role="dialog"
        aria-modal="true"
        aria-label={ask.title}
        className="anim-card max-h-[calc(100vh-4rem)] w-[340px] max-w-full overflow-y-auto rounded-lg border border-line bg-panel p-4 shadow-2xl"
        onSubmit={(e) => {
          e.preventDefault();
          confirm();
        }}
      >
        <div className="font-display text-display text-ink">{ask.title}</div>
        {ask.message && <p className="mt-1 text-caption text-faint">{ask.message}</p>}
        {ask.withName && (
          <input
            autoFocus
            className="input mt-3 w-full"
            placeholder={t("develop.groups.presetName")}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        )}
        <div className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1">
          {DEVELOP_GROUPS.map((g) => (
            <label key={g.value} className="flex items-center gap-2 py-0.5 text-caption text-ink">
              <input type="checkbox" checked={chosen.has(g.value)} onChange={() => toggle(g.value)} />
              {t(g.label)}
            </label>
          ))}
        </div>
        <div className="mt-4 flex items-center gap-2">
          {/* With the other buttons, as a button: it was a white underlined
              word floating under the boxes. */}
          <button
            type="button"
            onClick={() => setChosen(all ? new Set() : new Set(DEVELOP_GROUPS.map((g) => g.value)))}
            className="btn-ghost mr-auto px-2 text-caption text-muted hover:text-ink"
          >
            {all ? t("develop.groups.none") : t("develop.groups.all")}
          </button>
          <button type="button" onClick={() => ask.resolve(null)} className="btn-ghost text-caption">
            {t("common.cancel")}
          </button>
          <button type="submit" disabled={!ready} className="btn-accent text-caption">
            {ask.confirmLabel}
          </button>
        </div>
      </form>
    </div>
  );
}
