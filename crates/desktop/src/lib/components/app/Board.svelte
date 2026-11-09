<!-- Kanban: a column per state (or priority band). Dropping
     a card on another column writes exactly what `issue update --state`
     would; there is no manual order, so dropping in its own column is a
     no-op. ⇧h / ⇧l do the same from the keyboard. -->
<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { dropTarget, startDrag } from "$lib/drag";
  import { day, firstName, groupKeyId, groupLabel } from "$lib/format";
  import Check from "@lucide/svelte/icons/check";
  import MessageSquare from "@lucide/svelte/icons/message-square";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";
  import VirtualList from "./VirtualList.svelte";
  import GroupIcon from "./GroupIcon.svelte";

  let over = $state<string | null>(null);

  /** Room for the selection bar under the last card. */
  const pad = $derived(app.selection.size > 1 ? 56 : 0);
</script>

<div class="flex h-full gap-3 divide-x divide-rule-soft overflow-x-auto px-4 pt-3 pb-3">
  {#each app.visible.groups as g (groupKeyId(g.key))}
    {@const gid = groupKeyId(g.key)}
    <section
      class="flex h-full w-[19rem] shrink-0 flex-col rounded-lg transition-colors duration-150 {over === gid ? 'bg-mark-soft' : ''}"
      aria-label={groupLabel(g.key)}
      {...dropTarget(
        (on) => (over = on ? gid : over === gid ? null : over),
        (id) => app.dropCard(id, g.key),
      )}
    >
      <header class="flex items-center gap-2 px-3 pt-2 pb-2.5 text-[13px] font-bold text-ink-2">
        <GroupIcon key={g.key} />
        {groupLabel(g.key)}
        <span class="font-mono text-[12px] font-normal text-ink-3">{g.items.length}</span>
      </header>
      <VirtualList items={g.items} rowHeight={90} scrollTo={g.items.findIndex(app.atCursor)} class="flex-1 px-2" overscan={4} padEnd={pad}>
        {#snippet row(k)}
          {@const s = app.issues.get(k)}
          {#if s}
            {@const held = app.selection.has(s.id)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <article
              draggable="true"
              data-issue={s.id}
              ondragstart={(e) => startDrag(e, s.id, s.title)}
              onclick={(e) => app.clickIssue(s.id, e)}
              class="relative mb-2 flex h-[82px] cursor-default flex-col justify-between rounded-lg border px-3 py-2.5 text-[14px] transition-[border-color,box-shadow,background] duration-100
                {held ? 'card-selected' : `bg-sheet hover:border-ink-3/40 ${app.doc?.id === s.id ? 'border-ink-3/60' : 'border-rule'}`}
                {app.cursor === s.id && app.focus === 'issues' ? 'shadow-[0_0_0_2px_var(--link)]' : ''}"
            >
              {#if held}
                <span class="absolute top-2.5 right-2.5 grid size-4 place-items-center rounded-[4px] bg-link text-sheet" aria-hidden="true"
                  ><Check class="size-3" strokeWidth={3.25} /></span>
              {/if}
              <div class="line-clamp-2 leading-[1.35] text-ink {held ? 'pr-5' : ''}">
                <span class={app.fresh.has(s.key) ? "mark-fresh" : ""}>{s.title}</span>
              </div>
              <div class="flex items-center gap-2 text-[12.5px] text-ink-3">
                {#if g.key.kind === "state"}{#if s.priority}<PriorityIcon priority={s.priority} size={12} />{/if}{:else}<StateIcon state={s.state} size={12} />{/if}
                {#if s.labels.length}<span class="min-w-0 truncate">{s.labels.slice(0, 2).join(", ")}</span>{/if}
                <span class="flex-1"></span>
                {#if s.commentCount}<span class="tabular-nums flex items-center gap-0.5"><MessageSquare class="size-3" />{s.commentCount}</span>{/if}
                {#if s.due}<span class="tabular-nums">{day(s.due)}</span>{/if}
                {#if s.assigneeName}<span class="max-w-20 truncate" title={s.assignee}>{firstName(s.assigneeName)}</span>{/if}
                <span class="font-mono text-[11.5px]">{s.id}</span>
              </div>
            </article>
          {/if}
        {/snippet}
      </VirtualList>
    </section>
  {/each}
</div>
