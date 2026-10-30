import { useEffect, useRef } from "react";
import { t } from "../i18n";

/**
 * A long legal text in a scrolling card — SourceLight's licence and the
 * third-party licences. Plain pre-wrapped text: both are plain-text
 * files, and reflowing them would break the licences' own layout.
 */
export default function LegalDocument({
  title,
  text,
  onClose,
}: {
  title: string;
  text: string | null;
  onClose: () => void;
}) {
  // Modal to the keyboard too: focus starts on Close and Tab stays
  // inside. The first Tab used to land on the screen behind it.
  const card = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const buttons = card.current?.querySelectorAll<HTMLButtonElement>("button");
    buttons?.[buttons.length - 1]?.focus();
  }, []);
  const trap = (e: React.KeyboardEvent) => {
    if (e.key !== "Tab" || !card.current) return;
    const focusable = Array.from(
      card.current.querySelectorAll<HTMLElement>('button, [href], [tabindex]:not([tabindex="-1"])'),
    ).filter((el) => !el.hasAttribute("disabled"));
    const [first, last] = [focusable[0], focusable[focusable.length - 1]];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    } else if (!card.current.contains(document.activeElement)) {
      e.preventDefault();
      first.focus();
    }
  };
  return (
    <div className="anim-scrim fixed inset-0 z-[70] grid place-items-center bg-scrim p-8">
      <div
        ref={card}
        onKeyDown={trap}
        role="dialog"
        aria-modal="true"
        aria-labelledby="legal-title"
        className="anim-card flex max-h-[80vh] w-full max-w-2xl flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="border-b border-line px-4 py-3">
          <h2
            id="legal-title"
            className="font-display text-display text-ink"
          >
            {title}
          </h2>
        </div>
        <pre className="min-h-0 flex-1 overflow-y-auto whitespace-pre-wrap px-4 py-3 font-mono text-label leading-relaxed text-muted">
          {text ?? t("common.loading")}
        </pre>
        <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-3">
          <button onClick={onClose} className="btn-solid">
            {t("common.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
