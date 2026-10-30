import { describe, expect, it } from "vitest";
import { chooseKeeper, isRaw, resolveAll, type GroupPhoto } from "./resolveDuplicates";

const photo = (id: number, path: string, o: Partial<GroupPhoto> = {}): GroupPhoto => ({
  id,
  path,
  width: 6000,
  height: 4000,
  rating: 0,
  flag: 0,
  sharpness: 100,
  ...o,
});

// A raw, the full-size JPEG made from it, and a small one for the web.
const PAIR = [
  photo(3, "C:/Export/DSC1.jpg", { sharpness: 180 }),
  photo(1, "C:/Photos/DSC1.ARW"),
  photo(2, "C:/Web/DSC1-small.jpg", { width: 1600, height: 1067, rating: 4 }),
];

describe("resolving duplicates", () => {
  it("knows a raw by its extension", () => {
    expect(isRaw("a/B.ARW")).toBe(true);
    expect(isRaw("a/b.cr3")).toBe(true);
    expect(isRaw("a/b.jpg")).toBe(false);
  });

  it("keeps what each rule says", () => {
    expect(chooseKeeper(PAIR, "raw")).toBe(1);
    // Same size: the raw breaks the tie.
    expect(chooseKeeper(PAIR, "largest")).toBe(1);
    expect(chooseKeeper(PAIR, "best")).toBe(2);
    expect(chooseKeeper(PAIR, "first")).toBe(1);
    // Two raws of one size: the sharper.
    expect(chooseKeeper([photo(5, "a.ARW", { sharpness: 10 }), photo(6, "b.ARW", { sharpness: 90 })], "raw")).toBe(6);
  });

  it("picks the keeper and rejects the rest, as flags that change", () => {
    const r = resolveAll([PAIR], { keeper: "raw", skipDecided: true, pickKeepers: true });
    expect(r).toEqual({
      changes: [
        { id: 3, flag: -1 },
        { id: 1, flag: 1 },
        { id: 2, flag: -1 },
      ],
      groups: 1,
      skipped: 0,
      rejects: 2,
    });
    const quiet = resolveAll([PAIR], { keeper: "raw", skipDecided: true, pickKeepers: false });
    expect(quiet.changes.map((c) => c.id)).toEqual([3, 2]);
  });

  it("leaves a group someone has already judged, unless told not to", () => {
    const judged = [photo(7, "a.ARW", { flag: 1 }), photo(8, "a.jpg")];
    expect(resolveAll([judged], { keeper: "raw", skipDecided: true, pickKeepers: true })).toMatchObject({
      groups: 0,
      skipped: 1,
      changes: [],
    });
    const over = resolveAll([judged], { keeper: "raw", skipDecided: false, pickKeepers: true });
    expect(over.changes).toEqual([{ id: 8, flag: -1 }]);
  });

  it("decides a photo in two groups once, and ignores a group of one", () => {
    const a = [photo(1, "x.ARW"), photo(2, "x.jpg")];
    const b = [photo(2, "x.jpg"), photo(9, "y.jpg", { width: 100, height: 100 })];
    const r = resolveAll([a, b, [photo(4, "z.ARW")]], { keeper: "raw", skipDecided: true, pickKeepers: true });
    expect(r.changes.filter((c) => c.id === 2)).toHaveLength(1);
    expect(r.groups).toBe(2);
  });
});
