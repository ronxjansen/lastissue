<!-- `/` search (GUI-SPEC §6): instant hits from memory, full-text hits
     streamed in from Rust, tiered (title → label/ID → body → comment).
     ↵ opens the issue; ⌘↵ applies the query as the view's filter. -->
<script lang="ts">
  import { api, type Hit } from "$lib/api";
  import { app, report } from "$lib/app.svelte";
  import FuzzyList from "./FuzzyList.svelte";
  import Overlay from "./Overlay.svelte";
  import StateIcon from "./StateIcon.svelte";

  let query = $state(app.view.filter);

  $effect(() => {
    api.search(query).catch(report);
  });

  function close() {
    api.search("").catch(report);
    app.overlay = null;
  }

  function pick(h: Hit, _i: number, e: KeyboardEvent | MouseEvent) {
    if (e.metaKey) {
      app.setView({ filter: query });
    } else {
      app.open(h.id);
      app.focus = "detail";
    }
    close();
  }
</script>

<Overlay onclose={close}>
  <FuzzyList
    bind:query
    rows={app.search.query === query ? app.search.hits : []}
    placeholder="Search issues — state:todo @me label:bug due:<7d …"
    empty={app.search.error ?? (query.trim() ? (app.search.scanning ? "Searching…" : "No matches") : "Type to search titles, labels, bodies and comments")}
    onpick={pick}
    oncancel={close}
    width="w-[640px]"
  >
    {#snippet row(h)}
      {@const s = app.issues.get(h.key)}
      <div class="flex items-center gap-2">
        {#if s}<StateIcon state={s.state} />{/if}
        <span class="w-16 shrink-0 font-mono text-xs text-muted-foreground">{h.id}</span>
        <span class="flex-1 truncate">{s?.title ?? h.key}</span>
        <span class="text-xs text-muted-foreground">{h.tier}</span>
      </div>
      {#each h.snippets as sn, i (i)}
        <div class="truncate pl-24 text-xs text-muted-foreground"><span class="opacity-70">{sn.loc}</span> {sn.text}</div>
      {/each}
    {/snippet}
    {#snippet footer()}
      {#if app.search.scanning}Searching bodies and comments… ·{/if}
      ↵ open · ⌘↵ apply as filter · esc close
    {/snippet}
  </FuzzyList>
</Overlay>
