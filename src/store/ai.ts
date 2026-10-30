import type { StoreApi } from "zustand";
import { api } from "../lib/api";
import { downloadSize, ensureAi, errorText } from "../lib/aiReady";
import { confirm, toast } from "../lib/dialogs";
import { t } from "../i18n";
import { learnTagNames } from "../lib/tagNames";
import { canCancel, cancelJob, isCancelled } from "../lib/jobs";
import { facesBacklog } from "../lib/people";
import type { Progress, ProviderStatus } from "../types";
import type { Slice, State } from "./types";

/**
 * Background jobs and AI: tagging, the search index, and the automatic analysis
 * that runs after a scan.
 */

/** The analysis steps, in the order they run. */
export type AnalysisStep =
  | "colours"
  | "locations"
  | "focus"
  | "palette"
  | "search"
  | "tags"
  | "faces"
  | "descriptions";

/** The automatic passes a person can switch off one by one, in Settings › AI.
 *  Colours, locations and focus are not among them: no model, and nearly
 *  free. */
export type AutoStep = "search" | "tags" | "faces" | "descriptions";

/** Photos described per automatic batch. Between batches the pass starts
 *  again from the top, so a new import's quicker steps — search, tags, faces —
 *  wait a batch (a couple of minutes), not a library's worth of descriptions. */
export const DESCRIBE_BATCH = 20;

/** The run in progress, so a second trigger joins it rather than racing it. */
let analysisRun: Promise<void> | null = null;
/** A trigger arrived mid-run: photos may have landed after a step had passed. */
let analysisAgain = false;
/** Set by Stop: ends the whole run, not just the step in progress. */
let analysisStopped = false;
/** Photos described so far this run, across its batches. */
let describedThisRun = 0;

/** The background job each analysis step reports its progress under. Tags
 *  report through the tagging state. */
export const ANALYSIS_JOB: Record<AnalysisStep, string> = {
  colours: "colours",
  locations: "locations",
  focus: "quality",
  palette: "palette",
  search: "index",
  tags: "tagging",
  faces: "faces",
  descriptions: "caption",
};
/** Remembered per machine: automatic analysis sets AI up once, not per scan. */
const OFFERED_KEY = "northlight.autoAnalyseOffered";

/** Resolves once `pred` holds of the store — immediately if it already does. */
function waitUntil(store: StoreApi<State>, pred: (s: State) => boolean): Promise<void> {
  if (pred(store.getState())) return Promise.resolve();
  return new Promise((resolve) => {
    const un = store.subscribe((s) => {
      if (pred(s)) {
        un();
        resolve();
      }
    });
  });
}

/**
 * Whether `purpose` can run now, without asking anything. Automatic analysis
 * never configures AI behind the user's back or picks a provider: its own
 * download is `setUpAnalysisOnce`'s, and the one here — the vision model and
 * its runtime, when missing — needs a yes already on file and an unmetered
 * connection.
 */
async function preparedFor(purpose: "search" | "tagging" | "faces" | "captions"): Promise<boolean> {
  try {
    return (await api.ensureAiReady(purpose, false)).ready;
  } catch {
    return false;
  }
}

/**
 * Sets up everything automatic analysis uses — search and tags, and People —
 * the first time there is something to analyse, because "new photos are
 * searchable" is the whole point. One download, not one per feature met
 * later. It just starts, with a toast, unless the backend says to ask first (a
 * metered connection); a "Not now" or a cancel is remembered. A download that
 * fails is tried again on the next scan.
 */
async function setUpAnalysisOnce(): Promise<void> {
  let state;
  try {
    state = await api.ensureAiReady("analysis", false);
  } catch {
    return;
  }
  if (state.ready || state.blocked || state.needs.length === 0) return;
  const remember = (on: boolean) => {
    try {
      if (on) localStorage.setItem(OFFERED_KEY, "1");
      else localStorage.removeItem(OFFERED_KEY);
    } catch {
      // Storage unavailable: asking again next time is the lesser evil.
    }
  };
  try {
    if (localStorage.getItem(OFFERED_KEY) === "1") return;
  } catch {
    // As above.
  }
  remember(true);
  const size = downloadSize(state.needs.reduce((sum, r) => sum + r.approxMb, 0));
  if (state.askFirst) {
    const agreed = await confirm({
      title: t("store.aiOffer.title"),
      message: t(state.metered ? "store.aiOffer.messageMetered" : "store.aiOffer.message", { size }),
      confirmLabel: t("store.aiOffer.confirm"),
      cancelLabel: t("store.dialog.notNow"),
    });
    if (!agreed) return;
  } else {
    toast(t("store.aiOffer.settingUp", { size }), "info");
  }
  try {
    await api.ensureAiReady("analysis", true);
  } catch (e) {
    if (isCancelled(e)) return;
    toast(errorText(e), "error");
    remember(false);
  }
}

