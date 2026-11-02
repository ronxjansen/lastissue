# lastissue — Specification

A local, plaintext issue tracker written in Rust. Issues are stored as mbox
files, one per issue. A CLI manages them, built to be used equally well by
people and by AI agents.

---

## 1. Principles

- **The file is the truth.** There is no database, index, or cache. Nothing
  derived is stored on disk.
- **Real mail.** Every file is a valid mbox file that opens in `mutt -f`. Standard
  RFC headers are used for their real purpose wherever one fits.
- **Plaintext and greppable.** Headers are raw UTF-8, there is no header
  folding, and bodies are not encoded.
- **Never block, never surprise.** There are no prompts (the importer's TTY
  prompts are the one carve-out, §6.2), auto-commits, auto-expiry, or edits
  outside the issues root, except `init` recording the root in the repo
  registry (§2.7).
- **No concurrency machinery.** There are no locks and no merge driver. Writes are
  atomic (temp file + rename) only so a crash can't corrupt a file.

---

## 2. Storage

### 2.1 Root discovery

The issues root is resolved in this order:

1. `--root <path>` flag, or `--repo <label>` (a registered repo, §2.7; label
   prefixes are accepted like IDs). Passing both is a usage error (exit 1).
   An unknown label gives exit 2 listing the labels; an ambiguous one exit 3
   with the candidates.
2. `LASTISSUE_DIR` env var
3. The nearest directory containing a `.lastissue` marker, found by walking up
   from the current directory. At each level, `issues/.lastissue` is checked
   before `.lastissue`, so an `issues/` root takes precedence over its parent.
   The name `issues` is fixed.
4. The global root (platform data dir via the `directories` crate, e.g.
   `~/.local/share/lastissue`)

`lastissue init` creates a local root in `./issues` if that directory exists,
otherwise in the current directory (`--root` always uses the exact path), and
`lastissue init --global` creates the global root. Every `list` output reports
which root was used. `init` records a local root in the repo registry unless
`--no-register` is given.

### 2.2 Layout

```
<root>/
  .lastissue                              # marker
  fix-typo-in-readme-k3f9x2.mbox          # issue without a project
  new-feature-45aef68/                    # project
    login-crashes-on-empty-7hq2mz.mbox
```

- **Projects** are flat folders. Nesting is not possible.
- **A project's name** is `<slug>-<id>`: the slug is at most 2 words in kebab-case,
  and the id is a short ID generated the same way as an issue's. The id is the
  project's `List-Id`. Projects have no metadata files.
- **Issues** do not have to belong to a project.
- **Issue filenames** are `<slug>-<shortid>.mbox`. The slug is at most 4 words,
  taken from the title at creation, and is **frozen**: it is never renamed when
  the issue is retitled.
- **Identity** is the ID suffix only, never the slug. Prefix resolution matches IDs
  only.
- `project rename` changes the slug and keeps the ID suffix.

### 2.3 IDs

- Message-ID: `<{ULID}@lastissue>`, one for every issue, comment and event.
- Short ID: a lowercase Crockford base32 slice of the ULID, 6–8 characters.
- The CLI accepts any unambiguous prefix. An ambiguous prefix gives exit code 3
  and lists the candidates.
- Project IDs use the same scheme.
- Relations always store **full Message-IDs**, so files and projects can move
  freely.

### 2.4 File model

One mbox file per issue:

- **Message #1 is the issue.** Its headers hold the current state and are edited in
  place.
- **Messages #2…n** are comments and events, appended in order.
- A state change rewrites message #1's headers **and** appends an event
  message (§5.3).
- Every write goes to a temp file, which is then renamed over the original.

### 2.5 Byte-level format

| Concern | Rule |
|---|---|
| mbox variant | **mboxrd**: a body line matching `^>*From ` gets one extra `>` on write, which is removed on read |
| Line endings | LF only |
| Header encoding | Raw UTF-8 (RFC 6532). Never RFC 2047 encoded-words |
| Header folding | None, unless a line exceeds 998 bytes |
| Body | `Content-Type: text/markdown; charset=utf-8`, `Content-Transfer-Encoding: 8bit` |
| Attachments | Not supported in v1. Paste logs inline, and link files by path |
| Dates | RFC 5322 format on disk |

**Canonical header order** (keeps diffs minimal):

