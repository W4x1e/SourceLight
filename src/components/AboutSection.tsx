import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { confirm } from "../lib/dialogs";
import { unfinishedWork } from "../lib/quitGuard";
import { t } from "../i18n";
import { api } from "../lib/api";
import { useStore } from "../store";
import LegalDocument from "./LegalDocument";

/** An error's own words — `api` wraps command errors in `Error`, and "Error: "
 *  in front of a sentence meant for a person reads as a crash. */
function said(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * SourceLight's own licence and the third-party licences, both readable here.
 * Nothing asks for either to be accepted: the MPL grants rights to
 * whoever receives the program and asks nothing of someone who only runs it.
 * The third-party file ships with the installer but was unreachable from the
 * UI.
 */
function LicencesBox() {
  const [open, setOpen] = useState<{ title: string; text: string | null } | null>(null);

  const show = (title: string, load: () => Promise<string>) => {
    setOpen({ title, text: null });
    load()
      .then((text) => setOpen({ title, text }))
      .catch((e) => setOpen({ title, text: t("settings.about.licences.openFailed", { error: said(e) }) }));
  };

  return (
    <div className="rounded-md border border-line bg-raised p-3">
      <div className="text-caption font-medium text-ink">{t("settings.about.licences.title")}</div>
      <p className="mt-0.5 text-caption text-muted">{t("settings.about.licences.openSource")}</p>
      <div className="mt-2 flex flex-wrap gap-2">
        <button className="btn-solid" onClick={() => show(t("settings.about.licences.sourceLight"), api.licenceText)}>
          {t("settings.about.licences.sourceLight")}
        </button>
        <button
          className="btn-solid"
          onClick={() => show(t("settings.about.licences.thirdParty"), api.thirdPartyLicences)}
        >
          {t("settings.about.licences.thirdParty")}
        </button>
      </div>
      <div className="mt-2 text-label leading-relaxed text-faint">
        {t("settings.about.licences.attribution")}
      </div>
      {open && (
        <LegalDocument title={open.title} text={open.text} onClose={() => setOpen(null)} />
      )}
    </div>
  );
}

/** The guided tour and the shortcuts sheet, both findable from here. The
 *  sheet is otherwise only reachable from Browse's top bar. */
function GettingStartedBox() {
  const startTour = useStore((s) => s.startTour);
  const openShortcuts = () => {
    useStore.getState().setShowSettings(false);
    useStore.getState().setShowShortcuts(true);
  };
  return (
    <div className="rounded-md border border-line bg-raised p-3">
      <div className="text-caption font-medium text-ink">{t("settings.about.start.title")}</div>
      <p className="mt-0.5 text-caption text-muted">
        {t("settings.about.start.blurb")}
      </p>
      <div className="mt-2 flex flex-wrap gap-2">
        <button className="btn-solid" onClick={startTour}>
          {t("settings.about.start.replayTour")}
        </button>
        <button className="btn-solid" onClick={openShortcuts}>
          {t("settings.about.start.shortcuts")}
        </button>
      </div>
    </div>
  );
}

/**
 * About: the licences, the version, and updates.
 *
 * A check here is one the user asked for, so unlike the silent one at launch it
 * reports a failure — but in the language of being offline, which is by far the
 * likeliest cause and not a fault. The app works fully offline; being unable to
 * reach an update server is never an error state for the library itself.
 */
export default function AboutSection() {
  const update = useStore((s) => s.update);
  const checkUpdates = useStore((s) => s.checkUpdates);
  const setCheckUpdates = useStore((s) => s.setCheckUpdates);
  const setUpdate = useStore((s) => s.setUpdate);
  const [busy, setBusy] = useState<"checking" | "installing" | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [checked, setChecked] = useState(false);

  const check = async () => {
    setBusy("checking");
    setProblem(null);
    try {
      setUpdate(await api.checkForUpdate());
      setChecked(true);
    } catch (e) {
      console.warn("update check failed", e);
      setProblem(t("settings.about.updates.offline"));
    } finally {
      setBusy(null);
    }
  };

  // How far the download is, while installing.
  const [downloaded, setDownloaded] = useState<number | null>(null);
  useEffect(() => {
    if (busy !== "installing" || !("__TAURI_INTERNALS__" in window)) return;
    const un = listen<{ received: number; total: number }>("update-progress", (e) => {
      const { received, total } = e.payload;
      if (total > 0) setDownloaded(Math.min(100, Math.round((received / total) * 100)));
    });
    return () => void un.then((f) => f());
  }, [busy]);

  const install = async () => {
    // Installing restarts the app: asked first, as quitting is, when that
    // would cut off an export or a card import.
    const question = unfinishedWork(useStore.getState());
    if (question && !(await confirm(question))) return;
    setBusy("installing");
    setProblem(null);
    try {
      // On Windows the installer takes over and the app exits; elsewhere it
      // restarts into the new version. Either way this does not return normally.
      await api.installUpdate();
    } catch (e) {
      setProblem(t("settings.about.updates.installFailed", { error: said(e) }));
      setBusy(null);
    }
  };

  const available = update?.available ?? null;

  return (
    <div className="space-y-3">
      <LicencesBox />
      <GettingStartedBox />
      <div className="rounded-md border border-line bg-raised p-3">
        <div className="text-caption font-medium text-ink">{t("settings.about.updates.title")}</div>
        <label
          className="mt-1 flex items-center gap-2 text-caption text-muted"
          title={t("settings.about.updates.autoHint")}
        >
          <input type="checkbox" checked={checkUpdates} onChange={(e) => setCheckUpdates(e.target.checked)} />
          {t("settings.about.updates.auto")}
        </label>
        {update && !update.configured ? (
          <p className="mt-0.5 text-label leading-relaxed text-faint">
            {t("settings.about.updates.notConfigured")}
          </p>
        ) : available ? (
          <>
            <p className="mt-0.5 text-caption text-muted">
              {t("settings.about.updates.available", {
                version: available.version,
                current: update?.currentVersion ?? "",
              })}
            </p>
            {available.notes && (
              <pre className="mt-2 max-h-40 overflow-y-auto whitespace-pre-wrap rounded-md border border-line bg-base p-2 font-sans text-label leading-relaxed text-muted">
                {available.notes}
              </pre>
            )}
            <p className="mt-2 text-label leading-relaxed text-faint">
              {t("settings.about.updates.kept")}
            </p>
            <button
              onClick={() => void install()}
              disabled={busy !== null}
              className="btn-solid mt-2"
            >
              {busy === "installing"
                ? downloaded
                  ? t("settings.about.updates.downloading", { percent: downloaded })
                  : t("settings.about.updates.installing")
                : t("settings.about.updates.install")}
            </button>
          </>
        ) : (
          <>
            <p className="mt-0.5 text-caption text-muted">
              {checked && update
                ? t("settings.about.updates.upToDate", { version: update.currentVersion })
                : update
                  ? t("settings.about.updates.current", { version: update.currentVersion })
                  : t("settings.about.updates.quiet")}
            </p>
            <button
              onClick={() => void check()}
              disabled={busy !== null}
              className="btn-ghost mt-2 text-label"
            >
              {busy === "checking" ? t("common.checking") : t("settings.about.updates.check")}
            </button>
          </>
        )}
        {problem && <p className="mt-2 text-label leading-relaxed text-warning">{problem}</p>}
      </div>
    </div>
  );
}
