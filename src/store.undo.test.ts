import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({ api: { undoVerdict: vi.fn(), getPhotos: vi.fn(async () => []) } }));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { api } from "./lib/api";
import { toast } from "./lib/dialogs";
import { useStore } from "./store";
import type { Photo } from "./types";

const undo = vi.mocked(api.undoVerdict);

const photo = (id: number, over: Partial<Photo> = {}): Photo =>
  ({ id, rating: 1, flag: 1, colorLabel: 0, filename: `p${id}.arw`, ...over }) as Photo;

/** Undoing a rating, flag or label. */
describe("undoVerdict", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("puts each photo back to its own earlier verdict, everywhere it is shown", async () => {
    useStore.setState({
      photos: [photo(1), photo(2), photo(3)],
      searchResults: [photo(2)],
      compare: { sets: [[photo(1), photo(2)]], index: 0, focusId: 1, source: "selection", threshold: 0.9 },
    });
    undo.mockResolvedValue({
      label: "Rating on 2 photos",
      restored: [
        { id: 1, rating: 5, flag: 0, colorLabel: 2 },
        { id: 2, rating: 3, flag: -1, colorLabel: 0 },
      ],
      settings: [],
      beyond: false,
    });

    await useStore.getState().undoVerdict();

    const s = useStore.getState();
    expect(s.photos.map((p) => [p.id, p.rating, p.flag, p.colorLabel])).toEqual([
      [1, 5, 0, 2],
      [2, 3, -1, 0],
      [3, 1, 1, 0], // untouched
    ]);
    expect(s.searchResults?.[0]?.rating).toBe(3);
    expect(s.compare?.sets[0]?.map((p) => p.rating)).toEqual([5, 3]);
    expect(toast).toHaveBeenCalledWith("Undid: Rating on 2 photos.", "success");
  });

  /** A keyword added since the last rating is not undone by reverting that
   *  rating: Undo says what it can't take back, and stops there. */
  it("stops at a change it can't take back, rather than reverting an older verdict", async () => {
    useStore.setState({ photos: [photo(1, { rating: 4 })], searchResults: null, compare: null });
    undo.mockResolvedValue({ label: "Keyword on 400 photos", restored: [], settings: [], beyond: true });
    await useStore.getState().undoVerdict();
    expect(useStore.getState().photos[0]?.rating).toBe(4);
    expect(toast).toHaveBeenCalledWith(expect.stringContaining("Keyword on 400 photos"), "info");
  });

  it("says so when there is nothing left to undo", async () => {
    useStore.setState({ photos: [photo(1)], searchResults: null, compare: null });
    undo.mockResolvedValue(null);
    await useStore.getState().undoVerdict();
    expect(useStore.getState().photos[0]?.rating).toBe(1);
    expect(toast).toHaveBeenCalledWith("Nothing to undo.", "info");
  });
});
