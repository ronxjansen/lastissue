//! What the main view shows: list or board, the sidebar
//! scope, the filter text, sort and grouping. Plain data (and its IPC
//! shape); [`ViewState::compute`] evaluates it against the index with
//! core's filter, sort and grouping.

use super::search::{self, Candidate};
use lastissue::identity::Identity;
use lastissue::index::Index;
use lastissue::model::State;
use lastissue::query::{self, Filter, GroupKey, Grouping, ListQuery, Sort};
use lastissue::source::IssueSummary;
use lastissue::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    List,
    Board,
}

/// The sidebar selection.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "project", rename_all = "camelCase")]
pub enum Scope {
    /// Assigned to me (`@me`).
    Mine,
    #[default]
    All,
    /// Project short ID.
    Project(String),
    NoProject,
}

/// Core's [`Sort`] over IPC (`"priority"`).
#[derive(Serialize, Deserialize)]
#[serde(remote = "Sort", rename_all = "camelCase")]
enum SortDef {
    State,
    Created,
    Updated,
    Priority,
    Due,
}

/// Core's [`Grouping`] over IPC (`"state"`).
#[derive(Serialize, Deserialize)]
#[serde(remote = "Grouping", rename_all = "camelCase")]
enum GroupingDef {
    State,
    Priority,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    pub mode: Mode,
    pub scope: Scope,
    /// Filter tokens plus free text.
    pub filter: String,
    #[serde(with = "SortDef")]
    pub sort: Sort,
    #[serde(with = "GroupingDef")]
    pub grouping: Grouping,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            mode: Mode::List,
            scope: Scope::All,
            filter: String::new(),
            sort: Sort::Priority,
            grouping: Grouping::State,
        }
    }
}

/// One group of visible issues, by path (look them up in the index).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleGroup {
    pub key: GroupKey,
    pub items: Vec<PathBuf>,
}

/// The evaluated view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Visible {
    pub groups: Vec<VisibleGroup>,
    /// The list's order: indices into `groups`, non-empty ones only.
    pub list: Vec<usize>,
    pub total: usize,
    /// The filter didn't parse or resolve (e.g. `state:nope`, `@me`
    /// without an identity); `groups` is empty then.
    pub error: Option<String>,
    /// Issues passing every other criterion whose text the instant tier
    /// didn't find: the full-text tier's work.
    pub candidates: Vec<Candidate>,
}

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
                    error: Some(crate::format::error_line(&e)),
                    ..Visible::default()
                };
            }
        };
        let mut items: Vec<&IssueSummary> = Vec::new();
        let mut candidates = Vec::new();
        for (s, r) in search::classify_all(index, &filter) {
            match r {
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
        let mut list: Vec<usize> = (0..groups.len())
            .filter(|&i| !groups[i].items.is_empty())
            .collect();
        list.sort_by_key(|&i| match groups[i].key {
            GroupKey::State(s) => s.sort_rank(),
            GroupKey::Priority(_) => 0,
        });
        Visible {
            groups,
            list,
            total: items.len(),
            error: None,
            candidates,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::index;
    use lastissue::model::{Actor, Priority};
    use lastissue::ops::{self, NewIssue};
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
        ix.apply(index::scan(&s).unwrap());
        Fx {
            _dir: dir,
            s,
            me,
            ix,
        }
    }

    fn titles(f: &Fx, v: &Visible) -> Vec<String> {
        let mut out = Vec::new();
        for g in v.list.iter().map(|&i| &v.groups[i]) {
            out.push(format!("# {} {}", g.key.as_str(), g.items.len()));
            out.extend(g.items.iter().map(|p| f.ix.get(p).unwrap().title.clone()));
        }
        out
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
                "# in-progress 1",
                "wip",
                "# todo 2",
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
            ["# in-progress 1", "wip", "# todo 1", "todo low"]
        );

        let pid = f.ix.projects()[0].id.clone();
        vs.scope = Scope::Project(pid);
        vs.filter = "p:urgent".into();
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(titles(&f, &v), ["# todo 1", "todo urgent"]);

        vs.scope = Scope::NoProject;
        vs.filter = "state:done".into();
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(titles(&f, &v), ["# done 1", "done"]);

        vs.filter = "state:nope".into();
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert!(
            v.error.as_deref().unwrap().contains("invalid-state"),
            "{v:?}"
        );
        assert!(v.list.is_empty());
    }

    #[test]
    fn free_text_takes_full_text_hits() {
        let f = sample();
        let vs = ViewState {
            filter: "needle".into(),
            ..ViewState::default()
        };
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert!(v.list.is_empty());
        // Every issue is a candidate (a search includes closed ones).
        assert_eq!(v.candidates.len(), 4);
        let wip = f.ix.iter().find(|s| s.title == "wip").unwrap().path.clone();
        let v = vs.compute_with(&f.ix, &f.s, &f.me, &HashSet::from([wip]));
        assert_eq!(titles(&f, &v), ["# in-progress 1", "wip"]);
        // An instant match needs no scan.
        let vs = ViewState {
            filter: "urgent".into(),
            ..ViewState::default()
        };
        let v = vs.compute(&f.ix, &f.s, &f.me);
        assert_eq!(titles(&f, &v), ["# todo 1", "todo urgent"]);
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
            .map(|g| (g.key.as_str(), g.items.len()))
            .collect();
        assert_eq!(
            cols,
            [
                ("backlog", 0),
                ("todo", 2),
                ("in-progress", 1),
                ("in-review", 0)
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
                "# urgent 1",
                "todo urgent",
                "# low 1",
                "todo low",
                "# none 1",
                "wip"
            ]
        );
    }
}
