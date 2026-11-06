//! Domain types and normalization: states, priorities, labels, addresses,
//! and constructors for issue / comment / event messages.

use crate::error::{Error, Result};
use crate::store::mbox::{self, Msg};
use chrono::{DateTime, Local};
use ulid::Ulid;

// ---------------------------------------------------------------- State

/// With the `clap` feature, variant order and aliases define the CLI's
/// `--help` and `Valid:` list; aliases must agree with [`State::parse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum State {
    Backlog,
    Todo,
    #[cfg_attr(
        feature = "clap",
        value(
            alias = "in progress",
            alias = "in_progress",
            alias = "inprogress",
            alias = "doing",
            alias = "wip"
        )
    )]
    InProgress,
    #[cfg_attr(
        feature = "clap",
        value(
            alias = "in review",
            alias = "in_review",
            alias = "inreview",
            alias = "review"
        )
    )]
    InReview,
    #[cfg_attr(
        feature = "clap",
        value(alias = "complete", alias = "completed", alias = "closed")
    )]
    Done,
    #[cfg_attr(feature = "clap", value(alias = "canceled"))]
    Cancelled,
    #[cfg_attr(feature = "clap", value(alias = "duplicated", alias = "dup"))]
    Duplicate,
}

impl State {
    pub const ALL: [State; 7] = [
        State::Backlog,
        State::Todo,
        State::InProgress,
        State::InReview,
        State::Done,
        State::Cancelled,
        State::Duplicate,
    ];

    /// Lenient parse: case-insensitive, spaces/underscores become dashes.
    /// Accepts the canonical names plus aliases (`wip`, `review`, `closed`, …).
    pub fn parse(s: &str) -> Option<State> {
        let norm: String = s
            .trim()
            .to_lowercase()
            .split([' ', '_', '-'])
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        Some(match norm.as_str() {
            "backlog" => State::Backlog,
            "todo" => State::Todo,
            "in-progress" | "inprogress" | "doing" | "wip" => State::InProgress,
            "in-review" | "inreview" | "review" => State::InReview,
            "done" | "complete" | "completed" | "closed" => State::Done,
            "cancelled" | "canceled" => State::Cancelled,
            "duplicate" | "duplicated" | "dup" => State::Duplicate,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            State::Backlog => "backlog",
            State::Todo => "todo",
            State::InProgress => "in-progress",
            State::InReview => "in-review",
            State::Done => "done",
            State::Cancelled => "cancelled",
            State::Duplicate => "duplicate",
        }
    }

    pub fn is_open(self) -> bool {
        matches!(
            self,
            State::Backlog | State::Todo | State::InProgress | State::InReview
        )
    }

    /// Default list order: in-progress → in-review → todo → backlog → closed.
    pub fn sort_rank(self) -> u8 {
        match self {
            State::InProgress => 0,
            State::InReview => 1,
            State::Todo => 2,
            State::Backlog => 3,
            State::Done | State::Cancelled | State::Duplicate => 4,
        }
    }
}

// ---------------------------------------------------------------- Priority

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Priority {
    #[cfg_attr(
        feature = "clap",
        value(alias = "p0", alias = "u", alias = "critical", alias = "crit")
    )]
    Urgent,
    #[cfg_attr(feature = "clap", value(alias = "p1", alias = "h", alias = "hi"))]
    High,
    #[cfg_attr(
        feature = "clap",
        value(alias = "p2", alias = "m", alias = "med", alias = "normal")
    )]
    Medium,
    #[cfg_attr(
        feature = "clap",
        value(alias = "p3", alias = "l", alias = "lo", alias = "minor")
    )]
    Low,
}

