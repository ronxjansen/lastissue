<script lang="ts">
  import type { Scope } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { pretty } from "$lib/keymap";
  import * as ContextMenu from "$lib/components/ui/context-menu";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
  import ChevronsUpDown from "@lucide/svelte/icons/chevrons-up-down";
  import CircleDot from "@lucide/svelte/icons/circle-dot";
  import FolderClosed from "@lucide/svelte/icons/folder-closed";
  import Inbox from "@lucide/svelte/icons/inbox";
  import Layers from "@lucide/svelte/icons/layers";
  import Plus from "@lucide/svelte/icons/plus";
  import Settings from "@lucide/svelte/icons/settings";
  import UserRound from "@lucide/svelte/icons/user-round";

  const repo = $derived(app.meta.current !== null ? app.meta.repos[app.meta.current] : null);

  // Open issues per project (and outside any), and assigned to me.
  const counts = $derived.by(() => {
    const per = new Map<string, number>();
    let none = 0;
    let all = 0;
    let mine = 0;
    const me = app.meta.me?.toLowerCase();
    const email = (a: string) => (a.match(/<([^>]+)>/)?.[1] ?? a).trim().toLowerCase();
    const meEmail = me ? email(me) : null;
    for (const s of app.issues.values()) {
      if (["done", "cancelled", "duplicate"].includes(s.state)) continue;
      all++;
      if (s.project) per.set(s.project, (per.get(s.project) ?? 0) + 1);
      else none++;
      if (meEmail && s.assignee && email(s.assignee) === meEmail) mine++;
    }
    return { per, none, all, mine };
  });

  function active(s: Scope): boolean {
    const v = app.view.scope;
    return v.kind === s.kind && (s.kind !== "project" || (v.kind === "project" && v.project === s.project));
  }

  function go(scope: Scope) {
    app.setView({ scope });
    app.focus = "issues";
  }

  const item = "flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[14px] text-ink-2 transition-colors duration-100 hover:bg-wash hover:text-ink";
  const on = "bg-sidebar-accent font-bold text-ink";
  const count = "font-mono text-[12px] text-ink-3";
</script>

<aside class="flex h-full w-56 shrink-0 flex-col max-[1000px]:w-44 bg-sidebar text-sidebar-foreground">
  <div data-tauri-drag-region class="h-11 shrink-0"></div>
  <div class="px-2 pb-2">
    <DropdownMenu.Root>
      <DropdownMenu.Trigger class="{item} h-9 font-bold text-ink">
        <span class="grid size-6 -rotate-3 place-items-center rounded-[5px] bg-mark font-mono text-[12px] font-bold text-mark-ink shadow-[0_1px_0_oklch(0_0_0/0.08)]">
          {(repo?.label ?? "?").slice(0, 1).toUpperCase()}
        </span>
        <span class="flex-1 truncate">{repo?.label ?? "No repo"}</span>
        <ChevronsUpDown class="size-3.5 opacity-60" />
      </DropdownMenu.Trigger>
      <DropdownMenu.Content class="w-64" align="start">
        <DropdownMenu.Label>Repos</DropdownMenu.Label>
        {#each app.meta.repos as r, i (r.root)}
          <DropdownMenu.Item onclick={() => app.switchRepo(i)} disabled={!r.exists}>
            <span class="flex-1 truncate" title={r.root}>{r.label}{r.global ? " (global)" : ""}</span>
            {#if i < 9}<DropdownMenu.Shortcut>{pretty(`cmd-${i + 1}`)}</DropdownMenu.Shortcut>{/if}
          </DropdownMenu.Item>
        {/each}
        <DropdownMenu.Separator />
        <DropdownMenu.Item onclick={() => app.run("addRepo")}><Plus class="size-4" /> Add repo…</DropdownMenu.Item>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  </div>

  <nav class="flex-1 overflow-y-auto px-2 pt-1">
    <button class="{item} {active({ kind: 'mine' }) ? on : ''}" onclick={() => go({ kind: "mine" })}>
      <UserRound class="size-4 shrink-0 opacity-75" /><span class="flex-1">My issues</span>
      <span class={count}>{counts.mine || ""}</span>
    </button>
    <button class="{item} {active({ kind: 'all' }) ? on : ''}" onclick={() => go({ kind: "all" })}>
      <Layers class="size-4 shrink-0 opacity-75" /><span class="flex-1">All issues</span>
      <span class={count}>{counts.all || ""}</span>
    </button>

    <div class="mt-6 mb-1 flex items-center pr-1 pl-2.5 text-[12.5px] font-bold text-ink-3">
      <span class="flex-1">Projects</span>
      <button class="rounded-md p-1 transition-colors hover:bg-wash hover:text-ink" title="New project" aria-label="New project" onclick={() => app.run("newProject")}>
        <Plus class="size-3.5" />
      </button>
    </div>
    {#each app.projects as p (p.id)}
      <ContextMenu.Root>
        <ContextMenu.Trigger>
          <button
            class="{item} {active({ kind: 'project', project: p.id }) ? on : ''}"
            onclick={() => go({ kind: "project", project: p.id })}
          >
            <FolderClosed class="size-4 shrink-0 opacity-75" /><span class="flex-1 truncate capitalize">{p.name}</span>
            <span class={count}>{counts.per.get(p.id) || ""}</span>
          </button>
        </ContextMenu.Trigger>
        <ContextMenu.Content>
          <ContextMenu.Item
            onclick={() => {
              go({ kind: "project", project: p.id });
              app.run("renameProject");
            }}>Rename…</ContextMenu.Item
          >
        </ContextMenu.Content>
      </ContextMenu.Root>
    {/each}
    <button class="{item} {active({ kind: 'noProject' }) ? on : ''}" onclick={() => go({ kind: "noProject" })}>
      <Inbox class="size-4 shrink-0 opacity-75" /><span class="flex-1">No project</span>
      <span class={count}>{counts.none || ""}</span>
    </button>
  </nav>

  <div class="flex items-center gap-2 px-4 py-3 text-[12.5px] text-ink-3">
    <CircleDot class="size-3.5 shrink-0 {app.meta.me ? 'text-ok' : 'text-danger'}" />
    <span class="flex-1 truncate" title={app.meta.meError ?? app.meta.me ?? ""}>{app.meta.me ?? "No identity (git config user.email)"}</span>
    <button class="rounded-md p-1 transition-colors hover:bg-wash hover:text-ink" aria-label="Keymap settings" title="Keymap settings (⌘,)" onclick={() => app.run("openSettings")}>
      <Settings class="size-3.5" />
    </button>
  </div>
</aside>
