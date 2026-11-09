<script lang="ts">
  import type { Summary } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { startDrag } from "$lib/drag";
  import { ago, day, firstName } from "$lib/format";
  import Bot from "@lucide/svelte/icons/bot";
  import Check from "@lucide/svelte/icons/check";
  import MessageSquare from "@lucide/svelte/icons/message-square";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";

  let { s }: { s: Summary } = $props();
  const cursor = $derived(app.cursor === s.id);
  const selected = $derived(app.selection.has(s.id));
  const open = $derived(app.doc?.id === s.id);
  const fresh = $derived(app.fresh.has(s.key));
  const overdue = $derived(s.due !== null && s.due < Date.now() && s.open);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  role="row"
  tabindex="-1"
  draggable="true"
  data-issue={s.id}
  ondragstart={(e) => startDrag(e, s.id, s.title)}
  onclick={(e) => app.clickIssue(s.id, e)}
  aria-selected={selected}
  class="group relative flex h-full cursor-default items-center gap-3 border-b pr-5 pl-6 text-[14px] transition-colors duration-100
    {selected
    ? `border-select-rule ${open ? 'bg-select-strong' : 'bg-select hover:bg-select-strong'}`
    : `border-rule-soft ${open ? 'bg-wash-strong' : 'hover:bg-wash'}`}"
>
  {#if selected}
    <!-- Held: a filled check in the gutter. On the cursor row it gains a
         ring, so the cursor never hides behind the selection. -->
    <span
      class="absolute top-1/2 left-[5px] grid size-[14px] -translate-y-1/2 place-items-center rounded-[4px] bg-link text-sheet
        {cursor && app.focus === 'issues' ? 'ring-2 ring-link/35 ring-offset-1 ring-offset-paper' : ''}"
      aria-hidden="true"><Check class="size-[11px]" strokeWidth={3.25} /></span>
  {:else if cursor && app.focus === "issues"}
    <span class="absolute top-1/2 left-2 h-4 w-[3px] -translate-y-1/2 rounded-full bg-link" aria-hidden="true"></span>
  {/if}
  <span class="grid w-3.5 shrink-0 place-items-center">
    {#if s.priority}<PriorityIcon priority={s.priority} />{/if}
  </span>
  <StateIcon state={s.state} />
  <span class="min-w-0 flex-1 truncate {s.open ? 'text-ink' : 'text-ink-3'}">
    <span class={fresh ? "mark-fresh" : ""} title={fresh ? "Changed on disk since you last looked" : undefined}>{s.title}</span>
  </span>
  {#if s.labels.length}
    <span class="hidden max-w-48 shrink-0 truncate text-[13px] text-ink-3 @3xl:inline">{s.labels.slice(0, 3).join(", ")}</span>
  {/if}
  {#if s.projectName && app.view.scope.kind !== "project"}
    <span class="hidden max-w-32 shrink-0 truncate text-[13px] text-ink-3 capitalize @4xl:inline">{s.projectName}</span>
  {/if}
  {#if s.commentCount}
    <span class="tabular-nums hidden shrink-0 items-center gap-1 text-[13px] text-ink-3 @md:flex"><MessageSquare class="size-3.5" />{s.commentCount}</span>
  {/if}
  {#if s.due}
    <span class="tabular-nums hidden shrink-0 text-[13px] @sm:inline {overdue ? 'font-semibold text-danger' : 'text-ink-3'}">{day(s.due)}</span>
  {/if}
  {#if s.agent}<Bot class="hidden size-3.5 shrink-0 text-ink-3 @lg:block" aria-label="Written by an agent" />{/if}
  {#if s.assignee && s.assigneeName}
    <span class="hidden max-w-24 shrink-0 truncate text-[13px] text-ink-3 @2xl:inline" title={s.assignee}>{firstName(s.assigneeName)}</span>
  {/if}
  <span class="hidden w-[4.75rem] shrink-0 text-right font-mono text-[12px] text-ink-3 @xs:inline">{s.id}</span>
  <span class="tabular-nums hidden w-9 shrink-0 text-right text-[12px] text-ink-3 @sm:inline">{s.updated ? ago(s.updated) : ""}</span>
</div>
