<!-- Linear-style property picker: options come from Rust
     once (they need the index) and are ranked here as the user types; the
     typed value (a new label, an address, a date) is asked of Rust per
     keystroke. The chosen edit goes to every target (a bulk edit for a
     selection). Labels stay open to toggle several. -->
<script lang="ts">
  import { api, type PickerKind, type PickOption } from "$lib/api";
  import { app, report } from "$lib/app.svelte";
  import { titled } from "$lib/format";
  import { rank } from "$lib/fuzzy";
  import CheckIcon from "@lucide/svelte/icons/check";
  import FuzzyList from "./FuzzyList.svelte";

  let { picker, ids }: { picker: PickerKind; ids: string[] } = $props();

  const PLACEHOLDER: Record<PickerKind, string> = {
    state: "Change state…",
    priority: "Set priority…",
    assignee: "Assign to… (name or address)",
    labels: "Toggle labels, or type a new one…",
    due: "Due: tomorrow, +1w, fri, 2026-12-01…",
    project: "Move to project…",
  };

  let query = $state("");
  let options = $state<PickOption[]>([]);
  let typed = $state<PickOption | null>(null);
  let asked = 0;

  // On open, and when the index changes under us (e.g. a label toggled).
  $effect(() => {
    void app.issues.size;
    for (const id of ids) void app.byShortId(id)?.labels;
    api.pickerOptions(picker, ids).then((r) => (options = r.map(titled)), report);
  });

  $effect(() => {
    const n = ++asked;
    if (!query.trim()) typed = null;
    else api.pickerTyped(picker, ids, query).then((t) => n === asked && (typed = t), report);
  });

  const rows = $derived.by(() => {
    const ranked = rank(options, query, (o) => (o.detail ? `${o.label} ${o.detail}` : o.label));
    const q = query.trim().toLowerCase();
    return typed && !ranked.some((o) => o.label.toLowerCase() === q) ? [typed, ...ranked] : ranked;
  });

  async function pick(o: PickOption) {
    await app.edit(ids, o.edit);
    if (picker === "labels") query = "";
    else app.overlay = null;
  }

  const target = $derived(
    ids.length === 1 ? (app.byShortId(ids[0])?.title ?? ids[0]) : `${ids.length} issues`,
  );
</script>

<FuzzyList
  bind:query
  {rows}
  placeholder={PLACEHOLDER[picker]}
  onpick={pick}
  onclose={() => (app.overlay = null)}
  width="w-[440px]"
>
  {#snippet header()}
    <div class="truncate px-4 pt-2 text-xs text-muted-foreground">{target}</div>
  {/snippet}
  {#snippet row(o)}
    <div class="flex items-center gap-2">
      <span class="w-4 shrink-0 text-primary">{#if o.checked}<CheckIcon class="size-3.5" />{/if}</span>
      <span class="truncate">{o.label}</span>
      {#if o.detail}<span class="truncate text-xs text-muted-foreground">{o.detail}</span>{/if}
    </div>
  {/snippet}
</FuzzyList>
