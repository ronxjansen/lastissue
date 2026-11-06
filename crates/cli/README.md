# lastissue CLI

The `lastissue` binary: clap arguments in, core calls, text out. Built for
people and AI agents alike. The file format and what each operation means
live in [`crates/core`](../core/README.md); this file covers only the command
line.

## Grammar

Strictly noun-verb, no top-level aliases.

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
                       [--ready] [--expired] [--search TEXT]... [--regex]
                       [--filter "TOKENS"]...
                       [--sort state|created|updated|priority|due]
                       [--limit N] [--offset N] [--brief]
lastissue issue show   <id> [--no-events]
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
lastissue issue reopen <id>
lastissue issue move   <id> --project P | --no-project
lastissue issue delete <id>

lastissue comment create <id> [--body … | --body-file … | -] [--reply-to <id>/<n>]
lastissue comment list   <id>
lastissue comment update <id>/<n> | <comment-id> [--body … | --body-file … | -]
lastissue comment delete <id>/<n> | <comment-id>

lastissue import linear [--api-key KEY]
                        [--project NAME|SLUG|UUID | --team KEY] [--all]
                        [--into P | --no-project]
                        [--existing skip|update] [--yes]
```

Global flags:

| Flag | Env | Does |
|---|---|---|
| `--root PATH` | `LASTISSUE_DIR` | Issues root; skips discovery |
| `--repo LABEL` | | A registered repo; unknown is exit 2, ambiguous exit 3. Can't combine with `--root` |
| `--from "Name <email>"` | `LASTISSUE_FROM` | Acting identity; else git config, else `$USER@host` with a warning on stderr |
| `--sender "Agent <a@host>"` | `LASTISSUE_SENDER` | Agents set this; writes `Sender:` |

- `close` sets `done`, `--cancel` sets `cancelled`, and `--duplicate-of X`
  sets `duplicate` with `References: X`.
- `init` registers its root unless `--no-register` or `--global`. `repo add`
  is for cloned roots.
- Every `--filter` takes the core token grammar; `--search` terms and filter
  tokens combine.

## Input

- The body comes from `--body`, `--body-file`, or stdin (`-`, or detected when
  piped).
- `$EDITOR` opens **only** when stdin is a TTY and no body was given;
  otherwise the command fails fast.
- No prompts. Deleting a non-empty project needs `--force`.
- **One exception:** `import linear` prompts (via `inquire`) when stdin and
  stderr are TTYs. Every prompt has a flag. Without a TTY a missing value
  fails fast (exit 1) naming the flag, and `--yes` is required.

## Output

Everything is mbox or RFC 5322 header blocks. No JSON. Colour only when
stdout is a TTY.

- **`issue show`** prints the whole file. `--no-events` leaves out event
  messages.
- **`issue list`** prints one header block per issue (message #1 headers plus
  computed fields, no body), blank-line separated:
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
  Every list also reports which root it used (`X-Root`).
- **`--brief`**: one line per issue, `7hq2mz  todo  high  bug,auth  Login crashes…`.
- **Paging:** `--limit` defaults to 50; `0` is unlimited. `--offset N` skips
  N. When more exist, a final record says how to continue:
  `X-Next: --offset 50`. No total count.
- **Search hits:** with `--search`, each block says where it matched:
  ```
  X-Match: subject: Login crashes on empty password
  X-Match: /3: …so the empty password path hits unwrap() in auth.rs:41…
  X-Match-Count: 5
  ```
  The location is `subject`, `body` or `/n` (comment n). The snippet is about
  120 characters centred on the hit, on one line, with `…` at cuts. At most 3
  `X-Match` lines per issue but at least one per term; `X-Match-Count` counts
  every matching line. `--brief` prints none.
- **Mutations** print the messages as written.
- **Repos:** `repo list` prints one block per repo; `repo add` / `remove`
  print the affected one; `init` prints `X-Root`, plus `X-Repo` when it
  registered.
  ```
  X-Repo: lastissue
  X-Root: /Users/ron/Dev/rxj/lastissue/issues
  X-Repo-Status: ok
  ```
- Computed fields (`X-Short-Id`, `X-Path`, `X-Comment-Count`, `X-Updated`,
  `X-Root`, `X-Repo`, `X-Repo-Status`, `X-Match`, `X-Match-Count`, `X-Next`)
  are output only, never written.

## Errors

A header block on stderr, with the core error kind and exit code:

```
Error: ambiguous-id
Candidates: 7hq2mz, 7hq9ab
```

```
Error: invalid-state
Valid: backlog, todo, in-progress, in-review, done, cancelled, duplicate
```

## Help

`--help` is the only documentation an agent gets, so it carries the full
load: every subcommand's `long_about` has one or two concrete examples, enums
use `value_enum` so every valid value is listed, and errors name the valid
values or the right command.

## Linear import

`import linear` drives the core importer page by page.

- **Auth:** `--api-key` or `LINEAR_API_KEY`.
- **Scope:** `--project NAME|SLUG|UUID` or `--team KEY`; `--all` includes
  closed issues.
- **Destination:** `--into P` or `--no-project`; default is a project named
  after the Linear project or team.
- **Re-runs:** `--existing skip` (default) or `--existing update`.
- Progress goes to stderr. The output is a summary block (`X-Imported`,
  `X-Updated`, `X-Skipped`), then one `--brief` line per touched issue.

## Testing

Snapshot tests in `tests/` (`assert_cmd` + `insta`) run against a temp root,
with `LASTISSUE_REPOS` pointed at a temp file so `init` never touches the
real registry. They exercise the core contract end to end.
