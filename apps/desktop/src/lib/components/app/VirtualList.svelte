<!-- Fixed-height rows, rendered only around the viewport, so cost doesn't
     grow with the issue count (GUI-SPEC §3). `scrollTo` keeps a row in view. -->
<script lang="ts" generics="T">
  import type { Snippet } from "svelte";

  let {
    items,
    rowHeight,
    row,
    scrollTo = null,
    overscan = 8,
    class: klass = "",
    padTop = 0,
  }: {
    items: T[];
    rowHeight: number;
    row: Snippet<[T, number]>;
    scrollTo?: number | null;
    overscan?: number;
    class?: string;
    padTop?: number;
  } = $props();

  let el: HTMLDivElement | undefined = $state();
  let top = $state(0);
  let height = $state(600);

  const first = $derived(Math.max(0, Math.floor((top - padTop) / rowHeight) - overscan));
  const last = $derived(Math.min(items.length, Math.ceil((top + height) / rowHeight) + overscan));

  $effect(() => {
    if (scrollTo === null || !el || scrollTo < 0) return;
    const y = padTop + scrollTo * rowHeight;
    if (y < el.scrollTop + padTop) el.scrollTop = Math.max(0, y - padTop);
    else if (y + rowHeight > el.scrollTop + el.clientHeight) el.scrollTop = y + rowHeight - el.clientHeight;
  });
</script>

<div
  bind:this={el}
  bind:clientHeight={height}
  onscroll={() => (top = el!.scrollTop)}
  class="relative overflow-y-auto {klass}"
>
  <div style="height: {padTop + items.length * rowHeight}px" class="relative">
    {#each items.slice(first, last) as item, i (first + i)}
      <div class="absolute inset-x-0" style="top: {padTop + (first + i) * rowHeight}px; height: {rowHeight}px">
        {@render row(item, first + i)}
      </div>
    {/each}
  </div>
</div>
