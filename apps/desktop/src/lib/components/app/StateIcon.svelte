<script lang="ts">
  import type { State } from "$lib/api";
  import { STATE_LABEL } from "$lib/format";

  let { state, size = 14 }: { state: State; size?: number } = $props();

  const color: Record<State, string> = {
    backlog: "var(--ink-3)",
    todo: "var(--ink-3)",
    "in-progress": "var(--state-progress)",
    "in-review": "var(--state-review)",
    done: "var(--state-done)",
    cancelled: "var(--ink-3)",
    duplicate: "var(--ink-3)",
  };
  // Fraction of the pie filled for the in-flight states.
  const pie: Partial<Record<State, number>> = { "in-progress": 0.5, "in-review": 0.75 };

  function arc(frac: number): string {
    const a = frac * 2 * Math.PI;
    const x = 7 + 3.5 * Math.sin(a);
    const y = 7 - 3.5 * Math.cos(a);
    return `M7 7 L7 3.5 A3.5 3.5 0 ${frac > 0.5 ? 1 : 0} 1 ${x} ${y} Z`;
  }
</script>

<svg width={size} height={size} viewBox="0 0 14 14" class="shrink-0" role="img" aria-label={STATE_LABEL[state]}>
  <title>{STATE_LABEL[state]}</title>
  {#if state === "done" || state === "cancelled" || state === "duplicate"}
    <circle cx="7" cy="7" r="6" fill={color[state]} />
    {#if state === "done"}
      <path d="M4.5 7.2 6.3 9 9.6 5.4" stroke="var(--sheet)" stroke-width="1.5" fill="none" stroke-linecap="round" stroke-linejoin="round" />
    {:else}
      <path d="M5 5l4 4M9 5 5 9" stroke="var(--sheet)" stroke-width="1.5" stroke-linecap="round" />
    {/if}
  {:else}
    <circle
      cx="7"
      cy="7"
      r="5.5"
      fill="none"
      stroke={color[state]}
      stroke-width="1.5"
      stroke-dasharray={state === "backlog" ? "1.6 1.6" : undefined}
    />
    {#if pie[state]}
      <path d={arc(pie[state]!)} fill={color[state]} />
    {/if}
  {/if}
</svg>
