import { t } from "../i18n";

/** A two-state segmented switch, the app's one segmented-control language. */
export default function OnOff({
  on,
  disabled,
  onChange,
  label,
}: {
  on: boolean;
  disabled?: boolean;
  onChange: (on: boolean) => void;
  label: string;
}) {
  return (
    <div className="seg-group shrink-0" role="radiogroup" aria-label={label}>
      {([false, true] as const).map((v) => (
        <button
          key={String(v)}
          role="radio"
          aria-checked={on === v}
          disabled={disabled}
          onClick={() => on !== v && onChange(v)}
          className={`px-2.5 py-1 text-caption ${on === v ? "seg-on" : "text-muted hover:bg-hover"}`}
        >
          {v ? t("common.on") : t("common.off")}
        </button>
      ))}
    </div>
  );
}
