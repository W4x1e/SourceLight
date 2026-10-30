import { useMemo, useState } from "react";
import { t } from "../i18n";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { useEscape } from "../lib/useEscape";
import { KEEPERS, resolveAll, type Keeper } from "../lib/resolveDuplicates";

/**
 * Resolve all: settle every duplicate group in the review at once, by
 * one rule — keep the raw, the largest, the best or the first imported, and
 * reject the rest. It says exactly what it will do before it does it, and it
 * is one change, so one Undo takes it back. Nothing is deleted.
 */
export default function ResolveDuplicatesDialog({ onClose }: { onClose: () => void }) {
  const sets = useStore((s) => s.compare?.sets ?? []);
  const resolve = useStore((s) => s.resolveDuplicates);
  const [keeper, setKeeper] = useState<Keeper>("raw");
  const [skipDecided, setSkipDecided] = useState(true);
  const [pickKeepers, setPickKeepers] = useState(true);
  const [busy, setBusy] = useState(false);
  useEscape(true, onClose);

  const opts = { keeper, skipDecided, pickKeepers };
  const plan = useMemo(() => resolveAll(sets, { keeper, skipDecided, pickKeepers }), [sets, keeper, skipDecided, pickKeepers]);

  const apply = async () => {
    setBusy(true);
    await resolve(opts);
    onClose();
  };

  return (
    <div
      className="dialog-scrim anim-scrim fixed inset-0 z-[60] grid place-items-center bg-scrim p-8"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-label={t("viewer.resolve.label")}
        className="anim-card flex max-h-[min(640px,calc(100vh-4rem))] w-full max-w-md flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-4 py-3.5">
          <div className="min-w-0">
            <h2 className="font-display text-display text-ink">{t("viewer.resolve.title")}</h2>
            <p className="text-caption text-faint">{t("viewer.resolve.groups", { count: sets.length })}</p>
          </div>
          <button onClick={onClose} className="icon-chip ml-auto shrink-0" title={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex-1 space-y-4 overflow-y-auto px-4 py-4">
          <fieldset>
            <legend className="section-label mb-1.5 px-0">{t("viewer.resolve.keepEach")}</legend>
            <div className="space-y-1" role="radiogroup" aria-label={t("viewer.resolve.keep")}>
              {KEEPERS.map((k) => (
                <label
                  key={k.id}
                  className={`flex cursor-default items-start gap-2 rounded-sm px-2 py-1.5 ${
                    keeper === k.id ? "bg-accent-dim/25" : "hover:bg-hover"
                  }`}
                >
                  <input
                    type="radio"
                    name="keeper"
                    className="mt-0.5"
                    checked={keeper === k.id}
                    onChange={() => setKeeper(k.id)}
                  />
                  <span>
                    <span className="block text-caption text-ink">{t(k.label)}</span>
                    <span className="block text-label text-faint">{t(k.blurb)}</span>
                  </span>
                </label>
              ))}
            </div>
          </fieldset>

          <div className="space-y-1.5">
            <label className="flex items-center gap-2 text-caption text-muted">
              <input type="checkbox" checked={skipDecided} onChange={(e) => setSkipDecided(e.target.checked)} />
              {t("viewer.resolve.skipDecided")}
            </label>
            <label className="flex items-center gap-2 text-caption text-muted">
              <input type="checkbox" checked={pickKeepers} onChange={(e) => setPickKeepers(e.target.checked)} />
              {t("viewer.resolve.pickKeepers")}
            </label>
          </div>

          <div className="rounded-md bg-raised px-3 py-2 text-caption">
            <div className="text-ink">
              {plan.groups === 0
                ? t("viewer.resolve.nothing")
                : `${t("viewer.resolve.rejects", { count: plan.rejects })} ${t("viewer.resolve.inGroups", { count: plan.groups })}`}
            </div>
            {plan.skipped > 0 && (
              <div className="text-label text-faint">{t("viewer.resolve.skipped", { count: plan.skipped })}</div>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 border-t border-line px-4 py-3">
          <span className="text-label text-faint">{t("viewer.resolve.oneChange")}</span>
          <button onClick={onClose} className="btn-ghost ml-auto">
            {t("common.cancel")}
          </button>
          <button onClick={() => void apply()} className="btn-accent" disabled={plan.changes.length === 0 || busy}>
            {t("viewer.resolve.apply", { count: plan.groups })}
          </button>
        </div>
      </div>
    </div>
  );
}
