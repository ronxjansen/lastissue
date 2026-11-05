<!-- New issue (GUI-SPEC §5.2): written only on submit (⌘↵), so no
     half-made files. Defaults follow the current scope. -->
<script lang="ts">
  import type { Priority, State } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { PRIORITY_LABEL, STATE_LABEL } from "$lib/format";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import MarkdownEditor from "./MarkdownEditor.svelte";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";

  const scope = app.view.scope;
  let title = $state("");
  let body = $state("");
  let issueState = $state<State>("backlog");
  let priority = $state<Priority | null>(null);
  let project = $state<string | null>(scope.kind === "project" ? scope.project : null);
  let labels = $state("");
  let assignToMe = $state(scope.kind === "mine");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let open = $state(true);

  $effect(() => {
    if (!open) app.overlay = null;
  });

  async function submit() {
    if (!title.trim() || busy) return;
    busy = true;
    error = null;
    try {
      await app.createIssue({
        title,
        body,
        state: issueState,
        priority,
        project,
        labels: labels.split(",").map((l) => l.trim()).filter(Boolean),
        assignee: assignToMe ? "me" : null,
      });
      app.overlay = null;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Enter" && e.metaKey) {
      e.preventDefault();
      submit();
    }
  }

  const selectClass = "h-7 rounded-md border bg-transparent px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/50";
</script>

<Dialog.Root bind:open>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <Dialog.Content class="sm:max-w-2xl" onkeydown={keydown}>
    <Dialog.Header>
      <Dialog.Title>New issue</Dialog.Title>
    </Dialog.Header>
    <input
      bind:value={title}
      placeholder="Issue title"
      class="w-full bg-transparent text-lg font-medium outline-none placeholder:text-muted-foreground"
    />
    <MarkdownEditor
      value=""
      placeholder="Add a description… (markdown)"
      onchange={(md) => (body = md)}
      onsubmit={submit}
      class="max-h-[40vh] min-h-24 overflow-y-auto text-sm"
    />
    <div class="flex flex-wrap items-center gap-2">
      <label class="flex items-center gap-1.5">
        <StateIcon state={issueState} />
        <select bind:value={issueState} class={selectClass}>
          {#each Object.entries(STATE_LABEL).filter(([s]) => s !== "duplicate") as [s, l] (s)}
            <option value={s}>{l}</option>
          {/each}
        </select>
      </label>
      <label class="flex items-center gap-1.5">
        <PriorityIcon {priority} />
        <select bind:value={priority} class={selectClass}>
          <option value={null}>{PRIORITY_LABEL.none}</option>
          {#each ["urgent", "high", "medium", "low"] as const as p (p)}
            <option value={p}>{PRIORITY_LABEL[p]}</option>
          {/each}
        </select>
      </label>
      <select bind:value={project} class={selectClass}>
        <option value={null}>No project</option>
        {#each app.projects as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>
      <input bind:value={labels} placeholder="labels, comma separated" class="{selectClass} w-48" />
      <label class="flex items-center gap-1.5 text-xs">
        <input type="checkbox" bind:checked={assignToMe} /> Assign to me
      </label>
    </div>
    {#if error}<p class="text-sm text-destructive">{error}</p>{/if}
    <Dialog.Footer>
      <Button variant="ghost" onclick={() => (open = false)}>Cancel</Button>
      <Button onclick={submit} disabled={!title.trim() || busy}>Create issue <span class="opacity-60">⌘↵</span></Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
