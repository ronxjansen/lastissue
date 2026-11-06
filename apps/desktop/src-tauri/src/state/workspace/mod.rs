//! The window's state: the registry's repos, the open
//! repo (index, watcher, identity), the view state and its evaluated
//! result, the open issue, and both search tiers.
//!
//! One [`Hub`] holds it behind a mutex. Commands lock it briefly; disk I/O
//! and scans run on plain threads with the lock released, then lock again
//! to fold results in. Each repo open and each search gets a generation
//! number, so work for something already replaced drops out. Every change
//! the front-end needs is pushed through the [`Sink`].

mod doc;
mod links;
mod repo;
mod search;
mod writes;

pub use doc::{DocAction, doc_action, doc_edit, save_doc, select};
pub use links::open_url;
pub use repo::{open_repo, start};
pub use search::set_search;
pub use writes::{apply_to, delete, drop_card, move_cards, project_op, run};

use super::doc::Doc;
use super::draft::Drafts;
use super::view_state::{ViewState, Visible};
use super::watcher;
use crate::dto::VisibleDto;
use lastissue::identity::Identity;
use lastissue::index::Index;
use lastissue::registry::Repo;
use lastissue::store::Store;
use search::{FilterText, SearchState};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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

/// One open issues root: its index and what keeps it fresh.
pub struct RepoModel {
    pub generation: u64,
    pub repo: Repo,
    pub store: Store,
    /// Serializes this window's writes (core writes are atomic per file,
    /// but a read-modify-write must not interleave with another).
    write_lock: Arc<Mutex<()>>,
    pub index: Index,
    pub loaded: bool,
    pub errors: BTreeMap<PathBuf, String>,
    pub failure: Option<String>,
    _watcher: Option<watcher::RepoWatcher>,
}

impl RepoModel {
    /// The `lastissue://` URL for issue `id` here.
    pub fn link(&self, id: String) -> String {
        crate::link::Link {
            repo: self.repo.label.clone(),
            issue: Some(id),
        }
        .to_url()
    }
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
    pub(crate) search: SearchState,
    pub(crate) filter_text: FilterText,
    pub notice: Option<String>,
    pub error: Option<String>,
    drafts: Option<Drafts>,
    pending_link: Option<String>,
    watch: bool,
}

/// The identity for a repo: git config in its directory,
/// never the `$USER@host` fallback.
fn identity_for(root: &Path) -> Identity {
    Identity::new(None, None).in_dir(root).without_fallback()
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

    pub(crate) fn me(&self) -> Option<&str> {
        self.identity.actor().ok().map(|a| a.from.as_str())
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

    /// Re-evaluate the view (a new set of free-text terms restarts the
    /// filter's full-text tier).
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

/// Re-evaluate the view and push it; `scan`: also start the full-text
/// tier for candidates not yet scanned.
fn recompute(hub: &SharedHub, ws: &mut Workspace, scan: bool) {
    ws.refresh();
    hub.emit(events::VISIBLE, &VisibleDto::new(ws));
    if scan {
        search::scan_filter_text(hub, ws);
    }
}

pub fn set_view(hub: &SharedHub, v: ViewState) {
    let mut ws = hub.lock();
    if ws.view != v {
        ws.view = v;
        recompute(hub, &mut ws, true);
    }
}

#[cfg(test)]
mod fixture {
    use super::repo::{open_repo, repo_for};
    use super::{Hub, SharedHub, Sink, Workspace};
    use crate::state::draft::Drafts;
    use lastissue::identity::Identity;
    use lastissue::model::{Actor, State};
    use lastissue::ops::{self, NewIssue};
    use lastissue::store::{self, Store};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    pub struct Recorder(Mutex<Vec<(String, serde_json::Value)>>);

    impl Sink for Recorder {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    impl Recorder {
        pub fn last(&self, event: &str) -> Option<serde_json::Value> {
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

    pub struct Fx {
        _dir: tempfile::TempDir,
        pub store: Store,
        pub hub: SharedHub,
        pub rec: Arc<Recorder>,
    }

    pub fn wait(what: &str, f: impl Fn() -> bool) {
        for _ in 0..400 {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    pub fn fx(titles: &[&str]) -> Fx {
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
        open_repo(&hub, repo).unwrap();
        // open_repo resets the identity from git config; tests pin it.
        hub.lock().identity = Arc::new(actor());
        wait("load", || {
            hub.lock().repo.as_ref().is_some_and(|r| r.loaded)
        });
        let store = hub.lock().repo.as_ref().unwrap().store.clone();
        Fx {
            _dir: dir,
            store,
            hub,
            rec,
        }
    }

    pub fn id_of(f: &Fx, title: &str) -> String {
        let ws = f.hub.lock();
        let r = ws.repo.as_ref().unwrap();
        r.index
            .iter()
            .find(|s| s.title == title)
            .unwrap()
            .id
            .clone()
    }
}
