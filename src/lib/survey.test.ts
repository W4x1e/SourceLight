import { describe, expect, it } from "vitest";
import {
  allCopies,
  cullRubric,
  eyesClosed,
  facesAnalysed,
  focusBand,
  folderOf,
  hydrateGroups,
  isSoft,
  keepPlan,
  medianSharpness,
  orderGroupsBySharpness,
  sharpestId,
  sharpnessScores,
  releaseIndex,
  resumeIndex,
  sameShot,
  stepIndex,
  surveyLayout,
  highlightCandidates,
} from "./survey";
import type { Photo } from "../types";

/** A photo row with only the fields the survey maths looks at (optionally a
 *  focus score). */
const photo = (id: number, sharpness?: number): Photo =>
  ({ id, filename: `IMG_${id}.ARW`, sharpness }) as Photo;

/** A photo with a worst-eye-open score, for the blink check. */
const withEyes = (id: number, eyesOpen: number | null): Photo =>
  ({ id, filename: `IMG_${id}.ARW`, eyesOpen }) as Photo;

/** A photo carrying both a focus score and a worst-eye-open score. */
const photoEyes = (id: number, sharpness: number, eyesOpen: number): Photo =>
  ({ id, filename: `IMG_${id}.ARW`, sharpness, eyesOpen }) as Photo;

describe("surveyLayout", () => {
  it("puts two frames side by side on a landscape window", () => {
    const l = surveyLayout(2, 1200, 700);
    expect(l).toMatchObject({ cols: 2, rows: 1 });
  });

  it("stacks two frames on a tall narrow window", () => {
    const l = surveyLayout(2, 500, 1400);
    expect(l).toMatchObject({ cols: 1, rows: 2 });
  });

  it("goes 2×2 for four frames on a squarish window but 4×1 on a wide short one", () => {
    expect(surveyLayout(4, 1200, 900)).toMatchObject({ cols: 2, rows: 2 });
    expect(surveyLayout(4, 2400, 320)).toMatchObject({ cols: 4, rows: 1 });
  });

  it("keeps tiles inside the box, gaps included", () => {
    const gap = 8;
    const { cols, rows, tileW, tileH } = surveyLayout(6, 1000, 600, gap);
    expect(tileW * cols + gap * (cols - 1)).toBeLessThanOrEqual(1000 + 1e-6);
    expect(tileH * rows + gap * (rows - 1)).toBeLessThanOrEqual(600 + 1e-6);
  });

  it("holds the nominal aspect on every tile", () => {
    const { tileW, tileH } = surveyLayout(5, 1300, 800, 8, 3 / 2);
    expect(tileW / tileH).toBeCloseTo(3 / 2, 5);
  });

  it("stops shrinking at the minimum tile and overflows into rows instead", () => {
    // A 40-frame burst at the 940×620 window floor fits at ~127px a tile; with
    // a 150px floor the view keeps them readable and lets the box scroll.
    const fitted = surveyLayout(40, 940, 620, 8);
    expect(fitted.tileW).toBeLessThan(150);

    const floored = surveyLayout(40, 940, 620, 8, 3 / 2, 150);
    expect(floored.tileW).toBeGreaterThanOrEqual(150);
    expect(floored.rows * floored.tileH).toBeGreaterThan(620);
    // Fills the row, to within the whole-pixel rounding, and never past it.
    const used = floored.cols * floored.tileW + 8 * (floored.cols - 1);
    expect(used).toBeLessThanOrEqual(940);
    expect(used).toBeGreaterThan(940 - floored.cols);
  });

  it("leaves a set that already fits alone when a minimum is given", () => {
    expect(surveyLayout(4, 1200, 900, 8, 3 / 2, 150)).toEqual(surveyLayout(4, 1200, 900, 8));
  });

  it("degrades safely before the container has been measured", () => {
    expect(surveyLayout(4, 0, 0)).toMatchObject({ tileW: 0, tileH: 0 });
    expect(surveyLayout(0, 800, 600)).toMatchObject({ tileW: 0, tileH: 0 });
  });

  // The cull survey is row-virtualised: it derives its row count from `cols`
  // and slices `cols` tiles per row. If `rows` and `cols` ever disagreed, rows
  // would come out empty or tiles would be dropped off the end — silently. So
  // the relationship between the two is pinned, across sizes and the minimum
  // tile branch (which recomputes both).
  it("always reports rows as the row count cols implies", () => {
    const cases: [number, number, number, number][] = [
      [2, 1200, 700, 0],
      [4, 1200, 900, 0],
      [5, 1300, 800, 0],
      [7, 900, 900, 0],
      [40, 940, 620, 0],
      [40, 940, 620, 150],
      [1000, 1600, 900, 150],
      [20000, 1600, 900, 150],
    ];
    for (const [count, w, h, minTile] of cases) {
      const { cols, rows } = surveyLayout(count, w, h, 8, 3 / 2, minTile);
      expect(cols).toBeGreaterThanOrEqual(1);
      expect(rows).toBe(Math.ceil(count / cols));
    }
  });
});

