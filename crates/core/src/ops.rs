//! Every mutation: issue create / update / close / reopen /
//! move / delete, comment create / edit / delete, project create / rename /
//! delete. The CLI and the GUI call these, so both get identical semantics
//! and identical event messages.
//!
//! Inputs are plain structs. Values a user types keep their typed form:
//! WHO is an address or `me`, dates are friendly date input
//! ([`crate::dates`]), issue references are any unambiguous ID form, and
//! custom headers are `K=V`. Every input is resolved before anything is
//! written, so a bad one leaves the file untouched.

use crate::dates;
use crate::error::{Error, Result};
use crate::identity::Identity;
use crate::model::{
    self, Actor, Change, Priority, State, list_of, push_unique, set_list, state_of,
};
use crate::query::contains_lower;
use crate::store::mbox::{self, Msg};
use crate::store::{self, IssueRef, Project, Store};
use chrono::{DateTime, FixedOffset};
use std::fs;
use std::path::PathBuf;

/// The result of an issue mutation: the issue's messages as written (or as
/// read, when nothing changed) and which of them the operation appended.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub issue: IssueRef,
    pub msgs: Vec<Msg>,
    /// Indices into `msgs` of messages this operation appended (events,
    /// comments) or rewrote (an update event it coalesced into), in order.
    pub appended: Vec<usize>,
    /// Whether the file was written.
    pub written: bool,
}

impl Outcome {
    /// Message #1 followed by the appended messages: what the CLI prints.
    pub fn shown(&self) -> Vec<usize> {
        std::iter::once(0)
            .chain(self.appended.iter().copied())
            .collect()
    }
}

// ---------------------------------------------------------------- helpers

fn date(input: &str) -> Result<String> {
    Ok(dates::parse(input, model::now())?.to_rfc2822())
}

pub(crate) fn issue_message_id(store: &Store, id: &str) -> Result<String> {
    let r = store.resolve_issue(id)?;
    Ok(store.read_head(&r)?.message_id().to_string())
}

/// Message-ID for an ID argument; refuses self-references.
fn relation_target(store: &Store, id: &str, own: Option<&str>) -> Result<String> {
    let mid = issue_message_id(store, id)?;
    if Some(mid.as_str()) == own {
        return Err(Error::validation("self-reference")
            .with("Id", id)
            .with("Hint", "an issue cannot relate to itself"));
    }
    Ok(mid)
}

fn relation_targets(store: &Store, ids: &[String], own: Option<&str>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for id in ids {
        push_unique(&mut out, relation_target(store, id, own)?);
    }
    Ok(out)
}

/// For removals: resolve if possible, else match stored IDs by ULID prefix
/// (the target issue may have been deleted).
fn removal_target(store: &Store, id: &str, existing: &[String]) -> Option<String> {
    if let Ok(mid) = issue_message_id(store, id) {
        return Some(mid);
    }
    let q = mbox::ulid_of(id).to_ascii_uppercase();
    existing
        .iter()
        .find(|e| !q.is_empty() && mbox::ulid_of(e).to_ascii_uppercase().starts_with(&q))
        .cloned()
}

fn apply_custom_headers(m: &mut Msg, headers: &[String]) -> Result<()> {
    for kv in headers {
        let (k, v) = model::parse_custom_header(kv)?;
        m.set(&k, &v);
    }
    Ok(())
}

/// After header changes: enforce the duplicate rule.
fn validate_duplicate(m: &mut Msg) -> Result<()> {
    if state_of(m) == State::Duplicate {
        if !m.has("References") {
            return Err(Error::validation("duplicate-needs-original").with(
                "Hint",
                "use: lastissue issue close <id> --duplicate-of <original-id>",
            ));
        }
    } else {
        m.remove("References");
    }
    Ok(())
}

/// How long an update burst stays open, from its event's `Date`.
pub const BURST_SECS: i64 = 15 * 60;

/// The event line a body edit records (no diff).
pub const BODY_EDITED: &str = "body: edited";

/// Index of the last message when it is an `update` event that the next
/// change by `actor` at `now` merges into: same `From` and
/// `Sender`, and its `Date` less than [`BURST_SECS`] old.
fn open_burst(msgs: &[Msg], actor: &Actor, now: DateTime<FixedOffset>) -> Option<usize> {
    let i = msgs.len().checked_sub(1).filter(|i| *i > 0)?;
    let ev = &msgs[i];
    let age = now.timestamp() - ev.date()?.timestamp();
    (ev.get("X-Issue-Event") == Some("update")
        && ev.get("From") == Some(actor.from.as_str())
        && ev.get("Sender") == actor.sender.as_deref()
        && (0..BURST_SECS).contains(&age))
    .then_some(i)
}

/// The field name of an event line (`state` in `state: a → b`).
fn line_field(line: &str) -> &str {
    line.split_once(": ").map_or(line, |(f, _)| f)
}

/// Fold `c` into an existing line for the same field: keep the line's
/// original "from", take the new "to". `None`: "from" equals the new "to",
/// so the field is back where the burst started and the line goes.
fn merge_line(line: &str, c: &Change) -> Option<String> {
    let rest = line.split_once(": ").map_or("", |(_, r)| r);
    // The line's "to" is the value `c` started from; strip it to get the
    // original "from" (robust to values containing " → "). A hand-edited
    // line falls back to splitting at the first arrow.
    let cur = format!(" → {}", c.from.as_deref().unwrap_or(model::NONE));
    let from = rest
        .strip_suffix(cur.as_str())
        .or_else(|| rest.split_once(" → ").map(|(f, _)| f))
        .unwrap_or(rest);
    let to = c.to.as_deref().unwrap_or(model::NONE);
    (from != to).then(|| format!("{}: {from} → {to}", c.field))
}

