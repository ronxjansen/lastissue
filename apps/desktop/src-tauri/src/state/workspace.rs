//! The window's state (GUI-SPEC §3, §8): the registry's repos, the open
//! repo (index, watcher, identity), the view state and its evaluated
//! result, the open issue, and both search tiers.
//!
//! One [`Hub`] holds it behind a mutex. Commands lock it briefly; disk I/O
//! and scans run on plain threads with the lock released, then lock again
//! to fold results in. Each repo open and each search gets a generation
//! number, so work for something already replaced drops out. Every change
//! the front-end needs is pushed through the [`Sink`].

use super::doc::Doc;
use super::draft::{Drafts, FieldKind};
use super::edit::{self, Applied, Edit, Op};
use super::search::{self, Candidate, FullText, Hit};
use super::view_state::{Mode, ViewState, Visible};
use super::watcher;
use crate::dto::{self, DocDto, FieldDto, IndexDelta, MetaDto, SearchDto, SummaryDto, VisibleDto};
use crate::link::Link;
use lastissue::error::Result;
use lastissue::identity::Identity;
use lastissue::index::{self, Delta, Index, Reload};
use lastissue::query::{Filter, ListQuery};
use lastissue::registry::{self, Repo};
use lastissue::source::FsScan;
use lastissue::store::{self, Store};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The full-text tier starts after this long without a keystroke (§6).
pub const FULL_TEXT_DEBOUNCE: Duration = Duration::from_millis(150);
/// Files per background batch: small first, so early results stream in,
/// then doubling to amortize the round trips.
const FIRST_BATCH: usize = 64;
const MAX_BATCH: usize = 2048;

/// Where events for the front-end go (Tauri's emitter; a recorder in tests).
pub trait Sink: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

pub mod events {
    pub const META: &str = "meta";
    pub const INDEX: &str = "index";
    pub const VISIBLE: &str = "visible";
    pub const DOC: &str = "doc";
    pub const SEARCH: &str = "search";
    pub const KEYMAP: &str = "keymap";
    /// A link opened an issue: move focus to the detail pane.
    pub const FOCUS_DETAIL: &str = "focus-detail";
}

pub struct Hub {
    ws: Mutex<Workspace>,
    pub sink: Arc<dyn Sink>,
}

pub type SharedHub = Arc<Hub>;

