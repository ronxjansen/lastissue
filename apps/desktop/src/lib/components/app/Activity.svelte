<!-- The activity timeline (GUI-SPEC §5): comments and events in file order.
     Events are one compact line each; a run of three or more collapses
     into one expandable line. Comments edit in place and delete with a
     confirm; the composer at the end adds one (⌘↵). -->
<script lang="ts">
  import type { Activity } from "$lib/api";
  import { app, report } from "$lib/app.svelte";
  import { ago, dateTime } from "$lib/format";
  import { Button } from "$lib/components/ui/button";
  import Markdown from "./Markdown.svelte";
  import MarkdownEditor from "./MarkdownEditor.svelte";

  let { items }: { items: Activity[] } = $props();

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

  async function post() {
    if (!composer.trim() || posting) return;
    posting = true;
    try {
      await app.addComment(composer);
      composer = "";
      composerReset++;
    } catch (e) {
      report(e);
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
      report(e);
    }
  }

  function eventSummary(e: Extract<Activity, { kind: "event" }>): string {
    if (e.event === "comment-deleted") return "deleted a comment";
    return e.lines.join(" · ") || (e.event === "move" ? "moved" : "updated");
  }
</script>

<!-- One spine runs the whole thread; each mark's shape says what happened. -->
<ol class="relative">
  <span class="absolute top-3 bottom-6 left-[5px] w-px bg-rule" aria-hidden="true"></span>
  {#each blocks as b (b.kind === "comment" ? b.a.messageId : `e${b.id}`)}
    {#if b.kind === "events"}
      {#if b.list.length >= COLLAPSE_RUN && !expanded[b.id]}
        <li class="relative py-1.5 pl-7">
          <span class="absolute top-1/2 left-[2px] h-[7px] w-[7px] -translate-y-1/2 rounded-full border border-ink-3 bg-sheet" aria-hidden="true"></span>
          <button class="text-left text-[13px] text-ink-3 transition-colors hover:text-ink" onclick={() => (expanded[b.id] = true)}>
            {b.list.length} updates by {[...new Set(b.list.map((e) => e.who))].join(", ")}
            {#if b.list.at(-1)?.date}<span> · {ago(b.list.at(-1)!.date!)}</span>{/if}
          </button>
        </li>
      {:else}
        {#each b.list as e (e.index)}
          <li class="relative flex items-baseline gap-3 py-1 pl-7 text-[13px] text-ink-3">
            <span
              class="absolute top-[0.8em] left-[3px] size-[5px] bg-ink-3 {e.event === 'move' ? 'rounded-[1px]' : e.event === 'comment-deleted' ? 'rounded-full bg-transparent ring-1 ring-ink-3' : 'rounded-full'}"
              aria-hidden="true"
            ></span>
            <span class="min-w-0 flex-1"><span class="text-ink-2">{e.who}</span> {eventSummary(e)}</span>
            {#if e.date}<span class="tabular-nums shrink-0 text-[12px]" title={dateTime(e.date)}>{ago(e.date)}</span>{/if}
          </li>
        {/each}
      {/if}
    {:else}
      {@const c = b.a}
      <li class="group relative py-4 pl-7">
        <span class="absolute top-[1.3rem] left-0 size-[11px] rounded-full border-2 border-sheet bg-ink-2" aria-hidden="true"></span>
        <header class="flex items-baseline gap-2 text-[13px]">
          <span class="font-bold text-ink">{c.who}</span>
          {#if c.date}<span class="text-ink-3" title={dateTime(c.date)}>{ago(c.date)}</span>{/if}
          {#if c.replyTo}<span class="text-ink-3">replying to #{c.replyTo}</span>{/if}
          {#if c.edited}<span class="text-ink-3">· edited</span>{/if}
          <span class="flex-1"></span>
          {#if editing !== c.messageId}
            <button class="text-[12.5px] text-ink-3 opacity-0 transition-opacity group-hover:opacity-100 hover:text-ink focus-visible:opacity-100" onclick={() => { editText = c.body; editing = c.messageId; }}>Edit</button>
            <button class="text-[12.5px] text-ink-3 opacity-0 transition-opacity group-hover:opacity-100 hover:text-danger focus-visible:opacity-100" onclick={() => app.deleteComment(c.messageId)}>Delete</button>
          {/if}
          <span class="font-mono text-[12px] text-ink-3">#{c.number}</span>
        </header>
        {#if editing === c.messageId}
          <div class="mt-2 rounded-lg bg-paper px-4 py-3 ring-1 ring-rule">
            <MarkdownEditor
              value={c.body}
              autofocus
              onchange={(md) => (editText = md)}
              onsubmit={() => saveEdit(c.messageId)}
              onescape={() => (editing = null)}
              class="text-[15px]"
            />
          </div>
          <div class="mt-2 flex justify-end gap-2">
            <Button size="sm" variant="ghost" onclick={() => (editing = null)}>Cancel</Button>
            <Button size="sm" onclick={() => saveEdit(c.messageId)}>Save <span class="font-mono text-[11px] opacity-60">⌘↵</span></Button>
          </div>
        {:else}
          <Markdown source={c.body} class="md mt-1.5 text-[15px]" />
        {/if}
      </li>
    {/if}
  {/each}

  <li class="relative pt-4 pl-7">
    <span class="absolute top-[2.15rem] left-0 size-[11px] rounded-full border border-dashed border-ink-3 bg-sheet" aria-hidden="true"></span>
    <div class="rounded-lg border border-rule bg-paper px-4 py-3 transition-colors focus-within:border-ink-3">
      <MarkdownEditor
        value=""
        reset={composerReset}
        placeholder="Write a reply…"
        onchange={(md) => (composer = md)}
        onsubmit={post}
        onescape={() => (document.activeElement as HTMLElement | null)?.blur()}
        class="min-h-12 text-[15px]"
      />
      <div class="mt-2 flex items-center justify-end gap-3">
        <span class="font-mono text-[11px] text-ink-3">⌘↵</span>
        <Button size="sm" disabled={!composer.trim() || posting} onclick={post}>Comment</Button>
      </div>
    </div>
  </li>
</ol>
