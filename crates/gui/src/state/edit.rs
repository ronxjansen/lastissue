//! Every GUI write as plain data (GUI-SPEC §5.1, §5.2): an [`Op`] runs
//! through core `ops` (never an ad-hoc file write) and comes back as an
//! [`Applied`] carrying the issue's new messages and the [`Reload`] the
//! front-end folds into its index right away, so the watcher's echo of the
//! write is a no-op. No GPUI here: [`run`] is called on a background
//! executor and tests call it directly.

use lastissue::error::{Error, Result};
use lastissue::identity::Identity;
use lastissue::index::Reload;
use lastissue::model::{Priority, State};
use lastissue::ops::{self, CommentRef, CommentTarget, IssueUpdate, NewIssue, Outcome};
use lastissue::query::GroupKey;
use lastissue::source::IssueSource;
use lastissue::store::Store;
use lastissue::store::mbox::Msg;
use std::path::PathBuf;

/// One change to an issue's message #1. Pickers commit one of these
/// immediately; title and body arrive debounced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Title(String),
    Body(String),
    State(State),
    Priority(Option<Priority>),
    /// An address or `me`; `None` unassigns.
    Assignee(Option<String>),
    AddLabel(String),
    RemoveLabel(String),
    /// Friendly date input (`tomorrow`, `+1w`, `2026-12-01`); `None` clears.
    Due(Option<String>),
    /// Project short ID; `None` moves the issue to the root.
    Project(Option<String>),
}

#[derive(Debug, Clone)]
pub enum Op {
    Update {
        id: String,
        edit: Edit,
    },
    Create(Box<NewIssue>),
    AddComment {
        id: String,
        body: String,
        /// Message-ID of the comment this replies to.
        reply_to: Option<String>,
    },
    EditComment {
        id: String,
        message_id: String,
        body: String,
    },
    DeleteComment {
        id: String,
        message_id: String,
    },
    /// Remove the issue's file (`issue delete`).
    Delete {
        id: String,
    },
}

/// A finished write.
#[derive(Debug, Clone)]
pub struct Applied {
    /// Short ID of the issue written (the new one for a create).
    pub id: String,
    /// Where it lives now (a project change moves the file).
    pub path: PathBuf,
    pub msgs: Vec<Msg>,
    /// Fresh summaries for the index; empty when nothing was written.
    pub reload: Reload,
    pub written: bool,
    /// The issue is gone (an [`Op::Delete`]).
    pub deleted: bool,
}

fn update_for(edit: &Edit) -> IssueUpdate {
    let mut u = IssueUpdate::default();
    match edit.clone() {
        Edit::Title(t) => u.title = Some(t),
        Edit::Body(b) => u.body = Some(b),
        Edit::State(s) => u.state = Some(s),
        Edit::Priority(p) => u.priority = Some(p),
        Edit::Assignee(a) => u.assignee = Some(a),
        Edit::AddLabel(l) => u.add_labels = vec![l],
        Edit::RemoveLabel(l) => u.remove_labels = vec![l],
        Edit::Due(d) => u.due = Some(d),
        Edit::Project(_) => unreachable!("a project change is a move"),
    }
    u
}

fn comment_ref(store: &Store, id: &str, message_id: &str) -> Result<CommentRef> {
    let issue = store.resolve_issue(id)?;
    let msgs = store.read(&issue)?;
    let idx = msgs
        .iter()
        .position(|m| m.message_id() == message_id)
        .filter(|i| *i > 0)
        .ok_or_else(|| Error::not_found("comment-not-found").with("Comment", message_id))?;
    Ok(CommentRef { issue, msgs, idx })
}