impl Hub {
    pub fn new(ws: Workspace, sink: Arc<dyn Sink>) -> SharedHub {
        Arc::new(Hub {
            ws: Mutex::new(ws),
            sink,
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Workspace> {
        self.ws.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, event: &str, payload: &impl Serialize) {
        emit(&*self.sink, event, payload);
    }
}

fn emit(sink: &dyn Sink, event: &str, payload: &impl Serialize) {
    match serde_json::to_value(payload) {
        Ok(v) => sink.emit(event, v),
        Err(e) => eprintln!("lastissue-gui: cannot encode {event}: {e}"),
    }
}

fn error_line(e: &lastissue::error::Error) -> String {
    e.to_string().trim().lines().collect::<Vec<_>>().join(" · ")
}

/// One open issues root: its index and what keeps it fresh.
pub struct RepoModel {
    pub generation: u64,
    pub repo: Repo,
    pub store: Store,
    source: Arc<FsScan>,
    /// Serializes this window's writes (core writes are atomic per file,
    /// but a read-modify-write must not interleave with another).
    write_lock: Arc<Mutex<()>>,
    pub index: Index,
    pub loaded: bool,
    pub errors: BTreeMap<PathBuf, String>,
    pub failure: Option<String>,
    _watcher: Option<watcher::RepoWatcher>,
}

/// The filter bar's full-text tier: which candidate files have been
/// scanned for the current terms, and which matched.
#[derive(Default)]
struct FilterText {
    terms: Vec<String>,
    hits: HashSet<PathBuf>,
    scanned: HashSet<PathBuf>,
    running: bool,
    generation: u64,
}

/// The `/` search box (GUI-SPEC §6).
#[derive(Default)]
struct SearchState {
    generation: u64,
    query: String,
    hits: Vec<Hit>,
    scanning: bool,
    error: Option<String>,
}

pub struct Workspace {
    pub repos: Vec<Repo>,
    pub repo: Option<RepoModel>,
    repo_generation: u64,
    pub identity: Arc<Identity>,
    pub view: ViewState,
    pub visible: Visible,
    pub doc: Option<Doc>,
    doc_generation: u64,
    search: SearchState,
    filter_text: FilterText,
    pub notice: Option<String>,
    pub error: Option<String>,
    drafts: Option<Drafts>,
    pending_link: Option<String>,
    watch: bool,
}

/// The identity for a repo (GUI-SPEC §4): git config in its directory,
/// never the `$USER@host` fallback.
fn identity_for(root: &Path) -> Identity {
    Identity::new(None, None).in_dir(root).without_fallback()
}

/// The repo to open first: an explicit root (argument, `LASTISSUE_DIR`),
/// else discovery from the current directory up to the global root.
pub fn initial_repo(arg: Option<&Path>, repos: &[Repo]) -> Result<Repo> {
    let env = std::env::var_os("LASTISSUE_DIR").map(PathBuf::from);
    let root = store::discover(arg.or(env.as_deref()))?;
    Ok(repo_for(&root, repos))
}

/// The registry entry for `root`, or an ad-hoc one.
pub fn repo_for(root: &Path, repos: &[Repo]) -> Repo {
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
    pub fn new(drafts: Option<Drafts>, watch: bool) -> Self {
        Workspace {
            repos: Vec::new(),
            repo: None,
            repo_generation: 0,
            identity: Arc::new(identity_for(Path::new("."))),
            view: ViewState::default(),
            visible: Visible::default(),
            doc: None,
            doc_generation: 0,
            search: SearchState::default(),
            filter_text: FilterText::default(),
            notice: None,
            error: None,
            drafts,
            pending_link: None,
            watch,
        }
    }

    /// Tests: a fixed identity.
    #[cfg(test)]
    pub fn with_identity(mut self, me: Identity) -> Self {
        self.identity = Arc::new(me);
        self
    }

    fn me(&self) -> Option<&str> {
        self.identity.actor().ok().map(|a| a.from.as_str())
    }

    pub fn meta(&self) -> MetaDto {
        let (me, me_error) = match self.identity.actor() {
            Ok(a) => (Some(a.from.clone()), None),
            Err(e) => (None, Some(error_line(&e))),
        };
        let current = self.repo.as_ref().and_then(|r| {
            self.repos
                .iter()
                .position(|x| x.root == r.repo.root)
        });
        MetaDto {
            repos: self.repos.iter().map(dto::RepoDto::from).collect(),
            current,
            me,
            me_error,
            loaded: self.repo.as_ref().is_some_and(|r| r.loaded),
            notice: self.notice.clone(),
            failure: self.repo.as_ref().and_then(|r| r.failure.clone()),
            error: self.error.clone(),
            load_errors: self
                .repo
                .as_ref()
                .map(|r| {
                    r.errors
                        .iter()
                        .map(|(p, e)| format!("{}: {e}", r.store.rel(p)))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn issues(&self) -> Vec<SummaryDto> {
        self.repo
            .as_ref()
            .map(|r| r.index.iter().map(|s| SummaryDto::new(&r.store, s)).collect())
            .unwrap_or_default()
    }

    pub fn projects(&self) -> Vec<dto::ProjectDto> {
        self.repo
            .as_ref()
            .map(|r| dto::projects(&r.index))
            .unwrap_or_default()
    }

    pub fn visible_dto(&self) -> VisibleDto {
        let store = self.repo.as_ref().map_or_else(
            || Store::new(PathBuf::new()),
            |r| r.store.clone(),
        );
        VisibleDto::new(&store, &self.view, &self.visible, self.filter_text.running)
    }

    pub fn search_dto(&self) -> SearchDto {
        let hits = match &self.repo {
            Some(r) => self
                .search
                .hits
                .iter()
                .map(|h| dto::HitDto::new(&r.store, h))
                .collect(),
            None => Vec::new(),
        };
        SearchDto {
            generation: self.search.generation,
            query: self.search.query.clone(),
            hits,
            scanning: self.search.scanning,
            error: self.search.error.clone(),
        }
    }

    pub fn doc_dto(&self) -> Option<DocDto> {
        let d = self.doc.as_ref()?;
        let r = self.repo.as_ref()?;
        let field = |k: FieldKind| {
            d.field(k).map(|f| FieldDto {
                value: f.value().to_string(),
                dirty: f.is_dirty(),
                conflict: f.conflict.clone(),
            })
        };
        let s = r.index.get(&d.path).or_else(|| r.index.by_id(&d.id));
        let (parent, blocked_by, related) = match s {
            Some(s) => dto::relations(&r.store, &r.index, s),
            None => (None, Vec::new(), Vec::new()),
        };
        let watchers = d
            .msgs
            .as_ref()
            .and_then(|m| m.first())
            .map(|h| lastissue::model::list_of(h, "Cc"))
            .unwrap_or_default();
        Some(DocDto {
            key: dto::key_of(&r.store, &d.path),
            id: d.id.clone(),
            message_id: d.message_id(),
            loading: d.msgs.is_none() && d.error.is_none(),
            error: d.error.clone(),
            save_error: d.save_error.clone(),
            title: field(FieldKind::Title),
            body: field(FieldKind::Body),
            leftover: d.leftover.clone(),
            reset: d.reset,
            summary: s.map(|s| SummaryDto::new(&r.store, s)),
            watchers,
            parent,
            blocked_by,
            related,
            activity: d.msgs.as_deref().map(dto::activity).unwrap_or_default(),
            link: Some(
                Link {
                    repo: r.repo.label.clone(),
                    issue: Some(d.id.clone()),
                }
                .to_url(),
            ),
        })
    }

    pub fn snapshot(&self, keymap: crate::keymap::UserKeymap) -> dto::Snapshot {
        dto::Snapshot {
            meta: self.meta(),
            issues: self.issues(),
            projects: self.projects(),
            visible: self.visible_dto(),
            doc: self.doc_dto(),
            search: self.search_dto(),
            keymap,
        }
    }

    /// The issues an issue command acts on, in the order given, that exist.
    fn existing(&self, ids: &[String]) -> Vec<String> {
        let Some(r) = &self.repo else {
            return Vec::new();
        };
        ids.iter()
            .filter(|id| r.index.by_id(id).is_some())
            .cloned()
            .collect()
    }

    /// Re-evaluate the view. Returns the full-text candidates still to
    /// scan when `scan` is set and no scan runs.
    fn refresh(&mut self) {
        let terms: Vec<String> = self
            .view
            .query()
            .map(|q| q.search.iter().map(|t| t.to_lowercase()).collect())
            .unwrap_or_default();
        if terms != self.filter_text.terms {
            self.filter_text = FilterText {
                terms,
                generation: self.filter_text.generation + 1,
                ..FilterText::default()
            };
        }
        self.visible = match &self.repo {
            Some(r) => {
                self.view
                    .compute_with(&r.index, &r.store, &self.identity, &self.filter_text.hits)
            }
            None => Visible::default(),
        };
    }
}

// ---------------------------------------------------------------- repos

/// Load the registry and open the first repo (GUI start-up).
pub fn start(hub: &SharedHub, arg: Option<PathBuf>) {
    let (mut repos, mut notice) = match registry::load() {
        Ok(r) => (r, None),
        Err(e) => (Vec::new(), Some(error_line(&e))),
    };
    match initial_repo(arg.as_deref(), &repos) {
        Ok(repo) => {
            if !repos.iter().any(|r| r.root == repo.root) {
                repos.insert(0, repo.clone());
            }
            hub.lock().repos = repos;
            open_repo(hub, repo);
        }
        Err(e) => {
            let mut ws = hub.lock();
            ws.repos = repos;
            if notice.is_none() {
                notice = Some(error_line(&e));
            }
            ws.notice = notice;
        }
    }
}

/// Switch the window to `repo`: a fresh index, loaded in the background,
/// then kept live by the watcher.
pub fn open_repo(hub: &SharedHub, repo: Repo) {
    let root = repo
        .root
        .canonicalize()
        .unwrap_or_else(|_| repo.root.clone());
    let store = Store::new(root);
    let source = Arc::new(FsScan::new(store.clone()));
    let mut ws = hub.lock();
    let watching = if ws.watch {
        watcher::watch(&store.root)
    } else {
        Err(notify::Error::generic("not watched"))
    };
    let (w, rx, failure) = match watching {
        Ok((w, rx)) => (Some(w), Some(rx), None),
        Err(e) if ws.watch => (None, None, Some(format!("not watching: {e}"))),
        Err(_) => (None, None, None),
    };
    ws.repo_generation += 1;
    let generation = ws.repo_generation;
    ws.identity = Arc::new(identity_for(&repo.root));
    ws.repo = Some(RepoModel {
        generation,
        repo,
        store: store.clone(),
        source: source.clone(),
        write_lock: Arc::new(Mutex::new(())),
        index: Index::default(),
        loaded: false,
        errors: BTreeMap::new(),
        failure,
        _watcher: w,
    });
    ws.doc = None;
    ws.view.selected = None;
    ws.search = SearchState {
        generation: ws.search.generation + 1,
        ..SearchState::default()
    };
    ws.filter_text = FilterText {
        generation: ws.filter_text.generation + 1,
        ..FilterText::default()
    };
    ws.pending_link = None;
    ws.notice = None;
    ws.error = None;
    ws.refresh();
    hub.emit(events::META, &ws.meta());
    hub.emit(
        events::INDEX,
        &IndexDelta {
            full: true,
            changed: Vec::new(),
            removed: Vec::new(),
            projects: Some(Vec::new()),
        },
    );
    hub.emit(events::VISIBLE, &ws.visible_dto());
    hub.emit(events::DOC, &ws.doc_dto());
    hub.emit(events::SEARCH, &ws.search_dto());
    drop(ws);

    let hub = hub.clone();
    std::thread::Builder::new()
        .name("lastissue-repo".into())
        .spawn(move || {
            let r = index::scan(&store, &*source);
            if !apply(&hub, generation, r, false) {
                return;
            }
            let Some(rx) = rx else { return };
            // Ends when the watcher (owned by the RepoModel) is dropped.
            for batch in rx {
                let r = if batch.rescan {
                    index::scan(&store, &*source)
                } else {
                    index::reload(&store, &*source, &batch.paths())
                };
                if !apply(&hub, generation, r, false) {
                    return;
                }
            }
        })
        .expect("spawn repo thread");
}

/// Fold a disk read into the open repo (if it's still `generation`) and
/// push what changed. Returns false once the repo was replaced.
fn apply(hub: &SharedHub, generation: u64, r: Result<Reload>, own: bool) -> bool {
    let mut ws = hub.lock();
    let Some(repo) = ws.repo.as_mut().filter(|r| r.generation == generation) else {
        return false;
    };
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            repo.failure = Some(error_line(&e));
            repo.loaded = true;
            hub.emit(events::META, &ws.meta());
            return true;
        }
    };
    let full = matches!(r, Reload::Full { .. });
    match &r {
        Reload::Full { .. } => repo.errors.clear(),
        Reload::Paths { loaded, gone, .. } => {
            for p in loaded.iter().map(|s| &s.path).chain(gone) {
                repo.errors.remove(p);
            }
        }
    }
    for e in r.errors() {
        repo.errors.insert(e.path.clone(), e.message.clone());
    }
    let first = !repo.loaded;
    repo.loaded = true;
    let delta = repo.index.apply(r);
    if !first && delta.is_empty() {
        return true;
    }
    let store = repo.store.clone();
    let index_delta = IndexDelta {
        full: full || first,
        changed: if full || first {
            repo.index.iter().map(|s| SummaryDto::new(&store, s)).collect()
        } else {
            delta
                .changed
                .iter()
                .filter_map(|p| repo.index.get(p))
                .map(|s| SummaryDto::new(&store, s))
                .collect()
        },
        removed: delta.removed.iter().map(|p| dto::key_of(&store, p)).collect(),
        projects: (delta.projects || first).then(|| dto::projects(&repo.index)),
    };
    hub.emit(events::INDEX, &index_delta);
    on_repo_changed(hub, &mut ws, &delta, own, first);
    true
}

fn on_repo_changed(hub: &SharedHub, ws: &mut Workspace, delta: &Delta, own: bool, first: bool) {
    if ws.pending_link.is_some()
        && let Err(e) = open_pending_link(hub, ws)
    {
        ws.error = Some(e);
    }
    // Changed files need a fresh full-text look.
    for p in delta.changed.iter().chain(&delta.removed) {
        ws.filter_text.scanned.remove(p);
        ws.filter_text.hits.remove(p);
    }
    // Keep the selection's ID valid.
    if let Some(r) = &ws.repo
        && let Some(sel) = &ws.view.selected
        && r.index.by_id(sel).is_none()
        && ws.doc.is_none()
    {
        ws.view.selected = None;
    }
    recompute(hub, ws, true);
    if first {
        hub.emit(events::META, &ws.meta());
    }
    if own {
        // Whoever wrote hands the open issue its new messages.
        return;
    }
    let (Some(doc), Some(repo)) = (&ws.doc, &ws.repo) else {
        return;
    };
    if !delta.touches(&doc.path) && !delta.projects {
        return;
    }
    match repo.index.by_id(&doc.id) {
        Some(s) => {
            let p = s.path.clone();
            load_doc(hub, ws, p);
        }
        None => {
            ws.doc = None;
            ws.view.selected = None;
            hub.emit(events::DOC, &ws.doc_dto());
        }
    }
}

/// Re-evaluate the view and push it; `scan`: also start the full-text
/// tier for candidates not yet scanned.
fn recompute(hub: &SharedHub, ws: &mut Workspace, scan: bool) {
    ws.refresh();
    hub.emit(events::VISIBLE, &ws.visible_dto());
    if scan {
        scan_filter_text(hub, ws);
    }
}

// ------------------------------------------------------------------ view

pub fn set_view(hub: &SharedHub, v: &dto::ViewDto) -> std::result::Result<(), String> {
    let mut ws = hub.lock();
    let before = ws.view.clone();
    v.apply(&mut ws.view)?;
    if ws.view != before {
        recompute(hub, &mut ws, true);
    }
    Ok(())
}

/// Open an issue in the detail pane (`None` closes it).
pub fn select(hub: &SharedHub, id: Option<String>) {
    let mut ws = hub.lock();
    select_locked(hub, &mut ws, id);
}

fn select_locked(hub: &SharedHub, ws: &mut Workspace, id: Option<String>) {
    if id.is_some() && ws.doc.as_ref().map(|d| &d.id) == id.as_ref() {
        return;
    }
    if ws.doc.as_ref().is_some_and(Doc::is_dirty) {
        save_doc_locked(hub, ws);
    }
    ws.view.selected = id.clone();
    ws.doc = None;
    if let (Some(id), Some(r)) = (id, &ws.repo)
        && let Some(s) = r.index.by_id(&id)
    {
        let path = s.path.clone();
        ws.doc_generation += 1;
        ws.doc = Some(Doc::new(ws.doc_generation, id, path.clone()));
        load_doc(hub, ws, path);
    }
    hub.emit(events::DOC, &ws.doc_dto());
}

/// Read the open issue from `path` in the background.
fn load_doc(hub: &SharedHub, ws: &mut Workspace, path: PathBuf) {
    let (Some(doc), Some(r)) = (ws.doc.as_mut(), &ws.repo) else {
        return;
    };
    doc.path = path.clone();
    let (generation, store) = (doc.generation, r.store.clone());
    let hub = hub.clone();
    std::thread::spawn(move || {
        let r = store
            .issue_ref_for(&path)
            .ok_or_else(|| {
                lastissue::error::Error::not_found("issue-not-found")
                    .with("Path", path.display().to_string())
            })
            .and_then(|r| store.read(&r));
        let mut ws = hub.lock();
        let drafts = ws.drafts.clone();
        let Some(doc) = ws.doc.as_mut().filter(|d| d.generation == generation) else {
            return;
        };
        match r {
            Ok(msgs) if !msgs.is_empty() => doc.loaded(msgs, drafts.as_ref()),
            Ok(_) => doc.error = Some("the issue file is empty".into()),
            Err(e) => doc.error = Some(error_line(&e)),
        }
        hub.emit(events::DOC, &ws.doc_dto());
    });
}

// ----------------------------------------------------------------- doc

/// The editor's text changed. Returns whether a save is due (the front-end
/// then calls [`save_doc`] after its debounce).
pub fn doc_edit(hub: &SharedHub, id: &str, kind: FieldKind, text: &str) -> bool {
    let mut ws = hub.lock();
    let drafts = ws.drafts.clone();
    let Some(doc) = ws.doc.as_mut().filter(|d| d.id == id) else {
        return false;
    };
    let was = doc.is_dirty();
    let due = doc.edit(kind, text, drafts.as_ref());
    if due != was {
        hub.emit(events::DOC, &ws.doc_dto());
    }
    due
}

pub fn save_doc(hub: &SharedHub) {
    let mut ws = hub.lock();
    save_doc_locked(hub, &mut ws);
}

/// Write the open issue's drafts (title, then body). Runs on a thread;
/// each write goes through [`run_locked_op`]'s path.
fn save_doc_locked(hub: &SharedHub, ws: &mut Workspace) {
    let drafts = ws.drafts.clone();
    let Some(doc) = ws.doc.as_ref() else { return };
    let edits = doc.to_save(drafts.as_ref());
    if edits.is_empty() {
        return;
    }
    let (id, generation) = (doc.id.clone(), doc.generation);
    let hub = hub.clone();
    std::thread::spawn(move || {
        let mut error = None;
        for (kind, text) in edits {
            let edit = match kind {
                FieldKind::Title => Edit::Title(text.clone()),
                FieldKind::Body => Edit::Body(text.clone()),
            };
            match run(&hub, Op::Update {
                id: id.clone(),
                edit,
            }) {
                Ok(_) => {
                    let mut ws = hub.lock();
                    let drafts = ws.drafts.clone();
                    if let Some(d) = ws.doc.as_mut().filter(|d| d.generation == generation) {
                        d.saved(kind, &text, drafts.as_ref());
                    }
                }
                Err(e) => error = Some(e),
            }
        }
        let mut ws = hub.lock();
        if let Some(d) = ws.doc.as_mut().filter(|d| d.generation == generation) {
            d.save_error = error;
        }
        hub.emit(events::DOC, &ws.doc_dto());
    });
}

/// The conflict banner, the leftover-draft banner.
pub fn doc_action(hub: &SharedHub, action: &str) -> std::result::Result<(), String> {
    let mut ws = hub.lock();
    let drafts = ws.drafts.clone();
    let Some(doc) = ws.doc.as_mut() else {
        return Ok(());
    };
    let save = match action {
        "keepMine" => {
            doc.keep_mine();
            true
        }
        "useTheirs" => {
            doc.use_theirs(drafts.as_ref());
            false
        }
        "restoreLeftover" => doc.restore_leftover(drafts.as_ref()),
        "discardLeftover" => {
            doc.discard_leftover(drafts.as_ref());
            false
        }
        a => return Err(format!("unknown doc action {a:?}")),
    };
    if save {
        save_doc_locked(hub, &mut ws);
    }
    hub.emit(events::DOC, &ws.doc_dto());
    Ok(())
}

// ---------------------------------------------------------------- writes

/// Run a write through core, off the UI thread and outside the workspace
/// lock (but under the repo's write lock). The result is folded into the
/// index at once, so the watcher's echo is a no-op; the open issue takes
/// its new messages. A failure lands in the status line and is returned.
pub fn run(hub: &SharedHub, op: Op) -> std::result::Result<Applied, String> {
    let (store, src, lock, me, generation) = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else {
            return Err("no repo is open".into());
        };
        (
            r.store.clone(),
            r.source.clone(),
            r.write_lock.clone(),
            ws.identity.clone(),
            r.generation,
        )
    };
    let r = {
        let _g = lock.lock().unwrap_or_else(PoisonError::into_inner);
        edit::run(&store, &me, &*src, &op)
    };
    match r {
        Ok(a) => {
            apply(hub, generation, Ok(a.reload.clone()), true);
            let mut ws = hub.lock();
            ws.error = None;
            if a.deleted && ws.view.selected.as_deref() == Some(a.id.as_str()) {
                ws.doc = None;
                ws.view.selected = None;
            }
            if let Some(doc) = ws.doc.as_mut()
                && a.written
                && doc.id == a.id
            {
                doc.adopt_own(a.path.clone(), a.msgs.clone());
            }
            hub.emit(events::META, &ws.meta());
            hub.emit(events::DOC, &ws.doc_dto());
            Ok(a)
        }
        Err(e) => {
            let msg = error_line(&e);
            let mut ws = hub.lock();
            ws.error = Some(msg.clone());
            hub.emit(events::META, &ws.meta());
            Err(msg)
        }
    }
}

/// The same change to several issues (pickers, bulk edit, drag and drop).
pub fn apply_to(hub: &SharedHub, ids: &[String], edit: &Edit) -> std::result::Result<(), String> {
    let ids = hub.lock().existing(ids);
    let mut first_err = None;
    for id in ids {
        if let Err(e) = run(hub, Op::Update {
            id,
            edit: edit.clone(),
        }) {
            first_err.get_or_insert(e);
        }
    }
    first_err.map_or(Ok(()), Err)
}

/// A card dropped on a column (state or priority, per the grouping).
pub fn drop_card(
    hub: &SharedHub,
    ids: &[String],
    target: lastissue::query::GroupKey,
) -> std::result::Result<(), String> {
    let edits: Vec<(String, Edit)> = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else { return Ok(()) };
        ids.iter()
            .filter_map(|id| {
                let s = r.index.by_id(id)?;
                Some((id.clone(), edit::drop_edit(target, s.state, s.priority)?))
            })
            .collect()
    };
    for (id, e) in edits {
        run(hub, Op::Update { id, edit: e })?;
    }
    Ok(())
}

/// `⇧h` / `⇧l` on the board: move cards one column each.
pub fn move_cards(hub: &SharedHub, ids: &[String], step: isize) -> std::result::Result<(), String> {
    let edits: Vec<(String, Edit)> = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else { return Ok(()) };
        if ws.view.mode != Mode::Board {
            return Ok(());
        }
        let cols: Vec<_> = ws.visible.groups.iter().map(|g| g.key).collect();
        ids.iter()
            .filter_map(|id| {
                let s = r.index.by_id(id)?;
                let cur = edit::key_of(ws.view.grouping, s.state, s.priority);
                let target = edit::neighbour(&cols, cur, step)?;
                Some((id.clone(), edit::drop_edit(target, s.state, s.priority)?))
            })
            .collect()
    };
    for (id, e) in edits {
        run(hub, Op::Update { id, edit: e })?;
    }
    Ok(())
}

pub fn delete(hub: &SharedHub, ids: &[String]) -> std::result::Result<(), String> {
    let ids = hub.lock().existing(ids);
    for id in ids {
        run(hub, Op::Delete { id })?;
    }
    Ok(())
}

/// Create, rename (or delete) a project: a structural change, so the
/// index rescans right away.
pub fn project_op(
    hub: &SharedHub,
    f: impl FnOnce(&Store) -> Result<lastissue::store::Project>,
) -> std::result::Result<dto::ProjectDto, String> {
    let (store, src, lock, generation) = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else {
            return Err("no repo is open".into());
        };
        (r.store.clone(), r.source.clone(), r.write_lock.clone(), r.generation)
    };
    let p = {
        let _g = lock.lock().unwrap_or_else(PoisonError::into_inner);
        f(&store).map_err(|e| error_line(&e))?
    };
    let r = index::scan(&store, &*src);
    apply(hub, generation, r, true);
    Ok(dto::ProjectDto::from(&p))
}

// ---------------------------------------------------------------- search

/// The `/` box's text changed: instant results now, the full-text tier
/// after the debounce. Any scan of the previous text is dropped.
pub fn set_search(hub: &SharedHub, query: &str) {
    let mut ws = hub.lock();
    let generation = ws.search.generation + 1;
    ws.search = SearchState {
        generation,
        query: query.to_string(),
        ..SearchState::default()
    };
    let prepared = match (&ws.repo, query.trim().is_empty()) {
        (Some(r), false) => {
            let filter = (|| {
                let mut q = ListQuery::default();
                q.filter(query)?;
                Filter::resolve(&r.store, &ws.identity, &q)
            })();
            match filter {
                Ok(f) => Some((search::prepare(&r.index, &f), f.text)),
                Err(e) => {
                    ws.search.error = Some(error_line(&e));
                    None
                }
            }
        }
        _ => None,
    };
    if let Some((prepared, terms)) = prepared {
        ws.search.hits = prepared.hits;
        if !prepared.candidates.is_empty() {
            ws.search.scanning = true;
            spawn_scan(
                hub,
                prepared.candidates,
                terms,
                move |ws, sink, _paths, hits| {
                    if ws.search.generation != generation {
                        return false;
                    }
                    search::merge(&mut ws.search.hits, hits);
                    emit(sink, events::SEARCH, &ws.search_dto());
                    true
                },
                move |ws, sink| {
                    if ws.search.generation == generation {
                        ws.search.scanning = false;
                        emit(sink, events::SEARCH, &ws.search_dto());
                    }
                },
            );
        }
    }
    hub.emit(events::SEARCH, &ws.search_dto());
}

/// Run the full-text tier over `cands` in background batches after the
/// debounce. `batch` gets each batch's scanned paths and hits (under the
/// lock) and says whether to go on.
fn spawn_scan(
    hub: &SharedHub,
    cands: Vec<Candidate>,
    terms: Vec<String>,
    batch: impl Fn(&mut Workspace, &dyn Sink, Vec<PathBuf>, Vec<Hit>) -> bool + Send + 'static,
    done: impl FnOnce(&mut Workspace, &dyn Sink) + Send + 'static,
) {
    let hub = hub.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FULL_TEXT_DEBOUNCE);
        let ft = FullText::new(&terms);
        let (mut start, mut size) = (0, FIRST_BATCH);
        while start < cands.len() {
            let end = (start + size).min(cands.len());
            let hits = search::scan(&cands[start..end], &ft);
            let paths = cands[start..end].iter().map(|c| c.path.clone()).collect();
            {
                let mut ws = hub.lock();
                if !batch(&mut ws, &*hub.sink, paths, hits) {
                    return;
                }
            }
            start = end;
            size = (size * 2).min(MAX_BATCH);
        }
        let mut ws = hub.lock();
        done(&mut ws, &*hub.sink);
    });
}

