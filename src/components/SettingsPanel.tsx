import { useCallback, useEffect, useRef, useState } from "react";
import { useModal } from "../lib/useEscape";
import { useStore, type SettingsTab } from "../store";
import { Icon } from "../lib/icons";
import { t, type Key } from "../i18n";
import AiSection from "./AiSection";
import AboutSection from "./AboutSection";
import LibrarySection from "./LibrarySection";
import DiagnosticsSection from "./DiagnosticsSection";
import ExportRightsSection from "./ExportRightsSection";
import LanguageBox from "./LanguageBox";
import PhoneSection from "./PhoneSection";
import { api } from "../lib/api";

/**
 * Settings: AI · Library · Export · Phone · About. Phone only in a build
 * with the phone link (docs/MOBILE.md).
 *
 * It used to be AI · Advanced · Export · About, with Advanced holding a
 * provider dropdown, a model name, a server URL, runtime controls, a model
 * grid — and, above all of those, the catalogue backups, under an "AI & local
 * models" heading. The AI knobs are gone (the app decides them), backups moved
 * to Library where the work they protect lives, and diagnostics to About.
 *
 * There is no Save button. Every tab applies as you go — mixing the two modes
 * was once the bug that made Save write a stale model back over the one the
 * picker had just stored.
 */

// Keys, not words: a table at module scope is built once, before the language
// is known (see `i18n`).
const TABS: { id: SettingsTab; label: Key }[] = [
  { id: "ai", label: "settings.tab.ai" },
  { id: "library", label: "settings.tab.library" },
  { id: "export", label: "settings.tab.export" },
  { id: "phone", label: "settings.tab.phone" },
  { id: "about", label: "settings.tab.about" },
];

/** Each tab's heading and one line under it. AI's line is replaced by its
 *  status pill, the one-glance answer to "is AI working?". */
const HEADINGS: Record<SettingsTab, { title: Key; sub: Key | null }> = {
  library: { title: "settings.heading.library", sub: "settings.heading.librarySub" },
  ai: { title: "settings.heading.ai", sub: null },
  export: { title: "settings.heading.export", sub: "settings.heading.exportSub" },
  phone: { title: "settings.heading.phone", sub: "settings.heading.phoneSub" },
  about: { title: "settings.heading.about", sub: "settings.heading.aboutSub" },
};

/** `closing` runs the exit animation: App keeps the panel mounted for it (see `usePresence`). */
export default function SettingsPanel({ closing = false }: { closing?: boolean }) {
  const setShowSettings = useStore((s) => s.setShowSettings);
  const refreshAi = useStore((s) => s.refreshAi);
  const ai = useStore((s) => s.ai);
  const onClose = useCallback(() => setShowSettings(false), [setShowSettings]);

  const tab = useStore((s) => s.settingsTab);
  const openSettings = useStore((s) => s.openSettings);
  const setTab = openSettings;
  const version = useStore((s) => s.update?.currentVersion);

  useEffect(() => void refreshAi(), [refreshAi]);

  // The Phone tab exists only in a build with the phone link.
  const [phoneLink, setPhoneLink] = useState(false);
  useEffect(() => {
    api
      .phoneLinkStatus()
      .then((s) => setPhoneLink(s.available))
      .catch(() => setPhoneLink(false));
  }, []);
  const tabs = TABS.filter((item) => item.id !== "phone" || phoneLink);

  // A modal: Escape closes it — a confirm opened from in here owns
  // Escape first — Tab stays in it, and focus goes in and comes back.
  const card = useRef<HTMLDivElement>(null);
  useModal(!closing, card, onClose);

  // Status pill: the one-glance answer to "is AI working?". `configured`
  // separates "switched off on purpose" from "hasn't set itself up yet" — both
  // report a provider of "none", but only one of them is a decision.
  const pill = (() => {
    if (!ai) return { dot: "bg-faint", label: t("settings.tab.ai"), text: t("common.checking") };
    if (ai.provider === "none")
      return {
        dot: "bg-faint",
        label: ai.configured ? t("common.off") : t("settings.ai.pill.readyWhenNeeded"),
        // Its own wording for "off": the backend's detail points at this tab.
        text: ai.configured ? t("settings.ai.pill.offDetail") : ai.detail,
      };
    // Never the engine's name: which runtime serves a feature is the app's
    // business, not a setting.
    return {
      dot: ai.ready ? "bg-positive" : "bg-warning",
      label: ai.provider === "vision" ? t("settings.ai.pill.onDetailed") : t("common.on"),
      text: ai.ready ? t("settings.ai.pill.ready") : ai.detail,
    };
  })();

  return (
    <div
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      data-closing={closing || undefined}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      {/* One height for every tab, the tallest's, with the tab's content
          scrolling inside it. Sized to its content and centred, the card
          grew and shrank as the tabs were switched, and the title, the tabs
          under the pointer and Done all jumped up and down with it. */}
      <div
        ref={card}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        className="anim-card flex h-[min(720px,calc(100vh-4rem))] max-h-[min(720px,calc(100vh-4rem))] w-full max-w-xl flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-4 py-3.5">
          <div className="min-w-0">
            <h2 id="settings-title" className="font-display text-display text-ink">{t(HEADINGS[tab].title)}</h2>
            {tab === "ai" ? (
              // A status line where the other tabs have their subtitle, at
              // the same height: as a boxed pill it made this header taller,
              // and the tabs under it moved whenever AI was switched to.
              <div
                className="flex max-w-full items-center gap-2 text-caption"
                title={ai?.detail ?? ""}
                role="status"
              >
                <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${pill.dot}`} />
                <span className="shrink-0 font-medium text-ink">{pill.label}</span>
                <span className="shrink-0 text-faint">·</span>
                <span className="truncate text-muted">{pill.text}</span>
              </div>
            ) : (
              <p className="text-caption text-faint">
                {tab === "about" && version
                  ? t("settings.version", { version })
                  : HEADINGS[tab].sub && t(HEADINGS[tab].sub)}
              </p>
            )}
          </div>
          <button onClick={onClose} className="icon-chip ml-auto shrink-0" title={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        {/* Tabs. */}
        <div className="flex gap-1 border-b border-line px-3" role="tablist">
          {tabs.map((item) => (
            <button
              key={item.id}
              role="tab"
              aria-selected={tab === item.id}
              onClick={() => setTab(item.id)}
              className={`relative px-3 py-2.5 text-caption font-medium transition-colors ${
                tab === item.id ? "text-ink" : "text-muted hover:text-ink"
              }`}
            >
              {t(item.label)}
              {tab === item.id && (
                <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-sm bg-accent" />
              )}
            </button>
          ))}
        </div>

        <div className="flex-1 overflow-y-auto px-4 py-4">
          {tab === "library" ? (
            <LibrarySection />
          ) : tab === "ai" ? (
            <AiSection />
          ) : tab === "export" ? (
            <ExportRightsSection />
          ) : tab === "phone" ? (
            <PhoneSection />
          ) : (
            <>
              <AboutSection />
              <div className="mt-3">
                <LanguageBox />
              </div>
              <DiagnosticsSection />
            </>
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-line px-4 py-3">
          <button onClick={onClose} className="btn-accent ml-auto">
            {t("common.done")}
          </button>
        </div>
      </div>
    </div>
  );
}