```
From <shortid>@lastissue <asctime date>      # mbox separator line
Message-ID
Date
From
Sender
To
Cc
Subject
X-Issue-State
X-Priority-Level
Keywords
Reply-By
Expires
List-Id
In-Reply-To
References
X-Parent
X-Blocked-By
X-Related
X-Issue-Event
X-Edited
X-Edited-By
Sensitivity
Comments
<other X-* headers, in insertion order>
Content-Type
Content-Transfer-Encoding
```

Headers that don't apply are omitted.

### 2.6 Reading and writing

- **Reading:** `mail-parser`.
- **Writing:** a small serializer of our own. It gives full control over
  encoding, header order and mboxrd escaping. `mail-builder` is not used,
  because it emits encoded-words.

### 2.7 Repo registry

A plain file of issue-root paths, so a GUI (or `--repo`) can pick a repo and
load its issues. It holds pointers only, never an index.

- **Location:** `LASTISSUE_REPOS`, else `<platform config dir>/repos` (on
  macOS the same directory as the global root; non-`.mbox` files are ignored
  there).
- **Format:** one canonicalized absolute root path per line, in insertion
  order. `#` comments and blank lines are ignored and kept on rewrite. Adds are
  idempotent; writes are atomic.
- **Label:** the name of the folder containing `issues/`, or the root's own
  folder name when it isn't called `issues`. Labels are not stored. A GUI
  disambiguates collisions with the parent folder; the CLI reports them as
  ambiguous and accepts the root path instead.
- **Global root:** always listed implicitly, last, as `global`. It is never
  written to the file and cannot be removed (exit 4).
- **Missing roots** are listed with `X-Repo-Status: missing` and never pruned
  automatically.
- `init` registers its root (not with `--global`); cloned roots need
  `repo add`, which accepts a root or a directory containing `issues/` and
  fails with exit 4 when there is no `.lastissue` marker.
- `repo remove` only edits the registry; no issue files are touched.
- There are no cross-repo queries.

---

## 3. Data model

### 3.1 Issue fields (headers on message #1)

| Concept | Header | Values / notes |
|---|---|---|
| ID | `Message-ID` | `<{ULID}@lastissue>` |
| Created | `Date` | RFC 5322 |
| Author | `From` | `Name <email>` or `email` |
| Acting agent | `Sender` | Present when written by an agent |
| Title | `Subject` | Free text |
| State | `X-Issue-State` | `backlog` · `todo` · `in-progress` · `in-review` · `done` · `cancelled` · `duplicate` |
| Priority | `X-Priority-Level` | `urgent` · `high` · `medium` · `low`. Absent means no priority |
| Assignee | `To` | **Exactly one** address, or absent |
| Watchers | `Cc` | Addresses. Used only as a filter (no notifications) |
| Labels | `Keywords` | Comma-separated, lowercase, trimmed, deduplicated, free-form |
| Due date | `Reply-By` | RFC 5322 date |
| Stale date | `Expires` | RFC 5322 date. Shown only; nothing auto-closes |
| Project | `List-Id` | `<slug words> <{project-id}.lastissue>`. The folder is the source of truth |
| Duplicate of | `References` | Message-ID of the original. Required when state is `duplicate` |
| Parent | `X-Parent` | One Message-ID |
| Blocked by | `X-Blocked-By` | Comma-separated Message-IDs |
| Related | `X-Related` | Comma-separated Message-IDs |
| Sensitivity | `Sensitivity` | Stored and displayed, **not enforced** |
| Annotation | `Comments` | Short note. Set only via `--header` |
| Edited | `X-Edited`, `X-Edited-By` | Set when the body is edited (§5.2) |
| Anything else | `X-*` | Via `--header K=V` |
| Imported from | `X-Linear-Id`, `X-Linear-Url` | Linear identifier (`ENG-123`) and URL. Set by `import linear` |
| Imported comment | `X-Linear-Comment-Id` | On comments imported from Linear (§6.7) |

### 3.2 States

- Default on create: `backlog`.
- **Open** states: `backlog`, `todo`, `in-progress`, `in-review`.
- **Closed** states: `done`, `cancelled`, `duplicate`.
- On disk the value is always kebab-case. Input is normalized, so `"in progress"`,
  `in_progress` and `In Progress` all become `in-progress`, and `duplicated`
  becomes `duplicate`.

### 3.3 Relations

- Each relation is stored **in one direction only**. The reverse ("blocks",
  "children") is derived by scanning.
- An issue is **ready** when it is in `todo` and every `X-Blocked-By` target is
  in a closed state.

### 3.4 Not used

`Importance`, `Priority` (RFC 2156), `Resent-*`, `Supersedes` and `Archived-At`
are not used. Edits happen in place, and history lives in event messages and git.