/// Record changes of message #1 (and a body edit) as an `update` event:
/// merged into the open burst if there is one, else appended.
/// The merged event keeps its `Date` (the burst start); a field it
/// already lists keeps its original "from", and a field changed back is
/// dropped, as is an event left with no lines. Returns the index of the
/// event holding the changes, or `None` when there is none.
///
/// Every update event an operation here writes goes through this (the
/// Linear import writes its own, never coalesced); `now` is a parameter so
/// the burst window can be tested.
pub fn record_update(
    msgs: &mut Vec<Msg>,
    actor: &Actor,
    changes: &[Change],
    body_edited: bool,
    now: DateTime<FixedOffset>,
) -> Option<usize> {
    if changes.is_empty() && !body_edited {
        return None;
    }
    let Some(i) = open_burst(msgs, actor, now) else {
        let mut lines: Vec<String> = changes.iter().map(Change::line).collect();
        if body_edited {
            lines.push(BODY_EDITED.to_string());
        }
        let mut ev = model::event(actor, &msgs[0], "update", &lines);
        // Stamp with the caller's clock, not a second read of the wall
        // clock: the burst window is measured against this `now`.
        ev.set("Date", &now.to_rfc2822());
        msgs.push(ev);
        return Some(msgs.len() - 1);
    };
    let mut lines: Vec<String> = msgs[i].body.lines().map(str::to_string).collect();
    for c in changes {
        match lines.iter().position(|l| line_field(l) == c.field) {
            Some(j) => match merge_line(&lines[j], c) {
                Some(l) => lines[j] = l,
                None => {
                    lines.remove(j);
                }
            },
            None => lines.push(c.line()),
        }
    }
    if body_edited && !lines.iter().any(|l| l == BODY_EDITED) {
        lines.push(BODY_EDITED.to_string());
    }
    if lines.is_empty() {
        msgs.remove(i);
        return None;
    }
    let subject = model::reply_subject(msgs[0].get("Subject").unwrap_or(""));
    let ev = &mut msgs[i];
    ev.set("Subject", &subject);
    ev.body = lines.join("\n");
    Some(i)
}

fn read_issue(store: &Store, id: &str) -> Result<(IssueRef, Vec<Msg>)> {
    let r = store.resolve_issue(id)?;
    let msgs = store.read(&r)?;
    Ok((r, msgs))
}

fn unchanged(issue: IssueRef, msgs: Vec<Msg>) -> Outcome {
    Outcome {
        issue,
        msgs,
        appended: Vec::new(),
        written: false,
    }
}

fn check_body(body: &str) -> Result<()> {
    if body.trim().is_empty() {
        return Err(Error::validation("empty-body").with("Hint", "the body must not be empty"));
    }
    Ok(())
}

// ---------------------------------------------------------------- create

/// `issue create`.
#[derive(Debug, Clone, Default)]
pub struct NewIssue {
    pub title: String,
    /// Project reference (ID prefix or folder name); `None`: the root.
    pub project: Option<String>,
    pub body: String,
    /// Initial state; `None`: backlog.
    pub state: Option<State>,
    pub priority: Option<Priority>,
    /// Labels (comma-separated entries allowed).
    pub labels: Vec<String>,
    /// WHO.
    pub assignee: Option<String>,
    /// WHO each.
    pub watchers: Vec<String>,
    /// Date input.
    pub due: Option<String>,
    /// Date input.
    pub expires: Option<String>,
    /// Issue references.
    pub parent: Option<String>,
    pub blocked_by: Vec<String>,
    pub related: Vec<String>,
    /// `K=V` custom headers.
    pub headers: Vec<String>,
}

pub fn create_issue(store: &Store, me: &Identity, new: &NewIssue) -> Result<Outcome> {
    let title = mbox::clean_value(&new.title);
    if title.is_empty() {
        return Err(Error::validation("empty-title").with("Hint", "pass --title \"…\""));
    }
    let state = new.state.unwrap_or(State::Backlog);
    if state == State::Duplicate {
        return Err(Error::validation("duplicate-needs-original").with(
            "Hint",
            "create the issue, then: lastissue issue close <id> --duplicate-of <original-id>",
        ));
    }
    let project = new
        .project
        .as_deref()
        .map(|p| store.resolve_project(p))
        .transpose()?;

    let assignee = new.assignee.as_deref().map(|w| me.who(w)).transpose()?;
    let mut watchers = Vec::new();
    for w in &new.watchers {
        push_unique(&mut watchers, me.who(w)?);
    }
    let due = new.due.as_deref().map(date).transpose()?;
    let expires = new.expires.as_deref().map(date).transpose()?;
    let parent = new
        .parent
        .as_deref()
        .map(|p| relation_target(store, p, None))
        .transpose()?;
    let blocked = relation_targets(store, &new.blocked_by, None)?;
    let related = relation_targets(store, &new.related, None)?;
    let actor = me.actor()?;

    let existing: Vec<String> = store.issues()?.into_iter().map(|r| r.id).collect();
    let (ulid, id) = store::fresh_id(&existing);
    let mut m = model::base_message(actor, &title);
    m.set("Message-ID", &model::message_id(ulid));
    m.set_opt("To", assignee.as_deref());
    set_list(&mut m, "Cc", &watchers);
    m.set("X-Issue-State", state.as_str());
    m.set_opt("X-Priority-Level", new.priority.map(Priority::as_str));
    set_list(
        &mut m,
        "Keywords",
        &model::normalize_labels(new.labels.iter().map(String::as_str)),
    );
    m.set_opt("Reply-By", due.as_deref());
    m.set_opt("Expires", expires.as_deref());
    m.set_opt("List-Id", project.as_ref().map(Project::list_id).as_deref());
    m.set_opt("X-Parent", parent.as_deref());
    set_list(&mut m, "X-Blocked-By", &blocked);
    set_list(&mut m, "X-Related", &related);
    apply_custom_headers(&mut m, &new.headers)?;
    model::set_content_headers(&mut m);
    m.body = mbox::normalize_body(&new.body);

    let slug = store::slugify(&title, 4, "issue");
    let path = store.issue_path(project.as_ref(), &slug, &id);
    let issue = IssueRef { id, path, project };
    let msgs = vec![m];
    store.write(&issue.path, &msgs)?;
    Ok(Outcome {
        issue,
        msgs,
        appended: Vec::new(),
        written: true,
    })
}

