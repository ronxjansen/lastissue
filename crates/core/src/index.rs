//! The in-memory issue set a front-end keeps for one root (GUI-SPEC §3):
//! every issue head as an [`IssueSummary`], plus the project folders.
//!
//! Loading is split in two so the I/O can run off the UI thread: [`scan`]
//! and [`reload`] read the disk and return a [`Reload`]; [`Index::apply`]
//! folds it in (no I/O) and reports what actually changed as a [`Delta`].
//! A re-read that yields an identical summary (same content and mtime, e.g.
//! the front-end's own write it already applied) is not a change.

use crate::error::Result;
use crate::source::{self, IssueSummary};
use crate::store::{IssueRef, Project, Store};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// A file that could not be read (e.g. corrupt); it is skipped, and on a
/// path reload its previous summary is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

/// What a disk read produced, for [`Index::apply`].
#[derive(Debug, Clone)]
pub enum Reload {
    /// Everything: replaces the index.
    Full {
        issues: Vec<IssueSummary>,
        projects: Vec<Project>,
        errors: Vec<LoadError>,
    },
    /// Some files: `loaded` exist (new or changed), `gone` don't.
    Paths {
        loaded: Vec<IssueSummary>,
        gone: Vec<PathBuf>,
        errors: Vec<LoadError>,
    },
}

impl Reload {
    pub fn errors(&self) -> &[LoadError] {
        match self {
            Reload::Full { errors, .. } | Reload::Paths { errors, .. } => errors,
        }
    }
}

/// What [`Index::apply`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    /// Added or changed issues.
    pub changed: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    /// The project list changed.
    pub projects: bool,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty() && !self.projects
    }

    /// The issue at `path` was added, changed or removed.
    pub fn touches(&self, path: &Path) -> bool {
        self.changed.iter().chain(&self.removed).any(|p| p == path)
    }
}

fn load_error(path: &Path, e: &crate::error::Error) -> LoadError {
    LoadError {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

fn load_all(store: &Store, refs: &[IssueRef]) -> (Vec<IssueSummary>, Vec<LoadError>) {
    let results: Vec<_> = refs
        .par_iter()
        .map(|r| source::load(store, r).map_err(|e| load_error(&r.path, &e)))
        .collect();
    let mut ok = Vec::with_capacity(results.len());
    let mut errors = Vec::new();
    for r in results {
        match r {
            Ok(s) => ok.push(s),
            Err(e) => errors.push(e),
        }
    }
    (ok, errors)
}

/// Read every issue head and the project list. A corrupt file is reported,
/// not fatal.
pub fn scan(store: &Store) -> Result<Reload> {
    let projects = store.projects()?;
    let (issues, errors) = load_all(store, &store.issues()?);
    Ok(Reload::Full {
        issues,
        projects,
        errors,
    })
}

/// A path whose change can alter the project list: the root itself, or a
/// visible non-`.mbox` entry directly in it (a project folder created,
/// renamed or deleted).
fn is_structural(store: &Store, path: &Path) -> bool {
    if path == store.root {
        return true;
    }
    path.parent() == Some(store.root.as_path())
        && path.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            !n.starts_with('.') && !n.ends_with(".mbox")
        })
}

/// Re-read the given changed paths (watcher output). Falls back to a full
/// [`scan`] when the project structure may have changed; ignores paths
/// that are not issue files (temp files, the marker, other folders).
pub fn reload(store: &Store, paths: &[PathBuf]) -> Result<Reload> {
    if paths.iter().any(|p| is_structural(store, p)) {
        return scan(store);
    }
    let mut refs = Vec::new();
    let mut gone = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in paths {
        if !seen.insert(p) {
            continue;
        }
        let Some(r) = store.issue_ref_for(p) else {
            continue;
        };
        if r.path.is_file() {
            refs.push(r);
        } else {
            gone.push(r.path);
        }
    }
    let (loaded, errors) = load_all(store, &refs);
    Ok(Reload::Paths {
        loaded,
        gone,
        errors,
    })
}

/// Every issue head of one root, keyed by path.
#[derive(Debug, Clone, Default)]
pub struct Index {
    issues: BTreeMap<PathBuf, IssueSummary>,
    projects: Vec<Project>,
    /// Short ID → path, and Message-ID → path: O(1) lookups for selection,
    /// relations and the URL scheme (a linear find took ~0.8 ms at 100k).
    /// On a duplicate key the first path in path order wins, as a scan
    /// would find it.
    ids: HashMap<String, PathBuf>,
    mids: HashMap<String, PathBuf>,
}

#[derive(Clone, Copy)]
enum Key {
    Id,
    Mid,
}

impl Key {
    fn of(self, s: &IssueSummary) -> &str {
        match self {
            Key::Id => &s.id,
            Key::Mid => &s.message_id,
        }
    }
}

