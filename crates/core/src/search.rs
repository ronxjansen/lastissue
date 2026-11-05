//! Full-text search (SPEC §6.4): a byte-level prefilter over the raw file,
//! then line matching over the parsed issue for the match context (§6.3).

use crate::error::{Error, Result};
use crate::model::{self, comment_numbers};
use crate::store::mbox::Msg;
use regex::{Regex, RegexBuilder, bytes};

/// Snippet width in characters.
const WIDTH: usize = 120;
/// X-Match lines per issue (raised to the number of terms if that is larger).
const SHOWN: usize = 3;

/// The --search terms, compiled. Every term is case-insensitive; in literal
/// mode it is escaped and run through the same engine.
#[derive(Debug, Clone)]
pub struct Terms {
    lines: Vec<Regex>,
    raw: Vec<bytes::Regex>,
}

/// Where an issue matched: X-Match values plus X-Match-Count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matches {
    pub shown: Vec<String>,
    pub count: usize,
}

struct LineHit<'a> {
    loc: String,
    line: &'a str,
    span: (usize, usize),
}

impl Terms {
    pub fn new(terms: &[String], regex: bool) -> Result<Self> {
        let mut lines = Vec::new();
        let mut raw = Vec::new();
        for t in terms {
            let pat = if regex { t.clone() } else { regex::escape(t) };
            let bad = |e: regex::Error| {
                Error::validation("invalid-regex")
                    .with("Search", t.as_str())
                    .with("Message", e.to_string().replace('\n', " "))
            };
            lines.push(
                RegexBuilder::new(&pat)
                    .case_insensitive(true)
                    .build()
                    .map_err(bad)?,
            );
            raw.push(
                bytes::RegexBuilder::new(&pat)
                    .case_insensitive(true)
                    .multi_line(true)
                    .build()
                    .map_err(bad)?,
            );
        }
        Ok(Terms { lines, raw })
    }

    /// Every term occurs somewhere in the raw file. A superset of a real
    /// match (headers count here), so only files that pass get parsed.
    pub fn prefilter(&self, file: &[u8]) -> bool {
        self.raw.iter().all(|re| re.is_match(file))
    }

    /// Match every term against message #1's Subject and the bodies of
    /// non-event messages, line by line. None unless every term matched.
    pub fn matches(&self, msgs: &[Msg]) -> Option<Matches> {
        let nums = comment_numbers(msgs);
        let mut texts: Vec<(String, &str)> = Vec::new();
        if let Some(s) = msgs.first().and_then(|m| m.get("Subject")) {
            texts.push(("subject".into(), s));
        }
        for (i, m) in msgs.iter().enumerate() {
            if model::is_event(m) {
                continue;
            }
            let loc = match nums[i] {
                Some(n) => format!("/{n}"),
                None if i == 0 => "body".into(),
                None => continue,
            };
            texts.push((loc, &m.body));
        }

        let mut hits: Vec<LineHit> = Vec::new();
        // Per term: index of its first matching line.
        let mut first: Vec<Option<usize>> = vec![None; self.lines.len()];
        for (loc, text) in &texts {
            for line in text.lines() {
                let mut span: Option<(usize, usize)> = None;
                for (t, re) in self.lines.iter().enumerate() {
                    if let Some(m) = re.find(line) {
                        first[t].get_or_insert(hits.len());
                        if span.is_none_or(|s| m.start() < s.0) {
                            span = Some((m.start(), m.end()));
                        }
                    }
                }
                if let Some(span) = span {
                    hits.push(LineHit {
                        loc: loc.clone(),
                        line,
                        span,
                    });
                }
            }
        }
        if first.iter().any(|f| f.is_none()) {
            return None;
        }

        // One line per term first, then fill up in document order.
        let cap = SHOWN.max(self.lines.len());
        let mut pick: Vec<usize> = first.into_iter().flatten().collect();
        pick.sort_unstable();
        pick.dedup();
        for i in 0..hits.len() {
            if pick.len() >= cap {
                break;
            }
            if !pick.contains(&i) {
                pick.push(i);
            }
        }
        pick.sort_unstable();
        let shown = pick
            .into_iter()
            .map(|i| {
                let h = &hits[i];
                format!("{}: {}", h.loc, snippet(h.line, h.span))
            })
            .collect();
        Some(Matches {
            shown,
            count: hits.len(),
        })
    }
}

