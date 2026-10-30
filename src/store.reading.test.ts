import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./lib/api", () => ({
  api: {
    getPhotos: vi.fn(),
    getFacets: vi.fn(),
    getExtraFacets: vi.fn(),
    searchVocabulary: vi.fn(),
    searchIndexStatus: vi.fn(),
  },
}));
vi.mock("./lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));
vi.mock("./lib/aiReady", () => ({ ensureAi: vi.fn(async () => true), downloadSize: vi.fn() }));

import { api } from "./lib/api";
import { ensureAi } from "./lib/aiReady";
import { useStore } from "./store";
import { EMPTY_FILTER } from "./types";
import type { SearchVocabulary } from "./bindings";

const m = vi.mocked(api);
const place = (id: number, name: string, country: string | null) => ({
  id,
  name,
  lat: null,
  lon: null,
  count: 1,
  country,
});
const VOCAB: SearchVocabulary = {
  people: [
    { id: 1, name: "Anna" },
    { id: 2, name: "Ben" },
    { id: 3, name: "Paris" },
  ],
  places: [place(12, "Paris, France", "France")],
  genres: [],
  conditions: [],
  cameras: [],
  lenses: [],
  keywords: [],
};

const filter = () => useStore.getState().filter;
const search = (q: string) => useStore.getState().runSemanticSearch(q);

beforeEach(() => {
  vi.clearAllMocks();
  m.getPhotos.mockResolvedValue([]);
  m.getFacets.mockResolvedValue({} as never);
  m.searchVocabulary.mockResolvedValue(VOCAB);
  m.searchIndexStatus.mockResolvedValue(0);
  useStore.setState({ filter: { ...EMPTY_FILTER }, queryReading: null });
});

describe("a search read before content search sees it", () => {
  it("that is all facts filters, without asking for the model", async () => {
    await search("Anna 2023");
    expect(filter().personIds).toEqual([1]);
    expect([filter().dateFrom, filter().dateTo]).toEqual(["2023-01-01", "2023-12-31"]);
    expect(filter().semantic).toBeNull();
    expect(ensureAi).not.toHaveBeenCalled();
  });

  it("hands the rest to content search", async () => {
    await search("Anna at the beach");
    expect(filter().personIds).toEqual([1]);
    expect(filter().semantic).toBe("beach");
    expect(ensureAi).toHaveBeenCalled();
  });

  it("can be searched as typed instead, which takes the reading back out", async () => {
    await search("Anna at the beach");
    await useStore.getState().searchAsTyped();
    expect(filter().personIds).toEqual([]);
    expect(filter().semantic).toBe("Anna at the beach");
    expect(useStore.getState().queryReading).toBeNull();
  });

  it("replaces the last search's reading rather than adding to it", async () => {
    useStore.setState({ filter: { ...EMPTY_FILTER, personIds: [2] } });
    await search("Anna 2023");
    await search("Paris");
    expect(filter().personIds).toEqual([2]);
    expect([filter().dateFrom, filter().dateTo]).toEqual([null, null]);
    expect(filter().placeIds).toEqual([12]);
  });

  it("takes the other reading of a word that reads two ways", async () => {
    await search("Paris");
    useStore.getState().takeAlternative(0);
    expect(filter().placeIds).toEqual([]);
    expect(filter().personIds).toEqual([3]);
    expect(useStore.getState().queryReading?.reading.alternatives).toEqual([]);
  });

  it("searches the whole text by content when nothing is recognised, or the names can't be had", async () => {
    await search("golden retriever");
    expect(filter().semantic).toBe("golden retriever");
    expect(useStore.getState().queryReading).toBeNull();

    m.searchVocabulary.mockRejectedValueOnce(new Error("gone"));
    await search("Anna");
    expect(filter().semantic).toBe("Anna");
    expect(filter().personIds).toEqual([]);
  });

  it("is forgotten by Clear all", async () => {
    await search("Anna 2023");
    useStore.getState().clearFilter();
    expect(useStore.getState().queryReading).toBeNull();
  });
});
