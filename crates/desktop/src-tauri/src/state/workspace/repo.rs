//! Repo lifecycle: start-up, switching repos, and folding the background
//! load and the watcher's reloads into the index.

use super::doc::load_doc;
use super::links::open_pending_link;
use super::search::{FilterText, SearchState};
use super::{RepoModel, SharedHub, Workspace, events, identity_for, recompute, watcher};
use crate::dto::{self, DocDto, IndexDelta, MetaDto, SearchDto, SummaryDto, VisibleDto};
use crate::format::error_line;
use lastissue::error::Result;
use lastissue::index::{self, Delta, Index, Reload};
use lastissue::registry::{self, Repo};
use lastissue::store::{self, Store};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The repo to open first: an explicit root (argument, `LASTISSUE_DIR`),
/// else a local root discovered from the current directory, else the last
/// repo the GUI opened, else the global root, else the first registered repo
/// that still exists. A launch from Finder starts in `/`, so discovery alone
/// would rarely find anything.
/// `Ok(None)`: nothing is set up yet (the UI shows its welcome).
fn initial_repo(arg: Option<&Path>, repos: &[Repo]) -> Result<Option<Repo>> {
    let env = std::env::var_os("LASTISSUE_DIR").map(PathBuf::from);
    let explicit = arg.or(env.as_deref());
    let last = || {
        std::fs::read_to_string(last_file()?)
            .ok()
            .map(|s| repo_for(Path::new(s.trim()), repos))
            .filter(Repo::exists)
    };
    match store::discover(explicit) {
        Err(e) if explicit.is_some() => Err(e),
        Ok(root) if explicit.is_some() || !store::global_root().is_ok_and(|g| g == root) => {
            Ok(Some(repo_for(&root, repos)))
        }
        r => Ok(last()
            .or_else(|| r.ok().map(|g| repo_for(&g, repos)))
            .or_else(|| repos.iter().find(|r| r.exists()).cloned())),
    }
}

/// One-line file next to the registry holding the root the GUI opened last.
fn last_file() -> Option<PathBuf> {
    registry::path().ok().map(|p| p.with_file_name("last"))
}

/// The registry entry for `root`, or an ad-hoc one.
pub(super) fn repo_for(root: &Path, repos: &[Repo]) -> Repo {
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
/// Load the registry and open the first repo (GUI start-up).
pub fn start(hub: &SharedHub, arg: Option<PathBuf>) {
    let (mut repos, mut notice) = match registry::load() {
        Ok(r) => (r, None),
        Err(e) => (Vec::new(), Some(error_line(&e))),
    };
    match initial_repo(arg.as_deref(), &repos) {
        Ok(None) => {
            let mut ws = hub.lock();
            ws.repos = repos;
            ws.notice = notice;
        }
        Ok(Some(repo)) => {
            if !repos.iter().any(|r| r.root == repo.root) {
                repos.insert(0, repo.clone());
            }
            hub.lock().repos = repos;
            if let Err(e) = open_repo(hub, repo) {
                let mut ws = hub.lock();
                ws.notice = Some(e);
                hub.emit(events::META, &MetaDto::new(&ws));
            }
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
pub fn open_repo(hub: &SharedHub, repo: Repo) -> std::result::Result<(), String> {
    let root = repo
        .root
        .canonicalize()
        .unwrap_or_else(|_| repo.root.clone());
    let store = Store::new(root);
    let mut ws = hub.lock();
    // ponytail: best-effort; a lost write just falls back to discovery.
    // Gated on `watch` so tests (which don't watch) leave the real file alone.
    if ws.watch
        && let Some(f) = last_file()
    {
        let _ = std::fs::write(f, store.root.as_os_str().as_encoded_bytes());
    }
    let (w, rx, failure) = match ws.watch.then(|| watcher::watch(&store.root)) {
        Some(Ok((w, rx))) => (Some(w), Some(rx), None),
        Some(Err(e)) => (None, None, Some(format!("not watching: {e}"))),
        None => (None, None, None),
    };
    ws.repo_generation += 1;
    let generation = ws.repo_generation;
    ws.identity = Arc::new(identity_for(&repo.root));
    ws.repo = Some(RepoModel {
        generation,
        repo,
        store: store.clone(),
        write_lock: Arc::new(Mutex::new(())),
        index: Index::default(),
        loaded: false,
        errors: BTreeMap::new(),
        failure,
        _watcher: w,
    });
    ws.doc = None;
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
    hub.emit(events::META, &MetaDto::new(&ws));
    hub.emit(
        events::INDEX,
        &IndexDelta {
            full: true,
            changed: Vec::new(),
            removed: Vec::new(),
            projects: Some(Vec::new()),
        },
    );
    hub.emit(events::VISIBLE, &VisibleDto::new(&ws));
    hub.emit(events::DOC, &DocDto::new(&ws));
    hub.emit(events::SEARCH, &SearchDto::new(&ws));
    drop(ws);

    let hub = hub.clone();
    std::thread::Builder::new()
        .name("lastissue-repo".into())
        .spawn(move || {
            let r = index::scan(&store);
            if !apply(&hub, generation, r, false) {
                return;
            }
            let Some(rx) = rx else { return };
            // Ends when the watcher (owned by the RepoModel) is dropped.
            for batch in rx {
                let r = if batch.rescan {
                    index::scan(&store)
                } else {
                    index::reload(&store, &batch.paths())
                };
                if !apply(&hub, generation, r, false) {
                    return;
                }
            }
        })
        .map(drop)
        .map_err(|e| format!("cannot start loading the repo: {e}"))
}

/// Fold a disk read into the open repo (if it's still `generation`) and
/// push what changed. Returns false once the repo was replaced.
pub(super) fn apply(hub: &SharedHub, generation: u64, r: Result<Reload>, own: bool) -> bool {
    let mut ws = hub.lock();
    let Some(repo) = ws.repo.as_mut().filter(|r| r.generation == generation) else {
        return false;
    };
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            repo.failure = Some(error_line(&e));
            repo.loaded = true;
            hub.emit(events::META, &MetaDto::new(&ws));
            return true;
        }
    };
    let first = !repo.loaded;
    let full = first || matches!(r, Reload::Full { .. });
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
    repo.loaded = true;
    let delta = repo.index.apply(r);
    if !first && delta.is_empty() {
        return true;
    }
    let store = repo.store.clone();
    let index_delta = IndexDelta {
        full,
        changed: if full {
            repo.index
                .iter()
                .map(|s| SummaryDto::new(&store, s))
                .collect()
        } else {
            delta
                .changed
                .iter()
                .filter_map(|p| repo.index.get(p))
                .map(|s| SummaryDto::new(&store, s))
                .collect()
        },
        removed: delta.removed.iter().map(|p| store.rel(p)).collect(),
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
    recompute(hub, ws, true);
    if first {
        hub.emit(events::META, &MetaDto::new(ws));
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
    if let Some(s) = repo.index.by_id(&doc.id) {
        let p = s.path.clone();
        load_doc(hub, ws, p);
    } else {
        ws.doc = None;
        hub.emit(events::DOC, &DocDto::new(ws));
    }
}

#[cfg(test)]
mod tests {
    use super::super::events;
    use super::super::fixture::fx;

    #[test]
    fn loads_and_pushes_index_and_visible() {
        let f = fx(&["one", "two"]);
        let idx = f.rec.last(events::INDEX).unwrap();
        assert_eq!(idx["full"], true);
        assert_eq!(idx["changed"].as_array().unwrap().len(), 2);
        let vis = f.rec.last(events::VISIBLE).unwrap();
        assert_eq!(vis["total"], 2);
    }
}
