<script lang="ts">
  import { app } from "$lib/app.svelte";
  import * as AlertDialog from "$lib/components/ui/alert-dialog";
  import { buttonVariants } from "$lib/components/ui/button";

  let { title, detail, confirm, run }: { title: string; detail: string; confirm: string; run: () => void } =
    $props();

  function yes() {
    app.overlay = null;
    run();
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Enter" || (e.key === "Backspace" && e.metaKey)) {
      e.preventDefault();
      yes();
    }
  }
</script>

<AlertDialog.Root open={true} onOpenChange={(o) => o || (app.overlay = null)}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <AlertDialog.Content onkeydown={keydown}>
    <AlertDialog.Header>
      <AlertDialog.Title>{title}</AlertDialog.Title>
      <AlertDialog.Description>{detail}</AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
      <AlertDialog.Action class={buttonVariants({ variant: "destructive" })} onclick={yes}>{confirm} <span class="opacity-60">↵</span></AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
