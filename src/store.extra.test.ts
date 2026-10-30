import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({
  api: {
    getPhotos: vi.fn(),
    getFacets: vi.fn(),
    getExtraFacets: vi.fn(),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { api } from "./lib/api";
import { EXTRA_SECTIONS, extraGroups, useStore } from "./store";

const m = vi.mocked(api);
const EXTRA = {
  cameras: [{ value: "A7", count: 2 }],
  lenses: [],
  focalRanges: [],
  isoRanges: [],
  apertureRanges: [],
  fileKinds: [],
  shapes: [],
  faceCounts: [],
  palette: [],
  people: [],
  edited: 0,
  copies: 0,
  withFaces: 0,
  located: 0,
  unplaced: 0,
};

beforeEach(() => {
  vi.clearAllMocks();
  m.getPhotos.mockResolvedValue([]);
  m.getFacets.mockResolvedValue({} as never);
  m.getExtraFacets.mockResolvedValue(EXTRA);
  useStore.setState({ railSections: {}, extraFacets: null });
});

describe("the rail's optional counts", () => {
  it("are asked for only by the sections that are open", () => {
    expect(extraGroups({})).toEqual({ gear: false, people: false, status: false, kind: false, palette: false });
    expect(extraGroups({ [EXTRA_SECTIONS.gear]: true, [EXTRA_SECTIONS.status]: false })).toEqual({
      gear: true,
      people: false,
      status: false,
      kind: false,
      palette: false,
    });
    expect(extraGroups({ [EXTRA_SECTIONS.kind]: true }).kind).toBe(true);
  });

  it("cost nothing while every such section is closed", async () => {
    await useStore.getState().reload();
    expect(m.getFacets).toHaveBeenCalled();
    expect(m.getExtraFacets).not.toHaveBeenCalled();
  });

  it("are counted as their section opens, and again on every reload", async () => {
    useStore.getState().setRailSection(EXTRA_SECTIONS.gear, true);
    await vi.waitFor(() => expect(useStore.getState().extraFacets).toEqual(EXTRA));
    expect(m.getExtraFacets).toHaveBeenLastCalledWith(expect.anything(), { gear: true, people: false, status: false, kind: false, palette: false });

    await useStore.getState().reload();
    await vi.waitFor(() => expect(m.getExtraFacets).toHaveBeenCalledTimes(2));
  });

  it("aren't re-counted for a section that isn't theirs", () => {
    useStore.getState().setRailSection("Genre", true);
    expect(m.getExtraFacets).not.toHaveBeenCalled();
  });

  it("keep only the latest answer when two overlap", async () => {
    let first: (v: typeof EXTRA) => void = () => {};
    m.getExtraFacets.mockImplementationOnce(() => new Promise((r) => (first = r)));
    useStore.setState({ railSections: { [EXTRA_SECTIONS.people]: true } });
    const slow = useStore.getState().refreshExtraFacets();
    const later = { ...EXTRA, edited: 9 };
    m.getExtraFacets.mockResolvedValueOnce(later);
    await useStore.getState().refreshExtraFacets();
    first(EXTRA);
    await slow;
    expect(useStore.getState().extraFacets).toEqual(later);
  });
});

describe("reloading the grid", () => {
  it("keeps only the latest answer when two overlap", async () => {
    // The two queries run off the main thread, so the first can finish last.
    const stale = [{ id: 1 }] as never;
    const fresh = [{ id: 2 }] as never;
    let first: (v: never) => void = () => {};
    m.getPhotos.mockImplementationOnce(() => new Promise((r) => (first = r)));
    const slow = useStore.getState().reload();
    m.getPhotos.mockResolvedValueOnce(fresh);
    await useStore.getState().reload();
    first(stale);
    await slow;
    expect(useStore.getState().photos).toEqual(fresh);
    expect(useStore.getState().loading).toBe(false);
  });
});

describe("scan progress", () => {
  const progress = { phase: "scan", current: 73, total: 73, message: "DSC01096.ARW" };

  it("moves the bar while a scan is in flight", () => {
    useStore.setState({ scanning: { phase: "scan", current: 0, total: 0, message: "Starting" } });
    useStore.getState().scanProgress(progress);
    expect(useStore.getState().scanning).toEqual(progress);
  });

  it("ignores an event that arrives after the scan finished", () => {
    // The reply cleared it; a late event must not set it again, or automatic
    // analysis waits for an idle moment that never comes.
    useStore.setState({ scanning: null });
    useStore.getState().scanProgress(progress);
    expect(useStore.getState().scanning).toBeNull();
  });
});

describe("the File & shape section", () => {
  it("is counted when it is the only section open", async () => {
    useStore.getState().setRailSection(EXTRA_SECTIONS.gear, false);
    useStore.getState().setRailSection(EXTRA_SECTIONS.kind, true);
    await vi.waitFor(() =>
      expect(m.getExtraFacets).toHaveBeenLastCalledWith(expect.anything(), {
        gear: false,
        people: false,
        status: false,
        kind: true,
        palette: false,
      }),
    );
  });
});