describe("hydrateGroups", () => {
  it("orders a group sharpest-first — the suggested keeper leads", () => {
    const groups = hydrateGroups([[1, 2, 3]], [photo(1, 0.2), photo(2, 0.9), photo(3, 0.5)]);
    expect(groups[0]!.map((p) => p.id)).toEqual([2, 3, 1]);
  });

  it("sends unscored frames to the back, ties broken by id", () => {
    const groups = hydrateGroups([[3, 1, 2]], [photo(1), photo(2, 0.4), photo(3)]);
    // 2 is scored so leads; 1 and 3 are unscored and fall behind, id-ordered.
    expect(groups[0]!.map((p) => p.id)).toEqual([2, 1, 3]);
  });

  it("drops ids with no photo, and any group that leaves under two", () => {
    // 9 was indexed but is no longer in the library.
    const groups = hydrateGroups(
      [
        [1, 2],
        [3, 9],
      ],
      [photo(1), photo(2), photo(3)],
    );
    expect(groups.map((g) => g.map((p) => p.id))).toEqual([[1, 2]]);
  });

  it("is empty when nothing resolves", () => {
    expect(hydrateGroups([[1, 2]], [])).toEqual([]);
  });
});

describe("orderGroupsBySharpness", () => {
  it("reorders each group's ids sharpest-first for a keep-the-first cull", () => {
    const ordered = orderGroupsBySharpness(
      [
        [1, 2],
        [3, 4],
      ],
      [photo(1, 0.1), photo(2, 0.8), photo(3, 0.9), photo(4, 0.2)],
    );
    expect(ordered).toEqual([
      [2, 1],
      [3, 4],
    ]);
  });

  it("keeps ids with no loaded photo at the end, stably by id", () => {
    // 5 isn't in the grid (a duplicate outside the current filter).
    const ordered = orderGroupsBySharpness([[5, 1, 2]], [photo(1, 0.3), photo(2, 0.7)]);
    expect(ordered).toEqual([[2, 1, 5]]);
  });
});

