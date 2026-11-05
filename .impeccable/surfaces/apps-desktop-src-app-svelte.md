---
version: 1
slug: "apps-desktop-src-app-svelte"
primary_target: "apps/desktop/src/App.svelte"
related_targets: []
---

# Desktop app shell (list, board, detail, sidebar)

Mode: Operate, with the detail pane in Read mode. A small team reads issues and comments that people and agents wrote, triages them, and replies. Keyboard first, fast at 50k issues. Must stay untouched: all behaviour, the keymap, data flow, and copy that names real commands.

## Direction contract

THESIS: Each issue reads like a well-set plaintext standard: numbered, plain, honest about being a file. Refuses the dense Linear grid of chips, borders, and avatars.

OWN-WORLD: Neutral paper and ink (no cream), monochrome chrome with hairline rules, one link-blue accent, one highlighter-green mark as the only playful ink. Atkinson Hyperlegible Next for UI and prose, Atkinson Hyperlegible Mono for real mail headers, IDs and filenames. Mail-style `>` quotes and § section marks.

STORY: The reader opens an issue and wants to read it: a big title, a 68ch body, a thread on one spine.

FIRST VIEWPORT: Airy 40px list rows on the left (state, title, quiet meta). On the right, a document pane: a mono header block (ID, file, from, date), the title at 26px, the prose body, a property block, then § Activity.

FORM: Plaintext Standard, #6 of 7, seed 4e642214. Raises: rows changed on disk stay highlighted until seen; ID as catalog number; no button chrome in reading; monochrome chrome, where colour belongs only to state and the one accent (raise line as approved on the decision card); activity spine.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
