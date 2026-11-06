<!-- The one fuzzy-list overlay: a filter input over rows near the top of
     the window, ↑↓ to move the highlight, ↵ to pick, esc or a click outside
     to close. Pickers, the ⌘K palette and the / search box all render
     through it. -->
<script lang="ts" generics="T">
  import type { Snippet } from "svelte";
  import { step } from "$lib/fuzzy";

  let {
    query = $bindable(""),
    rows,
    placeholder,
    empty = "No matches",
    row,
    header,
    footer,
    onpick,
    onclose,
    width = "w-[520px]",
  }: {
    query?: string;
    rows: T[];
    placeholder: string;
    empty?: string;
    row: Snippet<[T]>;
    header?: Snippet;
    footer?: Snippet;
    onpick: (item: T, e: KeyboardEvent | MouseEvent) => void;
    onclose: () => void;
    width?: string;
  } = $props();

  // New rows (typing): back to the top.
  let highlight = $derived.by(() => {
    void rows;
    return 0;
  });
  let list: HTMLDivElement | undefined = $state();

  $effect(() => {
    list?.children[highlight]?.scrollIntoView({ block: "nearest" });
  });

  function keydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || (e.key === "n" && e.ctrlKey)) {
      e.preventDefault();
      highlight = step(highlight, rows.length, 1);
    } else if (e.key === "ArrowUp" || (e.key === "p" && e.ctrlKey)) {
      e.preventDefault();
      highlight = step(highlight, rows.length, -1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = rows[highlight];
      if (item !== undefined) onpick(item, e);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="fixed inset-0 z-50 flex justify-center bg-[oklch(0.2_0.01_95/0.18)] pt-[12vh] backdrop-blur-[1.5px] dark:bg-black/45" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="{width} h-fit max-w-[calc(100vw-2rem)] overflow-hidden rounded-xl border border-rule bg-popover text-popover-foreground shadow-pop">
    <!-- svelte-ignore a11y_autofocus -->
    <input
      bind:value={query}
      onkeydown={keydown}
      {placeholder}
      spellcheck="false"
      autocomplete="off"
      autofocus
      class="h-12 w-full border-b border-rule bg-transparent px-4 text-[15px] outline-none placeholder:text-ink-3"
    />
    {#if header}{@render header()}{/if}
    <div bind:this={list} class="max-h-[min(420px,60vh)] overflow-y-auto p-1">
      {#each rows as item, i (i)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          role="option"
          tabindex="-1"
          aria-selected={i === highlight}
          class="cursor-default rounded-md px-3 py-2 text-[14px] {i === highlight ? 'bg-wash-strong text-ink' : 'text-ink-2'}"
          onmousemove={() => (highlight = i)}
          onclick={(e) => onpick(item, e)}
        >
          {@render row(item)}
        </div>
      {:else}
        <div class="px-3 py-3 text-[14px] text-ink-3">{empty}</div>
      {/each}
    </div>
    {#if footer}
      <div class="border-t border-rule bg-paper px-4 py-2 text-[12px] text-ink-3">{@render footer()}</div>
    {/if}
  </div>
</div>
