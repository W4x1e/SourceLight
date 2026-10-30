/**
 * Burst stacking for the library grid.
 *
 * A burst — a run of near-identical frames from a duplicate group — collapses to
 * a single grid cell so the grid reads as distinct shots, not a wall of copies.
 * The maths is pure and lives here (a flat list of cells the grid renders one
 * per tile), so it can be tested without a DOM or the virtualizer, exactly like
 * `survey.ts`.
 */
import type { Photo } from "../types";

export interface StackInfo {
  /** Stable id for the stack — its representative photo's id. */
  id: number;
  /** How many photos the stack holds. */
  count: number;
  /** The representative cell carries the badge and the expand/collapse click. */
  head: boolean;
  /** Whether the stack is currently expanded (its members shown inline). */
  expanded: boolean;
}

export interface GridCell {
  photo: Photo;
  /** Present when this cell belongs to a burst stack. */
  stack?: StackInfo;
}

/**
 * Collapses near-duplicate groups into single grid cells.
 *
 * `groups` are the id lists from `duplicate_groups`; only those with two or more
 * members still present in `photos` become stacks. A stack sits where its
 * earliest member appears in `photos`, and shows the group's **first** member
 * as its cover: `toggleStacking` orders each group sharpest first, so a
 * collapsed burst shows its best frame (it used to show whichever came
 * first on screen, contradicting that). Its other members are hidden — unless
 * the stack's id is in `expanded`, in which case they are emitted inline right
 * after the cover, in display order, so a burst reads as a contiguous run
 * rather than staying scattered through the grid.
 *
 * Photos in no stack pass through untouched and in order, so with no groups the
 * result is simply `photos`, one per cell.
 */
export function buildStackCells(
  photos: Photo[],
  groups: number[][],
  expanded: ReadonlySet<number>,
): GridCell[] {
  const pos = new Map<number, number>();
  photos.forEach((p, i) => pos.set(p.id, i));

  // Group member id -> the stack it belongs to: its cover first (the stack id
  // is the cover's photo id), then the rest in display order, and the member
  // that comes first on screen, where the stack sits.
  const stackOf = new Map<number, { id: number; at: number; members: Photo[] }>();
  for (const group of groups) {
    const onScreen = group.filter((id) => pos.has(id));
    // A group that keeps fewer than two on-screen frames is nothing to stack.
    if (onScreen.length < 2) continue;
    const [cover, ...rest] = onScreen;
    const byPlace = (a: number, b: number) => pos.get(a)! - pos.get(b)!;
    const members = [cover!, ...rest.sort(byPlace)].map((id) => photos[pos.get(id)!]!);
    const at = [...onScreen].sort(byPlace)[0]!;
    const rec = { id: cover!, at, members };
    for (const m of members) stackOf.set(m.id, rec);
  }

  const cells: GridCell[] = [];
  const emitted = new Set<number>();
  for (const p of photos) {
    const rec = stackOf.get(p.id);
    if (!rec) {
      cells.push({ photo: p });
      continue;
    }
    // Every member is drawn (or hidden) at the stack's place, not its own.
    if (rec.at !== p.id || emitted.has(rec.id)) continue;
    emitted.add(rec.id);

    const isExpanded = expanded.has(rec.id);
    const info = (head: boolean): StackInfo => ({
      id: rec.id,
      count: rec.members.length,
      head,
      expanded: isExpanded,
    });
    cells.push({ photo: rec.members[0]!, stack: info(true) });
    if (isExpanded) {
      for (const m of rec.members) {
        if (m.id !== rec.id) cells.push({ photo: m, stack: info(false) });
      }
    }
  }
  return cells;
}
