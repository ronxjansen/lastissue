<!-- The issue detail pane (GUI-SPEC §5): inline title and body editing
     (saved on a debounce through Rust), property pickers, relations,
     activity and the composer. Banners cover a change on disk under an
     unsaved draft, and a draft left over from a crash. -->
<script lang="ts">
  import type { PickerKind, Relation } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { PRIORITY_LABEL, STATE_LABEL, dateTime } from "$lib/format";
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
  const hasLinks = $derived(!!doc.parent || doc.blockedBy.length > 0 || doc.related.length > 0 || doc.watchers.length > 0);
  // Properties read as a byline: quiet text until hovered, never boxed.
  const hv = "-mx-1.5 inline-flex h-6 max-w-full items-center gap-1.5 rounded px-1.5 text-ink-2 transition-colors duration-100 hover:bg-wash hover:text-ink";
  const unset = "text-ink-3";
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section
  bind:this={pane}
  tabindex="-1"
  onfocusin={() => (app.focus = "detail")}
  onmousedown={() => (app.focus = "detail")}
  class="flex h-full w-[min(50%,760px)] min-w-[340px] shrink-0 flex-col border-l border-rule bg-sheet outline-none"
  aria-label="Issue detail"
>
  <header data-tauri-drag-region class="flex h-11 shrink-0 items-center gap-2 pr-3 pl-6 font-mono text-[12px] text-ink-3">
    <span data-tauri-drag-region class="truncate">{s?.projectName ?? "issues"}<span class="px-1 opacity-60">/</span><span class="text-ink-2">{doc.id}</span></span>
    <span data-tauri-drag-region class="flex-1"></span>
    {#if doc.title?.dirty || doc.body?.dirty}<span class="font-sans text-[12px]">Saving…</span>{/if}
    <button class="rounded-md p-1.5 transition-colors hover:bg-wash hover:text-ink" title="Copy link (⌘⇧C)" aria-label="Copy link" onclick={() => app.run("copyIssueLink")}><Link class="size-3.5" /></button>
    <button class="rounded-md p-1.5 transition-colors hover:bg-wash hover:text-ink" title="Close (esc)" aria-label="Close" onclick={() => app.closeDoc()}><X class="size-3.5" /></button>
  </header>

  <div class="flex-1 overflow-y-auto">
    {#if doc.error}
      <p class="mx-10 my-8 text-[14px] text-danger">{doc.error}</p>
    {:else if doc.loading}
      <p class="mx-10 my-8 text-[14px] text-ink-3">Loading…</p>
    {:else}
      <article class="@container mx-auto max-w-[70ch] px-10 pt-6 pb-20">
        {#if conflict}
          <div class="mb-6 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg bg-warn/12 px-4 py-3 text-[13.5px] text-ink">
            <span class="flex-1">This issue changed on disk while you were editing.</span>
            <Button size="sm" variant="ghost" class="h-7" onclick={() => app.docAction("useTheirs")}>Reload theirs</Button>
            <Button size="sm" class="h-7" onclick={() => app.docAction("keepMine")}>Overwrite with mine</Button>
          </div>
        {/if}
        {#if doc.leftover !== null}
          <div class="mb-6 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg bg-wash px-4 py-3 text-[13.5px] text-ink">
            <span class="flex-1">An unsaved description draft from an earlier session was found.</span>
            <Button size="sm" variant="ghost" class="h-7" onclick={() => app.docAction("discardLeftover")}>Discard</Button>
            <Button size="sm" class="h-7" onclick={() => app.docAction("restoreLeftover")}>Restore</Button>
          </div>
        {/if}
        {#if doc.saveError}<p class="mb-4 text-[13px] text-danger">Not saved: {doc.saveError}</p>{/if}

        {#if s}
          <!-- The file's own headers, as plain text. Values are editable in place. -->
          <dl class="grid grid-cols-[5.25rem_minmax(0,1fr)] items-center gap-y-px @lg:grid-cols-[4.75rem_minmax(0,1fr)_5.25rem_minmax(0,1fr)] font-mono text-[12px] leading-6 text-ink-3">
            <dt>Issue:</dt>
            <dd class="font-bold text-ink">{doc.id}</dd>
            <dt>State:</dt>
            <dd><button class={hv} onclick={() => pick("state")}><StateIcon state={s.state} size={12} />{STATE_LABEL[s.state]}</button></dd>

            <dt>From:</dt>
            <dd class="truncate pr-3 text-ink-2" title={s.author}>{s.authorName}{s.agent ? " (agent)" : ""}</dd>
            <dt>Priority:</dt>
            <dd><button class={hv} onclick={() => pick("priority")}>{#if s.priority}<PriorityIcon priority={s.priority} size={12} />{PRIORITY_LABEL[s.priority]}{:else}<span class={unset}>none</span>{/if}</button></dd>

            <dt>To:</dt>
            <dd class="min-w-0 pr-3"><button class={hv} onclick={() => pick("assignee")} title={s.assignee ?? ""}><span class="truncate {s.assigneeName ? '' : unset}">{s.assigneeName ?? "unassigned"}</span></button></dd>
            <dt>Keywords:</dt>
            <dd class="min-w-0"><button class={hv} onclick={() => pick("labels")}><span class="truncate {s.labels.length ? '' : unset}">{s.labels.length ? s.labels.join(", ") : "none"}</span></button></dd>

            <dt>Date:</dt>
            <dd class="truncate pr-3 text-ink-2">{s.created ? dateTime(s.created) : "unknown"}</dd>
            <dt>Due:</dt>
            <dd><button class={hv} onclick={() => pick("due")}><span class={s.due ? "" : unset}>{s.due ? new Date(s.due).toLocaleDateString() : "none"}</span></button></dd>

            <dt>Project:</dt>
            <dd class="min-w-0 pr-3 @lg:col-span-3"><button class={hv} onclick={() => pick("project")}><span class="truncate {s.projectName ? '' : unset}">{s.projectName ?? "none"}</span></button></dd>
            <dt>File:</dt>
            <dd class="break-all @lg:col-span-3" title={s.key}>{s.key}</dd>
          </dl>
        {/if}

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
          class="mt-6 w-full resize-none overflow-hidden bg-transparent text-[26px] leading-[1.25] font-bold tracking-[-0.018em] text-balance text-ink outline-none placeholder:text-ink-3"
          placeholder="Issue title"
        ></textarea>

        {#key doc.id}
        <MarkdownEditor
          value={doc.body?.value ?? ""}
          reset={doc.reset}
          editable={app.editingBody}
          placeholder={app.editingBody ? "Add a description…" : "No description yet. Press e to write one."}
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
          class="mt-5 min-h-12 text-[15.5px] {app.editingBody ? '-mx-4 rounded-lg bg-paper px-4 pb-4 ring-1 ring-rule' : ''}"
        />
        {/key}

        {#if hasLinks}
          <h2 class="mt-14 mb-3 flex items-baseline gap-2 border-t border-rule-soft pt-6 text-[16px] font-bold text-ink"><span class="font-mono font-normal text-ink-3">§</span>Links</h2>
          <dl class="grid grid-cols-[6.5rem_1fr] items-baseline gap-y-1 text-[14px]">
            {#each [["Parent", doc.parent ? [doc.parent] : []], ["Blocked by", doc.blockedBy], ["Related", doc.related]] as const as [label, list] (label)}
              {#if list.length}
                <dt class="pt-1 text-[13px] text-ink-3">{label}</dt>
                <dd class="flex min-w-0 flex-col">
                  {#each list as r (r.messageId)}
                    <button class="flex h-7 min-w-0 items-center gap-2 rounded-md px-2 -mx-2 text-left transition-colors hover:bg-wash disabled:opacity-60" disabled={!r.id} onclick={() => rel(r)}>
                      {#if r.state}<StateIcon state={r.state} />{/if}
                      <span class="truncate text-ink">{r.title ?? r.messageId}</span>
                      <span class="ml-auto shrink-0 font-mono text-[12px] text-ink-3">{r.id ?? "?"}</span>
                    </button>
                  {/each}
                </dd>
              {/if}
            {/each}
            {#if doc.watchers.length}
              <dt class="pt-1 text-[13px] text-ink-3">Watching</dt>
              <dd class="truncate pt-1 text-ink-2">{doc.watchers.join(", ")}</dd>
            {/if}
          </dl>
        {/if}

        <h2 class="mt-14 mb-4 flex items-baseline gap-2 border-t border-rule-soft pt-6 text-[16px] font-bold text-ink"><span class="font-mono font-normal text-ink-3">§</span>Activity</h2>
        {#key doc.id}<Activity items={doc.activity} />{/key}
      </article>
    {/if}
  </div>
</section>
