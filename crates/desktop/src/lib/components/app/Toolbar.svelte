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

  const seg = "h-6 min-w-7 rounded-[5px] text-ink-3 data-[state=on]:bg-sheet data-[state=on]:text-ink data-[state=on]:shadow-[0_1px_2px_oklch(0_0_0/0.08)]";

  const title = $derived.by(() => {
    const s = app.view.scope;
    if (s.kind === "mine") return "My issues";
    if (s.kind === "noProject") return "No project";
    if (s.kind === "project") return app.currentProject?.name ?? "Project";
    return "All issues";
  });
</script>

<header data-tauri-drag-region class="flex h-11 shrink-0 items-center gap-2 border-b border-rule pr-3 pl-6">
  <h1 data-tauri-drag-region class="max-w-48 shrink-0 truncate text-[15px] font-bold text-ink {app.view.scope.kind === 'project' ? 'capitalize' : ''}">{title}</h1>
  <span data-tauri-drag-region class="mr-2 hidden shrink-0 font-mono text-[12px] text-ink-3 @sm:inline">{app.visible.total}</span>
  <div class="hidden shrink-0 @sm:block">
  <ToggleGroup.Root
    type="single"
    size="sm"
    variant="default"
    class="rounded-md bg-wash p-0.5"
    value={app.view.mode}
    onValueChange={(v) => v && app.setView({ mode: v as "list" | "board" })}
  >
    <ToggleGroup.Item value="list" aria-label="List (g l)" class={seg}><List class="size-3.5" /></ToggleGroup.Item>
    <ToggleGroup.Item value="board" aria-label="Board (g b)" class={seg}><Columns3 class="size-3.5" /></ToggleGroup.Item>
  </ToggleGroup.Root>
  </div>

  <div class="relative ml-1 hidden min-w-16 max-w-md flex-1 items-center @xs:flex">
    <ListFilter class="pointer-events-none absolute left-2.5 size-3.5 text-ink-3" />
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
      placeholder="Filter"
      spellcheck="false"
      class="h-8 w-full rounded-md border border-transparent bg-wash pr-8 pl-8 font-mono text-[12.5px] text-ink transition-colors outline-none placeholder:font-sans placeholder:text-[13px] placeholder:text-ink-3 hover:bg-wash-strong focus:border-rule focus:bg-sheet focus-visible:outline-none {app.visible.error ? 'border-danger! bg-danger/6' : ''}"
      title={app.visible.error ?? "Filter tokens: state:todo  p:high  label:bug  @me  due:<7d, or any text"}
    />
    {#if app.visible.scanning}
      <LoaderCircle class="absolute right-2.5 size-3.5 animate-spin text-ink-3" />
    {/if}
  </div>
  {#if app.visible.error}<span class="truncate text-[12.5px] text-danger">{app.visible.error}</span>{/if}

  <div class="ml-auto flex shrink-0 items-center gap-1">
    <div class="hidden @sm:block">
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <Button {...props} variant="ghost" size="sm" class="h-8 text-[13px] text-ink-2">
            <SlidersHorizontal class="size-3.5" /><span class="hidden @3xl:inline">{SORT_LABEL[app.view.sort]}<span class="text-ink-3">&nbsp;· by {app.view.grouping}</span></span>
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
    </div>
    <Button size="sm" class="h-8 gap-1.5 pr-2 text-[13px]" onclick={() => app.run("newIssue")}><Plus class="size-3.5" /><span class="hidden @sm:inline">New issue</span> <kbd class="hidden rounded-[4px] bg-sheet/15 px-1 text-[11px] leading-4 @3xl:inline">c</kbd></Button>
  </div>
</header>
