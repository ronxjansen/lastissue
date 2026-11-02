//! Shared command context: root, identity (core `identity`, plus the
//! no-identity warning), output buffer, body input (§6.2).

use super::BodyArgs;
use crate::error::{Error, Result};
use crate::format;
use crate::model::{self, Actor};
use crate::store::{IssueRef, Store, mbox::Msg};
use lastissue::identity::Identity;
use std::cell::Cell;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Ctx {
    pub store: Store,
    pub color: bool,
    pub me: Identity,
    warned: Cell<bool>,
    out: String,
}

impl Ctx {
    pub fn new(root: PathBuf, from: Option<String>, sender: Option<String>) -> Self {
        Ctx {
            store: Store::new(root),
            color: format::color_enabled(),
            me: Identity::new(from, sender),
            warned: Cell::new(false),
            out: String::new(),
        }
    }

    /// Resolve the acting identity (From: chain + Sender:), warning once on
    /// stderr if it fell back to $USER@host.
    pub fn actor(&self) -> Result<&Actor> {
        let a = self.me.actor()?;
        self.warn_identity();
        Ok(a)
    }

    /// §4: the $USER@host fallback is announced on stderr, once.
    pub fn warn_identity(&self) {
        if self.warned.get() || !self.me.used_fallback() {
            return;
        }
        self.warned.set(true);
        let addr = self.me.resolved().map(|r| r.address).unwrap_or_default();
        eprint!(
            "{}",
            format::block(&[
                ("Warning".into(), "no-identity".into()),
                ("Using".into(), addr),
                (
                    "Hint".into(),
                    "set --from, LASTISSUE_FROM or git config user.email".into()
                ),
            ])
        );
    }

    pub fn print(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// Print messages as written (mbox), with computed headers on message #1
    /// when it's the issue.
    pub fn print_msgs(&mut self, r: &IssueRef, msgs: &[&Msg]) {
        for m in msgs {
            let text = if model::is_event(m) || m.has("In-Reply-To") {
                format::message_with(m, &[])
            } else {
                format::message_with(
                    m,
                    &[
                        ("X-Short-Id", r.id.clone()),
                        ("X-Path", self.store.rel(&r.path)),
                    ],
                )
            };
            let painted = format::paint(&text, self.color, false);
            self.out.push_str(&painted);
        }
    }

    /// Print the given message indices of `msgs`.
    pub fn print_indices(&mut self, r: &IssueRef, msgs: &[Msg], idx: &[usize]) {
        let refs: Vec<&Msg> = idx.iter().map(|&i| &msgs[i]).collect();
        self.print_msgs(r, &refs);
    }

    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }
}

// ---------------------------------------------------------------- body input

/// stdin is a pipe or a redirected file (not a TTY, not /dev/null): only
/// then is it read implicitly, so we never block waiting on a terminal.
pub fn stdin_is_piped() -> bool {
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
    if b.stdin == Some(true) {
        return read_stdin().map(Some);
    }
    Ok(None)
}

/// Body for commands where it's optional: explicit, else piped stdin, else empty.
pub fn optional_body(b: &BodyArgs) -> Result<String> {
    if let Some(s) = explicit_body(b)? {
        return Ok(s);
    }
    if stdin_is_piped() {
        return read_stdin();
    }
    Ok(String::new())
}

/// Body for commands where it's required: explicit, piped stdin, $EDITOR on
/// a TTY, otherwise fail fast.
pub fn required_body(b: &BodyArgs, initial: &str) -> Result<String> {
    let body = if let Some(s) = explicit_body(b)? {
        s
    } else if stdin_is_piped() {
        read_stdin()?
    } else if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        editor(initial)?
    } else {
        return Err(Error::usage("missing-body").with(
            "Hint",
            "pass --body TEXT, --body-file PATH, or - with the body on stdin",
        ));
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
    std::fs::File::create(&path)?.write_all(initial.as_bytes())?;
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
        _ => Err(Error::usage("editor-failed").with("Editor", editor)),
    }
}