/// Scan the filter bar's unscanned candidates, unless a scan runs.
fn scan_filter_text(hub: &SharedHub, ws: &mut Workspace) {
    if ws.filter_text.running {
        return;
    }
    let ft = &ws.filter_text;
    let todo: Vec<Candidate> = ws
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
    ws.filter_text.running = true;
    let hub2 = hub.clone();
    let hub3 = hub.clone();
    spawn_scan(
        hub,
        todo,
        terms,
        move |ws, _, paths, hits| {
            if ws.filter_text.generation != generation {
                return false;
            }
            ws.filter_text.scanned.extend(paths);
            let any = !hits.is_empty();
            ws.filter_text.hits.extend(hits.into_iter().map(|h| h.path));
            if any {
                recompute(&hub2, ws, false);
            }
            true
        },
        move |ws, _| {
            if ws.filter_text.generation == generation {
                ws.filter_text.running = false;
                // Candidates may have grown meanwhile (scope change).
                recompute(&hub3, ws, true);
            }
        },
    );
    hub.emit(events::VISIBLE, &ws.visible_dto());
}

// ----------------------------------------------------------------- links

/// Open a `lastissue://` URL (GUI-SPEC §10): switch to the labelled repo
/// if needed, then select the issue and focus the detail pane, at once or
/// as soon as the repo has loaded.
pub fn open_url(hub: &SharedHub, url: &str) {
    if let Err(e) = crate::link::parse(url).and_then(|l| open_link(hub, l)) {
        let mut ws = hub.lock();
        ws.error = Some(e);
        hub.emit(events::META, &ws.meta());
    }
}