// ---------------------------------------------------------------- update

/// `issue update`. For `Option<Option<T>>` fields, `Some(None)`
/// removes the header.
#[derive(Debug, Clone, Default)]
pub struct IssueUpdate {
    pub title: Option<String>,
    /// New body (an in-place edit, marked `X-Edited`).
    pub body: Option<String>,
    pub state: Option<State>,
    pub priority: Option<Option<Priority>>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    /// WHO.
    pub assignee: Option<Option<String>>,
    /// WHO each.
    pub watch: Vec<String>,
    pub unwatch: Vec<String>,
    /// Date input.
    pub due: Option<Option<String>>,
    /// Date input.
    pub expires: Option<Option<String>>,
    /// Issue references.
    pub parent: Option<Option<String>>,
    pub add_blocked_by: Vec<String>,
    pub remove_blocked_by: Vec<String>,
    pub add_related: Vec<String>,
    pub remove_related: Vec<String>,
    /// `K=V` custom headers; an empty V removes.
    pub headers: Vec<String>,
}

pub fn update_issue(store: &Store, me: &Identity, id: &str, u: &IssueUpdate) -> Result<Outcome> {
    let (r, mut msgs) = read_issue(store, id)?;
    let before = msgs[0].clone();
    let own = before.message_id().to_string();

    // Resolve all inputs first so a bad one leaves the file untouched.
    let who: &dyn Fn(&str) -> Result<String> = &|w| me.who(w);
    let relation: &dyn Fn(&str) -> Result<String> = &|p| relation_target(store, p, Some(&own));
    // Set-or-clear fields: header → new value (`None` removes it).
    let mut set_or_clear: Vec<(&str, Option<String>)> = Vec::new();
    for (header, field, resolve) in [
        ("To", &u.assignee, who),
        ("Reply-By", &u.due, &date),
        ("Expires", &u.expires, &date),
        ("X-Parent", &u.parent, relation),
    ] {
        if let Some(v) = field {
            set_or_clear.push((header, v.as_deref().map(resolve).transpose()?));
        }
    }
    let watch: Vec<String> = u.watch.iter().map(|w| me.who(w)).collect::<Result<_>>()?;
    let unwatch: Vec<String> = u
        .unwatch
        .iter()
        .map(|w| me.who_pattern(w))
        .collect::<Result<_>>()?;
    let add_blocked = relation_targets(store, &u.add_blocked_by, Some(&own))?;
    let add_related = relation_targets(store, &u.add_related, Some(&own))?;

    let m = &mut msgs[0];
    if let Some(t) = &u.title {
        let t = mbox::clean_value(t);
        if t.is_empty() {
            return Err(Error::validation("empty-title"));
        }
        m.set("Subject", &t);
    }
    if let Some(s) = u.state {
        m.set("X-Issue-State", s.as_str());
    }
    if let Some(p) = u.priority {
        m.set_opt("X-Priority-Level", p.map(Priority::as_str));
    }
    if !u.add_labels.is_empty() || !u.remove_labels.is_empty() {
        let remove = model::normalize_labels(u.remove_labels.iter().map(String::as_str));
        let labels: Vec<String> = model::normalize_labels(
            m.get("Keywords")
                .into_iter()
                .chain(u.add_labels.iter().map(String::as_str)),
        )
        .into_iter()
        .filter(|l| !remove.contains(l))
        .collect();
        set_list(m, "Keywords", &labels);
    }
    for (header, v) in &set_or_clear {
        m.set_opt(header, v.as_deref());
    }
    if !watch.is_empty() || !unwatch.is_empty() {
        let mut cc = list_of(m, "Cc");
        for w in watch {
            if !cc.iter().any(|c| model::email_of(c) == model::email_of(&w)) {
                cc.push(w);
            }
        }
        cc.retain(|c| {
            !unwatch
                .iter()
                .any(|u| model::email_of(c) == model::email_of(u) || contains_lower(c, u))
        });
        set_list(m, "Cc", &cc);
    }
    for (header, add, remove) in [
        ("X-Blocked-By", &add_blocked, &u.remove_blocked_by),
        ("X-Related", &add_related, &u.remove_related),
    ] {
        if add.is_empty() && remove.is_empty() {
            continue;
        }
        let mut cur = list_of(m, header);
        for x in add {
            push_unique(&mut cur, x.clone());
        }
        for id in remove {
            match removal_target(store, id, &cur) {
                Some(mid) => cur.retain(|c| c != &mid),
                None => {
                    return Err(Error::not_found("relation-not-found")
                        .with("Id", id.as_str())
                        .with("Header", header));
                }
            }
        }
        set_list(m, header, &cur);
    }
    apply_custom_headers(m, &u.headers)?;
    validate_duplicate(m)?;

    let mut body_changed = false;
    if let Some(b) = &u.body {
        let b = mbox::normalize_body(b);
        if b != m.body {
            m.body = b;
            model::mark_edited(m, me.actor()?);
            body_changed = true;
        }
    }
    commit(store, me, r, msgs, &before, body_changed)
}

/// Record every changed header of message #1 and a body edit as one update
/// event (coalesced) and write the file, unless nothing
/// changed at all.
fn commit(
    store: &Store,
    me: &Identity,
    r: IssueRef,
    mut msgs: Vec<Msg>,
    before: &Msg,
    body_changed: bool,
) -> Result<Outcome> {
    let changes = model::diff(before, &msgs[0]);
    if changes.is_empty() && !body_changed {
        return Ok(unchanged(r, msgs));
    }
    let appended = record_update(&mut msgs, me.actor()?, &changes, body_changed, now())
        .into_iter()
        .collect();
    store.write(&r.path, &msgs)?;
    Ok(Outcome {
        issue: r,
        msgs,
        appended,
        written: true,
    })
}

