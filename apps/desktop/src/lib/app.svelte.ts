// The front-end's state: a mirror of what Rust pushes (meta, the issue
// heads, the evaluated view, the open issue, search results, the user
// keymap) plus what is purely UI (cursor, selection, focus, overlays).
// Every user command goes through `run(action)`.

import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { toast } from "svelte-sonner";
import {
  api,
  on,
  type Doc,
  type Edit,
  type GroupKey,
  type Meta,
  type NewIssue,
  type PickerKind,
  type Project,
  type Search,
  type Summary,
  type UserKeymap,
  type View,
  type Visible,
} from "./api";
import { SORT_LABEL } from "./format";
import { type Action, type Context, type Keymap, merge } from "./keymap";
import { type Columns, horizontal, locate, toggle, vertical } from "./nav";

/** Title/body edits save this long after the last keystroke (§5.2). */
const SAVE_DEBOUNCE_MS = 300;

type Overlay =
  | { kind: "picker"; picker: PickerKind; ids: string[] }
  | { kind: "palette"; mode: "all" | "scopes" }
  | { kind: "search" }
  | { kind: "create" }
  | { kind: "confirm"; title: string; detail: string; confirm: string; run: () => void }
  | {
      kind: "prompt";
      title: string;
      placeholder: string;
      value: string;
      submit: (v: string) => Promise<unknown>;
    };

const emptyMeta: Meta = {
  repos: [],
  current: null,
  me: null,
  meError: null,
  loaded: false,
  notice: null,
  failure: null,
  error: null,
  loadErrors: [],
};

const emptyView: View = {
  mode: "list",
  scope: { kind: "all" },
  filter: "",
  sort: "priority",
  grouping: "state",
};

const SORTS = Object.keys(SORT_LABEL) as View["sort"][];

function isTyping(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  return el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName);
}

/** Writes run one at a time, in order (async Tauri commands could
 *  otherwise reorder, e.g. an edit after the select that closes it). */
let queue: Promise<unknown> = Promise.resolve();
function serial<T>(f: () => Promise<T>): Promise<T> {
  const p = queue.then(f, f);
  queue = p.catch(() => {});
  return p;
}

/** When this window last wrote to disk; index deltas soon after are our own echo. */
let lastWrite = 0;
const ECHO_MS = 3000;

function write<T>(f: () => Promise<T>): Promise<T> {
  lastWrite = Date.now();
  return serial(f);
}

export function report(e: unknown) {
  toast.error(String(e));
}

class App {
  meta = $state<Meta>(emptyMeta);
  /** key → summary. */
  issues = new SvelteMap<string, Summary>();
  /** Keys that changed on disk (another writer, a git pull) and haven't been opened since. */
  fresh = new SvelteSet<string>();
  /** short ID → key (not reactive; read alongside `issues`). */
  private byId = new Map<string, string>();
  projects = $state<Project[]>([]);
  visible = $state<Visible>({
    view: emptyView,
    groups: [],
    list: [],
    total: 0,
    error: null,
    scanning: false,
    counts: { all: 0, mine: 0, noProject: 0, projects: {} },
  });
  /** The view as last set here (ahead of `visible.view` until Rust answers). */
  view = $state<View>(emptyView);
  doc = $state<Doc | null>(null);
  search = $state<Search>({ generation: 0, query: "", hits: [], scanning: false, error: null });
  userKeymap = $state<UserKeymap>({ path: null, bindings: {}, errors: [] });
  merged = $derived(merge(this.userKeymap.bindings));
  keymap: Keymap = $derived(this.merged.keymap);

  /** Keyboard position (short ID). */
  cursor = $state<string | null>(null);
  /** Issues `x` selected (short IDs). */
  selection = $state<string[]>([]);
  /** Where issue commands go: the list/board or the detail pane. */
  focus = $state<"issues" | "detail">("issues");
  overlay = $state<Overlay | null>(null);
  /** The detail pane's body is in edit mode. */
  editingBody = $state(false);
  /** Bumped to focus the title field (`r`). */
  renameTick = $state(0);
  /** Bumped to focus the detail pane (links). */
  focusDetailTick = $state(0);

  private saveTimer: ReturnType<typeof setTimeout> | null = null;

