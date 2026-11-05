# lastissue GUI — Design decisions

A desktop GUI for lastissue, built with Tauri. Its Rust side links the core
library directly; it never shells out to the CLI. Decided in a design review on
2026-11-02. `SPEC.md` stays the source of truth for the file format; §9 lists
the changes this design makes to it.

---

## 1. Toolkit

- **Tauri 2** shell. The Rust crate (`apps/desktop/src-tauri`) holds the GUI's
  state logic; the webview renders it.
- **Svelte 5 + TypeScript** (Vite, plain SPA, no SvelteKit), **Tailwind 4** and
  **shadcn-svelte** (bits-ui) for components, `@lucide/svelte` icons.
- **Milkdown** (ProseMirror + remark, commonmark + gfm presets) for WYSIWYG
  markdown editing of issue bodies and comments; `marked` + DOMPurify render
  read-only comments.
- **pnpm** for the front-end; `@tauri-apps/cli` is a dev dependency (no global
  install). Rust crate versions are pinned once in `[workspace.dependencies]`.

### 1.1 Where logic lives

Rust keeps what needs the index, the disk or core semantics; TypeScript keeps
pure interaction. The rule: whatever makes the app simpler, not "everything in
Rust" for its own sake.

| Rust (`src-tauri/src/state`) | TypeScript (`src/lib`) |
|---|---|
| Index + watcher, view filter/sort/group (50k scale), both search tiers, edits through `core::ops`, title/body drafts and disk conflicts, crash-guard drafts, picker options (need the index and date parsing), `lastissue://` links, the user keymap file | Cursor and multi-selection, keymap defaults and dispatch, the `⌘K` palette list and scoring, save debounce timing, rendering |

The front-end mirrors issue heads (keyed by path relative to the root) from
pushed `index` deltas, and renders the `visible` result Rust pushes after each
view change, disk change or full-text batch. Writes are queued in order on the
TS side so async commands can't reorder.

## 2. Workspace

```
lastissue/
  Cargo.toml            # virtual manifest, resolver = "3"
  crates/
    core/               # package "lastissue-core", [lib] name = "lastissue"
    cli/                # package "cli", [[bin]] name = "lastissue", publish = false
apps/
  desktop/              # Svelte front-end (pnpm)
    src-tauri/          # package "desktop", [[bin]] name = "lastissue-gui", publish = false
```

- The core package can't be named `core` (it would clash with Rust's built-in
  `core` crate). Its lib name is `lastissue`, so code reads `use lastissue::ops`.
- `[workspace.package]`: edition 2024, version, license, rust-version.
- `[workspace.dependencies]`: every version pinned once; crates use
  `{ workspace = true }`.
- `[workspace.lints]`: `unsafe_code = "forbid"`, clippy `pedantic = "warn"`
  (with targeted allows); every crate sets `[lints] workspace = true`.
- CLI snapshot tests move to `crates/cli/tests`.

### 2.1 Core (`lastissue-core`)

Modules: `model`, `store` (mbox), `registry`, `ops`, `query`, `search`,
`dates`, `identity`, `source`. Feature `linear` (ureq, serde_json) gates the
importer.

- **No clap, no TTY, no printing.** Functions take plain structs and return
  typed values and `Result<T, lastissue::Error>`.
- `clap::ValueEnum` leaves `model`. The CLI defines mirror enums with `From`
  impls.
- Core types are `Send + Sync`, so the GUI can do I/O on background executors.
- **`ops`** holds all mutations (create, update, close, reopen, move, delete,
  comment create/edit/delete) including event generation and coalescing (§9).
  The CLI and GUI get identical semantics: dragging a card writes exactly what
  `issue update --state` writes.
- **`query`** holds filtering, sorting, grouping and the filter-token parser
  (§6).
- **`identity`** holds `From:` resolution (moved from `cli/common.rs`).
- **`source`** holds the `IssueSource` trait. v1 implementation: `FsScan`
  (full head scan, rayon). A later `CachedSource` (SQLite in the platform cache
  dir, keyed by path + mtime, purely derived) can slot in without GUI changes.