/// Run `op` through core and re-read the written issue's summary.
pub fn run(store: &Store, me: &Identity, src: &dyn IssueSource, op: &Op) -> Result<Applied> {
    let (out, old_path): (Outcome, Option<PathBuf>) = match op {
        Op::Delete { id } => {
            let d = ops::delete_issue(store, id)?;
            return Ok(Applied {
                id: d.issue.id.clone(),
                path: d.issue.path.clone(),
                msgs: Vec::new(),
                reload: Reload::Paths {
                    loaded: Vec::new(),
                    gone: vec![d.issue.path],
                    errors: Vec::new(),
                },
                written: true,
                deleted: true,
            });
        }
        Op::Update {
            id,
            edit: Edit::Project(p),
        } => {
            let old = store.resolve_issue(id)?.path;
            let out = ops::move_issue(store, me, id, p.as_deref())?;
            let moved = (out.issue.path != old).then_some(old);
            (out, moved)
        }
        Op::Update { id, edit } => (ops::update_issue(store, me, id, &update_for(edit))?, None),
        Op::Create(new) => (ops::create_issue(store, me, new)?, None),
        Op::AddComment { id, body, reply_to } => {
            let target = CommentTarget {
                issue: store.resolve_issue(id)?,
                reply_to: reply_to.clone(),
            };
            let c = ops::add_comment(store, me, &target, body)?;
            (comment_outcome(c, true), None)
        }
        Op::EditComment {
            id,
            message_id,
            body,
        } => {
            let c = comment_ref(store, id, message_id)?;
            let before = c.msgs[c.idx].body.clone();
            let c = ops::edit_comment(store, me, c, body)?;
            let written = c.msgs[c.idx].body != before;
            (comment_outcome(c, written), None)
        }
        Op::DeleteComment { id, message_id } => (
            ops::delete_comment(store, me, comment_ref(store, id, message_id)?)?,
            None,
        ),
    };
    let loaded = if out.written {
        vec![src.load(&out.issue)?]
    } else {
        Vec::new()
    };
    Ok(Applied {
        id: out.issue.id.clone(),
        path: out.issue.path.clone(),
        msgs: out.msgs,
        reload: Reload::Paths {
            loaded,
            gone: old_path.into_iter().collect(),
            errors: Vec::new(),
        },
        written: out.written,
        deleted: false,
    })
}

fn comment_outcome(c: CommentRef, written: bool) -> Outcome {
    Outcome {
        appended: vec![c.idx],
        issue: c.issue,
        msgs: c.msgs,
        written,
    }
}

/// What dropping a card on a board column means (GUI-SPEC §5.1): the
/// column's state, or its priority when grouped by priority. `None` when
/// the card is already there (dragging within a column does nothing).
pub fn drop_edit(target: GroupKey, state: State, priority: Option<Priority>) -> Option<Edit> {
    match target {
        GroupKey::State(s) if s != state => Some(Edit::State(s)),
        GroupKey::Priority(p) if p != priority => Some(Edit::Priority(p)),
        _ => None,
    }
}

/// The column `step` places from `current` among the board's `columns`
/// (`⇧h` = -1, `⇧l` = +1); `None` at either end.
pub fn neighbour(columns: &[GroupKey], current: GroupKey, step: isize) -> Option<GroupKey> {
    let i = columns.iter().position(|k| *k == current)?;
    let j = i.checked_add_signed(step)?;
    columns.get(j).copied()
}

