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
  <VirtualList items={rows} rowHeight={40} scrollTo={cursorRow} class="h-full">
    {#snippet row(r)}
      {#if r.kind === "header"}
        <div class="flex h-full items-end gap-2 border-b border-rule bg-paper pb-2 pl-11 text-[12.5px] font-bold text-ink-2" data-group={groupKeyId(r.key)}>
          {#if r.key.kind === "state"}<StateIcon state={r.key.value} />{:else}<PriorityIcon priority={r.key.value} />{/if}
          {r.label}
          <span class="font-mono text-[12px] font-normal text-ink-3">{r.count}</span>
        </div>
      {:else}
        {@const s = app.issues.get(r.key)}
        {#if s}<IssueRow {s} />{/if}
      {/if}
    {/snippet}
  </VirtualList>
{:else}
  <div class="grid h-full place-items-center text-[14px] text-ink-3">
    {#if !app.meta.loaded && app.meta.current !== null}
      Loading…
    {:else if app.visible.error}
      <span class="text-danger">{app.visible.error}</span>
    {:else if app.view.filter}
      <div class="text-center">
        <p class="text-ink-2">No issues match <span class="rounded bg-wash px-1.5 py-0.5 font-mono text-[12.5px]">{app.view.filter}</span></p>
        <p class="mt-1.5 text-[13px]">Try fewer tokens, or clear the filter in the toolbar.</p>
      </div>
    {:else}
      <div class="text-center">
        <p class="font-mono text-[13px] text-ink-3">$ ls issues/<br /><span class="opacity-70">(empty)</span></p>
        <p class="mt-3 text-ink-2">Nothing here yet. Press <kbd class="mx-0.5 rounded border border-rule bg-sheet px-1.5 text-[12px] text-ink">c</kbd> to write the first issue.</p>
      </div>
    {/if}
  </div>
{/if}
