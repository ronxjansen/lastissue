//! The window's state (GUI-SPEC §3, §8): the registry's repos, the open
//! repo and its identity, the view state and its evaluated result, and the
//! open issue. Views render from it and dispatch to its methods; every
//! write goes through [`Workspace::run`] (core `ops` via the repo model).

use super::draft::Drafts;
use super::edit::{self, Applied, Edit, Op};
use super::issue_doc::IssueDoc;
use super::nav::{self, Selection};
use super::repo_model::{RepoChanged, RepoModel};
use super::search::{self, Candidate, FullText, Hit};
use super::view_state::{ListRow, Mode, ViewState, Visible};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task};
use lastissue::identity::Identity;
use lastissue::query::{Filter, ListQuery};
use lastissue::registry::{self, Repo};
use lastissue::store;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The full-text tier starts after this long without a keystroke (§6).
pub const FULL_TEXT_DEBOUNCE: Duration = Duration::from_millis(150);
/// Files per background batch: small first, so early results stream in,
/// then doubling to amortize the round trips.
const FIRST_BATCH: usize = 64;
const MAX_BATCH: usize = 2048;

/// The full-text tier behind the filter bar's free text: which candidate
/// files have been scanned for the current terms, and which matched.
#[derive(Default)]
struct FilterText {
    terms: Vec<String>,
    hits: HashSet<PathBuf>,
    scanned: HashSet<PathBuf>,
    running: bool,
    /// Bumped when the terms change; a batch from an older scan is dropped.
    generation: u64,
    #[allow(dead_code)]
    task: Option<Task<()>>,
}

/// The `/` search box (GUI-SPEC §6): instant hits at once, full-text hits
/// streamed in after the debounce, kept in tiered order.
#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub hits: Vec<Hit>,
    /// The full-text tier is still running.
    pub scanning: bool,
    /// The query's tokens didn't parse or resolve.
    pub error: Option<String>,
    /// Bumped per keystroke: a batch from an older scan is dropped.
    generation: u64,
    #[allow(dead_code)]
    task: Option<Task<()>>,
}

pub struct Workspace {
    /// The registry, plus the open repo if it isn't registered.
    pub repos: Vec<Repo>,
    pub repo: Option<Entity<RepoModel>>,
    pub identity: Arc<Identity>,
    pub view: ViewState,
    pub visible: Visible,
    pub doc: Option<Entity<IssueDoc>>,
    /// The keyboard position in the list or board (short ID).
    pub cursor: Option<String>,
    /// Issues `x` selected; issue commands act on all of them.
    pub selection: Selection,
    pub search: SearchState,
    filter_text: FilterText,
    /// Why no repo is open, or a registry problem.
    pub notice: Option<String>,
    /// The last write failed (shown in the status line until the next one).
    pub error: Option<String>,
    drafts: Option<Drafts>,
    /// A linked issue (URL scheme) to open once the repo has loaded.
    pending_link: Option<String>,
    /// A link opened an issue: the view moves focus to the detail pane
    /// (and clears this).
    pub focus_detail: bool,
    /// Held to keep receiving the open repo's changes.
    #[allow(dead_code)]
    repo_sub: Option<Subscription>,
}

/// The identity for a repo (GUI-SPEC §4): git config in its directory,
/// never the `$USER@host` fallback. (Settings override: later.)
fn identity_for(root: &Path) -> Identity {
    Identity::new(None, None).in_dir(root).without_fallback()
}

/// The repo to open first: an explicit root (argument, `LASTISSUE_DIR`),
/// else discovery from the current directory up to the global root.
pub fn initial_repo(arg: Option<&Path>, repos: &[Repo]) -> lastissue::error::Result<Repo> {
    let env = std::env::var_os("LASTISSUE_DIR").map(PathBuf::from);
    let root = store::discover(arg.or(env.as_deref()))?;
    Ok(repo_for(&root, repos))
}

/// The registry entry for `root`, or an ad-hoc one.
fn repo_for(root: &Path, repos: &[Repo]) -> Repo {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let c = canon(root);
    repos
        .iter()
        .find(|r| canon(&r.root) == c)
        .cloned()
        .unwrap_or_else(|| Repo {
            label: registry::label_of(root),
            root: root.to_path_buf(),
            global: false,
        })
}

