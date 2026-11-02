//! Domain types and normalization: states, priorities, labels, addresses,
//! and constructors for issue / comment / event messages.

use crate::error::{Error, Result};
use crate::store::mbox::{self, Msg};
use chrono::{DateTime, Local};
use clap::ValueEnum;
use ulid::Ulid;

// ---------------------------------------------------------------- State

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ValueEnum)]
pub enum State {
    Backlog,
    Todo,
    #[value(
        alias = "in progress",
        alias = "in_progress",
        alias = "inprogress",
        alias = "doing",
        alias = "wip"
    )]
    InProgress,
    #[value(
        alias = "in review",
        alias = "in_review",
        alias = "inreview",
        alias = "review"
    )]
    InReview,
    #[value(alias = "complete", alias = "completed", alias = "closed")]
    Done,
    #[value(alias = "canceled")]
    Cancelled,
    #[value(alias = "duplicated", alias = "dup")]
    Duplicate,
}

impl State {
    /// Lenient parse: case-insensitive, spaces/underscores become dashes.
    pub fn parse(s: &str) -> Option<State> {
        let norm: String = s
            .trim()
            .to_lowercase()
            .split([' ', '_', '-'])
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        <State as ValueEnum>::from_str(&norm, true).ok()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum)]
pub enum Priority {
    #[value(alias = "p0", alias = "u", alias = "critical", alias = "crit")]
    Urgent,
    #[value(alias = "p1", alias = "h", alias = "hi")]
    High,
    #[value(alias = "p2", alias = "m", alias = "med", alias = "normal")]
    Medium,
    #[value(alias = "p3", alias = "l", alias = "lo", alias = "minor")]
    Low,
}

impl Priority {
    pub fn parse(s: &str) -> Option<Priority> {
        <Priority as ValueEnum>::from_str(s.trim(), true).ok()
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

pub fn join_list(items: &[String]) -> Option<String> {
    if items.is_empty() {
        None
    } else {
        Some(items.join(", "))
    }
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

pub const CROCKFORD: &str = "0123456789abcdefghjkmnpqrstvwxyz";

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

pub fn date_header(d: DateTime<Local>) -> String {
    d.to_rfc2822()
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
    m.set("Date", &date_header(now()));
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

/// A comment replying to the issue (or to a comment, extending References).
pub fn comment(actor: &Actor, issue: &Msg, parent: Option<&Msg>, body: &str) -> Msg {
    let mut m = base_message(actor, &reply_subject(issue.get("Subject").unwrap_or("")));
    match parent {
        Some(p) => {
            m.set("In-Reply-To", p.message_id());
            let refs = p.get("References").unwrap_or(issue.message_id());
            m.set("References", &format!("{refs} {}", p.message_id()));
        }
        None => {
            m.set("In-Reply-To", issue.message_id());
            m.set("References", issue.message_id());
        }
    }
    set_content_headers(&mut m);
    m.body = mbox::normalize_body(body);
    m
}

pub fn event(actor: &Actor, issue: &Msg, kind: &str, lines: &[String]) -> Msg {
    let mut m = base_message(actor, &reply_subject(issue.get("Subject").unwrap_or("")));
    m.set("In-Reply-To", issue.message_id());
    m.set("References", issue.message_id());
    m.set("X-Issue-Event", kind);
    set_content_headers(&mut m);
    m.body = lines.join("\n");
    m
}

pub fn is_event(m: &Msg) -> bool {
    m.has("X-Issue-Event")
}

/// Mark a body edit (§5.2): X-Edited always, X-Edited-By when the editor
/// is not the author.
pub fn mark_edited(m: &mut Msg, actor: &Actor) {
    m.set("X-Edited", &date_header(now()));
    if email_of(m.get("From").unwrap_or("")) != email_of(&actor.from) {
        m.set("X-Edited-By", &actor.from);
    } else {
        m.remove("X-Edited-By");
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

/// One line per changed header of message #1: `state: todo → in-progress`.
pub fn diff_lines(before: &Msg, after: &Msg) -> Vec<String> {
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
    let mut probe = Msg {
        headers: names.iter().map(|n| (n.clone(), String::new())).collect(),
        body: String::new(),
    };
    probe.headers = probe.ordered_headers();
    probe
        .headers
        .iter()
        .filter_map(|(n, _)| {
            let (a, b) = (before.get(n), after.get(n));
            (a != b).then(|| {
                format!(
                    "{}: {} → {}",
                    field_label(n),
                    a.unwrap_or("(none)"),
                    b.unwrap_or("(none)")
                )
            })
        })
        .collect()
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
}