---

## 4. Identity

`From:` is resolved in this order:

1. `--from "Name <email>"`
2. `LASTISSUE_FROM`
3. `git config user.name` / `user.email`
4. `$USER@$(hostname)`, with a warning printed to stderr

**Agents** set `--sender` / `LASTISSUE_SENDER`, which writes the `Sender:`
header. The message then reads as "agent acting for the human in `From:`".
Events carry the same `From`/`Sender` as the command that caused them.

Assignee shorthand: `me` resolves through the chain above. Addresses must contain
`@`, and no domain is ever added implicitly.

---

## 5. Comments and events

### 5.1 Comments

- A plain reply: `In-Reply-To` and `References` point to the issue's Message-ID,
  and `Subject: Re: <title>`. Its `Subject` and `To` have **no** effect on state.
- **Flat by default.** `--reply-to <issue>/<n>` sets `In-Reply-To` to that
  comment and extends `References`. `show` renders it in file order with a
  `↳ replying to /n` marker.
- **Addressing:** `<issue>/<n>` (position among the comments, starting at 1) or a
  comment Message-ID prefix.

### 5.2 Edits and deletes

- **Comment edit:** in place, and adds/updates `X-Edited: <date>`. It also sets
  `X-Edited-By` when the editor is not the `From:` author. No event is appended.
- **Issue body edit:** the same rule, applied to message #1.
- **Comment delete:** removes the message and appends an event.
- **Issue delete:** removes the file.
- There is no authorization; anyone can edit anything. `X-Edited-By` keeps it
  honest.

### 5.3 Events

One event per mutating command, listing every changed field:

```
From 9m2c4a@lastissue Sat Oct 31 22:40:00 2026
Message-ID: <01JC…@lastissue>
Date: Sat, 31 Oct 2026 22:40:00 +0100
From: Ron <ron@example.com>
Sender: Claude Code <claude@agent>
Subject: Re: Login crashes on empty password
In-Reply-To: <01JB…@lastissue>
References: <01JB…@lastissue>
X-Issue-Event: update
Content-Type: text/markdown; charset=utf-8
Content-Transfer-Encoding: 8bit

state: todo → in-progress
assignee: (none) → claude@agent
```

`X-Issue-Event` values: `update`, `comment-deleted`, `move`. Events are
informational only; state is **never** derived from them.

---

## 6. CLI

### 6.1 Grammar

The grammar is strictly noun-verb, with no top-level aliases.

```
lastissue init [--global] [--no-register]

lastissue repo list
lastissue repo add [path]                         # default: current directory
lastissue repo remove <label|path>

lastissue project create <name>
lastissue project list
lastissue project rename <project> <name>
lastissue project delete <project> [--force]      # refuses if non-empty without --force

lastissue issue create --title "…" [--project P]
                       [--body "…" | --body-file PATH | -]
                       [--state S] [--priority P] [--label L]...
                       [--assign WHO] [--watch WHO]...
                       [--due DATE] [--expires DATE]
                       [--parent ID] [--blocked-by ID]... [--relate ID]...
                       [--header K=V]...
lastissue issue list   [--project P | --no-project] [--state S]... [--all]
                       [--label L]... [--assignee WHO] [--author WHO]
                       [--watching WHO] [--agent] [--parent ID]
                       [--ready] [--expired] [--search TEXT]
                       [--sort created|updated|priority|due] [--limit N]
                       [--brief]
lastissue issue show   <id>
lastissue issue update <id> [--title …] [--body … | --body-file … | -]
                       [--state S] [--priority P | --no-priority]
                       [--add-label L]... [--remove-label L]...
                       [--assign WHO | --unassign]
                       [--watch WHO]... [--unwatch WHO]...
                       [--due DATE | --no-due] [--expires DATE]
                       [--parent ID | --no-parent]
                       [--blocked-by ID]... [--unblock ID]...
                       [--relate ID]... [--unrelate ID]...
                       [--header K=V]...
lastissue issue close  <id> [--cancel | --duplicate-of ID] [--comment "…"]
lastissue issue reopen <id>                        # → todo
lastissue issue move   <id> --project P | --no-project
lastissue issue delete <id>

lastissue comment create <id> [--body … | --body-file … | -] [--reply-to <id>/<n>]
lastissue comment list   <id>
lastissue comment update <id>/<n> | <comment-id> [--body … | --body-file … | -]
lastissue comment delete <id>/<n> | <comment-id>

lastissue import linear [--api-key KEY]           # or LINEAR_API_KEY
                        [--project NAME|SLUG|UUID | --team KEY] [--all]
                        [--into P | --no-project]
                        [--existing skip|update] [--yes]
```