impl Workspace {
    pub fn new(arg: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let (mut repos, mut notice) = match registry::load() {
            Ok(r) => (r, None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        let mut ws = Workspace {
            repos: Vec::new(),
            repo: None,
            identity: Arc::new(identity_for(Path::new("."))),
            view: ViewState::default(),
            visible: Visible::default(),
            doc: None,
            cursor: None,
            selection: Selection::default(),
            search: SearchState::default(),
            filter_text: FilterText::default(),
            notice: None,
            error: None,
            drafts: Drafts::platform(),
            pending_link: None,
            focus_detail: false,
            repo_sub: None,
        };
        match initial_repo(arg.as_deref(), &repos) {
            Ok(repo) => {
                if !repos.iter().any(|r| r.root == repo.root) {
                    repos.insert(0, repo.clone());
                }
                ws.repos = repos;
                ws.open_repo(repo, cx);
            }
            Err(e) => {
                ws.repos = repos;
                notice = Some(e.to_string());
            }
        }
        ws.notice = notice;
        ws
    }

    /// Tests: a fixed identity and a scratch drafts dir.
    #[cfg(test)]
    pub fn for_test(&mut self, me: Identity, drafts: &Path) {
        self.identity = Arc::new(me);
        self.drafts = Some(Drafts::new(drafts));
    }

    pub fn open_repo(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.identity = Arc::new(identity_for(&repo.root));
        let model = cx.new(|cx| RepoModel::new(repo, cx));
        self.repo_sub = Some(cx.subscribe(&model, |this, _, ev: &RepoChanged, cx| {
            this.on_repo_changed(ev, cx);
        }));
        self.repo = Some(model);
        self.close_doc(cx);
        self.view.selected = None;
        self.cursor = None;
        self.selection.clear();
        self.search = SearchState::default();
        self.filter_text = FilterText::default();
        self.pending_link = None;
        self.notice = None;
        self.recompute(cx);
    }

    /// Change the view state and re-evaluate.
    pub fn update_view(&mut self, f: impl FnOnce(&mut ViewState), cx: &mut Context<Self>) {
        let before = self.view.clone();
        f(&mut self.view);
        if self.view != before {
            self.recompute(cx);
        }
    }

    /// Open an issue in the detail pane (`None` closes it).
    pub fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if id.is_some() && self.doc.as_ref().map(|d| d.read(cx).id.clone()) == id {
            return;
        }
        self.view.selected = id.clone();
        if id.is_some() {
            self.cursor = id.clone();
        }
        self.close_doc(cx);
        if let (Some(id), Some(repo)) = (id, &self.repo)
            && let Some(s) = repo.read(cx).index.by_id(&id)
        {
            let path = s.path.clone();
            let (repo, me, drafts) = (repo.clone(), self.identity.clone(), self.drafts.clone());
            self.doc = Some(cx.new(|cx| IssueDoc::new(id, path, &repo, me, drafts, cx)));
        }
        cx.notify();
    }

    /// Close the detail pane's issue, saving its drafts first.
    fn close_doc(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.take() {
            doc.update(cx, |d, cx| {
                if d.is_dirty() {
                    d.save_now(cx);
                }
            });
        }
    }

    /// Run a write through core. On success the open issue (if it's the
    /// one written) takes the new messages; a failure lands in `error`.
    pub fn run(&mut self, op: Op, cx: &mut Context<Self>) -> Task<Option<Applied>> {
        let Some(repo) = self.repo.clone() else {
            return Task::ready(None);
        };
        let me = self.identity.clone();
        let task = repo.update(cx, |r, cx| r.run(me, op, cx));
        cx.spawn(async move |this, cx| {
            let r = task.await;
            this.update(cx, |w, cx| match r {
                Ok(a) => {
                    w.error = None;
                    if a.deleted && w.view.selected.as_deref() == Some(a.id.as_str()) {
                        w.doc = None;
                        w.view.selected = None;
                    }
                    if let Some(doc) = &w.doc
                        && a.written
                        && doc.read(cx).id == a.id
                    {
                        let (p, m) = (a.path.clone(), a.msgs.clone());
                        doc.update(cx, |d, cx| d.adopt_own(p, m, cx));
                    }
                    cx.notify();
                    Some(a)
                }
                Err(e) => {
                    w.error = Some(e.to_string().trim().replace('\n', " · "));
                    cx.notify();
                    None
                }
            })
            .ok()
            .flatten()
        })
    }

    /// A property change of one issue (pickers, drag and drop).
    pub fn apply_edit(&mut self, id: String, edit: Edit, cx: &mut Context<Self>) {
        self.run(Op::Update { id, edit }, cx).detach();
    }

    /// The same change to several issues (a bulk edit).
    pub fn apply_to(&mut self, ids: &[String], edit: &Edit, cx: &mut Context<Self>) {
        for id in ids {
            self.apply_edit(id.clone(), edit.clone(), cx);
        }
    }

    /// Delete issues (after the view's confirmation).
    pub fn delete(&mut self, ids: &[String], cx: &mut Context<Self>) {
        for id in ids {
            self.run(Op::Delete { id: id.clone() }, cx).detach();
        }
        self.selection.clear();
    }

    /// `⇧h` / `⇧l` on the board: move the target cards one column each.
    pub fn move_selected(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(repo) = &self.repo else { return };
        let cols: Vec<_> = self.visible.groups.iter().map(|g| g.key).collect();
        let mut edits = Vec::new();
        for id in self.targets(false) {
            let Some(s) = repo.read(cx).index.by_id(&id) else {
                continue;
            };
            let cur = edit::key_of(self.view.grouping, s.state, s.priority);
            if let Some(target) = edit::neighbour(&cols, cur, step)
                && let Some(e) = edit::drop_edit(target, s.state, s.priority)
            {
                edits.push((id, e));
            }
        }
        for (id, e) in edits {
            self.apply_edit(id, e, cx);
        }
    }

    // ------------------------------------------------------- navigation

    /// The visible issues as columns of short IDs: one for the list (in
    /// row order), one per board column.
    pub fn columns(&self, cx: &App) -> Vec<Vec<String>> {
        let Some(repo) = &self.repo else {
            return Vec::new();
        };
        let ix = &repo.read(cx).index;
        let id = |p: &PathBuf| ix.get(p).map(|s| s.id.clone());
        match self.view.mode {
            Mode::List => vec![
                self.visible
                    .rows
                    .iter()
                    .filter_map(|r| match r {
                        ListRow::Issue(p) => id(p),
                        ListRow::Header { .. } => None,
                    })
                    .collect(),
            ],
            Mode::Board => self
                .visible
                .groups
                .iter()
                .map(|g| g.items.iter().filter_map(id).collect())
                .collect(),
        }
    }

    /// `j`/`k` (`vertical`) or `h`/`l`. With the detail pane open, it
    /// follows the cursor.
    pub fn move_cursor(&mut self, vertical: bool, step: isize, cx: &mut Context<Self>) {
        let cols = self.columns(cx);
        let cur = self.cursor.as_deref();
        let next = if vertical {
            nav::vertical(&cols, cur, step)
        } else {
            nav::horizontal(&cols, cur, step)
        };
        if let Some(next) = next {
            if self.doc.is_some() {
                self.select(Some(next.clone()), cx);
            }
            self.cursor = Some(next);
            cx.notify();
        }
    }

    /// `Enter`/`o`: open the cursor's issue (the first, without a cursor).
    pub fn open_cursor(&mut self, cx: &mut Context<Self>) {
        let cols = self.columns(cx);
        let visible = self
            .cursor
            .as_deref()
            .is_some_and(|c| nav::locate(&cols, c).is_some());
        let id = if visible {
            self.cursor.clone()
        } else {
            nav::vertical(&cols, None, 1)
        };
        if id.is_some() {
            self.select(id, cx);
        }
    }

    /// `x`: toggle the cursor's issue in the selection.
    pub fn toggle_select(&mut self, cx: &mut Context<Self>) {
        if self.cursor.is_none() {
            self.cursor = nav::vertical(&self.columns(cx), None, 1);
        }
        if let Some(c) = self.cursor.clone() {
            self.selection.toggle(&c);
            cx.notify();
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let cols = self.columns(cx);
        self.selection.set_all(&cols);
        cx.notify();
    }

    /// `Esc`: clear the selection, else close the detail pane. Returns
    /// whether there was anything to clear.
    pub fn clear(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.selection.is_empty() {
            self.selection.clear();
            cx.notify();
            true
        } else if self.doc.is_some() {
            self.select(None, cx);
            true
        } else {
            false
        }
    }

    /// The issues an issue command acts on: the open one when the detail
    /// pane has focus (`detail`), else the selection, else the cursor's,
    /// else the open one.
    pub fn targets(&self, detail: bool) -> Vec<String> {
        let open = self.view.selected.clone();
        if detail && let Some(o) = open {
            return vec![o];
        }
        if !self.selection.is_empty() {
            return self.selection.ids().to_vec();
        }
        self.cursor.clone().or(open).into_iter().collect()
    }

    // ----------------------------------------------------------- search

    /// The `/` box's text changed: instant results now, the full-text tier
    /// after the debounce. Any scan of the previous text is cancelled.
    pub fn set_search(&mut self, query: &str, cx: &mut Context<Self>) {
        let generation = self.search.generation + 1;
        self.search = SearchState {
            query: query.to_string(),
            generation,
            ..SearchState::default()
        };
        cx.notify();
        let Some(repo) = &self.repo else { return };
        if query.trim().is_empty() {
            return;
        }
        let r = repo.read(cx);
        let filter = (|| {
            let mut q = ListQuery::default();
            q.filter(query)?;
            Filter::resolve(&r.store, &self.identity, &q)
        })();
        let filter = match filter {
            Ok(f) => f,
            Err(e) => {
                self.search.error = Some(e.to_string().lines().collect::<Vec<_>>().join(" · "));
                return;
            }
        };
        let prepared = search::prepare(&r.index, &filter);
        self.search.hits = prepared.hits;
        if prepared.candidates.is_empty() {
            return;
        }
        self.search.scanning = true;
        let task = self.scan(
            prepared.candidates,
            filter.text,
            move |w, _, hits, cx| {
                if w.search.generation != generation {
                    return false;
                }
                search::merge(&mut w.search.hits, hits);
                cx.notify();
                true
            },
            move |w, cx| {
                if w.search.generation == generation {
                    w.search.scanning = false;
                    cx.notify();
                }
            },
            cx,
        );
        self.search.task = Some(task);
    }

    /// The search box closed: stop any scan.
    pub fn close_search(&mut self) {
        self.search = SearchState {
            generation: self.search.generation + 1,
            ..SearchState::default()
        };
    }

    /// Run the full-text tier over `cands` in background batches after
    /// the debounce. `batch` gets each batch's scanned paths and hits and
    /// says whether to go on; dropping the task stops it.
    fn scan(
        &mut self,
        cands: Vec<Candidate>,
        terms: Vec<String>,
        batch: impl Fn(&mut Workspace, Vec<PathBuf>, Vec<Hit>, &mut Context<Workspace>) -> bool
        + 'static,
        done: impl FnOnce(&mut Workspace, &mut Context<Workspace>) + 'static,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FULL_TEXT_DEBOUNCE).await;
            let ft = Arc::new(FullText::new(&terms));
            let cands = Arc::new(cands);
            let (mut start, mut size) = (0, FIRST_BATCH);
            while start < cands.len() {
                let end = (start + size).min(cands.len());
                let (c, f) = (cands.clone(), ft.clone());
                let hits = cx
                    .background_spawn(async move { search::scan(&c[start..end], &f) })
                    .await;
                let paths = cands[start..end].iter().map(|c| c.path.clone()).collect();
                match this.update(cx, |w, cx| batch(w, paths, hits, cx)) {
                    Ok(true) => {}
                    _ => return,
                }
                start = end;
                size = (size * 2).min(MAX_BATCH);
            }
            let _ = this.update(cx, done);
        })
    }

