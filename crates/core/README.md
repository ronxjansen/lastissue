# lastissue-core

The library behind every front-end (lib name `lastissue`, so code reads
`use lastissue::ops`). It owns the file format, the data model and every
operation on issues. 

## Principles

- **The file is the source of truth.** No database, index or cache on disk. Nothing
  derived is ever stored.
- **Real mail.** Every file is valid mbox and opens in `mutt -f`. Standard RFC
  headers keep their real meaning wherever one fits.
- **Plaintext and greppable.** Raw UTF-8 headers, no folding, no body
  encoding.
- **No concurrency machinery.** No locks, no merge driver. Writes are atomic
  (temp file, then rename) only so a crash cannot corrupt a file.

## Storage

### Root discovery

`store` resolves the issues root, first match wins:

1. An explicit root from the caller (a path, or a registered repo's label;
   label prefixes resolve like IDs).
2. `LASTISSUE_DIR`.
3. The nearest `.lastissue` marker, walking up from the current directory. At
   each level `issues/.lastissue` beats `.lastissue`, so an `issues/` root wins
   over its parent. The name `issues` is fixed.
4. The global root: the platform data dir (`directories` crate), e.g.
   `~/.local/share/lastissue`.

`init` creates a local root in `./issues` if that folder exists, else in the
current directory; an explicit path is used as given. The global root is
created on request.

### Layout

```
<root>/
  .lastissue                              # marker
  fix-typo-in-readme-k3f9x2.mbox          # issue without a project
  new-feature-45aef68/                    # project
    login-crashes-on-empty-7hq2mz.mbox
```

- **Projects** are flat folders named `<slug>-<id>`: a slug of at most 2
  kebab-case words and a short ID made like an issue's. The ID is the project's
  `List-Id`. No nesting, no metadata files. Renaming changes the slug and
  keeps the ID.
- **Issues** need not belong to a project. Filenames are
  `<slug>-<shortid>.mbox`, with a slug of at most 4 words taken from the title
  at creation. The slug is **frozen**: retitling never renames the file.
- **Identity is the ID suffix**, never the slug. Prefix resolution matches IDs
  only.

### IDs

- Message-ID: `<{ULID}@lastissue>`, for every issue, comment and event.
- Short ID: a 6–8 character lowercase Crockford base32 slice of the ULID.
  Projects use the same scheme.
- Any unambiguous prefix resolves. An ambiguous one is an `ambiguous-id`
  error listing the candidates.
- Relations store **full Message-IDs**, so files and projects move freely.

### File model

One mbox file per issue:

- **Message #1 is the issue.** Its headers hold the current state and are
  edited in place.
- **Messages #2…n** are comments and events, appended in order.
- A state change rewrites message #1's headers **and** appends an event.
- Every write goes to a temp file renamed over the original.

### Byte-level format

| Concern | Rule |
|---|---|
| mbox variant | **mboxrd**: a body line matching `^>*From ` gains one `>` on write and loses it on read |
| Line endings | LF only |
| Header encoding | Raw UTF-8 (RFC 6532), never RFC 2047 encoded-words |
| Header folding | None below 998 bytes |
| Body | `Content-Type: text/markdown; charset=utf-8`, `Content-Transfer-Encoding: 8bit` |
| Attachments | None in v1. Paste logs inline; link files by path |
| Dates | RFC 5322 |

Headers are written in a fixed order to keep diffs small, and omitted when
they don't apply:

```
From <shortid>@lastissue <asctime date>      # mbox separator line
Message-ID, Date, From, Sender, To, Cc, Subject,
X-Issue-State, X-Priority-Level, Keywords, Reply-By, Expires, List-Id,
In-Reply-To, References, X-Parent, X-Blocked-By, X-Related,
X-Issue-Event, X-Edited, X-Edited-By, Sensitivity, Comments,
<other X-* headers, in insertion order>,
Content-Type, Content-Transfer-Encoding
```

`mail-parser` reads. Our own serializer writes, for full control of encoding,
order and escaping; `mail-builder` emits encoded-words, so it is not used.

### Repo registry

A plain file of issue-root paths that lets a front-end pick a repo by label.
Pointers only, never an index.

- **Location:** `LASTISSUE_REPOS`, else `<platform config dir>/repos`. On
  macOS that is the global root's directory; non-`.mbox` files there are
  ignored.
- **Format:** one canonical absolute root path per line, in insertion order.
  `#` comments and blank lines survive rewrites. Adding is idempotent; writes
  are atomic.
- **Label:** the folder holding `issues/`, or the root's own folder name when
  it isn't called `issues`. Labels are derived, never stored. Two roots can
  share a label; front-ends decide how to tell them apart.
