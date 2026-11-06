# lastissue

A local, plaintext issue tracker. Each issue is one mbox file in your repo's
`issues/` folder, and the file is the truth: no database, no server. Git
syncs the files, `grep` searches them, `mutt -f` opens them, and people and AI
agents edit them through the same CLI.

```sh
lastissue init
lastissue issue create --title "Login crashes on empty password" --label bug
lastissue issue list --filter "is:ready @me"
```

## Workspace

| Path | Package | What it is |
|---|---|---|
| [`crates/core`](crates/core/README.md) | `lastissue-core` (lib `lastissue`) | File format, data model and every operation. The contract. |
| [`crates/cli`](crates/cli/README.md) | `cli` (bin `lastissue`) | Command-line front-end, for people and agents. |
| [`apps/desktop`](apps/desktop/README.md) | `desktop` (bin `lastissue-gui`) | Tauri + Svelte desktop app. |

Both front-ends link the core library and never touch issue files any other
way, so they share one set of semantics. Neither shells out to the other.

`DESIGN.md` holds the desktop app's visual design; `PRODUCT.md` holds its
product intent.

## Build

Needs Rust (pinned in `rust-toolchain.toml`) and, for the desktop app, pnpm.

| Command | Does |
|---|---|
| `make install` | Installs `lastissue` into `~/.cargo/bin` |
| `make dev` | Runs the desktop app with hot reload |
| `make bundle` | Builds an unsigned `lastissue.app` and `.dmg` under `target/release/bundle/` |
| `make build` | Release builds of the CLI and the app, no bundle |
| `make test` | `cargo test --workspace`, then `svelte-check` and `vitest` |
| `make bench` | Core scale benchmarks (10k and 100k synthetic issues) |

## Conventions

- One virtual Cargo manifest (`resolver = "3"`, edition 2024). Every
  dependency version is pinned once in `[workspace.dependencies]`.
- Lints: `unsafe_code = "forbid"`, clippy `pedantic` as warnings with a short
  list of allows. Every crate sets `[lints] workspace = true`.
- No async and no database anywhere.

## Out of scope (v1)

JSON output, attachments, locks, a merge driver, auto-commits, auto-expiry,
notifications, a web view, project metadata, nested projects, multiple
assignees, configurable workflows and a full-text index.
