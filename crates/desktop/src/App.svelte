<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { Resolver, stroke } from "$lib/keymap";
  import { Toaster } from "$lib/components/ui/sonner";
  import { ModeWatcher } from "mode-watcher";
  import { onMount } from "svelte";
  import Board from "$lib/components/app/Board.svelte";
  import ConfirmDialog from "$lib/components/app/ConfirmDialog.svelte";
  import CreateDialog from "$lib/components/app/CreateDialog.svelte";
  import Detail from "$lib/components/app/Detail.svelte";
  import IssueList from "$lib/components/app/IssueList.svelte";
  import IssueMenu from "$lib/components/app/IssueMenu.svelte";
  import Palette from "$lib/components/app/Palette.svelte";
  import Picker from "$lib/components/app/Picker.svelte";
  import PromptDialog from "$lib/components/app/PromptDialog.svelte";
  import SearchBox from "$lib/components/app/SearchBox.svelte";
  import SelectionBar from "$lib/components/app/SelectionBar.svelte";
  import Sidebar from "$lib/components/app/Sidebar.svelte";
  import StatusBar from "$lib/components/app/StatusBar.svelte";
  import Toolbar from "$lib/components/app/Toolbar.svelte";

  const resolver = new Resolver(app.keymap);

  let ready = $state(false);
  let failed = $state<string | null>(null);
  onMount(() => {
    app.init().then(() => (ready = true), (e) => (failed = String(e)));
  });

  function keydown(e: KeyboardEvent) {
    if (e.isComposing || e.defaultPrevented) return;
    const key = stroke(e);
    if (!key) return;
    resolver.keymap = app.keymap;
    const hit = resolver.feed(key, app.contexts());
    if (!hit) return;
    e.preventDefault();
    e.stopPropagation();
    if (hit !== "pending") app.run(hit);
  }
</script>

<!-- Leaving the window (or quitting) must not lose a pending save. -->
<svelte:window onkeydown={keydown} onblur={() => app.flushSave()} onbeforeunload={() => app.flushSave()} />
<ModeWatcher />
<Toaster position="bottom-right" />

<div class="flex h-screen flex-col overflow-hidden select-none">
  <div class="flex min-h-0 flex-1">
    <Sidebar />
    <main class="@container flex min-w-0 flex-1 flex-col overflow-hidden">
      <Toolbar />
      <div class="relative min-h-0 flex-1" role="grid" tabindex="-1" onfocusin={() => (app.focus = "issues")}>
        {#if !ready}
          <div class="grid h-full place-items-center text-sm {failed ? 'text-danger' : 'text-muted-foreground'}">{failed ?? "Loading…"}</div>
        {:else if app.meta.current === null}
          <div class="grid h-full place-items-center">
            <div class="max-w-sm text-center text-sm text-muted-foreground">
              {#if app.meta.notice}
                <p class="mb-3 text-danger">{app.meta.notice}</p>
              {:else}
                <p class="mb-1 text-base text-foreground">No issues folder open</p>
                <p class="mb-4">Open a repo to track its issues next to the code. lastissue creates an <code>issues/</code> folder if it doesn't have one yet.</p>
              {/if}
              <div class="flex justify-center gap-2">
                <button class="rounded-md border bg-accent px-3 py-1.5 text-foreground hover:bg-accent/80" onclick={() => app.run("addRepo")}>Open a folder…</button>
                <button class="rounded-md border px-3 py-1.5 text-foreground hover:bg-accent" onclick={() => app.run("useGlobalRoot")}>Use a personal inbox</button>
              </div>
            </div>
          </div>
        {:else}
          <IssueMenu>
            {#if app.visible.view.mode === "board"}<Board />{:else}<IssueList />{/if}
          </IssueMenu>
          {#if app.selection.size > 1}
            <div class="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center px-4">
              <SelectionBar />
            </div>
          {/if}
        {/if}
      </div>
    </main>
    {#if app.doc}
      <Detail />
    {/if}
  </div>
  <StatusBar />
</div>

{#key app.overlay}
  {#if app.overlay?.kind === "picker"}
    <Picker picker={app.overlay.picker} ids={app.overlay.ids} />
  {:else if app.overlay?.kind === "palette"}
    <Palette mode={app.overlay.mode} />
  {:else if app.overlay?.kind === "search"}
    <SearchBox />
  {:else if app.overlay?.kind === "create"}
    <CreateDialog />
  {:else if app.overlay?.kind === "confirm"}
    <ConfirmDialog {...app.overlay} />
  {:else if app.overlay?.kind === "prompt"}
    <PromptDialog {...app.overlay} />
  {/if}
{/key}
