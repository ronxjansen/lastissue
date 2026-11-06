//! Linear-style property pickers: the options for one
//! property of the target issues (one, or the multi-selection for a bulk
//! edit), and the option the typed text stands for. The front-end ranks
//! them as the user types and commits the chosen [`Edit`]. State and
//! priority options are labelled with their canonical value (`in-progress`,
//! `none`); the front-end titles them.

use super::edit::Edit;
use super::workspace::SharedHub;
use crate::dto;
use crate::format::display_name;
use lastissue::index::Index;
use lastissue::model::{self, Priority, State};
use lastissue::source::IssueSummary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PickerKind {
    State,
    Priority,
    Assignee,
    Labels,
    Due,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PickOption {
    pub label: String,
    /// Extra muted text (an address, a hint).
    pub detail: Option<String>,
    /// The current value (or, for labels, set).
    pub checked: bool,
    pub edit: Edit,
}

fn opt(label: impl Into<String>, checked: bool, edit: Edit) -> PickOption {
    PickOption {
        label: label.into(),
        detail: None,
        checked,
        edit,
    }
}

/// The options for `kind` over `targets` (one issue, or a selection),
/// unfiltered. An option is checked when every target has that value.
pub fn options(
    kind: PickerKind,
    targets: &[&IssueSummary],
    ix: &Index,
    me: Option<&str>,
) -> Vec<PickOption> {
    let all =
        |f: &dyn Fn(&IssueSummary) -> bool| !targets.is_empty() && targets.iter().all(|s| f(s));
    match kind {
        PickerKind::State => State::ALL
            .into_iter()
            // Duplicate needs an original (`issue close --duplicate-of`).
            .filter(|st| *st != State::Duplicate || all(&|s| s.state == State::Duplicate))
            .map(|st| opt(st.as_str(), all(&|s| s.state == st), Edit::State(st)))
            .collect(),
        PickerKind::Priority => [
            None,
            Some(Priority::Urgent),
            Some(Priority::High),
            Some(Priority::Medium),
            Some(Priority::Low),
        ]
        .into_iter()
        .map(|p| {
            opt(
                p.map_or("none", Priority::as_str),
                all(&|s| s.priority == p),
                Edit::Priority(p),
            )
        })
        .collect(),
        PickerKind::Assignee => {
            let has = |e: Option<&str>| {
                all(&|s| s.assignee.as_deref().map(model::email_of).as_deref() == e)
            };
            let mut out = vec![opt("No assignee", has(None), Edit::Assignee(None))];
            let mut seen = Vec::new();
            if let Some(me) = me {
                let e = model::email_of(me);
                out.push(PickOption {
                    label: format!("{} (me)", display_name(me)),
                    detail: Some(e.clone()),
                    checked: has(Some(&e)),
                    edit: Edit::Assignee(Some("me".into())),
                });
                seen.push(e);
            }
            let mut people: Vec<&str> = ix
                .iter()
                .flat_map(|i| {
                    i.assignee
                        .iter()
                        .map(String::as_str)
                        .chain([i.author.as_str()])
                })
                .filter(|a| !a.is_empty())
                .collect();
            people.sort_unstable();
            people.dedup();
            for p in people {
                let e = model::email_of(p);
                if seen.contains(&e) {
                    continue;
                }
                seen.push(e.clone());
                out.push(PickOption {
                    label: display_name(p),
                    detail: Some(e.clone()),
                    checked: has(Some(e.as_str())),
                    edit: Edit::Assignee(Some(p.to_string())),
                });
            }
            out
        }
        PickerKind::Labels => {
            let mut known: Vec<&str> = ix
                .iter()
                .flat_map(|i| i.labels.iter().map(String::as_str))
                .collect();
            known.sort_unstable();
            known.dedup();
            known
                .into_iter()
                .map(|l| {
                    let on = all(&|s| s.labels.iter().any(|x| x == l));
                    let edit = if on {
                        Edit::RemoveLabel(l.to_string())
                    } else {
                        Edit::AddLabel(l.to_string())
                    };
                    opt(l, on, edit)
                })
                .collect()
        }
        PickerKind::Due => {
            let mut out = vec![opt(
                "No due date",
                all(&|s| s.due.is_none()),
                Edit::Due(None),
            )];
            for (label, input) in [
                ("Today", "today"),
                ("Tomorrow", "tomorrow"),
                ("In a week", "+1w"),
                ("In two weeks", "+2w"),
                ("Next Monday", "monday"),
            ] {
                out.push(opt(label, false, Edit::Due(Some(input.into()))));
            }
            out
        }
        PickerKind::Project => {
            let mut out = vec![opt(
                "No project",
                all(&|s| s.project.is_none()),
                Edit::Project(None),
            )];
            for p in dto::projects(ix) {
                let checked = all(&|s| s.project.as_deref() == Some(p.id.as_str()));
                out.push(opt(p.name, checked, Edit::Project(Some(p.id))));
            }
            out
        }
    }
}

/// An option made from the typed text: a new label, an address, a date.
pub fn typed(kind: PickerKind, query: &str, targets: &[&IssueSummary]) -> Option<PickOption> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    match kind {
        PickerKind::Labels => {
            let l = model::normalize_labels([q]).into_iter().next()?;
            if !targets.is_empty() && targets.iter().all(|s| s.labels.contains(&l)) {
                return None;
            }
            Some(opt(format!("Add label “{l}”"), false, Edit::AddLabel(l)))
        }
        PickerKind::Assignee if q.contains('@') => Some(opt(
            format!("Assign to {q}"),
            false,
            Edit::Assignee(Some(q.to_string())),
        )),
        PickerKind::Due => {
            let today = chrono::Local::now().date_naive();
            let day = lastissue::dates::parse_day(q, today)?;
            Some(opt(
                format!("Due {}", day.format("%a %b %-d, %Y")),
                false,
                Edit::Due(Some(day.format("%Y-%m-%d").to_string())),
            ))
        }
        _ => None,
    }
}