    /// Scan the filter bar's unscanned candidates, unless a scan runs.
    fn scan_filter_text(&mut self, cx: &mut Context<Self>) {
        if self.filter_text.running {
            return;
        }
        let ft = &self.filter_text;
        let todo: Vec<Candidate> = self
            .visible
            .candidates
            .iter()
            .filter(|c| !ft.scanned.contains(&c.path))
            .cloned()
            .collect();
        if todo.is_empty() {
            return;
        }
        let (generation, terms) = (ft.generation, ft.terms.clone());
        self.filter_text.running = true;
        let task = self.scan(
            todo,
            terms,
            move |w, paths, hits, cx| {
                if w.filter_text.generation != generation {
                    return false;
                }
                w.filter_text.scanned.extend(paths);
                let any = !hits.is_empty();
                w.filter_text.hits.extend(hits.into_iter().map(|h| h.path));
                if any {
                    w.refresh(false, cx);
                }
                true
            },
            move |w, cx| {
                if w.filter_text.generation == generation {
                    w.filter_text.running = false;
                    // Candidates may have grown meanwhile (scope change).
                    w.refresh(true, cx);
                }
            },
            cx,
        );
        self.filter_text.task = Some(task);
    }

    /// The filter bar's full-text tier is still running.
    pub fn filter_scanning(&self) -> bool {
        self.filter_text.running
    }