fn now() -> DateTime<FixedOffset> {
    model::now().fixed_offset()
}

// ---------------------------------------------------------------- close / reopen

/// How `issue close` closes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CloseAs {
    #[default]
    Done,
    Cancelled,
    /// Duplicate of this issue reference (sets `References:`).
    DuplicateOf(String),
}

pub fn close_issue(
    store: &Store,
    me: &Identity,
    id: &str,
    how: &CloseAs,
    comment: Option<&str>,
) -> Result<Outcome> {
    let (r, mut msgs) = read_issue(store, id)?;
    let before = msgs[0].clone();
    let original = match how {
        CloseAs::DuplicateOf(d) => Some(relation_target(store, d, Some(before.message_id()))?),
        _ => None,
    };
    if comment.is_some_and(|c| c.trim().is_empty()) {
        return Err(Error::validation("empty-body").with("Hint", "--comment must not be empty"));
    }
    let state = match how {
        CloseAs::Done => State::Done,
        CloseAs::Cancelled => State::Cancelled,
        CloseAs::DuplicateOf(_) => State::Duplicate,
    };
    msgs[0].set("X-Issue-State", state.as_str());
    msgs[0].set_opt("References", original.as_deref());
    validate_duplicate(&mut msgs[0])?;
    let changes = model::diff(&before, &msgs[0]);
    let mut appended: Vec<usize> = record_update(&mut msgs, me.actor()?, &changes, false, now())
        .into_iter()
        .collect();
    if let Some(c) = comment {
        msgs.push(model::comment(me.actor()?, &msgs[0], None, c));
        appended.push(msgs.len() - 1);
    }
    let written = !changes.is_empty() || comment.is_some();
    if written {
        store.write(&r.path, &msgs)?;
    }
    Ok(Outcome {
        issue: r,
        msgs,
        appended,
        written,
    })
}

/// `issue reopen`: state → todo (a duplicate's `References:` is cleared).
pub fn reopen_issue(store: &Store, me: &Identity, id: &str) -> Result<Outcome> {
    let (r, mut msgs) = read_issue(store, id)?;
    let before = msgs[0].clone();
    msgs[0].set("X-Issue-State", State::Todo.as_str());
    validate_duplicate(&mut msgs[0])?;
    commit(store, me, r, msgs, &before, false)
}

// ---------------------------------------------------------------- move / delete

/// `issue move`: into the project `project` refers to, or to the root
/// (`None`). Appends a `move` event (never coalesced).
pub fn move_issue(
    store: &Store,
    me: &Identity,
    id: &str,
    project: Option<&str>,
) -> Result<Outcome> {
    let r = store.resolve_issue(id)?;
    let target = project.map(|p| store.resolve_project(p)).transpose()?;
    let mut msgs = store.read(&r)?;
    let before = msgs[0].clone();
    let file_name = r
        .path
        .file_name()
        .ok_or_else(|| Error::validation("invalid-path").with("Path", store.rel(&r.path)))?
        .to_owned();
    let new_path = target
        .as_ref()
        .map_or(&store.root, |p| &p.dir)
        .join(file_name);
    msgs[0].set_opt("List-Id", target.as_ref().map(Project::list_id).as_deref());
    if new_path == r.path && before == msgs[0] {
        return Ok(unchanged(r, msgs));
    }
    let mut lines = model::diff_lines(&before, &msgs[0]);
    lines.push(format!(
        "path: {} → {}",
        store.rel(&r.path),
        store.rel(&new_path)
    ));
    msgs.push(model::event(me.actor()?, &msgs[0], "move", &lines));
    store.write(&new_path, &msgs)?;
    if new_path != r.path {
        fs::remove_file(&r.path)?;
    }
    let last = msgs.len() - 1;
    Ok(Outcome {
        issue: IssueRef {
            id: r.id,
            path: new_path,
            project: target,
        },
        msgs,
        appended: vec![last],
        written: true,
    })
}

/// A deleted issue: where it was and its message #1.
#[derive(Debug, Clone)]
pub struct Deleted {
    pub issue: IssueRef,
    pub head: Msg,
}

/// `issue delete`: removes the file.
pub fn delete_issue(store: &Store, id: &str) -> Result<Deleted> {
    let issue = store.resolve_issue(id)?;
    let head = store.read_head(&issue)?;
    fs::remove_file(&issue.path)?;
    Ok(Deleted { issue, head })
}

// ---------------------------------------------------------------- comments

/// A resolved comment: its issue, the issue's messages, and its index.
#[derive(Debug, Clone)]
pub struct CommentRef {
    pub issue: IssueRef,
    pub msgs: Vec<Msg>,
    pub idx: usize,
}

impl CommentRef {
    /// The comment's number (`n` in `<issue>/<n>`).
    pub fn number(&self) -> Option<usize> {
        model::comment_numbers(&self.msgs)[self.idx]
    }
}

const COMMENT_REF_HINT: &str = "address a comment as <issue>/<n> (n from 1, see: lastissue comment list <issue>) or by Message-ID prefix";

fn invalid_comment_ref(q: &str) -> Error {
    Error::usage("invalid-comment-ref")
        .with("Value", q)
        .with("Hint", COMMENT_REF_HINT)
}