/// The picker's options for the target issues, unranked.
pub fn options_for(hub: &SharedHub, kind: PickerKind, ids: &[String]) -> Vec<PickOption> {
    let ws = hub.lock();
    let Some(r) = &ws.repo else {
        return Vec::new();
    };
    let targets: Vec<_> = ids.iter().filter_map(|id| r.index.by_id(id)).collect();
    options(kind, &targets, &r.index, ws.me())
}

/// The option the picker's typed text stands for, if any.
pub fn typed_for(
    hub: &SharedHub,
    kind: PickerKind,
    ids: &[String],
    query: &str,
) -> Option<PickOption> {
    let ws = hub.lock();
    let r = ws.repo.as_ref()?;
    let targets: Vec<_> = ids.iter().filter_map(|id| r.index.by_id(id)).collect();
    typed(kind, query, &targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::identity::Identity;
    use lastissue::index;
    use lastissue::model::Actor;
    use lastissue::ops::{self, NewIssue};
    use lastissue::store::{self, Store};

    fn fx() -> (tempfile::TempDir, Index) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(store::init(dir.path()).unwrap());
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        ops::create_project(&s, "Auth flow").unwrap();
        for (t, labels, who) in [
            ("One", vec!["bug", "ui"], Some("Bob <bob@example.com>")),
            ("Two", vec!["feature"], None),
        ] {
            let n = NewIssue {
                title: t.into(),
                labels: labels.into_iter().map(String::from).collect(),
                assignee: who.map(String::from),
                ..NewIssue::default()
            };
            ops::create_issue(&s, &me, &n).unwrap();
        }
        let mut ix = Index::default();
        ix.apply(index::scan(&s).unwrap());
        (dir, ix)
    }

    fn one(ix: &Index) -> IssueSummary {
        ix.iter().find(|s| s.title == "One").unwrap().clone()
    }

    fn labels(v: &[PickOption]) -> Vec<String> {
        v.iter()
            .map(|o| format!("{}{}", if o.checked { "*" } else { "" }, o.label))
            .collect()
    }

    #[test]
    fn state_and_priority_mark_current() {
        let (_d, ix) = fx();
        let s = one(&ix);
        let v = options(PickerKind::State, &[&s], &ix, None);
        assert_eq!(v.len(), 6, "duplicate is not offered");
        assert_eq!(labels(&v)[0], "*backlog");
        let v = options(PickerKind::Priority, &[&s], &ix, None);
        assert_eq!(labels(&v), ["*none", "urgent", "high", "medium", "low"]);
    }

    #[test]
    fn labels_toggle_and_offer_new() {
        let (_d, ix) = fx();
        let s = one(&ix);
        let v = options(PickerKind::Labels, &[&s], &ix, None);
        assert_eq!(labels(&v), ["*bug", "feature", "*ui"]);
        assert_eq!(v[0].edit, Edit::RemoveLabel("bug".into()));
        assert_eq!(v[1].edit, Edit::AddLabel("feature".into()));
        let t = typed(PickerKind::Labels, "Perf", &[&s]).unwrap();
        assert_eq!(t.edit, Edit::AddLabel("perf".into()));
        // A label every target has isn't offered to add.
        assert_eq!(typed(PickerKind::Labels, "bug", &[&s]), None);
    }

    #[test]
    fn assignees_from_index_with_me_first() {
        let (_d, ix) = fx();
        let s = one(&ix);
        let v = options(
            PickerKind::Assignee,
            &[&s],
            &ix,
            Some("Ada <ada@example.com>"),
        );
        assert_eq!(labels(&v), ["No assignee", "Ada (me)", "*Bob"]);
        let t = typed(PickerKind::Assignee, "carol@x.org", &[&s]).unwrap();
        assert_eq!(t.edit, Edit::Assignee(Some("carol@x.org".into())));
        assert_eq!(typed(PickerKind::Assignee, "bob", &[&s]), None);
    }

    #[test]
    fn due_parses_typed_dates_and_projects_list() {
        let (_d, ix) = fx();
        let s = one(&ix);
        let t = typed(PickerKind::Due, "2031-02-03", &[&s]).unwrap();
        assert_eq!(t.edit, Edit::Due(Some("2031-02-03".into())));
        assert_eq!(typed(PickerKind::Due, "nonsense", &[&s]), None);
        let v = options(PickerKind::Project, &[&s], &ix, None);
        assert_eq!(labels(&v), ["*No project", "auth flow"]);
    }

    #[test]
    fn bulk_checks_only_shared_values() {
        let (_d, ix) = fx();
        let one = one(&ix);
        let two = ix.iter().find(|s| s.title == "Two").unwrap().clone();
        let both = [&one, &two];
        let v = options(PickerKind::Labels, &both, &ix, None);
        // No label is on both: each adds to all.
        assert_eq!(labels(&v), ["bug", "feature", "ui"]);
        assert_eq!(v[0].edit, Edit::AddLabel("bug".into()));
        let v = options(PickerKind::State, &both, &ix, None);
        assert_eq!(labels(&v)[0], "*backlog", "both are in the backlog");
        let v = options(PickerKind::Assignee, &both, &ix, None);
        assert!(v.iter().all(|o| !o.checked));
    }
}
