import { describe, expect, it } from "vitest";
import { EMPTY_FILTER, isFilterEmpty, normaliseFilter, type PhotoFilter } from "./types";

/**
 * `isFilterEmpty` decides whether the UI offers "Clear all" and "Save as view"
 * (ContextHeader, FilterRail, and the Browse footer's "of N matching"). A field
 * missing from it becomes a filter the user can neither see nor clear — which is
 * exactly what happened to `personId`: People jumped into Browse with it set and
 * every clear affordance stayed hidden.
 *
 * So the invariant worth pinning is not "these particular fields work" but
 * "every field of the filter is accounted for".
 */

/** The fields of `PhotoFilter` that narrow nothing on their own: the order;
 *  whether chosen keywords must all match, which only says how
 *  `subjects` combine; and whether the tagger's keywords count, a view
 *  setting the store adds to every query. */
type NotAFilter = "sort" | "subjectsAll" | "ownKeywords";

/** A narrowing value for each field of `PhotoFilter`. */
const NARROWING: { [K in Exclude<keyof Required<PhotoFilter>, NotAFilter>]: PhotoFilter[K] } = {
  dateFrom: "2026-01-01",
  dateTo: "2026-12-31",
  genres: ["portrait"],
  subjects: ["dog"],
  conditions: ["golden hour"],
  excludeSubjects: ["clouds"],
  excludeGenres: ["street"],
  excludeConditions: ["overcast"],
  placeIds: [7],
  minRating: 3,
  ratings: [3],
  flags: [1],
  colorLabels: [2],
  collectionId: 4,
  personId: 9,
  personIds: [9],
  cameras: ["ILCE-6400"],
  lenses: ["E 16-50mm"],
  focalRanges: ["25-50"],
  isoRanges: ["0-400"],
  apertureRanges: ["0-2.8"],
  fileKinds: ["raw"],
  shapes: ["portrait"],
  faceCounts: ["1"],
  palette: ["orange"],
  editedOnly: true,
  copiesOnly: true,
  facesOnly: true,
  locatedOnly: true,
  unplacedOnly: true,
  untaggedOnly: true,
  folderIds: [2],
  missingOnly: true,
  shootId: 3,
  search: "beach",
  semantic: "a dog on a beach",
};

describe("isFilterEmpty", () => {
  it("is true for the empty filter", () => {
    expect(isFilterEmpty(EMPTY_FILTER)).toBe(true);
  });

  // The guard that matters: add a field to PhotoFilter and this fails until it
  // is given a sample here, at which point the per-field test below covers it.
  it("has a sample for every field of the filter", () => {
    const notFilters: string[] = ["sort", "subjectsAll"] satisfies NotAFilter[];
    const fields = Object.keys(EMPTY_FILTER).filter((k) => !notFilters.includes(k));
    expect(Object.keys(NARROWING).sort()).toEqual(fields.sort());
  });

  it.each(Object.keys(NARROWING) as (keyof typeof NARROWING)[])(
    "is false when only %s narrows the library",
    (field) => {
      const filter = { ...EMPTY_FILTER, [field]: NARROWING[field] } as PhotoFilter;
      expect(isFilterEmpty(filter)).toBe(false);
    },
  );

  it("doesn't count the sort, which orders the library rather than narrowing it", () => {
    expect(isFilterEmpty({ ...EMPTY_FILTER, sort: "rating" })).toBe(true);
  });

  // The two that regressed, called out by name so a failure reads clearly.
  it("counts a person filter, which People sets when it jumps into Browse", () => {
    expect(isFilterEmpty({ ...EMPTY_FILTER, personId: 3 })).toBe(false);
  });

  it("counts a content search, so it can be saved as a smart collection", () => {
    expect(isFilterEmpty({ ...EMPTY_FILTER, semantic: "sunset over water" })).toBe(false);
  });

  it("treats an absent personId the same as a null one", () => {
    const { personId: _drop, ...withoutPersonId } = EMPTY_FILTER;
    expect(isFilterEmpty(withoutPersonId as PhotoFilter)).toBe(true);
  });
});

describe("normaliseFilter", () => {
  it("turns an old minimum rating into the same exact ratings", () => {
    const f = normaliseFilter({ minRating: 3 });
    expect(f.minRating).toBeNull();
    expect(f.ratings).toEqual([3, 4, 5]);
  });

  it("fills in fields an old saved filter lacks", () => {
    expect(normaliseFilter({ genres: ["portrait"] })).toEqual({ ...EMPTY_FILTER, genres: ["portrait"] });
    expect(normaliseFilter(undefined)).toEqual(EMPTY_FILTER);
  });

  it("leaves exact ratings as they are", () => {
    expect(normaliseFilter({ ratings: [2, 5] }).ratings).toEqual([2, 5]);
  });
});

describe("normaliseFilter and people", () => {
  it("turns a saved single person into the list the rail shows", () => {
    const f = normaliseFilter({ personId: 4, personIds: [2] });
    expect(f.personId).toBeNull();
    expect(f.personIds).toEqual([2, 4]);
  });

  it("fills the filter fields a view saved before them lacks", () => {
    const f = normaliseFilter({ genres: ["landscape"] } as Partial<PhotoFilter>);
    expect(f.cameras).toEqual([]);
    expect(f.sort).toBe("newest");
  });
});
