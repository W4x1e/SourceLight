import { LOCALES, getLocale, t, type LanguagePref } from "../i18n";
import { useStore } from "../store";

/**
 * The interface language (Settings › About). Chips rather than a native
 * `<select>`, for the reason `AddToCollection` gives: the webview's native
 * dropdowns fire inconsistently. Each language is named in itself, which is
 * how someone who cannot read the current one finds theirs.
 */
export default function LanguageBox() {
  const language = useStore((s) => s.language);
  const setLanguage = useStore((s) => s.setLanguage);

  const options: { pref: LanguagePref; label: string }[] = [
    { pref: "system", label: t("settings.language.system") },
    ...LOCALES.map((l) => ({ pref: l.code, label: l.name })),
    // Every string accented and padded, to find one still hard-coded.
    ...(import.meta.env.DEV ? [{ pref: "en-XA" as const, label: "Pseudo" }] : []),
  ];

  return (
    <div className="rounded-md border border-line bg-raised p-3">
      <div className="text-caption font-medium text-ink">{t("settings.language.title")}</div>
      <div className="mt-2 flex flex-wrap gap-1.5" role="radiogroup" aria-label={t("settings.language.title")}>
        {options.map((o) => (
          <button
            key={o.pref}
            role="radio"
            aria-checked={language === o.pref}
            lang={o.pref === "system" ? undefined : o.pref}
            onClick={() => void setLanguage(o.pref)}
            // The chosen one in the app's one "this is on" treatment, as every
            // other switch in Settings shows it, not an outline of its own.
            className={`rounded-sm px-2.5 py-1 text-caption transition-colors ${
              language === o.pref ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
            }`}
          >
            {o.label}
          </button>
        ))}
      </div>
      {getLocale() !== "en" && getLocale() !== "en-XA" && (
        <p className="mt-2 text-label leading-relaxed text-faint">{t("settings.language.machine")}</p>
      )}
    </div>
  );
}