Global flags: `--root`, `--repo`, `--from`, `--sender`.

`repo list` prints one header block per repo:

```
X-Repo: lastissue
X-Root: /Users/ron/Dev/rxj/lastissue/issues
X-Repo-Status: ok
```

`repo add` / `repo remove` print the block of the repo added or removed, and
`init` prints `X-Root` plus `X-Repo` when it registered the root.

`close` sets `done`, `close --cancel` sets `cancelled`, and
`close --duplicate-of X` sets `duplicate` and `References: X`.

### 6.2 Input

- The body comes from `--body`, `--body-file`, or stdin (`-`, or detected when
  piped).
- `$EDITOR` opens **only** when stdin is a TTY and no body was given.
  Otherwise the command fails fast.
- There are no interactive prompts. Destructive operations on non-empty projects
  require `--force`.
- **Carve-out:** `import linear` prompts (via `inquire`) when stdin and stderr
  are TTYs. Every prompt has a flag; without a TTY a missing value fails fast
  (exit 1) naming the flag, and `--yes` is required.
- Friendly date input is turned into RFC 5322 on disk. Accepted forms:
  `YYYY-MM-DD`, `today`, `tomorrow`, `+Nd`, `+Nw`, and weekday names.
- Priority input aliases are accepted. Output is always canonical.

### 6.3 Output

Everything is mbox / RFC 5322 header blocks. There is no JSON.

- **`issue show`** prints the full mbox content of the issue file.
- **`issue list`** prints one header block per issue (message #1 headers plus
  computed fields, no body), separated by blank lines:
  ```
  Message-ID: <01JBXK…@lastissue>
  X-Short-Id: 7hq2mz
  Subject: Login crashes on empty password
  From: Ron <ron@example.com>
  To: claude@agent
  Keywords: bug, auth
  X-Issue-State: todo
  X-Priority-Level: high
  List-Id: new feature <45aef68.lastissue>
  X-Comment-Count: 3
  X-Path: new-feature-45aef68/login-crashes-on-empty-7hq2mz.mbox
  ```
- **`--brief`** prints one line per issue, e.g. `7hq2mz  todo  high  bug,auth  Login crashes…`.
- **Mutations** print the resulting message(s) as written.
- **Computed fields** (`X-Short-Id`, `X-Path`, `X-Comment-Count`, `X-Updated`,
  `X-Root`) are output only and never written to disk.
- **Truncation:** the default `--limit` is 50, and `--limit 0` means unlimited.
  When results are truncated, a final record says so: `X-Truncated: 50 of 312`.
- Colour is used only when stdout is a TTY.

### 6.4 Listing semantics

- Closed issues are hidden by default and shown with `--all` or an explicit `--state`.
- Default sort: state (`in-progress` → `in-review` → `todo` → `backlog` →
  closed), then priority (`urgent` → `low` → none), then newest first.
- "Updated" is derived as the max `Date` across all messages in the file.
- `--search` is a case-insensitive substring match over the subject, body and all
  comments. No regex, no ranking.
- Filters: `--assignee` matches `To:`, `--author` matches `From:`, `--agent` matches
  any issue with a `Sender:`, and `--watching` matches `Cc:`.
- Performance: list filters read only up to the end of message #1's headers.
  `--search` and `--sort updated` read whole files. There is no index.

### 6.5 Errors

Errors are written to stderr as a header block:

```
Error: ambiguous-id
Candidates: 7hq2mz, 7hq9ab
```

```
Error: invalid-state
Valid: backlog, todo, in-progress, in-review, done, cancelled, duplicate
```

| Exit code | Meaning |
|---|---|
| 0 | OK |
| 1 | Usage error |
| 2 | Not found |
| 3 | Ambiguous ID prefix |
| 4 | Validation error |

### 6.6 Discoverability

The clap `--help` output is the only documentation agents get, so it must
carry the full weight:

- Every subcommand's `long_about` has 1–2 concrete examples.
- Enums use `value_enum`, so every valid value is listed.
- Errors name the valid values or the correct command.

### 6.7 Linear import

`lastissue import linear` imports one Linear project or team. Auth is a
personal API key only (never stored). The workspace is the key's org.

