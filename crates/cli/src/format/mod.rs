//! Output: mbox / header blocks, `--brief` lines, error blocks, colour.

use crate::error::Error;
use crate::store::mbox::{self, Msg};
use std::io::IsTerminal;

pub fn color_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn state_color(state: &str) -> &'static str {
    match state {
        "in-progress" => "\x1b[33m",
        "in-review" => "\x1b[35m",
        "todo" => "\x1b[36m",
        "backlog" => "\x1b[2m",
        "done" => "\x1b[32m",
        _ => "\x1b[2m",
    }
}

fn priority_color(p: &str) -> &'static str {
    match p {
        "urgent" => "\x1b[1;31m",
        "high" => "\x1b[31m",
        "medium" => "\x1b[33m",
        _ => "\x1b[2m",
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
            out.push_str(&format!("{DIM}{bare}{RESET}\n"));
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
                format!("{}{v}{RESET}", state_color(v))
            } else if k == "X-Priority-Level" {
                format!("{}{v}{RESET}", priority_color(v))
            } else {
                v.to_string()
            };
            out.push_str(&format!("{BOLD}{k}:{RESET} {v}\n"));
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
pub fn block(fields: &[(String, String)]) -> String {
    let mut s = String::new();
    for (k, v) in fields {
        s.push_str(&format!("{k}: {v}\n"));
    }
    s
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
pub fn brief_line(
    id: &str,
    state: &str,
    priority: Option<&str>,
    labels: &[String],
    subject: &str,
    color: bool,
) -> String {
    let labels = if labels.is_empty() {
        "-".to_string()
    } else {
        labels.join(",")
    };
    let prio = priority.unwrap_or("-");
    let (sc, pc, r) = if color {
        (state_color(state), priority_color(prio), RESET)
    } else {
        ("", "", "")
    };
    format!(
        "{id:<8}  {sc}{state:<11}{r}  {pc}{prio:<6}{r}  {labels}  {}\n",
        truncate(subject, 72)
    )
}

pub fn print_error(e: &Error) {
    eprint!("{e}");
}
