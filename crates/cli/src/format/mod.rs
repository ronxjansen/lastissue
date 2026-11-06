//! Output: mbox / header blocks, `--brief` lines, computed headers, colour.

use crate::model::{Activity, Priority, State, labels_of, priority_of, state_of};
use crate::store::IssueRef;
use crate::store::mbox::{self, Msg};
use std::fmt::Write as _;
use std::io::IsTerminal;

pub fn color_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn state_color(state: State) -> &'static str {
    match state {
        State::InProgress => "\x1b[33m",
        State::InReview => "\x1b[35m",
        State::Todo => "\x1b[36m",
        State::Done => "\x1b[32m",
        State::Backlog | State::Cancelled | State::Duplicate => DIM,
    }
}

fn priority_color(p: Option<Priority>) -> &'static str {
    match p {
        Some(Priority::Urgent) => "\x1b[1;31m",
        Some(Priority::High) => "\x1b[31m",
        Some(Priority::Medium) => "\x1b[33m",
        _ => DIM,
    }
}

/// Colourize header names and separator lines. `blocks` = the text is only
/// header blocks (list output); otherwise it is mbox and bodies are left alone.
pub fn paint(text: &str, color: bool, blocks: bool) -> String {
    if !color {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 256);
    let mut in_headers = true;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        if bare.starts_with("From ") && bare.contains("@lastissue ") {
            let _ = writeln!(out, "{DIM}{bare}{RESET}");
            in_headers = true;
            continue;
        }
        if bare.is_empty() {
            in_headers = blocks;
            out.push_str(line);
            continue;
        }
        if in_headers
            && let Some((k, v)) = bare.split_once(": ")
            && !k.contains(' ')
        {
            let v = if k == "X-Issue-State" {
                format!("{}{v}{RESET}", State::parse(v).map_or(DIM, state_color))
            } else if k == "X-Priority-Level" {
                format!("{}{v}{RESET}", priority_color(Priority::parse(v)))
            } else {
                v.to_string()
            };
            let _ = writeln!(out, "{BOLD}{k}:{RESET} {v}");
            continue;
        }
        out.push_str(line);
    }
    out
}

/// Serialize a message with output-only (computed) headers added. They are
/// placed among the custom headers, just before Content-Type.
pub fn message_with(msg: &Msg, computed: &[(&str, String)]) -> String {
    let mut m = msg.clone();
    for (k, v) in computed {
        m.headers.push((k.to_string(), v.clone()));
    }
    mbox::serialize_msg(&m)
}

/// A bare header block (no separator line, no body).
pub fn block<K: AsRef<str>, V: AsRef<str>>(fields: &[(K, V)]) -> String {
    fields
        .iter()
        .flat_map(|(k, v)| [k.as_ref(), ": ", v.as_ref(), "\n"])
        .collect()
}

/// Computed headers for a comment (X-Comment, X-Replying-To); none for events.
pub fn comment_headers(r: &IssueRef, head: &Msg, a: &Activity) -> Vec<(&'static str, String)> {
    let Activity::Comment {
        number,
        reply_to,
        msg,
        ..
    } = a
    else {
        return Vec::new();
    };
    let mut h = vec![("X-Comment", format!("{}/{number}", r.id))];
    // `reply_to` is None both for a reply to the issue and for a deleted target.
    if msg.get("In-Reply-To") != Some(head.message_id()) {
        h.push((
            "X-Replying-To",
            match reply_to {
                Some(t) => format!("↳ replying to /{t}"),
                None => "↳ replying to (deleted)".into(),
            },
        ));
    }
    h
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max - 1).collect();
        format!("{t}…")
    }
}

/// `7hq2mz  todo  high  bug,auth  Login crashes…`
pub fn brief(r: &IssueRef, head: &Msg, color: bool) -> String {
    let state = state_of(head);
    let prio = priority_of(head);
    let labels = labels_of(head);
    let labels = if labels.is_empty() {
        "-".to_string()
    } else {
        labels.join(",")
    };
    let (sc, pc, reset) = if color {
        (state_color(state), priority_color(prio), RESET)
    } else {
        ("", "", "")
    };
    format!(
        "{:<8}  {sc}{:<11}{reset}  {pc}{:<6}{reset}  {labels}  {}\n",
        r.id,
        state.as_str(),
        prio.map_or("-", Priority::as_str),
        truncate(head.get("Subject").unwrap_or(""), 72)
    )
}
