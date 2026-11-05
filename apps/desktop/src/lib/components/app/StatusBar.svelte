<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { ACTIONS, pretty } from "$lib/keymap";

  const sel = $derived(app.selection.length);
</script>

<footer class="flex h-6 shrink-0 items-center gap-3 border-t px-3 text-[11px] text-muted-foreground">
  {#if app.meta.notice}<span class="truncate text-destructive">{app.meta.notice}</span>{/if}
  {#if app.meta.failure}<span class="truncate text-amber-600">{app.meta.failure}</span>{/if}
  {#if app.meta.loadErrors.length}
    <span class="truncate text-amber-600" title={app.meta.loadErrors.join("\n")}>{app.meta.loadErrors.length} unreadable file(s)</span>
  {/if}
  {#if app.meta.error}<span class="truncate text-destructive" title={app.meta.error}>{app.meta.error}</span>{/if}
  <span class="flex-1"></span>
  {#if sel}<span>{sel} selected · {pretty("escape")} clear</span>{/if}
  <span>{pretty("cmd-k")} {ACTIONS.openPalette.toLowerCase()}</span>
</footer>
