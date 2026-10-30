import type { Key } from "../i18n";
/**
 * Settling every duplicate group at once.
 *
 * Duplicate review goes a group at a time, which is right for a burst and slow
 * for an import of old backups with hundreds of groups of the same files.
 * This settles them all at once, by one rule the user picks. It only
 * flags — the keeper a pick, the rest rejects — so the decision stays visible,
 * filterable and undoable, and deleting files is still the user's own step.
 */

/** What a group's photos need for the rules to choose between them. */
export interface GroupPhoto {
  id: number;
  path: string;
  width: number;
  height: number;
  rating: number;
  flag: number;
  sharpness: number | null;
}

/** Which photo of each group stays. */
export type Keeper = "raw" | "largest" | "best" | "first";

export const KEEPERS: { id: Keeper; label: Key; blurb: Key }[] = [
  { id: "raw", label: "viewer.resolve.raw", blurb: "viewer.resolve.rawBlurb" },
  { id: "largest", label: "viewer.resolve.largest", blurb: "viewer.resolve.largestBlurb" },
  { id: "best", label: "viewer.resolve.best", blurb: "viewer.resolve.bestBlurb" },
  { id: "first", label: "viewer.resolve.first", blurb: "viewer.resolve.firstBlurb" },
];

export interface ResolveOptions {
  keeper: Keeper;
  /** Leave any group where a photo already has a pick or reject. */
  skipDecided: boolean;
  /** Flag each keeper as a pick, not just the rest as rejects. */
  pickKeepers: boolean;
}

export interface Resolution {
  /** The flags to set, as one undoable change. */
  changes: { id: number; flag: number }[];
  /** Groups settled, and left alone for having a verdict already. */
  groups: number;
  skipped: number;
  rejects: number;
}

const RAW = new Set([
  "arw", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "rw2", "dng", "pef", "srw", "srf", "sr2", "rwl", "3fr",
  "fff", "iiq", "x3f",
]);

export function isRaw(path: string): boolean {
  return RAW.has(path.split(".").pop()?.toLowerCase() ?? "");
}

const pixels = (p: GroupPhoto) => p.width * p.height;
const sharp = (p: GroupPhoto) => p.sharpness ?? -1;

/** The order each rule ranks a group's photos in; the first is kept. Ties fall
 *  through to the next test and, last, to the lowest id, so a rule always
 *  chooses the same photo. */
const ORDER: Record<Keeper, (a: GroupPhoto, b: GroupPhoto) => number> = {
  raw: (a, b) => Number(isRaw(b.path)) - Number(isRaw(a.path)) || pixels(b) - pixels(a) || sharp(b) - sharp(a) || a.id - b.id,
  largest: (a, b) => pixels(b) - pixels(a) || Number(isRaw(b.path)) - Number(isRaw(a.path)) || sharp(b) - sharp(a) || a.id - b.id,
  best: (a, b) => b.rating - a.rating || sharp(b) - sharp(a) || pixels(b) - pixels(a) || a.id - b.id,
  first: (a, b) => a.id - b.id,
};

/** The photo a rule keeps from a group. */
export function chooseKeeper(group: GroupPhoto[], keeper: Keeper): number | null {
  return [...group].sort(ORDER[keeper])[0]?.id ?? null;
}

/** What settling `groups` by `opts` would do: a flag for each photo whose flag
 *  would change, and the counts for saying so first. */
export function resolveAll(groups: GroupPhoto[][], opts: ResolveOptions): Resolution {
  const out: Resolution = { changes: [], groups: 0, skipped: 0, rejects: 0 };
  const seen = new Set<number>();
  for (const group of groups) {
    if (group.length < 2) continue;
    if (opts.skipDecided && group.some((p) => p.flag !== 0)) {
      out.skipped++;
      continue;
    }
    const keep = chooseKeeper(group, opts.keeper);
    out.groups++;
    for (const p of group) {
      // A photo in two groups is decided by the first.
      if (seen.has(p.id)) continue;
      seen.add(p.id);
      const flag = p.id === keep ? (opts.pickKeepers ? 1 : p.flag) : -1;
      if (flag === -1) out.rejects++;
      if (flag !== p.flag) out.changes.push({ id: p.id, flag });
    }
  }
  return out;
}
