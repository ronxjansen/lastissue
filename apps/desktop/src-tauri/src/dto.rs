//! What crosses the IPC boundary: serde shapes of core and state types.
//! Core stays serde-free; everything here is a plain mirror built from it.
//!
//! Issues are keyed by their path relative to the issues root (`key`):
//! unique even when two files share a short ID, and short enough to send
//! by the thousand.

use crate::format::display_name;
use crate::state::edit::Edit;
use crate::state::search::{Hit, Tier};
use crate::state::view_state::{self, Mode, Scope, ViewState, Visible, group_label};
use lastissue::index::Index;
use lastissue::model::{self, Activity, Priority, State};
use lastissue::query::{GroupKey, Grouping, Sort};
use lastissue::registry::Repo;
use lastissue::source::IssueSummary;
use lastissue::store::mbox::Msg;
use lastissue::store::{Project, Store};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn key_of(store: &Store, path: &Path) -> String {
    store.rel(path)
}

fn millis(d: Option<chrono::DateTime<chrono::FixedOffset>>) -> Option<i64> {
    d.map(|d| d.timestamp_millis())
}

pub fn state_str(s: State) -> &'static str {
    s.as_str()
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

// ------------------------------------------------------------- summaries

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryDto {
    pub key: String,
    pub id: String,
    pub message_id: String,
    pub project: Option<String>,
    pub project_slug: Option<String>,
    pub title: String,
    pub state: &'static str,
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
    pub blocked_by: usize,
    pub has_parent: bool,
}

