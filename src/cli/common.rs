//! Shared command context: root, identity (§4), body input (§6.2).

use super::BodyArgs;
use crate::dates;
use crate::error::{Error, Result};
use crate::format;
use crate::model::{self, Actor};
use crate::store::{IssueRef, Store, mbox::Msg};
use std::cell::OnceCell;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Ctx {
    pub store: Store,
    pub color: bool,
    from_flag: Option<String>,
    sender_flag: Option<String>,
    actor: OnceCell<Actor>,
    out: String,
}

impl Ctx {
    pub fn new(root: PathBuf, from: Option<String>, sender: Option<String>) -> Self {
        Ctx {
            store: Store::new(root),
            color: format::color_enabled(),
            from_flag: from.filter(|s| !s.trim().is_empty()),
            sender_flag: sender.filter(|s| !s.trim().is_empty()),
            actor: OnceCell::new(),
            out: String::new(),
        }
    }

    /// Resolve the acting identity once (From: chain + Sender:).
    pub fn actor(&self) -> Result<&Actor> {
        if let Some(a) = self.actor.get() {
            return Ok(a);
        }
        let from = resolve_from(self.from_flag.as_deref())?;
        let sender = self
            .sender_flag
            .as_deref()
            .map(model::validate_address)
            .transpose()?;
        Ok(self.actor.get_or_init(|| Actor { from, sender }))
    }

    /// `me` → own From:, otherwise a validated address.
    pub fn who(&self, who: &str) -> Result<String> {
        if who.trim().eq_ignore_ascii_case("me") {
            Ok(self.actor()?.from.clone())
        } else {
            model::validate_address(who)
        }
    }

    /// For filters: `me` → own email; otherwise the email part (or raw text).
    pub fn who_pattern(&self, who: &str) -> Result<String> {
        if who.trim().eq_ignore_ascii_case("me") {
            Ok(model::email_of(&self.actor()?.from))
        } else if who.contains('<') {
            Ok(model::email_of(who))
        } else {
            Ok(who.trim().to_lowercase())
        }
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

    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    /// Resolve an issue reference to its Message-ID.
    pub fn issue_message_id(&self, id: &str) -> Result<String> {
        let r = self.store.resolve_issue(id)?;
        Ok(self.store.read_head(&r)?.message_id().to_string())
    }

    pub fn date(&self, input: &str) -> Result<String> {
        Ok(dates::to_header(dates::parse(input, model::now())?))
    }
}

fn git_config(key: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// §4: --from / LASTISSUE_FROM (both via clap), git config, $USER@host.
pub fn resolve_from(flag: Option<&str>) -> Result<String> {
    if let Some(f) = flag {
        return model::validate_address(f);
    }
    if let Some(email) = git_config("user.email") {
        let addr = match git_config("user.name") {
            Some(name) => format!("{name} <{email}>"),
            None => email,
        };
        if let Ok(a) = model::validate_address(&addr) {
            return Ok(a);
        }
    }
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let host = Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "localhost".into());
    let addr = format!("{user}@{host}");
    eprint!(
        "{}",
        format::block(&[
            ("Warning".into(), "no-identity".into()),
            ("Using".into(), addr.clone()),
            (
                "Hint".into(),
                "set --from, LASTISSUE_FROM or git config user.email".into()
            ),
        ])
    );
    Ok(addr)
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
