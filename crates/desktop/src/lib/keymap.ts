// Keyboard: the action registry, the default keymap, user
// overrides (`gui-keymap.json`, loaded and watched by Rust), and key
// resolution with contexts and two-key sequences (`g i`).
//
// Contexts, least to most specific: `global` (always), `workspace` (no
// text input focused, no overlay), then `issues` + `list`/`board` when the
// list or board has focus, or `detail` when the detail pane has. A more
// specific context's binding wins.

import type { Bindings } from "./api";

export type Context = "global" | "workspace" | "issues" | "list" | "board" | "detail";

export const ACTIONS = {
  openPalette: "Command palette",
  openSearch: "Search issues",
  newIssue: "New issue",
  goMyIssues: "Go to my issues",
  goAllIssues: "Go to all issues",
  goProjects: "Go to project…",
  goBoard: "Show board",
  goList: "Show list",
  toggleGrouping: "Toggle grouping (state / priority)",
  cycleSort: "Cycle sort order",
  setState: "Change state…",
  setPriority: "Set priority…",
  setAssignee: "Assign to…",
  assignToMe: "Assign to me",
  setLabels: "Edit labels…",
  setDue: "Set due date…",
  moveToProject: "Move to project…",
  rename: "Rename issue",
  editBody: "Edit description",
  copyIssueLink: "Copy issue link (lastissue://)",
  deleteIssue: "Delete issue…",
  moveCardPrev: "Move card to previous state",
  moveCardNext: "Move card to next state",
  openIssue: "Open issue",
  closeIssue: "Close issue pane",
  cursorDown: "Next issue",
  cursorUp: "Previous issue",
  columnNext: "Next column",
  columnPrev: "Previous column",
  toggleSelect: "Toggle selection",
  selectAll: "Select all",
  clearSelection: "Clear selection / back",
  newProject: "New project…",
  renameProject: "Rename project…",
  addRepo: "Add repo…",
  openSettings: "Open settings (keymap file)",
  switchRepo1: "Switch to repo 1",
  switchRepo2: "Switch to repo 2",
  switchRepo3: "Switch to repo 3",
  switchRepo4: "Switch to repo 4",
  switchRepo5: "Switch to repo 5",
  switchRepo6: "Switch to repo 6",
  switchRepo7: "Switch to repo 7",
  switchRepo8: "Switch to repo 8",
  switchRepo9: "Switch to repo 9",
} as const;

export type Action = keyof typeof ACTIONS;

function isAction(a: string): a is Action {
  return Object.hasOwn(ACTIONS, a);
}

const issueKeys: Record<string, Action> = {
  s: "setState",
  p: "setPriority",
  a: "setAssignee",
  i: "assignToMe",
  d: "setDue",
  m: "moveToProject",
  r: "rename",
  e: "editBody",
  "cmd-backspace": "deleteIssue",
  "cmd-shift-c": "copyIssueLink",
};

const DEFAULT_KEYMAP: Record<Context, Record<string, Action>> = {
  global: {
    "cmd-k": "openPalette",
    "cmd-,": "openSettings",
    ...Object.fromEntries(
      [1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [`cmd-${n}`, `switchRepo${n}` as Action]),
    ),
  },
  workspace: {
    "/": "openSearch",
    c: "newIssue",
    "g i": "goMyIssues",
    "g a": "goAllIssues",
    "g p": "goProjects",
    "g b": "goBoard",
    "g l": "goList",
  },
  issues: {
    down: "cursorDown",
    j: "cursorDown",
    up: "cursorUp",
    k: "cursorUp",
    o: "openIssue",
    enter: "openIssue",
    x: "toggleSelect",
    "cmd-a": "selectAll",
    escape: "clearSelection",
    ...issueKeys,
  },
  list: { l: "setLabels" },
  board: {
    left: "columnPrev",
    h: "columnPrev",
    right: "columnNext",
    l: "columnNext",
    "shift-h": "moveCardPrev",
    "shift-l": "moveCardNext",
  },
  detail: {
    down: "cursorDown",
    j: "cursorDown",
    up: "cursorUp",
    k: "cursorUp",
    escape: "clearSelection",
    l: "setLabels",
    "cmd-enter": "closeIssue",
    ...issueKeys,
  },
};

export type Keymap = Record<string, Record<string, Action>>;

/** Defaults with user overrides on top. Returns problems (unknown
 *  contexts or actions, bad entries) alongside; `//` keys are comments. */
