//! One open repo (GUI-SPEC §3): the store, the in-memory index of issue
//! heads, and the watcher that keeps it live. Emits [`RepoChanged`] with
//! core's [`Delta`] whenever the index actually changed. Every GUI write
//! goes through [`RepoModel::run`]: one at a time (a lock around core
//! `ops`), off the UI thread, folded into the index at once.

use super::edit::{self, Applied, Op};
use super::watcher::{self, RepoWatcher};
use futures::StreamExt as _;
use gpui::{AppContext as _, Context, EventEmitter, Task};
use lastissue::error::Result;
use lastissue::identity::Identity;
use lastissue::index::{self, Delta, Index, Reload};
use lastissue::registry::Repo;
use lastissue::source::FsScan;
use lastissue::store::Store;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct RepoChanged {
    pub delta: Delta,
    /// The GUI's own write: the open issue already has the new messages,
    /// so it needn't re-read (nor treat it as someone else's change).
    pub own: bool,
}

pub struct RepoModel {
    pub repo: Repo,
    pub store: Store,
    source: Arc<FsScan>,
    /// Serializes writes: two read-modify-writes of one file must not
    /// interleave (a debounced body save and a picker, say).
    write_lock: Arc<Mutex<()>>,
    pub index: Index,
    /// The first scan finished.
    pub loaded: bool,
    /// Files that could not be read, by path.
    pub errors: BTreeMap<PathBuf, String>,
    /// The scan or the watcher failed.
    pub failure: Option<String>,
    _watcher: Option<RepoWatcher>,
    _task: Task<()>,
}

impl EventEmitter<RepoChanged> for RepoModel {}

impl RepoModel {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        // FSEvents reports real paths (/private/var/…), so key everything by
        // the canonical root.
        let root = repo
            .root
            .canonicalize()
            .unwrap_or_else(|_| repo.root.clone());
        let store = Store::new(root);
        let source = Arc::new(FsScan::new(store.clone()));
        // Watch before scanning, so a write between the two isn't missed.
        // Unit tests drive changes by hand: a real watcher thread waking the
        // deterministic test scheduler would abort it.
        let watching = if cfg!(test) {
            Err(notify::Error::generic("not watched in tests"))
        } else {
            watcher::watch(&store.root)
        };
        let (watcher, rx, failure) = match watching {
            Ok((w, rx)) => (Some(w), Some(rx), None),
            Err(e) => (None, None, Some(format!("not watching: {e}"))),
        };
        let (s, src) = (store.clone(), source.clone());
        let task = cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move { index::scan(&s, &*src) })
                .await;
            if this.update(cx, |m, cx| m.apply(r, false, cx)).is_err() {
                return;
            }
            let Some(mut rx) = rx else { return };
            while let Some(batch) = rx.next().await {
                let Ok((s, src)) = this.read_with(cx, |m, _| (m.store.clone(), m.source.clone()))
                else {
                    return;
                };
                let r = cx
                    .background_spawn(async move {
                        if batch.rescan {
                            index::scan(&s, &*src)
                        } else {
                            index::reload(&s, &*src, &batch.paths())
                        }
                    })
                    .await;
                if this.update(cx, |m, cx| m.apply(r, false, cx)).is_err() {
                    return;
                }
            }
        });
        RepoModel {
            repo,
            store,
            source,
            write_lock: Arc::new(Mutex::new(())),
            index: Index::default(),
            loaded: false,
            errors: BTreeMap::new(),
            failure,
            _watcher: watcher,
            _task: task,
        }
    }

    /// Run a write through core on the background executor, then fold the
    /// written file into the index (emitting an `own` change).
    pub fn run(
        &mut self,
        me: Arc<Identity>,
        op: Op,
        cx: &mut Context<Self>,
    ) -> Task<Result<Applied>> {
        let (store, src, lock) = (
            self.store.clone(),
            self.source.clone(),
            self.write_lock.clone(),
        );
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move {
                    let _g = lock
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    edit::run(&store, &me, &*src, &op)
                })
                .await;
            if let Ok(a) = &r {
                let reload = a.reload.clone();
                let _ = this.update(cx, |m, cx| m.apply(Ok(reload), true, cx));
            }
            r
        })
    }

    fn apply(&mut self, r: Result<Reload>, own: bool, cx: &mut Context<Self>) {
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                self.failure = Some(e.to_string().trim().replace('\n', " · "));
                self.loaded = true;
                cx.notify();
                return;
            }
        };
        match &r {
            Reload::Full { .. } => self.errors.clear(),
            Reload::Paths { loaded, gone, .. } => {
                for p in loaded.iter().map(|s| &s.path).chain(gone) {
                    self.errors.remove(p);
                }
            }
        }
        for e in r.errors() {
            self.errors.insert(e.path.clone(), e.message.clone());
        }
        let first = !self.loaded;
        self.loaded = true;
        let delta = self.index.apply(r);
        if first || !delta.is_empty() {
            cx.emit(RepoChanged { delta, own });
            cx.notify();
        }
    }
}
