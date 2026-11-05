<!-- The one fuzzy-list component (GUI-SPEC §7): a filter input over rows,
     ↑↓ to move the highlight, ↵ to pick, esc to close. Pickers, the ⌘K
     palette and the / search box all render through it. -->
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
    oncancel,
    width = "w-[520px]",
  }: {
    query?: string;
    rows: T[];
    placeholder: string;
    empty?: string;
    row: Snippet<[T, boolean]>;
    header?: Snippet;
    footer?: Snippet;
    onpick: (item: T, index: number, e: KeyboardEvent | MouseEvent) => void;
    oncancel: () => void;
    width?: string;
  } = $props();

  let highlight = $state(0);
  let list: HTMLDivElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();

  $effect(() => {
    // New rows (typing): back to the top.
    void rows;
    highlight = 0;
  });

  $effect(() => {
    list?.querySelector(`[data-row="${highlight}"]`)?.scrollIntoView({ block: "nearest" });
  });

  $effect(() => input?.focus());

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
      if (item !== undefined) onpick(item, highlight, e);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancel();
    }
  }
</script>

<div class="{width} max-w-[calc(100vw-2rem)] overflow-hidden rounded-xl border border-rule bg-popover text-popover-foreground shadow-pop">
  <input
    bind:this={input}
    bind:value={query}
    onkeydown={keydown}
    {placeholder}
    spellcheck="false"
    autocomplete="off"
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
        data-row={i}
        class="cursor-default rounded-md px-3 py-2 text-[14px] {i === highlight ? 'bg-wash-strong text-ink' : 'text-ink-2'}"
        onmousemove={() => (highlight = i)}
        onclick={(e) => onpick(item, i, e)}
      >
        {@render row(item, i === highlight)}
      </div>
    {:else}
      <div class="px-3 py-3 text-[14px] text-ink-3">{empty}</div>
    {/each}
  </div>
  {#if footer}
    <div class="border-t border-rule bg-paper px-4 py-2 text-[12px] text-ink-3">{@render footer()}</div>
  {/if}
</div>
