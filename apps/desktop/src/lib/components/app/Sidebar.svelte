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

  const item = "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-sidebar-accent";
</script>

<aside class="flex h-full w-56 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground">
  <div data-tauri-drag-region class="h-10 shrink-0"></div>
  <div class="px-2 pb-2">
    <DropdownMenu.Root>
      <DropdownMenu.Trigger class="{item} h-8 font-medium">
        <span class="grid size-5 place-items-center rounded bg-primary text-[11px] font-semibold text-primary-foreground">
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

  <nav class="flex-1 overflow-y-auto px-2 text-[13px]">
    <button class="{item} {active({ kind: 'mine' }) ? 'bg-sidebar-accent font-medium' : ''}" onclick={() => go({ kind: "mine" })}>
      <UserRound class="size-4 opacity-70" /><span class="flex-1">My issues</span>
      <span class="text-xs text-muted-foreground">{counts.mine || ""}</span>
    </button>
    <button class="{item} {active({ kind: 'all' }) ? 'bg-sidebar-accent font-medium' : ''}" onclick={() => go({ kind: "all" })}>
      <Layers class="size-4 opacity-70" /><span class="flex-1">All issues</span>
      <span class="text-xs text-muted-foreground">{counts.all || ""}</span>
    </button>

    <div class="mt-4 mb-1 flex items-center px-2 text-xs font-medium text-muted-foreground">
      <span class="flex-1">Projects</span>
      <button class="rounded p-0.5 hover:bg-sidebar-accent" title="New project" onclick={() => app.run("newProject")}>
        <Plus class="size-3.5" />
      </button>
    </div>
    {#each app.projects as p (p.id)}
      <ContextMenu.Root>
        <ContextMenu.Trigger>
          <button
            class="{item} {active({ kind: 'project', project: p.id }) ? 'bg-sidebar-accent font-medium' : ''}"
            onclick={() => go({ kind: "project", project: p.id })}
          >
            <FolderClosed class="size-4 opacity-70" /><span class="flex-1 truncate capitalize">{p.name}</span>
            <span class="text-xs text-muted-foreground">{counts.per.get(p.id) || ""}</span>
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
    <button class="{item} {active({ kind: 'noProject' }) ? 'bg-sidebar-accent font-medium' : ''}" onclick={() => go({ kind: "noProject" })}>
      <Inbox class="size-4 opacity-70" /><span class="flex-1">No project</span>
      <span class="text-xs text-muted-foreground">{counts.none || ""}</span>
    </button>
  </nav>

  <div class="flex items-center gap-2 border-t px-3 py-2 text-xs text-muted-foreground">
    <CircleDot class="size-3.5 {app.meta.me ? 'text-green-600' : 'text-destructive'}" />
    <span class="flex-1 truncate" title={app.meta.meError ?? app.meta.me ?? ""}>{app.meta.me ?? "No identity (git config user.email)"}</span>
    <button class="rounded p-1 hover:bg-sidebar-accent" title="Keymap settings (⌘,)" onclick={() => app.run("openSettings")}>
      <Settings class="size-3.5" />
    </button>
  </div>
</aside>