impl Priority {
    /// Case-insensitive parse of the canonical names plus aliases
    /// (`p0`–`p3`, `u`/`h`/`m`/`l`, …).
    pub fn parse(s: &str) -> Option<Priority> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "urgent" | "p0" | "u" | "critical" | "crit" => Priority::Urgent,
            "high" | "p1" | "h" | "hi" => Priority::High,
            "medium" | "p2" | "m" | "med" | "normal" => Priority::Medium,
            "low" | "p3" | "l" | "lo" | "minor" => Priority::Low,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Urgent => "urgent",
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        }
    }

    /// urgent → low → none
    pub fn rank(p: Option<Priority>) -> u8 {
        p.map(|p| p as u8).unwrap_or(4)
    }
}

// ---------------------------------------------------------------- Lists

/// Labels: split on commas, trimmed, lowercase, deduplicated, order kept.
pub fn normalize_labels<'a>(inputs: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in inputs {
        for l in raw.split(',') {
            let l = l.trim().to_lowercase();
            if !l.is_empty() && !out.contains(&l) {
                out.push(l);
            }
        }
    }
    out
}

/// Split a comma-separated header, respecting quotes and angle brackets.
pub fn split_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quote, mut angle) = (false, false);
    for c in s.chars() {
        match c {
            '"' => quote = !quote,
            '<' if !quote => angle = true,
            '>' if !quote => angle = false,
            ',' if !quote && !angle => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

// ---------------------------------------------------------------- Addresses

/// Lowercased email part of `Name <email>` or `email`.
pub fn email_of(addr: &str) -> String {
    let a = addr.trim();
    let inner = match (a.rfind('<'), a.rfind('>')) {
        (Some(s), Some(e)) if s < e => &a[s + 1..e],
        _ => a,
    };
    inner.trim().to_lowercase()
}

pub fn validate_address(addr: &str) -> Result<String> {
    let a = mbox::clean_value(addr);
    if !email_of(&a).contains('@') || split_list(&a).len() != 1 {
        return Err(Error::validation("invalid-address").with("Value", a).with(
            "Valid",
            "\"Name <user@host>\" or user@host (must contain @, no implicit domain)",
        ));
    }
    Ok(a)
}

// ---------------------------------------------------------------- IDs

const CROCKFORD: &str = "0123456789abcdefghjkmnpqrstvwxyz";

pub fn new_ulid() -> Ulid {
    Ulid::generate()
}

pub fn message_id(u: Ulid) -> String {
    format!("<{u}@lastissue>")
}

/// Normalize user input for an ID: lowercase, Crockford aliases (i/l → 1, o → 0).
pub fn normalize_id_input(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'i' | 'l' => '1',
            'o' => '0',
            c => c,
        })
        .collect()
}

pub fn is_short_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| CROCKFORD.contains(c))
}

// ---------------------------------------------------------------- Messages

pub fn now() -> DateTime<Local> {
    Local::now()
}

pub fn now_header() -> String {
    now().to_rfc2822()
}

/// Identity of whoever runs the command.
#[derive(Debug, Clone)]
pub struct Actor {
    pub from: String,
    pub sender: Option<String>,
}

pub fn base_message(actor: &Actor, subject: &str) -> Msg {
    let mut m = Msg::default();
    m.set("Message-ID", &message_id(new_ulid()));
    m.set("Date", &now_header());
    m.set("From", &actor.from);
    m.set_opt("Sender", actor.sender.as_deref());
    m.set("Subject", subject);
    m
}

pub fn set_content_headers(m: &mut Msg) {
    m.set("Content-Type", mbox::CONTENT_TYPE);
    m.set("Content-Transfer-Encoding", mbox::CTE);
}

pub fn reply_subject(title: &str) -> String {
    format!("Re: {title}")
}

/// A message replying to the issue (or to a comment, extending References).
fn reply(actor: &Actor, issue: &Msg, parent: Option<&Msg>) -> Msg {
    let mut m = base_message(actor, &reply_subject(issue.get("Subject").unwrap_or("")));
    if let Some(p) = parent {
        m.set("In-Reply-To", p.message_id());
        let refs = p.get("References").unwrap_or(issue.message_id());
        m.set("References", &format!("{refs} {}", p.message_id()));
    } else {
        m.set("In-Reply-To", issue.message_id());
        m.set("References", issue.message_id());
    }
    m
}