describe("sharpness helpers", () => {
  it("medianSharpness ignores unscored frames and returns null when none are scored", () => {
    expect(medianSharpness([photo(1, 0.2), photo(2, 0.6), photo(3)])).toBeCloseTo(0.4, 5);
    expect(medianSharpness([photo(1), photo(2)])).toBeNull();
  });

  it("sharpestId picks the highest score, or null when nothing is scored", () => {
    expect(sharpestId([photo(1, 0.3), photo(2, 0.9), photo(3, 0.5)])).toBe(2);
    expect(sharpestId([photo(1), photo(2)])).toBeNull();
  });

  it("isSoft flags only scored frames below half the set median, never on a bare basis", () => {
    const median = medianSharpness([photo(1, 1.0), photo(2, 1.0), photo(3, 0.2)]);
    expect(isSoft(photo(3, 0.2), median)).toBe(true); // 0.2 < 1.0 * 0.5
    expect(isSoft(photo(1, 1.0), median)).toBe(false);
    expect(isSoft(photo(9), median)).toBe(false); // unscored → never accused
    expect(isSoft(photo(3, 0.2), null)).toBe(false); // no basis → no warning
  });

  it("eyesClosed flags a blink only on analysed frames with a face", () => {
    expect(eyesClosed(withEyes(1, 0.1))).toBe(true); // worst eye shut
    expect(eyesClosed(withEyes(2, 0.9))).toBe(false); // eyes open
    expect(eyesClosed(withEyes(3, null))).toBe(false); // unanalysed / no face
  });

  it("judges focus by genre: stricter for a landscape, looser for a portrait", () => {
    const median = medianSharpness([photo(1, 1.0), photo(2, 1.0), photo(3, 1.0)]);
    const at = (sharpness: number, genre: string | null) =>
      isSoft({ ...photo(9, sharpness), genre } as Photo, median);
    // 0.55 of the median: soft for a landscape (0.6), fine by default (0.5).
    expect(at(0.55, "landscape")).toBe(true);
    expect(at(0.55, null)).toBe(false);
    // 0.4: soft by default, but a portrait's bokeh is not a focus miss.
    expect(at(0.4, null)).toBe(true);
    expect(at(0.4, "portrait")).toBe(false);
    // Astro is never called soft; an unknown genre gets the default.
    expect(at(0.01, "astro")).toBe(false);
    expect(at(0.4, "not-a-genre")).toBe(true);
  });

  it("a blink only counts where people are the subject", () => {
    const blink = (genre: string | null) => ({ ...withEyes(1, 0.1), genre }) as Photo;
    expect(eyesClosed(blink("portrait"))).toBe(true);
    expect(eyesClosed(blink(null))).toBe(true);
    expect(eyesClosed(blink("landscape"))).toBe(false);
    // The user's own verdict still counts in any genre.
    expect(eyesClosed({ ...blink("landscape"), eyesOverride: 1 } as Photo)).toBe(true);
    expect(cullRubric("landscape").eyesMatter).toBe(false);
  });

  it("the user's verdict overrules the model in both directions", () => {
    // The downcast gaze the classifier calls a blink.
    expect(eyesClosed({ ...withEyes(1, 0.1), eyesOverride: 0 } as Photo)).toBe(false);
    // And the reverse: a blink it scored as open.
    expect(eyesClosed({ ...withEyes(2, 0.9), eyesOverride: 1 } as Photo)).toBe(true);
    // Cleared, it defers to the model again.
    expect(eyesClosed({ ...withEyes(3, 0.1), eyesOverride: null } as Photo)).toBe(true);
  });

});

describe("the eye reading ranks keepers again", () => {
  // Off while the old reading called smiles blinks; back with the face mesh,
  // which tells them apart and says "can't tell" rather than guessing.
  it("puts an open-eyed frame ahead of a sharper blink", () => {
    const blink = photoEyes(1, 0.9, 0.1);
    const open = photoEyes(2, 0.6, 0.9);
    expect(sharpestId([blink, open])).toBe(2);
    expect(hydrateGroups([[1, 2]], [blink, open])[0]!.map((p) => p.id)).toEqual([2, 1]);
    expect(orderGroupsBySharpness([[1, 2]], [blink, open])).toEqual([[2, 1]]);
  });

  it("never demotes a frame whose eyes can't be read", () => {
    const cantTell = { ...photoEyes(1, 0.9, 0.5), eyesOpen: null } as Photo;
    expect(sharpestId([cantTell, photoEyes(2, 0.6, 0.9)])).toBe(1);
  });

  it("follows a person's verdict over the reading", () => {
    const blink = photoEyes(1, 0.9, 0.1);
    expect(sharpestId([{ ...blink, eyesOverride: 0 } as Photo, photoEyes(2, 0.6, 0.9)])).toBe(1);
    expect(eyesClosed({ ...blink, eyesOverride: 0 } as Photo)).toBe(false);
  });
});

