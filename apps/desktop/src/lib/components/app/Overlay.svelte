<!-- A full-window layer that holds a panel near the top and closes on a
     click outside it. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  let { children, onclose }: { children: Snippet; onclose: () => void } = $props();
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="fixed inset-0 z-50 flex justify-center bg-[oklch(0.2_0.01_95/0.18)] pt-[12vh] backdrop-blur-[1.5px] dark:bg-black/45" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="h-fit">{@render children()}</div>
</div>