pub fn comment(actor: &Actor, issue: &Msg, parent: Option<&Msg>, body: &str) -> Msg {
    let mut m = reply(actor, issue, parent);
    set_content_headers(&mut m);
    m.body = mbox::normalize_body(body);
    m
}

pub fn event(actor: &Actor, issue: &Msg, kind: &str, lines: &[String]) -> Msg {
    let mut m = reply(actor, issue, None);
    m.set("X-Issue-Event", kind);
    set_content_headers(&mut m);
    m.body = lines.join("\n");
    m
}

pub fn is_event(m: &Msg) -> bool {
    m.has("X-Issue-Event")
}

/// Mark a body edit: X-Edited always, X-Edited-By when the editor
/// is not the author.
pub fn mark_edited(m: &mut Msg, actor: &Actor) {
    m.set("X-Edited", &now_header());
    if email_of(m.get("From").unwrap_or("")) == email_of(&actor.from) {
        m.remove("X-Edited-By");
    } else {
        m.set("X-Edited-By", &actor.from);
    }
}

/// Friendly names for event lines.
fn field_label(header: &str) -> String {
    let l = match header.to_ascii_lowercase().as_str() {
        "subject" => "title",
        "x-issue-state" => "state",
        "x-priority-level" => "priority",
        "to" => "assignee",
        "cc" => "watchers",
        "keywords" => "labels",
        "reply-by" => "due",
        "expires" => "expires",
        "list-id" => "project",
        "references" => "duplicate-of",
        "x-parent" => "parent",
        "x-blocked-by" => "blocked-by",
        "x-related" => "related",
        "sensitivity" => "sensitivity",
        "comments" => "comments",
        _ => return header.to_string(),
    };
    l.to_string()
}

const NOT_DIFFED: &[&str] = &[
    "message-id",
    "date",
    "from",
    "sender",
    "content-type",
    "content-transfer-encoding",
    "x-edited",
    "x-edited-by",
];

/// Placeholder for an absent value in event lines.
pub const NONE: &str = "(none)";

/// One changed field of message #1, as an event line shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Friendly field name (`state`, `assignee`, or the header name).
    pub field: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl Change {
    /// `state: todo → in-progress`.
    pub fn line(&self) -> String {
        format!(
            "{}: {} → {}",
            self.field,
            self.from.as_deref().unwrap_or(NONE),
            self.to.as_deref().unwrap_or(NONE)
        )
    }
}

/// Every changed header of message #1, in canonical header order.
pub fn diff(before: &Msg, after: &Msg) -> Vec<Change> {
    let mut names: Vec<String> = Vec::new();
    for (k, _) in before
        .ordered_headers()
        .into_iter()
        .chain(after.ordered_headers())
    {
        if NOT_DIFFED.contains(&k.to_ascii_lowercase().as_str()) {
            continue;
        }
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&k)) {
            names.push(k);
        }
    }
    names.sort_by_key(|n| mbox::rank(n));
    names
        .iter()
        .filter_map(|n| {
            let (a, b) = (before.get(n), after.get(n));
            (a != b).then(|| Change {
                field: field_label(n),
                from: a.map(str::to_string),
                to: b.map(str::to_string),
            })
        })
        .collect()
}

/// One line per changed header of message #1: `state: todo → in-progress`.
pub fn diff_lines(before: &Msg, after: &Msg) -> Vec<String> {
    diff(before, after).iter().map(Change::line).collect()
}

// ---------------------------------------------------------------- Custom headers