    // ------------------------------------------------------------- links

    /// Open a `lastissue://` URL (GUI-SPEC §10): switch to the labelled
    /// repo if needed, then select the issue and focus the detail pane,
    /// at once or as soon as the repo has loaded. A bad link is shown in
    /// the status line.
    pub fn open_url(&mut self, url: &str, cx: &mut Context<Self>) {
        if let Err(e) = crate::link::parse(url).and_then(|l| self.open_link(l, cx)) {
            self.error = Some(e);
            cx.notify();
        }
    }

    pub fn open_link(
        &mut self,
        link: crate::link::Link,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let repo = self
            .repos
            .iter()
            .find(|r| r.label == link.repo)
            .or_else(|| {
                self.repos
                    .iter()
                    .find(|r| r.label.eq_ignore_ascii_case(&link.repo))
            })
            .cloned()
            .ok_or_else(|| format!("no registered repo labelled {:?}", link.repo))?;
        let current = self.repo.as_ref().map(|r| r.read(cx).repo.root.clone());
        if current.as_ref() != Some(&repo.root) {
            self.open_repo(repo, cx);
        }
        self.pending_link = link.issue;
        self.open_pending_link(cx)
    }

    /// Select the pending linked issue if the repo has loaded: an exact
    /// short ID, else a unique prefix.
    fn open_pending_link(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(repo) = &self.repo else {
            return Ok(());
        };
        let r = repo.read(cx);
        if !r.loaded {
            return Ok(());
        }
        let Some(want) = self.pending_link.take() else {
            return Ok(());
        };
        let id = match r.index.by_id(&want) {
            Some(s) => s.id.clone(),
            None => {
                let mut m = r.index.iter().filter(|s| s.id.starts_with(&want));
                match (m.next(), m.next()) {
                    (Some(s), None) => s.id.clone(),
                    (None, _) => return Err(format!("no issue {want} in {}", r.repo.label)),
                    (Some(_), Some(_)) => return Err(format!("issue ID {want} is ambiguous")),
                }
            }
        };
        self.select(Some(id), cx);
        self.focus_detail = true;
        Ok(())
    }