/** How often a pass held for the charger looks again. */
export const POWER_POLL_MS = 60_000;
let powerWatch: ReturnType<typeof setInterval> | null = null;

/** Looks for the charger while vision-model work is held, and resumes
 *  automatic analysis when it is plugged in (or the hold is switched off). */
function watchForPower(set: (partial: Partial<State>) => void, get: () => State) {
  if (powerWatch) return;
  powerWatch = setInterval(() => {
    void api
      .onBattery()
      .catch(() => false)
      .then((battery) => {
        if (battery && get().holdOnBattery && get().heldForPower) return;
        stopWatchingPower();
        if (!get().heldForPower) return;
        set({ heldForPower: false });
        void get().analyseNewPhotos();
      });
  }, POWER_POLL_MS);
}

function stopWatchingPower() {
  if (powerWatch) clearInterval(powerWatch);
  powerWatch = null;
}

/**
 * One pass of automatic analysis over whatever has not had it yet: focus,
 * then the search index, then tags (which read the search embedding, so they
 * come after), then faces. Each step is an existing job, so it shows and can be
 * cancelled in Activity; a cancel ends the whole pass. It holds while Develop
 * is open or another job runs — they contend for the same GPU.
 */
/** One pass. Resolves true when descriptions are still waiting and the last
 *  batch made progress, so the caller runs another pass. `force` is someone
 *  pressing Analyse now: it runs even on battery. */
async function runAnalysis(
  set: (partial: Partial<State>) => void,
  get: () => State,
  store: StoreApi<State>,
  force = false,
): Promise<boolean> {
  const auto = get().autoSteps;
  set({ heldForPower: false });
  // The vision model is seconds of GPU a photo: a laptop describing its
  // library on battery would be flat by the end of it. Its passes wait for the
  // charger; the rest are fractions of a second a photo and carry on. Asked
  // once a pass, and only when there is vision-model work to hold.
  let battery: boolean | undefined;
  const waitForPower = async () => {
    if (force || !get().holdOnBattery) return false;
    battery ??= await api.onBattery().catch(() => false);
    if (battery) {
      set({ heldForPower: true });
      watchForPower(set, get);
    }
    return battery;
  };
  const idle = () =>
    waitUntil(store, (s) => s.develop === null && s.scanning === null && s.tagging === null);
  const step = async (name: AnalysisStep, run: () => Promise<unknown>): Promise<boolean> => {
    await idle();
    // Stopped while waiting, or by the step before: nothing more starts. A
    // stopped step returned normally, so the run went straight on.
    if (analysisStopped) return false;
    set({ autoAnalysis: name });
    try {
      await run();
      return !analysisStopped;
    } catch (e) {
      if (!isCancelled(e)) console.error(`automatic ${name} failed`, e);
      return false;
    }
  };
  // Getting the vision model ready is the long part, not a formality: it loads
  // the model, first downloading whatever is missing when a yes on file allows
  // — the llama.cpp runtime and the model, half a minute or minutes.
  // The step is named from the start of that, not left naming the one before.
  const prepare = async (name: AnalysisStep, purpose: Parameters<typeof preparedFor>[0]) => {
    if (analysisStopped) return false;
    set({ autoAnalysis: name });
    return (await preparedFor(purpose)) && !analysisStopped;
  };

  // Colours: previews of photos a camera recorded in Adobe RGB, made before
  // that was read, show them flat. First, since it is what the grid shows;
  // new scans get it right as they go. Reloaded after, for the new previews.
  if ((await api.coloursStatus().catch(() => 0)) > 0) {
    let remade = 0;
    const ok = await step("colours", async () => {
      remade = await api.readColours();
    });
    if (!ok) return false;
    if (remade > 0) await get().reload();
  }

  // Locations: no model, just the start of each file. New scans read them as
  // they go; this catches photos catalogued before positions were read.
  if ((await api.locationsStatus().catch(() => 0)) > 0) {
    if (!(await step("locations", () => api.readLocations()))) return false;
  }

  // Focus: no model at all. Scans score as they go; this catches stragglers.
  if ((await api.qualityStatus().catch(() => 0)) > 0) {
    if (!(await step("focus", () => api.scoreQuality()))) return false;
  }

  // Colours: no model either — each photo's thumbnail, or a small
  // render of it once edited. New and edited photos are picked up here.
  if ((await api.paletteStatus().catch(() => 0)) > 0) {
    if (!(await step("palette", () => api.readPalettes()))) return false;
  }

  const toIndex = auto.search ? await api.searchIndexStatus().catch(() => 0) : 0;
  const faces = auto.faces ? facesBacklog(await api.facesStatus().catch(() => null)) : 0;
  if (toIndex > 0 || faces > 0) await setUpAnalysisOnce();
  if (analysisStopped) return false;

  if (toIndex > 0 && (await preparedFor("search"))) {
    if (!(await step("search", () => api.indexForSearch()))) return false;
  }

  // Tags only under a provider the user has already chosen: asking the backend
  // whether tagging is ready would otherwise pick one for them.
  await get().refreshAi();
  const ai = get().ai;
  if (auto.tags && ai?.configured && ai.provider !== "none" && (get().facets?.untagged ?? 0) > 0) {
    if (!(ai.provider === "vision" && (await waitForPower())) && (await prepare("tags", "tagging"))) {
      if (!(await step("tags", () => api.startTagging()))) return false;
      await get().reload();
      void learnTagNames(get().facets, true);
    }
  }

  if (
    auto.faces &&
    facesBacklog(await api.facesStatus().catch(() => null)) > 0 &&
    (await preparedFor("faces"))
  ) {
    // Analysing without regrouping left People showing the groups from
    // before the new faces arrived, until someone opened the page.
    if (await step("faces", () => api.analyzeFaces())) await api.groupPeople().catch(() => undefined);
  }

  // Descriptions last, and a batch at a time: they are the slow pass (seconds
  // a photo), and only with Detailed descriptions on. Turning that on is what
  // fetches the model, as a rule. Not always: with a yes on file and an
  // unmetered connection, whatever is missing downloads right here — on an
  // install upgraded from Ollama, the llama.cpp runtime and the model both.
  if (auto.descriptions && ai?.provider === "vision") {
    const before = await api.captionStatus().catch(() => 0);
    if (before > 0 && !(await waitForPower()) && (await prepare("descriptions", "captions"))) {
      // Counted again each batch, so photos imported during the run join the total.
      set({ describeRun: { done: describedThisRun, total: describedThisRun + before } });
      if (!(await step("descriptions", () => api.startCaptioning(DESCRIBE_BATCH)))) return false;
      // Describing tags any photo it reaches untagged, in the same request.
      await get().reload();
      void learnTagNames(get().facets, true);
      const after = await api.captionStatus().catch(() => 0);
      describedThisRun += Math.max(0, before - after);
      // Only while a batch gets somewhere: photos that cannot be read stay
      // queued, and must not keep a loop going over them.
      return after > 0 && after < before;
    }
  }
  return false;
}

