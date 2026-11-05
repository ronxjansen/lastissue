<script lang="ts">
  import type { GroupKey } from "$lib/api";
  import PriorityIcon from "./PriorityIcon.svelte";
  import StateIcon from "./StateIcon.svelte";

  let { key }: { key: GroupKey } = $props();
</script>

{#if key.kind === "state"}<StateIcon state={key.value} />{:else}<PriorityIcon priority={key.value} />{/if}
