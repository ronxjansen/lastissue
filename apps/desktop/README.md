# lastissue desktop app

A Tauri app over the core library. Its Rust side links `lastissue-core`
directly and never shells out to the CLI. The file format and what each edit
means live in [`crates/core`](../../crates/core/README.md). Visual design is
in `DESIGN.md`, product intent in `PRODUCT.md`, both at the repo root.

## Stack

- **Tauri 2.** The crate in `src-tauri` (package `desktop`, binary
  `lastissue-gui`) holds the app state; the webview renders it.
- **Svelte 5 + TypeScript** (Vite, plain SPA, no SvelteKit), **Tailwind 4**,
  **shadcn-svelte** (bits-ui), `@lucide/svelte` icons, pnpm.
- **Milkdown** (commonmark + gfm) edits bodies and comments WYSIWYG; `marked`
  + DOMPurify render read-only comments.
- `notify` watches the disk; `tauri-plugin-deep-link`, `-clipboard-manager`
  and `-dialog`.

## Where logic lives

Rust keeps whatever needs the index, the disk or core semantics; TypeScript
keeps pure interaction. The test is which side makes the app simpler, not
"everything in Rust".

| Rust (`src-tauri/src/state`) | TypeScript (`src/lib`) |
|---|---|
| Index and watcher; view filter, sort and group; both search tiers; edits through `core::ops`; title/body drafts and disk conflicts; crash-guard drafts; picker options; `lastissue://` links; the user keymap file | Cursor and multi-selection; keymap defaults and dispatch; the `⌘K` list and scoring; save debounce; rendering |

The front-end mirrors issue heads (keyed by path relative to the root) from
pushed `index` deltas, and renders the `visible` result Rust pushes after each
view change, disk change or full-text batch. The TS side queues writes in
order so async commands can't reorder them.

## Data and sync

- Opening a repo loads every issue head through the core index. Bodies and
  comments load when an issue opens.
- Lists and board columns are virtualized with fixed row heights, so render
  cost doesn't grow with the issue count. Target: ~50k issues per repo.
- **Watcher:** debounced `notify` on the open issues root. Changed `.mbox`
  files and project folders are re-read and diffed into the index.
- **Writes** go through `core::ops` and update the UI optimistically. The app
  recognises its own writes by path and mtime, so they cause no flicker.
- **Conflicts:** last write wins. If the open issue changes on disk while
  being edited, a "changed on disk: reload / overwrite" banner appears.
- **Repos:** one per window, with a switcher fed by the registry. Not built
  yet: telling apart two repos with the same label.

## Identity

`git config user.name` / `user.email`, run in the repo's directory, so
per-repo identities hold. Never `$USER@host`, and never `Sender:`: the GUI is
always a human. "My issues", `@me` and "assign to me" (`i`) use this identity.

Not built yet: an app-level override (`gui.toml` in the config dir) and a
first-run prompt for when git has no identity.

## Views and editing

A sidebar (repo switcher, my issues, all, projects, no project), a list or
board of the filtered set, and a detail pane: title, body, properties,
relations, and an activity timeline of comments plus compact, collapsible
events.

Not built: Linear import, rich relations editing, saved views, inbox,
notifications, attachments.

### Board

- Columns are states. Dropping a card on another column writes exactly what
  `issue update --state` would. Grouped by priority, dropping changes the
  priority.
- **No manual order.** Cards follow the active sort; there is no rank header,
  and dropping within a column does nothing.
- `⇧h` / `⇧l` move a card to the previous / next state.

### Saving

- Title and body edits save on a **300 ms debounce**. Core event coalescing
  keeps each burst to one event.
- Property pickers commit at once.
- The create dialog writes only on submit (`⌘Enter`): no half-made files.
- **Crash guard:** unsaved drafts are mirrored to the platform cache dir
  (`drafts/<message-id>.md`) and offered back on the next open.

### Markdown

- **Issue body:** a Milkdown editor, read-only until `e` or a click.
- **Comments:** rendered read-only with sanitized `marked`. Editing a comment
  swaps in a Milkdown editor for that comment only; the composer is one too.
