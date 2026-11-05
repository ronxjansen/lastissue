//! What the main view shows (GUI-SPEC §5): list or board, the sidebar
//! scope, the filter text, sort and grouping, and the selected issue.
//! Plain data; [`ViewState::compute`] evaluates it against the index with
//! core's filter, sort and grouping. No GPUI here, so it tests directly.

use super::search::{self, Candidate};
use lastissue::identity::Identity;
use lastissue::index::Index;
use lastissue::model::{Priority, State};
use lastissue::query::{self, Filter, GroupKey, Grouping, ListQuery, Sort};
use lastissue::source::IssueSummary;
use lastissue::store::Store;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    List,
    Board,
}

/// The sidebar selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Scope {
    /// Assigned to me (`@me`).
    Mine,
    #[default]
    All,
    /// Project short ID.
    Project(String),
    NoProject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    pub mode: Mode,
    pub scope: Scope,
    /// Filter tokens plus free text (GUI-SPEC §6).
    pub filter: String,
    pub sort: Sort,
    pub grouping: Grouping,
    /// Short ID of the issue in the detail pane.
    pub selected: Option<String>,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            mode: Mode::List,
            scope: Scope::All,
            filter: String::new(),
            sort: Sort::Priority,
            grouping: Grouping::State,
            selected: None,
        }
    }
}

/// One group of visible issues, by path (look them up in the index).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleGroup {
    pub key: GroupKey,
    pub items: Vec<PathBuf>,
}

/// A row of the grouped list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListRow {
    Header { key: GroupKey, count: usize },
    Issue(PathBuf),
}

/// The evaluated view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Visible {
    pub groups: Vec<VisibleGroup>,
    /// Flattened for the virtualized list: a header per non-empty group.
    pub rows: Vec<ListRow>,
    pub total: usize,
    /// The filter didn't parse or resolve (e.g. `state:nope`, `@me`
    /// without an identity); `groups` is empty then.
    pub error: Option<String>,
    /// The filter's free-text terms (lowercased).
    pub text: Vec<String>,
    /// Issues passing every other criterion whose text the instant tier
    /// didn't find: the full-text tier's work (GUI-SPEC §6).
    pub candidates: Vec<Candidate>,
}

pub const SORTS: [(Sort, &str); 5] = [
    (Sort::Priority, "Priority"),
    (Sort::Updated, "Updated"),
    (Sort::Created, "Created"),
    (Sort::Due, "Due"),
    (Sort::State, "State"),
];

impl ViewState {
    /// The query the scope and filter text stand for.
    pub fn query(&self) -> lastissue::error::Result<ListQuery> {
        let mut q = ListQuery::default();
        match &self.scope {
            Scope::Mine => q.assignee = Some("me".into()),
            Scope::All => {}
            Scope::Project(p) => q.project = Some(p.clone()),
            Scope::NoProject => q.no_project = true,
        }
        q.filter(&self.filter)?;
        Ok(q)
    }

    /// Board columns that show even when empty: every open state (or every
    /// priority), so a column is always there to drop on (M4).
    fn always(&self) -> Vec<GroupKey> {
        match (self.mode, self.grouping) {
            (Mode::List, _) => Vec::new(),
            (Mode::Board, Grouping::State) => State::ALL
                .into_iter()
                .filter(|s| s.is_open())
                .map(GroupKey::State)
                .collect(),
            (Mode::Board, Grouping::Priority) => GroupKey::all(Grouping::Priority),
        }
    }

    #[cfg(test)]
    pub fn compute(&self, index: &Index, store: &Store, me: &Identity) -> Visible {
        self.compute_with(index, store, me, &HashSet::new())
    }