- **Global root:** always listed, last, as `global`. Never written to the file
  and cannot be removed (validation error).
- **Missing roots** stay listed with status `missing` and are never pruned.
- Adding accepts a root or a directory holding `issues/`, and fails without a
  `.lastissue` marker. Removing edits only the registry, never issue files.
- No cross-repo queries.

## Data model

### Issue fields (headers on message #1)

| Concept | Header | Values |
|---|---|---|
| ID | `Message-ID` | `<{ULID}@lastissue>` |
| Created | `Date` | RFC 5322 |
| Author | `From` | `Name <email>` or `email` |
| Acting agent | `Sender` | Present when an agent wrote it |
| Title | `Subject` | Free text |
| State | `X-Issue-State` | `backlog` · `todo` · `in-progress` · `in-review` · `done` · `cancelled` · `duplicate` |
| Priority | `X-Priority-Level` | `urgent` · `high` · `medium` · `low`; absent means none |
| Assignee | `To` | **Exactly one** address, or absent |
| Watchers | `Cc` | Addresses; a filter only, no notifications |
| Labels | `Keywords` | Comma-separated, lowercase, trimmed, deduplicated |
| Due | `Reply-By` | RFC 5322 date |
| Stale | `Expires` | RFC 5322 date; shown only, nothing auto-closes |
| Project | `List-Id` | `<slug words> <{project-id}.lastissue>`; the folder is the truth |
| Duplicate of | `References` | Original's Message-ID; required when state is `duplicate` |
| Parent | `X-Parent` | One Message-ID |
| Blocked by | `X-Blocked-By` | Comma-separated Message-IDs |
| Related | `X-Related` | Comma-separated Message-IDs |
| Sensitivity | `Sensitivity` | Stored and shown, **not enforced** |
| Annotation | `Comments` | Short note, custom header only |
| Edited | `X-Edited`, `X-Edited-By` | See edits below |
| Anything else | `X-*` | Custom header |
| Imported from | `X-Linear-Id`, `X-Linear-Url` | Linear identifier (`ENG-123`) and URL |
| Imported comment | `X-Linear-Comment-Id` | On comments from Linear |

Not used: `Importance`, `Priority` (RFC 2156), `Resent-*`, `Supersedes`,
`Archived-At`. Edits happen in place; history lives in events and git.

### States

- New issues start in `backlog`.
- **Open:** `backlog`, `todo`, `in-progress`, `in-review`. **Closed:** `done`,
  `cancelled`, `duplicate`.
- Reopening sets `todo`.

### Relations

- Each relation is stored **one way only**. The reverse ("blocks",
  "children") comes from scanning.
- An issue is **ready** when it is `todo` and every `X-Blocked-By` target is
  closed. It is **blocked** when at least one target is open.

### Input forms

Core normalizes everything a user types, so every front-end accepts the same
forms. Output is always canonical.

- **States:** kebab-case on disk. `"in progress"`, `in_progress` and
  `In Progress` all become `in-progress`; `duplicated` becomes `duplicate`.
- **Priorities:** case-insensitive, with aliases `p0`–`p3`, `u`/`h`/`m`/`l`,
  `critical`, `hi`, `med`, `normal`, `lo`, `minor`.
- **Dates:** `YYYY-MM-DD`, `today`, `tomorrow`, `+Nd`, `+Nw` or a weekday
  name, stored as RFC 5322.
- **Addresses:** must contain `@`; no domain is ever added. `me` resolves to
  the acting identity.
- **Custom headers:** `K=V`, where K is `Comments`, `Sensitivity` or a
  non-reserved `X-*` name. A reserved name is an error naming the proper
  input. An empty value removes the header.

## Identity

`From:` is who acts; `Sender:` is the agent acting for them, so a message
reads as "agent acting for the human in `From:`". Events carry the same
`From`/`Sender` as the operation that caused them.

`Identity` resolves `From:` in this order: an explicit address, then
`git config user.name` / `user.email` (optionally run in a given directory, to
respect per-repo identities), then `$USER@host`. Front-ends may disable the
last step. Blank values count as absent.

## Comments and events

### Comments

- A reply whose `In-Reply-To` and `References` point at the issue, with
  `Subject: Re: <title>`. Its `Subject` and `To` never change state.
- **Flat by default.** Replying to comment *n* sets `In-Reply-To` to it and
  extends `References`.
- **Addressing:** `<issue>/<n>` (1-based position among comments) or a
  comment Message-ID prefix.

### Edits and deletes

- **Comment edit:** in place. Sets `X-Edited: <date>`, plus `X-Edited-By` when
  the editor isn't the author. No event.