/// Headers managed by dedicated flags (or computed); `--header` refuses them.
const RESERVED: &[(&str, &str)] = &[
    ("message-id", "(immutable)"),
    ("date", "(immutable)"),
    ("from", "--from at creation"),
    ("sender", "--sender at creation"),
    ("to", "--assign / --unassign"),
    ("cc", "--watch / --unwatch"),
    ("subject", "--title"),
    ("x-issue-state", "--state, issue close, issue reopen"),
    ("x-priority-level", "--priority / --no-priority"),
    ("keywords", "--label, --add-label / --remove-label"),
    ("reply-by", "--due / --no-due"),
    ("expires", "--expires"),
    ("list-id", "issue move"),
    ("in-reply-to", "(managed)"),
    ("references", "issue close --duplicate-of"),
    ("x-parent", "--parent / --no-parent"),
    ("x-blocked-by", "--blocked-by / --unblock"),
    ("x-related", "--relate / --unrelate"),
    ("x-issue-event", "(managed)"),
    ("x-edited", "(managed)"),
    ("x-edited-by", "(managed)"),
    ("content-type", "(managed)"),
    ("content-transfer-encoding", "(managed)"),
    ("x-short-id", "(computed, never stored)"),
    ("x-path", "(computed, never stored)"),
    ("x-comment-count", "(computed, never stored)"),
    ("x-updated", "(computed, never stored)"),
    ("x-root", "(computed, never stored)"),
    ("x-truncated", "(computed, never stored)"),
    ("x-comment", "(computed, never stored)"),
    ("x-replying-to", "(computed, never stored)"),
];

/// Parse `K=V`. Allowed keys: `X-*` (non-reserved), `Comments`, `Sensitivity`.
/// An empty value removes the header.
pub fn parse_custom_header(kv: &str) -> Result<(String, String)> {
    let (k, v) = kv.split_once('=').ok_or_else(|| {
        Error::usage("invalid-header")
            .with("Value", kv)
            .with("Valid", "K=V, e.g. --header X-Estimate=3")
    })?;
    let k = k.trim();
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Error::validation("invalid-header")
            .with("Value", kv)
            .with("Valid", "header names use A-Z, a-z, 0-9 and -"));
    }
    if let Some((_, flag)) = RESERVED.iter().find(|(r, _)| r.eq_ignore_ascii_case(k)) {
        return Err(Error::validation("reserved-header")
            .with("Header", k)
            .with("Use", *flag));
    }
    let lower = k.to_ascii_lowercase();
    if !(lower.starts_with("x-") || lower == "comments" || lower == "sensitivity") {
        return Err(Error::validation("invalid-header")
            .with("Header", k)
            .with("Valid", "X-* (custom), Comments, Sensitivity"));
    }
    Ok((mbox::canonical_name(k), mbox::clean_value(v)))
}

// ---------------------------------------------------------------- Head accessors

/// `X-Issue-State` of message #1 (missing or unknown: backlog).
pub fn state_of(m: &Msg) -> State {
    m.get("X-Issue-State")
        .and_then(State::parse)
        .unwrap_or(State::Backlog)
}

pub fn priority_of(m: &Msg) -> Option<Priority> {
    m.get("X-Priority-Level").and_then(Priority::parse)
}

/// `Keywords`, normalized.
pub fn labels_of(m: &Msg) -> Vec<String> {
    m.get("Keywords")
        .map(|k| normalize_labels([k]))
        .unwrap_or_default()
}

/// A comma-separated header as a list (`Cc`, `X-Blocked-By`, …).
pub fn list_of(m: &Msg, header: &str) -> Vec<String> {
    m.get(header).map(split_list).unwrap_or_default()
}

/// Set (or with an empty list, remove) a comma-separated header.
pub fn set_list(m: &mut Msg, header: &str, items: &[String]) {
    m.set(header, &items.join(", "));
}

/// 1-based comment numbers: message index → n (message #1 and events are
/// `None`).
pub fn comment_numbers(msgs: &[Msg]) -> Vec<Option<usize>> {
    let mut n = 0;
    msgs.iter()
        .enumerate()
        .map(|(i, m)| {
            if i == 0 || is_event(m) {
                None
            } else {
                n += 1;
                Some(n)
            }
        })
        .collect()
}