/** A background job's progress, as the Activity panel and the rail pulse read it. */
export interface Job {
  label: string;
  current: number;
  total: number;
  /** `bytes` formats as "1.4 GB of 6.0 GB"; `count` as "312 / 1,240". */
  unit: "bytes" | "count";
  /** Why it failed, for a download that did: the row stays, saying
   *  so, until dismissed — it used to vanish. */
  error?: string;
}

export interface AiSlice {
  /**
   * Long-running background jobs, keyed by id.
   *
   * Model downloads, the vision runtime and model downloads, captioning, CLIP
   * indexing, face analysis and focus scoring all used to live in the component
   * that started them — so closing Settings made a multi-gigabyte download
   * invisible everywhere in the app, and the rail's activity pulse only ever
   * knew about scan, tagging and export. Held here so anything can see them.
   */
  jobs: Record<string, Job>;
  setJob: (id: string, job: Job | null) => void;
  tagging: Progress | null;
  ai: ProviderStatus | null;
  setTagging: (p: Progress | null) => void;
  refreshAi: () => Promise<void>;
  /** Analyse new photos after every scan that adds some. Persisted. */
  autoAnalyse: boolean;
  setAutoAnalyse: (on: boolean) => void;
  /** The automatic-analysis step running now, if any. */
  autoAnalysis: AnalysisStep | null;
  /** Automatic descriptions run in batches of `DESCRIBE_BATCH`. This is the
   *  whole run so far: photos described before the current batch, and the
   *  total including the photos still waiting. The progress bar counts
   *  against it rather than against the batch. */
  describeRun: { done: number; total: number } | null;
  /** Runs the analysis pass over photos that have not had it. `force` runs it
   *  even with automatic analysis off — "Analyse now". */
  analyseNewPhotos: (force?: boolean) => Promise<void>;
  /** Stops the analysis run: the step in progress, and every step after it.
   *  What's done is kept. */
  stopAnalysis: () => Promise<void>;
  /** Which automatic passes run, each switchable in Settings › AI. */
  autoSteps: Record<AutoStep, boolean>;
  setAutoStep: (step: AutoStep, on: boolean) => void;
  /** Hold the vision model's automatic passes while on battery (Settings › AI). */
  holdOnBattery: boolean;
  setHoldOnBattery: (on: boolean) => void;
  /** Automatic analysis has vision-model work waiting for the charger. */
  heldForPower: boolean;
  runTagging: () => Promise<void>;
  retagAll: () => Promise<void>;
  cancelTagging: () => void;
  /** Embeds the library for semantic search, setting up CLIP if needed. Lives
   *  in the store because it is a whole-library job the Activity panel owns —
   *  it used to be buried in Settings, three clicks from anything. */
  runIndexing: () => Promise<void>;
}

