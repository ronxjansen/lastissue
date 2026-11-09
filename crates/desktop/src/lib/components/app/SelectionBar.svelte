<!-- Floats over the bottom of the list or board while two or more issues
     are held: the count, the bulk pickers, and Clear. Each button opens the
     same picker as its key, aimed at the selection. Monochrome chrome; the
     count is the only link ink (DESIGN.md). The list and board pad their
     ends by the bar's height so it never covers the last row. -->
<script lang="ts">
  import type { PickerKind } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { type Action, pretty } from "$lib/keymap";
  import X from "@lucide/svelte/icons/x";

  const BUTTONS: { kind: PickerKind; label: string; action: Action }[] = [
    { kind: "state", label: "Status", action: "setState" },
    { kind: "priority", label: "Priority", action: "setPriority" },
    { kind: "project", label: "Project", action: "moveToProject" },
    { kind: "labels", label: "Labels", action: "setLabels" },
    { kind: "assignee", label: "Assignee", action: "setAssignee" },
  ];

  /** The key for `a` where the list or board has focus (`l` is labels in
   *  the list but a column move on the board, so no fallback). */
  function key(a: Action): string | null {
    for (const c of [app.visible.view.mode, "issues", "workspace", "global"]) {
      const hit = Object.entries(app.keymap[c] ?? {}).find(([, x]) => x === a);
      if (hit) return pretty(hit[0]);
    }
    return null;
  }

  function pick(kind: PickerKind) {
    // The bar is about the selection, even if the detail pane had focus.
    app.focus = "issues";
    app.pick(kind, [...app.selection]);
  }

  const btn =
    "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[13px] text-ink-2 transition-colors duration-100 hover:bg-wash hover:text-ink";
</script>

<div
  role="toolbar"
  aria-label="Selected issues"
  class="pointer-events-auto flex h-10 items-center gap-0.5 rounded-lg border border-rule bg-sheet pr-1 pl-3 shadow-pop"
>
  <span class="mr-2 flex items-center gap-1.5 text-[13px] text-ink-2">
    <b class="font-mono font-semibold text-link tabular-nums">{app.selection.size}</b>selected
  </span>
  <span class="mr-1 h-4 w-px bg-rule"></span>
  {#each BUTTONS as b (b.kind)}
    {@const k = key(b.action)}
    <button class={btn} onclick={() => pick(b.kind)} title={k ? `${b.label} (${k})` : b.label}>
      {b.label}{#if k}<kbd class="hidden font-mono text-[11px] text-ink-3 @2xl:inline">{k}</kbd>{/if}
    </button>
  {/each}
  <span class="mx-1 h-4 w-px bg-rule"></span>
  <button class={btn} onclick={() => app.select([])} title="Clear selection (esc)">
    <X class="size-3.5" />Clear<kbd class="hidden font-mono text-[11px] text-ink-3 @2xl:inline">{pretty("escape")}</kbd>
  </button>
</div>
