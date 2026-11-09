<!-- Fixed-height rows, rendered only around the viewport, so cost doesn't
     grow with the issue count. `scrollTo` keeps a row in view. -->
<script lang="ts" generics="T">
  import type { Snippet } from "svelte";

  let {
    items,
    rowHeight,
    row,
    scrollTo = -1,
    overscan = 8,
    padEnd = 0,
    class: klass = "",
  }: {
    items: T[];
    rowHeight: number;
    row: Snippet<[T]>;
    scrollTo?: number;
    overscan?: number;
    /** Extra space after the last row (for something floating over the end). */
    padEnd?: number;
    class?: string;
  } = $props();

  let el: HTMLDivElement | undefined = $state();
  let top = $state(0);
  let height = $state(600);

  const first = $derived(Math.max(0, Math.floor(top / rowHeight) - overscan));
  const last = $derived(Math.min(items.length, Math.ceil((top + height) / rowHeight) + overscan));

  $effect(() => {
    if (!el || scrollTo < 0) return;
    const y = scrollTo * rowHeight;
    if (y < el.scrollTop) el.scrollTop = y;
    else if (y + rowHeight > el.scrollTop + el.clientHeight - padEnd)
      el.scrollTop = y + rowHeight - el.clientHeight + padEnd;
  });
</script>

<div
  bind:this={el}
  bind:clientHeight={height}
  onscroll={() => (top = el!.scrollTop)}
  class="relative overflow-y-auto {klass}"
>
  <div style="height: {items.length * rowHeight + padEnd}px" class="relative">
    {#each items.slice(first, last) as item, i (first + i)}
      <div class="absolute inset-x-0" style="top: {(first + i) * rowHeight}px; height: {rowHeight}px">
        {@render row(item)}
      </div>
    {/each}
  </div>
</div>