    fn on_repo_changed(&mut self, ev: &RepoChanged, cx: &mut Context<Self>) {
        if self.pending_link.is_some()
            && let Err(e) = self.open_pending_link(cx)
        {
            self.error = Some(e);
        }
        // Changed files need a fresh full-text look.
        for p in ev.delta.changed.iter().chain(&ev.delta.removed) {
            self.filter_text.scanned.remove(p);
            self.filter_text.hits.remove(p);
        }
        self.recompute(cx);
        if ev.own {
            // Whoever wrote hands the open issue its new messages.
            return;
        }
        let (Some(doc), Some(repo)) = (&self.doc, &self.repo) else {
            return;
        };
        let (id, path) = {
            let d = doc.read(cx);
            (d.id.clone(), d.path.clone())
        };
        let delta = &ev.delta;
        if !delta.touches(&path) && !delta.projects {
            return;
        }
        let repo = repo.read(cx);
        match repo.index.by_id(&id) {
            // Changed in place, or moved (new path).
            Some(s) => {
                let p = s.path.clone();
                doc.update(cx, |d, cx| d.reload(p, cx));
            }
            // Deleted on disk.
            None => {
                self.doc = None;
                self.view.selected = None;
                cx.notify();
            }
        }
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.refresh(true, cx);
    }