/// One entry of an issue's activity timeline: each message
/// after #1, in file order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity<'a> {
    Comment {
        /// Message index in the file.
        index: usize,
        /// 1-based comment number (`/n`).
        number: usize,
        /// The comment number this replies to, if it replies to a comment.
        reply_to: Option<usize>,
        msg: &'a Msg,
    },
    Event {
        index: usize,
        /// `X-Issue-Event` (`update`, `move`, `comment-deleted`, …).
        kind: &'a str,
        msg: &'a Msg,
    },
}

impl<'a> Activity<'a> {
    pub fn msg(&self) -> &'a Msg {
        match self {
            Activity::Comment { msg, .. } | Activity::Event { msg, .. } => msg,
        }
    }
}

pub fn activity(msgs: &[Msg]) -> Vec<Activity<'_>> {
    let numbers = comment_numbers(msgs);
    let number_of = |mid: &str| {
        msgs.iter()
            .zip(&numbers)
            .find(|(m, _)| m.message_id() == mid)
            .and_then(|(_, n)| *n)
    };
    msgs.iter()
        .zip(&numbers)
        .enumerate()
        .skip(1)
        .map(|(index, (msg, n))| match (msg.get("X-Issue-Event"), n) {
            (Some(kind), _) => Activity::Event { index, kind, msg },
            (None, n) => Activity::Comment {
                index,
                number: n.unwrap_or(0),
                reply_to: msg.get("In-Reply-To").and_then(number_of),
                msg,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_normalization() {
        for s in [
            "in progress",
            "in_progress",
            "In Progress",
            "IN-PROGRESS",
            "in-progress",
        ] {
            assert_eq!(State::parse(s), Some(State::InProgress), "{s}");
        }
        assert_eq!(State::parse("duplicated"), Some(State::Duplicate));
        assert_eq!(State::parse("nope"), None);
    }

    #[test]
    fn labels() {
        assert_eq!(
            normalize_labels(["Bug, auth", " bug", "UI,,"]),
            ["bug", "auth", "ui"]
        );
    }

    #[test]
    fn addresses() {
        assert_eq!(
            split_list("A <a@x>, \"Doe, J\" <j@x>,b@y"),
            ["A <a@x>", "\"Doe, J\" <j@x>", "b@y"]
        );
        assert_eq!(email_of("Ron <Ron@Example.com>"), "ron@example.com");
        assert!(validate_address("ron").is_err());
        assert!(validate_address("a@x, b@y").is_err());
    }

    #[test]
    fn diff() {
        let mut a = Msg::default();
        a.set("X-Issue-State", "todo");
        let mut b = a.clone();
        b.set("X-Issue-State", "in-progress");
        b.set("To", "claude@agent");
        assert_eq!(
            diff_lines(&a, &b),
            [
                "assignee: (none) → claude@agent",
                "state: todo → in-progress"
            ]
        );
    }

    #[test]
    fn activity_numbers_comments_and_replies() {
        let a = Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        };
        let mut head = base_message(&a, "T");
        head.set("Message-ID", "<1@x>");
        let mut c1 = comment(&a, &head, None, "one");
        c1.set("Message-ID", "<2@x>");
        let ev = event(&a, &head, "update", &["state: a → b".into()]);
        let c2 = comment(&a, &head, Some(&c1), "two");
        let msgs = [head, c1, ev, c2];
        let act = activity(&msgs);
        assert_eq!(act.len(), 3);
        assert!(matches!(
            act[0],
            Activity::Comment {
                index: 1,
                number: 1,
                reply_to: None,
                ..
            }
        ));
        assert!(matches!(
            act[1],
            Activity::Event {
                index: 2,
                kind: "update",
                ..
            }
        ));
        assert!(matches!(
            act[2],
            Activity::Comment {
                number: 2,
                reply_to: Some(1),
                ..
            }
        ));
        assert_eq!(act[2].msg().body.trim(), "two");
    }
}
