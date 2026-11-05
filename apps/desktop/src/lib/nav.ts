// Keyboard navigation over the visible issues (GUI-SPEC §7): plain
// functions over columns of IDs (the list is one column in display order,
// the board one column per group). The cursor is the keyboard's position;
// the selection is the set `x` toggles; the open issue is separate.

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

/** Toggle `id` in the ordered selection. */
export function toggle(sel: string[], id: string): string[] {
  return sel.includes(id) ? sel.filter((x) => x !== id) : [...sel, id];
}
