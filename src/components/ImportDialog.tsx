import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ProgressBar from "./ProgressBar";
import { open } from "@tauri-apps/plugin-dialog";
import { useStore } from "../store";
import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { humanBytes } from "../lib/format";
import { Icon } from "../lib/icons";
import { useModal } from "../lib/useEscape";
import { folderName } from "./LibraryMenu";
import type { ImportOutcome, ImportPreview, ImportProblem, ImportSource, IptcTemplate, LeftBehind } from "../bindings";
import { formatNumber, t, type Key } from "../i18n";
import { usePresence } from "../lib/usePresence";

/**
 * Import from a memory card.
 *
 * The first step of every shoot used to happen outside the app: copy the card
 * in Explorer, then point SourceLight at the folder. This does it here — into
 * dated folders of a library folder, each file verified, anything already
 * imported skipped, optionally a second copy for backup — and then the usual
 * scan and automatic analysis take over. The card is only ever read.
 *
 * **It ends on a report, not a toast.** Photographers format the card
 * on the strength of what the import says, so the dialog stays open on what
 * happened: a "safe to format" verdict only when every photo is in the
 * library (and on the backup, if one was asked for) and nothing else was left
 * on the card; otherwise what didn't make it, by cause, with the way to fix
 * each — try those files again, or finish the backup.
 */

/** "3 videos, 1 sound recording" — what stays on the card, or "" for nothing. */
function leftBehindText(l: LeftBehind): string {
  const parts: string[] = [];
  if (l.videos > 0) parts.push(t("transfer.import.videos", { count: l.videos }));
  if (l.audio > 0) parts.push(t("transfer.import.audio", { count: l.audio }));
  if (l.other > 0) parts.push(t("transfer.import.otherFiles", { count: l.other }));
  return parts.join(", ");
}

/** A path's last part: the file name a photographer recognises. */
function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** One cause in the report: a heading and the files under it, with why. */
function ProblemList({
  title,
  items,
  count = items.length,
}: {
  title: string;
  items: { path: string; reason?: string }[];
  /** How many there are, when `items` names only some of them. */
  count?: number;
}) {
  if (items.length === 0) return null;
  return (
    <div>
      <div className="section-label mb-1 px-0">
        {title} <span className="tabular-nums text-faint">({formatNumber(count)})</span>
      </div>
      <ul className="max-h-32 space-y-1 overflow-y-auto rounded-md bg-raised px-3 py-2">
        {items.map((p) => (
          <li key={p.path} className="text-label" title={p.path}>
            <span className="font-mono text-ink">{baseName(p.path)}</span>
            {/* Whole, not cut to a line: why a file wasn't copied ("the card
                or its reader may be failing") is what decides whether the
                card can be formatted, and the tooltip only had the path. */}
            {p.reason && <span className="block break-words text-faint">{p.reason}</span>}
          </li>
        ))}
      </ul>
    </div>
  );
}

/** The folder layouts on offer, as templates for `ImportOptions.folders`. */
const LAYOUTS: readonly { id: string; label: Key; example: string }[] = [
  { id: "{yyyy}/{yyyy-mm-dd}", label: "transfer.import.layoutYearDate", example: "2025/2025-06-14" },
  { id: "{yyyy-mm-dd}", label: "transfer.import.layoutDate", example: "2025-06-14" },
  { id: "{yyyy}/{yyyy-mm-dd} {job}", label: "transfer.import.layoutYearDateJob", example: "2025/2025-06-14 Smith wedding" },
  // One folder for the job however late it ran: a reception past
  // midnight split the dated layouts in two.
  { id: "{yyyy}/{job}", label: "transfer.import.layoutYearJob", example: "2025/Smith wedding" },
  { id: "", label: "transfer.import.layoutFlat", example: "" },
];