- **No silent rewrites.** A WYSIWYG editor re-serializes markdown, so the
  editor stores its own serialization of the loaded text as a baseline and
  reports nothing until the user changes the document. A change that
  serializes back to the baseline returns the original text byte for byte.
  Viewing an agent-written issue never writes it; a real edit may normalize
  the edited field (e.g. `*` bullets become `-`). Core keeps untouched
  messages verbatim.

## Search

`/` opens one box with two tiers:

1. **Instant**, in memory, on every keystroke: title, short ID, labels,
   assignee, project.
2. **Full text**, after 150 ms idle: core search over bodies and comments on
   a background thread. Results stream in with snippets. Each keystroke
   cancels the previous scan (generation counter).

Filter tokens use the core parser, the same one as `issue list --filter`.
`⌘Enter` applies the search as the view's filter. Results are tiered, not
ranked: title, then label or ID, then body, then comment; within a tier,
newest first, then ID.

## Keyboard

One action registry (`src/lib/keymap.ts`); every action shows in `⌘K` with its
binding. Contexts, least to most specific: `global`, `workspace` (no text
input or overlay focused), then `issues` + `list`/`board`, or `detail`. The
most specific binding wins.

| Context | Keys |
|---|---|
| Global | `⌘K` palette · `/` search · `c` create · `g i`/`g p`/`g b`/`g l` my issues, projects, board, list · `⌘1..9` switch repo · `⌘,` settings |
| List / board | `j`/`k` (and arrows) move · `h`/`l` between columns · `Enter`/`o` open · `x` select · `Esc` clear |
| Issue | `s` state · `p` priority · `a` assignee · `i` assign to me · `l` labels · `d` due · `m` move to project · `r` rename · `e` edit body · `⌘Enter` submit/close · `⌘⌫` delete (confirms) |
| Board | `⇧h`/`⇧l` move card to previous / next state |
| Pickers | type to filter · `↑↓` · `Enter` · `Esc` (one fuzzy-list component, shared with `⌘K`) |

- Defaults live in TS. Overrides go in `gui-keymap.json` in the config dir:
  `{ "context": { "keys": "action" | null } }`, where `null` unbinds. Rust
  reads and watches it and the app applies changes live; `⌘,` creates and
  opens it. Problems show in a toast and the palette footer.
- Single-letter bindings fire only when no text input has focus.
- No vim mode.

## Code layout

```
src-tauri/src/
  main.rs           # Tauri setup: plugins, workspace, keymap watcher
  commands.rs       # #[tauri::command]s, thin over state/workspace
  dto.rs            # serde mirrors of core and state types (core has no serde)
  keymap.rs         # gui-keymap.json: parse, load, watch
  link.rs           # lastissue:// parsing
  state/
    workspace/      # repos, open repo, view, doc, searches, writes; pushes events via a Sink
    view_state.rs   # list|board, filter, sort/group → visible groups
    search.rs       # two-tier search
    edit.rs         # Op/Edit → core ops
    doc.rs, draft.rs  # open issue: drafts, conflicts, crash guard
    picker.rs       # picker options
    watcher.rs      # debounced notify
src/lib/
  api.ts            # typed commands and events (the only Tauri import besides plugins)
  app.svelte.ts     # front-end state (runes) and action dispatch
  keymap.ts, nav.ts, fuzzy.ts, format.ts
  components/app/   # sidebar, toolbar, list, board, detail, activity, pickers, palette, search, dialogs
  components/ui/    # shadcn-svelte
```

## Testing

- **Rust state:** `cargo test -p desktop` drives `state/*` with a recording
  `Sink` and no window: loading, edits, debounced saves and coalescing, both
  search tiers, links, deletes, keymap parsing.
- **Front-end:** `vitest` on `keymap`, `nav`, `fuzzy` and `format`;
  `svelte-check` for types.
- No screenshot tests; a manual checklist instead.

## Platform

- **macOS first, portable code.** Built, tested and bundled on macOS; no
  macOS-only APIs outside `cfg`, so Linux and Windows can follow.
- `make dev` runs with hot reload. `make bundle` builds an unsigned `.app` and
  `.dmg`; signing and notarization are not done.
- **Opening:** `lastissue-gui [ROOT]` opens ROOT, else `LASTISSUE_DIR`, else
  the root discovered from the current directory.
  `lastissue://repo/<label>/issue/<id>` (registered by the bundle) switches to
  that repo and opens the issue.
