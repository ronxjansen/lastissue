<script lang="ts">
  import type { Grouping, Sort } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { SORT_LABEL } from "$lib/format";
  import { Button } from "$lib/components/ui/button";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import Columns3 from "@lucide/svelte/icons/columns-3";
  import List from "@lucide/svelte/icons/list";
  import ListFilter from "@lucide/svelte/icons/list-filter";
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";
  import Plus from "@lucide/svelte/icons/plus";
  import SlidersHorizontal from "@lucide/svelte/icons/sliders-horizontal";

  let filter = $state(app.view.filter);
  let timer: ReturnType<typeof setTimeout> | undefined;

  // Follow outside changes (⌘↵ from search, repo switch).
  $effect(() => {
    const f = app.visible.view.filter;
    if (document.activeElement !== input) filter = f;
  });

  function onInput() {
    clearTimeout(timer);
    timer = setTimeout(() => app.setView({ filter }), 120);
  }

  let input: HTMLInputElement | undefined = $state();

  const title = $derived.by(() => {
    const s = app.view.scope;
    if (s.kind === "mine") return "My issues";
    if (s.kind === "noProject") return "No project";
    if (s.kind === "project") return app.projects.find((p) => p.id === s.project)?.name ?? "Project";
    return "All issues";
  });
</script>

<header data-tauri-drag-region class="flex h-10 shrink-0 items-center gap-2 border-b px-3">
  <h1 data-tauri-drag-region class="mr-1 truncate text-[13px] font-medium capitalize">{title}</h1>
  <span class="text-xs text-muted-foreground tabular-nums">{app.visible.total}</span>
  <ToggleGroup.Root
    type="single"
    size="sm"
    variant="outline"
    value={app.view.mode}
    onValueChange={(v) => v && app.setView({ mode: v as "list" | "board" })}
  >
    <ToggleGroup.Item value="list" aria-label="List (g l)"><List class="size-3.5" /></ToggleGroup.Item>
    <ToggleGroup.Item value="board" aria-label="Board (g b)"><Columns3 class="size-3.5" /></ToggleGroup.Item>
  </ToggleGroup.Root>

  <div class="relative ml-2 flex max-w-md flex-1 items-center">
    <ListFilter class="pointer-events-none absolute left-2 size-3.5 text-muted-foreground" />
    <input
      bind:this={input}
      bind:value={filter}
      oninput={onInput}
      onkeydown={(e) => {
        if (e.key === "Escape") input?.blur();
        if (e.key === "Enter") {
          clearTimeout(timer);
          app.setView({ filter });
          input?.blur();
        }
      }}
      placeholder="Filter: state:todo p:high label:bug @me due:<7d text…"
      spellcheck="false"
      class="h-7 w-full rounded-md border bg-transparent pr-7 pl-7 text-xs outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring/40 {app.visible.error ? 'border-destructive' : ''}"
      title={app.visible.error ?? ""}
    />
    {#if app.visible.scanning}
      <LoaderCircle class="absolute right-2 size-3.5 animate-spin text-muted-foreground" />
    {/if}
  </div>
  {#if app.visible.error}<span class="truncate text-xs text-destructive">{app.visible.error}</span>{/if}

  <div class="ml-auto flex items-center gap-1">
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <Button {...props} variant="ghost" size="sm" class="h-7 text-xs">
            <SlidersHorizontal class="size-3.5" /> {SORT_LABEL[app.view.sort]} · by {app.view.grouping}
          </Button>
        {/snippet}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content align="end" class="w-48">
        <DropdownMenu.Label>Sort</DropdownMenu.Label>
        <DropdownMenu.RadioGroup value={app.view.sort} onValueChange={(v) => app.setView({ sort: v as Sort })}>
          {#each Object.entries(SORT_LABEL) as [k, l] (k)}
            <DropdownMenu.RadioItem value={k}>{l}</DropdownMenu.RadioItem>
          {/each}
        </DropdownMenu.RadioGroup>
        <DropdownMenu.Separator />
        <DropdownMenu.Label>Group by</DropdownMenu.Label>
        <DropdownMenu.RadioGroup value={app.view.grouping} onValueChange={(v) => app.setView({ grouping: v as Grouping })}>
          <DropdownMenu.RadioItem value="state">State</DropdownMenu.RadioItem>
          <DropdownMenu.RadioItem value="priority">Priority</DropdownMenu.RadioItem>
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
    <Button size="sm" class="h-7 text-xs" onclick={() => app.run("newIssue")}><Plus class="size-3.5" /> New <span class="opacity-60">C</span></Button>
  </div>
</header>
