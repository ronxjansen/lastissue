// The fuzzy matcher for the palette and the pickers:
// lower is better, null is no match.
// Prefix 0, word start 1, substring 2+i, scattered letters 100+last.

export function score(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  if (t.startsWith(q)) return 0;
  const i = t.indexOf(q);
  if (i >= 0) return /[ \-<@.]$/.test(t.slice(0, i)) ? 1 : 2 + i;
  let pos = -1;
  for (const c of q) {
    pos = t.indexOf(c, pos + 1);
    if (pos < 0) return null;
  }
  return 100 + pos;
}

/** Items matching `query`, best first, stable among equals. */
export function rank<T>(items: T[], query: string, text: (t: T) => string): T[] {
  return items
    .map((item, i) => ({ item, i, s: score(query, text(item)) }))
    .filter((x): x is { item: T; i: number; s: number } => x.s !== null)
    .sort((a, b) => a.s - b.s || a.i - b.i)
    .map((x) => x.item);
}

/** Move a highlight by `step` through `len` rows, wrapping. */
export function step(cur: number, len: number, by: number): number {
  return len === 0 ? 0 : (((cur + by) % len) + len) % len;
}