export function merge(user: Bindings): { keymap: Keymap; errors: string[] } {
  const keymap: Keymap = {};
  for (const [ctx, binds] of Object.entries(DEFAULT_KEYMAP)) keymap[ctx] = { ...binds };
  const errors: string[] = [];
  for (const [ctx, binds] of Object.entries(user)) {
    if (ctx.startsWith("//")) continue;
    if (!(ctx in keymap)) {
      errors.push(`unknown context "${ctx}"`);
      continue;
    }
    if (typeof binds !== "object" || binds === null || Array.isArray(binds)) {
      errors.push(`${ctx}: expected an object of key → action`);
      continue;
    }
    for (const [keys, action] of Object.entries(binds)) {
      const k = normalizeKeys(keys);
      if (action === null) delete keymap[ctx][k];
      else if (typeof action === "string" && isAction(action)) keymap[ctx][k] = action;
      else errors.push(`${ctx} / ${keys}: unknown action ${JSON.stringify(action)}`);
    }
  }
  return { keymap, errors };
}

const MODS = ["cmd", "ctrl", "alt", "shift"] as const;

/** Canonical form of a binding: modifiers in a fixed order. */
export function normalizeKeys(keys: string): string {
  return keys
    .trim()
    .split(/\s+/)
    .map((stroke) => {
      const parts = stroke.toLowerCase().split("-");
      // "cmd--" binds the minus key.
      const key = stroke.endsWith("--") ? "-" : parts.pop()!;
      const mods = MODS.filter((m) => parts.includes(m));
      return [...mods, key].join("-");
    })
    .join(" ");
}

const NAMES: Record<string, string> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Enter: "enter",
  Escape: "escape",
  Backspace: "backspace",
  Delete: "delete",
  Tab: "tab",
  " ": "space",
};

/** A keydown as a keystroke (`cmd-k`, `shift-h`, `/`), or null for a
 *  lone modifier. Shift only counts for letters and named keys: `?` is
 *  just `?`. */
export function stroke(e: Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">): string | null {
  if (["Meta", "Control", "Alt", "Shift", "CapsLock"].includes(e.key)) return null;
  let key = NAMES[e.key] ?? e.key.toLowerCase();
  // Alt changes the character on macOS (alt-a is "å"); use the key code.
  if (e.altKey && /^Key[A-Z]$/.test(e.code)) key = e.code.slice(3).toLowerCase();
  if (e.altKey && /^Digit\d$/.test(e.code)) key = e.code.slice(5);
  const named = key.length > 1 || /^[a-z]$/.test(key);
  const mods = [
    e.metaKey && "cmd",
    e.ctrlKey && "ctrl",
    e.altKey && "alt",
    e.shiftKey && named && "shift",
  ].filter(Boolean);
  return [...mods, key].join("-");
}

/** Resolves keystrokes against the active contexts, holding the first
 *  key of a sequence for up to a second. */
export class Resolver {
  private pending: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(public keymap: Keymap) {}

  /** `contexts` least to most specific. Returns the action, "pending" if
   *  the key starts a sequence, or null. */
  feed(key: string, contexts: Context[]): Action | "pending" | null {
    const candidate = this.pending ? `${this.pending} ${key}` : key;
    const hadPending = this.pending !== null;
    this.clear();
    const found = this.lookup(candidate, contexts);
    if (found) return found;
    if (this.starts(candidate, contexts)) {
      this.pending = candidate;
      this.timer = setTimeout(() => this.clear(), 1000);
      return "pending";
    }
    return hadPending ? this.feed(key, contexts) : null;
  }

  private lookup(keys: string, contexts: Context[]): Action | null {
    for (let i = contexts.length - 1; i >= 0; i--) {
      const a = this.keymap[contexts[i]]?.[keys];
      if (a) return a;
    }
    return null;
  }

  private starts(prefix: string, contexts: Context[]): boolean {
    return contexts.some((c) =>
      Object.keys(this.keymap[c] ?? {}).some((k) => k.startsWith(`${prefix} `)),
    );
  }

  private clear() {
    this.pending = null;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}

/** The binding to show for an action in the active contexts. */
export function bindingFor(keymap: Keymap, action: Action, contexts: Context[]): string | null {
  for (let i = contexts.length - 1; i >= 0; i--) {
    const hit = Object.entries(keymap[contexts[i]] ?? {}).find(([, a]) => a === action);
    if (hit) return hit[0];
  }
  for (const binds of Object.values(keymap)) {
    const hit = Object.entries(binds).find(([, a]) => a === action);
    if (hit) return hit[0];
  }
  return null;
}

const GLYPH: Record<string, string> = {
  cmd: "⌘",
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
  enter: "↵",
  escape: "esc",
  backspace: "⌫",
  up: "↑",
  down: "↓",
  left: "←",
  right: "→",
};

/** `cmd-shift-c` → `⌘⇧C`, `g i` → `g i`. */
export function pretty(keys: string): string {
  return keys
    .split(" ")
    .map((s) => {
      const parts = s === "-" ? ["-"] : s.split("-");
      return parts
        .map((p) => GLYPH[p] ?? (p.length === 1 && parts.length > 1 ? p.toUpperCase() : p))
        .join("");
    })
    .join(" ");
}
