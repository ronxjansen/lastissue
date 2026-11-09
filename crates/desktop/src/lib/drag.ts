// Dragging issues: board cards and list rows carry a short ID; drop
// targets (board columns, list group headers, sidebar projects) act on it
// via `app.heldWith`, so a held issue carries the whole selection.

import { app } from "./app.svelte";

const MIME = "text/x-lastissue-id";

export function startDrag(e: DragEvent, id: string, title: string) {
  const dt = e.dataTransfer;
  if (!dt) return;
  dt.setData(MIME, id);
  dt.effectAllowed = "move";
  // Dragging a held issue carries the whole selection: say so in the ghost.
  const n = app.selection.has(id) ? app.selection.size : 1;
  if (n < 2) return;
  const el = document.createElement("div");
  el.className = "drag-stack";
  const count = document.createElement("b");
  count.textContent = String(n);
  const label = document.createElement("span");
  label.textContent = `${title} and ${n - 1} more`;
  el.append(count, label);
  document.body.append(el);
  dt.setDragImage(el, 16, 18);
  requestAnimationFrame(() => el.remove());
}

/** Whether a drag over a target is an issue drag (not a file, not text). */
export function isIssueDrag(e: DragEvent): boolean {
  return !!e.dataTransfer?.types.includes(MIME);
}

/** Drop-target handlers: `over` lights the target while an issue hovers it. */
export function dropTarget(over: (on: boolean) => void, drop: (id: string) => void) {
  return {
    ondragover: (e: DragEvent) => {
      if (!isIssueDrag(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
      over(true);
    },
    ondragleave: (e: DragEvent) => {
      // Ignore leaving into a child of the target.
      const to = e.relatedTarget as Node | null;
      if (to && (e.currentTarget as Node).contains(to)) return;
      over(false);
    },
    ondrop: (e: DragEvent) => {
      over(false);
      const id = e.dataTransfer?.getData(MIME);
      if (!id) return;
      e.preventDefault();
      drop(id);
    },
  };
}
