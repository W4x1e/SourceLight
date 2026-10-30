import { describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({
  api: {
    getPhotos: vi.fn(),
    getFacets: vi.fn(),
    getExtraFacets: vi.fn(),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { api } from "./lib/api";
import { toast } from "./lib/dialogs";
import { useStore } from "./store";
import type { Photo } from "./types";

const photo = (id: number) => ({ id }) as Photo;
const library = Array.from({ length: 100 }, (_, i) => photo(i + 1));

/** Selection works on what the grid shows, and outlives a filter change for
 *  the photos still shown. */
describe("selection", () => {
  it("takes a Shift range over the search results shown, not the library under them", () => {
    useStore.setState({
      photos: library,
      searchResults: [photo(10), photo(50), photo(90)],
      selected: new Set(),
      anchorId: null,
    });
    const { select } = useStore.getState();
    select(10, "single");
    select(90, "range");
    expect([...useStore.getState().selected].sort((a, b) => a - b)).toEqual([10, 50, 90]);
  });

  it("keeps an anchor that the list moved under, by photo rather than position", () => {
    useStore.setState({ photos: library, searchResults: null, selected: new Set(), anchorId: null });
    useStore.getState().select(5, "single");
    // The list changes under the anchor: two photos in front of it go.
    useStore.setState({ photos: library.filter((p) => p.id !== 1 && p.id !== 2) });
    useStore.getState().select(7, "range");
    expect([...useStore.getState().selected].sort((a, b) => a - b)).toEqual([5, 6, 7]);
  });

  it("keeps the selected photos a new filter still shows, and says how many it let go", async () => {
    vi.useFakeTimers();
    try {
      vi.mocked(api.getPhotos).mockResolvedValue([photo(1), photo(2)] as never);
      vi.mocked(api.getFacets).mockResolvedValue({} as never);
      vi.mocked(api.getExtraFacets).mockResolvedValue({} as never);
      useStore.setState({
        photos: library,
        searchResults: null,
        selected: new Set([1, 2, 3]),
        focusId: 3,
      });
      useStore.getState().setFilter({ ...useStore.getState().filter, minRating: 4 });
      await vi.runAllTimersAsync();
      expect([...useStore.getState().selected].sort()).toEqual([1, 2]);
      expect(useStore.getState().focusId).toBe(1);
      expect(vi.mocked(toast)).toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
