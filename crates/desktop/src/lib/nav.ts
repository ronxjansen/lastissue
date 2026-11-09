// Keyboard navigation over the visible issues: plain
// functions over columns of IDs (the list is one column in display order,
// the board one column per group). The cursor is the keyboard's position;
// the selection is the set `x` toggles (and shift extends); the open issue is separate.

export type Columns = string[][];

export function locate(cols: Columns, id: string): [number, number] | null {
  for (let c = 0; c < cols.length; c++) {
    const r = cols[c].indexOf(id);
    if (r >= 0) return [c, r];
  }
  return null;
}

function first(cols: Columns): string | null {
  for (const c of cols) if (c.length) return c[0];
  return null;
}

/** `j`/`k`: move within the column, clamped; no cursor → the first issue. */
export function vertical(cols: Columns, cursor: string | null, step: number): string | null {
  const at = cursor ? locate(cols, cursor) : null;
  if (!at) return first(cols);
  const col = cols[at[0]];
  const r = Math.min(Math.max(at[1] + step, 0), col.length - 1);
  return col[r] ?? null;
}

/** `h`/`l`: the nearest non-empty column that way, same row or its last. */
export function horizontal(cols: Columns, cursor: string | null, step: number): string | null {
  const at = cursor ? locate(cols, cursor) : null;
  if (!at) return first(cols);
  const [c, r] = at;
  const dir = Math.sign(step);
  for (let i = c + dir; i >= 0 && i < cols.length; i += dir) {
    if (cols[i].length) return cols[i][Math.min(r, cols[i].length - 1)];
  }
  return cols[c][r] ?? null;
}

/** Toggle `id` in the selection (a new Set; insertion order is kept). */
export function toggle(sel: ReadonlySet<string>, id: string): Set<string> {
  const out = new Set(sel);
  if (!out.delete(id)) out.add(id);
  return out;
}

/** The issues from `from` to `to`, inclusive, in display order, within
 *  `to`'s column. If `from` is elsewhere (another column, gone), just `to`. */
export function range(cols: Columns, from: string | null, to: string): string[] {
  const b = locate(cols, to);
  if (!b) return [];
  const a = from ? locate(cols, from) : null;
  if (!a || a[0] !== b[0]) return [to];
  const [lo, hi] = a[1] < b[1] ? [a[1], b[1]] : [b[1], a[1]];
  return cols[b[0]].slice(lo, hi + 1);
}
