<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { ACTIONS, pretty } from "$lib/keymap";

  const sel = $derived(app.selection.length);
</script>

<footer class="flex h-7 shrink-0 items-center gap-4 border-t border-rule bg-rail px-4 text-[12px] text-ink-3">
  {#if app.meta.notice}<span class="truncate text-danger">{app.meta.notice}</span>{/if}
  {#if app.meta.failure}<span class="truncate text-warn">{app.meta.failure}</span>{/if}
  {#if app.meta.loadErrors.length}
    <span class="truncate text-warn" title={app.meta.loadErrors.join("\n")}>{app.meta.loadErrors.length} unreadable file(s)</span>
  {/if}
  {#if app.meta.error}<span class="truncate text-danger" title={app.meta.error}>{app.meta.error}</span>{/if}
  <span class="flex-1"></span>
  {#if sel}<span>{sel} selected · {pretty("escape")} clear</span>{/if}
  <span><kbd class="text-ink-2">{pretty("cmd-k")}</kbd> {ACTIONS.openPalette.toLowerCase()}</span>
</footer>
