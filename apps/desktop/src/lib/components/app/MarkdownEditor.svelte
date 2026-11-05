<!--
  WYSIWYG markdown (Milkdown: ProseMirror + remark). Markdown stays the
  source of truth, and untouched text is never rewritten:

  - Loading a value (`value` + a new `reset`) records the editor's own
    serialization of it as the baseline.
  - A change reports Milkdown's markdown, unless it serializes back to the
    baseline (an undo to the start), which reports the original `value`
    byte for byte. Nothing is reported until the user changes the doc.
-->
<script lang="ts">
  import { Editor, defaultValueCtx, editorViewCtx, editorViewOptionsCtx, rootCtx } from "@milkdown/kit/core";
  import { history } from "@milkdown/kit/plugin/history";
  import { clipboard } from "@milkdown/kit/plugin/clipboard";
  import { listener, listenerCtx } from "@milkdown/kit/plugin/listener";
  import { commonmark } from "@milkdown/kit/preset/commonmark";
  import { gfm } from "@milkdown/kit/preset/gfm";
  import { getMarkdown, replaceAll } from "@milkdown/kit/utils";
  import { onDestroy, onMount } from "svelte";

  let {
    value,
    reset = 0,
    editable = true,
    placeholder = "",
    autofocus = false,
    onchange,
    onsubmit,
    onescape,
    onclick,
    class: klass = "",
  }: {
    value: string;
    /** Bump to load `value` into the editor (replacing its text). */
    reset?: number;
    editable?: boolean;
    placeholder?: string;
    autofocus?: boolean;
    onchange?: (markdown: string) => void;
    /** ⌘↵ */
    onsubmit?: () => void;
    onescape?: () => void;
    onclick?: () => void;
    class?: string;
  } = $props();

  let host: HTMLDivElement;
  let editor: Editor | null = null;
  let baseline = "";
  let original = "";
  let loadedReset = -1;
  let empty = $state(true);
  let editableNow = false;

  function load(v: string) {
    if (!editor) return;
    original = v;
    editor.action(replaceAll(v, true));
    baseline = editor.action(getMarkdown());
    empty = v.trim() === "";
  }

  onMount(async () => {
    original = value;
    editor = await Editor.make()
      .config((ctx) => {
        ctx.set(rootCtx, host);
        ctx.set(defaultValueCtx, value);
        ctx.update(editorViewOptionsCtx, (prev) => ({ ...prev, editable: () => editableNow }));
        ctx.get(listenerCtx).markdownUpdated((_, md) => {
          empty = md.trim() === "";
          onchange?.(md === baseline ? original : md);
        });
      })
      .use(commonmark)
      .use(gfm)
      .use(history)
      .use(clipboard)
      .use(listener)
      .create();
    baseline = editor.action(getMarkdown());
    loadedReset = reset;
    empty = value.trim() === "";
    if (autofocus) focus();
  });

  onDestroy(() => {
    editor?.destroy();
    editor = null;
  });

  $effect(() => {
    if (editor && reset !== loadedReset) {
      loadedReset = reset;
      load(value);
    }
  });

  $effect(() => {
    editableNow = editable;
    editor?.action((ctx) => {
      const view = ctx.get(editorViewCtx);
      view.setProps({ editable: () => editableNow });
      if (editableNow) view.focus();
    });
  });

  function focus() {
    editor?.action((ctx) => ctx.get(editorViewCtx).focus());
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && onsubmit) {
      e.preventDefault();
      e.stopPropagation();
      onsubmit();
    } else if (e.key === "Escape" && onescape) {
      e.preventDefault();
      e.stopPropagation();
      onescape();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="md-editor relative {klass}" class:md-readonly={!editable} onkeydown={keydown} {onclick}>
  {#if empty && placeholder}
    <div class="pointer-events-none absolute top-0 left-0 text-muted-foreground">{placeholder}</div>
  {/if}
  <div bind:this={host}></div>
</div>
