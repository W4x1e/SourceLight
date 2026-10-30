import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({
  api: {
    coloursStatus: vi.fn(),
    readColours: vi.fn(),
    locationsStatus: vi.fn(),
    readLocations: vi.fn(),
    qualityStatus: vi.fn(),
    scoreQuality: vi.fn(),
    paletteStatus: vi.fn(),
    readPalettes: vi.fn(),
    searchIndexStatus: vi.fn(),
    indexForSearch: vi.fn(),
    ensureAiReady: vi.fn(),
    providerStatus: vi.fn(),
    startTagging: vi.fn(),
    facesStatus: vi.fn(),
    analyzeFaces: vi.fn(),
    groupPeople: vi.fn(),
    captionStatus: vi.fn(),
    startCaptioning: vi.fn(),
    getPhotos: vi.fn(),
    getFacets: vi.fn(),
    onBattery: vi.fn(),
    cancelJob: vi.fn(),
    cancelTagging: vi.fn(),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { api } from "./lib/api";
import { confirm, toast } from "./lib/dialogs";
import { DESCRIBE_BATCH, POWER_POLL_MS, useStore } from "./store";

const m = vi.mocked(api);
const ready = {
  ready: true,
  blocked: null,
  needs: [],
  askFirst: false,
  metered: false,
  provider: "local",
  model: "",
  detail: "",
};
/** Nothing set up, on a metered connection: the backend says to ask. */
const needsAsking = {
  ready: false,
  blocked: null,
  needs: [
    { label: "the Search & Auto-Tagging model", approxMb: 785 },
    { label: "the People model", approxMb: 261 },
  ],
  askFirst: true,
  metered: true,
  provider: "local",
  model: "",
  detail: "",
};
/** Nothing set up, on an ordinary connection: it just downloads. */
const needsNoQuestion = { ...needsAsking, askFirst: false, metered: false };

/** Readiness for each purpose from `before`, and from `after` once "analysis" was set up. */
function setUpWith(before: typeof needsAsking) {
  let done = false;
  m.ensureAiReady.mockImplementation(async (purpose, consented) => {
    if (purpose === "analysis" && consented) done = true;
    return done ? ready : before;
  });
}

/** Runs a pass with `purpose` held short of ready, as a download there would
 *  hold it, and calls `check` meanwhile. Every other purpose is ready. */
async function whileGettingReady(purpose: Parameters<typeof api.ensureAiReady>[0], check: () => void) {
  let release = () => {};
  const held = new Promise<typeof ready>((resolve) => {
    release = () => resolve(ready);
  });
  m.ensureAiReady.mockImplementation(async (p) => (p === purpose ? held : ready));
  const run = useStore.getState().analyseNewPhotos();
  try {
    await vi.waitFor(() => expect(m.ensureAiReady).toHaveBeenCalledWith(purpose, false));
    check();
  } finally {
    // Even when `check` fails: a run left open would be joined by every test after.
    release();
    await run;
  }
}

/** Automatic analysis after a scan. */
describe("analyseNewPhotos", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    m.coloursStatus.mockResolvedValue(0);
    m.locationsStatus.mockResolvedValue(0);
    m.qualityStatus.mockResolvedValue(0);
    m.paletteStatus.mockResolvedValue(0);
    m.searchIndexStatus.mockResolvedValue(3);
    m.facesStatus.mockResolvedValue({ toDetect: 0, toRecognise: 0, recogniserInstalled: false });
    m.indexForSearch.mockResolvedValue(undefined as never);
    m.startTagging.mockResolvedValue(undefined as never);
    m.getPhotos.mockResolvedValue([]);
    m.getFacets.mockResolvedValue({ untagged: 5 } as never);
    m.providerStatus.mockResolvedValue({ provider: "builtin", configured: true } as never);
    m.captionStatus.mockResolvedValue(0);
    m.startCaptioning.mockResolvedValue(0 as never);
    m.onBattery.mockResolvedValue(false);
    useStore.setState({
      holdOnBattery: true,
      heldForPower: false,
      autoAnalyse: true,
      autoSteps: { search: true, tags: true, faces: true, descriptions: true },
      develop: null,
      scanning: null,
      tagging: null,
      facets: { untagged: 5 } as never,
    });
  });

  it("stops the whole run on Stop, not just the step in progress", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    // Indexing, stopped part way: the backend says "Stopped" and returns.
    m.indexForSearch.mockImplementation(async () => {
      await useStore.getState().stopAnalysis();
      return 1 as never;
    });
    await useStore.getState().analyseNewPhotos();
    expect(m.cancelJob).toHaveBeenCalledWith("index");
    expect(m.startTagging).not.toHaveBeenCalled();
    expect(m.analyzeFaces).not.toHaveBeenCalled();
    expect(useStore.getState().autoAnalysis).toBeNull();

    // The next run starts afresh.
    m.indexForSearch.mockResolvedValue(undefined as never);
    await useStore.getState().analyseNewPhotos();
    expect(m.startTagging).toHaveBeenCalledOnce();
  });

  it("stops the run in progress when automatic analysis is switched off", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    m.indexForSearch.mockImplementation(async () => {
      useStore.getState().setAutoAnalyse(false);
      return 1 as never;
    });
    await useStore.getState().analyseNewPhotos();
    expect(m.startTagging).not.toHaveBeenCalled();
  });

  it("does nothing when it is switched off, unless asked to", async () => {
    useStore.setState({ autoAnalyse: false });
    await useStore.getState().analyseNewPhotos();
    expect(m.searchIndexStatus).not.toHaveBeenCalled();

    m.ensureAiReady.mockResolvedValue(ready);
    await useStore.getState().analyseNewPhotos(true);
    expect(m.indexForSearch).toHaveBeenCalledOnce();
  });

  it("remakes previews in the camera's colours first, and reloads the grid for them", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    m.coloursStatus.mockResolvedValue(40);
    m.readColours.mockResolvedValue(3);
    await useStore.getState().analyseNewPhotos();
    expect(m.readColours).toHaveBeenCalledOnce();
    // Reloaded straight after, before the slower passes start.
    const reload = m.getPhotos.mock.invocationCallOrder[0]!;
    expect(m.readColours.mock.invocationCallOrder[0]).toBeLessThan(reload);
    expect(reload).toBeLessThan(m.indexForSearch.mock.invocationCallOrder[0]!);
  });

  it("indexes, then tags, when both are already set up", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    await useStore.getState().analyseNewPhotos();
    expect(m.indexForSearch).toHaveBeenCalledOnce();
    expect(m.startTagging).toHaveBeenCalledOnce();
    expect(m.indexForSearch.mock.invocationCallOrder[0]).toBeLessThan(
      m.startTagging.mock.invocationCallOrder[0]!,
    );
    expect(useStore.getState().autoAnalysis).toBeNull();
  });

  it("never picks an AI provider for someone who has not chosen one", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    m.providerStatus.mockResolvedValue({ provider: "none", configured: false } as never);
    await useStore.getState().analyseNewPhotos();
    expect(m.startTagging).not.toHaveBeenCalled();
    expect(m.ensureAiReady).not.toHaveBeenCalledWith("tagging", expect.anything());
  });

  it("sets everything up in one download, without a question, on an ordinary connection", async () => {
    setUpWith(needsNoQuestion);
    await useStore.getState().analyseNewPhotos();
    expect(confirm).not.toHaveBeenCalled();
    expect(toast).toHaveBeenCalledWith(expect.stringContaining("1.0 GB"), "info");
    expect(m.ensureAiReady).toHaveBeenCalledWith("analysis", true);
    expect(m.indexForSearch).toHaveBeenCalledOnce();
  });

  it("asks first on a metered connection, once, and only downloads on a yes", async () => {
    setUpWith(needsAsking);
    vi.mocked(confirm).mockResolvedValue(false);
    await useStore.getState().analyseNewPhotos();
    expect(confirm).toHaveBeenCalledOnce();
    expect(vi.mocked(confirm).mock.calls[0]![0].message).toContain("metered");
    expect(m.ensureAiReady).not.toHaveBeenCalledWith("analysis", true);
    expect(m.indexForSearch).not.toHaveBeenCalled();

    // Declined once is declined: the next scan does not ask again.
    await useStore.getState().analyseNewPhotos();
    expect(confirm).toHaveBeenCalledOnce();
  });

  it("tries a failed setup download again on the next scan", async () => {
    m.ensureAiReady.mockImplementation(async (purpose, consented) => {
      if (purpose === "analysis" && consented) throw new Error("network is down");
      return needsNoQuestion;
    });
    await useStore.getState().analyseNewPhotos();
    await useStore.getState().analyseNewPhotos();
    expect(m.ensureAiReady.mock.calls.filter(([p, c]) => p === "analysis" && c)).toHaveLength(2);
  });

  it("waits for Develop to close before touching the GPU", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    useStore.setState({ develop: {} as never });
    const run = useStore.getState().analyseNewPhotos();
    await new Promise((r) => setTimeout(r, 20));
    expect(m.indexForSearch).not.toHaveBeenCalled();
    useStore.setState({ develop: null });
    await run;
    expect(m.indexForSearch).toHaveBeenCalledOnce();
  });

  it("joins a run in progress rather than starting a second", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    const a = useStore.getState().analyseNewPhotos();
    const b = useStore.getState().analyseNewPhotos();
    await Promise.all([a, b]);
    // The second trigger reruns the pass once, after the first — never in
    // parallel with it.
    expect(m.indexForSearch).toHaveBeenCalledTimes(2);
  });

  it("skips a pass that is switched off, and runs the rest", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    useStore.setState({ autoSteps: { search: false, tags: true, faces: true, descriptions: true } });
    await useStore.getState().analyseNewPhotos();
    expect(m.indexForSearch).not.toHaveBeenCalled();
    expect(m.startTagging).toHaveBeenCalledOnce();
  });

  it("describes the library a batch at a time, until it is done", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    m.providerStatus.mockResolvedValue({ provider: "vision", configured: true } as never);
    // Each batch describes some; the pass goes round again while any remain.
    const left = [45, 25, 25, 5, 5, 0];
    m.captionStatus.mockImplementation(async () => left.shift() ?? 0);
    await useStore.getState().analyseNewPhotos();
    expect(m.startCaptioning).toHaveBeenCalledTimes(3);
    expect(m.startCaptioning).toHaveBeenCalledWith(DESCRIBE_BATCH);
    // Describing comes after the quicker passes.
    expect(m.startTagging.mock.invocationCallOrder[0]).toBeLessThan(
      m.startCaptioning.mock.invocationCallOrder[0]!,
    );
  });

  it("counts descriptions against the whole run, not the batch", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    m.providerStatus.mockResolvedValue({ provider: "vision", configured: true } as never);
    const left = [45, 25, 25, 5, 5, 0];
    m.captionStatus.mockImplementation(async () => left.shift() ?? 0);
    const seen: unknown[] = [];
    m.startCaptioning.mockImplementation(async () => {
      seen.push(useStore.getState().describeRun);
      return 0 as never;
    });
    await useStore.getState().analyseNewPhotos();
    expect(seen).toEqual([
      { done: 0, total: 45 },
      { done: 20, total: 45 },
      { done: 40, total: 45 },
    ]);
    expect(useStore.getState().describeRun).toBeNull();
  });

  it("stops describing when a batch gets nowhere", async () => {
    // Photos that cannot be read stay queued: they must not loop forever.
    m.ensureAiReady.mockResolvedValue(ready);
    m.providerStatus.mockResolvedValue({ provider: "vision", configured: true } as never);
    m.captionStatus.mockResolvedValue(5);
    await useStore.getState().analyseNewPhotos();
    expect(m.startCaptioning).toHaveBeenCalledOnce();
  });

  it("describes nothing without the vision model, or with descriptions switched off", async () => {
    m.ensureAiReady.mockResolvedValue(ready);
    m.captionStatus.mockResolvedValue(40);
    await useStore.getState().analyseNewPhotos();
    expect(m.startCaptioning).not.toHaveBeenCalled();

    m.providerStatus.mockResolvedValue({ provider: "vision", configured: true } as never);
    useStore.setState({ autoSteps: { search: true, tags: true, faces: true, descriptions: false } });
    await useStore.getState().analyseNewPhotos();
    expect(m.startCaptioning).not.toHaveBeenCalled();
  });

  it("says it is describing while the vision model gets ready, not the step before", async () => {
    // With a yes on file, getting ready can download the llama.cpp runtime and
    // the model right there: half a minute, or minutes.
    m.providerStatus.mockResolvedValue({ provider: "vision", configured: true } as never);
    m.captionStatus.mockResolvedValue(5);
    await whileGettingReady("captions", () => {
      expect(useStore.getState().autoAnalysis).toBe("descriptions");
      expect(m.startCaptioning).not.toHaveBeenCalled();
    });
    expect(m.startCaptioning).toHaveBeenCalledOnce();
  });

  it("says it is tagging while the vision model gets ready for tags", async () => {
    m.providerStatus.mockResolvedValue({ provider: "vision", configured: true } as never);
    await whileGettingReady("tagging", () => {
      expect(useStore.getState().autoAnalysis).toBe("tags");
      expect(m.startTagging).not.toHaveBeenCalled();
    });
    expect(m.startTagging).toHaveBeenCalledOnce();
  });

  describe("on battery", () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
      m.ensureAiReady.mockResolvedValue(ready);
      m.providerStatus.mockResolvedValue({ provider: "vision", configured: true } as never);
      m.captionStatus.mockResolvedValue(40);
      m.onBattery.mockResolvedValue(true);
    });
    afterEach(() => {
      // Ends any watch for the charger a test left running.
      useStore.getState().setHoldOnBattery(false);
      vi.useRealTimers();
    });

    it("holds the vision model for the charger, and carries on with the rest", async () => {
      await useStore.getState().analyseNewPhotos();
      expect(m.indexForSearch).toHaveBeenCalledOnce();
      expect(m.startTagging).not.toHaveBeenCalled();
      expect(m.startCaptioning).not.toHaveBeenCalled();
      expect(useStore.getState().heldForPower).toBe(true);

      // Still on battery a minute later: still waiting.
      await vi.advanceTimersByTimeAsync(POWER_POLL_MS);
      expect(m.startCaptioning).not.toHaveBeenCalled();

      // Plugged in: it picks up where it stopped.
      m.onBattery.mockResolvedValue(false);
      m.captionStatus.mockResolvedValueOnce(40).mockResolvedValue(0);
      await vi.advanceTimersByTimeAsync(POWER_POLL_MS);
      await vi.waitFor(() => expect(m.startCaptioning).toHaveBeenCalledOnce());
      expect(m.startTagging).toHaveBeenCalledOnce();
      expect(useStore.getState().heldForPower).toBe(false);
    });

    it("keeps tagging with the built-in model", async () => {
      m.providerStatus.mockResolvedValue({ provider: "builtin", configured: true } as never);
      await useStore.getState().analyseNewPhotos();
      expect(m.startTagging).toHaveBeenCalledOnce();
      expect(useStore.getState().heldForPower).toBe(false);
    });

    it("runs everything when someone presses Analyse now", async () => {
      m.captionStatus.mockResolvedValueOnce(40).mockResolvedValue(0);
      await useStore.getState().analyseNewPhotos(true);
      expect(m.startCaptioning).toHaveBeenCalledOnce();
      expect(m.onBattery).not.toHaveBeenCalled();
    });

    it("runs what waited as soon as the hold is switched off", async () => {
      await useStore.getState().analyseNewPhotos();
      expect(m.startCaptioning).not.toHaveBeenCalled();
      m.captionStatus.mockResolvedValueOnce(40).mockResolvedValue(0);
      useStore.getState().setHoldOnBattery(false);
      await vi.waitFor(() => expect(m.startCaptioning).toHaveBeenCalledOnce());
    });
  });
});
