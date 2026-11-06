// The IPC surface: types mirroring `src-tauri/src/dto.rs` and thin
// wrappers over the Tauri commands and events. Nothing else talks to Tauri.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type State =
  | "backlog"
  | "todo"
  | "in-progress"
  | "in-review"
  | "done"
  | "cancelled"
  | "duplicate";
export type Priority = "urgent" | "high" | "medium" | "low";

export interface Summary {
  key: string;
  id: string;
  project: string | null;
  projectName: string | null;
  title: string;
  state: State;
  /** Not done, cancelled or a duplicate. */
  open: boolean;
  priority: Priority | null;
  labels: string[];
  assignee: string | null;
  assigneeName: string | null;
  author: string;
  authorName: string;
  agent: boolean;
  created: number | null;
  updated: number | null;
  due: number | null;
  commentCount: number;
}

export interface Project {
  id: string;
  slug: string;
  name: string;
}

interface IndexDelta {
  full: boolean;
  changed: Summary[];
  removed: string[];
  projects: Project[] | null;
}

interface Repo {
  label: string;
  root: string;
  global: boolean;
  exists: boolean;
}

export type GroupKey =
  | { kind: "state"; value: State }
  | { kind: "priority"; value: Priority | null };

export type Scope =
  | { kind: "mine" }
  | { kind: "all" }
  | { kind: "project"; project: string }
  | { kind: "noProject" };

type Mode = "list" | "board";
export type Sort = "priority" | "updated" | "created" | "due" | "state";
export type Grouping = "state" | "priority";

export interface View {
  mode: Mode;
  scope: Scope;
  filter: string;
  sort: Sort;
  grouping: Grouping;
}

interface Group {
  key: GroupKey;
  items: string[];
}

/** Open issues per sidebar scope. */
interface Counts {
  all: number;
  mine: number;
  noProject: number;
  /** Project short ID → count. */
  projects: Record<string, number>;
}

export interface Visible {
  view: View;
  groups: Group[];
  list: number[];
  total: number;
  error: string | null;
  scanning: boolean;
  counts: Counts;
}

export type Edit =
  | { kind: "title"; value: string }
  | { kind: "body"; value: string }
  | { kind: "state"; value: State }
  | { kind: "priority"; value: Priority | null }
  | { kind: "assignee"; value: string | null }
  | { kind: "addLabel"; value: string }
  | { kind: "removeLabel"; value: string }
  | { kind: "due"; value: string | null }
  | { kind: "project"; value: string | null };

export type PickerKind = "state" | "priority" | "assignee" | "labels" | "due" | "project";

export interface PickOption {
  label: string;
  detail: string | null;
  checked: boolean;
  edit: Edit;
}

export interface Hit {
  key: string;
  id: string;
  tier: string;
  snippets: { loc: string; text: string }[];
}

export interface Search {
  generation: number;
  query: string;
  hits: Hit[];
  scanning: boolean;
  error: string | null;
}

export interface Relation {
  messageId: string;
  id: string | null;
  title: string | null;
  state: State | null;
}

export type Activity =
  | {
      kind: "comment";
      index: number;
      number: number;
      replyTo: number | null;
      messageId: string;
      who: string;
      date: number | null;
      edited: boolean;
      body: string;
    }
  | {
      kind: "event";
      index: number;
      event: string;
      who: string;
      date: number | null;
      lines: string[];
    };

interface Field {
  value: string;
  dirty: boolean;
  conflict: string | null;
}

export interface Doc {
  id: string;
  loading: boolean;
  error: string | null;
  saveError: string | null;
  title: Field | null;
  body: Field | null;
  leftover: string | null;
  reset: number;
  summary: Summary | null;
  /** `Cc:` display names. */
  watchers: string[];
  parent: Relation | null;
  blockedBy: Relation[];
  related: Relation[];
  activity: Activity[];
  link: string | null;
}

export interface Meta {
  repos: Repo[];
  current: number | null;
  me: string | null;
  meError: string | null;
  loaded: boolean;
  notice: string | null;
  failure: string | null;
  error: string | null;
  loadErrors: string[];
}

/** context → keys → action (null unbinds), as written: unchecked. */
export type Bindings = Record<string, unknown>;

export interface UserKeymap {
  path: string | null;
  bindings: Bindings;
  errors: string[];
}

interface Snapshot {
  meta: Meta;
  issues: Summary[];
  projects: Project[];
  visible: Visible;
  doc: Doc | null;
  search: Search;
  keymap: UserKeymap;
}

export interface NewIssue {
  title: string;
  body: string;
  project?: string | null;
  state?: State | null;
  priority?: Priority | null;
  labels?: string[];
  assignee?: string | null;
  due?: string | null;
}

export type DocAction = "keepMine" | "useTheirs" | "restoreLeftover" | "discardLeftover";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  setView: (view: View) => invoke<void>("set_view", { view }),
  select: (id: string | null) => invoke<void>("select", { id }),
  docEdit: (id: string, fieldKind: "title" | "body", text: string) =>
    invoke<boolean>("doc_edit", { id, fieldKind, text }),
  docSave: () => invoke<void>("doc_save"),
  docAction: (action: DocAction) => invoke<void>("doc_action", { action }),
  applyEdit: (ids: string[], edit: Edit) => invoke<void>("apply_edit", { ids, edit }),
  dropCard: (ids: string[], target: GroupKey) => invoke<void>("drop_card", { ids, target }),
  moveCards: (ids: string[], step: number) => invoke<void>("move_cards", { ids, step }),
  deleteIssues: (ids: string[]) => invoke<void>("delete_issues", { ids }),
  createIssue: (issue: NewIssue) => invoke<string>("create_issue", { issue }),
  addComment: (id: string, body: string) => invoke<void>("add_comment", { id, body, replyTo: null }),
  editComment: (id: string, messageId: string, body: string) =>
    invoke<void>("edit_comment", { id, messageId, body }),
  deleteComment: (id: string, messageId: string) =>
    invoke<void>("delete_comment", { id, messageId }),
  createProject: (name: string) => invoke<Project>("create_project", { name }),
  renameProject: (project: string, name: string) =>
    invoke<Project>("rename_project", { project, name }),
  pickerOptions: (kind: PickerKind, ids: string[]) =>
    invoke<PickOption[]>("picker_options", { kind, ids }),
  pickerTyped: (kind: PickerKind, ids: string[], query: string) =>
    invoke<PickOption | null>("picker_typed", { kind, ids, query }),
  search: (query: string) => invoke<void>("search", { query }),
  openRepo: (index: number) => invoke<void>("open_repo", { index }),
  addRepo: (path: string) => invoke<void>("add_repo", { path }),
  issueLink: (id: string) => invoke<string>("issue_link", { id }),
  openKeymapFile: () => invoke<string>("open_keymap_file"),
};

interface Events {
  meta: Meta;
  index: IndexDelta;
  visible: Visible;
  doc: Doc | null;
  search: Search;
  keymap: UserKeymap;
  "focus-detail": string;
}

export function on<K extends keyof Events>(
  event: K,
  f: (payload: Events[K]) => void,
): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => f(e.payload));
}
