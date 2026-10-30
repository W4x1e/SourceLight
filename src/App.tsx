import InsightsView from "./components/InsightsView";
import { coalescer } from "./lib/coalesce";
import Slideshow from "./components/Slideshow";
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import ModeRail from "./components/ModeRail";
import BrowseWorkspace from "./components/BrowseWorkspace";
import CullWorkspace from "./components/CullWorkspace";
import PlacesWorkspace from "./components/PlacesWorkspace";
import PeopleWorkspace from "./components/PeopleWorkspace";
import CompareView from "./components/CompareView";
import Develop from "./components/Develop";
import DevelopOpening from "./components/dev/DevelopOpening";
import Lightbox from "./components/Lightbox";
import ActivityPanel from "./components/ActivityPanel";
import ExportQueuePanel from "./components/ExportQueuePanel";
import SettingsPanel from "./components/SettingsPanel";
import ShortcutsPanel from "./components/ShortcutsPanel";
import GuidedTour from "./components/GuidedTour";
import { usePresence } from "./lib/usePresence";
import PhotoMenu from "./components/PhotoMenu";
import ImportDialog from "./components/ImportDialog";
import CaptureTimeDialog from "./components/CaptureTimeDialog";
import { DialogHost, toast } from "./lib/dialogs";
import { Icon } from "./lib/icons";
import { api } from "./lib/api";
import AppMark from "./components/AppMark";
import { reportRecovery, watchForDamage } from "./lib/catalogueHealth";
import { reportUncleanExit } from "./lib/uncleanExit";
import ErrorBoundary from "./components/ErrorBoundary";
import { developShown, useStore, type Job } from "./store";
import ExportDialog from "./components/ExportDialog";
import CommandPalette from "./components/CommandPalette";
import type { Progress } from "./types";
import type { PhoneTrack } from "./bindings";
import { t, type Key } from "./i18n";
import { progressText, translateCoreError } from "./lib/coreText";
import { isCancelled } from "./lib/jobs";

/**
 * The four count-based passes share one payload shape (`current`/`total`/`phase`),
 * so their listeners are the same three lines each. Returns an array so the call
 * site can spread it into the unlisten list.
 */
function countJob(
  event: string,
  id: string,
  label: Key,
  // Maps a batch's own count onto a longer run, as for automatic descriptions.
  run: () => { done: number; total: number } | null = () => null,
) {
  return [
    listen<{ current: number; total: number; phase: string }>(event, (e) => {
      const p = e.payload;
      const done = p.phase === "done";
      const r = run();
      const current = r ? r.done + p.current : p.current;
      const total = r ? Math.max(r.total, current) : p.total;
      jobProgress(id, done ? null : { label: t(label), current, total, unit: "count" }, done);
    }),
  ];
}

// Progress reaches the store at most ten times a second: a pass
// reports every photo, and each report re-rendered Activity and Cull.
const jobProgress = coalescer<Job | null>((id, job) => useStore.getState().setJob(id, job));
const scanReport = coalescer<Progress>((_, p) => useStore.getState().scanProgress(p));
const tagReport = coalescer<Progress | null>((_, p) => useStore.getState().setTagging(p));

