/**
 * A suggested cull: a proposed verdict for every frame nobody has judged
 * yet, so a two-thousand-frame shoot starts as a review rather than as two
 * thousand decisions.
 *
 * Suggestions are not verdicts. They are computed here, shown apart from real
 * flags, and become flags only when someone accepts them — a frame a person has
 * already judged (rated or flagged) is never given one (principle 6: human
 * intent outranks the machine). They are recomputed from the photos on every
 * change, so judging one frame of a burst re-plans the rest of it.
 *
 * Rejects are only ever suggested for a reason that can be named, because a
 * reject is one "Bin rejects" away from the Recycle Bin: a better frame of the
 * same moment, missed focus, or a frame that failed outright. A frame there is
 * no basis to judge — not scored yet — gets nothing.
 *
 * Closed eyes are not a reason. The eye reading calls most smiling,
 * small or turned faces closed, and a reject on that would bin the good frame.
 */
import type { Photo } from "../types";
import { t, type Key } from "../i18n";
import { isSoft, keeperOrder, medianSharpness, sameShot } from "./survey";
import { parseCapture } from "./captureTime";

export type SuggestReason =
  /** The best frame of its burst. */
  | "best"
  /** Not in a burst, and nothing wrong found. */
  | "clean"
  /** In a burst whose best frame is another. */
  | "burst"
  | "soft"
  | "dark"
  | "bright";

export interface Suggestion {
  /** 1 pick, -1 reject. */
  flag: 1 | -1;
  reason: SuggestReason;
  /** For "a better frame of this moment": which frame that is. */
  keeperId?: number;
}

/**
 * How far from the better frame another can be and still be "this moment".
 * A burst group chains frame to frame, so the same framing shot every
 * few seconds — vows, rings, the kiss — could be one group the length of a
 * ceremony, and picking the kiss suggested binning the rest. Frames with no
 * capture time are taken as the same moment, as before.
 */
export const MOMENT_SECS = 20;

const secondsApart = (a: Photo, b: Photo): number | null => {
  const ta = parseCapture(a.captureDate);
  const tb = parseCapture(b.captureDate);
  return ta && tb ? Math.abs(ta.getTime() - tb.getTime()) / 1000 : null;
};
const sameMoment = (a: Photo, b: Photo) => (secondsApart(a, b) ?? 0) <= MOMENT_SECS;

const SUGGEST_REASON_KEY: Record<SuggestReason, Key> = {
  best: "filters.suggest.best",
  clean: "filters.suggest.clean",
  burst: "filters.suggest.burst",
  soft: "filters.suggest.soft",
  dark: "filters.suggest.dark",
  bright: "filters.suggest.bright",
};

/**
 * How each reason reads after "Suggested pick · " or "Suggested reject · ", in
 * the active language. Getters, so the words are looked up where they are shown
 * and not once at import.
 */
export const SUGGEST_REASON: Record<SuggestReason, string> = {
  get best() {
    return t(SUGGEST_REASON_KEY.best);
  },
  get clean() {
    return t(SUGGEST_REASON_KEY.clean);
  },
  get burst() {
    return t(SUGGEST_REASON_KEY.burst);
  },
  get soft() {
    return t(SUGGEST_REASON_KEY.soft);
  },
  get dark() {
    return t(SUGGEST_REASON_KEY.dark);
  },
  get bright() {
    return t(SUGGEST_REASON_KEY.bright);
  },
};

/**
 * Mean brightness (0 black … 1 white, from the camera's own rendering) below
 * which a frame failed rather than was exposed for a mood: a lens cap, a flash
 * that didn't fire.
 *
 * Measured on a real library of 1,115 frames (2026-09-27): every one fell
 * between 0.053 and 0.611, the darkest a deliberately low-key stained-glass
 * interior. So 0.02 flags nothing a person meant to take, with room to spare
 * for night and concert work the sample didn't have. Deliberately cautious —
 * a missed black frame costs a keystroke, a wrong reject could cost a photo.
 */
export const TOO_DARK = 0.02;
/** And above which it is blown to white. The same library's brightest frame
 *  (an overcast seascape) measured 0.61. */
export const TOO_BRIGHT = 0.97;

/** Not yet judged by a person: no rating and no flag. What Cull's "To review" means. */
export function untouched(p: Photo): boolean {
  return p.rating === 0 && p.flag === 0;
}

