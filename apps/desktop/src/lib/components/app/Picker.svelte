<!-- Linear-style property picker (GUI-SPEC §5): options come from Rust
     (they need the index and date parsing) and are re-asked per keystroke.
     The chosen edit goes to every target (a bulk edit for a selection).
     Labels stay open to toggle several. -->
<script lang="ts">
  import { api, type PickerKind, type PickOption } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import CheckIcon from "@lucide/svelte/icons/check";
  import FuzzyList from "./FuzzyList.svelte";
  import Overlay from "./Overlay.svelte";

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
  let rows = $state<PickOption[]>([]);
  let asked = 0;

  async function refresh() {
    const n = ++asked;
    const r = await api.pickerOptions(picker, ids, query);
    if (n === asked) rows = r;
  }

  $effect(() => {
    void query;
    refresh();
  });

  // The index changed under us (e.g. a label toggled): re-check marks.
  $effect(() => {
    void app.issues.size;
    for (const id of ids) void app.byShortId(id)?.labels;
    refresh();
  });

  async function pick(o: PickOption) {
    await app.edit(ids, o.edit);
    if (picker === "labels") {
      query = "";
      await refresh();
    } else {
      app.overlay = null;
    }
  }

  const target = $derived(
    ids.length === 1 ? (app.byShortId(ids[0])?.title ?? ids[0]) : `${ids.length} issues`,
  );
</script>

<Overlay onclose={() => (app.overlay = null)}>
  <FuzzyList
    bind:query
    {rows}
    placeholder={PLACEHOLDER[picker]}
    onpick={(o) => pick(o)}
    oncancel={() => (app.overlay = null)}
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
</Overlay>
