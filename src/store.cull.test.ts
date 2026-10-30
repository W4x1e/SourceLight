import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({
  api: {
    getPhotos: vi.fn(),
    getFacets: vi.fn(),
    createCollection: vi.fn(),
    addToCollection: vi.fn(),
    listCollections: vi.fn(),
    exportBatch: vi.fn(),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { api } from "./lib/api";
import { toast } from "./lib/dialogs";
import { useStore } from "./store";

const m = vi.mocked(api);

beforeEach(() => {
  vi.clearAllMocks();
  m.createCollection.mockResolvedValue(7);
  m.addToCollection.mockResolvedValue(null as never);
  m.listCollections.mockResolvedValue([]);
  useStore.setState({ showExport: false, selected: new Set() });
});

describe("Cull's exits", () => {
  it("exports the selection through the full dialog, not fixed settings", async () => {
    await useStore.getState().exportSelection();
    expect(useStore.getState().showExport).toBe(false);

    useStore.setState({ selected: new Set([1, 2]) });
    await useStore.getState().exportSelection();
    expect(useStore.getState().showExport).toBe(true);
    expect(m.exportBatch).not.toHaveBeenCalled();
  });

  it("saves picks as a collection", async () => {
    await useStore.getState().saveAsCollection("  Keepers ", [3, 4]);
    expect(m.createCollection).toHaveBeenCalledWith("Keepers");
    expect(m.addToCollection).toHaveBeenCalledWith(7, [3, 4]);
    expect(toast).toHaveBeenCalledWith("Saved 2 photos as “Keepers”.", "success");
  });

  it("saves nothing without a name or photos", async () => {
    await useStore.getState().saveAsCollection(" ", [3]);
    await useStore.getState().saveAsCollection("Keepers", []);
    expect(m.createCollection).not.toHaveBeenCalled();
  });
});