/// Resolve `<issue>/<n>`, or a comment Message-ID (ULID) prefix.
pub fn find_comment(store: &Store, q: &str) -> Result<CommentRef> {
    let q = q.trim();
    if let Some((issue, n)) = q.rsplit_once('/') {
        if issue.is_empty() {
            return Err(invalid_comment_ref(q));
        }
        let n: usize = n.parse().map_err(|_| invalid_comment_ref(q))?;
        let (r, msgs) = read_issue(store, issue)?;
        let nums = model::comment_numbers(&msgs);
        let count = nums.iter().flatten().count();
        let idx = nums.iter().position(|x| *x == Some(n)).ok_or_else(|| {
            Error::not_found("comment-not-found")
                .with("Comment", format!("{}/{n}", r.id))
                .with(
                    "Valid",
                    if count == 0 {
                        "(no comments)".to_string()
                    } else {
                        format!("1..{count}")
                    },
                )
        })?;
        return Ok(CommentRef {
            issue: r,
            msgs,
            idx,
        });
    }
    let needle = model::normalize_id_input(mbox::ulid_of(q));
    if needle.is_empty() || !model::is_short_id(&needle) {
        return Err(invalid_comment_ref(q));
    }
    let mut hits: Vec<(CommentRef, String)> = Vec::new();
    for r in store.issues()? {
        let msgs = store.read(&r)?;
        let nums = model::comment_numbers(&msgs);
        for (i, n) in nums.iter().enumerate() {
            let Some(n) = n else { continue };
            if mbox::ulid_of(msgs[i].message_id())
                .to_lowercase()
                .starts_with(&needle)
            {
                let label = format!("{}/{n}", r.id);
                hits.push((
                    CommentRef {
                        issue: r.clone(),
                        msgs: msgs.clone(),
                        idx: i,
                    },
                    label,
                ));
            }
        }
    }
    match hits.len() {
        0 => Err(Error::not_found("comment-not-found")
            .with("Comment", q)
            .with("Hint", COMMENT_REF_HINT)),
        1 => Ok(hits.remove(0).0),
        _ => Err(Error::ambiguous(hits.into_iter().map(|h| h.1).collect())),
    }
}

/// Where a new comment goes: the issue, and the comment it replies to.
#[derive(Debug, Clone)]
pub struct CommentTarget {
    pub issue: IssueRef,
    /// Message-ID of the parent comment; `None`: a flat comment.
    pub reply_to: Option<String>,
}

/// Resolve where `comment create` writes. `reply_to` is a comment
/// reference; `/n` or `n` are shorthand for a comment on this same issue.
/// Separate from [`add_comment`] so a front-end can validate the target
/// before asking for the body.
pub fn comment_target(store: &Store, id: &str, reply_to: Option<&str>) -> Result<CommentTarget> {
    let r = store.resolve_issue(id)?;
    let reply_to = match reply_to {
        Some(rt) => {
            let rt = rt.trim();
            let rt = if rt.starts_with('/') || rt.chars().all(|c| c.is_ascii_digit()) {
                format!("{}/{}", r.id, rt.trim_start_matches('/'))
            } else {
                rt.to_string()
            };
            let f = find_comment(store, &rt)?;
            if f.issue.path != r.path {
                return Err(Error::validation("reply-to-other-issue").with(
                    "Hint",
                    format!("--reply-to must reference a comment on {}", r.id),
                ));
            }
            Some(f.msgs[f.idx].message_id().to_string())
        }
        None => None,
    };
    Ok(CommentTarget { issue: r, reply_to })
}

/// Append a comment. Ends an event burst.
pub fn add_comment(
    store: &Store,
    me: &Identity,
    target: &CommentTarget,
    body: &str,
) -> Result<CommentRef> {
    check_body(body)?;
    let mut msgs = store.read(&target.issue)?;
    let parent = target
        .reply_to
        .as_deref()
        .and_then(|p| msgs.iter().find(|m| m.message_id() == p))
        .cloned();
    let c = model::comment(me.actor()?, &msgs[0], parent.as_ref(), body);
    msgs.push(c);
    store.write(&target.issue.path, &msgs)?;
    Ok(CommentRef {
        issue: target.issue.clone(),
        idx: msgs.len() - 1,
        msgs,
    })
}

/// Replace a comment's body in place (marks `X-Edited`); no write when the
/// normalized body is unchanged.
pub fn edit_comment(
    store: &Store,
    me: &Identity,
    mut c: CommentRef,
    body: &str,
) -> Result<CommentRef> {
    check_body(body)?;
    let text = mbox::normalize_body(body);
    if text != c.msgs[c.idx].body {
        let actor = me.actor()?;
        let m = &mut c.msgs[c.idx];
        m.body = text;
        model::mark_edited(m, actor);
        store.write(&c.issue.path, &c.msgs)?;
    }
    Ok(c)
}

/// Remove a comment and append a `comment-deleted` event (never coalesced).
pub fn delete_comment(store: &Store, me: &Identity, c: CommentRef) -> Result<Outcome> {
    let n = c.number().unwrap_or(0);
    let CommentRef {
        issue,
        mut msgs,
        idx,
    } = c;
    let removed = msgs.remove(idx);
    let line = format!(
        "deleted: /{n} {} by {}",
        removed.message_id(),
        removed.get("From").unwrap_or("(unknown)")
    );
    let ev = model::event(me.actor()?, &msgs[0], "comment-deleted", &[line]);
    msgs.push(ev);
    store.write(&issue.path, &msgs)?;
    let last = msgs.len() - 1;
    Ok(Outcome {
        issue,
        msgs,
        appended: vec![last],
        written: true,
    })
}

// ---------------------------------------------------------------- projects

/// `project create`: a folder `<slug>-<id>`, the slug being the first two
/// words of `name`.
pub fn create_project(store: &Store, name: &str) -> Result<Project> {
    new_project(
        store,
        store::slugify(name, 2, "project"),
        &store.projects()?,
    )
}

/// Create the folder for a project `slug`, its ID unique among `existing`.
pub(crate) fn new_project(store: &Store, slug: String, existing: &[Project]) -> Result<Project> {
    let ids: Vec<String> = existing.iter().map(|p| p.id.clone()).collect();
    let (_, id) = store::fresh_id(&ids);
    let mut p = Project {
        id,
        slug,
        dir: PathBuf::new(),
    };
    p.dir = store.root.join(p.dir_name());
    fs::create_dir(&p.dir)?;
    Ok(p)
}

