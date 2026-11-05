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
  import Palette from "$lib/components/app/Palette.svelte";
  import Picker from "$lib/components/app/Picker.svelte";
  import PromptDialog from "$lib/components/app/PromptDialog.svelte";
  import SearchBox from "$lib/components/app/SearchBox.svelte";
  import Sidebar from "$lib/components/app/Sidebar.svelte";
  import StatusBar from "$lib/components/app/StatusBar.svelte";
  import Toolbar from "$lib/components/app/Toolbar.svelte";

  const resolver = new Resolver(app.keymap);
  $effect(() => {
    resolver.keymap = app.keymap;
  });

  let ready = $state(false);
  onMount(() => {
    app.init().then(() => (ready = true), (e) => console.error(e));
  });

  function keydown(e: KeyboardEvent) {
    if (e.isComposing || e.defaultPrevented) return;
    const key = stroke(e);
    if (!key) return;
    const hit = resolver.feed(key, app.contexts());
    if (!hit) return;
    e.preventDefault();
    e.stopPropagation();
    if (hit !== "pending") app.run(hit);
  }

  // Leaving the window (or quitting) must not lose a pending save.
  function blur() {
    app.flushSave();
  }
</script>

<svelte:window onkeydown={keydown} onblur={blur} onbeforeunload={blur} />
<ModeWatcher />
<Toaster position="bottom-right" />

<div class="flex h-screen flex-col overflow-hidden bg-background text-foreground select-none">
  <div class="flex min-h-0 flex-1">
    <Sidebar />
    <main class="@container flex min-w-0 flex-1 flex-col overflow-hidden">
      <Toolbar />
      <div class="min-h-0 flex-1" role="grid" tabindex="-1" onfocusin={() => (app.focus = "issues")}>
        {#if !ready}
          <div class="grid h-full place-items-center text-sm text-muted-foreground">Loading…</div>
        {:else if app.meta.current === null}
          <div class="grid h-full place-items-center">
            <div class="max-w-sm text-center text-sm text-muted-foreground">
              <p class="mb-3">{app.meta.notice ?? "No issues root open."}</p>
              <button class="rounded-md border px-3 py-1.5 text-foreground hover:bg-accent" onclick={() => app.run("addRepo")}>Open a folder…</button>
            </div>
          </div>
        {:else if app.visible.view.mode === "board"}
          <Board />
        {:else}
          <IssueList />
        {/if}
      </div>
    </main>
    {#if app.doc}
      <Detail />
    {/if}
  </div>
  <StatusBar />
</div>

{#if app.overlay?.kind === "picker"}
  {#key app.overlay}<Picker picker={app.overlay.picker} ids={app.overlay.ids} />{/key}
{:else if app.overlay?.kind === "palette"}
  {#key app.overlay}<Palette mode={app.overlay.mode} />{/key}
{:else if app.overlay?.kind === "search"}
  <SearchBox />
{:else if app.overlay?.kind === "create"}
  <CreateDialog />
{:else if app.overlay?.kind === "confirm"}
  <ConfirmDialog {...app.overlay} />
{:else if app.overlay?.kind === "prompt"}
  {#key app.overlay}<PromptDialog {...app.overlay} />{/key}
{/if}
