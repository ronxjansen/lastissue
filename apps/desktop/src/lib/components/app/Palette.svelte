<!-- ⌘K: every command with its binding, plus repos and
     projects. `g p` opens it narrowed to scopes. -->
<script lang="ts">
  import type { Scope } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { rank } from "$lib/fuzzy";
  import { ACTIONS, type Action, bindingFor, pretty } from "$lib/keymap";
  import { Kbd } from "$lib/components/ui/kbd";
  import FuzzyList from "./FuzzyList.svelte";

  let { mode }: { mode: "all" | "scopes" } = $props();

  type Entry =
    | { kind: "action"; action: Action; title: string; hint: string | null }
    | { kind: "repo"; index: number; title: string; detail: string }
    | { kind: "scope"; scope: Scope; title: string };

  // Bindings as they apply behind the palette.
  const contexts = app.contexts(true);

  const entries = $derived.by((): Entry[] => {
    const out: Entry[] = [];
    if (mode === "all") {
      for (const [action, title] of Object.entries(ACTIONS) as [Action, string][]) {
        if (action.startsWith("switchRepo")) continue;
        const keys = bindingFor(app.keymap, action, contexts);
        out.push({ kind: "action", action, title, hint: keys && pretty(keys) });
      }
      app.meta.repos.forEach((r, index) =>
        out.push({ kind: "repo", index, title: `Switch to repo ${r.label}`, detail: r.root }),
      );
    }
    const prefix = mode === "all" ? "Go to project " : "";
    for (const p of app.projects)
      out.push({ kind: "scope", scope: { kind: "project", project: p.id }, title: prefix + p.name });
    if (mode === "scopes") {
      out.push({ kind: "scope", scope: { kind: "noProject" }, title: "No project" });
      out.push({ kind: "scope", scope: { kind: "all" }, title: "All issues" });
      out.push({ kind: "scope", scope: { kind: "mine" }, title: "My issues" });
    }
    return out;
  });

  let query = $state("");
  const rows = $derived(rank(entries, query, (e) => e.title));

  function pick(e: Entry) {
    app.overlay = null;
    if (e.kind === "action") app.run(e.action);
    else if (e.kind === "repo") app.switchRepo(e.index);
    else app.setView({ scope: e.scope });
  }
</script>

<FuzzyList
  bind:query
  {rows}
  placeholder={mode === "all" ? "Type a command…" : "Go to project…"}
  onpick={pick}
  onclose={() => (app.overlay = null)}
>
  {#snippet row(e)}
    <div class="flex items-center gap-2">
      <span class="flex-1 truncate">{e.title}</span>
      {#if e.kind === "repo"}<span class="truncate text-xs text-muted-foreground">{e.detail}</span>{/if}
      {#if e.kind === "action" && e.hint}<Kbd>{e.hint}</Kbd>{/if}
    </div>
  {/snippet}
  {#snippet footer()}
    {#if app.merged.errors.length || app.userKeymap.errors.length}
      <span class="text-destructive">Keymap: {[...app.userKeymap.errors, ...app.merged.errors].join("; ")}</span>
    {:else}
      ↵ run · esc close
    {/if}
  {/snippet}
</FuzzyList>
