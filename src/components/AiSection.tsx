import { useCallback, useEffect, useState } from "react";
import ProgressBar from "./ProgressBar";
import { inSentence, t, type Key } from "../i18n";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import { learnTagNames } from "../lib/tagNames";
import { ensureAi } from "../lib/aiReady";
import { confirm, toast } from "../lib/dialogs";
import { humanBytes } from "../lib/format";
import { isCancelled } from "../lib/jobs";
import { describePeopleRun, findPeople, peopleBacklog } from "../lib/people";
import { useStore, type AutoStep } from "../store";
import OnOff from "./OnOff";
import { Icon, type IconName } from "../lib/icons";
import type { AiModelStatus, EditServerStatus, LocalVlmStorage, VlmModelInfo } from "../bindings";
import { modelName, modelPurpose } from "../lib/coreText";

/**
 * Settings → AI: what the on-device AI can do for this library, and the few
 * decisions that are genuinely the photographer's.
 *
 * There used to be an Advanced tab behind this one: a provider dropdown, a
 * free-text model name, a server URL, runtime Start/Stop, a vision-model grid
 * graded Fits/Tight/Too large, and a parallel-requests field. None of
 * those is a photographer's decision — the app can see what the machine has —
 * and the URL sent photos to whatever machine it named, contrary to everything
 * this tab promises. What is left is three decisions:
 *
 * - whether to use on-device AI at all;
 * - whether to fetch the vision model that writes detailed descriptions (a
 *   several-gigabyte download, so it is asked for, never a default);
 * - what to delete to get disk space back.
 *
 * Everything else — which provider tags, which vision model suits the GPU, the
 * runtime and its port — is decided by the backend (`ensure_ai_ready`).
 */

/** The passes that hold the GPU and the library, one at a time. */
const HEAVY_PASSES = ["tagging", "index", "faces", "people", "caption", "denoise"];
/** What uses each on-device model, so it isn't removed from under them. */
const MODEL_PASSES: Record<string, string[]> = {
  tagger: ["index", "tagging"],
  faces: ["faces", "people"],
  people: ["faces", "people"],
  denoise: ["denoise"],
};
/** What uses or fetches the vision model and its runtime. */
const VLM_PASSES = ["caption", "tagging", "vision-runtime", "vlm-model"];

interface Capability {
  id: string;
  icon: IconName;
  name: string;
  blurb: string;
  /** Photos still waiting, or null while unknown. */
  backlog: number | null;
  /** Live progress while this capability is running, else null. Tagging reports
   *  through `store.tagging` and the rest through `store.jobs`, so the parent
   *  resolves it rather than each row guessing where to look. */
  progress: { label: string; current: number; total: number; unit?: "bytes" | "count" } | null;
  run: () => Promise<void>;
  /** Wording once the backlog is empty. */
  doneLabel: string;
  /** Wording for the action, given a non-zero backlog. */
  actionLabel: (n: number) => string;
}

/**
 * A row's action waits while another heavy pass runs — they share the GPU and
 * the library — and says what it waits for. Every button used to grey
 * out while anything at all ran, a download or a colour pass included, with
 * nothing to say why.
 */
