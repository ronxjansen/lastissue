<!-- Read-only markdown (comments, previews). Editing uses MarkdownEditor. -->
<script lang="ts">
  import DOMPurify from "dompurify";
  import { marked } from "marked";

  let { source, class: klass = "" }: { source: string; class?: string } = $props();
  const html = $derived(DOMPurify.sanitize(marked.parse(source, { async: false, gfm: true, breaks: false })));
</script>

<div class="prose-li {klass}">
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitized above -->
  {@html html}
</div>
