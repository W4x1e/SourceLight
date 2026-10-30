import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({
  api: {
    getPhotos: vi.fn(async () => []),
    getFacets: vi.fn(async () => ({ total: 3 })),
    getExtraFacets: vi.fn(async () => ({})),
    getPhotosByIds: vi.fn(async (ids: number[]) => ids.map((id) => ({ id, rating: 5 }))),
    closeDevelop: vi.fn(async () => undefined),
    openDevelop: vi.fn(async (id: number) => ({ photoId: id, stack: {} })),
    addTag: vi.fn(async () => undefined),
    listKeywords: vi.fn(async () => []),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { api } from "./lib/api";
import { useStore } from "./store";
import { EMPTY_FILTER } from "./types";

const page = () => [1, 2, 3].map((id) => ({ id, rating: 0 })) as never;

/** One photo's change patches its row, not the 20,000-row page. */
describe("refreshPhotos", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useStore.setState({ photos: page(), committedFilter: { ...EMPTY_FILTER }, searchResults: null });
  });

  it("patches the rows and recounts the facets in an unfiltered view", async () => {
    await useStore.getState().refreshPhotos([2]);
    expect(api.getPhotosByIds).toHaveBeenCalledWith([2]);
    expect(api.getPhotos).not.toHaveBeenCalled();
    expect(api.getFacets).toHaveBeenCalledOnce();
    expect(useStore.getState().photos.map((p) => p.rating)).toEqual([0, 5, 0]);
  });

  it("reloads the page under a filter, where a change can move a photo out of it", async () => {
    useStore.setState({ committedFilter: { ...EMPTY_FILTER, ratings: [0] } });
    await useStore.getState().refreshPhotos([2]);
    expect(api.getPhotosByIds).not.toHaveBeenCalled();
    expect(api.getPhotos).toHaveBeenCalledOnce();
  });

  it("reloads the page for a photo that isn't on it", async () => {
    await useStore.getState().refreshPhotos([9]);
    expect(api.getPhotos).toHaveBeenCalledOnce();
  });

  it("refreshes only what Develop opened when it closes", async () => {
    await useStore.getState().openDevelop(1);
    await useStore.getState().openDevelop(3);
    await useStore.getState().closeDevelop();
    expect(api.getPhotosByIds).toHaveBeenCalledWith(expect.arrayContaining([1, 3]));
    expect(api.getPhotos).not.toHaveBeenCalled();
  });
});