function Row({ cap, waitingFor, off }: { cap: Capability; waitingFor: string | null; off: boolean }) {
  const job = cap.progress;
  const running = job !== null;
  const waiting = !running && waitingFor !== null;
  const pct = job && job.total > 1 ? Math.min(100, Math.round((job.current / job.total) * 100)) : 0;
  const idle = cap.backlog === 0;

  return (
    <div className={`rounded-md bg-raised p-3 ${off ? "opacity-60" : ""}`}>
      <div className="flex items-start gap-2.5">
        <Icon name={cap.icon} size={16} className="mt-0.5 shrink-0 text-faint" />
        <div className="min-w-0 flex-1">
          <div className="text-caption font-medium text-ink">{cap.name}</div>
          <div className="mt-0.5 text-label leading-relaxed text-faint">{cap.blurb}</div>
        </div>
        <button
          onClick={() => void cap.run()}
          disabled={off || waiting || running || idle}
          title={waiting ? t("settings.ai.row.waiting", { job: waitingFor }) : undefined}
          // Solid, not the key: several rows can be pending at once, and the
          // dialog already has its one key (Done).
          className="btn-solid shrink-0"
        >
          {running
            ? t("settings.ai.row.working")
            : idle
              ? cap.doneLabel
              : cap.backlog === null
                ? cap.actionLabel(0)
                : cap.actionLabel(cap.backlog)}
        </button>
      </div>

      {waiting && !off && !idle && (
        <p className="mt-2 text-label text-faint">{t("settings.ai.row.waiting", { job: waitingFor })}</p>
      )}
      {running && (
        <div className="mt-2">
          <div className="mb-1 flex items-center justify-between text-label text-faint">
            <span>{job.label}</span>
            <span className="font-mono tabular-nums">
              {/* total 1 is the placeholder a job carries before its first real
                  count arrives — a percentage there would read as a stall. */}
              {job.total <= 1
                ? t("common.starting")
                : job.unit === "bytes"
                  ? t("common.progress", { current: humanBytes(job.current), total: humanBytes(job.total) })
                  : t("common.progress", { current: job.current, total: job.total })}
            </span>
          </div>
          <ProgressBar pct={pct} label={job.label} className="h-1 overflow-hidden rounded-full bg-base" />
        </div>
      )}
    </div>
  );
}


/** One automatic pass and its switch, in the Automatic section. */
function AutoRow({
  name,
  blurb,
  on,
  disabled,
  onChange,
}: {
  name: string;
  blurb: string;
  on: boolean;
  disabled: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <div className={`flex items-start gap-3 py-1 ${disabled ? "opacity-60" : ""}`}>
      <div className="min-w-0 flex-1">
        {/* The size and weight of every other setting's name on this tab:
            these were a step larger, at body size, and unbolded. */}
        <div className="text-caption font-medium text-ink">{name}</div>
        <p className="mt-0.5 text-label leading-relaxed text-faint">{blurb}</p>
      </div>
      <OnOff label={name} on={on} disabled={disabled} onChange={onChange} />
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="mt-5">
      <div className="section-label mb-1.5 px-0">{title}</div>
      {children}
    </div>
  );
}

const mb = (n: number) => humanBytes(n * 1024 * 1024);

/**
 * How each AI app is pointed at the editing server. App names are product
 * names, so they stay as they are in every language. Claude Desktop and Codex
 * start a server as a command, so they run this program with `--mcp`, which
 * passes their messages to the open app (`mcp::bridge_stdio`) with the
 * server's token. The others connect to the address and send the token.
 */
function connections(s: EditServerStatus): { app: string; how: Key; snippet: string }[] {
  const config = (server: object) => JSON.stringify({ mcpServers: { sourcelight: server } }, null, 2);
  const headers = { Authorization: `Bearer ${s.token}` };
  return [
    {
      app: "Claude Desktop",
      how: "settings.ai.apps.how.claudeDesktop",
      snippet: config({ command: s.command, args: ["--mcp"] }),
    },
    {
      app: "Claude Code",
      how: "settings.ai.apps.how.terminal",
      snippet: `claude mcp add --transport http sourcelight ${s.url} --header "Authorization: Bearer ${s.token}"`,
    },
    { app: "Gemini CLI", how: "settings.ai.apps.how.gemini", snippet: config({ httpUrl: s.url, headers }) },
    {
      app: "Codex",
      how: "settings.ai.apps.how.terminal",
      snippet: `codex mcp add sourcelight -- "${s.command}" --mcp`,
    },
    { app: "LM Studio", how: "settings.ai.apps.how.lmStudio", snippet: config({ url: s.url, headers }) },
  ];
}

/** A snippet to paste, with a button that copies it. */
function Snippet({ text, copy }: { text: string; copy: Key }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="mt-1 flex items-start gap-2 rounded-md bg-raised px-3 py-2">
      {/* Wrapped, not scrolled: the program's path is what people check. */}
      <pre className="min-w-0 flex-1 whitespace-pre-wrap break-all font-mono text-label text-ink">{text}</pre>
      <button
        className="btn-ghost shrink-0 text-label"
        onClick={() => void navigator.clipboard.writeText(text).then(() => setCopied(true))}
      >
        {copied ? t("settings.ai.apps.copied") : t(copy)}
      </button>
    </div>
  );
}

