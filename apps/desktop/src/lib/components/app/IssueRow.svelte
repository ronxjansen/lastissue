<script lang="ts">
  import type { Summary } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { ago, day } from "$lib/format";
  import Bot from "@lucide/svelte/icons/bot";
  import MessageSquare from "@lucide/svelte/icons/message-square";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";

  let { s }: { s: Summary } = $props();
  const cursor = $derived(app.cursor === s.id);
  const selected = $derived(app.selection.includes(s.id));
  const open = $derived(app.doc?.id === s.id);
  const overdue = $derived(s.due !== null && s.due < Date.now() && !["done", "cancelled", "duplicate"].includes(s.state));

  function click(e: MouseEvent) {
    app.focus = "issues";
    if (e.metaKey) {
      app.cursor = s.id;
      app.run("toggleSelect");
    } else {
      app.open(s.id);
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  role="row"
  tabindex="-1"
  onclick={click}
  class="group flex h-full cursor-default items-center gap-2.5 border-b border-border/50 px-4 text-[13px]
    {open ? 'bg-accent' : selected ? 'bg-primary/8' : 'hover:bg-accent/50'}
    {cursor && app.focus === 'issues' ? 'shadow-[inset_2px_0_0_var(--primary)]' : ''}"
>
  <span class="w-3 text-primary">{selected ? "●" : ""}</span>
  <PriorityIcon priority={s.priority} />
  <span class="w-16 shrink-0 font-mono text-xs text-muted-foreground">{s.id}</span>
  <StateIcon state={s.state} />
  <span class="min-w-0 flex-1 truncate">{s.title}</span>
  {#each s.labels.slice(0, 3) as l (l)}
    <span class="hidden shrink-0 rounded-full border px-2 py-px text-[11px] text-muted-foreground md:inline">{l}</span>
  {/each}
  {#if s.projectSlug && app.view.scope.kind !== "project"}
    <span class="hidden max-w-32 shrink-0 truncate text-xs text-muted-foreground capitalize lg:inline">{s.projectSlug.replaceAll("-", " ")}</span>
  {/if}
  {#if s.commentCount}
    <span class="flex shrink-0 items-center gap-0.5 text-xs text-muted-foreground"><MessageSquare class="size-3" />{s.commentCount}</span>
  {/if}
  {#if s.due}
    <span class="shrink-0 text-xs {overdue ? 'text-destructive' : 'text-muted-foreground'}">{day(s.due)}</span>
  {/if}
  {#if s.agent}<Bot class="size-3.5 shrink-0 text-muted-foreground" />{/if}
  <span
    class="grid size-5 shrink-0 place-items-center rounded-full text-[10px] font-medium {s.assignee ? 'bg-muted text-foreground' : 'border border-dashed text-muted-foreground'}"
    title={s.assignee ?? "No assignee"}
  >
    {s.assigneeName?.slice(0, 1).toUpperCase() ?? ""}
  </span>
  <span class="w-10 shrink-0 text-right text-xs text-muted-foreground tabular-nums">{s.updated ? ago(s.updated) : ""}</span>
</div>