    /// Evaluate against the index. Free text matches in the instant tier,
    /// or for the files in `fulltext` (the full-text tier's hits for the
    /// same terms).
    pub fn compute_with(
        &self,
        index: &Index,
        store: &Store,
        me: &Identity,
        fulltext: &HashSet<PathBuf>,
    ) -> Visible {
        let filter = match self.query().and_then(|q| Filter::resolve(store, me, &q)) {
            Ok(f) => f,
            Err(e) => {
                return Visible {
                    error: Some(error_line(&e)),
                    ..Visible::default()
                };
            }
        };
        let blockers = filter.blockers(index.iter());
        let mut items: Vec<&IssueSummary> = Vec::new();
        let mut candidates = Vec::new();
        for s in index.iter() {
            if !filter.matches_summary_fields(s, &blockers) {
                continue;
            }
            match search::classify(s, &filter.text) {
                Ok(_) => items.push(s),
                Err(c) => {
                    if fulltext.contains(&s.path) {
                        items.push(s);
                    }
                    candidates.push(c);
                }
            }
        }
        query::sort_summaries(&mut items, self.sort);
        let groups: Vec<VisibleGroup> = query::group(&items, self.grouping, &self.always())
            .into_iter()
            .map(|g| VisibleGroup {
                key: g.key,
                items: g.items.iter().map(|s| s.path.clone()).collect(),
            })
            .collect();
        // The list leads with active work (the CLI's state order:
        // in-progress, in-review, todo, backlog, closed); the board keeps
        // the workflow order so neighbouring columns are adjacent states.
        let mut listed: Vec<&VisibleGroup> =
            groups.iter().filter(|g| !g.items.is_empty()).collect();
        listed.sort_by_key(|g| match g.key {
            GroupKey::State(s) => s.sort_rank(),
            GroupKey::Priority(_) => 0,
        });
        let mut rows = Vec::with_capacity(items.len() + groups.len());
        for g in listed {
            rows.push(ListRow::Header {
                key: g.key,
                count: g.items.len(),
            });
            rows.extend(g.items.iter().cloned().map(ListRow::Issue));
        }
        Visible {
            groups,
            rows,
            total: items.len(),
            error: None,
            text: filter.text.clone(),
            candidates,
        }
    }
}

/// `invalid-state: Value: nope` → one line for the filter bar.
fn error_line(e: &lastissue::error::Error) -> String {
    e.to_string().lines().collect::<Vec<_>>().join(" · ")
}

/// Title-case label of a group (`In progress`, `No priority`).
pub fn group_label(key: GroupKey) -> &'static str {
    match key {
        GroupKey::State(s) => state_label(s),
        GroupKey::Priority(p) => priority_label(p),
    }
}

pub fn state_label(s: State) -> &'static str {
    match s {
        State::Backlog => "Backlog",
        State::Todo => "Todo",
        State::InProgress => "In progress",
        State::InReview => "In review",
        State::Done => "Done",
        State::Cancelled => "Cancelled",
        State::Duplicate => "Duplicate",
    }
}