/** The editing server (docs/AI-EDIT.md): AI apps edit the photo open in Develop. */
function EditServerRow() {
  const [status, setStatus] = useState<EditServerStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [howTo, setHowTo] = useState(false);

  useEffect(() => {
    api.editServerStatus().then(setStatus).catch(() => setStatus(null));
  }, []);

  if (!status) return null;

  const set = async (on: boolean) => {
    setBusy(true);
    try {
      setStatus(await api.setEditServer(on));
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <AutoRow
        name={t("settings.ai.apps.server")}
        blurb={t("settings.ai.apps.blurb")}
        on={status.enabled}
        disabled={busy}
        onChange={(on) => void set(on)}
      />
      {status.enabled && status.running && (
        <>
          <Snippet text={status.url} copy="settings.ai.apps.copy" />
          <p className="mt-2 text-label leading-relaxed text-faint">{t("settings.ai.apps.tokenNote")}</p>
          <Snippet text={`Authorization: Bearer ${status.token}`} copy="settings.ai.apps.copySnippet" />
          <button
            className="btn-ghost mt-2 text-label"
            aria-expanded={howTo}
            onClick={() => setHowTo((v) => !v)}
          >
            {t("settings.ai.apps.howTo")}
          </button>
          {howTo && (
            <div className="mt-1 space-y-3">
              {connections(status).map((c) => (
                <div key={c.app}>
                  <div className="text-caption font-medium text-ink">{c.app}</div>
                  <p className="text-label leading-relaxed text-faint">{t(c.how)}</p>
                  <Snippet text={c.snippet} copy="settings.ai.apps.copySnippet" />
                </div>
              ))}
              <p className="text-label leading-relaxed text-faint">{t("settings.ai.apps.chatgpt")}</p>
              <p className="text-label leading-relaxed text-faint">{t("settings.ai.apps.keepOpen")}</p>
            </div>
          )}
        </>
      )}
      {status.enabled && !status.running && status.error && (
        <p className="mt-1 text-label text-danger">{t("settings.ai.apps.notRunning", { error: status.error })}</p>
      )}
    </>
  );
}

export default function AiSection() {
  const ai = useStore((s) => s.ai);
  const refreshAi = useStore((s) => s.refreshAi);
  const untagged = useStore((s) => s.facets?.untagged ?? null);
  const photoCount = useStore((s) => s.photos.length);
  const tagging = useStore((s) => s.tagging);
  const runTagging = useStore((s) => s.runTagging);
  const runIndexing = useStore((s) => s.runIndexing);
  const jobs = useStore((s) => s.jobs);

  const off = ai?.provider === "none" && ai.configured;
  const autoAnalyse = useStore((s) => s.autoAnalyse);
  const setAutoAnalyse = useStore((s) => s.setAutoAnalyse);
  const autoSteps = useStore((s) => s.autoSteps);
  const setAutoStep = useStore((s) => s.setAutoStep);
  const holdOnBattery = useStore((s) => s.holdOnBattery);
  const showAiKeywords = useStore((s) => s.showAiKeywords);
  const setShowAiKeywords = useStore((s) => s.setShowAiKeywords);
  const setHoldOnBattery = useStore((s) => s.setHoldOnBattery);
  const detailed = ai?.provider === "vision";
  // What runs by itself, each switchable. Descriptions only exist with the
  // vision model on, and they run last and a batch at a time, so a large
  // library's hours of describing never hold up new photos' search and tags.
  const autoRows: { step: AutoStep; name: string; blurb: string }[] = [
    { step: "search", name: t("settings.ai.auto.search.name"), blurb: t("settings.ai.auto.search.blurb") },
    {
      step: "tags",
      name: t("settings.ai.auto.tags.name"),
      blurb: detailed ? t("settings.ai.auto.tags.blurbDetailed") : t("settings.ai.auto.tags.blurbBuiltin"),
    },
    { step: "faces", name: t("settings.ai.auto.faces.name"), blurb: t("settings.ai.auto.faces.blurb") },
    ...(detailed
      ? [
          {
            step: "descriptions" as const,
            name: t("settings.ai.auto.descriptions.name"),
            blurb: t("settings.ai.auto.descriptions.blurb"),
          },
        ]
      : []),
  ];

  // --- Backlogs ------------------------------------------------------------
  const [uncaptioned, setUncaptioned] = useState<number | null>(null);
  const [unindexed, setUnindexed] = useState<number | null>(null);
  const [unanalysed, setUnanalysed] = useState<number | null>(null);

  const refreshBacklogs = useCallback(() => {
    void api.captionStatus().then(setUncaptioned).catch(() => setUncaptioned(null));
    void api.searchIndexStatus().then(setUnindexed).catch(() => setUnindexed(null));
    void api.facesStatus().then((s) => setUnanalysed(peopleBacklog(s))).catch(() => setUnanalysed(null));
  }, []);

  // --- Storage -------------------------------------------------------------
  const [models, setModels] = useState<AiModelStatus[] | null>(null);
  const [vlm, setVlm] = useState<LocalVlmStorage | null>(null);
  const [pick, setPick] = useState<VlmModelInfo | null>(null);

  const refreshStorage = useCallback(() => {
    void api.aiAvailableModels().then(setModels).catch(() => setModels(null));
    void api.localVlmStorage().then(setVlm).catch(() => setVlm(null));
    // The model detailed descriptions would use on this machine — named up
    // front so turning it on is an informed choice, licence included.
    void api
      .vlmCatalogue()
      .then((c) => setPick(c.find((m) => m.recommended) ?? null))
      .catch(() => setPick(null));
  }, []);

  useEffect(() => {
    refreshBacklogs();
    refreshStorage();
  }, [refreshBacklogs, refreshStorage]);

  // Backlogs shrink as the passes run, and downloads land in storage, so
  // re-read whenever one finishes rather than leaving stale figures.
  useEffect(() => {
    // Tagging with the vision model describes as it goes, so its end changes
    // the descriptions backlog too.
    const events = ["caption-progress", "embed-progress", "faces-progress", "tag-progress"];
    const uns = [
      ...events.map((name) =>
        listen<{ phase: string }>(name, (e) => {
          if (e.payload.phase === "done") refreshBacklogs();
        }),
      ),
      listen<{ done: boolean }>("ai-download-progress", (e) => e.payload.done && refreshStorage()),
      listen<{ done: boolean }>("vision-model-progress", (e) => e.payload.done && refreshStorage()),
      listen<{ done: boolean }>("vision-runtime-progress", (e) => e.payload.done && refreshStorage()),
    ];
    return () => void Promise.all(uns).then((fns) => fns.forEach((f) => f()));
  }, [refreshBacklogs, refreshStorage]);

  // The first of `keys` that is running. A row lists its own pass first and
  // then the downloads it waits on, so turning a feature on shows the model
  // arriving in the row that asked for it, not only in Activity.
  const jobProgress = (...keys: string[]) => {
    const j = keys.map((k) => jobs[k]).find((job) => job !== undefined);
    return j ? { label: j.label, current: j.current, total: j.total, unit: j.unit } : null;
  };

  // The heavy pass running now, by name, if any: it holds the other rows'
  // actions, which contend for the same GPU and library. Downloads and the
  // light passes (colours, places, focus) hold nothing.
  const heavy = (id: string) => (id === "tagging" ? tagging !== null : jobs[id] !== undefined);
  const labelOf = (id: string) =>
    id === "tagging" ? t("settings.ai.tagging.name") : (jobs[id]?.label ?? t("settings.ai.row.anotherPass"));
  const runningPass = HEAVY_PASSES.find(heavy) ?? null;
  const waitingFor = runningPass ? inSentence(labelOf(runningPass)) : null;
  /** Why a model can't be removed now, or null: only while something uses
   *  or fetches it. */
  const inUse = (ids: string[]): string | null => {
    const id = ids.find((k) => heavy(k));
    return id ? t("settings.ai.row.inUse", { job: inSentence(labelOf(id)) }) : null;
  };

  // --- The two switches ----------------------------------------------------
  const [switching, setSwitching] = useState(false);

  const setEnabled = async (on: boolean) => {
    setSwitching(true);
    try {
      // Back on as it was: detailed descriptions too, if they were on.
      if (on) await api.turnAiOn();
      else await api.setAiSettings("none", "");
      await refreshAi();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setSwitching(false);
    }
  };

  const setDetailed = async (on: boolean) => {
    setSwitching(true);
    try {
      if (!on) {
        await api.setAiSettings("builtin", "");
        return;
      }
      await api.setAiSettings("vision", "");
      // Installs the runtime and the model after one consent dialog naming
      // both and their size. Declined or failed: put things back as they were.
      if (!(await ensureAi("captions"))) await api.setAiSettings("builtin", "");
      // Set up: describing the library starts by itself, in the background,
      // unless automatic descriptions are off.
      else void useStore.getState().analyseNewPhotos();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      await refreshAi();
      refreshStorage();
      refreshBacklogs();
      setSwitching(false);
    }
  };

  // --- Removal -------------------------------------------------------------
  const removeModel = async (m: AiModelStatus) => {
    const ok = await confirm({
      title: t("settings.ai.removeModel.title", { name: modelName(m.id, m.name) }),
      message: t("settings.ai.removeModel.message", { size: mb(m.approxMb), purpose: modelPurpose(m.id, m.purpose) }),
      confirmLabel: t("common.remove"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteAiModel(m.id);
    } catch (e) {
      toast(String(e), "error");
    }
    refreshStorage();
  };

  const vlmMb = vlm
    ? (vlm.runtimeInstalled ? vlm.runtimeMb : 0) +
      vlm.models.reduce((sum, m) => sum + Math.round((m.sizeGb ?? 0) * 1024), 0)
    : 0;

  const removeVlm = async () => {
    const ok = await confirm({
      title: t("settings.ai.removeVlm.title"),
      message: t("settings.ai.removeVlm.message", { size: mb(vlmMb) }),
      confirmLabel: t("common.remove"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.removeLocalVlm();
      toast(t("settings.ai.removeVlm.done"), "success");
    } catch (e) {
      toast(String(e), "error");
    }
    await refreshAi();
    refreshStorage();
  };

  // --- Rows ----------------------------------------------------------------
  const caps: Capability[] = [
    {
      id: "search",
      icon: "search",
      name: t("settings.ai.search.name"),
      blurb: t("settings.ai.search.blurb"),
      backlog: unindexed,
      progress: jobProgress("index", "model:tagger"),
      run: runIndexing,
      doneLabel: t("settings.ai.search.done"),
      actionLabel: (n) => (n ? t("settings.ai.search.action", { count: n }) : t("settings.ai.search.actionAll")),
    },
    {
      id: "tagging",
      icon: "bookmark",
      name: t("settings.ai.tagging.name"),
      blurb: detailed ? t("settings.ai.tagging.blurbDetailed") : t("settings.ai.tagging.blurbBuiltin"),
      backlog: untagged,
      progress: tagging
        ? {
            label: tagging.phase === "loading" ? t("settings.ai.tagging.loadingModel") : t("settings.ai.tagging.name"),
            current: tagging.current,
            total: tagging.total,
          }
        : jobProgress("model:tagger", "vision-runtime", "vlm-model"),
      run: runTagging,
      doneLabel: t("settings.ai.tagging.done"),
      actionLabel: (n) => (n ? t("settings.ai.tagging.action", { count: n }) : t("settings.ai.tagging.actionAll")),
    },
    {
      id: "faces",
      icon: "user",
      name: t("settings.ai.faces.name"),
      blurb: t("settings.ai.faces.blurb"),
      backlog: unanalysed,
      progress: jobProgress("faces", "model:faces", "model:people"),
      // The same one action as the People page, grouping included: this row
      // used to stop after the face pass, so its faces never became people.
      run: async () => {
        try {
          const result = await findPeople();
          if (result) {
            const { text, tone } = describePeopleRun(result);
            toast(text, tone);
          }
        } catch (e) {
          if (!isCancelled(e)) toast(t("settings.ai.faces.failed", { error: String(e) }), "error");
        }
      },
      doneLabel: t("settings.ai.faces.done"),
      actionLabel: (n) => (n ? t("settings.ai.faces.action", { count: n }) : t("settings.ai.faces.actionAll")),
    },
    // Offered only when it can run: a Describe button that can only answer
    // "turn something on first" was a dead end.
    ...(detailed
      ? [
          {
            id: "captions",
            icon: "pencil" as const,
            name: t("settings.ai.captions.name"),
            blurb: t("settings.ai.captions.blurb"),
            backlog: uncaptioned,
            progress: jobProgress("caption", "vision-runtime", "vlm-model"),
            run: async () => {
              if (!(await ensureAi("captions"))) return;
              // A run that stops early says why. Swallowed, a model that
              // failed on every photo looked like a button that did nothing.
              await api.startCaptioning().catch((e) => {
                if (!isCancelled(e)) toast(String(e), "error");
              });
              // Photos not yet tagged were tagged in the same pass, which
              // changes the filters' counts and the Tagging row's backlog.
              await useStore.getState().reload();
              void learnTagNames(useStore.getState().facets, true);
            },
            doneLabel: t("settings.ai.captions.done"),
            actionLabel: (n: number) =>
              n ? t("settings.ai.captions.action", { count: n }) : t("settings.ai.captions.actionAll"),
          },
        ]
      : []),
  ];

  const installed = (models ?? []).filter((m) => m.installed);
  const hasVlm = vlm !== null && (vlm.runtimeInstalled || vlm.models.length > 0);

  return (
    <div>
      {/* The one switch that decides whether any of the rest applies. */}
      <div className="flex items-start gap-3 rounded-md bg-raised p-3">
        <div className="min-w-0 flex-1">
          <div className="text-caption font-medium text-ink">{t("settings.ai.enable.title")}</div>
          <p className="mt-0.5 text-label leading-relaxed text-faint">{t("settings.ai.enable.blurb")}</p>
        </div>
        <OnOff label={t("settings.ai.enable.switch")} on={!off} disabled={switching} onChange={(v) => void setEnabled(v)} />
      </div>

      <Section title={t("settings.ai.capabilities.title")}>
        <div className="space-y-2">
          {caps.map((cap) => (
            <Row key={cap.id} cap={cap} waitingFor={waitingFor} off={off} />
          ))}
        </div>
        {photoCount === 0 && (
          <p className="mt-2 text-label leading-relaxed text-faint">
            {t("settings.ai.capabilities.noPhotos")}
          </p>
        )}
      </Section>

      <Section title={t("settings.ai.detailed.title")}>
        <div className={`flex items-start gap-3 rounded-md bg-raised p-3 ${off ? "opacity-60" : ""}`}>
          <div className="min-w-0 flex-1">
            <p className="text-label leading-relaxed text-faint">
              {t("settings.ai.detailed.blurb")}
            </p>
            {pick && (
              <p className="mt-1.5 text-label text-faint">
                <span className="text-muted">{pick.name}</span>
                {pick.sizeGb !== null && (
                  <span className="font-mono tabular-nums"> · {t("settings.ai.detailed.size", { size: pick.sizeGb })}</span>
                )}
                {" · "}
                {pick.license}
                {!detailed && !hasVlm && (
                  <span className="font-mono tabular-nums">
                    {" · "}
                    {t("settings.ai.detailed.runtime", { size: mb(vlm?.runtimeMb ?? 0) })}
                  </span>
                )}
              </p>
            )}
          </div>
          <OnOff
            label={t("settings.ai.detailed.title")}
            on={detailed}
            // Only while describing runs, which switching would cut off.
            disabled={off || switching || heavy("caption")}
            onChange={(v) => void setDetailed(v)}
          />
        </div>
      </Section>

      <Section title={t("settings.ai.auto.title")}>
        <div className={`space-y-1 rounded-md bg-raised p-3 ${off ? "opacity-60" : ""}`}>
          <AutoRow
            name={t("settings.ai.auto.analyse.name")}
            blurb={t("settings.ai.auto.analyse.blurb")}
            on={autoAnalyse}
            disabled={off}
            onChange={setAutoAnalyse}
          />
          <div className="ml-3 space-y-1 border-l border-line pl-3">
            {autoRows.map((r) => (
              <AutoRow
                key={r.step}
                name={r.name}
                blurb={r.blurb}
                on={autoSteps[r.step]}
                disabled={off || !autoAnalyse}
                onChange={(on) => setAutoStep(r.step, on)}
              />
            ))}
          </div>
          {detailed && (
            <AutoRow
              name={t("settings.ai.auto.charger.name")}
              blurb={t("settings.ai.auto.charger.blurb")}
              on={holdOnBattery}
              disabled={off || !autoAnalyse}
              onChange={setHoldOnBattery}
            />
          )}
        </div>
      </Section>

      {/* The library as the user labelled it, without the tagger's
          guesses. Confirmed keywords are the user's and stay. */}
      <Section title={t("settings.ai.apps.title")}>
        <EditServerRow />
      </Section>

      <Section title={t("settings.ai.keywords.title")}>
        {/* In a card like every other setting on the tab; bare, it sat
            flush with the section headings. */}
        <div className="rounded-md bg-raised p-3">
          <AutoRow
            name={t("settings.ai.keywords.show")}
            blurb={t("settings.ai.keywords.showBlurb")}
            on={showAiKeywords}
            disabled={false}
            onChange={(on) => void setShowAiKeywords(on)}
          />
        </div>
      </Section>

      <Section title={t("settings.ai.storage.title")}>
        {installed.length === 0 && !hasVlm ? (
          <p className="text-label leading-relaxed text-faint">
            {t("settings.ai.storage.empty")}
          </p>
        ) : (
          <div className="divide-y divide-line rounded-md bg-raised">
            {installed.map((m) => (
              <div key={m.id} className="flex items-center gap-3 px-3 py-2">
                <div className="min-w-0 flex-1">
                  {/* Named as the feature it serves, in sentence case and
                      the user's language: the core's names are English
                      title case ("Search & Auto-Tagging"). */}
                  <div className="text-caption text-ink">{modelName(m.id, m.name)}</div>
                  {/* Wrapped, not cut: "Clears colour speckle and blotchy
                      high-ISO noise in Develop, keeping edge…" ended before
                      saying what the model is, in every language. */}
                  <div className="text-label leading-snug text-faint">
                    {modelPurpose(m.id, m.purpose)} · {m.license}
                  </div>
                </div>
                <span className="shrink-0 font-mono text-label tabular-nums text-faint">{mb(m.approxMb)}</span>
                <button
                  onClick={() => void removeModel(m)}
                  disabled={inUse([`model:${m.id}`, ...(MODEL_PASSES[m.id] ?? [])]) !== null}
                  title={inUse([`model:${m.id}`, ...(MODEL_PASSES[m.id] ?? [])]) ?? undefined}
                  className="btn-ghost shrink-0 text-label"
                >
                  {t("common.remove")}
                </button>
              </div>
            ))}
            {hasVlm && vlm && (
              <div className="flex items-center gap-3 px-3 py-2">
                <div className="min-w-0 flex-1">
                  <div className="text-caption text-ink">{t("settings.ai.detailed.title")}</div>
                  <div className="truncate text-label text-faint">
                    {[
                      vlm.runtimeInstalled ? t("settings.ai.storage.runtime") : null,
                      ...vlm.models.map((m) => `${m.name} · ${m.license}`),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </div>
                </div>
                <span className="shrink-0 font-mono text-label tabular-nums text-faint">{mb(vlmMb)}</span>
                <button
                  onClick={() => void removeVlm()}
                  disabled={inUse(VLM_PASSES) !== null}
                  title={inUse(VLM_PASSES) ?? undefined}
                  className="btn-ghost shrink-0 text-label"
                >
                  {t("common.remove")}
                </button>
              </div>
            )}
          </div>
        )}
      </Section>
    </div>
  );
}
