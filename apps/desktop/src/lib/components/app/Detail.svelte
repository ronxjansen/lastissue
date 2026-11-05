<!-- The issue detail pane (GUI-SPEC §5): inline title and body editing
     (saved on a debounce through Rust), property pickers, relations,
     activity and the composer. Banners cover a change on disk under an
     unsaved draft, and a draft left over from a crash. -->
<script lang="ts">
  import type { PickerKind, Relation } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { PRIORITY_LABEL, STATE_LABEL, dateTime, displayName } from "$lib/format";
  import { Button } from "$lib/components/ui/button";
  import Link from "@lucide/svelte/icons/link";
  import X from "@lucide/svelte/icons/x";
  import Activity from "./Activity.svelte";
  import MarkdownEditor from "./MarkdownEditor.svelte";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";

  const doc = $derived(app.doc!);
  const s = $derived(doc.summary);

  let title = $state("");
  let titleEl: HTMLTextAreaElement | undefined = $state();
  let pane: HTMLElement | undefined = $state();
  let shownReset = -1;
  let shownId = "";

  // Take the doc's text when it says so (load, adopted disk change, use
  // theirs); otherwise keep what the user is typing.
  $effect(() => {
    if (doc.id !== shownId || doc.reset !== shownReset) {
      shownId = doc.id;
      shownReset = doc.reset;
      title = doc.title?.value ?? s?.title ?? "";
    }
  });

  $effect(() => {
    if (app.renameTick && titleEl) {
      titleEl.focus();
      titleEl.select();
    }
  });

  $effect(() => {
    if (app.focusDetailTick) pane?.focus();
  });

  $effect(() => {
    // Autosize the title.
    void title;
    if (titleEl) {
      titleEl.style.height = "auto";
      titleEl.style.height = `${titleEl.scrollHeight}px`;
    }
  });

  function pick(picker: PickerKind) {
    app.focus = "detail";
    app.overlay = { kind: "picker", picker, ids: [doc.id] };
  }

  function rel(r: Relation) {
    if (r.id) app.open(r.id);
  }

  const conflict = $derived((doc.title?.conflict ?? null) !== null || (doc.body?.conflict ?? null) !== null);
  const prop = "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-accent";
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section
  bind:this={pane}
  tabindex="-1"
  onfocusin={() => (app.focus = "detail")}
  onmousedown={() => (app.focus = "detail")}
  class="flex h-full w-[min(46%,720px)] min-w-[380px] shrink-0 flex-col border-l bg-background outline-none"
  aria-label="Issue detail"