- **Issue body edit:** the same rule on message #1, plus an update event line
  `body: edited` (no diff).
- **Comment delete:** removes the message and appends an event.
- **Issue delete:** removes the file.
- No authorization: anyone can edit anything. `X-Edited-By` keeps it honest.

### Events

Events stay in the file as an audit log. They are informational; state is
**never** derived from them. An update lists each changed field on its own
line:

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

`X-Issue-Event` is `update`, `comment-deleted` or `move`.

**Coalescing** turns quick edits by one actor into one event:

- If the file's **last message** is an `update` event with the same `From`
  and `Sender` (both absent counts as equal) and a `Date` under **15 minutes**
  old, the new changes merge into it. Otherwise a new event is appended. The
  window runs from the burst's start, so a burst lasts at most 15 minutes.
- A field already listed keeps its old "from" and takes the new "to". A new
  field becomes a new last line. A line whose "to" equals its "from" is
  dropped, and an event left empty is removed. `body: edited` appears once at
  most.
- The event keeps the burst's start `Date`; its `Subject` follows the current
  title.
- A comment or another actor's change ends the burst.
- `move`, `comment-deleted` and Linear re-import events never coalesce.

## Querying

`query::ListQuery` holds the criteria; `query::list` streams matching issues
from disk and `query::memory` runs the same filters over in-memory heads.

- **Closed issues** are hidden unless `all` is set or `states` names them. A
  search includes closed issues unless `states` narrows it.
- **Order:** path order by default (root-level files, then each project
  folder, filenames sorted within). Sorting is opt-in: `state`, `created`,
  `updated`, `priority` or `due`. `state` orders `in-progress` → `in-review` →
  `todo` → `backlog` → closed, then priority (`urgent` → `low` → none), then
  newest first.
- **Updated** is the latest `Date` among all messages in the file.
- **Search terms** must all match. Each is a case-insensitive literal, or a
  regex when `regex` is set (`regex` crate, ripgrep syntax, still
  case-insensitive). Terms match line by line over message #1's `Subject` and
  the bodies of non-event messages; headers are not searched. No ranking.
  Each hit reports its location (`subject`, `body` or `/n`) and the line.
- **Field filters:** `assignee` matches `To:`, `author` `From:`, `watching`
  `Cc:`; `agent` matches any issue with a `Sender:`. Plus `parent`, `labels`
  (all must match), `priorities`, due bounds, `ready`, `blocked`, `expired`
  and project scope.
- **Ready** turns each blocker's Message-ID into a short-ID filename suffix,
  so it needs no scan of other issues.

### Filter tokens

`query::tokens` parses one grammar for every front-end. Words split on
whitespace; double quotes group words and are removed (`label:"needs review"`).

| Token | Meaning |
|---|---|
| `state:S` | State (normalized). Repeats are alternatives; closed states show |
| `p:P`, `priority:P` | Priority (aliases work), or `none`. Repeats are alternatives |
| `label:L` | Has label L. Repeats must all match |
| `@WHO`, `assignee:WHO` | Assignee (`@me` works) |
| `project:P` | Project by ID prefix or slug (exact, else a unique prefix) |
| `is:ready`, `is:blocked` | As in Relations |
| `due:<DATE`, `due:>DATE` | Due strictly before / after DATE; `7d` and `2w` mean `+7d` and `+2w`. Undated issues never match |

Keys are case-insensitive. Any other word (`auth.rs:41` included) or quoted
phrase is a search term. Tokens fold into a `ListQuery`: list criteria
(states, priorities, labels, terms) add up, and a single-valued one
(assignee, project, a due bound) replaces what was there. Bad values are
`invalid-state` or `invalid-priority` errors; an unknown `is:` value, an
empty value or a `due:` without `<`/`>` is `invalid-filter`.

### Performance

No index or cache on disk. Files are scanned in parallel in ordered batches,
in path order, and the scan stops once the page (offset + limit) is full.
Sorting or an unlimited page scans everything. Filters read only up to the end
of message #1's headers. Search first runs a byte-level regex over the raw
file and parses only files where every term occurs. Comment counts and
updated dates are computed only for returned rows, unless sorting by updated.

## Linear import

Feature `linear`. Imports one Linear project or team using a personal API key,
which is never stored. The workspace is the key's org.

