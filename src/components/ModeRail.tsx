import { developShown, useStore } from "../store";
import { Icon, type IconName } from "../lib/icons";
import { t, type Key } from "../i18n";
import AppMark from "./AppMark";

/**
 * The application's top-level spine: a slim vertical rail that switches between
 * the four workspaces (Browse / Cull / Places / People) and launches the photo-scoped
 * overlays and panels (Develop, Activity, Settings).
 *
 * This replaces the old scatter of modal buttons in the top bar with one place
 * that always says where you are. Develop is not a mode — it opens on the
 * focused photo as an overlay — so its button is enabled only when there is a
 * photo to open.
 */

/**
 * One rail destination: a glyph with its name under it.
 *
 * The rail was icon-only, on the theory that five glyphs and a tooltip are
 * enough. They are not — a loupe, a pin and a slider say nothing about culling,
 * places or developing until you already know, and a tooltip only helps someone
 * who has guessed where to hover. `title` stays for the disabled Develop
 * button, whose tooltip carries the reason it is disabled.
 *
 * `caption` lets a button show a shorter word than it announces to a screen
 * reader, so "Develop — select a photo first" doesn't have to fit under an icon.
 */
function RailButton({
  icon,
  label,
  caption,
  tour,
  active,
  disabled,
  onClick,
}: {
  icon: IconName;
  label: Key;
  caption?: Key;
  /** Where the guided tour points (see lib/tour). */
  tour?: string;
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={t(label)}
      aria-label={t(label)}
      data-tour={tour}
      // The rail is where you are: say so, not only show it.
      aria-current={active ? "page" : undefined}
      className={`mode-btn ${active ? "mode-on" : ""}`}
    >
      <Icon name={icon} size={19} />
      <span className="mode-btn-label">{t(caption ?? label)}</span>
    </button>
  );
}

export default function ModeRail() {
  const mode = useStore((s) => s.mode);
  const setMode = useStore((s) => s.setMode);
  const developing = useStore(developShown);
  const focusId = useStore((s) => s.focusId);
  const hasSelection = useStore((s) => s.selected.size > 0);
  const openDevelop = useStore((s) => s.openDevelop);
  const setShowSettings = useStore((s) => s.setShowSettings);
  const showActivity = useStore((s) => s.showActivity);
  const setShowActivity = useStore((s) => s.setShowActivity);

  // A job is running — the Activity button pulses so background work is visible
  // from anywhere without opening the panel. `jobs` carries the rest (model
  // downloads, captioning, indexing, faces, focus scoring), which the pulse used
  // to miss entirely, so the promise above was only half true.
  const busy = useStore(
    (s) =>
      s.scanning !== null ||
      s.tagging !== null ||
      !!s.exportJob?.running ||
      Object.keys(s.jobs).length > 0,
  );

  // Develop opens on the focused photo, or the first selected one as a fallback.
  const developTarget = useStore((s) => s.focusId ?? [...s.selected][0] ?? null);
  const canDevelop = focusId !== null || hasSelection;

  return (
    // `library-view`: hidden while Develop draws through a transparent page.
    <nav className="library-view flex w-[84px] shrink-0 flex-col items-center gap-0.5 border-r border-line bg-panel py-3">
      {/* Wordmark: the app icon's mark, a 3×3 grid lit from the top-left
          (app-icon.svg), in the same greys. */}
      {/* On a Mac this cell is also the traffic lights' corner and part of the
          title bar: drag the window from it; the mark shows in full screen. */}
      {/* The product's name, the same in every language. i18n-ok */}
      <div className="rail-logo mb-2 grid h-10 w-10 place-items-center" title="SourceLight" data-tauri-drag-region="deep">
        <AppMark size={22} />
      </div>

      <RailButton
        icon="grid"
        label="shell.mode.browse"
        tour="rail-browse"
        active={mode === "browse" && !developing}
        onClick={() => setMode("browse")}
      />
      <RailButton
        icon="cull"
        label="shell.mode.cull"
        tour="rail-cull"
        active={mode === "cull" && !developing}
        onClick={() => setMode("cull")}
      />
      <RailButton
        icon="pin"
        label="shell.mode.places"
        tour="rail-places"
        active={mode === "places" && !developing}
        onClick={() => setMode("places")}
      />
      <RailButton
        icon="user"
        label="shell.mode.people"
        tour="rail-people"
        active={mode === "people" && !developing}
        onClick={() => setMode("people")}
      />
      <RailButton
        icon="adjust"
        label={canDevelop ? "shell.mode.develop" : "shell.mode.developNeedsPhoto"}
        caption="shell.mode.developCaption"
        tour="rail-develop"
        active={developing}
        disabled={!canDevelop}
        onClick={() => developTarget !== null && void openDevelop(developTarget)}
      />

      <div className="flex-1" />

      <button aria-pressed={showActivity}
        onClick={() => setShowActivity(!showActivity)}
        title={t("shell.mode.activity")}
        aria-label={t("shell.mode.activity")}
        data-tour="rail-activity"
        className={`mode-btn ${showActivity ? "mode-on" : ""}`}
      >
        <span className="relative">
          <Icon name="activity" size={19} />
          {busy && (
            <span className="absolute -right-1 -top-1 h-2 w-2 animate-pulse rounded-full bg-accent" />
          )}
        </span>
        <span className="mode-btn-label">{t("shell.mode.activity")}</span>
      </button>
      <button
        onClick={() => setShowSettings(true)}
        title={t("shell.mode.settings")}
        aria-label={t("shell.mode.settings")}
        data-tour="rail-settings"
        className="mode-btn"
      >
        <Icon name="settings" size={19} />
        <span className="mode-btn-label">{t("shell.mode.settingsCaption")}</span>
      </button>
    </nav>
  );
}
