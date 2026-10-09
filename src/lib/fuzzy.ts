/**
 * How well `query` matches `text`: every query character must appear in order. Matches at the
 * start, after a separator, and runs of consecutive characters score higher. `null` = no match.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  if (t.includes(q)) return 1000 - t.indexOf(q) * 2 - (t.length - q.length) * 0.1;
  let score = 0;
  let ti = 0;
  let run = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    const found = t.indexOf(ch, ti);
    if (found === -1) return null;
    const boundary = found === 0 || /[\s._\-/:]/.test(t[found - 1]);
    run = found === ti ? run + 1 : 0;
    score += 10 + (boundary ? 15 : 0) + run * 5 - Math.min(found - ti, 10);
    ti = found + 1;
  }
  return score;
}

/** Items that match, best first. */
export function fuzzyFilter<T>(items: T[], query: string, text: (item: T) => string): T[] {
  if (!query.trim()) return items;
  return items
    .map((item, i) => ({ item, i, score: fuzzyScore(query, text(item)) }))
    .filter((x): x is { item: T; i: number; score: number } => x.score !== null)
    .sort((a, b) => b.score - a.score || a.i - b.i)
    .map((x) => x.item);
}