describe("face-aware keeper ordering", () => {
  /** A portrait frame: whole-frame focus, the main face's focus, eyes open. */
  const portrait = (id: number, sharpness: number, faceSharpness: number | null): Photo =>
    ({ id, filename: `IMG_${id}.ARW`, genre: "portrait", sharpness, faceSharpness, eyesOpen: 0.95 }) as Photo;

  it("keeps the sharper face over the busier background in a portrait burst", () => {
    // 2's whole frame scores higher (a busier background), 1's face is sharper.
    const burst = [portrait(1, 0.4, 0.9), portrait(2, 0.8, 0.3)];
    expect(sharpestId(burst)).toBe(1);
    expect(hydrateGroups([[2, 1]], burst)[0]!.map((p) => p.id)).toEqual([1, 2]);
    expect(orderGroupsBySharpness([[2, 1]], burst)).toEqual([[1, 2]]);
  });

  it("falls back to the whole frame when any frame has no face score", () => {
    const burst = [portrait(1, 0.4, 0.9), portrait(2, 0.8, null)];
    expect(sharpestId(burst)).toBe(2);
  });

  it("judges a landscape on the whole frame even with a face in it", () => {
    const land = (id: number, sharpness: number, faceSharpness: number): Photo =>
      ({ id, filename: `IMG_${id}.ARW`, genre: "landscape", sharpness, faceSharpness }) as Photo;
    expect(sharpestId([land(1, 0.4, 0.9), land(2, 0.8, 0.3)])).toBe(2);
  });

});

describe("focusBand", () => {
  const set = [photo(1, 0.1), photo(2, 0.2), photo(3, 0.3), photo(4, 0.4), photo(5, 0.5)];
  const sorted = sharpnessScores(set);

  it("ranks the sharpest frame in the top band and the softest in the bottom", () => {
    expect(focusBand(photo(9, 0.5), sorted)).toMatchObject({ level: 4, label: "Sharp", percentile: 80 });
    // Soft by the suggestions' own rule, against the set's median.
    expect(focusBand(photo(9, 0.1), sorted, 0.3)).toMatchObject({ level: 0, label: "Very soft", percentile: 0, soft: true });
    expect(focusBand(photo(9, 0.3), sorted)).toMatchObject({ level: 2, label: "Fair", percentile: 40 });
  });

  it("calls a sharp frame low in a sharp set a rank, not soft", () => {
    // A tack-sharp burst: the bottom fifth is nowhere near the soft cut.
    const sharp = sharpnessScores([photo(1, 0.9), photo(2, 0.92), photo(3, 0.94), photo(4, 0.96), photo(5, 0.98)]);
    expect(focusBand(photo(9, 0.9), sharp, 0.94)).toMatchObject({
      level: 0,
      label: "Less sharp than most here",
      soft: false,
    });
  });

  it("returns null with no score or no set to compare against", () => {
    expect(focusBand(photo(9), sorted)).toBeNull();
    expect(focusBand(photo(9, 0.5), [])).toBeNull();
  });
});

describe("facesAnalysed", () => {
  it("is true once a face count is stamped (even zero), false before", () => {
    expect(facesAnalysed({ id: 1, faceCount: 2 } as Photo)).toBe(true);
    expect(facesAnalysed({ id: 1, faceCount: 0 } as Photo)).toBe(true);
    expect(facesAnalysed({ id: 1, faceCount: null } as Photo)).toBe(false);
    expect(facesAnalysed({ id: 1 } as Photo)).toBe(false);
  });
});

describe("keepPlan", () => {
  it("picks one and rejects the rest", () => {
    expect(keepPlan([4, 5, 6], 5)).toEqual({ keep: 5, reject: [4, 6] });
  });

  it("rejects nothing when the set holds only the keeper", () => {
    expect(keepPlan([7], 7)).toEqual({ keep: 7, reject: [] });
  });
});

describe("stepIndex", () => {
  it("clamps at both ends rather than wrapping", () => {
    expect(stepIndex(0, -1, 3)).toBe(0);
    expect(stepIndex(2, 1, 3)).toBe(2);
    expect(stepIndex(1, 1, 3)).toBe(2);
    expect(stepIndex(0, 0, 0)).toBe(0);
  });
});

