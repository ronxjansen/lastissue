# lastissue

A local, plaintext issue tracker, optimized for LLM usage. Lastissue is both a Desktop application and a CLI that share the same state. Each issue is one mbox file in your repo's `issues/` folder. There is no database, no server. 

```sh
lastissue init
lastissue issue create --title "Login crashes on empty password" --label bug
lastissue issue list --filter "is:ready @me"
```

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

## Releasing

Releases are automated with [release-please](https://github.com/googleapis/release-please).
Write commits as [Conventional Commits](https://www.conventionalcommits.org/)
(`feat:`, `fix:`, `perf:`, `feat!:` for breaking changes). On every push to
`master`, release-please keeps a release PR open that bumps the version
(workspace `Cargo.toml`, `Cargo.lock`, `package.json`, `tauri.conf.json`) and
updates [`CHANGELOG.md`](CHANGELOG.md). Merging that PR tags `vX.Y.Z`, creates
the GitHub release and attaches CLI binaries (macOS arm64/x86_64, Linux x86_64)
and an unsigned macOS `.dmg`.

Before 1.0, `feat` and breaking changes bump the minor version; `fix` and
`perf` bump the patch version.

## Out of scope (v1)

JSON output, attachments, locks, a merge driver, auto-commits, auto-expiry,
notifications, a web view, project metadata, nested projects, multiple
assignees, configurable workflows and a full-text index.
