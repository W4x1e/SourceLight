import { t } from "../../i18n";
import { hint } from "./shortcutHint";

interface Props {
  copySettings: () => Promise<void>;
  pasteSettings: () => Promise<void>;
  pastePrevious: () => Promise<void>;
  pasteToSelection: () => Promise<void>;
  /** Something has been copied to paste. */
  hasCopied: boolean;
  /** The photo open before this one, for Previous. */
  previousId: number | null;
  /** Photos selected in the library besides this one. */
  selectedOthers: number;
  copyKey: string;
  pasteKey: string;
}

/**
 * Settings that travel between photos, at the foot of the rail where
 * photographers expect Previous and Reset: always here, always in the
 * same place. Paste previous used to appear in the header only
 * once a second photo had been opened, shoving every button beside
 * it along; Copy to selected sat in the old Looks panel as a second
 * key.
 */
export default function CopyPasteBar({
  copySettings,
  pasteSettings,
  pastePrevious,
  pasteToSelection,
  hasCopied,
  previousId,
  selectedOthers,
  copyKey,
  pasteKey,
}: Props) {
  return (
    <div className="flex items-center gap-1 border-t border-line p-2">
      <button
        onClick={() => void copySettings()}
        className="btn-ghost flex-1 px-1.5 text-muted hover:text-ink"
        title={hint(t("develop.copy.copyTitle"), copyKey)}
      >
        {t("develop.copy.copy")}
      </button>
      <button
        onClick={() => void pasteSettings()}
        disabled={!hasCopied}
        className="btn-ghost flex-1 px-1.5 text-muted hover:text-ink disabled:pointer-events-none disabled:opacity-40"
        title={hint(t("develop.copy.pasteTitle"), pasteKey)}
      >
        {t("develop.copy.paste")}
      </button>
      <button
        onClick={() => void pastePrevious()}
        disabled={previousId == null}
        className="btn-ghost flex-1 px-1.5 text-muted hover:text-ink disabled:pointer-events-none disabled:opacity-40"
        title={t("develop.copy.previousTitle")}
      >
        {t("develop.copy.previous")}
      </button>
      {selectedOthers > 0 && (
        <button
          onClick={() => void pasteToSelection()}
          className="btn-solid flex-1 px-1.5"
          title={t("develop.copy.toSelectedTitle", { count: selectedOthers })}
        >
          {t("develop.copy.toSelected", { count: selectedOthers })}
        </button>
      )}
    </div>
  );
}
