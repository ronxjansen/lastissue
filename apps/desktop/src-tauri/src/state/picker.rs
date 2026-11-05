//! Linear-style property pickers (GUI-SPEC §5, §7): the options for one
//! property of one issue, filtered as the user types. Plain data; the view
//! renders it and commits the chosen [`Edit`] to every target issue (one,
//! or the multi-selection for a bulk edit). The front-end's `⌘K` palette
//! uses the same scoring as [`score`].

use super::edit::Edit;
use super::view_state::{priority_label, state_label};
use lastissue::index::Index;
use lastissue::model::{self, Priority, State};
use lastissue::source::IssueSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    State,
    Priority,
    Assignee,
    Labels,
    Due,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Case-insensitive subsequence match; lower is better, `None` is no match.
/// Prefix beats word start beats scattered.
pub fn score(query: &str, text: &str) -> Option<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let t = text.to_lowercase();
    if t.starts_with(&q) {
        return Some(0);
    }
    if let Some(i) = t.find(&q) {
        let word_start = t[..i].ends_with([' ', '-', '<', '@', '.']);
        return Some(if word_start { 1 } else { 2 + i });
    }
    let mut chars = t.char_indices();
    let mut last = 0;
    for qc in q.chars() {
        let (i, _) = chars.find(|(_, c)| *c == qc)?;
        last = i;
    }
    Some(100 + last)
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
            .map(|st| opt(state_label(st), all(&|s| s.state == st), Edit::State(st)))
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
                priority_label(p),
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
                seen.push(model::email_of(me));
                out.push(PickOption {
                    label: format!("{} (me)", crate::format::display_name(me)),
                    detail: Some(model::email_of(me)),
                    checked: has(Some(model::email_of(me).as_str())),
                    edit: Edit::Assignee(Some("me".into())),
                });
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
            for p in people {
                let e = model::email_of(p);
                if seen.contains(&e) {
                    continue;
                }
                seen.push(e.clone());
                out.push(PickOption {
                    label: crate::format::display_name(p),
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
            let mut projects: Vec<_> = ix.projects().iter().collect();
            projects.sort_by(|a, b| a.slug.cmp(&b.slug));
            for p in projects {
                out.push(opt(
                    p.slug.replace('-', " "),
                    all(&|s| s.project.as_deref() == Some(p.id.as_str())),
                    Edit::Project(Some(p.id.clone())),
                ));
            }
            out
        }
    }
}

/// An option made from the typed text, when it's a value the list doesn't
/// have: a new label, an address, a date.
fn typed(kind: PickerKind, query: &str, targets: &[&IssueSummary]) -> Option<PickOption> {
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
            Some(PickOption {
                label: format!("Add label “{l}”"),
                detail: None,
                checked: false,
                edit: Edit::AddLabel(l),
            })
        }
        PickerKind::Assignee if q.contains('@') => Some(PickOption {
            label: format!("Assign to {q}"),
            detail: None,
            checked: false,
            edit: Edit::Assignee(Some(q.to_string())),
        }),
        PickerKind::Due => {
            let today = chrono::Local::now().date_naive();
            let day = lastissue::dates::parse_day(q, today)?;
            Some(PickOption {
                label: format!("Due {}", day.format("%a %b %-d, %Y")),
                detail: None,
                checked: false,
                edit: Edit::Due(Some(day.format("%Y-%m-%d").to_string())),
            })
        }
        _ => None,
    }
}

/// The visible options for `query`, best match first (ties keep list
/// order), with the typed-value option first when there is one.
pub fn filtered(
    kind: PickerKind,
    query: &str,
    targets: &[&IssueSummary],
    ix: &Index,
    me: Option<&str>,
) -> Vec<PickOption> {
    let mut scored: Vec<(usize, usize, PickOption)> = options(kind, targets, ix, me)
        .into_iter()
        .enumerate()
        .filter_map(|(i, o)| {
            let hay = match &o.detail {
                Some(d) => format!("{} {d}", o.label),
                None => o.label.clone(),
            };
            score(query, &hay).map(|sc| (sc, i, o))
        })
        .collect();
    scored.sort_by_key(|(sc, i, _)| (*sc, *i));
    let exact = scored
        .iter()
        .any(|(_, _, o)| o.label.eq_ignore_ascii_case(query.trim()));
    let mut out: Vec<PickOption> = Vec::new();
    if !exact {
        out.extend(typed(kind, query, targets));
    }
    out.extend(scored.into_iter().map(|(_, _, o)| o));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::identity::Identity;
    use lastissue::index;
    use lastissue::model::Actor;
    use lastissue::ops::{self, NewIssue};
    use lastissue::source::FsScan;
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
        ix.apply(index::scan(&s, &FsScan::new(s.clone())).unwrap());
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
    fn scoring_prefers_prefix_then_word_then_scattered() {
        assert_eq!(score("", "x"), Some(0));
        assert_eq!(score("in", "In progress"), Some(0));
        assert_eq!(score("pro", "In progress"), Some(1));
        assert!(score("ipg", "In progress").unwrap() >= 100);
        assert_eq!(score("zz", "In progress"), None);
    }

    #[test]
    fn state_and_priority_mark_current() {
        let (_d, ix) = fx();
        let s = one(&ix);
        let v = options(PickerKind::State, &[&s], &ix, None);
        assert_eq!(v.len(), 6, "duplicate is not offered");
        assert_eq!(labels(&v)[0], "*Backlog");
        let v = filtered(PickerKind::Priority, "hi", &[&s], &ix, None);
        assert_eq!(labels(&v), ["High"]);
    }

    #[test]
    fn labels_toggle_and_offer_new() {
        let (_d, ix) = fx();
        let s = one(&ix);
        let v = options(PickerKind::Labels, &[&s], &ix, None);
        assert_eq!(labels(&v), ["*bug", "feature", "*ui"]);
        assert_eq!(v[0].edit, Edit::RemoveLabel("bug".into()));
        assert_eq!(v[1].edit, Edit::AddLabel("feature".into()));
        let v = filtered(PickerKind::Labels, "Perf", &[&s], &ix, None);
        assert_eq!(v[0].edit, Edit::AddLabel("perf".into()));
        // An existing label typed exactly isn't offered twice.
        let v = filtered(PickerKind::Labels, "feature", &[&s], &ix, None);
        assert_eq!(labels(&v), ["feature"]);
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
        let v = filtered(PickerKind::Assignee, "carol@x.org", &[&s], &ix, None);
        assert_eq!(v[0].edit, Edit::Assignee(Some("carol@x.org".into())));
        let v = filtered(PickerKind::Assignee, "bob", &[&s], &ix, None);
        assert_eq!(labels(&v), ["*Bob"]);
    }

    #[test]
    fn due_parses_typed_dates_and_projects_list() {
        let (_d, ix) = fx();
        let s = one(&ix);
        let v = filtered(PickerKind::Due, "2031-02-03", &[&s], &ix, None);
        assert_eq!(v[0].edit, Edit::Due(Some("2031-02-03".into())));
        let v = filtered(PickerKind::Due, "nonsense", &[&s], &ix, None);
        assert!(v.is_empty());
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
        assert_eq!(labels(&v)[0], "*Backlog", "both are in the backlog");
        let v = options(PickerKind::Assignee, &both, &ix, None);
        assert!(v.iter().all(|o| !o.checked));
    }
}