fn open_link(hub: &SharedHub, link: Link) -> std::result::Result<(), String> {
    let (repo, switch) = {
        let ws = hub.lock();
        let repo = ws
            .repos
            .iter()
            .find(|r| r.label == link.repo)
            .or_else(|| {
                ws.repos
                    .iter()
                    .find(|r| r.label.eq_ignore_ascii_case(&link.repo))
            })
            .cloned()
            .ok_or_else(|| format!("no registered repo labelled {:?}", link.repo))?;
        let current = ws.repo.as_ref().map(|r| r.repo.root.clone());
        let switch = current.as_ref() != Some(&repo.root);
        (repo, switch)
    };
    if switch {
        open_repo(hub, repo);
    }
    let mut ws = hub.lock();
    ws.pending_link = link.issue;
    open_pending_link(hub, &mut ws)
}

/// Select the pending linked issue if the repo has loaded: an exact short
/// ID, else a unique prefix.
fn open_pending_link(hub: &SharedHub, ws: &mut Workspace) -> std::result::Result<(), String> {
    let Some(r) = &ws.repo else { return Ok(()) };
    if !r.loaded {
        return Ok(());
    }
    let Some(want) = ws.pending_link.take() else {
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
    select_locked(hub, ws, Some(id.clone()));
    hub.emit(events::FOCUS_DETAIL, &id);
    Ok(())
}

// --------------------------------------------------------------- pickers

pub fn picker_options(
    hub: &SharedHub,
    kind: super::picker::PickerKind,
    ids: &[String],
    query: &str,
) -> Vec<dto::PickOptionDto> {
    let ws = hub.lock();
    let Some(r) = &ws.repo else {
        return Vec::new();
    };
    let targets: Vec<_> = ids.iter().filter_map(|id| r.index.by_id(id)).collect();
    super::picker::filtered(kind, query, &targets, &r.index, ws.me())
        .into_iter()
        .map(|o| dto::PickOptionDto {
            label: o.label,
            detail: o.detail,
            checked: o.checked,
            edit: (&o.edit).into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::model::{Actor, State};
    use lastissue::ops::{self, NewIssue};

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, serde_json::Value)>>);

    impl Sink for Recorder {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    impl Recorder {
        fn last(&self, event: &str) -> Option<serde_json::Value> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(e, _)| e == event)
                .map(|(_, v)| v.clone())
        }
    }

    fn actor() -> Identity {
        Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        })
    }

    struct Fx {
        _dir: tempfile::TempDir,
        store: Store,
        hub: SharedHub,
        rec: Arc<Recorder>,
    }

    fn wait(what: &str, f: impl Fn() -> bool) {
        for _ in 0..400 {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    fn fx(titles: &[&str]) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(store::init(dir.path()).unwrap());
        for t in titles {
            let n = NewIssue {
                title: (*t).into(),
                state: Some(State::Todo),
                body: format!("body of {t}\n"),
                ..NewIssue::default()
            };
            ops::create_issue(&store, &actor(), &n).unwrap();
        }
        let rec = Arc::new(Recorder::default());
        let drafts = Drafts::new(dir.path().join(".drafts"));
        let ws = Workspace::new(Some(drafts), false).with_identity(actor());
        let hub = Hub::new(ws, rec.clone());
        let repo = repo_for(&store.root, &[]);
        hub.lock().repos = vec![repo.clone()];
        open_repo(&hub, repo);
        // open_repo resets the identity from git config; tests pin it.
        hub.lock().identity = Arc::new(actor());
        wait("load", || hub.lock().repo.as_ref().is_some_and(|r| r.loaded));
        let store = hub.lock().repo.as_ref().unwrap().store.clone();
        Fx {
            _dir: dir,
            store,
            hub,
            rec,
        }
    }

    fn id_of(f: &Fx, title: &str) -> String {
        let ws = f.hub.lock();
        let r = ws.repo.as_ref().unwrap();
        r.index.iter().find(|s| s.title == title).unwrap().id.clone()
    }

    #[test]
    fn loads_and_pushes_index_and_visible() {
        let f = fx(&["one", "two"]);
        let idx = f.rec.last(events::INDEX).unwrap();
        assert_eq!(idx["full"], true);
        assert_eq!(idx["changed"].as_array().unwrap().len(), 2);
        let vis = f.rec.last(events::VISIBLE).unwrap();
        assert_eq!(vis["total"], 2);
    }

    #[test]
    fn edits_go_through_core_and_update_the_index() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        apply_to(&f.hub, std::slice::from_ref(&id), &Edit::State(State::InProgress)).unwrap();
        let ws = f.hub.lock();
        let s = ws.repo.as_ref().unwrap().index.by_id(&id).unwrap();
        assert_eq!(s.state, State::InProgress);
        let vis = f.rec.last(events::VISIBLE).unwrap();
        assert_eq!(vis["groups"][usize::try_from(vis["list"][0].as_u64().unwrap()).unwrap()]["label"], "In progress");
    }

    #[test]
    fn doc_edits_save_and_coalesce() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        select(&f.hub, Some(id.clone()));
        wait("doc load", || f.hub.lock().doc.as_ref().is_some_and(|d| d.msgs.is_some()));
        assert!(doc_edit(&f.hub, &id, FieldKind::Body, "new body\n"));
        assert!(doc_edit(&f.hub, &id, FieldKind::Title, "renamed"));
        save_doc(&f.hub);
        wait("save", || !f.hub.lock().doc.as_ref().unwrap().is_dirty());
        let r = f.store.resolve_issue(&id).unwrap();
        let msgs = f.store.read(&r).unwrap();
        assert_eq!(msgs[0].get("Subject"), Some("renamed"));
        assert_eq!(msgs[0].body, "new body");
        // Title and body edits coalesce into one event.
        let events: Vec<_> = msgs.iter().filter(|m| lastissue::model::is_event(m)).collect();
        assert_eq!(events.len(), 1, "{msgs:?}");
    }

    #[test]
    fn search_streams_full_text_hits() {
        let f = fx(&["alpha", "beta"]);
        set_search(&f.hub, "body of beta");
        wait("scan", || !f.hub.lock().search.scanning);
        let s = f.rec.last(events::SEARCH).unwrap();
        let hits = s["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "{s}");
        assert_eq!(hits[0]["tier"], "body");
    }

    #[test]
    fn filter_text_uses_full_text_tier() {
        let f = fx(&["alpha", "beta"]);
        let mut v = dto::ViewDto::from(&f.hub.lock().view);
        v.filter = "\"body of alpha\"".into();
        set_view(&f.hub, &v).unwrap();
        wait("filter scan", || {
            f.rec.last(events::VISIBLE).is_some_and(|v| v["total"] == 1 && v["scanning"] == false)
        });
    }

    #[test]
    fn links_select_and_focus() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        let label = f.hub.lock().repos[0].label.clone();
        open_url(&f.hub, &Link { repo: label, issue: Some(id.clone()) }.to_url());
        assert_eq!(f.rec.last(events::FOCUS_DETAIL).unwrap(), serde_json::json!(id));
        assert_eq!(f.hub.lock().view.selected.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn delete_closes_the_open_issue() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        select(&f.hub, Some(id.clone()));
        delete(&f.hub, std::slice::from_ref(&id)).unwrap();
        let ws = f.hub.lock();
        assert!(ws.doc.is_none() && ws.view.selected.is_none());
        assert_eq!(ws.repo.as_ref().unwrap().index.len(), 0);
    }
}