export default function App() {
  const showExport = useStore((s) => s.showExport);
  const setShowExport = useStore((s) => s.setShowExport);
  const libraryFolders = useStore((s) => s.libraryFolders);
  const mode = useStore((s) => s.mode);
  // At most one full-screen overlay at a time, highest first: Develop covers
  // Compare, which covers the Lightbox. One value rather than three booleans
  // with cascading `!` guards, so adding a fourth cannot forget a guard.
  // Develop is up from the moment it is asked for, its first photo still
  // opening (`DevelopOpening`).
  const overlay = useStore((s): "develop" | "compare" | "lightbox" | null =>
    developShown(s)
      ? "develop"
      : s.compare !== null
        ? "compare"
        : s.lightboxIndex !== null
          ? "lightbox"
          : null,
  );
  const developOpen = useStore((s) => s.develop !== null);
  const showSettings = useStore((s) => s.showSettings);
  const showShortcuts = useStore((s) => s.showShortcuts);
  // Mounted a moment past closing, so each can animate out rather than vanish.
  const settings = usePresence(showSettings);
  const shortcuts = usePresence(showShortcuts);
  const tour = usePresence(useStore((s) => s.tour !== null));
  const error = useStore((s) => s.error);
  const update = useStore((s) => s.update);
  const [updateDismissed, setUpdateDismissed] = useState(false);

  const setShowSettings = useStore((s) => s.setShowSettings);
  const openSettings = useStore((s) => s.openSettings);
  const setError = useStore((s) => s.setError);
  const addLibraryFolder = useStore((s) => s.addLibraryFolder);
  const setShowImport = useStore((s) => s.setShowImport);

  useEffect(() => {
    void useStore.getState().init();
  }, []);

  // A damaged catalogue: say what startup restored, and offer the newest good
  // backup if the check after startup finds damage deeper in. Then a last
  // session that never exited, one dialog after the other.
  useEffect(() => {
    void reportRecovery().then(reportUncleanExit);
    const unlisten = watchForDamage();
    return () => void unlisten.then((un) => un());
  }, []);

  // A quiet check for a newer release, well after launch so it never
  // competes with opening the library, and silent when it fails: the app works
  // fully offline, and being unable to reach an update server is not an error.
  // A build with no update source configured answers without any network call.
  useEffect(() => {
    // Only when the user hasn't turned it off.
    if (!useStore.getState().checkUpdates) return;
    const t = window.setTimeout(() => {
      api
        .checkForUpdate()
        .then((u) => useStore.getState().setUpdate(u))
        .catch((e) => console.info("update check skipped:", e));
    }, 15_000);
    return () => window.clearTimeout(t);
  }, []);

  // Suppress the webview's default right-click menu so the app reads as a native
  // desktop tool, not a web page. Text fields keep theirs.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null;
      if (!el?.closest("input, textarea, [contenteditable]")) e.preventDefault();
    };
    document.addEventListener("contextmenu", onContextMenu);
    return () => document.removeEventListener("contextmenu", onContextMenu);
  }, []);

  // Progress events from the Rust side.
  useEffect(() => {
    const unlisten = [
      // Picks and rejects made on a paired phone (docs/MOBILE.md).
      listen<number>("phone-changes", () => void useStore.getState().reload()),
      // A shoot's GPS track from a paired phone: offer to place its photos,
      // through the same dialog as a GPX file opened by hand.
      listen<PhoneTrack>("phone-track", (e) =>
        toast(t("shell.phoneTrack.arrived", { name: e.payload.name }), "info", {
          label: t("shell.phoneTrack.place"),
          run: () => {
            useStore.getState().setPhoneTrack(e.payload.path);
            useStore.getState().setMode("places");
          },
        }),
      ),
      listen<Progress>("scan-progress", (e) =>
        scanReport("scan", { ...e.payload, message: progressText(e.payload) }),
      ),
      listen<Progress>("tag-progress", (e) => {
        if (e.payload.phase === "done") {
          tagReport("tag", null, true);
          void useStore.getState().reload();
        } else {
          tagReport("tag", { ...e.payload, message: progressText(e.payload) });
        }
      }),
      listen<Progress>("export-progress", (e) => {
        const p = e.payload;
        useStore.getState().updateExportProgress(p.current, p.total, progressText(p));
      }),
      // Each photo's result as it is written.
      listen<{ photoId: number; error: string | null }>("export-item", (e) => {
        useStore.getState().noteExportItem(e.payload.photoId, e.payload.error);
      }),

      // The long jobs, mirrored into the store so they are visible from
      // anywhere. Each is also listened to by the panel that starts it, which
      // keeps its own detailed UI — but that panel unmounts, and a 6 GB pull
      // must not become invisible just because Settings was closed.
      // Automatic descriptions count against the whole run, not the batch.
      ...countJob("caption-progress", "caption", "shell.job.captioning", () => {
        const s = useStore.getState();
        return s.autoAnalysis === "descriptions" ? s.describeRun : null;
      }),
      ...countJob("embed-progress", "index", "shell.job.indexing"),
      ...countJob("faces-progress", "faces", "shell.job.faces"),
      ...countJob("people-progress", "people", "shell.job.people"),
      ...countJob("quality-progress", "quality", "shell.job.quality"),
      ...countJob("import-progress", "import", "shell.job.import"),
      ...countJob("locations-progress", "locations", "shell.job.locations"),
      ...countJob("colours-progress", "colours", "shell.job.colours"),
      ...countJob("denoise-progress", "denoise", "shell.job.denoise"),
      ...countJob("erase-progress", "erase", "shell.job.erase"),
      ...countJob("palette-progress", "palette", "settings.activity.step.palette"),
      ...countJob("sidecars-progress", "sidecars", "shell.job.sidecars"),

      listen<{ id: string; received: number; total: number; done: boolean; error: string | null }>(
        "ai-download-progress",
        (e) => {
          const p = e.payload;
          const label = t("shell.job.downloadModel");
          // A failure stays in Activity with its reason until dismissed;
          // a cancel the user chose just goes.
          const failed = p.done && p.error && !isCancelled(p.error) ? translateCoreError(p.error) : null;
          useStore
            .getState()
            .setJob(
              `model:${p.id}`,
              failed
                ? { label, current: p.received, total: p.total, unit: "bytes", error: failed }
                : p.done
                  ? null
                  : { label, current: p.received, total: p.total, unit: "bytes" },
            );
        },
      ),
      listen<{ stage: string; received: number; total: number; done: boolean }>(
        "vision-runtime-progress",
        (e) => {
          const p = e.payload;
          useStore
            .getState()
            .setJob(
              "vision-runtime",
              p.done
                ? null
                : {
                    label: p.stage === "extracting" ? t("shell.job.extractRuntime") : t("shell.job.downloadRuntime"),
                    current: p.received,
                    total: p.total,
                    unit: "bytes",
                  },
            );
        },
      ),
      listen<{ received: number; total: number; done: boolean }>("vision-model-progress", (e) => {
        const p = e.payload;
        useStore
          .getState()
          .setJob(
            "vlm-model",
            p.done
              ? null
              : { label: t("shell.job.downloadVisionModel"), current: p.received, total: p.total, unit: "bytes" },
          );
      }),
    ];
    return () => {
      void Promise.all(unlisten).then((fns) => fns.forEach((f) => f()));
    };
  }, []);

  // The welcome screen only once the folder list has loaded and is empty —
  // not while it loads, which would flash it on every launch.
  const emptyLibrary =
    libraryFolders !== null && libraryFolders.folders.length === 0 && libraryFolders.unfiled === 0;

  // The guided tour, once: after the first folder is in, and never over
  // something the user opened. The pause lets the shell paint before it dims.
  const tourDue = useStore(
    (s) => !s.tourSeen && s.tour === null && !s.showSettings && !s.showImport,
  );
  const inShell = libraryFolders !== null && !emptyLibrary;
  useEffect(() => {
    if (!tourDue || !inShell || overlay !== null) return;
    const t = window.setTimeout(() => useStore.getState().startTour(), 600);
    return () => window.clearTimeout(t);
  }, [tourDue, inShell, overlay]);

  if (emptyLibrary) {
    return (
      <>
      <div className="grid h-full place-items-center bg-base">
        <div className="max-w-sm text-center">
          <div className="mb-4 grid place-items-center">
            <AppMark size={44} />
          </div>
          {/* i18n-ok: the product's name */}
          <h1 className="font-display mb-2 text-4xl text-ink">SourceLight</h1>
          <p className="mb-6 text-caption leading-relaxed text-muted">
            {t("shell.welcome.intro")}
          </p>
          {/* One width for the two ways in, so they read as a pair of
              choices rather than two buttons that happen to be stacked. */}
          <div className="mx-auto inline-flex flex-col items-stretch gap-2">
            <button onClick={() => void addLibraryFolder()} className="btn-accent">
              {t("shell.welcome.choose")}
            </button>
            {/* Card-first photographers: a card picked as "a folder"
                is catalogued in place, and lost when it is formatted. */}
            <button onClick={() => setShowImport(true)} className="btn-solid">
              {t("shell.welcome.importCard")}
            </button>
          </div>
          <p className="mt-3 text-label text-faint">{t("shell.welcome.importCardHint")}</p>
          <p className="mt-1 text-label text-faint">{t("shell.welcome.addLater")}</p>
        </div>
        <DialogHost />
      </div>
      {/* Here and in the shell at the same place in the tree, so an import
          started from this screen keeps its report when the first folder
          turns this screen into the shell. */}
      <ImportDialog />
      </>
    );
  }

  return (
    // `.app-shell` and Develop-as-a-direct-child are load-bearing: the direct
    // GPU canvas presents through a native surface behind the webview, and
    // `html.direct-canvas .app-shell > .library-view` hides the Library's
    // layout so the page is a transparent hole. Do not wrap Develop.
    <>
    <div className="app-shell flex h-full bg-base">
      <ModeRail />

      <div className="library-view workspace-host flex min-w-0 flex-1 flex-col">
        {error && (
          <div role="alert" className="flex items-start gap-2 border-b border-danger/40 bg-danger/10 px-3 py-2 text-caption text-danger-ink">
            <span className="flex-1">{error}</span>
            <button onClick={() => setShowSettings(true)} className="shrink-0 underline hover:text-danger-ink">
              {t("shell.banner.openSettings")}
            </button>
            <button
              onClick={() => setError(null)}
              className="shrink-0 text-danger hover:text-danger-ink"
              aria-label={t("shell.banner.dismissError")}
            >
              <Icon name="close" size={13} />
            </button>
          </div>
        )}

        {update?.available && !updateDismissed && (
          <div className="flex items-center gap-2 border-b border-accent/30 bg-accent/10 px-3 py-1.5 text-caption text-ink">
            <span className="flex-1">{t("shell.banner.updateAvailable", { version: update.available.version })}</span>
            <button onClick={() => openSettings("about")} className="shrink-0 underline hover:text-ink">
              {t("shell.banner.whatsNew")}
            </button>
            <button
              onClick={() => setUpdateDismissed(true)}
              className="shrink-0 text-muted hover:text-ink"
              aria-label={t("shell.banner.dismissUpdate")}
            >
              <Icon name="close" size={13} />
            </button>
          </div>
        )}

        {/* Keyed by mode so switching workspaces clears a failed one, rather
            than leaving the user stuck on the error until they retry. */}
        <ErrorBoundary key={mode} label={t("shell.error.thisWorkspace")}>
          {mode === "cull" ? (
            <CullWorkspace />
          ) : mode === "places" ? (
            <PlacesWorkspace />
          ) : mode === "people" ? (
            <PeopleWorkspace />
          ) : (
            <BrowseWorkspace />
          )}
        </ErrorBoundary>
      </div>

      {/* Develop is the largest surface in the app and the likeliest to throw,
          so it gets its own boundary — a failure there must not take the
          library with it. The boundary renders a Fragment when healthy, so
          Develop remains a direct DOM child of `.app-shell` as required. */}
      {overlay === "develop" && (
        <ErrorBoundary
          label={t("shell.mode.develop")}
          fallbackClassName="develop-shell"
          onLeave={() => void useStore.getState().closeDevelop()}
        >
          {developOpen ? <Develop /> : <DevelopOpening />}
        </ErrorBoundary>
      )}
      {overlay === "compare" && (
        <ErrorBoundary label={t("shell.error.compare")} onLeave={() => useStore.getState().closeCompare()}>
          <CompareView />
        </ErrorBoundary>
      )}
      {overlay === "lightbox" && (
        <ErrorBoundary label={t("shell.error.lightbox")} onLeave={() => useStore.getState().openLightbox(null)}>
          <Lightbox />
        </ErrorBoundary>
      )}
      <ActivityPanel />
      {settings.mounted && <SettingsPanel closing={settings.closing} />}
      {shortcuts.mounted && <ShortcutsPanel closing={shortcuts.closing} />}
      {tour.mounted && <GuidedTour closing={tour.closing} />}
      <ExportQueuePanel />
      <ExportDialog open={showExport} onClose={() => setShowExport(false)} batch />
      <PhotoMenu />
      <CommandPalette />
      <InsightsView />
      <Slideshow />
      <CaptureTimeDialog />
      <DialogHost />
    </div>
    <ImportDialog />
    </>
  );
}
