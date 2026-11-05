//! The same filters and sort orders over in-memory [`IssueSummary`] heads
//! (GUI-SPEC §3, §5, §6), plus grouping for the list and board. No I/O:
//! a front-end resolves a [`Filter`] once and re-evaluates it on every
//! keystroke or watcher change.

use super::{Filter, HeadFields, Sort, contains_lower, lower, sort_key};
use crate::model::{Priority, State};
use crate::source::IssueSummary;
use std::borrow::Cow;
use std::collections::HashMap;

impl HeadFields for IssueSummary {
    fn state(&self) -> State {
        self.state
    }
    fn priority(&self) -> Option<Priority> {
        self.priority
    }
    fn created(&self) -> i64 {
        self.created.map_or(0, |d| d.timestamp())
    }
    fn due(&self) -> Option<i64> {
        self.due.map(|d| d.timestamp())
    }
    fn expires(&self) -> Option<i64> {
        self.expires.map(|d| d.timestamp())
    }
    fn labels(&self) -> Cow<'_, [String]> {
        Cow::Borrowed(&self.labels)
    }
    fn assignee(&self) -> Option<&str> {
        self.assignee.as_deref()
    }
    fn author(&self) -> Option<&str> {
        Some(&self.author)
    }
    fn watchers(&self) -> Cow<'_, [String]> {
        Cow::Borrowed(&self.watchers)
    }
    fn has_sender(&self) -> bool {
        self.sender.is_some()
    }
    fn parent(&self) -> Option<&str> {
        self.parent.as_deref()
    }
}

/// Message-ID → state of every loaded issue, for `is:ready` /
/// `is:blocked` without touching the disk.
#[derive(Debug, Default)]
pub struct Blockers<'a>(HashMap<&'a str, State>);

impl<'a> Blockers<'a> {
    fn new(all: impl IntoIterator<Item = &'a IssueSummary>) -> Self {
        Blockers(
            all.into_iter()
                .map(|s| (s.message_id.as_str(), s.state))
                .collect(),
        )
    }

    /// At least one blocker exists and is open (SPEC §6.4).
    fn is_blocked(&self, s: &IssueSummary) -> bool {
        s.blocked_by
            .iter()
            .any(|b| self.0.get(b.as_str()).is_some_and(|st| st.is_open()))
    }
}

impl Filter {
    /// The [`Blockers`] [`Filter::matches_summary_fields`] needs: the full
    /// map for `is:ready` / `is:blocked`, else an empty one (building it
    /// over 100k issues cost ~5 ms per evaluation).
    pub fn blockers<'a>(&self, all: impl IntoIterator<Item = &'a IssueSummary>) -> Blockers<'a> {
        if self.ready || self.blocked {
            Blockers::new(all)
        } else {
            Blockers::default()
        }
    }

    /// Everything [`super::list`] checks, in memory, except the free text:
    /// scope, header fields and `is:ready` / `is:blocked`. The caller
    /// checks the text ([`instant_match`], or the full-text tier,
    /// GUI-SPEC §6).
    pub fn matches_summary_fields(&self, s: &IssueSummary, blockers: &Blockers) -> bool {
        self.in_scope(s.project.as_deref())
            && self.matches_fields(s)
            && self.passes_blocked(blockers.is_blocked(s))
    }
}

/// Where a search term matched, best first (GUI-SPEC §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Title,
    /// Short ID, a label, the assignee or the project.
    Field,
    Body,
    Comment,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Title => "title",
            Tier::Field => "label/ID",
            Tier::Body => "body",
            Tier::Comment => "comment",
        }
    }
}

/// The best instant tier (title or field) for one lowercased term.
pub fn term_tier(s: &IssueSummary, t: &str) -> Option<Tier> {
    if contains_lower(&s.title, t) {
        return Some(Tier::Title);
    }
    let field = s.id.starts_with(t)
        || s.labels.iter().any(|l| contains_lower(l, t))
        || s.assignee.as_deref().is_some_and(|a| contains_lower(a, t))
        || s.project_slug
            .as_deref()
            .is_some_and(|p| contains_lower(p, t))
        || s.project.as_deref().is_some_and(|p| p.starts_with(t));
    field.then_some(Tier::Field)
}

/// Every term occurs (case-insensitively) in the title, short ID, a
/// label, the assignee or the project (GUI-SPEC §6, tier 1).
pub fn instant_match(s: &IssueSummary, terms: &[String]) -> bool {
    terms.iter().all(|t| term_tier(s, &lower(t)).is_some())
}