  async init() {
    await Promise.all([
      on("meta", (m) => {
        if (m.error && m.error !== this.meta.error) toast.error(m.error);
        this.meta = m;
      }),
      on("index", (d) => this.applyIndex(d.full, d.changed, d.removed, d.projects)),
      on("visible", (v) => {
        this.visible = v;
        this.view = v.view;
        this.pruneSelection();
      }),
      on("doc", (d) => this.setDoc(d)),
      on("search", (s) => {
        if (s.generation >= this.search.generation) this.search = s;
      }),
      on("keymap", (k) => {
        this.userKeymap = k;
        if (k.errors.length) toast.error(`Keymap: ${k.errors.join("; ")}`);
      }),
      on("focus-detail", () => {
        this.focus = "detail";
        this.focusDetailTick++;
      }),
    ]);
    const s = await api.snapshot();
    this.meta = s.meta;
    this.applyIndex(true, s.issues, [], s.projects);
    this.visible = s.visible;
    this.view = s.visible.view;
    this.setDoc(s.doc);
    this.search = s.search;
    this.userKeymap = s.keymap;
  }

  private applyIndex(full: boolean, changed: Summary[], removed: string[], projects: Project[] | null) {
    if (full) {
      this.issues.clear();
      this.byId.clear();
      this.fresh.clear();
    }
    for (const k of removed) {
      this.fresh.delete(k);
      const s = this.issues.get(k);
      if (s && this.byId.get(s.id) === k) this.byId.delete(s.id);
      this.issues.delete(k);
    }
    const echo = Date.now() - lastWrite < ECHO_MS;
    for (const s of changed) {
      const prev = this.issues.get(s.key);
      if (!full && !echo && s.id !== this.doc?.id && (!prev || prev.updated !== s.updated)) this.fresh.add(s.key);
      this.issues.set(s.key, s);
      if (!this.byId.has(s.id) || full) this.byId.set(s.id, s.key);
    }
    if (projects) this.projects = projects;
  }

  private setDoc(d: Doc | null) {
    if (!d || d.id !== this.doc?.id) this.editingBody = false;
    this.doc = d;
    if (!d && this.focus === "detail") this.focus = "issues";
  }

  atCursor = (key: string) => this.cursor !== null && this.issues.get(key)?.id === this.cursor;

  byShortId(id: string): Summary | undefined {
    const k = this.byId.get(id);
    return k ? this.issues.get(k) : undefined;
  }

  // ------------------------------------------------------------- view

  setView(patch: Partial<View>) {
    this.view = { ...this.view, ...patch };
    serial(() => api.setView(this.view)).catch(report);
  }

  /** The visible issues as columns of short IDs (§7 navigation). */
  columns(): Columns {
    const id = (k: string) => this.issues.get(k)?.id;
    const ids = (keys: string[]) => keys.map(id).filter((x): x is string => !!x);
    if (this.visible.view.mode === "board") return this.visible.groups.map((g) => ids(g.items));
    return [this.visible.list.flatMap((i) => ids(this.visible.groups[i].items))];
  }

  private pruneSelection() {
    if (this.selection.some((id) => !this.byId.has(id)))
      this.selection = this.selection.filter((id) => this.byId.has(id));
  }

  /** The issues an issue command acts on: the open one when the detail
   *  pane has focus, else the selection, else the cursor's, else the open one. */
  targets(): string[] {
    const open = this.doc?.id ?? null;
    if (this.focus === "detail" && open) return [open];
    if (this.selection.length) return [...this.selection];
    const t = this.cursor ?? open;
    return t ? [t] : [];
  }

  /** Key contexts, least to most specific. `behind`: as if no overlay
   *  or text input had focus (for showing bindings). */
  contexts(behind = false): Context[] {
    if (!behind && (this.overlay || isTyping())) return ["global"];
    if (this.focus === "detail" && this.doc) return ["global", "workspace", "detail"];
    return ["global", "workspace", "issues", this.visible.view.mode];
  }

  // ------------------------------------------------------- open issue

  open(id: string | null) {
    if (id) {
      this.cursor = id;
      const k = this.byId.get(id);
      if (k) this.fresh.delete(k);
    }
    serial(() => api.select(id)).catch(report);
  }