export default function ImportDialog() {
  const show = useStore((s) => s.showImport);
  const setShow = useStore((s) => s.setShowImport);
  const lib = useStore((s) => s.libraryFolders);
  const runScan = useStore((s) => s.runScan);
  const refreshFolders = useStore((s) => s.refreshFolders);
  const job = useStore((s) => s.jobs["import"]);
  const resumeShoot = useStore((s) => s.importShootId);
  const loadShoots = useStore((s) => s.loadShoots);

  const [sources, setSources] = useState<ImportSource[]>([]);
  const [source, setSource] = useState<string | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  // The last import's choices, remembered across launches.
  const remembered = useStore.getState().importSettings;
  const [destination, setDestination] = useState<string>(remembered.destination);
  const [layout, setLayout] = useState<string>(
    LAYOUTS.some((l) => l.id === remembered.layout) ? remembered.layout : LAYOUTS[0]!.id,
  );
  const [jobName, setJobName] = useState("");
  const [rename, setRename] = useState(remembered.rename);
  // The folders an import lands in ("2026-09-20 Smith wedding") as keywords.
  const [folderKeywords, setFolderKeywords] = useState(remembered.folderKeywords);
  const [backup, setBackup] = useState<string | null>(remembered.backup);
  // Whether the remembered backup drive is connected.
  const [backupMissing, setBackupMissing] = useState(false);
  // A title, caption and rights template for everything imported.
  const [templates, setTemplates] = useState<IptcTemplate[]>([]);
  const [template, setTemplate] = useState<number | null>(remembered.template);
  useEffect(() => {
    useStore.getState().setImportSettings({ destination, layout, rename, folderKeywords, backup, template });
  }, [destination, layout, rename, folderKeywords, backup, template]);
  useEffect(() => {
    if (!show || !backup) {
      setBackupMissing(false);
      return;
    }
    void api.folderExists(backup).then((there) => setBackupMissing(!there));
  }, [show, backup]);
  const [running, setRunning] = useState(false);
  // What a running dialog is doing, for its footer: copying can be stopped,
  // and so can a backup, but the library scan after an import is not this
  // dialog's to stop (it used to show "Copying to the backup" and Stop then).
  const [activity, setActivity] = useState<"copying" | "backup" | "scanning">("copying");
  // What the last import did, shown until the dialog is closed.
  const [report, setReport] = useState<ImportOutcome | null>(null);
  const [reportSource, setReportSource] = useState("");
  // Bumped to read the card again with the same source selected: on every
  // open, after every import, and on request. A swapped card in the
  // same reader has the same path, and must not show the last card's preview.
  const [reread, setReread] = useState(0);

  const close = useCallback(() => {
    if (!running) setShow(false);
  }, [running, setShow]);
  const card = useRef<HTMLDivElement>(null);
  useModal(show, card, close);

  // On open: look for cards, and default the destination to the first folder.
  // Every open, not the first: the card in the reader may have changed.
  useEffect(() => {
    if (!show) return;
    setReport(null);
    void api.importSources().then((s) => {
      setSources(s);
      // Keep a chosen folder; a card that has gone gives way to one present.
      const cards = new Set(s.map((x) => x.path));
      const wasCard = source != null && sources.some((x) => x.path === source);
      if (!source || (wasCard && !cards.has(source))) setSource(s[0]?.path ?? null);
    });
    // The remembered destination while it is still a library folder.
    const folders = lib?.folders ?? [];
    const first = folders[0]?.path;
    if (first && (!destination || !folders.some((f) => f.path === destination))) setDestination(first);
    void api.listIptcTemplates().then(setTemplates).catch(() => setTemplates([]));
    setReread((n) => n + 1);
  }, [show]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!show || !source) return;
    setPreview(null);
    setPreviewing(true);
    void api
      .importPreview(source)
      .then(setPreview)
      .catch((e) => toast(String(e), "error"))
      .finally(() => setPreviewing(false));
  }, [source, reread]); // eslint-disable-line react-hooks/exhaustive-deps

  const newFiles = preview ? preview.files - preview.already : 0;
  const example = useMemo(() => {
    const l = LAYOUTS.find((x) => x.id === layout);
    const folder = (l?.example ?? "").replace("Smith wedding", jobName.trim() || t("transfer.import.exampleJob"));
    const file = rename ? "20250614_0001.ARW" : "DSC00042.ARW";
    return [folderName(destination || t("transfer.import.exampleLibrary")), folder, file].filter(Boolean).join(" / ");
  }, [layout, jobName, rename, destination]);

  // Mounted a moment past closing, for the exit (see usePresence).
  const presence = usePresence(show);
  if (!presence.mounted) return null;

  const pickSource = async () => {
    const dir = await open({ directory: true, multiple: false, title: t("transfer.import.pickSourceTitle") });
    if (typeof dir === "string") setSource(dir);
  };
  const pickDestination = async () => {
    const dir = await open({ directory: true, multiple: false, title: t("transfer.import.pickDestinationTitle") });
    if (typeof dir === "string") setDestination(dir);
  };
  const pickBackup = async () => {
    const dir = await open({ directory: true, multiple: false, title: t("transfer.import.pickBackupTitle") });
    if (typeof dir === "string") setBackup(dir);
  };

  /** Runs the import — of everything new, or of `only` these card files when
   *  retrying — and shows its report. A retry adds to the report it retries. */
  const start = async (only: string[] | null = null) => {
    if (!source || !destination) return;
    const previous = only ? report : null;
    setRunning(true);
    setActivity("copying");
    try {
      const out = await api.importRun({
        source,
        destination,
        folders: layout,
        rename: rename ? "{yyyymmdd}_{seq}" : "",
        job: jobName,
        backup,
        only,
        // A retry joins the shoot it retries, as does "Import the rest" from a
        // shoot's line, so the shoot's warning clears when the photos are in.
        shootId: previous?.shootId ?? resumeShoot,
      });
      setReportSource(sources.find((x) => x.path === source)?.label ?? source);
      setReport(
        previous
          ? {
              ...out,
              copied: previous.copied + out.copied,
              skipped: previous.skipped,
              notBackedUp: [...previous.notBackedUp, ...out.notBackedUp],
              shootId: out.shootId ?? previous.shootId,
            }
          : out,
      );
      setReread((n) => n + 1);
      setActivity("scanning");
      await refreshFolders();
      // Scanning the destination files everything it copied; analysis follows.
      await runScan(destination);
      // The import is a shoot now, counted once its photos are in.
      void loadShoots();
      if (folderKeywords && out.shootId != null) await useStore.getState().addFolderKeywords(null, out.shootId);
      if (template != null && out.shootId != null) {
        const n = await api.applyIptcTemplate([], out.shootId, template);
        if (n > 0) useStore.getState().bumpIptc();
      }
    } catch (e) {
      // What is left here is refused before anything is copied: no space,
      // or a backup drive that isn't connected.
      toast(String(e), "error");
    } finally {
      setRunning(false);
    }
  };

  /** Copies what the import could not to the backup, from the library. */
  const finishBackup = async () => {
    if (report?.shootId == null) return;
    setRunning(true);
    setActivity("backup");
    try {
      const b = await api.finishShootBackup(report.shootId);
      setReport({ ...report, notBackedUp: b.failed });
      void loadShoots();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setRunning(false);
    }
  };

  const pct = job && job.total > 1 ? Math.round((job.current / job.total) * 100) : 0;

  return (
    <div
      data-closing={presence.closing || undefined}
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      onClick={(e) => e.target === e.currentTarget && close()}
    >
      <div
        ref={card}
        role="dialog"
        aria-modal="true"
        aria-label={t("transfer.import.title")}
        className="anim-card flex max-h-[min(720px,calc(100vh-4rem))] w-full max-w-lg flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-4 py-3.5">
          <div className="min-w-0">
            <h2 className="font-display text-display text-ink">{t("transfer.import.title")}</h2>
            <p className="text-caption text-faint">{t("transfer.import.subtitle")}</p>
          </div>
          <button onClick={close} className="icon-chip ml-auto shrink-0" title={t("common.close")} disabled={running}>
            <Icon name="close" size={15} />
          </button>
        </div>

        {report ? (
          <Report report={report} source={reportSource} backup={backup} />
        ) : (
        <div className="flex-1 space-y-5 overflow-y-auto px-4 py-4">
          <div>
            <div className="section-label mb-1.5 px-0">{t("transfer.import.from")}</div>
            <div className="flex flex-wrap gap-1.5">
              {sources.map((s) => (
                <button aria-pressed={source === s.path}
                  key={s.path}
                  onClick={() => setSource(s.path)}
                  className={`btn-solid ${source === s.path ? "seg-on" : ""}`}
                  title={s.path}
                >
                  {t("transfer.import.card", { label: s.label })}
                </button>
              ))}
              <button onClick={() => void pickSource()} className="btn-ghost">
                {sources.length === 0 ? t("transfer.import.chooseFolder") : t("transfer.import.anotherFolder")}
              </button>
            </div>
            {sources.length === 0 && !source && (
              <p className="mt-1.5 text-label text-faint">{t("transfer.import.noCard")}</p>
            )}
            {source && (
              <div className="mt-2 rounded-md bg-raised px-3 py-2">
                <div className="flex items-baseline gap-2">
                  <div className="min-w-0 flex-1 truncate font-mono text-label text-faint" title={source}>
                    {source}
                  </div>
                  <button
                    onClick={() => setReread((n) => n + 1)}
                    className="shrink-0 text-label text-accent hover:underline disabled:opacity-50"
                    disabled={previewing || running}
                  >
                    {t("transfer.import.reread")}
                  </button>
                </div>
                {previewing ? (
                  <div className="mt-1 text-caption text-muted">{t("transfer.import.reading")}</div>
                ) : preview ? (
                  <>
                    <div className="mt-1 text-caption text-ink">
                      {preview.files === 0
                        ? t("transfer.import.noPhotos")
                        : t("transfer.import.newOfTotal", {
                            newCount: newFiles,
                            count: preview.files,
                            size: humanBytes(preview.bytes ?? 0),
                          })}
                    </div>
                    {preview.already > 0 && (
                      <div className="text-label text-faint">
                        {t("transfer.import.alreadyInLibrary", { count: preview.already })}
                      </div>
                    )}
                    {preview.unreadable.length > 0 && (
                      <div
                        className="text-label text-danger"
                        title={preview.unreadable.map((p) => `${p.path}: ${p.reason}`).join("\n")}
                      >
                        {t("transfer.import.previewUnreadable", { count: preview.unreadable.length })}
                      </div>
                    )}
                    {preview.leftBehind.videos + preview.leftBehind.audio + preview.leftBehind.other > 0 && (
                      <div className="text-label text-muted" title={preview.leftBehind.examples.join("\n")}>
                        {t("transfer.import.previewLeftBehind", { what: leftBehindText(preview.leftBehind) })}
                      </div>
                    )}
                    {preview.days.length > 0 && (
                      <ul className="mt-1.5 max-h-28 space-y-0.5 overflow-y-auto font-mono text-label tabular-nums text-faint">
                        {preview.days.map((d) => (
                          <li key={d.date} className="flex gap-3">
                            <span>{d.date}</span>
                            <span className="ml-auto">
                              {d.newFiles === 0
                                ? t("transfer.import.dayAlready", { count: d.files })
                                : d.newFiles < d.files
                                  ? t("transfer.import.dayNewOf", { newCount: d.newFiles, total: d.files })
                                  : t("transfer.import.dayNew", { newCount: d.newFiles })}
                            </span>
                          </li>
                        ))}
                      </ul>
                    )}
                  </>
                ) : null}
              </div>
            )}
          </div>

          <div>
            <div className="section-label mb-1.5 px-0">{t("transfer.import.into")}</div>
            <div className="flex flex-wrap gap-1.5">
              {(lib?.folders ?? []).map((f) => (
                <button aria-pressed={destination === f.path}
                  key={f.id}
                  onClick={() => setDestination(f.path)}
                  className={`btn-solid ${destination === f.path ? "seg-on" : ""}`}
                  title={f.path}
                >
                  {folderName(f.path)}
                </button>
              ))}
              <button onClick={() => void pickDestination()} className="btn-ghost">
                {t("transfer.import.anotherFolder")}
              </button>
            </div>
            {destination && !(lib?.folders ?? []).some((f) => f.path === destination) && (
              <p className="mt-1.5 truncate text-label text-faint" title={destination}>
                {t("transfer.import.addedToLibrary", { path: destination })}
              </p>
            )}

            <div className="mt-3 flex flex-wrap gap-1.5">
              {LAYOUTS.map((l) => (
                <button aria-pressed={layout === l.id}
                  key={l.id}
                  onClick={() => setLayout(l.id)}
                  className={`btn-solid text-caption ${layout === l.id ? "seg-on" : ""}`}
                >
                  {t(l.label)}
                </button>
              ))}
            </div>
            {layout.includes("{job}") && (
              <input
                className="input mt-2"
                placeholder={t("transfer.import.jobPlaceholder")}
                value={jobName}
                onChange={(e) => setJobName(e.target.value)}
              />
            )}
            <label className="mt-2 flex items-center gap-2 text-caption text-muted">
              <input type="checkbox" checked={rename} onChange={(e) => setRename(e.target.checked)} />
              {t("transfer.import.renameToDate")}
            </label>
            <label
              className="mt-1 flex items-center gap-2 text-caption text-muted"
              title={t("transfer.import.folderKeywordsTitle")}
            >
              <input
                type="checkbox"
                checked={folderKeywords}
                onChange={(e) => setFolderKeywords(e.target.checked)}
              />
              {t("transfer.import.folderKeywords")}
            </label>
            {templates.length > 0 && (
              <label
                className="mt-1.5 flex items-center gap-2 text-caption text-muted"
                title={t("transfer.import.templateTitle")}
              >
                {t("transfer.import.template")}
                <select
                  className="input w-auto flex-1"
                  value={template ?? ""}
                  onChange={(e) => setTemplate(e.target.value ? Number(e.target.value) : null)}
                >
                  <option value="">{t("transfer.import.noTemplate")}</option>
                  {templates.map((tpl) => (
                    <option key={tpl.id} value={tpl.id}>
                      {tpl.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <p className="mt-1.5 truncate font-mono text-label text-faint">{example}</p>
            {/* Said before it happens: one folder a day. */}
            {preview && layout.includes("{yyyy-mm-dd}") && preview.days.length > 1 && (
              <p className="mt-1 text-label text-faint">{t("transfer.import.dayFolders", { count: preview.days.length })}</p>
            )}
          </div>

          <div>
            <div className="section-label mb-1.5 px-0">{t("transfer.import.backupCopy")}</div>
            {backup ? (
              <div className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate font-mono text-label text-muted" title={backup}>
                  {backup}
                </span>
                <button onClick={() => setBackup(null)} className="btn-ghost text-label">
                  {t("common.remove")}
                </button>
              </div>
            ) : null}
            {backup && backupMissing ? (
              <p className="mt-1 text-label text-warning">{t("transfer.import.backupMissing")}</p>
            ) : backup ? null : (
              <button onClick={() => void pickBackup()} className="btn-ghost">
                {t("transfer.import.backupAlso")}
              </button>
            )}
          </div>

          {running && (
            <div>
              <div className="mb-1 flex justify-between text-label text-faint">
                <span>{t("transfer.import.copying")}</span>
                <span className="font-mono tabular-nums">
                  {job && job.total > 1 ? t("common.progress", { current: job.current, total: job.total }) : t("common.starting")}
                </span>
              </div>
              <ProgressBar pct={pct} label={t("transfer.import.copying")} className="h-1 overflow-hidden rounded-full bg-base" />
            </div>
          )}
        </div>
        )}

        <div className="flex items-center gap-2 border-t border-line px-4 py-3">
          {running && report ? (
            <span className="text-label text-faint">
              {t(
                activity === "backup"
                  ? "transfer.import.backingUp"
                  : activity === "copying"
                    ? "transfer.import.copying"
                    : "transfer.import.addingToLibrary",
              )}
              {activity !== "scanning" && job && job.total > 1
                ? ` · ${t("common.progress", { current: job.current, total: job.total })}`
                : ""}
            </span>
          ) : null}
          {running && activity === "scanning" ? null : running ? (
            <button onClick={() => void api.cancelJob("import")} className="btn-ghost ml-auto">
              {t("common.stop")}
            </button>
          ) : report ? (
            <>
              {notInLibrary(report).length > 0 && (
                <button onClick={() => void start(notInLibrary(report))} className="btn-ghost">
                  {t("transfer.import.retry", { count: notInLibrary(report).length })}
                </button>
              )}
              {report.notBackedUp.length > 0 && report.shootId != null && (
                <button onClick={() => void finishBackup()} className="btn-ghost">
                  {t("transfer.import.finishBackup")}
                </button>
              )}
              {/* The next step, scoped: Cull opened on whatever Browse
                  showed, often the whole library, not the shoot just in. */}
              {report.shootId != null && report.copied > 0 && (
                <button
                  onClick={() => {
                    const id = report.shootId;
                    close();
                    void loadShoots().then(() => {
                      const shoot = useStore.getState().shoots.find((x) => x.id === id);
                      if (shoot) void useStore.getState().continueShoot(shoot);
                    });
                  }}
                  className="btn-solid ml-auto"
                >
                  {t("transfer.import.cullShoot")}
                </button>
              )}
              <button onClick={close} className={`btn-accent ${report.shootId != null && report.copied > 0 ? "" : "ml-auto"}`}>
                {t("common.done")}
              </button>
            </>
          ) : (
            <>
              <button onClick={close} className="btn-ghost ml-auto">
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void start()}
                className="btn-accent"
                disabled={!source || !destination || previewing || newFiles === 0 || (!!backup && backupMissing)}
              >
                {backup && backupMissing && newFiles > 0
                  ? t("transfer.import.importNoBackup", { count: newFiles })
                  : newFiles > 0
                    ? t("transfer.import.importCount", { count: newFiles })
                    : t("transfer.import.import")}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** The card files an import's report says are not in the library: failed,
 *  unreadable, or never reached because it was stopped. */
function notInLibrary(r: ImportOutcome): string[] {
  return [...r.failed.map((p) => p.path), ...r.unreadable.map((p) => p.path), ...r.notReached];
}

/**
 * What an import did, led by the one sentence a photographer reads
 * before formatting: safe, or not and why.
 */
function Report({ report, source, backup }: { report: ImportOutcome; source: string; backup: string | null }) {
  const missing = notInLibrary(report).length;
  const total = report.copied + report.skipped + missing;
  const left = leftBehindText(report.leftBehind);
  let verdict: { text: string; tone: "danger" | "warn" | "ok" };
  if (missing > 0) verdict = { text: t("transfer.import.notCopied", { count: missing, total }), tone: "danger" };
  else if (report.notBackedUp.length > 0)
    verdict = { text: t("transfer.import.notBackedUp", { count: report.notBackedUp.length }), tone: "danger" };
  else if (left) verdict = { text: t("transfer.import.leftBehindWarning", { what: left }), tone: "warn" };
  else
    verdict = {
      text: t(backup ? "transfer.import.safeWithBackup" : "transfer.import.safe", { count: total, source }),
      tone: "ok",
    };
  const tone = {
    danger: "border-danger/40 bg-danger/10 text-ink",
    warn: "border-line bg-raised text-ink",
    ok: "border-accent/40 bg-accent/10 text-ink",
  }[verdict.tone];
  const named = (paths: string[]): { path: string }[] => paths.map((path) => ({ path }));
  const problems = (list: ImportProblem[]) => list.map((p) => ({ path: p.path, reason: p.reason }));
  return (
    <div className="flex-1 space-y-4 overflow-y-auto px-4 py-4">
      <div role="status" className={`rounded-md border px-3 py-2.5 text-body ${tone}`}>
        {report.stopped && <span className="font-medium">{t("transfer.import.stoppedNote")} </span>}
        {verdict.text}
        {report.skipped > 0 && (
          <div className="mt-1 text-label text-faint">{t("transfer.import.alreadyCount", { count: report.skipped })}</div>
        )}
      </div>
      <ProblemList title={t("transfer.import.groupNotCopied")} items={[...problems(report.failed), ...problems(report.unreadable)]} />
      <ProblemList title={t("transfer.import.groupNotReached")} items={named(report.notReached)} />
      <ProblemList title={t("transfer.import.groupNotBackedUp")} items={problems(report.notBackedUp)} />
      {left && (
        <ProblemList
          title={`${t("transfer.import.groupLeftBehind")}: ${left}`}
          items={named(report.leftBehind.examples)}
          count={report.leftBehind.videos + report.leftBehind.audio + report.leftBehind.other}
        />
      )}
    </div>
  );
}
