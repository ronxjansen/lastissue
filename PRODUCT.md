# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(A Tauri 2 desktop app; the webview UI follows web conventions. It runs on macOS first.)

## Users

Small teams who share an `issues/` folder through git. Each person tracks, triages and discusses issues in their own repos, and reads issues and comments that teammates and AI agents wrote through the CLI. Much of the time goes to reading: issue bodies, comment threads and activity. Less goes to bulk triage.

## Product Purpose

lastissue is a local, plaintext issue tracker. Every issue is one mbox file in the repo, and the file is the source of truth: there is no database and no server. The desktop app is a calm way to read, write and organise those files. Success means the team actually reads the issues, because the issues are pleasant to read and quick to find.

## Positioning

Issues live in the repo as real mail files (they open in `mutt -f`). Git syncs them, they can be grepped, and a person or an agent can edit them with the same CLI. A hosted tracker cannot honestly claim that its data is just files in your repo.

## Operating Context

- Desktop app with a sidebar (repos, projects, views), an issue list or kanban board, a detail pane with the issue body and an activity/comment thread, a command palette (⌘K), pickers, and a status bar.
- Keyboard-driven, with a configurable keymap. Mouse works everywhere.
- Issue bodies and comments are markdown. The app edits them WYSIWYG with Milkdown and renders read-only comments with marked.
- Issues come from people, AI agents and the Linear importer, so text length and quality vary widely.
- The app has to stay fast at 50k issues.

## Capabilities and Constraints

- Stack: Svelte 5, Tailwind 4, shadcn-svelte (bits-ui), and Lucide icons, inside Tauri.
- States, priorities, keywords (labels), projects, relations, comments and events, all from SPEC.md.
- Both themes get equal care, and the app follows the OS appearance.

## Brand Commitments

- Name: **lastissue**, always lowercase.
- References the user gave: Linear (smooth and good-looking, but too dense) and Fizzy by Basecamp (warm and characterful, but so playful it gets cluttered). The target sits between them.
- Readability comes first. The UI should make people want to read, and a few small playful touches are welcome.

## Product Principles

1. Reading beats scanning: treat issue text as prose that deserves room.
2. Calm by default: fewer borders, fewer badges, less chrome. State comes through quiet signals.
3. Playful only in small doses, at moments that don't interrupt reading.
4. Keyboard-first and fast, never ornamental at the cost of speed.
5. It's just files: be honest and plain about where the data lives.