impl SummaryDto {
    pub fn new(store: &Store, s: &IssueSummary) -> Self {
        SummaryDto {
            key: key_of(store, &s.path),
            id: s.id.clone(),
            message_id: s.message_id.clone(),
            project: s.project.clone(),
            project_slug: s.project_slug.clone(),
            title: s.title.clone(),
            state: state_str(s.state),
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
            blocked_by: s.blocked_by.len(),
            has_parent: s.parent.is_some(),
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
            name: p.slug.replace('-', " "),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum GroupKeyDto {
    State(String),
    Priority(Option<String>),
}

impl GroupKeyDto {
    pub fn to_key(&self) -> Result<GroupKey, String> {
        Ok(match self {
            GroupKeyDto::State(s) => GroupKey::State(parse_state(s)?),
            GroupKeyDto::Priority(p) => GroupKey::Priority(parse_priority(p.as_deref())?),
        })
    }
}

impl From<GroupKey> for GroupKeyDto {
    fn from(k: GroupKey) -> Self {
        match k {
            GroupKey::State(s) => GroupKeyDto::State(state_str(s).into()),
            GroupKey::Priority(p) => GroupKeyDto::Priority(p.map(|p| p.as_str().into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "project", rename_all = "camelCase")]
pub enum ScopeDto {
    Mine,
    All,
    Project(String),
    NoProject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewDto {
    pub mode: String,
    pub scope: ScopeDto,
    pub filter: String,
    pub sort: String,
    pub grouping: String,
}

fn sort_name(s: Sort) -> &'static str {
    match s {
        Sort::Priority => "priority",
        Sort::Updated => "updated",
        Sort::Created => "created",
        Sort::Due => "due",
        Sort::State => "state",
    }
}

impl From<&ViewState> for ViewDto {
    fn from(v: &ViewState) -> Self {
        ViewDto {
            mode: match v.mode {
                Mode::List => "list",
                Mode::Board => "board",
            }
            .into(),
            scope: match &v.scope {
                Scope::Mine => ScopeDto::Mine,
                Scope::All => ScopeDto::All,
                Scope::Project(p) => ScopeDto::Project(p.clone()),
                Scope::NoProject => ScopeDto::NoProject,
            },
            filter: v.filter.clone(),
            sort: sort_name(v.sort).into(),
            grouping: match v.grouping {
                Grouping::State => "state",
                Grouping::Priority => "priority",
            }
            .into(),
        }
    }
}

impl ViewDto {
    /// Apply onto `v`, keeping its selection.
    pub fn apply(&self, v: &mut ViewState) -> Result<(), String> {
        v.mode = match self.mode.as_str() {
            "board" => Mode::Board,
            _ => Mode::List,
        };
        v.scope = match &self.scope {
            ScopeDto::Mine => Scope::Mine,
            ScopeDto::All => Scope::All,
            ScopeDto::Project(p) => Scope::Project(p.clone()),
            ScopeDto::NoProject => Scope::NoProject,
        };
        v.filter = self.filter.clone();
        v.sort = view_state::SORTS
            .iter()
            .map(|(s, _)| *s)
            .find(|s| sort_name(*s) == self.sort)
            .ok_or_else(|| format!("unknown sort {:?}", self.sort))?;
        v.grouping = match self.grouping.as_str() {
            "priority" => Grouping::Priority,
            _ => Grouping::State,
        };
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    pub key: GroupKeyDto,
    pub label: &'static str,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleDto {
    pub view: ViewDto,
    /// Board columns (and every group, in grouping order).
    pub groups: Vec<GroupDto>,
    /// The list's order: indices into `groups`, non-empty ones only.
    pub list: Vec<usize>,
    pub total: usize,
    pub error: Option<String>,
    /// The filter's free-text terms (for highlighting).
    pub text: Vec<String>,
    /// The filter's full-text tier is still running.
    pub scanning: bool,
}

impl VisibleDto {
    pub fn new(store: &Store, view: &ViewState, v: &Visible, scanning: bool) -> Self {
        let groups: Vec<GroupDto> = v
            .groups
            .iter()
            .map(|g| GroupDto {
                key: g.key.into(),
                label: group_label(g.key),
                items: g.items.iter().map(|p| key_of(store, p)).collect(),
            })
            .collect();
        let list = v
            .rows
            .iter()
            .filter_map(|r| match r {
                view_state::ListRow::Header { key, .. } => {
                    v.groups.iter().position(|g| g.key == *key)
                }
                view_state::ListRow::Issue(_) => None,
            })
            .collect();
        VisibleDto {
            view: view.into(),
            groups,
            list,
            total: v.total,
            error: v.error.clone(),
            text: v.text.clone(),
            scanning,
        }
    }
}

// ----------------------------------------------------------------- edits

/// An [`Edit`] over IPC: `{kind: "state", value: "todo"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum EditDto {
    Title(String),
    Body(String),
    State(String),
    Priority(Option<String>),
    Assignee(Option<String>),
    AddLabel(String),
    RemoveLabel(String),
    Due(Option<String>),
    Project(Option<String>),
}

impl EditDto {
    pub fn to_edit(&self) -> Result<Edit, String> {
        Ok(match self.clone() {
            EditDto::Title(t) => Edit::Title(t),
            EditDto::Body(b) => Edit::Body(b),
            EditDto::State(s) => Edit::State(parse_state(&s)?),
            EditDto::Priority(p) => Edit::Priority(parse_priority(p.as_deref())?),
            EditDto::Assignee(a) => Edit::Assignee(a),
            EditDto::AddLabel(l) => Edit::AddLabel(l),
            EditDto::RemoveLabel(l) => Edit::RemoveLabel(l),
            EditDto::Due(d) => Edit::Due(d),
            EditDto::Project(p) => Edit::Project(p),
        })
    }
}

impl From<&Edit> for EditDto {
    fn from(e: &Edit) -> Self {
        match e.clone() {
            Edit::Title(t) => EditDto::Title(t),
            Edit::Body(b) => EditDto::Body(b),
            Edit::State(s) => EditDto::State(state_str(s).into()),
            Edit::Priority(p) => EditDto::Priority(p.map(|p| p.as_str().into())),
            Edit::Assignee(a) => EditDto::Assignee(a),
            Edit::AddLabel(l) => EditDto::AddLabel(l),
            Edit::RemoveLabel(l) => EditDto::RemoveLabel(l),
            Edit::Due(d) => EditDto::Due(d),
            Edit::Project(p) => EditDto::Project(p),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickOptionDto {
    pub label: String,
    pub detail: Option<String>,
    pub checked: bool,
    pub edit: EditDto,
}

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
    pub snippets: Vec<SnippetDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetDto {
    pub loc: String,
    pub text: String,
}

impl HitDto {
    pub fn new(store: &Store, h: &Hit) -> Self {
        HitDto {
            key: key_of(store, &h.path),
            id: h.id.clone(),
            tier: Tier::label(h.tier),
            snippets: h
                .snippets
                .iter()
                .map(|s| SnippetDto {
                    loc: s.loc.clone(),
                    text: s.text.clone(),
                })
                .collect(),
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

// ------------------------------------------------------------------- doc

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationDto {
    pub message_id: String,
    pub key: Option<String>,
    pub id: Option<String>,
    pub title: Option<String>,
    pub state: Option<&'static str>,
}

fn relation(store: &Store, ix: &Index, mid: &str) -> RelationDto {
    let s = ix.by_message_id(mid);
    RelationDto {
        message_id: mid.to_string(),
        key: s.map(|s| key_of(store, &s.path)),
        id: s.map(|s| s.id.clone()),
        title: s.map(|s| s.title.clone()),
        state: s.map(|s| state_str(s.state)),
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
    pub key: String,
    pub id: String,
    pub message_id: Option<String>,
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
    pub watchers: Vec<String>,
    pub parent: Option<RelationDto>,
    pub blocked_by: Vec<RelationDto>,
    pub related: Vec<RelationDto>,
    pub activity: Vec<ActivityDto>,
    pub link: Option<String>,
}

pub fn relations(
    store: &Store,
    ix: &Index,
    s: &IssueSummary,
) -> (Option<RelationDto>, Vec<RelationDto>, Vec<RelationDto>) {
    (
        s.parent.as_deref().map(|m| relation(store, ix, m)),
        s.blocked_by.iter().map(|m| relation(store, ix, m)).collect(),
        s.related.iter().map(|m| relation(store, ix, m)).collect(),
    )
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
