//! Identity (SPEC §4, GUI-SPEC §4): who is acting (`From:`) and on whose
//! behalf an agent acts (`Sender:`), plus `me` resolution for WHO arguments.
//!
//! Resolution is lazy: nothing (not even `git config`) runs until an
//! operation actually needs the actor. Core never prints; a front-end that
//! allows the `$USER@host` fallback checks [`Identity::used_fallback`] and
//! warns itself.

use crate::error::{Error, Result};
use crate::model::{self, Actor};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Where the `From:` address came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FromSource {
    /// An explicit address (`--from`, `LASTISSUE_FROM`, a GUI setting).
    Explicit,
    /// `git config user.name` / `user.email`.
    Git,
    /// `$USER@host`: the last resort, which a front-end should warn about.
    Fallback,
}

/// A resolved `From:` address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFrom {
    pub address: String,
    pub source: FromSource,
}

/// How to find the acting identity. Cheap to build; resolves on first use
/// and caches the result.
#[derive(Debug)]
pub struct Identity {
    from: Option<String>,
    sender: Option<String>,
    /// Directory `git config` runs in (`None`: the process's current dir).
    git_dir: Option<PathBuf>,
    allow_fallback: bool,
    resolved: OnceLock<(Actor, FromSource)>,
}

impl Identity {
    /// The CLI chain (SPEC §4): explicit `from`, then git config, then
    /// `$USER@host`. Blank values count as absent.
    pub fn new(from: Option<String>, sender: Option<String>) -> Self {
        Identity {
            from: from.filter(|s| !s.trim().is_empty()),
            sender: sender.filter(|s| !s.trim().is_empty()),
            git_dir: None,
            allow_fallback: true,
            resolved: OnceLock::new(),
        }
    }

    /// An already known actor (GUI settings, tests). Never shells out.
    pub fn fixed(actor: Actor) -> Self {
        let id = Identity::new(None, None);
        let _ = id.resolved.set((actor, FromSource::Explicit));
        id
    }

    /// Run `git config` in `dir` (GUI-SPEC §4: respects per-repo identities).
    pub fn in_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.git_dir = Some(dir.into());
        self
    }

    /// Fail with `no-identity` instead of falling back to `$USER@host`
    /// (the GUI never uses it).
    pub fn without_fallback(mut self) -> Self {
        self.allow_fallback = false;
        self
    }

    /// The acting identity, resolved once.
    pub fn actor(&self) -> Result<&Actor> {
        if let Some((a, _)) = self.resolved.get() {
            return Ok(a);
        }
        let from = match &self.from {
            Some(f) => ResolvedFrom {
                address: model::validate_address(f)?,
                source: FromSource::Explicit,
            },
            None => match from_git(self.git_dir.as_deref()) {
                Some(address) => ResolvedFrom {
                    address,
                    source: FromSource::Git,
                },
                None if self.allow_fallback => ResolvedFrom {
                    address: fallback_address(),
                    source: FromSource::Fallback,
                },
                None => {
                    return Err(Error::validation("no-identity").with(
                        "Hint",
                        "set git config user.email, or an identity in settings",
                    ));
                }
            },
        };
        let sender = self
            .sender
            .as_deref()
            .map(model::validate_address)
            .transpose()?;
        let actor = Actor {
            from: from.address,
            sender,
        };
        Ok(&self.resolved.get_or_init(|| (actor, from.source)).0)
    }

    /// The resolved `From:` and its source, if resolution already happened.
    pub fn resolved(&self) -> Option<ResolvedFrom> {
        self.resolved.get().map(|(a, s)| ResolvedFrom {
            address: a.from.clone(),
            source: *s,
        })
    }

    /// True once the actor resolved to the `$USER@host` fallback.
    pub fn used_fallback(&self) -> bool {
        self.resolved
            .get()
            .is_some_and(|(_, s)| *s == FromSource::Fallback)
    }

    /// A WHO argument for writing: `me` → own `From:`, otherwise a
    /// validated address.
    pub fn who(&self, who: &str) -> Result<String> {
        if is_me(who) {
            Ok(self.actor()?.from.clone())
        } else {
            model::validate_address(who)
        }
    }

    /// A WHO argument for matching: `me` → own email; otherwise the email
    /// part of an address, or the lowercased raw text.
    pub fn who_pattern(&self, who: &str) -> Result<String> {
        if is_me(who) {
            Ok(model::email_of(&self.actor()?.from))
        } else if who.contains('<') {
            Ok(model::email_of(who))
        } else {
            Ok(who.trim().to_lowercase())
        }
    }
}

fn is_me(who: &str) -> bool {
    who.trim().eq_ignore_ascii_case("me")
}

fn git_config(dir: Option<&Path>, key: &str) -> Option<String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd.args(["config", "--get", key]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// `Name <email>` from git config (run in `dir`), if it yields a valid
/// address.
pub fn from_git(dir: Option<&Path>) -> Option<String> {
    let email = git_config(dir, "user.email")?;
    let addr = match git_config(dir, "user.name") {
        Some(name) => format!("{name} <{email}>"),
        None => email,
    };
    model::validate_address(&addr).ok()
}

/// `$USER@host` (SPEC §4's last resort).
pub fn fallback_address() -> String {
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
    format!("{user}@{host}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ada() -> Identity {
        Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        })
    }

    #[test]
    fn explicit_from_wins_and_is_validated() {
        let id = Identity::new(Some("Bob <bob@example.com>".into()), None);
        assert_eq!(id.actor().unwrap().from, "Bob <bob@example.com>");
        assert_eq!(id.resolved().unwrap().source, FromSource::Explicit);
        assert!(!id.used_fallback());
        assert!(
            Identity::new(Some("not an address".into()), None)
                .actor()
                .is_err()
        );
    }

    #[test]
    fn blank_flags_are_absent() {
        let id = Identity::new(Some("  ".into()), Some(String::new()));
        assert!(id.from.is_none() && id.sender.is_none());
    }

    #[test]
    fn resolution_is_lazy() {
        let id = Identity::new(None, None);
        assert!(id.resolved().is_none());
        assert_eq!(
            id.who("Eve <eve@example.com>").unwrap(),
            "Eve <eve@example.com>"
        );
        assert!(id.resolved().is_none(), "a non-`me` WHO needs no identity");
    }

    #[test]
    fn me_resolves_to_own_identity() {
        let id = ada();
        assert_eq!(id.who(" ME ").unwrap(), "Ada <ada@example.com>");
        assert_eq!(id.who_pattern("me").unwrap(), "ada@example.com");
        assert_eq!(id.who_pattern("Bob <BOB@x.org>").unwrap(), "bob@x.org");
        assert_eq!(id.who_pattern(" Bob ").unwrap(), "bob");
    }
}
