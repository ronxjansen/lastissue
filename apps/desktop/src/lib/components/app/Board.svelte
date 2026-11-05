<!-- Kanban (GUI-SPEC §5.1): a column per state (or priority band). Dropping
     a card on another column writes exactly what `issue update --state`
     would; there is no manual order, so dropping in its own column is a
     no-op. ⇧h / ⇧l do the same from the keyboard. -->
<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { day, firstName, groupKeyId, groupLabel } from "$lib/format";
  import MessageSquare from "@lucide/svelte/icons/message-square";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";
  import VirtualList from "./VirtualList.svelte";
  import GroupIcon from "./GroupIcon.svelte";

  let over = $state<string | null>(null);
</script>

<div class="flex h-full gap-3 divide-x divide-rule-soft overflow-x-auto px-4 pt-3 pb-3">
  {#each app.visible.groups as g (groupKeyId(g.key))}
    {@const gid = groupKeyId(g.key)}
    <section
      class="flex h-full w-[19rem] shrink-0 flex-col rounded-lg transition-colors duration-150 {over === gid ? 'bg-mark-soft' : ''}"
      aria-label={groupLabel(g.key)}
      ondragover={(e) => {
        e.preventDefault();
        over = gid;
      }}
      ondragleave={() => over === gid && (over = null)}
      ondrop={(e) => {
        e.preventDefault();
        over = null;
        const id = e.dataTransfer?.getData("text/x-lastissue-id");
        if (id) app.dropCard(id, g.key);
      }}
    >
      <header class="flex items-center gap-2 px-3 pt-2 pb-2.5 text-[13px] font-bold text-ink-2">
        <GroupIcon key={g.key} />
        {groupLabel(g.key)}
        <span class="font-mono text-[12px] font-normal text-ink-3">{g.items.length}</span>
      </header>
      <VirtualList items={g.items} rowHeight={90} scrollTo={g.items.findIndex(app.atCursor)} class="flex-1 px-2" overscan={4}>
        {#snippet row(k)}
          {@const s = app.issues.get(k)}
          {#if s}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <article
              draggable="true"
              ondragstart={(e) => {
                e.dataTransfer?.setData("text/x-lastissue-id", s.id);
                if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
              }}
              onclick={(e) => app.clickIssue(s.id, e)}
              class="relative mb-2 flex h-[82px] cursor-default flex-col justify-between rounded-lg border bg-sheet px-3 py-2.5 text-[14px] transition-[border-color,box-shadow] duration-100 hover:border-ink-3/40
                {app.doc?.id === s.id ? 'border-ink-3/60' : 'border-rule'}
                {app.selection.includes(s.id) ? 'bg-link/6' : ''}
                {app.cursor === s.id && app.focus === 'issues' ? 'shadow-[0_0_0_2px_var(--link)]' : ''}"
            >
              <div class="line-clamp-2 leading-[1.35] text-ink">
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