export const createAiSlice: Slice<AiSlice> = (set, get, store) => ({
  tagging: null,
  ai: null,

  jobs: {},
  setJob: (id, job) =>
    set((s) => {
      const next = { ...s.jobs };
      if (job) next[id] = job;
      else delete next[id];
      return { jobs: next };
    }),
  setTagging: (p) => set({ tagging: p }),

  refreshAi: async () => {
    set({ ai: await api.providerStatus() });
  },

  autoAnalyse: true,
  setAutoAnalyse: (on) => {
    set({ autoAnalyse: on });
    // Off stops the run in progress too, not only the next one.
    if (!on) void get().stopAnalysis();
    // Turning it on catches up on whatever arrived while it was off.
    if (on) void get().analyseNewPhotos();
  },
  autoSteps: { search: true, tags: true, faces: true, descriptions: true },
  setAutoStep: (step, on) => {
    set({ autoSteps: { ...get().autoSteps, [step]: on } });
    if (on) void get().analyseNewPhotos();
  },
  holdOnBattery: true,
  setHoldOnBattery: (on) => {
    set({ holdOnBattery: on });
    // Switched off while work waits: run it now.
    if (!on && get().heldForPower) {
      stopWatchingPower();
      set({ heldForPower: false });
      void get().analyseNewPhotos();
    }
  },
  heldForPower: false,
  autoAnalysis: null,
  describeRun: null,
  stopAnalysis: async () => {
    if (!analysisRun) return;
    analysisStopped = true;
    analysisAgain = false;
    const step = get().autoAnalysis;
    if (step === "tags") get().cancelTagging();
    else if (step && canCancel(ANALYSIS_JOB[step])) await cancelJob(ANALYSIS_JOB[step]).catch(() => undefined);
  },
  analyseNewPhotos: async (force = false) => {
    if (!force && !get().autoAnalyse) return;
    if (analysisRun) {
      analysisAgain = true;
      return analysisRun;
    }
    analysisStopped = false;
    describedThisRun = 0;
    analysisRun = (async () => {
      try {
        do {
          analysisAgain = false;
          // Another pass while descriptions remain, unless automatic
          // analysis was switched off in the meantime.
          if ((await runAnalysis(set, get, store, force)) && (force || get().autoAnalyse)) analysisAgain = true;
        } while (analysisAgain && !analysisStopped);
      } finally {
        set({ autoAnalysis: null, describeRun: null });
        analysisRun = null;
      }
    })();
    return analysisRun;
  },

  runTagging: async () => {
    // Set up whatever tagging needs first, rather than disabling the button
    // until someone finds Settings and does it by hand.
    if (!(await ensureAi("tagging"))) return;
    set({
      error: null,
      tagging: { phase: "loading", current: 0, total: 0, message: t("store.tagging.preparing") },
    });
    try {
      await api.startTagging();
    } catch (e) {
      // A missing key, a wrong model id or a revoked credential all land
      // here. Silence would look identical to "nothing needed tagging".
      set({ error: String(e) });
    } finally {
      set({ tagging: null });
      void get().refreshAi();
      await get().reload();
      // The model that wrote any new tags is still loaded: name them now.
      void learnTagNames(get().facets, true);
    }
  },

  retagAll: async () => {
    // Says what re-tagging replaces before it starts. The run cannot be
    // undone photo by photo, so the backend backs the catalogue up first —
    // and says so here, so the way back is known before it is needed.
    const ok = await confirm({
      title: t("store.tagging.retagTitle"),
      message: t("store.tagging.retagMessage"),
      confirmLabel: t("store.tagging.retagConfirm"),
    });
    if (!ok) return;
    // AI setup is asked for before anything is reset, so declining it leaves
    // the library exactly as it was rather than queued with nothing running.
    if (!(await ensureAi("tagging"))) return;
    try {
      await api.resetTagging();
    } catch (e) {
      // Includes a failed backup, which stops the reset by design.
      set({ error: String(e) });
      return;
    }
    await get().runTagging();
  },

  runIndexing: async () => {
    if (!(await ensureAi("search"))) return;
    set({ error: null });
    try {
      await api.indexForSearch();
    } catch (e) {
      set({ error: String(e) });
    }
  },

  cancelTagging: () => void api.cancelTagging(),
});