/// The key a card currently sits under for a grouping.
pub fn key_of(grouping: lastissue::query::Grouping, state: State, p: Option<Priority>) -> GroupKey {
    match grouping {
        lastissue::query::Grouping::State => GroupKey::State(state),
        lastissue::query::Grouping::Priority => GroupKey::Priority(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::index::{self, Index};
    use lastissue::model::Actor;
    use lastissue::source::FsScan;
    use lastissue::store;

    struct Fx {
        _dir: tempfile::TempDir,
        s: Store,
        src: FsScan,
        me: Identity,
        ix: Index,
        id: String,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(store::init(dir.path()).unwrap());
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        let id = ops::create_issue(
            &s,
            &me,
            &NewIssue {
                title: "Drag me".into(),
                body: "Hello".into(),
                ..NewIssue::default()
            },
        )
        .unwrap()
        .issue
        .id;
        let src = FsScan::new(s.clone());
        let mut ix = Index::default();
        ix.apply(index::scan(&s, &src).unwrap());
        Fx {
            _dir: dir,
            s,
            src,
            me,
            ix,
            id,
        }
    }

    fn update(f: &Fx, edit: Edit) -> Applied {
        let op = Op::Update {
            id: f.id.clone(),
            edit,
        };
        run(&f.s, &f.me, &f.src, &op).unwrap()
    }

    fn events(msgs: &[Msg]) -> Vec<String> {
        msgs.iter()
            .filter(|m| m.get("X-Issue-Event").is_some())
            .map(|m| m.body.clone())
            .collect()
    }

    /// Drag → file state + one event; folding the result into the index
    /// makes the watcher's re-read of the same file a no-op.
    #[test]
    fn drop_writes_once_and_watcher_echo_is_noop() {
        let mut f = fx();
        let edit = drop_edit(GroupKey::State(State::InProgress), State::Backlog, None).unwrap();
        let a = update(&f, edit);
        assert!(a.written);
        let d = f.ix.apply(a.reload.clone());
        assert_eq!(d.changed, std::slice::from_ref(&a.path));
        assert_eq!(f.ix.by_id(&f.id).unwrap().state, State::InProgress);
        let echo = index::reload(&f.s, &f.src, std::slice::from_ref(&a.path)).unwrap();
        assert!(f.ix.apply(echo).is_empty());
        assert_eq!(events(&a.msgs), ["state: backlog → in-progress"]);

        // A second drag inside the burst coalesces into the same event.
        let a = update(&f, Edit::State(State::InReview));
        assert_eq!(events(&a.msgs), ["state: backlog → in-review"]);
    }

    #[test]
    fn delete_removes_the_file_and_the_index_entry() {
        let mut f = fx();
        let op = Op::Delete { id: f.id.clone() };
        let a = run(&f.s, &f.me, &f.src, &op).unwrap();
        assert!(a.deleted && !a.path.exists());
        f.ix.apply(a.reload);
        assert!(f.ix.by_id(&f.id).is_none());
    }

    #[test]
    fn drop_on_own_column_does_nothing() {
        assert_eq!(
            drop_edit(GroupKey::State(State::Todo), State::Todo, None),
            None
        );
        assert_eq!(
            drop_edit(
                GroupKey::Priority(Some(Priority::High)),
                State::Todo,
                Some(Priority::High)
            ),
            None
        );
        assert_eq!(
            drop_edit(GroupKey::Priority(None), State::Todo, Some(Priority::High)),
            Some(Edit::Priority(None))
        );
    }

    #[test]
    fn neighbours_stop_at_the_ends() {
        let cols: Vec<GroupKey> = [State::Backlog, State::Todo, State::InProgress]
            .into_iter()
            .map(GroupKey::State)
            .collect();
        let at = |s| GroupKey::State(s);
        assert_eq!(
            neighbour(&cols, at(State::Todo), 1),
            Some(at(State::InProgress))
        );
        assert_eq!(
            neighbour(&cols, at(State::Todo), -1),
            Some(at(State::Backlog))
        );
        assert_eq!(neighbour(&cols, at(State::Backlog), -1), None);
        assert_eq!(neighbour(&cols, at(State::InProgress), 1), None);
        assert_eq!(neighbour(&cols, at(State::Done), 1), None);
    }

    #[test]
    fn title_and_body_bursts_coalesce() {
        let f = fx();
        for t in ["Drag", "Drag me now", "Drag me later"] {
            update(&f, Edit::Title(t.into()));
        }
        update(&f, Edit::Body("Hello\n\nworld".into()));
        let a = update(&f, Edit::Body("Hello world".into()));
        assert_eq!(
            events(&a.msgs),
            ["title: Drag me → Drag me later\nbody: edited"]
        );
        assert_eq!(a.msgs[0].body, "Hello world");
        assert!(a.msgs[0].has("X-Edited"));
        // Unchanged text: no write, no reload.
        let a = update(&f, Edit::Body("Hello world\n".into()));
        assert!(!a.written);
        assert!(matches!(&a.reload, Reload::Paths { loaded, .. } if loaded.is_empty()));
    }

    #[test]
    fn labels_assignee_due_and_project_move() {
        let mut f = fx();
        update(&f, Edit::AddLabel("Bug".into()));
        update(&f, Edit::Assignee(Some("me".into())));
        update(&f, Edit::Due(Some("2030-01-02".into())));
        let p = ops::create_project(&f.s, "Auth").unwrap();
        let before = f.ix.by_id(&f.id).unwrap().path.clone();
        let a = update(&f, Edit::Project(Some(p.id.clone())));
        assert_ne!(a.path, before);
        assert!(!before.exists() && a.path.exists());
        let d = f.ix.apply(a.reload.clone());
        assert_eq!(d.removed, [before]);
        let s = f.ix.by_id(&f.id).unwrap();
        assert_eq!(s.labels, ["bug"]);
        assert_eq!(s.assignee.as_deref(), Some("Ada <ada@example.com>"));
        assert!(s.due.is_some());
        assert_eq!(s.project.as_deref(), Some(p.id.as_str()));
        let a = update(&f, Edit::RemoveLabel("bug".into()));
        assert!(model_labels(&a.msgs).is_empty());
    }

    fn model_labels(msgs: &[Msg]) -> Vec<String> {
        lastissue::model::labels_of(&msgs[0])
    }

    #[test]
    fn bad_input_leaves_file_untouched() {
        let f = fx();
        let path = f.ix.by_id(&f.id).unwrap().path.clone();
        let before = std::fs::read(&path).unwrap();
        let op = Op::Update {
            id: f.id.clone(),
            edit: Edit::Due(Some("someday".into())),
        };
        assert!(run(&f.s, &f.me, &f.src, &op).is_err());
        let op = Op::Update {
            id: f.id.clone(),
            edit: Edit::Title("  ".into()),
        };
        assert!(run(&f.s, &f.me, &f.src, &op).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn comments_create_edit_delete() {
        let f = fx();
        let add = |body: &str| {
            let op = Op::AddComment {
                id: f.id.clone(),
                body: body.into(),
                reply_to: None,
            };
            run(&f.s, &f.me, &f.src, &op).unwrap()
        };
        let a = add("First");
        let mid = a.msgs.last().unwrap().message_id().to_string();
        let op = Op::AddComment {
            id: f.id.clone(),
            body: "Reply".into(),
            reply_to: Some(mid.clone()),
        };
        let a = run(&f.s, &f.me, &f.src, &op).unwrap();
        assert_eq!(
            a.msgs.last().unwrap().get("In-Reply-To"),
            Some(mid.as_str())
        );

        let op = Op::EditComment {
            id: f.id.clone(),
            message_id: mid.clone(),
            body: "First, edited".into(),
        };
        let a = run(&f.s, &f.me, &f.src, &op).unwrap();
        let c = a.msgs.iter().find(|m| m.message_id() == mid).unwrap();
        assert_eq!(c.body, "First, edited");
        assert!(c.has("X-Edited"));

        let op = Op::DeleteComment {
            id: f.id.clone(),
            message_id: mid.clone(),
        };
        let a = run(&f.s, &f.me, &f.src, &op).unwrap();
        assert!(a.msgs.iter().all(|m| m.message_id() != mid));
        assert_eq!(
            a.msgs.last().unwrap().get("X-Issue-Event"),
            Some("comment-deleted")
        );
        // Message #1 is never a comment.
        let op = Op::DeleteComment {
            id: f.id.clone(),
            message_id: a.msgs[0].message_id().to_string(),
        };
        assert!(run(&f.s, &f.me, &f.src, &op).is_err());
    }

    #[test]
    fn create_returns_the_new_issue() {
        let mut f = fx();
        let op = Op::Create(Box::new(NewIssue {
            title: "Fresh".into(),
            state: Some(State::Todo),
            ..NewIssue::default()
        }));
        let a = run(&f.s, &f.me, &f.src, &op).unwrap();
        f.ix.apply(a.reload);
        let s = f.ix.by_id(&a.id).unwrap();
        assert_eq!((s.title.as_str(), s.state), ("Fresh", State::Todo));
    }
}