impl Index {
    pub fn apply(&mut self, reload: Reload) -> Delta {
        let mut d = Delta::default();
        match reload {
            Reload::Full {
                issues, projects, ..
            } => {
                let mut next: BTreeMap<PathBuf, IssueSummary> =
                    issues.into_iter().map(|s| (s.path.clone(), s)).collect();
                d.removed = self
                    .issues
                    .keys()
                    .filter(|p| !next.contains_key(*p))
                    .cloned()
                    .collect();
                for (p, s) in &next {
                    if self.issues.get(p) != Some(s) {
                        d.changed.push(p.clone());
                    }
                }
                d.projects = self.projects != projects;
                std::mem::swap(&mut self.issues, &mut next);
                self.projects = projects;
                self.ids.clear();
                self.mids.clear();
                // Path order: the first of a duplicate key wins.
                for s in self.issues.values() {
                    self.ids
                        .entry(s.id.clone())
                        .or_insert_with(|| s.path.clone());
                    self.mids
                        .entry(s.message_id.clone())
                        .or_insert_with(|| s.path.clone());
                }
            }
            Reload::Paths { loaded, gone, .. } => {
                for s in loaded {
                    if self.issues.get(&s.path) != Some(&s) {
                        d.changed.push(s.path.clone());
                        self.upsert(s);
                    }
                }
                for p in gone {
                    if let Some(old) = self.issues.remove(&p) {
                        self.unlink(&old);
                        d.removed.push(p);
                    }
                }
            }
        }
        d
    }

    /// Insert or replace a summary, keeping the ID maps in step.
    fn upsert(&mut self, s: IssueSummary) {
        let path = s.path.clone();
        if let Some(old) = self.issues.insert(path.clone(), s) {
            let new = &self.issues[&path];
            if old.id != new.id || old.message_id != new.message_id {
                self.unlink(&old);
            }
        }
        for key in [Key::Id, Key::Mid] {
            let k = key.of(&self.issues[&path]).to_string();
            let map = self.map(key);
            match map.get(&k) {
                Some(p) if *p <= path => {}
                _ => {
                    map.insert(k, path.clone());
                }
            }
        }
    }

    fn map(&mut self, key: Key) -> &mut HashMap<String, PathBuf> {
        match key {
            Key::Id => &mut self.ids,
            Key::Mid => &mut self.mids,
        }
    }

    /// Drop `old`'s keys (it was removed or replaced), falling back to
    /// another issue with the same key, if any.
    fn unlink(&mut self, old: &IssueSummary) {
        for key in [Key::Id, Key::Mid] {
            let k = key.of(old);
            if self.map(key).get(k) != Some(&old.path) {
                continue;
            }
            let next = self
                .issues
                .values()
                .find(|s| key.of(s) == k)
                .map(|s| s.path.clone());
            let map = self.map(key);
            match next {
                Some(p) => map.insert(k.to_string(), p),
                None => map.remove(k),
            };
        }
    }

