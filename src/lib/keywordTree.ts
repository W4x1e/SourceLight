import type { FacetCount, KeywordNode } from "../bindings";
import { tagName } from "../i18n/tags";

/**
 * The keyword tree, as the rail draws it.
 *
 * Its shape comes from `keyword_tree` and its counts from the facets, which
 * follow the filter: a keyword with others filed beneath it already counts its
 * whole branch there. Keywords are addressed by name throughout, as the filter
 * and saved views address them; names are unique among keywords.
 */

export interface KeywordRow {
  name: string;
  /** Photos under the current filter, the whole branch counted once each. */
  count: number;
  children: KeywordRow[];
  /** Part of the tree someone built — filed, made by hand, or holding others —
   *  so shown even while nothing under the filter has it. */
  filed: boolean;
}

/** One row as drawn: the tree walked in order, with how deep each sits. */
export interface FlatRow {
  row: KeywordRow;
  depth: number;
}

/** What each keyword is filed under, by name. */
export function parentsOf(nodes: KeywordNode[]): Map<string, string> {
  const out = new Map<string, string>();
  for (const n of nodes) if (n.parent) out.set(n.name, n.parent);
  return out;
}

/** A keyword's path, root first and ending with it. A loop — which the backend
 *  refuses to make — is cut rather than followed. */
export function pathOf(parents: Map<string, string>, name: string): string[] {
  const path = [name];
  let at = parents.get(name);
  while (at !== undefined && !path.includes(at)) {
    path.unshift(at);
    at = parents.get(at);
  }
  return path;
}

/** Whether `name` is `branch` or filed anywhere beneath it. */
export function inBranch(parents: Map<string, string>, name: string, branch: string): boolean {
  return pathOf(parents, name).includes(branch);
}

/** Most photos first, then by name — the order the flat facets use. */
const byCount = (a: KeywordRow, b: KeywordRow) => b.count - a.count || a.name.localeCompare(b.name);
/** Alphabetical: inside a branch someone built, rows shouldn't jump about as
 *  the filter changes the counts. */
const byName = (a: KeywordRow, b: KeywordRow) => a.name.localeCompare(b.name);
/** The top level: the tree someone built first, by name — it is what gets
 *  arranged, and a keyword just made has no photos to rank it by, so by count
 *  it sank below hundreds of the tagger's words, out of sight behind "Show
 *  more" at the moment it was wanted. Then the rest, most-used first. */
const topLevel = (a: KeywordRow, b: KeywordRow) =>
  Number(b.filed) - Number(a.filed) || (a.filed ? byName(a, b) : byCount(a, b));

/**
 * The rows to draw. A keyword is shown when photos under the filter have it,
 * when it is chosen (so a choice can always be taken back), or when it is part
 * of the tree someone built. A word of the tagger's that nobody has filed and
 * that nothing under the filter has stays out, as it did in the flat list.
 *
 * At the top level the user's own keywords — made by hand, or with others
 * filed beneath them — come first, by name; then the rest most-used first,
 * like every facet. A branch's own keywords are alphabetical.
 */
export function buildKeywordTree(nodes: KeywordNode[], counts: FacetCount[], chosen: string[]): KeywordRow[] {
  const count = new Map(counts.map((c) => [c.value, c.count]));
  const parents = parentsOf(nodes);
  const hasChildren = new Set(parents.values());
  const rows = new Map<string, KeywordRow>();
  const row = (name: string): KeywordRow => {
    let r = rows.get(name);
    if (!r) {
      r = { name, count: count.get(name) ?? 0, children: [], filed: false };
      rows.set(name, r);
    }
    return r;
  };
  for (const n of nodes) {
    const filed = n.kept || n.parent !== null || hasChildren.has(n.name);
    if (filed || (count.get(n.name) ?? 0) > 0 || chosen.includes(n.name)) row(n.name).filed ||= filed;
  }
  // Counted but not in the tree's list yet (it was read before a tagging
  // run finished): still a keyword, at the top.
  for (const c of counts) if (c.count > 0) row(c.value);
  for (const name of chosen) row(name);

  const roots: KeywordRow[] = [];
  for (const r of rows.values()) {
    const p = parents.get(r.name);
    // A parent is always drawn: it has a child, so it is filed.
    const parent = p !== undefined && !inBranch(parents, p, r.name) ? (rows.get(p) ?? row(p)) : undefined;
    if (parent) {
      parent.filed = true;
      parent.children.push(r);
    } else roots.push(r);
  }
  const sortBranch = (list: KeywordRow[]) => {
    list.sort(byName);
    for (const r of list) sortBranch(r.children);
  };
  for (const r of roots) sortBranch(r.children);
  return roots.sort(topLevel);
}

/** The tree cut down to keywords whose name holds `query`, with the branches
 *  that lead to them. Case-insensitive; an empty query keeps everything. */
export function findInTree(rows: KeywordRow[], query: string): KeywordRow[] {
  const q = query.trim().toLowerCase();
  if (!q) return rows;
  const keep = (r: KeywordRow): KeywordRow | null => {
    const children = r.children.map(keep).filter((c): c is KeywordRow => c !== null);
    // By the stored name or the one shown, so "Strand" finds "beach".
    return r.name.includes(q) || tagName(r.name).toLowerCase().includes(q) || children.length > 0
      ? { ...r, children }
      : null;
  };
  return rows.map(keep).filter((r): r is KeywordRow => r !== null);
}

/** The rows in drawing order: each one's children follow it while it is open. */
export function flatten(rows: KeywordRow[], isOpen: (name: string) => boolean, depth = 0): FlatRow[] {
  const out: FlatRow[] = [];
  for (const row of rows) {
    out.push({ row, depth });
    if (row.children.length > 0 && isOpen(row.name)) out.push(...flatten(row.children, isOpen, depth + 1));
  }
  return out;
}

/** How many of the given photos have each keyword, for marking what a
 *  selection has: all of them, some, or none. */
export function selectionCounts(photos: { subjects: string[] }[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const p of photos) for (const s of p.subjects) out.set(s, (out.get(s) ?? 0) + 1);
  return out;
}