  clickIssue(id: string, e: MouseEvent) {
    this.focus = "issues";
    if (e.metaKey) {
      this.cursor = id;
      this.run("toggleSelect");
    } else this.open(id);
  }

  closeDoc() {
    this.flushSave();
    this.open(null);
    this.focus = "issues";
  }

  /** The title or body editor changed (only real user edits). */
  editField(kind: "title" | "body", text: string) {
    const id = this.doc?.id;
    if (!id) return;
    write(() => api.docEdit(id, kind, text))
      .then((due) => due && this.scheduleSave())
      .catch(report);
  }

  private scheduleSave() {
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      this.saveTimer = null;
      write(() => api.docSave()).catch(report);
    }, SAVE_DEBOUNCE_MS);
  }

  flushSave() {
    if (!this.saveTimer) return;
    clearTimeout(this.saveTimer);
    this.saveTimer = null;
    write(() => api.docSave()).catch(report);
  }

  docAction(a: "keepMine" | "useTheirs" | "restoreLeftover" | "discardLeftover") {
    serial(() => api.docAction(a)).catch(report);
  }

  // ---------------------------------------------------------- writes

  edit(ids: string[], edit: Edit) {
    if (!ids.length) return Promise.resolve();
    return write(() => api.applyEdit(ids, edit)).catch(report);
  }

  dropCard(id: string, target: GroupKey) {
    const ids = this.selection.includes(id) ? [...this.selection] : [id];
    write(() => api.dropCard(ids, target)).catch(report);
  }

  createIssue(issue: NewIssue) {
    return write(() => api.createIssue(issue)).then((id) => {
      this.cursor = id;
      this.focus = "detail";
      return id;
    });
  }

  addComment(body: string) {
    const id = this.doc?.id;
    if (!id) return Promise.reject(new Error("no open issue"));
    return write(() => api.addComment(id, body));
  }

  editComment(messageId: string, body: string) {
    const id = this.doc?.id;
    if (!id) return Promise.reject(new Error("no open issue"));
    return write(() => api.editComment(id, messageId, body));
  }

  deleteComment(messageId: string) {
    const id = this.doc?.id;
    if (!id) return;
    this.overlay = {
      kind: "confirm",
      title: "Delete comment?",
      detail: "The comment is removed from the file; an event records it.",
      confirm: "Delete",
      run: () => write(() => api.deleteComment(id, messageId)).catch(report),
    };
  }

  // ---------------------------------------------------------- actions

  private pick(picker: PickerKind) {
    const ids = this.targets();
    if (ids.length) this.overlay = { kind: "picker", picker, ids };
  }

  /** Open the cursor's issue first if the command needs the detail pane. */
  private withDoc(f: () => void) {
    const id = this.targets()[0];
    if (!id) return;
    if (this.doc?.id !== id) this.open(id);
    this.focus = "detail";
    f();
  }

  private moveCursor(next: string | null) {
    if (!next) return;
    this.cursor = next;
    if (this.doc && this.doc.id !== next) this.open(next);
  }

  private currentProject(): Project | undefined {
    const s = this.view.scope;
    return s.kind === "project" ? this.projects.find((p) => p.id === s.project) : undefined;
  }

  run(a: Action) {
    switch (a) {
      case "openPalette":
        this.overlay = this.overlay?.kind === "palette" ? null : { kind: "palette", mode: "all" };
        return;
      case "openSearch":
        this.overlay = { kind: "search" };
        return;
      case "newIssue":
        this.overlay = { kind: "create" };
        return;
      case "goMyIssues":
        return this.setView({ scope: { kind: "mine" } });
      case "goAllIssues":
        return this.setView({ scope: { kind: "all" } });
      case "goProjects":
        this.overlay = { kind: "palette", mode: "scopes" };
        return;
      case "goBoard":
        return this.setView({ mode: "board" });
      case "goList":
        return this.setView({ mode: "list" });
      case "toggleGrouping":
        return this.setView({ grouping: this.view.grouping === "state" ? "priority" : "state" });
      case "cycleSort":
        return this.setView({ sort: SORTS[(SORTS.indexOf(this.view.sort) + 1) % SORTS.length] });
      case "cursorDown":
      case "cursorUp":
        return this.moveCursor(vertical(this.columns(), this.cursor, a === "cursorDown" ? 1 : -1));
      case "columnNext":
      case "columnPrev":
        return this.moveCursor(horizontal(this.columns(), this.cursor, a === "columnNext" ? 1 : -1));
      case "openIssue": {
        const cols = this.columns();
        const id =
          this.cursor && locate(cols, this.cursor) ? this.cursor : vertical(cols, null, 1);
        if (id) this.open(id);
        return;
      }
      case "closeIssue":
        return this.closeDoc();
      case "toggleSelect": {
        if (!this.cursor) this.cursor = vertical(this.columns(), null, 1);
        if (this.cursor) this.selection = toggle(this.selection, this.cursor);
        return;
      }
      case "selectAll":
        this.selection = this.columns().flat();
        return;
      case "clearSelection":
        if (this.selection.length) this.selection = [];
        else if (this.doc) this.closeDoc();
        return;
      case "moveCardPrev":
      case "moveCardNext": {
        const ids = this.targets();
        if (ids.length)
          write(() => api.moveCards(ids, a === "moveCardNext" ? 1 : -1)).catch(report);
        return;
      }
      case "setState":
        return this.pick("state");
      case "setPriority":
        return this.pick("priority");
      case "setAssignee":
        return this.pick("assignee");
      case "setLabels":
        return this.pick("labels");
      case "setDue":
        return this.pick("due");
      case "moveToProject":
        return this.pick("project");
      case "assignToMe":
        this.edit(this.targets(), { kind: "assignee", value: "me" });
        return;
      case "rename":
        return this.withDoc(() => this.renameTick++);
      case "editBody":
        return this.withDoc(() => (this.editingBody = true));
      case "copyIssueLink": {
        const id = this.targets()[0];
        if (!id) return;
        const link = this.doc?.id === id && this.doc.link;
        (link ? Promise.resolve(link) : api.issueLink(id))
          .then((url) => writeText(url).then(() => toast.success(`Copied ${url}`)))
          .catch(report);
        return;
      }
      case "deleteIssue": {
        const ids = this.targets();
        if (!ids.length) return;
        const what =
          ids.length === 1
            ? `“${this.byShortId(ids[0])?.title ?? ids[0]}”`
            : `${ids.length} issues`;
        this.overlay = {
          kind: "confirm",
          title: `Delete ${what}?`,
          detail: "The issue file is removed from disk. This cannot be undone here.",
          confirm: "Delete",
          run: () => {
            this.selection = [];
            write(() => api.deleteIssues(ids)).catch(report);
          },
        };
        return;
      }
      case "newProject":
        this.overlay = {
          kind: "prompt",
          title: "New project",
          placeholder: "Project name",
          value: "",
          submit: (name) =>
            serial(() => api.createProject(name)).then((p) =>
              this.setView({ scope: { kind: "project", project: p.id } }),
            ),
        };
        return;
      case "renameProject": {
        const p = this.currentProject();
        if (!p) {
          toast.info("Open a project first (g p).");
          return;
        }
        this.overlay = {
          kind: "prompt",
          title: `Rename project ${p.name}`,
          placeholder: "Project name",
          value: p.name,
          submit: (name) =>
            serial(() => api.renameProject(p.id, name)).then((np) =>
              this.setView({ scope: { kind: "project", project: np.id } }),
            ),
        };
        return;
      }
      case "addRepo":
        openDialog({ directory: true, title: "Open an issues folder or a repo containing one" })
          .then((path) => {
            if (path) return serial(() => api.addRepo(path));
          })
          .catch(report);
        return;
      case "openSettings":
        api
          .openKeymapFile()
          .then((p) => toast.info(`Editing ${p} — changes apply on save.`))
          .catch(report);
        return;
      default:
        if (a.startsWith("switchRepo")) {
          const i = Number(a.slice("switchRepo".length)) - 1;
          if (i < this.meta.repos.length && i !== this.meta.current) this.switchRepo(i);
        }
    }
  }

  switchRepo(i: number) {
    this.cursor = null;
    this.selection = [];
    this.focus = "issues";
    serial(() => api.openRepo(i)).catch(report);
  }
}

export const app = new App();
