<!-- The activity timeline (GUI-SPEC §5): comments and events in file order.
     Events are one compact line each; a run of three or more collapses
     into one expandable line. Comments edit in place and delete with a
     confirm; the composer at the end adds one (⌘↵). -->
<script lang="ts">
  import type { Activity } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { ago, dateTime } from "$lib/format";
  import { Button } from "$lib/components/ui/button";
  import History from "@lucide/svelte/icons/history";
  import Markdown from "./Markdown.svelte";
  import MarkdownEditor from "./MarkdownEditor.svelte";

  let { items, issueId }: { items: Activity[]; issueId: string } = $props();

  const COLLAPSE_RUN = 3;
  type Block =
    | { kind: "comment"; a: Extract<Activity, { kind: "comment" }> }
    | { kind: "events"; list: Extract<Activity, { kind: "event" }>[]; id: number };

  const blocks = $derived.by((): Block[] => {
    const out: Block[] = [];
    for (const a of items) {
      const last = out.at(-1);
      if (a.kind === "event") {
        if (last?.kind === "events") last.list.push(a);
        else out.push({ kind: "events", list: [a], id: a.index });
      } else out.push({ kind: "comment", a });
    }
    return out;
  });

  let expanded = $state<Record<number, boolean>>({});
  let editing = $state<string | null>(null);
  let editText = "";
  let composer = $state("");
  let composerReset = $state(0);
  let posting = $state(false);

  // A different issue: drop per-issue UI state.
  $effect(() => {
    void issueId;
    expanded = {};
    editing = null;
    composer = "";
    composerReset++;
  });

  async function post() {
    if (!composer.trim() || posting) return;
    posting = true;
    try {
      await app.addComment(composer);
      composer = "";
      composerReset++;
    } catch (e) {
      console.error(e);
    } finally {
      posting = false;
    }
  }

  async function saveEdit(mid: string) {
    if (!editText.trim()) {
      app.deleteComment(mid);
      return;
    }
    try {
      await app.editComment(mid, editText);
      editing = null;
    } catch (e) {
      console.error(e);
    }
  }

  function eventSummary(e: Extract<Activity, { kind: "event" }>): string {
    if (e.event === "update") return e.lines.join(" · ") || "updated";
    if (e.event === "move") return e.lines.join(" · ") || "moved";
    if (e.event === "comment-deleted") return "deleted a comment";
    return e.lines.join(" · ") || e.event;
  }
</script>

<div class="flex flex-col gap-3">
  {#each blocks as b (b.kind === "comment" ? b.a.messageId : `e${b.id}`)}
    {#if b.kind === "events"}
      {#if b.list.length >= COLLAPSE_RUN && !expanded[b.id]}
        <button class="flex items-center gap-2 text-left text-xs text-muted-foreground hover:text-foreground" onclick={() => (expanded[b.id] = true)}>
          <History class="size-3.5" /> {b.list.length} updates by {[...new Set(b.list.map((e) => e.who))].join(", ")}
          {#if b.list.at(-1)?.date}<span>· {ago(b.list.at(-1)!.date!)}</span>{/if}
        </button>
      {:else}
        {#each b.list as e (e.index)}
          <div class="flex items-start gap-2 text-xs text-muted-foreground">
            <History class="mt-0.5 size-3.5 shrink-0" />
            <span class="min-w-0 flex-1"><span class="text-foreground">{e.who}</span> {eventSummary(e)}</span>
            {#if e.date}<span class="shrink-0" title={dateTime(e.date)}>{ago(e.date)}</span>{/if}
          </div>
        {/each}
      {/if}
    {:else}
      {@const c = b.a}
      <article class="group rounded-lg border bg-card p-3">
        <header class="mb-1.5 flex items-center gap-2 text-xs">
          <span class="font-medium">{c.who}</span>
          {#if c.replyTo}<span class="text-muted-foreground">replying to #{c.replyTo}</span>{/if}
          {#if c.date}<span class="text-muted-foreground" title={dateTime(c.date)}>{ago(c.date)}</span>{/if}
          {#if c.edited}<span class="text-muted-foreground">(edited)</span>{/if}
          <span class="flex-1"></span>
          <span class="text-muted-foreground">#{c.number}</span>
          {#if editing !== c.messageId}
            <button class="text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-foreground" onclick={() => { editText = c.body; editing = c.messageId; }}>Edit</button>
            <button class="text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-destructive" onclick={() => app.deleteComment(c.messageId)}>Delete</button>
          {/if}
        </header>
        {#if editing === c.messageId}
          <MarkdownEditor
            value={c.body}
            autofocus
            onchange={(md) => (editText = md)}
            onsubmit={() => saveEdit(c.messageId)}
            onescape={() => (editing = null)}
            class="text-sm"
          />
          <div class="mt-2 flex justify-end gap-2">
            <Button size="sm" variant="ghost" onclick={() => (editing = null)}>Cancel</Button>
            <Button size="sm" onclick={() => saveEdit(c.messageId)}>Save <span class="opacity-60">⌘↵</span></Button>
          </div>
        {:else}
          <Markdown source={c.body} class="md text-sm" />
        {/if}
      </article>
    {/if}
  {/each}

  <div class="rounded-lg border bg-card p-3 focus-within:ring-2 focus-within:ring-ring/30">
    <MarkdownEditor
      value=""
      reset={composerReset}
      placeholder="Leave a comment… (⌘↵ to send)"
      onchange={(md) => (composer = md)}
      onsubmit={post}
      onescape={() => (document.activeElement as HTMLElement | null)?.blur()}
      class="min-h-10 text-sm"
    />
    <div class="mt-2 flex justify-end">
      <Button size="sm" disabled={!composer.trim() || posting} onclick={post}>Comment</Button>
    </div>
  </div>
</div>