| Linear | lastissue |
|---|---|
| title / description | `Subject` / body |
| createdAt / creator | `Date` / `From` |
| assignee / subscribers | `To` / `Cc` |
| state type | triage, backlog → `backlog`; unstarted → `todo`; started → `in-progress` (`in-review` if the name contains "review"); completed → `done`; canceled → `cancelled`, or `duplicate` + `References` when the duplicate's target is in scope |
| priority 1–4 / 0 | `urgent` `high` `medium` `low` / none |
| labels | `Keywords` (team imports add the Linear project name) |
| dueDate | `Reply-By` |
| parent / blocks / related | `X-Parent` / `X-Blocked-By` / `X-Related`, when the other end is in scope |
| comments | messages #2…n with original author and date; threads → `In-Reply-To`; `X-Linear-Comment-Id` |
| identifier, url | `X-Linear-Id`, `X-Linear-Url` |
| attachments | `[title](url)` links appended to the body |
| history, estimates, cycles, reactions | dropped |

- Everything written carries `Sender: lastissue-import`. A fresh create writes
  no events.
- Message-IDs are deterministic ULIDs: timestamp from `createdAt`, random part
  an 80-bit hash of the Linear UUID, so relations resolve without lookups.
- **Destination:** by default a project named after the Linear project or
  team (an existing local project with that slug is reused, so re-runs add no
  folders); or a given project; or none.
- **Re-runs** match on Message-ID or `X-Linear-Id`. **Skip** (default) leaves
  those files alone. **Update** lets Linear win on the fields above, keeps
  local-only headers (custom `X-*`, `Comments`, `Sensitivity`, `Expires`,
  `List-Id`) and local comments, appends or edits Linear comments in place,
  and writes one uncoalesced update event. Nothing local is ever deleted.
- **Streaming:** each page is fetched, then written file by file. On failure
  the written files stay, and the error says how many were done and to re-run
  with skip. HTTP 429 / `RATELIMITED` honours `Retry-After`, up to 3 retries.
- `LINEAR_API_URL` overrides the endpoint (tests).

## Errors

Every `lastissue::Error` has a kind (e.g. `ambiguous-id`, `invalid-state`) and
a fixed exit code, and names the valid values or the right input.

| Code | Meaning |
|---|---|
| 0 | OK |
| 1 | Usage error |
| 2 | Not found |
| 3 | Ambiguous ID prefix |
| 4 | Validation error |

## Implementation

- Functions take plain structs and return typed values or
  `Result<T, lastissue::Error>`. No TTY, no printing, no async, no database.
- No serde. `clap::ValueEnum` on `State`, `Priority` and
  `linear::import::Existing` sits behind feature `clap`.
- Types are `Send + Sync`, so front-ends can do I/O off their UI thread.
- `ops` is the single write path, so every front-end writes the same bytes
  for the same change.

| Module | Holds |
|---|---|
| `model` | `State`, `Priority`, labels, addresses, messages, event lines, the activity timeline |
| `store` | root discovery, paths, slugs, ID resolution, atomic writes; `store::mbox` parses and serializes |
| `registry` | the repo registry |
| `identity` | `From:` / `Sender:` resolution |
| `ops` | every mutation: issues, comments, projects; event generation and coalescing |
| `query` | filters, sorting, paging and the token parser over files; `query::memory` over in-memory heads, plus grouping by state or priority |
| `search` | search terms: byte prefilter, then line matching for hit context |
| `source` | `load`: one file's head (`IssueSummary`) and stats |
| `index` | the in-memory issue set for one root |
| `dates` | friendly date parsing |
| `error` | `lastissue::Error`, kinds and exit codes |
| `linear` | the Linear importer (feature `linear`) |

Dependencies: `mail-parser`, `ulid`, `chrono`, `rayon`, `regex`,
`directories`; `ureq` and `serde_json` for the importer only.

### In-memory index

For front-ends that hold every issue in memory:

- `index::scan` loads every issue **head** (message #1 headers plus stats)
  into an `Index` of compact `IssueSummary` values: enums for state and
  priority, no bodies. Bodies and comments load when an issue is opened.
- I/O and state are split: `index::scan` and `reload` read the disk, and
  `Index::apply` folds the result in without I/O, returning a `Delta`. A
  re-read with the same content and mtime is no change, so a front-end's own
  writes don't come back as changes.
- Memory only; nothing derived reaches the issues root.
- **Scale target:** ~50k issues per repo. `benches/scale.rs` measures scan
  time and memory on 10k and 100k synthetic repos (`make bench`;
  `LASTISSUE_BENCH_DIR` sets where stores go).

### Testing

- Round-trip property tests (`proptest`): write, parse, compare. Cases cover
  `From ` and `>From ` body lines, empty bodies, unicode everywhere, very long
  subjects and many custom headers.
- `ops`, coalescing and token-parser tests; `mockito` for the importer.
- After any format change, open a sample file with `mutt -f`.