    pub fn len(&self) -> usize {
        self.issues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &IssueSummary> {
        self.issues.values()
    }

    pub fn get(&self, path: &Path) -> Option<&IssueSummary> {
        self.issues.get(path)
    }

    /// By exact short ID.
    pub fn by_id(&self, id: &str) -> Option<&IssueSummary> {
        self.issues.get(self.ids.get(id)?)
    }

    pub fn by_message_id(&self, message_id: &str) -> Option<&IssueSummary> {
        self.issues.get(self.mids.get(message_id)?)
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::model::{Actor, State};
    use crate::ops::{self, IssueUpdate, NewIssue};
    use crate::store;

    fn setup() -> (tempfile::TempDir, Store, Identity) {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(dir.path()).unwrap();
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        (dir, Store::new(root), me)
    }

    fn new(title: &str) -> NewIssue {
        NewIssue {
            title: title.into(),
            ..NewIssue::default()
        }
    }

    fn set_state(s: &Store, me: &Identity, id: &str, st: State) -> PathBuf {
        let u = IssueUpdate {
            state: Some(st),
            ..IssueUpdate::default()
        };
        ops::update_issue(s, me, id, &u).unwrap().issue.path
    }

    #[test]
    fn lookups_follow_reloads_and_upserts() {
        let (_d, s, me) = setup();
        let a = ops::create_issue(&s, &me, &new("A")).unwrap().issue;
        let b = ops::create_issue(&s, &me, &new("B")).unwrap().issue;
        let mut ix = Index::default();
        ix.apply(scan(&s).unwrap());
        let mid = ix.by_id(&a.id).unwrap().message_id.clone();
        assert_eq!(ix.by_message_id(&mid).unwrap().path, a.path);
        assert_eq!(ix.by_id(&b.id).unwrap().path, b.path);

        // Moved file: the ID now points at the new path.
        let p = ops::create_project(&s, "P").unwrap();
        let moved = ops::move_issue(&s, &me, &a.id, Some(&p.id))
            .unwrap()
            .issue
            .path;
        ix.apply(reload(&s, &[a.path.clone(), moved.clone()]).unwrap());
        assert_eq!(ix.by_id(&a.id).unwrap().path, moved);
        assert_eq!(ix.by_message_id(&mid).unwrap().path, moved);

        // Deleted: gone from both maps.
        ops::delete_issue(&s, &b.id).unwrap();
        ix.apply(reload(&s, std::slice::from_ref(&b.path)).unwrap());
        assert!(ix.by_id(&b.id).is_none());

        // A full rescan rebuilds them.
        ix.apply(scan(&s).unwrap());
        assert_eq!(ix.by_id(&a.id).unwrap().path, moved);
        assert_eq!(ix.len(), 1);
    }

    #[test]
    fn path_reload_diffs_and_ignores_noise() {
        let (_d, s, me) = setup();
        let a = ops::create_issue(&s, &me, &new("A")).unwrap().issue;
        let mut ix = Index::default();
        let d = ix.apply(scan(&s).unwrap());
        assert_eq!((d.changed.len(), d.projects), (1, false));

        // Re-reading an unchanged file is not a change.
        assert!(
            ix.apply(reload(&s, std::slice::from_ref(&a.path)).unwrap())
                .is_empty()
        );

        // Two updates within the burst window: the second rewrites the last
        // event in place (no new message), yet the reload sees it.
        set_state(&s, &me, &a.id, State::Todo);
        let d = ix.apply(reload(&s, std::slice::from_ref(&a.path)).unwrap());
        assert_eq!(d.changed, vec![a.path.clone()]);
        let msgs = s.read(&a).unwrap().len();
        set_state(&s, &me, &a.id, State::InProgress);
        assert_eq!(s.read(&a).unwrap().len(), msgs, "coalesced in place");
        let d = ix.apply(reload(&s, std::slice::from_ref(&a.path)).unwrap());
        assert_eq!(d.changed, vec![a.path.clone()]);
        assert_eq!(ix.get(&a.path).unwrap().state, State::InProgress);

        // Temp files, the marker and stray paths are ignored.
        let noise = [
            s.root.join(".a.mbox.01J.tmp"),
            s.root.join(store::MARKER),
            s.root.join("x.txt"),
        ];
        assert!(ix.apply(reload(&s, &noise).unwrap()).is_empty());

        // Deleted: removed.
        ops::delete_issue(&s, &a.id).unwrap();
        let d = ix.apply(reload(&s, std::slice::from_ref(&a.path)).unwrap());
        assert_eq!(d.removed, [a.path]);
        assert!(ix.is_empty());
    }

    #[test]
    fn structural_changes_rescan() {
        let (_d, s, me) = setup();
        let a = ops::create_issue(&s, &me, &new("A")).unwrap().issue;
        let mut ix = Index::default();
        ix.apply(scan(&s).unwrap());

        let p = ops::create_project(&s, "Auth").unwrap();
        let d = ix.apply(reload(&s, std::slice::from_ref(&p.dir)).unwrap());
        assert!(d.projects && d.changed.is_empty());

        // Moving an issue: old path gone, new path present.
        let moved = ops::move_issue(&s, &me, &a.id, Some(&p.id)).unwrap().issue;
        let d = ix.apply(reload(&s, &[a.path.clone(), moved.path.clone()]).unwrap());
        assert_eq!(
            (d.changed, d.removed),
            (vec![moved.path.clone()], vec![a.path])
        );
        assert_eq!(
            ix.by_id(&a.id).unwrap().project.as_deref(),
            Some(p.id.as_str())
        );

        // Renaming the project folder moves every issue in it.
        let renamed = ops::rename_project(&s, &p.id, "Login").unwrap();
        let d = ix.apply(reload(&s, &[p.dir.clone(), renamed.dir.clone()]).unwrap());
        assert!(d.projects);
        assert_eq!(d.removed, [moved.path]);
        assert_eq!(ix.projects()[0].slug, "login");
        assert_eq!(
            ix.by_id(&a.id).unwrap().project.as_deref(),
            Some(p.id.as_str())
        );
    }

    #[test]
    fn corrupt_file_is_reported_and_kept() {
        let (_d, s, me) = setup();
        let a = ops::create_issue(&s, &me, &new("A")).unwrap().issue;
        let mut ix = Index::default();
        ix.apply(scan(&s).unwrap());
        std::fs::write(&a.path, "not an mbox").unwrap();
        let r = reload(&s, std::slice::from_ref(&a.path)).unwrap();
        assert_eq!(r.errors().len(), 1);
        assert!(ix.apply(r).is_empty());
        assert_eq!(ix.len(), 1);
        let r = scan(&s).unwrap();
        assert_eq!(r.errors().len(), 1);
    }
}
