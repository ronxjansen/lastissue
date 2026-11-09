<!-- The right-click menu over the list and board. One menu for the whole
     pane: the clicked row or card is found by its `data-issue`. On a held
     issue it acts on the whole selection; otherwise on that issue alone,
     leaving the selection as it is. Options and their checks come from the
     same Rust picker the keyboard uses, and every choice is one
     `applyEdit` over the targets. -->
<script lang="ts">
  import { api, type PickerKind, type PickOption } from "$lib/api";
  import { app, report } from "$lib/app.svelte";
  import { titled } from "$lib/format";
  import * as ContextMenu from "$lib/components/ui/context-menu";
  import Check from "@lucide/svelte/icons/check";
  import CircleCheck from "@lucide/svelte/icons/circle-check";
  import CircleDashed from "@lucide/svelte/icons/circle-dashed";
  import Copy from "@lucide/svelte/icons/copy";
  import FolderClosed from "@lucide/svelte/icons/folder-closed";
  import SignalHigh from "@lucide/svelte/icons/signal-high";
  import Tag from "@lucide/svelte/icons/tag";
  import UserRound from "@lucide/svelte/icons/user-round";
  import type { Snippet } from "svelte";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";

  let { children }: { children: Snippet } = $props();

  type Kind = Exclude<PickerKind, "due">;
  const SUBS: { kind: Kind; label: string; icon: typeof Tag; more?: string }[] = [
    { kind: "state", label: "Status", icon: CircleDashed },
    { kind: "priority", label: "Priority", icon: SignalHigh },
    { kind: "project", label: "Project", icon: FolderClosed },
    { kind: "labels", label: "Labels", icon: Tag, more: "New label…" },
    { kind: "assignee", label: "Assignee", icon: UserRound, more: "Someone else…" },
  ];

  let open = $state(false);
  let ids = $state<string[]>([]);
  let options = $state<Partial<Record<Kind, PickOption[]>>>({});

  function oncontextmenu(e: MouseEvent) {
    const el = (e.target as Element | null)?.closest<HTMLElement>("[data-issue]");
    const id = el?.dataset.issue;
    // Off any issue: no menu (and no browser menu either).
    if (!id) return e.preventDefault();
    ids = app.heldWith(id);
    options = {};
  }

  // While open, and again when the targets change under us (a label toggled).
  $effect(() => {
    if (!open) return;
    for (const id of ids) void app.byShortId(id)?.updated;
    const want = ids;
    for (const { kind } of SUBS)
      api.pickerOptions(kind, want).then((r) => {
        if (want === ids) options[kind] = r.map(titled);
      }, report);
  });

  const header = $derived(
    ids.length === 1 ? `${ids[0]} · ${app.byShortId(ids[0])?.title ?? ""}` : `${ids.length} issues`,
  );
  const anyOpen = $derived(ids.some((id) => app.byShortId(id)?.open));

  function more(kind: Kind) {
    app.focus = "issues";
    app.pick(kind, ids);
  }
</script>

<ContextMenu.Root bind:open>
  <ContextMenu.Trigger class="block h-full" {oncontextmenu}>
    {@render children()}
  </ContextMenu.Trigger>
  <ContextMenu.Content class="w-56">
    <ContextMenu.Label class="truncate">{header}</ContextMenu.Label>
    <ContextMenu.Separator />
    {#each SUBS as sub (sub.kind)}
      <ContextMenu.Sub>
        <ContextMenu.SubTrigger class="gap-2"><sub.icon class="size-4 opacity-70" />{sub.label}</ContextMenu.SubTrigger>
        <ContextMenu.SubContent class="max-h-80 w-56">
          {#each options[sub.kind] ?? [] as o (o.label)}
            <ContextMenu.Item closeOnSelect={sub.kind !== "labels"} onSelect={() => app.edit(ids, o.edit)}>
              <span class="grid w-3.5 shrink-0 place-items-center text-link">{#if o.checked}<Check class="size-3.5" />{/if}</span>
              {#if o.edit.kind === "state"}<StateIcon state={o.edit.value} />
              {:else if o.edit.kind === "priority"}<PriorityIcon priority={o.edit.value} />{/if}
              <span class="truncate">{o.label}</span>
              {#if o.detail}<span class="ml-auto truncate pl-2 text-xs text-ink-3">{o.detail}</span>{/if}
            </ContextMenu.Item>
          {:else}
            <ContextMenu.Item disabled><span class="pl-5.5 text-ink-3">{options[sub.kind] ? "Nothing to pick" : "Loading…"}</span></ContextMenu.Item>
          {/each}
          {#if sub.more}
            <ContextMenu.Separator />
            <ContextMenu.Item onSelect={() => more(sub.kind)}><span class="pl-5.5">{sub.more}</span></ContextMenu.Item>
          {/if}
        </ContextMenu.SubContent>
      </ContextMenu.Sub>
    {/each}
    <ContextMenu.Separator />
    <ContextMenu.Item onSelect={() => app.copyIds(ids)}><Copy class="opacity-70" />{ids.length > 1 ? "Copy IDs" : "Copy ID"}</ContextMenu.Item>
    <ContextMenu.Item disabled={!anyOpen} onSelect={() => app.edit(ids, { kind: "state", value: "done" })}>
      <CircleCheck class="opacity-70" />{ids.length > 1 ? `Close ${ids.length} issues` : "Close issue"}
    </ContextMenu.Item>
  </ContextMenu.Content>
</ContextMenu.Root>
