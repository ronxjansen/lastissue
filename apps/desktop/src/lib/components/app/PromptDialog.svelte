<script lang="ts">
  import { app } from "$lib/app.svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Input } from "$lib/components/ui/input";

  let {
    title,
    placeholder,
    value: initial,
    submit,
  }: { title: string; placeholder: string; value: string; submit: (v: string) => Promise<unknown> } = $props();

  // svelte-ignore state_referenced_locally
  let value = $state(initial);
  let error = $state<string | null>(null);

  async function go(e: Event) {
    e.preventDefault();
    if (!value.trim()) return;
    try {
      await submit(value.trim());
      app.overlay = null;
    } catch (err) {
      error = String(err);
    }
  }
</script>

<Dialog.Root open={true} onOpenChange={(o) => o || (app.overlay = null)}>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header><Dialog.Title>{title}</Dialog.Title></Dialog.Header>
    <form onsubmit={go} class="grid gap-3">
      <Input bind:value {placeholder} autofocus />
      {#if error}<p class="text-sm text-destructive">{error}</p>{/if}
      <Dialog.Footer>
        <Button variant="ghost" type="button" onclick={() => (app.overlay = null)}>Cancel</Button>
        <Button type="submit" disabled={!value.trim()}>Save</Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
