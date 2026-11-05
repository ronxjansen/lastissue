<script lang="ts">
  import type { Priority } from "$lib/api";
  import { PRIORITY_LABEL } from "$lib/format";

  let { priority, size = 14 }: { priority: Priority | null; size?: number } = $props();
  const level = $derived(priority === "high" ? 3 : priority === "medium" ? 2 : priority === "low" ? 1 : 0);
</script>

<svg width={size} height={size} viewBox="0 0 14 14" class="shrink-0" role="img" aria-label={PRIORITY_LABEL[priority ?? "none"]}>
  <title>{PRIORITY_LABEL[priority ?? "none"]}</title>
  {#if priority === "urgent"}
    <rect x="1" y="1" width="12" height="12" rx="3" fill="oklch(0.65 0.2 40)" />
    <path d="M7 3.8v4" stroke="white" stroke-width="1.6" stroke-linecap="round" />
    <circle cx="7" cy="10.2" r="0.95" fill="white" />
  {:else if priority === null}
    {#each [2, 6, 10] as x (x)}
      <rect {x} y="6.3" width="2" height="1.4" rx="0.5" fill="var(--muted-foreground)" opacity="0.7" />
    {/each}
  {:else}
    {#each [1, 2, 3] as bar (bar)}
      <rect
        x={1.5 + (bar - 1) * 4}
        y={11 - bar * 3}
        width="3"
        height={bar * 3}
        rx="0.8"
        fill="var(--foreground)"
        opacity={bar <= level ? 0.85 : 0.2}
      />
    {/each}
  {/if}
</svg>
