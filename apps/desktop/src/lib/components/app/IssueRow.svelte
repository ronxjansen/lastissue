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
  const fresh = $derived(app.fresh.has(s.key));
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
  class="group relative flex h-full cursor-default items-center gap-3 border-b border-rule-soft pr-5 pl-6 text-[14px] transition-colors duration-100
    {open ? 'bg-wash-strong' : selected ? 'bg-link/8' : 'hover:bg-wash'}"
>
  {#if cursor && app.focus === "issues"}
    <span class="absolute top-1/2 left-2 h-4 w-[3px] -translate-y-1/2 rounded-full bg-link" aria-hidden="true"></span>
  {/if}
  {#if selected}
    <span class="absolute top-1/2 left-[13px] size-1.5 -translate-y-1/2 rounded-full bg-link" aria-label="Selected"></span>
  {/if}
  <span class="grid w-3.5 shrink-0 place-items-center">
    {#if s.priority}<PriorityIcon priority={s.priority} />{/if}
  </span>
  <StateIcon state={s.state} />
  <span class="min-w-0 flex-1 truncate {s.state === 'done' || s.state === 'cancelled' || s.state === 'duplicate' ? 'text-ink-3' : 'text-ink'}">
    <span class={fresh ? "mark-fresh" : ""} title={fresh ? "Changed on disk since you last looked" : undefined}>{s.title}</span>
  </span>
  {#if s.labels.length}
    <span class="hidden max-w-48 shrink-0 truncate text-[13px] text-ink-3 @3xl:inline">{s.labels.slice(0, 3).join(", ")}</span>
  {/if}
  {#if s.projectSlug && app.view.scope.kind !== "project"}
    <span class="hidden max-w-32 shrink-0 truncate text-[13px] text-ink-3 capitalize @4xl:inline">{s.projectSlug.replaceAll("-", " ")}</span>
  {/if}
  {#if s.commentCount}
    <span class="tnum hidden shrink-0 items-center gap-1 text-[13px] text-ink-3 @md:flex"><MessageSquare class="size-3.5" />{s.commentCount}</span>
  {/if}
  {#if s.due}
    <span class="tnum hidden shrink-0 text-[13px] @sm:inline {overdue ? 'font-semibold text-danger' : 'text-ink-3'}">{day(s.due)}</span>
  {/if}
  {#if s.agent}<Bot class="hidden size-3.5 shrink-0 text-ink-3 @lg:block" aria-label="Written by an agent" />{/if}
  {#if s.assignee}
    <span class="hidden max-w-24 shrink-0 truncate text-[13px] text-ink-3 @2xl:inline" title={s.assignee}>{s.assigneeName?.split(" ")[0]}</span>
  {/if}
  <span class="hidden w-[4.75rem] shrink-0 text-right font-mono text-[12px] text-ink-3 @xs:inline">{s.id}</span>
  <span class="tnum hidden w-9 shrink-0 text-right text-[12px] text-ink-3 @sm:inline">{s.updated ? ago(s.updated) : ""}</span>
</div>
