<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { groupKeyId } from "$lib/format";
  import IssueRow from "./IssueRow.svelte";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";
  import VirtualList from "./VirtualList.svelte";
  import type { GroupKey } from "$lib/api";

  type Row = { kind: "header"; key: GroupKey; label: string; count: number } | { kind: "issue"; key: string };

  const rows = $derived.by((): Row[] => {
    const out: Row[] = [];
    for (const gi of app.visible.list) {
      const g = app.visible.groups[gi];
      out.push({ kind: "header", key: g.key, label: g.label, count: g.items.length });
      for (const k of g.items) out.push({ kind: "issue", key: k });
    }
    return out;
  });

  const cursorRow = $derived.by(() => {
    const id = app.cursor;
    if (!id) return null;
    const i = rows.findIndex((r) => r.kind === "issue" && app.issues.get(r.key)?.id === id);
    return i >= 0 ? i : null;
  });
</script>

{#if rows.length}
  <VirtualList items={rows} rowHeight={36} scrollTo={cursorRow} class="h-full">
    {#snippet row(r)}
      {#if r.kind === "header"}
        <div class="flex h-full items-center gap-2 border-b bg-muted/40 px-4 text-xs font-medium" data-group={groupKeyId(r.key)}>
          {#if r.key.kind === "state"}<StateIcon state={r.key.value} />{:else}<PriorityIcon priority={r.key.value} />{/if}
          {r.label}
          <span class="text-muted-foreground tabular-nums">{r.count}</span>
        </div>
      {:else}
        {@const s = app.issues.get(r.key)}
        {#if s}<IssueRow {s} />{/if}
      {/if}
    {/snippet}
  </VirtualList>
{:else}
  <div class="grid h-full place-items-center text-sm text-muted-foreground">
    {#if !app.meta.loaded && app.meta.current !== null}Loading…{:else if app.visible.error}{app.visible.error}{:else if app.view.filter}No issues match the filter{:else}No issues here — press <kbd class="mx-1 rounded border px-1">c</kbd> to create one{/if}
  </div>
{/if}