describe("releaseIndex", () => {
  const ids = [10, 11, 12, 13];

  it("is the target itself when nothing leaves", () => {
    expect(releaseIndex(ids, 2, new Set())).toBe(2);
  });

  it("steps forward onto the frame after a held one as it leaves", () => {
    // Judged 11 (index 1) in "To review", then "next": 12 closes up to index 1.
    expect(releaseIndex(ids, 2, new Set([11]))).toBe(1);
  });

  it("is unaffected by a held frame after the target", () => {
    expect(releaseIndex(ids, 0, new Set([11]))).toBe(0);
  });
});

describe("resumeIndex", () => {
  it("returns to the saved photo wherever it now sits", () => {
    // New imports ahead of it moved it along; the id still finds it.
    expect(resumeIndex([9, 8, 1, 2, 3], { photoId: 2, index: 1 })).toBe(3);
  });

  it("falls back to the same position when the photo left the set", () => {
    // It was judged, so "To review" no longer holds it: the next frame now
    // sits where it was.
    expect(resumeIndex([1, 3, 4], { photoId: 2, index: 1 })).toBe(1);
  });

  it("clamps to the end of a set that shrank, and copes with none", () => {
    expect(resumeIndex([1, 2], { photoId: 99, index: 40 })).toBe(1);
    expect(resumeIndex([], { photoId: 1, index: 3 })).toBe(0);
  });
});

describe("copies of one shot", () => {
  const shot = (id: number, path: string, over: Partial<Photo> = {}): Photo =>
    ({
      id,
      path,
      filename: "DSC00068.ARW",
      captureDate: "2025-08-12 10:14:03",
      width: 6000,
      height: 4000,
      shutter: "1/640",
      aperture: 7.1,
      iso: 100,
      ...over,
    }) as Photo;

  it("names the folder by its last two segments, on either separator", () => {
    expect(folderOf("C:\\Users\\james\\Pictures\\A6400\\DSC00068.ARW")).toBe("Pictures\\A6400");
    expect(folderOf("/home/j/Nextcloud/Sony A6400/DSC00068.ARW")).toBe("Nextcloud/Sony A6400");
    expect(folderOf("DSC00068.ARW")).toBe("");
  });

  it("matches a copy in another folder, and not a different frame or the same file", () => {
    const a = shot(60, "C:\\Pictures\\A6400\\DSC00068.ARW");
    const b = shot(195, "C:\\Nextcloud\\Sony A6400\\DSC00068.ARW");
    expect(sameShot(a, b)).toBe(true);
    // A burst neighbour: same scene, a moment later.
    expect(sameShot(a, shot(61, "C:\\x\\DSC00069.ARW", { filename: "DSC00069.ARW" }))).toBe(false);
    expect(sameShot(a, shot(62, "C:\\y\\DSC00068.ARW", { shutter: "1/320" }))).toBe(false);
    // A virtual copy shares the path: one file, not two.
    expect(sameShot(a, shot(63, a.path))).toBe(false);
  });

  it("calls a set all copies only when every frame is one", () => {
    const a = shot(1, "/a/DSC00068.ARW");
    const b = shot(2, "/b/DSC00068.ARW");
    const c = shot(3, "/c/DSC00070.ARW", { filename: "DSC00070.ARW" });
    expect(allCopies([a, b])).toBe(true);
    expect(allCopies([a, b, c])).toBe(false);
    expect(allCopies([a])).toBe(false);
  });
});

describe("highlight candidates", () => {
  it("leaves out rejects, blinks and soft frames, and keeps the rest", () => {
    const frame = (id: number, extra: Partial<Photo>): Photo =>
      ({ id, filename: `IMG_${id}.ARW`, genre: "portrait", sharpness: 0.8, eyesOpen: 0.9, flag: 0, ...extra }) as Photo;
    const set = [
      frame(1, {}),
      frame(2, { flag: -1 }),
      frame(3, { eyesOpen: 0.1 }),
      frame(4, { sharpness: 0.05 }),
      frame(5, { flag: 1 }),
      frame(6, { eyesOpen: null }),
    ];
    expect(highlightCandidates(set).map((p) => p.id)).toEqual([1, 5, 6]);
  });
});