>
  <header data-tauri-drag-region class="flex h-10 shrink-0 items-center gap-2 border-b px-4 text-xs text-muted-foreground">
    <span class="font-mono">{doc.id}</span>
    {#if s?.projectSlug}<span>· <span class="capitalize">{s.projectSlug.replaceAll("-", " ")}</span></span>{/if}
    <span class="flex-1"></span>
    <button class="rounded p-1 hover:bg-accent hover:text-foreground" title="Copy link (⌘⇧C)" onclick={() => app.run("copyIssueLink")}><Link class="size-3.5" /></button>
    <button class="rounded p-1 hover:bg-accent hover:text-foreground" title="Close (esc)" onclick={() => app.closeDoc()}><X class="size-3.5" /></button>
  </header>

  <div class="flex-1 overflow-y-auto">
    {#if doc.error}
      <p class="m-4 text-sm text-destructive">{doc.error}</p>
    {:else if doc.loading}
      <p class="m-4 text-sm text-muted-foreground">Loading…</p>
    {:else}
      <div class="flex flex-col gap-3 p-5">
        {#if conflict}
          <div class="flex items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs">
            <span class="flex-1">This issue changed on disk while you were editing.</span>
            <Button size="sm" variant="outline" class="h-6 text-xs" onclick={() => app.docAction("useTheirs")}>Reload theirs</Button>
            <Button size="sm" class="h-6 text-xs" onclick={() => app.docAction("keepMine")}>Overwrite with mine</Button>
          </div>
        {/if}
        {#if doc.leftover !== null}
          <div class="flex items-center gap-2 rounded-md border bg-muted px-3 py-2 text-xs">
            <span class="flex-1">An unsaved description draft from an earlier session was found.</span>
            <Button size="sm" variant="ghost" class="h-6 text-xs" onclick={() => app.docAction("discardLeftover")}>Discard</Button>
            <Button size="sm" class="h-6 text-xs" onclick={() => app.docAction("restoreLeftover")}>Restore</Button>
          </div>
        {/if}
        {#if doc.saveError}<p class="text-xs text-destructive">Not saved: {doc.saveError}</p>{/if}

        <textarea
          bind:this={titleEl}
          bind:value={title}
          rows="1"
          oninput={() => app.editField("title", title)}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === "Escape") {
              e.preventDefault();
              titleEl?.blur();
              pane?.focus();
            }
          }}
          class="w-full resize-none overflow-hidden bg-transparent text-xl font-semibold outline-none"
          placeholder="Issue title"
        ></textarea>

        {#key doc.id}
        <MarkdownEditor
          value={doc.body?.value ?? ""}
          reset={doc.reset}
          editable={app.editingBody}
          placeholder={app.editingBody ? "Add a description…" : "No description — press e to write one"}
          onclick={() => (app.editingBody = true)}
          onchange={(md) => app.editField("body", md)}
          onescape={() => {
            app.editingBody = false;
            pane?.focus();
          }}
          onsubmit={() => {
            app.editingBody = false;
            app.flushSave();
            pane?.focus();
          }}
          class="min-h-12 text-sm {app.editingBody ? 'rounded-md ring-1 ring-border' : ''}"
        />
        {/key}
        {#if doc.title?.dirty || doc.body?.dirty}<p class="text-xs text-muted-foreground">Saving…</p>{/if}

        {#if s}
          <div class="grid grid-cols-[96px_1fr] items-center gap-x-2 gap-y-0.5 border-y py-3 text-[13px]">
            <span class="text-xs text-muted-foreground">State</span>
            <button class={prop} onclick={() => pick("state")}><StateIcon state={s.state} />{STATE_LABEL[s.state]}</button>
            <span class="text-xs text-muted-foreground">Priority</span>
            <button class={prop} onclick={() => pick("priority")}><PriorityIcon priority={s.priority} />{PRIORITY_LABEL[s.priority ?? "none"]}</button>
            <span class="text-xs text-muted-foreground">Assignee</span>
            <button class={prop} onclick={() => pick("assignee")} title={s.assignee ?? ""}>{s.assigneeName ?? "Unassigned"}</button>
            <span class="text-xs text-muted-foreground">Labels</span>
            <button class="{prop} flex-wrap" onclick={() => pick("labels")}>
              {#each s.labels as l (l)}<span class="rounded-full border px-2 text-[11px]">{l}</span>{:else}<span class="text-muted-foreground">Add labels</span>{/each}
            </button>
            <span class="text-xs text-muted-foreground">Due</span>
            <button class={prop} onclick={() => pick("due")}>{s.due ? new Date(s.due).toLocaleDateString() : "No due date"}</button>
            <span class="text-xs text-muted-foreground">Project</span>
            <button class="{prop} capitalize" onclick={() => pick("project")}>{s.projectSlug?.replaceAll("-", " ") ?? "No project"}</button>
            <span class="text-xs text-muted-foreground">Author</span>
            <span class="px-2 text-[13px]" title={s.author}>{s.authorName}{s.agent ? " (agent)" : ""}{s.created ? ` · ${dateTime(s.created)}` : ""}</span>
            {#if doc.watchers.length}
              <span class="text-xs text-muted-foreground">Watching</span>
              <span class="truncate px-2 text-[13px]">{doc.watchers.map(displayName).join(", ")}</span>
            {/if}
            {#each [["Parent", doc.parent ? [doc.parent] : []], ["Blocked by", doc.blockedBy], ["Related", doc.related]] as const as [label, list] (label)}
              {#if list.length}
                <span class="text-xs text-muted-foreground">{label}</span>
                <div class="flex flex-col">
                  {#each list as r (r.messageId)}
                    <button class="{prop} disabled:opacity-60" disabled={!r.id} onclick={() => rel(r)}>
                      {#if r.state}<StateIcon state={r.state} />{/if}
                      <span class="font-mono text-xs text-muted-foreground">{r.id ?? "?"}</span>
                      <span class="truncate">{r.title ?? r.messageId}</span>
                    </button>
                  {/each}
                </div>
              {/if}
            {/each}
          </div>
        {/if}

        <h2 class="mt-2 text-xs font-medium text-muted-foreground">Activity</h2>
        <Activity items={doc.activity} issueId={doc.id} />
      </div>
    {/if}
  </div>
</section>