/// Sort summaries like `issue list --sort` sorts rows.
pub fn sort_summaries(items: &mut [&IssueSummary], sort: Sort) {
    items.sort_by_cached_key(|s| sort_key(sort, *s, s.updated.map(|d| d.timestamp())));
}

/// What the list and board group by (GUI-SPEC §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Grouping {
    #[default]
    State,
    Priority,
}

/// One group: a state column or a priority band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupKey {
    State(State),
    Priority(Option<Priority>),
}

impl GroupKey {
    /// Every key of a grouping, in display order: the workflow order of
    /// states (backlog → done → cancelled → duplicate, so the board's
    /// neighbours are the previous/next state), or urgent → none.
    pub fn all(by: Grouping) -> Vec<GroupKey> {
        match by {
            Grouping::State => State::ALL.into_iter().map(GroupKey::State).collect(),
            Grouping::Priority => [
                Some(Priority::Urgent),
                Some(Priority::High),
                Some(Priority::Medium),
                Some(Priority::Low),
                None,
            ]
            .into_iter()
            .map(GroupKey::Priority)
            .collect(),
        }
    }

    pub fn of(by: Grouping, s: &IssueSummary) -> GroupKey {
        match by {
            Grouping::State => GroupKey::State(s.state),
            Grouping::Priority => GroupKey::Priority(s.priority),
        }
    }