### 2.2 CLI

A thin adapter: clap args → core structs → `format`. Output, exit codes and
behaviour stay as specified in `SPEC.md`.

## 3. Data and sync

- On repo open, all issue **heads** (message #1 headers plus stats) are loaded
  into memory as compact `IssueSummary` values: enums for state and priority,
  interned labels and addresses, no bodies. Bodies and comments load lazily
  when an issue is opened.
- This is an in-memory cache only. Nothing derived is written to the issues
  root.
- **Scale target:** ~50k issues per repo. Expected today: low thousands. A
  `criterion` benchmark in core generates 10k and 100k synthetic repos and
  records scan time and memory.
- Lists and board columns are virtualized (fixed row heights), so render cost doesn't grow
  with issue count.
- **Watcher:** `notify` (debounced) on the open repo's issues root. Changed
  `.mbox` files and project folders are re-read and diffed into the model.
- **Writes** go through `core::ops` (atomic temp file + rename). The GUI
  updates optimistically; its own writes are recognised by path + mtime, so
  they don't cause flicker.
- **Conflicts:** last write wins (no concurrency machinery, as in `SPEC.md`).
  If the issue being edited changes on disk, a "changed on disk — reload /
  overwrite" banner appears. The editor reloads via `reload_doc`.
- One repo per window, with a switcher fed by the registry.

## 4. Identity

1. `git config user.name` / `user.email`, run **in the repo's directory**
   (respects per-repo identities).
2. An app-level override in GUI settings
   (`~/Library/Application Support/lastissue/gui.toml`).
3. If neither exists, a first-run prompt. Never `$USER@host`.

The GUI never sets `Sender:`; it is always a human. "My issues", `@me` and
"assign to me" (`i`) use this identity.

## 5. v1 scope

```
┌───────────┬──────────────────────────────────────────┬─────────────────────┐
│ Sidebar   │ Main view                                │ Detail pane         │
│ ▸ Repo ▾  │  [List | Board]  filter bar  sort/group  │ title, body (md)    │
│ My issues │                                          │ props: state, prio, │
│ All       │  virtualized list  ─or─  kanban columns   │ assignee, labels,   │
│ Projects  │  grouped by state (or priority)          │ due, project,       │
│  • proj-a │                                          │ relations           │
│ No project│                                          │ activity + composer │
└───────────┴──────────────────────────────────────────┴─────────────────────┘
                 ⌘K command palette over everything
```

**In:**
- Repo switcher.
- Projects: list, create, rename.
- List and board views of the same filtered set.
- Filter bar; sort and group (by state or priority).
- Issue create (modal, `c`).
- Issue detail with inline title and body editing and Linear-style property
  pickers.
- Activity timeline merging comments and compact, collapsible events.
- Comment create, edit and delete.
- Search (§6).
- `⌘K` palette and full keyboard control (§7).
- Multi-select bulk edit.

**Deferred:**
- Linear import from the GUI.
- Rich relations editing (beyond simple pickers).
- Saved views.
- Inbox / notifications (not in the model).
- Attachments (out of scope in `SPEC.md`).

### 5.1 Kanban

- Columns are states. Dragging a card between columns runs `ops::update` with
  the new state.
- **No manual ordering.** Cards sort by the active sort key; there is no rank
  header. Dragging within a column does nothing. When grouped by priority,
  dragging between groups changes priority.
- Keyboard equivalent of dragging: `⇧h` / `⇧l` move a card to the
  previous/next state.

### 5.2 Editing and saving

- Every edit saves on a **~300 ms debounce**, title and body alike. Coalescing
  (§9) keeps the file to one event per burst.
- Property pickers commit immediately.
- The create modal writes the file only on submit (`⌘Enter`); no half-made
  files.
- Crash guard: unsaved drafts are mirrored to the platform cache dir
  (`drafts/<message-id>.md`) and offered back on the next open.

### 5.3 Markdown

- **Issue body:** a Milkdown editor, read-only until `e` or a click makes it
  editable in place.
- **Comments:** rendered read-only with `marked` (sanitized). Editing a comment
  swaps in a Milkdown editor for that comment only; the composer is one too.
- **No silent rewrites:** a WYSIWYG editor re-serializes markdown, so the
  editor records its own serialization of the loaded text as a baseline and
  reports nothing until the user changes the document. A change that
  serializes back to the baseline reports the original text byte for byte.
  Opening or viewing an agent-written issue never writes it; a real edit may
  normalize formatting in the edited field (e.g. `*` bullets become `-`).
- Core writes keep untouched messages verbatim, as before.
- **Theme:** shadcn CSS variables in `app.css`, light and dark following the
  system (`mode-watcher`); markdown styles use the same tokens.

## 6. Search and filter tokens

- `/` opens one search box with two tiers:
  1. **Instant**, in memory, on every keystroke: title, short ID, labels,
     assignee, project.
  2. **Full text**, after ~150 ms idle: core search over bodies and comments on
     a background executor. Results stream in with snippets showing where they
     matched. Each new keystroke cancels the previous scan (generation
     counter).
- **Filter tokens** are parsed in `core::query` and shared by the search box,
  the filter bar and the CLI (`issue list --filter`):
  `state:todo`, `p:high`, `label:bug`, `@me`, `assignee:bob`, `project:auth`,
  `is:ready`, `is:blocked`, `due:<7d`. Remaining text is free text.
  `⌘Enter` applies a search as the current view's filter.
- **Ordering (GUI only):** tiered, not ranked. Title match → label/ID match →
  body → comment, then by updated date (newest first).

## 7. Keyboard

One action registry (`src/lib/keymap.ts`); every action appears in `⌘K` with
its binding. Contexts, least to most specific: `global`, `workspace` (no text
input or overlay focused), then `issues` + `list`/`board`, or `detail`. The
more specific binding wins.

| Context | Keys |
|---|---|
| Global | `⌘K` palette · `/` search · `c` create · `g i`/`g p`/`g b`/`g l` go to my issues, projects, board, list · `⌘1..9` switch repo · `⌘,` settings |
| List/Board | `j`/`k` (and arrows) move · `h`/`l` move between columns · `Enter`/`o` open · `x` select · `Esc` clear |
| Issue | `s` state · `p` priority · `a` assignee · `i` assign to me · `l` labels · `d` due · `m` move to project · `r` rename · `e` edit body · `⌘Enter` submit/close · `⌘⌫` delete (with confirm) |
| Board | `⇧h`/`⇧l` move card to previous/next state |
| Pickers | type to filter · `↑↓` · `Enter` · `Esc` (one fuzzy-list component, shared with `⌘K`) |

- Defaults live in TS. User overrides go in `gui-keymap.json` in the config
  dir, a flat `{ "context": { "keys": "action" | null } }` (`null` unbinds),
  read and watched by Rust and applied live. `⌘,` creates and opens it.
  Problems show in a toast and the palette footer.
- Single-letter bindings fire only when no text input has focus.
- No app-level vim mode.

## 8. App structure and testing

```
apps/desktop/
  src-tauri/src/
    main.rs           # Tauri setup: plugins (deep-link, clipboard, dialog), workspace, keymap watcher
    commands.rs       # #[tauri::command]s, thin over state/workspace
    dto.rs            # serde mirrors of core and state types (core stays serde-free)
    keymap.rs         # gui-keymap.json: parse, load, watch
    link.rs           # lastissue:// parsing
    state/
      workspace.rs    #   repos, open repo (index, watcher), view, doc, searches; pushes events via a Sink
      view_state.rs   #   list|board, filter tokens, sort/group → visible groups
      search.rs       #   two-tier search
      edit.rs         #   Op/Edit → core ops
      doc.rs, draft.rs#   open issue: drafts, conflicts, crash guard
      picker.rs       #   picker options
      watcher.rs      #   debounced notify
  src/
    lib/api.ts        # typed commands + events (the only Tauri import besides plugins)
    lib/app.svelte.ts # front-end state (runes) and action dispatch
    lib/keymap.ts, nav.ts, fuzzy.ts, format.ts
    lib/components/app/   # sidebar, toolbar, list, board, detail, activity, pickers, palette, search, dialogs
    lib/components/ui/    # shadcn-svelte components
```

Testing:
1. **Core:** `ops` round-trip and property tests, coalescing properties, the
   token parser, and the CLI `insta` snapshots.
2. **App state (Rust):** `cargo test -p desktop` drives `state/*` directly with
   a recording `Sink` and no window (load, edits, debounced doc saves and
   coalescing, both search tiers, links, deletes, keymap parsing).
3. **Front-end logic:** `vitest` on `keymap`, `nav`, `fuzzy`, `format`;
   `svelte-check` for types.
4. **No screenshot tests in v1.** A manual checklist instead.

Fixtures: the criterion
generator for 10k and 100k.

## 9. Changes to SPEC.md (applied in M2)

**§5.2 / §5.3: event coalescing.** Events stay in the file as an audit log;
state is still never derived from them.

- **Burst condition:** if the file's **last message** is an
  `X-Issue-Event: update` with the same `From` **and** `Sender`, and its
  `Date` is less than **15 minutes** old, the new changes merge into it.
  Otherwise a new event is appended.
- **Merging:** a field already in the event keeps its original "from" value
  and gets the new "to". If "to" equals "from", the line is dropped; an event
  left with no lines is removed.
- The event's `Date` stays at the burst start.
- A comment, or another actor's change, ends the burst.
- `move` and `comment-deleted` events never coalesce.
- No per-repo configuration.

**§5.2: body edits** append (or coalesce into) an update event with the line
`body: edited` (no diff). `X-Edited` is still set.

**§6.1 / §6.3:**
- `issue show --no-events` hides event messages.
- `issue list --filter "<tokens>"` accepts the token grammar from §6.

**§7.2:** the layout becomes the workspace in §2.

## 10. Platform and distribution

- **macOS first, portable code.** Built, tested and bundled on macOS; no
  macOS-only APIs outside `cfg`, so Linux/Windows can be turned on later.
- `make dev` (hot reload), `make bundle` (`tauri build`: unsigned `.app` +
  `.dmg`). Signing and notarization come with M6.
- `lastissue-gui <path>` and the `lastissue://repo/<label>/issue/<id>` URL
  scheme (`tauri-plugin-deep-link`, registered by the bundle).

## 11. Milestones

| # | Milestone | Done when |
|---|---|---|
| M0 | Workspace split: `crates/{core,cli}` + the GUI crate, workspace deps and lints, `ValueEnum` mirrors, empty window | `cargo test` green with **zero snapshot changes**; `lastissue` binary unchanged |
| M1 | Extract core `ops`, `query`, `search`, `identity`; `IssueSource` + `FsScan` | CLI is a thin adapter; snapshots unchanged; core has no clap/TTY deps |
| M2 | Core behaviour changes from §9 | SPEC.md updated; coalescing property tests; snapshots updated intentionally |
| M3 | GUI read-only: repo switcher, sidebar, list, board, filters, detail, watcher | Opens this repo; CLI edits by agents appear live |
| M4 | GUI editing: pickers, drag and drop, `⇧h/⇧l`, title/body editing with debounced save, create modal, comments, drafts guard, disk-change banner | Usable day to day on this repo |
| M5 | Keyboard and search polish: full keymap, `⌘K`, keymap override, two-tier search with tokens, multi-select bulk edit | A full triage session with no mouse |
| M6 | Hardening: 10k/100k benchmarks, activity-feed profiling, `.app` bundle, URL scheme | Perf numbers recorded; installable `.app` |

M0–M2 land before any real GUI work. Each milestone is tracked as a lastissue
issue in the `gui` project.