    /// Re-evaluate the view; `scan`: also start the full-text tier for
    /// candidates not yet scanned.
    fn refresh(&mut self, scan: bool, cx: &mut Context<Self>) {
        let terms: Vec<String> = self
            .view
            .query()
            .map(|q| q.search.iter().map(|t| t.to_lowercase()).collect())
            .unwrap_or_default();
        if terms != self.filter_text.terms {
            // New text: drop the old scan and its hits.
            self.filter_text = FilterText {
                terms,
                generation: self.filter_text.generation + 1,
                ..FilterText::default()
            };
        }
        self.visible = match &self.repo {
            Some(repo) => {
                let repo = repo.read(cx);
                self.view.compute_with(
                    &repo.index,
                    &repo.store,
                    &self.identity,
                    &self.filter_text.hits,
                )
            }
            None => Visible::default(),
        };
        if let Some(repo) = &self.repo {
            let ix = &repo.read(cx).index;
            self.selection.retain(|id| ix.by_id(id).is_some());
        }
        if scan {
            self.scan_filter_text(cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_for_matches_registry_or_makes_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(dir.path()).unwrap();
        let reg = Repo {
            label: "mine".into(),
            root: root.clone(),
            global: false,
        };
        assert_eq!(repo_for(&root, std::slice::from_ref(&reg)).label, "mine");
        let adhoc = repo_for(&root, &[]);
        assert_eq!(adhoc.root, root);
        assert_eq!(adhoc.label, registry::label_of(&root));
    }

    use super::super::draft::FieldKind;
    use super::super::view_state::Mode;
    use gpui::TestAppContext;
    use lastissue::model::{Actor, State};
    use lastissue::ops::{self, IssueUpdate, NewIssue};
    use std::time::Duration;

    fn actor(from: &str) -> Identity {
        Identity::fixed(Actor {
            from: from.into(),
            sender: None,
        })
    }

    fn events(s: &store::Store, id: &str) -> Vec<(String, String)> {
        let r = s.resolve_issue(id).unwrap();
        s.read(&r)
            .unwrap()
            .iter()
            .filter(|m| m.get("X-Issue-Event").is_some())
            .map(|m| (m.get("From").unwrap().to_string(), m.body.clone()))
            .collect()
    }

    /// Headless end to end: title/body drafts save once after the
    /// debounce, a picker edit and a board move coalesce into the same
    /// event, and an external change under a draft is a conflict that
    /// pauses saving until "keep mine".
    #[gpui::test]
    fn edits_save_debounced_coalesce_and_detect_conflicts(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(dir.path()).unwrap();
        let s = store::Store::new(root.clone());
        let ada = "Ada <ada@example.com>";
        let id = ops::create_issue(
            &s,
            &actor(ada),
            &NewIssue {
                title: "Old title".into(),
                body: "Old body".into(),
                state: Some(State::Todo),
                ..NewIssue::default()
            },
        )
        .unwrap()
        .issue
        .id;

        let ws = cx.new(|cx| {
            let mut w = Workspace::new(Some(root.clone()), cx);
            w.identity = Arc::new(actor(ada));
            w.drafts = Some(Drafts::new(dir.path().join("drafts")));
            w
        });
        cx.run_until_parked();
        ws.update(cx, |w, cx| {
            w.update_view(|v| v.mode = Mode::Board, cx);
            w.select(Some(id.clone()), cx);
        });
        cx.run_until_parked();
        let doc = ws.read_with(cx, |w, _| w.doc.clone().unwrap());
        assert!(doc.read_with(cx, |d, _| d.title.is_some()));

        // Typing: nothing is written before the debounce fires.
        doc.update(cx, |d, cx| {
            d.edit(FieldKind::Title, "New", cx);
            d.edit(FieldKind::Title, "New title", cx);
            d.edit(FieldKind::Body, "New body", cx);
        });
        cx.run_until_parked();
        assert!(events(&s, &id).is_empty());
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        let r = s.resolve_issue(&id).unwrap();
        let head = s.read(&r).unwrap()[0].clone();
        assert_eq!(head.get("Subject"), Some("New title"));
        assert_eq!(head.body, "New body");
        assert!(!doc.read_with(cx, |d, _| d.is_dirty()));

        // A picker edit and `⇧l` on the board join the same burst.
        ws.update(cx, |w, cx| {
            w.apply_edit(id.clone(), Edit::AddLabel("bug".into()), cx);
        });
        cx.run_until_parked();
        ws.update(cx, |w, cx| w.move_selected(1, cx));
        cx.run_until_parked();
        assert_eq!(
            events(&s, &id),
            [(
                ada.to_string(),
                "title: Old title → New title\nbody: edited\nlabels: (none) → bug\nstate: todo → in-progress".to_string()
            )]
        );
        assert_eq!(
            ws.read_with(cx, |w, cx| w
                .repo
                .as_ref()
                .unwrap()
                .read(cx)
                .index
                .by_id(&id)
                .unwrap()
                .state),
            State::InProgress
        );

        // Someone else rewrites the body under an open draft.
        doc.update(cx, |d, cx| d.edit(FieldKind::Body, "mine", cx));
        let bob = actor("Bob <bob@example.com>");
        let u = IssueUpdate {
            body: Some("theirs".into()),
            ..IssueUpdate::default()
        };
        ops::update_issue(&s, &bob, &id, &u).unwrap();
        // The watcher would do this; paths are keyed by the canonical root.
        let path = doc.read_with(cx, |d, _| d.path.clone());
        doc.update(cx, |d, cx| d.reload(path, cx));
        cx.run_until_parked();
        assert!(
            doc.read_with(cx, |d, _| d.has_conflict()),
            "{:?} {:?}",
            doc.read_with(cx, |d, _| (d.body.clone(), d.error.clone(), d.saving)),
            events(&s, &id)
        );
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        let body = |s: &store::Store| {
            s.read(&s.resolve_issue(&id).unwrap()).unwrap()[0]
                .body
                .clone()
        };
        assert_eq!(body(&s), "theirs", "no autosave over a conflict");
        doc.update(cx, |d, cx| d.keep_mine(cx));
        cx.run_until_parked();
        assert_eq!(body(&s), "mine");
        let ev = events(&s, &id);
        assert_eq!(ev.len(), 3, "Bob's change ended Ada's burst: {ev:?}");
        assert_eq!(ev[2], (ada.to_string(), "body: edited".to_string()));
    }

    /// Each keystroke cancels the previous scan: only the last query's
    /// full-text hits land. The filter bar's tier scans only candidates it
    /// hasn't scanned yet, and the bulk path edits every target.
    #[gpui::test]
    fn search_generations_and_filter_text(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(&dir.path().join("repo")).unwrap();
        let s = store::Store::new(root.clone());
        let ada = actor("Ada <ada@example.com>");
        let mut ids = Vec::new();
        for (t, b) in [
            ("One", "apple pie"),
            ("Two", "banana split"),
            ("Apple tart", "x"),
        ] {
            let n = NewIssue {
                title: t.into(),
                body: b.into(),
                state: Some(State::Todo),
                ..NewIssue::default()
            };
            ids.push(ops::create_issue(&s, &ada, &n).unwrap().issue.id);
        }
        let ws = cx.new(|cx| {
            let mut w = Workspace::new(Some(root.clone()), cx);
            w.for_test(actor("Ada <ada@example.com>"), &dir.path().join("drafts"));
            w
        });
        cx.run_until_parked();
        let wait = |cx: &mut TestAppContext| {
            cx.executor()
                .advance_clock(FULL_TEXT_DEBOUNCE + Duration::from_millis(5));
            cx.run_until_parked();
        };

        ws.update(cx, |w, cx| w.set_search("banana", cx));
        cx.run_until_parked();
        ws.update(cx, |w, cx| w.set_search("apple", cx));
        // Instant tier at once: the title match.
        let hits = |cx: &mut TestAppContext| {
            ws.read_with(cx, |w, _| {
                w.search
                    .hits
                    .iter()
                    .map(|h| h.id.clone())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(hits(cx), [ids[2].clone()]);
        wait(cx);
        assert_eq!(hits(cx), [ids[2].clone(), ids[0].clone()], "no banana");
        assert!(!ws.read_with(cx, |w, _| w.search.scanning));
        ws.update(cx, |w, cx| w.set_search("state:nope", cx));
        assert!(ws.read_with(cx, |w, _| w.search.error.is_some()));
        ws.update(cx, |w, _| w.close_search());

        // The filter bar: body hits after the debounce.
        ws.update(cx, |w, cx| {
            w.update_view(|v| v.filter = "banana".into(), cx)
        });
        let shown = |cx: &mut TestAppContext| ws.read_with(cx, |w, _| w.visible.total);
        assert_eq!(shown(cx), 0);
        assert!(ws.read_with(cx, |w, _| w.filter_scanning()));
        wait(cx);
        assert_eq!(shown(cx), 1);
        assert!(!ws.read_with(cx, |w, _| w.filter_scanning()));
        // Re-evaluating with the same text needs no new scan.
        ws.update(cx, |w, cx| {
            w.update_view(|v| v.sort = lastissue::query::Sort::Created, cx)
        });
        assert!(!ws.read_with(cx, |w, _| w.filter_scanning()));
        assert_eq!(shown(cx), 1);

        // Bulk: one edit to every selected issue.
        ws.update(cx, |w, cx| {
            w.update_view(|v| v.filter.clear(), cx);
            w.select_all(cx);
            let t = w.targets(false);
            assert_eq!(t.len(), 3);
            w.apply_to(&t, &Edit::AddLabel("bulk".into()), cx);
        });
        cx.run_until_parked();
        let labelled = ws.read_with(cx, |w, cx| {
            w.repo
                .as_ref()
                .unwrap()
                .read(cx)
                .index
                .iter()
                .filter(|s| s.labels == ["bulk"])
                .count()
        });
        assert_eq!(labelled, 3);
    }

    /// Manual check against a real store (`lastissue issue show` after):
    /// `LASTISSUE_GUI_E2E=<root> LASTISSUE_GUI_E2E_ISSUE=<id> cargo test -p gui
    /// e2e_on_real_store -- --ignored`. Uses the root's real identity, as
    /// the app does.
    #[gpui::test]
    #[ignore]
    fn e2e_on_real_store(cx: &mut TestAppContext) {
        let root = PathBuf::from(std::env::var("LASTISSUE_GUI_E2E").unwrap());
        let id = std::env::var("LASTISSUE_GUI_E2E_ISSUE").unwrap();
        let ws = cx.new(|cx| Workspace::new(Some(root), cx));
        cx.run_until_parked();
        ws.update(cx, |w, cx| w.select(Some(id.clone()), cx));
        cx.run_until_parked();
        let doc = ws.read_with(cx, |w, _| w.doc.clone().unwrap());
        doc.update(cx, |d, cx| {
            for t in [
                "Login crashes on empty",
                "Login crashes on empty password (Safari)",
            ] {
                d.edit(FieldKind::Title, t, cx);
            }
            d.edit(
                FieldKind::Body,
                "Steps: open login, submit an empty password.\n\nSeen in Safari 18.",
                cx,
            );
        });
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        ws.update(cx, |w, cx| {
            w.apply_edit(id.clone(), Edit::State(State::InProgress), cx);
            w.apply_edit(
                id.clone(),
                Edit::Priority(Some(lastissue::model::Priority::Urgent)),
                cx,
            );
        });
        cx.run_until_parked();
        assert!(ws.read_with(cx, |w, _| w.error.is_none()));
    }

    /// `lastissue://repo/<label>/issue/<id>` switches repo, waits for its
    /// scan, selects the issue and asks the view to focus the detail pane;
    /// bad links land in the status line.
    #[gpui::test]
    fn url_switches_repo_and_opens_issue(cx: &mut TestAppContext) {
        let ada = actor("Ada <ada@example.com>");
        let mk = |name: &str, title: &str| {
            let dir = tempfile::tempdir().unwrap();
            let root = store::init(&dir.path().join(name)).unwrap();
            let s = store::Store::new(root.clone());
            let new = NewIssue {
                title: title.into(),
                ..NewIssue::default()
            };
            let id = ops::create_issue(&s, &ada, &new).unwrap().issue.id;
            (dir, root, id)
        };
        let (_d1, one, _) = mk("one", "First");
        let (_d2, two, id2) = mk("two", "Second");
        let ws = cx.new(|cx| {
            let mut w = Workspace::new(Some(one.clone()), cx);
            w.repos = vec![
                Repo {
                    label: "one".into(),
                    root: one.clone(),
                    global: false,
                },
                Repo {
                    label: "two".into(),
                    root: two.clone(),
                    global: false,
                },
            ];
            w
        });
        cx.run_until_parked();

        // A prefix of the ID, other case: resolved once repo two has loaded.
        let url = format!("lastissue://repo/two/issue/{}", id2[..5].to_uppercase());
        ws.update(cx, |w, cx| w.open_url(&url, cx));
        cx.run_until_parked();
        ws.read_with(cx, |w, cx| {
            assert_eq!(w.repo.as_ref().unwrap().read(cx).repo.label, "two");
            assert_eq!(w.view.selected.as_deref(), Some(id2.as_str()));
            assert_eq!(w.doc.as_ref().unwrap().read(cx).id, id2);
            assert!(w.focus_detail);
            assert!(w.error.is_none(), "{:?}", w.error);
        });

        for (bad, msg) in [
            ("lastissue://repo/nope/issue/abc", "no registered repo"),
            ("lastissue://repo/two/issue/zzzzzz", "no issue zzzzzz"),
            ("https://example.com", "not a lastissue link"),
        ] {
            ws.update(cx, |w, cx| w.open_url(bad, cx));
            cx.run_until_parked();
            let err = ws.read_with(cx, |w, _| w.error.clone()).unwrap_or_default();
            assert!(err.contains(msg), "{bad}: {err}");
        }
    }
}
