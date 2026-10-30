/**
 * Matching for the command palette: what to show for what was typed.
 *
 * Every letter typed must appear in the command's name, in order, but not
 * together — "exsel" finds "Export selected…". A match scores higher where
 * the letters start words and run on unbroken, and a shorter name beats a
 * longer one that matches as well, so "cull" puts "Go to Cull" above
 * "Accept the suggested pick … in Cull".
 */

/** How well `query` matches `text`: `null` for no match, higher for better. */
export function matchScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  let at = 0;
  let run = 0;
  for (const ch of q) {
    if (ch === " ") {
      run = 0;
      continue;
    }
    const i = t.indexOf(ch, at);
    if (i < 0) return null;
    const wordStart = i === 0 || /[\s\-/(·]/.test(t[i - 1] ?? "");
    run = i === at && at > 0 ? run + 1 : 0;
    score += 1 + (wordStart ? 6 : 0) + run * 3 - Math.min(i - at, 8) * 0.25;
    at = i + 1;
  }
  return score - t.length * 0.02;
}

export interface Command {
  id: string;
  label: string;
  /** Where it comes from: "App", "Photo", a person, a place … */
  group: string;
  /** The key it is bound to, as shown. */
  keys?: string;
  /** Other words it answers to, beyond its label. */
  aliases?: string[];
  run: () => void;
}

/** The commands matching `query`, best first; all of them, in order, for none. */
export function rankCommands(commands: readonly Command[], query: string, limit = 50): Command[] {
  if (!query.trim()) return commands.slice(0, limit);
  const scored: { c: Command; s: number }[] = [];
  for (const c of commands) {
    const best = [c.label, ...(c.aliases ?? [])]
      .map((t) => matchScore(query, t))
      .reduce<number | null>((a, b) => (b === null ? a : a === null ? b : Math.max(a, b)), null);
    if (best !== null) scored.push({ c, s: best });
  }
  return scored
    .sort((a, b) => b.s - a.s)
    .slice(0, limit)
    .map((x) => x.c);
}
