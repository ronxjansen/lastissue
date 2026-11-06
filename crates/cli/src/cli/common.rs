//! Shared command context: root, identity (core `identity`, plus the
//! no-identity warning), output buffer, body input.

use super::BodyArgs;
use crate::error::{Error, Result};
use crate::format;
use crate::model;
use crate::store::{IssueRef, Store, mbox::Msg};
use lastissue::identity::Identity;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Ctx {
    pub store: Store,
    pub color: bool,
    pub me: Identity,
    warned: bool,
    pub out: String,
}

impl Ctx {
    pub fn new(root: PathBuf, from: Option<String>, sender: Option<String>) -> Self {
        Ctx {
            store: Store::new(root),
            color: format::color_enabled(),
            me: Identity::new(from, sender),
            warned: false,
            out: String::new(),
        }
    }

    /// The $USER@host fallback is announced on stderr, once.
    pub fn warn_identity(&mut self) {
        if self.warned || !self.me.used_fallback() {
            return;
        }
        self.warned = true;
        let addr = self.me.resolved().map(|r| r.address).unwrap_or_default();
        eprint!(
            "{}",
            format::block(&[
                ("Warning", "no-identity"),
                ("Using", &addr),
                (
                    "Hint",
                    "set --from, LASTISSUE_FROM or git config user.email"
                ),
            ])
        );
    }

    pub fn print(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// Print `text` coloured (see [`format::paint`]).
    pub fn paint(&mut self, text: &str, blocks: bool) {
        self.out.push_str(&format::paint(text, self.color, blocks));
    }

    /// Computed X-Short-Id and X-Path for an issue.
    pub fn ident(&self, r: &IssueRef) -> [(&'static str, String); 2] {
        [
            ("X-Short-Id", r.id.clone()),
            ("X-Path", self.store.rel(&r.path)),
        ]
    }

    /// Print the messages at `idx` as written (mbox), with computed headers
    /// on message #1.
    pub fn print_indices(&mut self, r: &IssueRef, msgs: &[Msg], idx: &[usize]) {
        let ident = self.ident(r);
        let text: String = idx
            .iter()
            .map(|&i| format::message_with(&msgs[i], if i == 0 { &ident } else { &[] }))
            .collect();
        self.paint(&text, false);
    }
}

// ---------------------------------------------------------------- body input

/// stdin is a pipe or a redirected file (not a TTY, not /dev/null): only
/// then is it read implicitly, so we never block waiting on a terminal.
fn stdin_is_piped() -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        use std::os::unix::fs::FileTypeExt;
        let Ok(fd) = std::io::stdin().as_fd().try_clone_to_owned() else {
            return false;
        };
        let Ok(meta) = std::fs::File::from(fd).metadata() else {
            return false;
        };
        let ft = meta.file_type();
        ft.is_fifo() || ft.is_file()
    }
    #[cfg(not(unix))]
    {
        !std::io::stdin().is_terminal()
    }
}

fn read_stdin() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s)
}

fn read_file(p: &Path) -> Result<String> {
    if p == Path::new("-") {
        return read_stdin();
    }
    std::fs::read_to_string(p).map_err(|e| {
        Error::not_found("file-not-found")
            .with("Path", p.display().to_string())
            .with("Message", e.to_string())
    })
}

/// Explicitly given body, if any (--body, --body-file, "-").
pub fn explicit_body(b: &BodyArgs) -> Result<Option<String>> {
    if let Some(s) = &b.body {
        return Ok(Some(s.clone()));
    }
    if let Some(p) = &b.body_file {
        return read_file(p).map(Some);
    }
    if b.stdin.is_some() {
        return read_stdin().map(Some);
    }
    Ok(None)
}

/// Explicit body, else piped stdin.
pub fn given_body(b: &BodyArgs) -> Result<Option<String>> {
    match explicit_body(b)? {
        None if stdin_is_piped() => read_stdin().map(Some),
        body => Ok(body),
    }
}

/// Body for commands where it's required: explicit, piped stdin, $EDITOR on
/// a TTY, otherwise fail fast.
pub fn required_body(b: &BodyArgs, initial: &str) -> Result<String> {
    let body = match given_body(b)? {
        Some(s) => s,
        None if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => {
            editor(initial)?
        }
        None => {
            return Err(Error::usage("missing-body").with(
                "Hint",
                "pass --body TEXT, --body-file PATH, or - with the body on stdin",
            ));
        }
    };
    if body.trim().is_empty() {
        return Err(Error::validation("empty-body").with("Hint", "the body must not be empty"));
    }
    Ok(body)
}

fn editor(initial: &str) -> Result<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let path = std::env::temp_dir().join(format!("lastissue-{}.md", model::new_ulid()));
    std::fs::write(&path, initial)?;
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status();
    let text = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(s) if s.success() => Ok(text?),
        res => Err(Error::usage("editor-failed").with("Editor", editor).with(
            "Message",
            res.map_or_else(|e| e.to_string(), |s| s.to_string()),
        )),
    }
}