/// About WIDTH characters of `line` centered on the hit, on one line, with
/// `…` where it was cut.
fn snippet(line: &str, (start, end): (usize, usize)) -> String {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let n = chars.len();
    let (from, to) = if n <= WIDTH {
        (0, n)
    } else {
        let cs = chars.partition_point(|(i, _)| *i < start);
        let ce = chars.partition_point(|(i, _)| *i < end);
        let to = (cs.midpoint(ce) + WIDTH / 2).clamp(WIDTH, n);
        (to - WIDTH, to)
    };
    let text: String = chars[from..to]
        .iter()
        .map(|&(_, c)| if c.is_control() { ' ' } else { c })
        .collect();
    format!(
        "{}{}{}",
        if from > 0 { "…" } else { "" },
        text.trim(),
        if to < n { "…" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(headers: &[(&str, &str)], body: &str) -> Msg {
        Msg {
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.to_string(),
        }
    }

    fn terms(t: &[&str], regex: bool) -> Terms {
        Terms::new(&t.iter().map(|s| s.to_string()).collect::<Vec<_>>(), regex).unwrap()
    }

    fn issue() -> Vec<Msg> {
        vec![
            msg(
                &[("Subject", "Login crashes on empty password")],
                "Steps:\n1. leave the password empty\n2. submit",
            ),
            msg(&[("In-Reply-To", "<x>")], "Reproduced on main."),
            msg(&[("X-Issue-Event", "update")], "state: password → done"),
            msg(
                &[("In-Reply-To", "<x>")],
                "so the empty password path hits unwrap() in auth.rs:41",
            ),
        ]
    }

    #[test]
    fn locations_and_count() {
        let m = terms(&["PASSWORD"], false).matches(&issue()).unwrap();
        assert_eq!(m.count, 3, "events are not searched");
        assert_eq!(
            m.shown,
            [
                "subject: Login crashes on empty password",
                "body: 1. leave the password empty",
                "/2: so the empty password path hits unwrap() in auth.rs:41",
            ]
        );
    }

    #[test]
    fn terms_are_anded_and_each_shown() {
        let t = terms(&["password", "reproduced"], false);
        let m = t.matches(&issue()).unwrap();
        assert_eq!(m.count, 4);
        assert!(m.shown.contains(&"/1: Reproduced on main.".to_string()));
        assert_eq!(m.shown.len(), 3);
        assert!(
            terms(&["password", "nowhere"], false)
                .matches(&issue())
                .is_none()
        );
    }

    #[test]
    fn literal_vs_regex() {
        assert!(terms(&["unwrap()"], false).matches(&issue()).is_some());
        assert!(terms(&["unwrap()"], true).matches(&issue()).is_some());
        assert!(terms(&[r"auth\.rs:\d+"], true).matches(&issue()).is_some());
        assert!(terms(&[r"auth\.rs:\d+"], false).matches(&issue()).is_none());
        assert!(Terms::new(&["(".into()], true).is_err());
        assert!(
            terms(&["state: password"], false)
                .matches(&issue())
                .is_none()
        );
    }

    #[test]
    fn prefilter_is_case_insensitive() {
        let t = terms(&["zygote", "ünï"], false);
        assert!(t.prefilter("x\nZYGOTE y\nÜNÏ".as_bytes()));
        assert!(!t.prefilter(b"zygote only"));
    }

    #[test]
    fn snippets_center_on_the_hit() {
        let line = format!("{}NEEDLE{}", "a".repeat(300), "b".repeat(300));
        let s = snippet(&line, (300, 306));
        assert!(s.starts_with('…') && s.ends_with('…'));
        assert!(s.contains("NEEDLE"));
        assert_eq!(s.chars().count(), WIDTH + 2);
        let s = snippet(&line, (0, 1));
        assert!(!s.starts_with('…') && s.ends_with('…'));
        assert_eq!(snippet("  short\tline ", (2, 7)), "short line");
    }
}
