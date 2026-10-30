import type { Photo } from "../types";

/**
 * Choosing several frames at once in Cull's survey: a verdict key then
 * applies to all of them. The survey had one cursor only, so rejecting the
 * five soft frames of a burst took five trips round it.
 */

/**
 * The frames chosen after a click on frame `i` of `ids`, with the cursor at
 * `cursor`:
 * - **Shift** chooses the run from the cursor to `i` (adding to what was
 *   chosen when Ctrl is held too, as in Explorer and other editors);
 * - **Ctrl** (⌘ on a Mac) adds or removes `i` — and the cursor's frame comes
 *   along on the first Ctrl-click, since it was the one being looked at;
 * - a plain click chooses nothing: it only moves the cursor.
 */
export function clickSelect(
  prev: ReadonlySet<number>,
  ids: readonly number[],
  cursor: number,
  i: number,
  mods: { toggle: boolean; range: boolean },
): Set<number> {
  const id = ids[i];
  if (id === undefined) return new Set(prev);
  if (mods.range) {
    const next = new Set(mods.toggle ? prev : []);
    const [a, b] = cursor <= i ? [cursor, i] : [i, cursor];
    for (let k = Math.max(0, a); k <= b && k < ids.length; k++) next.add(ids[k]!);
    return next;
  }
  if (mods.toggle) {
    const next = new Set(prev);
    const here = ids[cursor];
    if (next.size === 0 && here !== undefined && here !== id) next.add(here);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  }
  return new Set();
}

/**
 * The flag a flag key sets on several frames. It toggles, as on one frame:
 * when every chosen frame already has it, it comes off them all; otherwise
 * they all get it.
 */
export function flagForAll(photos: readonly Photo[], flag: number): number {
  return photos.length > 0 && photos.every((p) => p.flag === flag) ? 0 : flag;
}
