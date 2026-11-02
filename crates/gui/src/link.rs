//! The `lastissue://` URL scheme (GUI-SPEC §10). Plain parsing, no GPUI.
//!
//! - `lastissue://repo/<label>/issue/<id>`: open the repo with that
//!   registry label and show the issue in the detail pane.
//! - `lastissue://repo/<label>`: just open (switch to) the repo.
//!
//! `<label>` is percent-decoded (labels are folder names, so spaces and
//! non-ASCII arrive encoded). `<id>` is a short ID or a unique prefix of
//! one, normalized like CLI input (case, Crockford `i`/`l`/`o`). A query or
//! fragment and a trailing slash are ignored; the scheme and the `repo`
//! and `issue` words are case-insensitive.

use lastissue::model;

pub const SCHEME: &str = "lastissue";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Registry label.
    pub repo: String,
    /// Short ID (or prefix), normalized.
    pub issue: Option<String>,
}

impl Link {
    /// The URL for this link (labels percent-encoded).
    pub fn to_url(&self) -> String {
        let mut s = format!("{SCHEME}://repo/{}", encode(&self.repo));
        if let Some(id) = &self.issue {
            s.push_str("/issue/");
            s.push_str(id);
        }
        s
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| format!("bad percent-escape in {s:?}"))?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("{s:?} is not UTF-8"))
}

pub fn parse(url: &str) -> Result<Link, String> {
    let url = url.trim();
    let bad = |why: &str| format!("not a lastissue link ({why}): {url}");
    let (scheme, rest) = url.split_once("://").ok_or_else(|| bad("no scheme"))?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return Err(bad("scheme"));
    }
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    let word = |i: usize, w: &str| parts.get(i).is_some_and(|p| p.eq_ignore_ascii_case(w));
    if !word(0, "repo") {
        return Err(bad("expected lastissue://repo/<label>/issue/<id>"));
    }
    let repo = decode(parts.get(1).copied().unwrap_or(""))?;
    if repo.is_empty() {
        return Err(bad("empty repo label"));
    }
    let issue = match parts.len() {
        2 => None,
        4 if word(2, "issue") => {
            let id = model::normalize_id_input(&decode(parts[3])?);
            if !model::is_short_id(&id) {
                return Err(bad("invalid issue ID"));
            }
            Some(id)
        }
        _ => return Err(bad("expected lastissue://repo/<label>/issue/<id>")),
    };
    Ok(Link { repo, issue })
}

/// Whether a command-line argument is a link rather than a root path.
pub fn is_link(arg: &str) -> bool {
    arg.split_once("://")
        .is_some_and(|(s, _)| s.eq_ignore_ascii_case(SCHEME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(repo: &str, issue: Option<&str>) -> Link {
        Link {
            repo: repo.into(),
            issue: issue.map(String::from),
        }
    }

    #[test]
    fn parses_issue_and_repo_links() {
        assert_eq!(
            parse("lastissue://repo/lastissue/issue/8tsrzq").unwrap(),
            link("lastissue", Some("8tsrzq"))
        );
        assert_eq!(
            parse("lastissue://repo/lastissue").unwrap(),
            link("lastissue", None)
        );
        // Case, trailing slash, query and fragment; ID normalized.
        assert_eq!(
            parse("LastIssue://Repo/web/ISSUE/8TSRZQ/?from=slack#x").unwrap(),
            link("web", Some("8tsrzq"))
        );
        assert_eq!(
            parse("lastissue://repo/a/issue/0l1o")
                .unwrap()
                .issue
                .as_deref(),
            Some("0110")
        );
        assert_eq!(
            parse("lastissue://repo/my%20repo%C3%A9/issue/abc").unwrap(),
            link("my repoé", Some("abc"))
        );
    }

    #[test]
    fn rejects_other_shapes() {
        for bad in [
            "",
            "lastissue:repo/x",
            "https://repo/x/issue/abc",
            "lastissue://",
            "lastissue://repo/",
            "lastissue://repo//issue/abc",
            "lastissue://issue/abc",
            "lastissue://repo/x/issue",
            "lastissue://repo/x/issue/",
            "lastissue://repo/x/issues/abc",
            "lastissue://repo/x/issue/not-an-id!",
            "lastissue://repo/x/issue/abc/extra",
            "lastissue://repo/%zz",
            "lastissue://repo/%ff",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn round_trips() {
        for l in [
            link("lastissue", Some("8tsrzq")),
            link("my repo/é", None),
            link("a%b", Some("01")),
        ] {
            assert_eq!(parse(&l.to_url()).unwrap(), l);
        }
    }

    #[test]
    fn tells_links_from_paths() {
        assert!(is_link("lastissue://repo/x"));
        assert!(!is_link("/Users/me/issues"));
        assert!(!is_link("issues"));
    }
}