/** A frame that failed outright, by brightness — never for astro, where near-black is the picture. */
export function exposureFailure(p: Photo): "dark" | "bright" | null {
  if (p.brightness == null) return null;
  if (p.brightness < TOO_DARK && p.genre !== "astro") return "dark";
  if (p.brightness > TOO_BRIGHT) return "bright";
  return null;
}

/**
 * Suggestions for the untouched frames of `photos`, keyed by photo id.
 *
 * `groups` are the bursts and duplicates (`duplicate_groups`, over the whole
 * library); only their members in `photos` count, so a burst half outside the
 * cull is judged on the half inside it. "Soft" is relative to `photos`, as the
 * rest of Cull judges it.
 */
export function suggestCull(
  photos: Photo[],
  groups: number[][],
  // Passed in by a caller that keeps it across verdicts: a verdict
  // never changes a frame's focus, so re-sorting every score was waste.
  median: number | null = medianSharpness(photos),
): Map<number, Suggestion> {
  const out = new Map<number, Suggestion>();
  const byId = new Map(photos.map((p) => [p.id, p]));
  const flaw = (p: Photo): SuggestReason | null => exposureFailure(p) ?? (isSoft(p, median) ? "soft" : null);

  const single = (p: Photo) => {
    if (!untouched(p)) return;
    const why = flaw(p);
    if (why) out.set(p.id, { flag: -1, reason: why });
    else if (p.sharpness != null) out.set(p.id, { flag: 1, reason: "clean" });
  };

  const grouped = new Set<number>();
  for (const ids of groups) {
    const members = ids.map((id) => byId.get(id)).filter((p): p is Photo => p !== undefined);
    if (members.length < 2) continue;
    for (const p of members) grouped.add(p.id);

    // Copies of one shot in two folders — a card imported twice, or its backup
    // copy — are not a burst. Which *file* to keep is a folder decision, and
    // suggesting the "extra" for the bin could delete the backup. Left alone,
    // as Duplicates leaves the choice to the person.
    const frames = members.filter((p) => !members.some((o) => sameShot(p, o)));
    if (frames.length < 2) {
      frames.forEach(single);
      continue;
    }

    // The person already chose this moment's keeper: the rest of the moment
    // are extras. Further along the group is another moment, judged alone.
    const chosen = frames.filter((p) => p.flag === 1);
    if (chosen.length > 0) {
      for (const p of frames) {
        if (!untouched(p) || p.sharpness == null) continue;
        const keeper = chosen.find((k) => sameMoment(k, p));
        if (!keeper) single(p);
        else {
          const failed = exposureFailure(p);
          out.set(p.id, failed ? { flag: -1, reason: failed } : { flag: -1, reason: "burst", keeperId: keeper.id });
        }
      }
      continue;
    }

    // Otherwise the best frame the person hasn't rejected. With nothing scored
    // there is no basis to prefer one, so only outright failures are named.
    const candidates = frames.filter((p) => p.flag !== -1 && p.sharpness != null);
    const keeper = candidates.length > 0 ? [...candidates].sort(keeperOrder(candidates))[0]! : null;
    for (const p of frames) {
      if (!untouched(p)) continue;
      const failed = exposureFailure(p);
      if (failed) out.set(p.id, { flag: -1, reason: failed });
      else if (!keeper || p.sharpness == null) continue;
      else if (p.id !== keeper.id && !sameMoment(keeper, p)) single(p);
      else if (p.id !== keeper.id) out.set(p.id, { flag: -1, reason: "burst", keeperId: keeper.id });
      // The best frame of a moment is proposed only when nothing is wrong with
      // it. Whether a moment whose best frame is still soft is worth keeping at
      // all is the person's call.
      else if (!flaw(p)) out.set(p.id, { flag: 1, reason: "best" });
    }
  }

  for (const p of photos) if (!grouped.has(p.id)) single(p);
  return out;
}

/** How many picks and rejects a set of suggestions proposes. */
export function suggestionCounts(suggestions: Map<number, Suggestion>): { picks: number; rejects: number } {
  let picks = 0;
  let rejects = 0;
  for (const s of suggestions.values()) {
    if (s.flag === 1) picks++;
    else rejects++;
  }
  return { picks, rejects };
}
