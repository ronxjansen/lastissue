//! What crosses the IPC boundary: serde shapes of core and state types.
//! Core stays serde-free; everything here is a plain mirror built from it.
//!
//! Issues are keyed by their path relative to the issues root (`key`):
//! unique even when two files share a short ID, and short enough to send
//! by the thousand.

use crate::format::{display_name, error_line, project_name};
use crate::state::draft::FieldKind;
use crate::state::search::{Hit, Snippet, Tier};
use crate::state::view_state::ViewState;
use crate::state::workspace::Workspace;
use lastissue::index::Index;
use lastissue::model::{self, Activity, Priority, State};
use lastissue::query::GroupKey;
use lastissue::registry::Repo;
use lastissue::source::IssueSummary;
use lastissue::store::mbox::Msg;
use lastissue::store::{Project, Store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn millis(d: Option<chrono::DateTime<chrono::FixedOffset>>) -> Option<i64> {
    d.map(|d| d.timestamp_millis())
}

pub fn parse_state(s: &str) -> Result<State, String> {
    State::parse(s).ok_or_else(|| format!("unknown state {s:?}"))
}

pub fn parse_priority(s: Option<&str>) -> Result<Option<Priority>, String> {
    match s {
        None | Some("none" | "") => Ok(None),
        Some(p) => Priority::parse(p)
            .map(Some)
            .ok_or_else(|| format!("unknown priority {p:?}")),
    }
}

/// `#[serde(with)]` for a [`State`] as its canonical name (serde fixes the
/// by-reference signatures).
#[allow(clippy::trivially_copy_pass_by_ref)]
pub mod state_str {
    use super::{State, parse_state};
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(s: &State, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(s.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<State, D::Error> {
        parse_state(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// `#[serde(with)]` for an optional [`Priority`] (`null`, `"none"` or
/// `""` for none).
#[allow(clippy::trivially_copy_pass_by_ref, clippy::ref_option)]
pub mod priority_str {
    use super::{Priority, parse_priority};
    use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer};

    pub fn serialize<S: Serializer>(p: &Option<Priority>, ser: S) -> Result<S::Ok, S::Error> {
        p.map(Priority::as_str).serialize(ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Priority>, D::Error> {
        parse_priority(Option::<String>::deserialize(d)?.as_deref())
            .map_err(serde::de::Error::custom)
    }
}

// ------------------------------------------------------------- summaries

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryDto {
    pub key: String,
    pub id: String,
    pub project: Option<String>,
    pub project_name: Option<String>,
    pub title: String,
    pub state: &'static str,
    /// Not done, cancelled or a duplicate.
    pub open: bool,
    pub priority: Option<&'static str>,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub assignee_name: Option<String>,
    pub author: String,
    pub author_name: String,
    /// Written by an agent (`Sender:`).
    pub agent: bool,
    pub created: Option<i64>,
    pub updated: Option<i64>,
    pub due: Option<i64>,
    pub comment_count: usize,
}

impl SummaryDto {
    pub fn new(store: &Store, s: &IssueSummary) -> Self {
        SummaryDto {
            key: store.rel(&s.path),
            id: s.id.clone(),
            project: s.project.clone(),
            project_name: s.project_slug.as_deref().map(project_name),
            title: s.title.clone(),
            state: s.state.as_str(),
            open: s.state.is_open(),
            priority: s.priority.map(Priority::as_str),
            labels: s.labels.clone(),
            assignee: s.assignee.clone(),
            assignee_name: s.assignee.as_deref().map(display_name),
            author: s.author.clone(),
            author_name: display_name(&s.author),
            agent: s.sender.is_some(),
            created: millis(s.created),
            updated: millis(s.updated),
            due: millis(s.due),
            comment_count: s.comment_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub slug: String,
    pub name: String,
}

impl From<&Project> for ProjectDto {
    fn from(p: &Project) -> Self {
        ProjectDto {
            id: p.id.clone(),
            slug: p.slug.clone(),
            name: project_name(&p.slug),
        }
    }
}

pub fn projects(ix: &Index) -> Vec<ProjectDto> {
    let mut v: Vec<ProjectDto> = ix.projects().iter().map(ProjectDto::from).collect();
    v.sort_by(|a, b| a.slug.cmp(&b.slug));
    v
}

/// A change to the front-end's copy of the index.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDelta {
    /// `changed` is everything; drop what you had.
    pub full: bool,
    pub changed: Vec<SummaryDto>,
    pub removed: Vec<String>,
    pub projects: Option<Vec<ProjectDto>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoDto {
    pub label: String,
    pub root: String,
    pub global: bool,
    pub exists: bool,
}

impl From<&Repo> for RepoDto {
    fn from(r: &Repo) -> Self {
        RepoDto {
            label: r.label.clone(),
            root: r.root.display().to_string(),
            global: r.global,
            exists: r.exists(),
        }
    }
}

// ------------------------------------------------------------------ view

/// Core's [`GroupKey`] over IPC: `{kind: "state", value: "todo"}`.
#[derive(Serialize, Deserialize)]
#[serde(
    remote = "GroupKey",
    tag = "kind",
    content = "value",
    rename_all = "camelCase"
)]
pub enum GroupKeyDef {
    #[serde(with = "state_str")]
    State(State),
    #[serde(with = "priority_str")]
    Priority(Option<Priority>),
}

/// A [`GroupKey`] as a command argument.
#[derive(Deserialize)]
pub struct GroupKeyArg(#[serde(with = "GroupKeyDef")] pub GroupKey);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    #[serde(with = "GroupKeyDef")]
    pub key: GroupKey,
    pub items: Vec<String>,
}

/// Open issues per sidebar scope.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountsDto {
    pub all: usize,
    pub mine: usize,
    pub no_project: usize,
    /// Project short ID → count.
    pub projects: BTreeMap<String, usize>,
}

fn counts(ix: &Index, me: Option<&str>) -> CountsDto {
    let me = me.map(model::email_of);
    let mut c = CountsDto::default();
    for s in ix.iter().filter(|s| s.state.is_open()) {
        c.all += 1;
        match &s.project {
            Some(p) => *c.projects.entry(p.clone()).or_default() += 1,
            None => c.no_project += 1,
        }
        if me.is_some() && s.assignee.as_deref().map(model::email_of) == me {
            c.mine += 1;
        }
    }
    c
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleDto {
    pub view: ViewState,
    /// Board columns (and every group, in grouping order).
    pub groups: Vec<GroupDto>,
    /// The list's order: indices into `groups`, non-empty ones only.
    pub list: Vec<usize>,
    pub total: usize,
    pub error: Option<String>,
    /// The filter's full-text tier is still running.
    pub scanning: bool,
    pub counts: CountsDto,
}

impl VisibleDto {
    pub fn new(ws: &Workspace) -> Self {
        let v = &ws.visible;
        let (groups, counts) = match &ws.repo {
            Some(r) => (
                v.groups
                    .iter()
                    .map(|g| GroupDto {
                        key: g.key,
                        items: g.items.iter().map(|p| r.store.rel(p)).collect(),
                    })
                    .collect(),
                counts(&r.index, ws.me()),
            ),
            None => Default::default(),
        };
        VisibleDto {
            view: ws.view.clone(),
            groups,
            list: v.list.clone(),
            total: v.total,
            error: v.error.clone(),
            scanning: ws.filter_text.running,
            counts,
        }
    }
}

// ----------------------------------------------------------------- edits

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewIssueDto {
    pub title: String,
    pub body: String,
    pub project: Option<String>,
    pub state: Option<String>,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub due: Option<String>,
}

impl NewIssueDto {
    pub fn to_new(&self) -> Result<lastissue::ops::NewIssue, String> {
        Ok(lastissue::ops::NewIssue {
            title: self.title.clone(),
            body: self.body.clone(),
            project: self.project.clone(),
            state: self.state.as_deref().map(parse_state).transpose()?,
            priority: parse_priority(self.priority.as_deref())?,
            labels: self.labels.clone(),
            assignee: self.assignee.clone(),
            due: self.due.clone(),
            ..lastissue::ops::NewIssue::default()
        })
    }
}

// ---------------------------------------------------------------- search

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitDto {
    pub key: String,
    pub id: String,
    pub tier: &'static str,
    pub snippets: Vec<Snippet>,
}

impl HitDto {
    pub fn new(store: &Store, h: &Hit) -> Self {
        HitDto {
            key: store.rel(&h.path),
            id: h.id.clone(),
            tier: Tier::label(h.tier),
            snippets: h.snippets.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDto {
    pub generation: u64,
    pub query: String,
    pub hits: Vec<HitDto>,
    pub scanning: bool,
    pub error: Option<String>,
}

impl SearchDto {
    pub fn new(ws: &Workspace) -> Self {
        let hits = match &ws.repo {
            Some(r) => ws
                .search
                .hits
                .iter()
                .map(|h| HitDto::new(&r.store, h))
                .collect(),
            None => Vec::new(),
        };
        SearchDto {
            generation: ws.search.generation,
            query: ws.search.query.clone(),
            hits,
            scanning: ws.search.scanning,
            error: ws.search.error.clone(),
        }
    }
}

// ------------------------------------------------------------------- doc

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationDto {
    pub message_id: String,
    pub id: Option<String>,
    pub title: Option<String>,
    pub state: Option<&'static str>,
}

fn relation(ix: &Index, mid: &str) -> RelationDto {
    let s = ix.by_message_id(mid);
    RelationDto {
        message_id: mid.to_string(),
        id: s.map(|s| s.id.clone()),
        title: s.map(|s| s.title.clone()),
        state: s.map(|s| s.state.as_str()),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivityDto {
    #[serde(rename_all = "camelCase")]
    Comment {
        index: usize,
        number: usize,
        reply_to: Option<usize>,
        message_id: String,
        who: String,
        date: Option<i64>,
        edited: bool,
        body: String,
    },
    #[serde(rename_all = "camelCase")]
    Event {
        index: usize,
        event: String,
        who: String,
        date: Option<i64>,
        lines: Vec<String>,
    },
}

fn who(m: &Msg) -> String {
    let from = display_name(m.get("From").unwrap_or("?"));
    match m.get("Sender") {
        Some(s) => format!("{from} (via {})", display_name(s)),
        None => from,
    }
}

pub fn activity(msgs: &[Msg]) -> Vec<ActivityDto> {
    model::activity(msgs)
        .into_iter()
        .map(|a| match a {
            Activity::Comment {
                index,
                number,
                reply_to,
                msg,
            } => ActivityDto::Comment {
                index,
                number,
                reply_to,
                message_id: msg.message_id().to_string(),
                who: who(msg),
                date: millis(msg.date()),
                edited: msg.get("X-Edited").is_some(),
                body: msg.body.clone(),
            },
            Activity::Event { index, kind, msg } => ActivityDto::Event {
                index,
                event: kind.to_string(),
                who: who(msg),
                date: millis(msg.date()),
                lines: msg
                    .body
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_string)
                    .collect(),
            },
        })
        .collect()
}

/// One title or body field's draft state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDto {
    /// What the editor should show.
    pub value: String,
    pub dirty: bool,
    /// The disk's text, when it changed under an unsaved draft.
    pub conflict: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocDto {
    pub id: String,
    pub loading: bool,
    pub error: Option<String>,
    pub save_error: Option<String>,
    pub title: Option<FieldDto>,
    pub body: Option<FieldDto>,
    /// An unsaved body draft from an earlier session.
    pub leftover: Option<String>,
    /// Bumped when the editors must take `title`/`body` from here (load,
    /// adopted disk change, "use theirs"); otherwise they keep their text.
    pub reset: u64,
    pub summary: Option<SummaryDto>,
    /// `Cc:` display names.
    pub watchers: Vec<String>,
    pub parent: Option<RelationDto>,
    pub blocked_by: Vec<RelationDto>,
    pub related: Vec<RelationDto>,
    pub activity: Vec<ActivityDto>,
    pub link: Option<String>,
}

impl DocDto {
    pub fn new(ws: &Workspace) -> Option<Self> {
        let d = ws.doc.as_ref()?;
        let r = ws.repo.as_ref()?;
        let field = |k: FieldKind| {
            d.field(k).map(|f| FieldDto {
                value: f.value().to_string(),
                dirty: f.is_dirty(),
                conflict: f.conflict.clone(),
            })
        };
        let s = r.index.get(&d.path).or_else(|| r.index.by_id(&d.id));
        let rel = |m: &String| relation(&r.index, m);
        let watchers = d
            .msgs
            .as_ref()
            .and_then(|m| m.first())
            .map(|h| model::list_of(h, "Cc"))
            .unwrap_or_default()
            .iter()
            .map(|w| display_name(w))
            .collect();
        Some(DocDto {
            id: d.id.clone(),
            loading: d.msgs.is_none() && d.error.is_none(),
            error: d.error.clone(),
            save_error: d.save_error.clone(),
            title: field(FieldKind::Title),
            body: field(FieldKind::Body),
            leftover: d.leftover.clone(),
            reset: d.reset,
            summary: s.map(|s| SummaryDto::new(&r.store, s)),
            watchers,
            parent: s.and_then(|s| s.parent.as_ref()).map(rel),
            blocked_by: s
                .map(|s| s.blocked_by.iter().map(rel).collect())
                .unwrap_or_default(),
            related: s
                .map(|s| s.related.iter().map(rel).collect())
                .unwrap_or_default(),
            activity: d.msgs.as_deref().map(activity).unwrap_or_default(),
            link: Some(r.link(d.id.clone())),
        })
    }
}

// ------------------------------------------------------------------ meta

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaDto {
    pub repos: Vec<RepoDto>,
    /// Index into `repos` of the open one.
    pub current: Option<usize>,
    /// `From:` for writes, if git config has one.
    pub me: Option<String>,
    pub me_error: Option<String>,
    pub loaded: bool,
    /// Why no repo is open, or a registry problem.
    pub notice: Option<String>,
    /// Watcher or scan failure.
    pub failure: Option<String>,
    /// The last write failed.
    pub error: Option<String>,
    pub load_errors: Vec<String>,
}

impl MetaDto {
    pub fn new(ws: &Workspace) -> Self {
        let (me, me_error) = match ws.identity.actor() {
            Ok(a) => (Some(a.from.clone()), None),
            Err(e) => (None, Some(error_line(&e))),
        };
        let current = ws
            .repo
            .as_ref()
            .and_then(|r| ws.repos.iter().position(|x| x.root == r.repo.root));
        MetaDto {
            repos: ws.repos.iter().map(RepoDto::from).collect(),
            current,
            me,
            me_error,
            loaded: ws.repo.as_ref().is_some_and(|r| r.loaded),
            notice: ws.notice.clone(),
            failure: ws.repo.as_ref().and_then(|r| r.failure.clone()),
            error: ws.error.clone(),
            load_errors: ws
                .repo
                .as_ref()
                .map(|r| {
                    r.errors
                        .iter()
                        .map(|(p, e)| format!("{}: {e}", r.store.rel(p)))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub meta: MetaDto,
    pub issues: Vec<SummaryDto>,
    pub projects: Vec<ProjectDto>,
    pub visible: VisibleDto,
    pub doc: Option<DocDto>,
    pub search: SearchDto,
    pub keymap: crate::keymap::UserKeymap,
}

impl Snapshot {
    pub fn new(ws: &Workspace, keymap: crate::keymap::UserKeymap) -> Self {
        let (issues, projects) = match &ws.repo {
            Some(r) => (
                r.index
                    .iter()
                    .map(|s| SummaryDto::new(&r.store, s))
                    .collect(),
                projects(&r.index),
            ),
            None => Default::default(),
        };
        Snapshot {
            meta: MetaDto::new(ws),
            issues,
            projects,
            visible: VisibleDto::new(ws),
            doc: DocDto::new(ws),
            search: SearchDto::new(ws),
            keymap,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::edit::Edit;
    use serde_json::{from_value, json, to_value};

    #[test]
    fn edits_and_group_keys_keep_their_ipc_shape() {
        for (e, j) in [
            (
                Edit::State(State::InProgress),
                json!({"kind": "state", "value": "in-progress"}),
            ),
            (
                Edit::Priority(Some(Priority::High)),
                json!({"kind": "priority", "value": "high"}),
            ),
            (
                Edit::Priority(None),
                json!({"kind": "priority", "value": null}),
            ),
            (
                Edit::AddLabel("x".into()),
                json!({"kind": "addLabel", "value": "x"}),
            ),
            (Edit::Due(None), json!({"kind": "due", "value": null})),
        ] {
            assert_eq!(to_value(&e).unwrap(), j);
            assert_eq!(from_value::<Edit>(j).unwrap(), e);
        }
        let none: Edit = from_value(json!({"kind": "priority", "value": "none"})).unwrap();
        assert_eq!(none, Edit::Priority(None));
        assert!(from_value::<Edit>(json!({"kind": "state", "value": "nope"})).is_err());

        let g = GroupDto {
            key: GroupKey::State(State::Todo),
            items: Vec::new(),
        };
        assert_eq!(
            to_value(&g).unwrap(),
            json!({"key": {"kind": "state", "value": "todo"}, "items": []})
        );
        let GroupKeyArg(k) = from_value(json!({"kind": "priority", "value": null})).unwrap();
        assert_eq!(k, GroupKey::Priority(None));
    }
}