    /// The canonical value (`in-progress`, `high`, `none`).
    pub fn as_str(self) -> &'static str {
        match self {
            GroupKey::State(s) => s.as_str(),
            GroupKey::Priority(Some(p)) => p.as_str(),
            GroupKey::Priority(None) => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Group<'a> {
    pub key: GroupKey,
    pub items: Vec<&'a IssueSummary>,
}

/// Split already sorted items into groups, keeping their order within
/// each. A group shows when it has items or is listed in `always` (the
/// board keeps every open-state column, as a drop target).
pub fn group<'a>(items: &[&'a IssueSummary], by: Grouping, always: &[GroupKey]) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = GroupKey::all(by)
        .into_iter()
        .map(|key| Group {
            key,
            items: Vec::new(),
        })
        .collect();
    for s in items {
        let k = GroupKey::of(by, s);
        if let Some(g) = groups.iter_mut().find(|g| g.key == k) {
            g.items.push(s);
        }
    }
    groups.retain(|g| !g.items.is_empty() || always.contains(&g.key));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::model::Actor;
    use crate::ops::{self, IssueUpdate, NewIssue};
    use crate::query::{ListQuery, list};
    use crate::source;
    use crate::store::{self, Store};

    struct Fx {
        _dir: tempfile::TempDir,
        s: Store,
        me: Identity,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(store::init(dir.path()).unwrap());
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        Fx { _dir: dir, s, me }
    }

    fn mk(f: &Fx, n: NewIssue) -> String {
        ops::create_issue(&f.s, &f.me, &n).unwrap().issue.id
    }

    fn issue(title: &str) -> NewIssue {
        NewIssue {
            title: title.into(),
            ..NewIssue::default()
        }
    }

    fn summaries(f: &Fx) -> Vec<IssueSummary> {
        f.s.issues()
            .unwrap()
            .iter()
            .map(|r| source::load(&f.s, r).unwrap())
            .collect()
    }

    /// In-memory evaluation agrees with the disk-based `list` (as sets:
    /// fixtures share a creation second).
    fn agree(f: &Fx, filter: &str) -> Vec<String> {
        let mut q = ListQuery {
            sort: Some(Sort::Created),
            ..ListQuery::default()
        };
        q.filter(filter).unwrap();
        let mut disk: Vec<String> = list(&f.s, &f.me, &q)
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.issue.id)
            .collect();
        disk.sort();
        let all = summaries(f);
        let flt = Filter::resolve(&f.s, &f.me, &q).unwrap();
        let blockers = Blockers::new(&all);
        let mut mem: Vec<&IssueSummary> = all
            .iter()
            .filter(|s| flt.matches_summary_fields(s, &blockers) && instant_match(s, &flt.text))
            .collect();
        sort_summaries(&mut mem, Sort::Created);
        let mut mem: Vec<String> = mem.into_iter().map(|s| s.id.clone()).collect();
        mem.sort();
        assert_eq!(mem, disk, "filter {filter:?}");
        mem
    }

    #[test]
    fn memory_filters_agree_with_list() {
        let f = fx();
        let p = ops::create_project(&f.s, "Auth flow").unwrap();
        let a = mk(
            &f,
            NewIssue {
                labels: vec!["bug".into()],
                priority: Some(Priority::High),
                project: Some(p.id.clone()),
                assignee: Some("Bob <bob@example.com>".into()),
                state: Some(State::Todo),
                ..issue("Login fails")
            },
        );
        let b = mk(
            &f,
            NewIssue {
                state: Some(State::Todo),
                blocked_by: vec![a.clone()],
                ..issue("Blocked one")
            },
        );
        let c = mk(&f, issue("Closed one"));
        ops::close_issue(&f.s, &f.me, &c, &ops::CloseAs::Done, None).unwrap();

        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!(agree(&f, ""), sorted(vec![a.clone(), b.clone()]));
        assert_eq!(agree(&f, "label:bug p:high"), vec![a.clone()]);
        assert_eq!(agree(&f, "project:auth"), vec![a.clone()]);
        assert_eq!(agree(&f, "@bob"), vec![a.clone()]);
        assert_eq!(agree(&f, "is:blocked"), vec![b.clone()]);
        assert_eq!(agree(&f, "is:ready"), vec![a.clone()]);
        assert_eq!(agree(&f, "state:done"), vec![c.clone()]);
        // Free text includes closed issues, like a search.
        assert_eq!(agree(&f, "one"), sorted(vec![b.clone(), c.clone()]));

        // Unblocking flips is:ready / is:blocked.
        ops::close_issue(&f.s, &f.me, &a, &ops::CloseAs::Done, None).unwrap();
        assert_eq!(agree(&f, "is:ready"), [b]);
        assert!(agree(&f, "is:blocked").is_empty());
    }

    #[test]
    fn instant_tier_fields() {
        let f = fx();
        let p = ops::create_project(&f.s, "Payments").unwrap();
        let id = mk(
            &f,
            NewIssue {
                labels: vec!["needs-review".into()],
                project: Some(p.id),
                ..issue("Refund flow")
            },
        );
        let all = summaries(&f);
        let s = &all[0];
        let t = |w: &str| instant_match(s, &[w.to_lowercase()]);
        assert!(t("REFUND") && t(&id[..4]) && t("review") && t("paym"));
        assert!(!t("nothing"));
        assert!(instant_match(s, &["refund".into(), "flow".into()]));
        assert!(!instant_match(s, &["refund".into(), "nope".into()]));
    }

    #[test]
    fn grouping_keeps_order_and_always_columns() {
        let f = fx();
        let mut ids = Vec::new();
        for (t, st, p) in [
            ("a", State::Todo, Some(Priority::Low)),
            ("b", State::InProgress, None),
            ("c", State::Todo, Some(Priority::Urgent)),
        ] {
            ids.push(mk(
                &f,
                NewIssue {
                    state: Some(st),
                    priority: p,
                    ..issue(t)
                },
            ));
        }
        let all = summaries(&f);
        let mut items: Vec<&IssueSummary> = all.iter().collect();
        sort_summaries(&mut items, Sort::Priority);
        let open: Vec<GroupKey> = State::ALL
            .into_iter()
            .filter(|s| s.is_open())
            .map(GroupKey::State)
            .collect();
        let g = group(&items, Grouping::State, &open);
        let shape: Vec<(&str, Vec<&str>)> = g
            .iter()
            .map(|g| {
                (
                    g.key.as_str(),
                    g.items.iter().map(|s| s.title.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("backlog", vec![]),
                ("todo", vec!["c", "a"]),
                ("in-progress", vec!["b"]),
                ("in-review", vec![]),
            ]
        );
        let g = group(&items, Grouping::Priority, &[]);
        let keys: Vec<&str> = g.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["urgent", "low", "none"]);

        // Updated order follows the newest message.
        ops::update_issue(
            &f.s,
            &f.me,
            &ids[0],
            &IssueUpdate {
                title: Some("a2".into()),
                ..IssueUpdate::default()
            },
        )
        .unwrap();
        let all = summaries(&f);
        let mut items: Vec<&IssueSummary> = all.iter().collect();
        sort_summaries(&mut items, Sort::Updated);
        assert_eq!(items[0].title, "a2");
    }
}
