//! Filesystem watcher for the open repo (GUI-SPEC §3): `notify` on the
//! issues root, debounced on its own thread into [`Batch`]es of changed
//! paths. What a batch means is decided in core (`index::reload`): issue
//! files are re-read and diffed (an in-place rewrite of the last event is
//! just another change), structural changes trigger a full rescan.

use notify::{EventKind, RecursiveMode, Watcher as _};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Flush once no event arrived for this long…
pub const QUIET: Duration = Duration::from_millis(75);
/// …or this long after the first event of a burst, whichever is first.
pub const MAX_DELAY: Duration = Duration::from_millis(500);

/// Changed paths collected over one debounce window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Batch {
    pub paths: BTreeSet<PathBuf>,
    /// The backend lost track (overflow, error): rescan everything.
    pub rescan: bool,
}

impl Batch {
    pub fn add(&mut self, ev: notify::Result<notify::Event>) {
        match ev {
            Ok(ev) => {
                if ev.need_rescan() {
                    self.rescan = true;
                }
                if !matches!(ev.kind, EventKind::Access(_)) {
                    self.paths.extend(ev.paths);
                }
            }
            Err(_) => self.rescan = true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && !self.rescan
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.paths.iter().cloned().collect()
    }
}

/// Collect raw events into batches: a batch closes after `quiet` without
/// events or `max` after it opened. Returns when either side hangs up.
pub fn debounce<E>(
    rx: &mpsc::Receiver<E>,
    tx: &mpsc::Sender<Batch>,
    quiet: Duration,
    max: Duration,
    mut add: impl FnMut(&mut Batch, E),
) {
    while let Ok(first) = rx.recv() {
        let mut batch = Batch::default();
        add(&mut batch, first);
        let opened = Instant::now();
        let mut disconnected = false;
        loop {
            let left = max.saturating_sub(opened.elapsed());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(quiet.min(left)) {
                Ok(ev) => add(&mut batch, ev),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if !batch.is_empty() && tx.send(batch).is_err() {
            return;
        }
        if disconnected {
            return;
        }
    }
}

/// Watches one root while alive; dropping it stops the debounce thread.
pub struct RepoWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Start watching `root` recursively. Batches arrive on the receiver.
pub fn watch(root: &Path) -> notify::Result<(RepoWatcher, mpsc::Receiver<Batch>)> {
    let (raw_tx, raw_rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(raw_tx)?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("lastissue-watch".into())
        .spawn(move || debounce(&raw_rx, &tx, QUIET, MAX_DELAY, Batch::add))
        .map_err(notify::Error::io)?;
    Ok((RepoWatcher { _watcher: watcher }, rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::identity::Identity;
    use lastissue::model::{Actor, State};
    use lastissue::ops::{self, IssueUpdate, NewIssue};
    use lastissue::store::{self, Store};
    use std::sync::mpsc::TryRecvError;

    fn path_batches(events: &[(u64, &str)], quiet: u64, max: u64) -> Vec<Vec<String>> {
        let (raw_tx, raw_rx) = mpsc::channel::<PathBuf>();
        let (tx, rx) = mpsc::channel();
        let t = std::thread::spawn(move || {
            debounce(
                &raw_rx,
                &tx,
                Duration::from_millis(quiet),
                Duration::from_millis(max),
                |b, p| {
                    b.paths.insert(p);
                },
            );
        });
        for (delay, p) in events {
            std::thread::sleep(Duration::from_millis(*delay));
            raw_tx.send(PathBuf::from(p)).unwrap();
        }
        drop(raw_tx);
        t.join().unwrap();
        rx.iter()
            .collect::<Vec<_>>()
            .into_iter()
            .map(|b| b.paths.iter().map(|p| p.display().to_string()).collect())
            .collect()
    }

    #[test]
    fn burst_becomes_one_batch_and_dedups() {
        let b = path_batches(&[(0, "a"), (5, "b"), (5, "a")], 100, 2000);
        assert_eq!(b, [vec!["a", "b"]]);
    }

    #[test]
    fn quiet_gap_splits_batches() {
        let b = path_batches(&[(0, "a"), (250, "b")], 50, 2000);
        assert_eq!(b, [vec!["a"], vec!["b"]]);
    }

    #[test]
    fn max_delay_caps_a_continuous_stream() {
        let events: Vec<(u64, &str)> = (0..30).map(|_| (20, "a")).collect();
        let b = path_batches(&events, 100, 150);
        assert!(b.len() >= 2, "{b:?}");
    }

    #[test]
    fn access_events_are_ignored_and_errors_rescan() {
        let mut b = Batch::default();
        b.add(Ok(notify::Event::new(EventKind::Access(
            notify::event::AccessKind::Any,
        ))
        .add_path("x".into())));
        assert!(b.is_empty());
        b.add(Err(notify::Error::generic("lost")));
        assert!(b.rescan && !b.is_empty());
    }

    /// Real watcher: a CLI-style write (atomic rename) of an issue file,
    /// including an in-place coalesced rewrite, shows up as a batch with
    /// that path.
    #[test]
    fn watcher_sees_core_writes() {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(dir.path()).unwrap().canonicalize().unwrap();
        let s = Store::new(root.clone());
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        let issue = ops::create_issue(
            &s,
            &me,
            &NewIssue {
                title: "Watch me".into(),
                ..NewIssue::default()
            },
        )
        .unwrap()
        .issue;
        let (_w, rx) = watch(&root).unwrap();
        // FSEvents can deliver the creation late; give the stream a moment.
        std::thread::sleep(Duration::from_millis(300));
        for st in [State::Todo, State::InProgress] {
            let u = IssueUpdate {
                state: Some(st),
                ..IssueUpdate::default()
            };
            ops::update_issue(&s, &me, &issue.id, &u).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut seen = false;
            while !seen && Instant::now() < deadline {
                match rx.try_recv() {
                    Ok(b) => seen = b.rescan || b.paths.contains(&issue.path),
                    Err(TryRecvError::Disconnected) => break,
                    Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
            assert!(seen, "no batch for {st:?}");
        }
    }
}