pub fn priority_label(p: Option<Priority>) -> &'static str {
    match p {
        Some(Priority::Urgent) => "Urgent",
        Some(Priority::High) => "High",
        Some(Priority::Medium) => "Medium",
        Some(Priority::Low) => "Low",
        None => "No priority",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::index;
    use lastissue::model::Actor;
    use lastissue::ops::{self, NewIssue};
    use lastissue::source::FsScan;
    use lastissue::store;

    struct Fx {
        _dir: tempfile::TempDir,
        s: Store,
        me: Identity,
        ix: Index,
    }

    fn fx(issues: &[(&str, State, Option<Priority>, bool, bool)]) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(store::init(dir.path()).unwrap());
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        let p = ops::create_project(&s, "Auth").unwrap();
        for (title, state, priority, mine, in_project) in issues {
            let n = NewIssue {
                title: (*title).into(),
                state: Some(*state),
                priority: *priority,
                assignee: mine.then(|| "me".into()),
                project: in_project.then(|| p.id.clone()),
                ..NewIssue::default()
            };
            ops::create_issue(&s, &me, &n).unwrap();
        }
        let mut ix = Index::default();
        ix.apply(index::scan(&s, &FsScan::new(s.clone())).unwrap());
        Fx {
            _dir: dir,
            s,
            me,
            ix,
        }
    }

    fn titles(f: &Fx, v: &Visible) -> Vec<String> {
        v.rows
            .iter()
            .map(|r| match r {
                ListRow::Header { key, count } => format!("# {} {count}", group_label(*key)),
                ListRow::Issue(p) => f.ix.get(p).unwrap().title.clone(),
            })
            .collect()
    }

    fn sample() -> Fx {
        fx(&[
            ("todo low", State::Todo, Some(Priority::Low), true, false),
            (
                "todo urgent",
                State::Todo,
                Some(Priority::Urgent),
                false,
                true,
            ),
            ("wip", State::InProgress, None, true, true),
            ("done", State::Done, None, false, false),
        ])
    }

    #[test]
    fn list_groups_by_state_and_sorts_within() {
        let f = sample();
        let v = ViewState::default().compute(&f.ix, &f.s, &f.me);
        assert_eq!(
            titles(&f, &v),
            [
                "# In progress 1",
                "wip",
                "# Todo 2",
                "todo urgent",
                "todo low"
            ]
        );
        assert_eq!(v.total, 3);
    }

    #[test]
    fn scopes_and_filter_tokens() {
        let f = sample();
        let mut vs = ViewState {
            scope: Scope::Mine,
            ..ViewState::default()
        };
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(
            titles(&f, &v),
            ["# In progress 1", "wip", "# Todo 1", "todo low"]
        );

        let pid = f.ix.projects()[0].id.clone();
        vs.scope = Scope::Project(pid);
        vs.filter = "p:urgent".into();
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(titles(&f, &v), ["# Todo 1", "todo urgent"]);

        vs.scope = Scope::NoProject;
        vs.filter = "state:done".into();
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(titles(&f, &v), ["# Done 1", "done"]);

        vs.filter = "state:nope".into();
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert!(
            v.error.as_deref().unwrap().contains("invalid-state"),
            "{v:?}"
        );
        assert!(v.rows.is_empty());
    }

    #[test]
    fn free_text_takes_full_text_hits() {
        let f = sample();
        let vs = ViewState {
            filter: "needle".into(),
            ..ViewState::default()
        };
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert!(v.rows.is_empty());
        // Every issue is a candidate (a search includes closed ones).
        assert_eq!(v.candidates.len(), 4);
        assert_eq!(v.text, ["needle"]);
        let wip = f.ix.iter().find(|s| s.title == "wip").unwrap().path.clone();
        let v = vs.compute_with(&f.ix, &f.s, &f.me, &HashSet::from([wip]));
        assert_eq!(titles(&f, &v), ["# In progress 1", "wip"]);
        // An instant match needs no scan.
        let vs = ViewState {
            filter: "urgent".into(),
            ..ViewState::default()
        };
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(titles(&f, &v), ["# Todo 1", "todo urgent"]);
    }

    #[test]
    fn board_keeps_open_columns_and_priority_grouping() {
        let f = sample();
        let mut vs = ViewState {
            mode: Mode::Board,
            ..ViewState::default()
        };
        let v = vs.compute(&f.ix, &f.s, &f.me);
        let cols: Vec<(&str, usize)> = v
            .groups
            .iter()
            .map(|g| (group_label(g.key), g.items.len()))
            .collect();
        assert_eq!(
            cols,
            [
                ("Backlog", 0),
                ("Todo", 2),
                ("In progress", 1),
                ("In review", 0)
            ]
        );
        vs.grouping = Grouping::Priority;
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(v.groups.len(), 5);
        vs.mode = Mode::List;
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(
            titles(&f, &v),
            [
                "# Urgent 1",
                "todo urgent",
                "# Low 1",
                "todo low",
                "# No priority 1",
                "wip"
            ]
        );
    }
}
