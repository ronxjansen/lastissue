<!-- Kanban (GUI-SPEC §5.1): a column per state (or priority band). Dropping
     a card on another column writes exactly what `issue update --state`
     would; there is no manual order, so dropping in its own column is a
     no-op. ⇧h / ⇧l do the same from the keyboard. -->
<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { day, groupKeyId } from "$lib/format";
  import MessageSquare from "@lucide/svelte/icons/message-square";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";
  import VirtualList from "./VirtualList.svelte";

  let over = $state<string | null>(null);

  function cursorIndex(items: string[]): number | null {
    const id = app.cursor;
    if (!id) return null;
    const i = items.findIndex((k) => app.issues.get(k)?.id === id);
    return i >= 0 ? i : null;
  }
</script>

<div class="flex h-full gap-3 overflow-x-auto p-3">
  {#each app.visible.groups as g (groupKeyId(g.key))}
    {@const gid = groupKeyId(g.key)}
    <section
      class="flex h-full w-72 shrink-0 flex-col rounded-lg bg-muted/40 {over === gid ? 'ring-2 ring-primary/50' : ''}"
      aria-label={g.label}
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
      <header class="flex items-center gap-2 px-3 py-2 text-xs font-medium">
        {#if g.key.kind === "state"}<StateIcon state={g.key.value} />{:else}<PriorityIcon priority={g.key.value} />{/if}
        {g.label}
        <span class="text-muted-foreground tabular-nums">{g.items.length}</span>
      </header>
      <VirtualList items={g.items} rowHeight={84} scrollTo={cursorIndex(g.items)} class="flex-1 px-2" overscan={4}>
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
              onclick={(e) => {
                app.focus = "issues";
                if (e.metaKey) {
                  app.cursor = s.id;
                  app.run("toggleSelect");
                } else app.open(s.id);
              }}
              class="mb-2 flex h-[76px] cursor-default flex-col gap-1 rounded-md border bg-card p-2.5 text-[13px] shadow-xs
                {app.doc?.id === s.id ? 'border-primary/60' : ''}
                {app.selection.includes(s.id) ? 'bg-primary/8' : ''}
                {app.cursor === s.id && app.focus === 'issues' ? 'ring-2 ring-primary/40' : ''}"
            >
              <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
                <span class="font-mono">{s.id}</span>
                <span class="flex-1"></span>
                {#if s.assigneeName}<span class="grid size-4 place-items-center rounded-full bg-muted text-[9px] text-foreground" title={s.assignee}>{s.assigneeName.slice(0, 1).toUpperCase()}</span>{/if}
              </div>
              <div class="line-clamp-1 font-medium">{s.title}</div>
              <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
                {#if g.key.kind === "state"}<PriorityIcon priority={s.priority} size={12} />{:else}<StateIcon state={s.state} size={12} />{/if}
                {#each s.labels.slice(0, 2) as l (l)}<span class="rounded-full border px-1.5 text-[10px]">{l}</span>{/each}
                <span class="flex-1"></span>
                {#if s.commentCount}<span class="flex items-center gap-0.5"><MessageSquare class="size-3" />{s.commentCount}</span>{/if}
                {#if s.due}<span>{day(s.due)}</span>{/if}
              </div>
            </article>
          {/if}
        {/snippet}
      </VirtualList>
    </section>
  {/each}
</div>