/// `project rename`: new slug, same id; the issues' `List-Id` follows.
pub fn rename_project(store: &Store, project: &str, name: &str) -> Result<Project> {
    let old = store.resolve_project(project)?;
    let mut new = Project {
        slug: store::slugify(name, 2, "project"),
        ..old.clone()
    };
    new.dir = store.root.join(new.dir_name());
    if new.dir != old.dir {
        if new.dir.exists() {
            return Err(Error::validation("project-exists").with("Path", store.rel(&new.dir)));
        }
        fs::rename(&old.dir, &new.dir)?;
    }
    // Keep List-Id display names in step with the folder (the source of truth).
    for r in store.project_issues(&new)? {
        let mut msgs = store.read(&r)?;
        if msgs[0].get("List-Id") != Some(new.list_id().as_str()) {
            msgs[0].set("List-Id", &new.list_id());
            store.write(&r.path, &msgs)?;
        }
    }
    Ok(new)
}

/// `project delete`: refuses a non-empty folder unless `force`.
pub fn delete_project(store: &Store, project: &str, force: bool) -> Result<Project> {
    let p = store.resolve_project(project)?;
    let count = fs::read_dir(&p.dir)?.count();
    if count > 0 && !force {
        let issues = store.project_issues(&p)?.len();
        return Err(Error::validation("project-not-empty")
            .with("Project", p.dir_name())
            .with("Issues", issues.to_string())
            .with(
                "Hint",
                format!(
                    "move the issues out first, or: lastissue project delete {} --force",
                    p.id
                ),
            ));
    }
    fs::remove_dir_all(&p.dir)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Store, Identity) {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(dir.path()).unwrap();
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: Some("Bot <bot@example.com>".into()),
        });
        (dir, Store::new(root), me)
    }

    fn new(title: &str) -> NewIssue {
        NewIssue {
            title: title.into(),
            ..NewIssue::default()
        }
    }

    #[test]
    fn create_writes_head_only() {
        let (_d, s, me) = setup();
        let o = create_issue(
            &s,
            &me,
            &NewIssue {
                labels: vec!["Bug, auth".into()],
                assignee: Some("me".into()),
                body: "hello".into(),
                ..new("Login crashes on empty password")
            },
        )
        .unwrap();
        assert!(o.written && o.appended.is_empty());
        assert!(
            o.issue
                .path
                .ends_with(format!("login-crashes-on-empty-{}.mbox", o.issue.id))
        );
        let msgs = s.read(&o.issue).unwrap();
        assert_eq!(msgs.len(), 1);
        let h = &msgs[0];
        assert_eq!(h.get("Keywords"), Some("bug, auth"));
        assert_eq!(h.get("To"), Some("Ada <ada@example.com>"));
        assert_eq!(h.get("Sender"), Some("Bot <bot@example.com>"));
        assert_eq!(state_of(h), State::Backlog);
        assert!(create_issue(&s, &me, &new("  ")).is_err());
    }

    #[test]
    fn update_appends_one_event_and_noop_writes_nothing() {
        let (_d, s, me) = setup();
        let id = create_issue(&s, &me, &new("A")).unwrap().issue.id;
        let u = IssueUpdate {
            state: Some(State::InProgress),
            priority: Some(Some(Priority::High)),
            add_labels: vec!["x".into()],
            ..IssueUpdate::default()
        };
        let o = update_issue(&s, &me, &id, &u).unwrap();
        assert_eq!(o.shown(), [0, 1]);
        assert_eq!(o.msgs[1].get("X-Issue-Event"), Some("update"));
        assert_eq!(
            o.msgs[1].body.lines().count(),
            3,
            "one line per field: {}",
            o.msgs[1].body
        );
        let again = update_issue(&s, &me, &id, &u).unwrap();
        assert!(!again.written && again.appended.is_empty());
        assert_eq!(s.read(&again.issue).unwrap().len(), 2);
    }

    #[test]
    fn body_edit_marks_edited_and_records_event() {
        let (_d, s, me) = setup();
        let id = create_issue(&s, &me, &new("A")).unwrap().issue.id;
        let u = IssueUpdate {
            body: Some("new body".into()),
            ..IssueUpdate::default()
        };
        let o = update_issue(&s, &me, &id, &u).unwrap();
        assert!(o.written);
        assert!(o.msgs[0].has("X-Edited"));
        assert_eq!(o.appended, [1]);
        assert_eq!(o.msgs[1].body, BODY_EDITED);
        // A second edit in the burst keeps one line.
        let u = IssueUpdate {
            body: Some("newer body".into()),
            state: Some(State::Todo),
            ..IssueUpdate::default()
        };
        let o = update_issue(&s, &me, &id, &u).unwrap();
        assert_eq!(o.msgs.len(), 2);
        assert_eq!(o.msgs[1].body, "body: edited\nstate: backlog → todo");
    }

    #[test]
    fn updates_coalesce_until_comment_or_other_actor() {
        let (_d, s, me) = setup();
        let id = create_issue(&s, &me, &new("A")).unwrap().issue.id;
        let set = |st: State| IssueUpdate {
            state: Some(st),
            ..IssueUpdate::default()
        };
        update_issue(&s, &me, &id, &set(State::Todo)).unwrap();
        let o = update_issue(&s, &me, &id, &set(State::InProgress)).unwrap();
        assert_eq!(o.appended, [1], "merged into the first event");
        assert_eq!(o.msgs[1].body, "state: backlog → in-progress");
        // Changed back: the line and then the empty event go.
        let o = update_issue(&s, &me, &id, &set(State::Backlog)).unwrap();
        assert!(o.written && o.appended.is_empty());
        assert_eq!(s.read(&o.issue).unwrap().len(), 1);

        // A comment ends the burst.
        update_issue(&s, &me, &id, &set(State::Todo)).unwrap();
        add_comment(&s, &me, &comment_target(&s, &id, None).unwrap(), "hi").unwrap();
        let o = update_issue(&s, &me, &id, &set(State::InProgress)).unwrap();
        assert_eq!(o.msgs.len(), 4);
        assert_eq!(o.msgs[3].body, "state: todo → in-progress");

        // Another actor (same From, other Sender) ends it too.
        let human = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        let o = update_issue(&s, &human, &id, &set(State::InReview)).unwrap();
        assert_eq!(o.msgs.len(), 5);
        // Close coalesces into the human's burst.
        let o = close_issue(&s, &human, &id, &CloseAs::Done, None).unwrap();
        assert_eq!(o.msgs.len(), 5);
        assert_eq!(o.msgs[4].body, "state: in-progress → done");
    }

    #[test]
    fn move_and_comment_deleted_never_coalesce() {
        let (_d, s, me) = setup();
        let p = create_project(&s, "P").unwrap();
        let id = create_issue(&s, &me, &new("A")).unwrap().issue.id;
        move_issue(&s, &me, &id, Some(&p.id)).unwrap();
        let u = IssueUpdate {
            state: Some(State::Todo),
            ..IssueUpdate::default()
        };
        let o = update_issue(&s, &me, &id, &u).unwrap();
        assert_eq!(o.msgs.len(), 3, "update after a move starts a new event");
        let o = move_issue(&s, &me, &id, None).unwrap();
        assert_eq!(o.msgs.len(), 4, "a move after an update is its own event");
    }

    // ------------------------------------------------------------ coalescing

    mod coalescing {
        use super::*;
        use proptest::prelude::*;

        const FIELDS: &[&str] = &["X-Issue-State", "X-Priority-Level", "To", "Keywords"];
        const VALUES: &[&str] = &["a", "b", "c → d", "e: f"];

        fn actor(sender: Option<&str>) -> Actor {
            Actor {
                from: "Ada <ada@example.com>".into(),
                sender: sender.map(str::to_string),
            }
        }

        fn head() -> Msg {
            let mut m = model::base_message(&actor(None), "T");
            m.set("X-Issue-State", "a");
            m
        }

        /// One step: header writes (`None` removes) and maybe a body edit.
        type Step = (Vec<(usize, Option<usize>)>, bool);

        fn step() -> impl Strategy<Value = Step> {
            (
                prop::collection::vec((0..FIELDS.len(), prop::option::of(0..VALUES.len())), 0..4),
                prop::bool::weighted(0.2),
            )
        }

        /// Apply a step to message #1 and record it.
        fn apply(msgs: &mut Vec<Msg>, a: &Actor, st: &Step, now: DateTime<FixedOffset>) {
            let before = msgs[0].clone();
            for (f, v) in &st.0 {
                msgs[0].set_opt(FIELDS[*f], v.map(|v| VALUES[v]));
            }
            let changes = model::diff(&before, &msgs[0]);
            record_update(msgs, a, &changes, st.1, now);
        }

        fn sorted(mut v: Vec<String>) -> Vec<String> {
            v.sort();
            v
        }

        proptest! {
            /// A burst by one actor leaves at most one event; it lists each
            /// field once with its original "from" and drops fields changed
            /// back; its Date stays at the burst start.
            #[test]
            fn one_event_per_burst(steps in prop::collection::vec(step(), 1..12)) {
                let a = actor(Some("Bot <bot@x>"));
                let original = head();
                let mut msgs = vec![original.clone()];
                let now = model::now().fixed_offset();
                let mut edited = false;
                for st in &steps {
                    let date_before = msgs.get(1).map(|e| e.get("Date").unwrap().to_string());
                    apply(&mut msgs, &a, st, now);
                    edited |= st.1;
                    prop_assert!(msgs.len() <= 2);
                    if let (Some(d), Some(e)) = (date_before, msgs.get(1)) {
                        prop_assert_eq!(Some(d.as_str()), e.get("Date"));
                    }
                }
                let want = model::diff_lines(&original, &msgs[0]);
                match msgs.get(1) {
                    None => prop_assert!(want.is_empty() && !edited),
                    Some(ev) => {
                        let lines: Vec<String> = ev.body.lines().map(str::to_string).collect();
                        let body = lines.iter().filter(|l| *l == BODY_EDITED).count();
                        prop_assert!(body <= 1);
                        let fields: Vec<String> =
                            lines.into_iter().filter(|l| l != BODY_EDITED).collect();
                        prop_assert_eq!(sorted(fields), sorted(want));
                        prop_assert!(!ev.body.is_empty());
                    }
                }
            }

            /// A different actor never merges: every step that changed
            /// something appends its own event, listing exactly its diff.
            #[test]
            fn other_actor_ends_burst(steps in prop::collection::vec(step(), 1..8)) {
                let actors = [actor(None), actor(Some("Bot <bot@x>"))];
                let mut msgs = vec![head()];
                let now = model::now().fixed_offset();
                for st in &steps {
                    let before = msgs[0].clone();
                    let n = msgs.len();
                    // Never the actor of the last event.
                    apply(&mut msgs, &actors[n % 2], st, now);
                    let want = model::diff_lines(&before, &msgs[0]);
                    if want.is_empty() && !st.1 {
                        prop_assert_eq!(msgs.len(), n);
                    } else {
                        prop_assert_eq!(msgs.len(), n + 1);
                        let mut lines = want;
                        if st.1 { lines.push(BODY_EDITED.into()); }
                        prop_assert_eq!(&msgs[n].body, &lines.join("\n"));
                    }
                }
            }
        }

        #[test]
        fn burst_window_is_15_minutes_from_start() {
            let a = actor(None);
            let mut msgs = vec![head()];
            let t0 = model::now().fixed_offset();
            let st = |v: usize| -> Step { (vec![(0, Some(v))], false) };
            apply(&mut msgs, &a, &st(1), t0);
            let start = msgs[1].date().unwrap();
            apply(
                &mut msgs,
                &a,
                &st(2),
                start + chrono::Duration::seconds(BURST_SECS - 1),
            );
            assert_eq!(msgs.len(), 2);
            assert_eq!(msgs[1].date(), Some(start));
            apply(
                &mut msgs,
                &a,
                &st(3),
                start + chrono::Duration::seconds(BURST_SECS),
            );
            assert_eq!(msgs.len(), 3, "a burst never outlives its start + 15 min");
        }

        #[test]
        fn non_update_events_and_comments_end_burst() {
            let a = actor(None);
            let st = (vec![(0, Some(1))], false);
            for kind in ["move", "comment-deleted"] {
                let mut msgs = vec![head()];
                msgs.push(model::event(&a, &msgs[0], kind, &["x: y → z".into()]));
                apply(&mut msgs, &a, &st, model::now().fixed_offset());
                assert_eq!(msgs.len(), 3, "{kind}");
                assert_eq!(msgs[1].body, "x: y → z");
            }
            let mut msgs = vec![head()];
            msgs.push(model::comment(&a, &msgs[0], None, "hi"));
            apply(&mut msgs, &a, &st, model::now().fixed_offset());
            assert_eq!(msgs.len(), 3);
        }
    }

    #[test]
    fn bad_input_leaves_file_untouched() {
        let (_d, s, me) = setup();
        let o = create_issue(&s, &me, &new("A")).unwrap();
        let before = fs::read(&o.issue.path).unwrap();
        let u = IssueUpdate {
            state: Some(State::Done),
            add_blocked_by: vec![o.issue.id.clone()],
            ..IssueUpdate::default()
        };
        let e = update_issue(&s, &me, &o.issue.id, &u).unwrap_err();
        assert_eq!(e.kind, "self-reference");
        assert_eq!(fs::read(&o.issue.path).unwrap(), before);
    }

    #[test]
    fn close_duplicate_and_reopen() {
        let (_d, s, me) = setup();
        let a = create_issue(&s, &me, &new("A")).unwrap().issue.id;
        let b = create_issue(&s, &me, &new("B")).unwrap().issue.id;
        let o = close_issue(&s, &me, &b, &CloseAs::DuplicateOf(a), Some("dup")).unwrap();
        assert_eq!(state_of(&o.msgs[0]), State::Duplicate);
        assert!(o.msgs[0].has("References"));
        assert_eq!(o.appended.len(), 2, "event + comment");
        let o = reopen_issue(&s, &me, &b).unwrap();
        assert_eq!(state_of(&o.msgs[0]), State::Todo);
        assert!(!o.msgs[0].has("References"));
        assert!(
            update_issue(
                &s,
                &me,
                &b,
                &IssueUpdate {
                    state: Some(State::Duplicate),
                    ..IssueUpdate::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn move_between_project_and_root() {
        let (_d, s, me) = setup();
        let p = create_project(&s, "New feature work").unwrap();
        assert_eq!(p.slug, "new-feature");
        let id = create_issue(&s, &me, &new("A")).unwrap().issue.id;
        let o = move_issue(&s, &me, &id, Some(&p.id)).unwrap();
        assert!(o.issue.path.starts_with(&p.dir));
        assert_eq!(o.msgs[o.appended[0]].get("X-Issue-Event"), Some("move"));
        let noop = move_issue(&s, &me, &id, Some(&p.id)).unwrap();
        assert!(!noop.written);
        let back = move_issue(&s, &me, &id, None).unwrap();
        assert_eq!(back.issue.path.parent(), Some(s.root.as_path()));
        assert!(!back.msgs[0].has("List-Id"));
    }

    #[test]
    fn comments_round_trip() {
        let (_d, s, me) = setup();
        let id = create_issue(&s, &me, &new("A")).unwrap().issue.id;
        let t = comment_target(&s, &id, None).unwrap();
        let c1 = add_comment(&s, &me, &t, "first").unwrap();
        assert_eq!(c1.number(), Some(1));
        let t = comment_target(&s, &id, Some("/1")).unwrap();
        let c2 = add_comment(&s, &me, &t, "reply").unwrap();
        assert_eq!(
            c2.msgs[c2.idx].get("In-Reply-To"),
            Some(c1.msgs[1].message_id())
        );
        let f = find_comment(&s, &format!("{id}/2")).unwrap();
        let e = edit_comment(&s, &me, f, "edited").unwrap();
        assert!(e.msgs[e.idx].has("X-Edited"));
        assert!(add_comment(&s, &me, &t, " \n").is_err());
        let d = delete_comment(&s, &me, find_comment(&s, &format!("{id}/1")).unwrap()).unwrap();
        assert_eq!(
            d.msgs[d.appended[0]].get("X-Issue-Event"),
            Some("comment-deleted")
        );
        assert_eq!(find_comment(&s, &format!("{id}/2")).unwrap_err().code, 2);
    }

    #[test]
    fn project_rename_updates_list_id_and_delete_guards() {
        let (_d, s, me) = setup();
        let p = create_project(&s, "Old name").unwrap();
        let id = create_issue(
            &s,
            &me,
            &NewIssue {
                project: Some(p.id.clone()),
                ..new("A")
            },
        )
        .unwrap()
        .issue
        .id;
        let np = rename_project(&s, &p.id, "Fresh name").unwrap();
        let r = s.resolve_issue(&id).unwrap();
        assert_eq!(
            s.read_head(&r).unwrap().get("List-Id"),
            Some(np.list_id().as_str())
        );
        assert_eq!(
            delete_project(&s, &np.id, false).unwrap_err().kind,
            "project-not-empty"
        );
        delete_project(&s, &np.id, true).unwrap();
        assert!(s.issues().unwrap().is_empty());
    }
}