| Linear | lastissue |
|---|---|
| title / description | `Subject` / body, 1:1 |
| createdAt / creator | `Date` / `From` |
| assignee / subscribers | `To` / `Cc` |
| state type | triage, backlog → `backlog`; unstarted → `todo`; started → `in-progress` (`in-review` if the name contains "review"); completed → `done`; canceled → `cancelled`, or `duplicate` + `References` when the duplicate relation's target is in scope |
| priority 1–4 / 0 | `urgent` `high` `medium` `low` / none |
| labels | `Keywords` (team imports add the issue's Linear project name) |
| dueDate | `Reply-By` |
| parent / blocks / related | `X-Parent` / `X-Blocked-By` / `X-Related`, only if the other end is in scope |
| comments | messages #2…n with original author and date; threads → `In-Reply-To`; `X-Linear-Comment-Id` |
| identifier, url | `X-Linear-Id`, `X-Linear-Url` |
| attachments | `[title](url)` links appended to the body |
| history, estimates, cycles, reactions | dropped |

- Everything written carries `Sender: lastissue-import`. No events on fresh create.
- Message-IDs are deterministic ULIDs: timestamp = `createdAt`, random part =
  an 80-bit hash of the Linear UUID. Relations resolve without lookups.
- Destination: a new project named after the Linear project/team (default; an
  existing local project with that slug is reused, so re-runs don't add folders),
  `--into P`, or `--no-project`.
- Re-runs match on Message-ID or `X-Linear-Id`. `--existing skip` (default)
  leaves those files alone. `--existing update`: Linear wins on the fields above;
  local-only headers (custom `X-*`, `Comments`, `Sensitivity`, `Expires`,
  `List-Id`) and local comments are kept; Linear comments are appended or edited
  in place (`X-Edited`); one `X-Issue-Event: update` event lists the changes.
  Nothing is ever deleted locally.
- Streaming: each page is fetched then written (atomic per file); progress goes
  to stderr. On failure what was written is kept and the error says how many were
  done and to re-run with `--existing skip`. HTTP 429 / `RATELIMITED` honors
  `Retry-After`, retrying 3 times.
- Output: a summary block (`X-Imported`, `X-Updated`, `X-Skipped`) followed by
  one `--brief` line per touched issue.
- Hidden `LINEAR_API_URL` env var overrides the endpoint (tests).

---

## 7. Implementation

### 7.1 Crates

| Purpose | Crate |
|---|---|
| CLI | `clap` (derive) |
| Parse | `mail-parser` |
| IDs | `ulid` |
| Dates | `chrono`, plus a hand-rolled friendly-date parser |
| Global dir | `directories` |
| Tests | `proptest`, `assert_cmd`, `insta`, `tempfile`, `mockito` |
| Importer only | `ureq` (blocking HTTP), `serde_json`, `inquire` (TTY prompts) |

No async, no database. `serde_json` is used only by the importer.

### 7.2 Layout

One crate, split into a library (for front-ends such as a GUI) and the CLI
binary. The library exposes `error`, `model`, `store` and `registry`
(`load()`, `add()`, `remove()`, `resolve()`):

```
src/
  lib.rs     # library: pub mod error, model, registry, store
  main.rs    # binary: cli, format, linear, dates
  cli/       # clap definitions + command handlers
  model/     # Issue, Comment, Event, State, Priority, validation, normalization
  store/     # root discovery, paths, slugs, ID resolution, mbox read/write, atomic write
  registry.rs # repo registry (§2.7)
  format/    # header-block output, --brief, error blocks
  linear/    # Linear importer: GraphQL client + JSON → message mapping
  dates.rs   # friendly date parsing
```

### 7.3 Testing

- **Round-trip property tests:** write → parse → compare must give equality. Cases
  must include `From ` and `>From ` lines in bodies, empty bodies, unicode
  everywhere, very long subjects, and many custom `X-*` headers.
- **CLI snapshot tests:** `assert_cmd` + `insta` against a temp root, with
  `LASTISSUE_REPOS` pointed at a temp file so `init` never touches the real
  registry.
- **Manual check:** `mutt -f` on a sample file after every format change.

### 7.4 Things to check early

1. Whether `mail-parser`'s mbox iterator removes mboxrd `>From ` escaping. If
   not, add a small read shim.
2. Whether `mail-parser` keeps raw UTF-8 headers and repeated or custom
   `X-*` headers unchanged.

---

## 8. Out of scope (v1)

JSON output, attachments, locks, a merge driver, auto-commits, auto-expiry,
notifications, a web view, project metadata, nested projects, multiple assignees,
configurable workflows, and a full-text index.
