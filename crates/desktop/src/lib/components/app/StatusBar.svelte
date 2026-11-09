<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { ACTIONS, pretty } from "$lib/keymap";
</script>

<footer class="flex h-7 shrink-0 items-center gap-4 border-t border-rule bg-rail px-4 text-[12px] text-ink-3">
  {#if app.meta.notice}<span class="truncate text-danger">{app.meta.notice}</span>{/if}
  {#if app.meta.failure}<span class="truncate text-warn">{app.meta.failure}</span>{/if}
  {#if app.meta.loadErrors.length}
    <span class="truncate text-warn" title={app.meta.loadErrors.join("\n")}>{app.meta.loadErrors.length} unreadable file(s)</span>
  {/if}
  {#if app.meta.error}<span class="truncate text-danger" title={app.meta.error}>{app.meta.error}</span>{/if}
  <span class="flex-1"></span>
  <!-- Two or more held: the selection bar shows the count instead. -->
  {#if app.selection.size === 1}
    <span class="flex items-center gap-1.5 text-ink-2">
      <b class="grid h-4 min-w-4 place-items-center rounded-full bg-link px-1 font-mono text-[11px] font-semibold text-sheet tabular-nums">1</b>
      selected<span class="text-ink-3">· {pretty("escape")} clear</span>
    </span>
  {/if}
  <span><kbd class="text-ink-2">{pretty("cmd-k")}</kbd> {ACTIONS.openPalette.toLowerCase()}</span>
</footer>
