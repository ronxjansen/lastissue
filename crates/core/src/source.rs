//! Where a front-end gets issue heads: [`load`] reads
//! one file's head and stats from disk.

use crate::error::{Error, Result};
use crate::model::{self, Priority, State};
use crate::query::Stats;
use crate::store::mbox::{self, Msg};
use crate::store::{IssueRef, Store};
use chrono::{DateTime, FixedOffset};
use std::path::Path;
use std::time::SystemTime;

/// An issue's head: message #1's headers plus stats, no bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueSummary {
    pub id: String,
    pub path: std::path::PathBuf,
    /// Project short ID, if the issue lives in a project folder.
    pub project: Option<String>,
    /// That project's slug.
    pub project_slug: Option<String>,
    pub message_id: String,
    pub title: String,
    pub state: State,
    pub priority: Option<Priority>,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub author: String,
    pub sender: Option<String>,
    pub watchers: Vec<String>,
    pub created: Option<DateTime<FixedOffset>>,
    /// Newest message date (falls back to `created`).
    pub updated: Option<DateTime<FixedOffset>>,
    pub due: Option<DateTime<FixedOffset>>,
    pub expires: Option<DateTime<FixedOffset>>,
    pub comment_count: usize,
    /// Message-IDs.
    pub parent: Option<String>,
    pub blocked_by: Vec<String>,
    pub related: Vec<String>,
    /// File mtime when read, to recognise one's own writes.
    pub mtime: Option<SystemTime>,
}

impl IssueSummary {
    fn new(issue: &IssueRef, head: &Msg, stats: Stats, mtime: Option<SystemTime>) -> Self {
        let created = head.date();
        let date = |h: &str| head.get(h).and_then(mbox::parse_date);
        IssueSummary {
            id: issue.id.clone(),
            path: issue.path.clone(),
            project: issue.project.as_ref().map(|p| p.id.clone()),
            project_slug: issue.project.as_ref().map(|p| p.slug.clone()),
            message_id: head.message_id().to_string(),
            title: head.get("Subject").unwrap_or("").to_string(),
            state: model::state_of(head),
            priority: model::priority_of(head),
            labels: model::labels_of(head),
            assignee: head.get("To").map(str::to_string),
            author: head.get("From").unwrap_or("").to_string(),
            sender: head.get("Sender").map(str::to_string),
            watchers: model::list_of(head, "Cc"),
            created,
            updated: stats.updated.or(created),
            due: date("Reply-By"),
            expires: date("Expires"),
            comment_count: stats.comments,
            parent: head.get("X-Parent").map(str::to_string),
            blocked_by: model::list_of(head, "X-Blocked-By"),
            related: model::list_of(head, "X-Related"),
            mtime,
        }
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Read one issue's head and stats (e.g. after a watcher event).
pub fn load(store: &Store, issue: &IssueRef) -> Result<IssueSummary> {
    let mtime = mtime(&issue.path);
    let bytes = std::fs::read(&issue.path)?;
    let head = mbox::read_head(&bytes[..])?;
    if head.headers.is_empty() {
        // No message #1 (e.g. a file caught mid-write by a non-atomic
        // writer): not an issue head.
        return Err(Error::validation("corrupt-file").with("Path", store.rel(&issue.path)));
    }
    let stats = mbox::scan_stats(&bytes[..])?;
    Ok(IssueSummary::new(issue, &head, stats, mtime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::model::Actor;
    use crate::ops::{self, NewIssue};
    use crate::store;

    #[test]
    fn scan_reads_heads_and_stats() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(store::init(dir.path()).unwrap());
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        let p = ops::create_project(&s, "Auth").unwrap();
        let new = NewIssue {
            title: "Login".into(),
            project: Some(p.id.clone()),
            priority: Some(Priority::High),
            labels: vec!["bug".into()],
            ..NewIssue::default()
        };
        let id = ops::create_issue(&s, &me, &new).unwrap().issue.id;
        let t = ops::comment_target(&s, &id, None).unwrap();
        ops::add_comment(&s, &me, &t, "hi").unwrap();

        let all: Vec<IssueSummary> = s
            .issues()
            .unwrap()
            .iter()
            .map(|r| load(&s, r).unwrap())
            .collect();
        assert_eq!(all.len(), 1);
        let i = &all[0];
        assert_eq!(
            (i.id.as_str(), i.title.as_str(), i.state, i.priority),
            (id.as_str(), "Login", State::Backlog, Some(Priority::High))
        );
        assert_eq!(i.project.as_deref(), Some(p.id.as_str()));
        assert_eq!(i.labels, ["bug"]);
        assert_eq!(i.comment_count, 1);
        assert!(i.updated >= i.created && i.mtime.is_some());
    }

    #[test]
    fn core_types_are_send_sync() {
        fn check<T: Send + Sync>() {}
        check::<Store>();
        check::<Identity>();
        check::<IssueSummary>();
        check::<crate::query::Filter>();
        check::<crate::error::Error>();
    }
}
