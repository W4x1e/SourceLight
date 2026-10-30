import { describe, expect, it } from "vitest";
import { exposureFailure, suggestCull, suggestionCounts, TOO_BRIGHT, TOO_DARK, untouched } from "./suggest";
import type { Photo } from "../types";

/** An untouched, scored frame with only the fields the suggestions read. */
const frame = (id: number, over: Partial<Photo> = {}): Photo =>
  ({
    id,
    path: `C:/Photos/IMG_${id}.ARW`,
    filename: `IMG_${id}.ARW`,
    rating: 0,
    flag: 0,
    genre: null,
    sharpness: 0.5,
    brightness: 0.45,
    faceSharpness: null,
    eyesOpen: null,
    eyesOverride: null,
    ...over,
  }) as Photo;

const verdictOf = (s: ReturnType<typeof suggestCull>, id: number) => {
  const v = s.get(id);
  return v ? `${v.flag > 0 ? "pick" : "reject"}:${v.reason}` : "none";
};

describe("suggestCull", () => {
  it("names the better frame of a moment", () => {
    const burst = [frame(1, { sharpness: 0.4 }), frame(2, { sharpness: 0.9 })];
    expect(suggestCull(burst, [[1, 2]]).get(1)).toMatchObject({ flag: -1, reason: "burst", keeperId: 2 });
  });

  it("keeps a burst to its moment: frames far from the keeper are judged alone", () => {
    // One framing, shot through a ceremony: the group chains, the moment doesn't.
    const at = (s: number) => `2025-06-14 14:00:${String(s).padStart(2, "0")}`;
    const later = "2025-06-14 14:03:10";
    const burst = [
      frame(1, { sharpness: 0.9, flag: 1, captureDate: at(0) }),
      frame(2, { sharpness: 0.7, captureDate: at(5) }),
      frame(3, { sharpness: 0.7, captureDate: later }),
    ];
    const s = suggestCull(burst, [[1, 2, 3]]);
    expect(verdictOf(s, 2)).toBe("reject:burst");
    // Three minutes on: not "a better frame of this moment", nothing wrong.
    expect(verdictOf(s, 3)).toBe("pick:clean");
  });

  it("picks the sharpest frame of a burst and suggests the rest go", () => {
    const burst = [frame(1, { sharpness: 0.4 }), frame(2, { sharpness: 0.9 }), frame(3, { sharpness: 0.6 })];
    const s = suggestCull(burst, [[1, 2, 3]]);
    expect(verdictOf(s, 2)).toBe("pick:best");
    expect(verdictOf(s, 1)).toBe("reject:burst");
    expect(verdictOf(s, 3)).toBe("reject:burst");
  });

  it("never suggests anything for a frame a person has judged", () => {
    const photos = [frame(1, { rating: 3 }), frame(2, { flag: -1 }), frame(3, { brightness: 0.001, flag: 1 })];
    expect(suggestCull(photos, []).size).toBe(0);
  });

  it("treats a burst whose keeper a person already picked as decided", () => {
    const burst = [frame(1, { sharpness: 0.2, flag: 1 }), frame(2, { sharpness: 0.9 })];
    const s = suggestCull(burst, [[1, 2]]);
    expect(verdictOf(s, 1)).toBe("none");
    expect(verdictOf(s, 2)).toBe("reject:burst");
  });

  it("skips a frame the person rejected when choosing the keeper", () => {
    const burst = [frame(1, { sharpness: 0.9, flag: -1 }), frame(2, { sharpness: 0.5 }), frame(3, { sharpness: 0.3 })];
    const s = suggestCull(burst, [[1, 2, 3]]);
    expect(verdictOf(s, 2)).toBe("pick:best");
    expect(verdictOf(s, 3)).toBe("reject:burst");
  });

  it("leaves a burst's best frame for the person when it is soft for the set", () => {
    // The best of a moment that is still soft beside the rest of the shoot:
    // keeping the moment at all is not the machine's call.
    const burst = [frame(1, { sharpness: 0.1 }), frame(2, { sharpness: 0.05 })];
    const s = suggestCull([...burst, frame(3, { sharpness: 0.8 }), frame(4, { sharpness: 0.9 })], [[1, 2]]);
    expect(verdictOf(s, 1)).toBe("none");
    expect(verdictOf(s, 2)).toBe("reject:burst");
  });

  it("never rejects a frame for its eyes", () => {
    // The eye reading calls most smiling, small or turned faces closed.
    const s = suggestCull([frame(1, { eyesOpen: 0.05 }), frame(2, { eyesOpen: 0.95 })], []);
    expect(verdictOf(s, 1)).toBe("pick:clean");
    expect(verdictOf(s, 2)).toBe("pick:clean");
  });

  it("rejects single frames for missed focus or a failed exposure, and picks the rest", () => {
    const photos = [
      frame(1),
      frame(2, { sharpness: 0.05 }), // far below the set's median
      frame(3, { brightness: 0.005 }),
      frame(4, { brightness: 0.99 }),
      frame(5),
      frame(6),
    ];
    const s = suggestCull(photos, []);
    expect([1, 2, 3, 4, 5, 6].map((id) => verdictOf(s, id))).toEqual([
      "pick:clean",
      "reject:soft",
      "reject:dark",
      "reject:bright",
      "pick:clean",
      "pick:clean",
    ]);
    expect(suggestionCounts(s)).toEqual({ picks: 3, rejects: 3 });
  });

  it("says nothing about a frame there is no basis to judge", () => {
    const s = suggestCull([frame(1, { sharpness: null, brightness: null }), frame(2)], []);
    expect(verdictOf(s, 1)).toBe("none");
  });

  it("does not choose between copies of one shot in two folders", () => {
    const a = frame(1, { path: "C:/Photos/IMG_1.ARW", filename: "IMG_1.ARW", sharpness: 0.9 });
    const copy = frame(2, { path: "D:/Backup/IMG_1.ARW", filename: "IMG_1.ARW", sharpness: 0.4 });
    const s = suggestCull([a, copy], [[1, 2]]);
    expect(s.size).toBe(0);
  });

  it("judges only the part of a burst inside the cull", () => {
    // 3 is sharper but outside the set being culled.
    const s = suggestCull([frame(1, { sharpness: 0.5 }), frame(2, { sharpness: 0.4 })], [[1, 2, 3]]);
    expect(verdictOf(s, 1)).toBe("pick:best");
    expect(verdictOf(s, 2)).toBe("reject:burst");
  });

  it("rejects a black frame in a burst for being black, not for being an extra", () => {
    const burst = [frame(1, { sharpness: 0.9 }), frame(2, { sharpness: 0.1, brightness: 0.001 })];
    expect(verdictOf(suggestCull(burst, [[1, 2]]), 2)).toBe("reject:dark");
  });
});

describe("exposureFailure", () => {
  it("reads only the extremes", () => {
    expect(exposureFailure(frame(1, { brightness: TOO_DARK / 2 }))).toBe("dark");
    expect(exposureFailure(frame(1, { brightness: 0.1 }))).toBeNull();
    expect(exposureFailure(frame(1, { brightness: 0.9 }))).toBeNull();
    expect(exposureFailure(frame(1, { brightness: (TOO_BRIGHT + 1) / 2 }))).toBe("bright");
    expect(exposureFailure(frame(1, { brightness: null }))).toBeNull();
  });

  it("never calls an astro frame too dark", () => {
    expect(exposureFailure(frame(1, { brightness: 0.005, genre: "astro" }))).toBeNull();
  });
});

describe("untouched", () => {
  it("means no rating and no flag", () => {
    expect(untouched(frame(1))).toBe(true);
    expect(untouched(frame(1, { rating: 1 }))).toBe(false);
    expect(untouched(frame(1, { flag: -1 }))).toBe(false);
  });
});
