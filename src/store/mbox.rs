//! mboxrd reading (via `mail-parser`) and writing (own serializer).
//!
//! Byte-level rules (§2.5): LF only, raw UTF-8 headers, no folding below 998
//! bytes, canonical header order, `^>*From ` body lines get one extra `>`.

use crate::error::{Error, Result};
use chrono::{DateTime, FixedOffset};
use mail_parser::MessageParser;
use mail_parser::mailbox::mbox::MessageIterator;
use std::io::BufRead;

/// Canonical header order. Unknown headers (custom `X-*`) sort after these,
/// in insertion order, followed by [`TAIL_ORDER`].
pub const HEADER_ORDER: &[&str] = &[
    "Message-ID",
    "Date",
    "From",
    "Sender",
    "To",
    "Cc",
    "Subject",
    "X-Issue-State",
    "X-Priority-Level",
    "Keywords",
    "Reply-By",
    "Expires",
    "List-Id",
    "In-Reply-To",
    "References",
    "X-Parent",
    "X-Blocked-By",
    "X-Related",
    "X-Issue-Event",
    "X-Edited",
    "X-Edited-By",
    "Sensitivity",
    "Comments",
];
pub const TAIL_ORDER: &[&str] = &["Content-Type", "Content-Transfer-Encoding"];

pub const CONTENT_TYPE: &str = "text/markdown; charset=utf-8";
pub const CTE: &str = "8bit";

const MAX_LINE: usize = 998;

/// One message: ordered headers plus a body. Header names are compared
/// case-insensitively; values never contain newlines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Msg {
    pub headers: Vec<(String, String)>,
    pub body: String,
}

fn rank(name: &str) -> usize {
    if let Some(i) = HEADER_ORDER
        .iter()
        .position(|h| h.eq_ignore_ascii_case(name))
    {
        return i;
    }
    if let Some(i) = TAIL_ORDER.iter().position(|h| h.eq_ignore_ascii_case(name)) {
        return HEADER_ORDER.len() + 1 + i;
    }
    HEADER_ORDER.len()
}

/// The canonical spelling for a known header, or the name as given.
pub fn canonical_name(name: &str) -> String {
    HEADER_ORDER
        .iter()
        .chain(TAIL_ORDER)
        .find(|h| h.eq_ignore_ascii_case(name))
        .map(|h| h.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Collapse a value onto one line and trim it.
pub fn clean_value(v: &str) -> String {
    v.replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string()
}

/// Bodies are stored LF-only with no trailing whitespace.
pub fn normalize_body(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end()
        .to_string()
}

impl Msg {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Replace the value in place (keeping position), or append.
    pub fn set(&mut self, name: &str, value: &str) {
        let value = clean_value(value);
        if value.is_empty() {
            self.remove(name);
            return;
        }
        if let Some(slot) = self
            .headers
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
        {
            slot.1 = value;
        } else {
            self.headers.push((canonical_name(name), value));
        }
    }

    pub fn set_opt(&mut self, name: &str, value: Option<&str>) {
        match value {
            Some(v) => self.set(name, v),
            None => self.remove(name),
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }

    pub fn message_id(&self) -> &str {
        self.get("Message-ID").unwrap_or("")
    }

    pub fn date(&self) -> Option<DateTime<FixedOffset>> {
        self.get("Date").and_then(parse_date)
    }

    /// Headers in canonical order (stable: unknown headers keep insertion order).
    pub fn ordered_headers(&self) -> Vec<(String, String)> {
        let mut hs = self.headers.clone();
        hs.sort_by_key(|(k, _)| rank(k));
        hs
    }
}

pub fn parse_date(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc2822(s.trim()).ok()
}

/// `<01JB…@lastissue>` → `01JB…`
pub fn ulid_of(message_id: &str) -> &str {
    let s = message_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    s.split('@').next().unwrap_or(s)
}

fn from_line(msg: &Msg) -> String {
    let ulid = ulid_of(msg.message_id()).to_ascii_lowercase();
    let short = &ulid[ulid.len().saturating_sub(6)..];
    let short = if short.is_empty() { "unknown" } else { short };
    let date = msg
        .date()
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap().fixed_offset());
    format!(
        "From {short}@lastissue {}",
        date.format("%a %b %e %H:%M:%S %Y")
    )
}

fn is_from_line(line: &str) -> bool {
    line.trim_start_matches('>').starts_with("From ")
}

/// Fold a header line at existing spaces only when it exceeds 998 bytes.
fn header_line(name: &str, value: &str) -> String {
    let line = format!("{name}: {value}");
    if line.len() <= MAX_LINE {
        return line + "\n";
    }
    let mut out = String::new();
    let mut cur = String::new();
    let mut first = true;
    for (i, seg) in line.split(' ').enumerate() {
        let piece = if i == 0 {
            seg.to_string()
        } else {
            format!(" {seg}")
        };
        if !first && !cur.is_empty() && cur.len() + piece.len() > MAX_LINE && !cur.trim().is_empty()
        {
            out.push_str(&cur);
            out.push('\n');
            cur.clear();
        }
        first = false;
        cur.push_str(&piece);
    }
    out.push_str(&cur);
    out.push('\n');
    out
}

/// Serialize one message (separator line, headers, body, trailing blank line).
pub fn serialize_msg(msg: &Msg) -> String {
    let mut out = from_line(msg);
    out.push('\n');
    for (k, v) in msg.ordered_headers() {
        out.push_str(&header_line(&k, &v));
    }
    out.push('\n');
    let body = normalize_body(&msg.body);
    if !body.is_empty() {
        for line in body.split('\n') {
            if is_from_line(line) {
                out.push('>');
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

pub fn serialize(msgs: &[Msg]) -> String {
    msgs.iter().map(serialize_msg).collect()
}

fn unfold(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c == '\r' {
            continue;
        }
        if c == '\n' {
            // Unfolding removes the line break; the following WSP is kept.
            continue;
        }
        out.push(c);
    }
    out
}

/// Parse one message (without its `From ` separator line).
pub fn parse_message(raw: &[u8]) -> Result<Msg> {
    let parsed = MessageParser::new().parse(raw).ok_or_else(|| {
        Error::validation("corrupt-message").with("Message", "could not parse message")
    })?;
    let mut headers = Vec::new();
    for h in parsed.headers() {
        let name = String::from_utf8_lossy(&raw[h.offset_field as usize..h.offset_start as usize]);
        let name = name.trim().trim_end_matches(':').trim().to_string();
        let value = String::from_utf8_lossy(&raw[h.offset_start as usize..h.offset_end as usize]);
        let value = unfold(&value).trim().to_string();
        if name.is_empty() || value.is_empty() {
            continue;
        }
        headers.push((name, value));
    }
    let off = parsed.root_part().offset_body as usize;
    let body = if off <= raw.len() {
        String::from_utf8_lossy(&raw[off..]).into_owned()
    } else {
        String::new()
    };
    Ok(Msg {
        headers,
        body: normalize_body(&body),
    })
}

/// Parse a whole mbox file.
pub fn parse_mbox(bytes: &[u8]) -> Result<Vec<Msg>> {
    let mut msgs = Vec::new();
    for m in MessageIterator::new(bytes) {
        let m = m?;
        msgs.push(parse_message(m.contents())?);
    }
    Ok(msgs)
}

/// Read only message #1's headers: stop at the first blank line after the
/// separator. Used by list filters so large files aren't read in full.
pub fn read_head(reader: impl BufRead) -> Result<Msg> {
    let mut buf = Vec::new();
    let mut seen_from = false;
    for line in reader.split(b'\n') {
        let line = line?;
        if !seen_from {
            if line.starts_with(b"From ") {
                seen_from = true;
            }
            continue;
        }
        if line.is_empty() || line == b"\r" {
            break;
        }
        buf.extend_from_slice(&line);
        buf.push(b'\n');
    }
    buf.push(b'\n');
    parse_message(&buf)
}

/// Cheap line scan of a whole file: (comment count, max Date).
pub fn scan_stats(reader: impl BufRead) -> Result<(usize, Option<DateTime<FixedOffset>>)> {
    let mut in_headers = false;
    let mut index = 0usize;
    let mut is_event = false;
    let mut comments = 0usize;
    let mut updated: Option<DateTime<FixedOffset>> = None;
    let close = |index: usize, is_event: bool, comments: &mut usize| {
        if index > 1 && !is_event {
            *comments += 1;
        }
    };
    for line in reader.split(b'\n') {
        let line = line?;
        if line.starts_with(b"From ") {
            close(index, is_event, &mut comments);
            index += 1;
            in_headers = true;
            is_event = false;
            continue;
        }
        if !in_headers {
            continue;
        }
        if line.is_empty() {
            in_headers = false;
            continue;
        }
        let l = String::from_utf8_lossy(&line);
        if let Some((k, v)) = l.split_once(':') {
            if k.eq_ignore_ascii_case("X-Issue-Event") {
                is_event = true;
            } else if k.eq_ignore_ascii_case("Date")
                && let Some(d) = parse_date(v)
                && updated.is_none_or(|u| d > u)
            {
                updated = Some(d);
            }
        }
    }
    close(index, is_event, &mut comments);
    Ok((comments, updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn msg(headers: &[(&str, &str)], body: &str) -> Msg {
        Msg {
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.to_string(),
        }
    }

    fn base(id: &str) -> Vec<(String, String)> {
        vec![
            ("Message-ID".into(), format!("<{id}@lastissue>")),
            ("Date".into(), "Sat, 31 Oct 2026 22:40:00 +0100".into()),
            ("From".into(), "Ron <ron@example.com>".into()),
        ]
    }

    #[test]
    fn mboxrd_escaping() {
        let m = msg(
            &[
                ("Message-ID", "<01JBXK0000000000000000AAAA@lastissue>"),
                ("Subject", "x"),
            ],
            "From here\n>From there\n>>From everywhere\nFromage",
        );
        let s = serialize_msg(&m);
        assert!(s.contains("\n>From here\n>>From there\n>>>From everywhere\nFromage\n"));
        let back = parse_mbox(s.as_bytes()).unwrap();
        assert_eq!(back[0].body, m.body);
    }

    #[test]
    fn raw_utf8_headers_kept() {
        let m = msg(
            &[
                ("Subject", "Ünïcødé — 日本語 ✓"),
                ("X-Thing", "a"),
                ("X-Thing-2", "b"),
            ],
            "",
        );
        let back = parse_mbox(serialize_msg(&m).as_bytes()).unwrap();
        assert_eq!(back[0].get("Subject"), Some("Ünïcødé — 日本語 ✓"));
        assert!(!serialize_msg(&m).contains("=?"));
    }

    #[test]
    fn canonical_order() {
        let m = msg(
            &[
                ("X-Custom", "1"),
                ("Content-Type", CONTENT_TYPE),
                ("Subject", "s"),
                ("Message-ID", "<A@lastissue>"),
            ],
            "",
        );
        let names: Vec<_> = m.ordered_headers().into_iter().map(|(k, _)| k).collect();
        assert_eq!(names, ["Message-ID", "Subject", "X-Custom", "Content-Type"]);
    }

    #[test]
    fn head_and_stats() {
        let id = "01JBXK0000000000000000AAAA";
        let mut a = Msg {
            headers: base(id),
            body: "body\nFrom x".into(),
        };
        a.set("Subject", "t");
        let mut c = Msg {
            headers: base("01JBXK0000000000000000BBBB"),
            body: "c".into(),
        };
        c.set("Date", "Sun, 01 Nov 2026 10:00:00 +0100");
        let mut e = Msg {
            headers: base("01JBXK0000000000000000CCCC"),
            body: "state: a → b".into(),
        };
        e.set("X-Issue-Event", "update");
        let s = serialize(&[a.clone(), c, e]);
        let head = read_head(s.as_bytes()).unwrap();
        assert_eq!(head.headers, a.headers);
        let (count, upd) = scan_stats(s.as_bytes()).unwrap();
        assert_eq!(count, 1);
        assert_eq!(upd.unwrap().to_rfc2822(), "Sun, 1 Nov 2026 10:00:00 +0100");
    }

    fn header_value() -> impl Strategy<Value = String> {
        // Any printable text including unicode and `From ` lookalikes, single line.
        "[^\\r\\n\\x00-\\x1f\\x7f]{1,60}"
            .prop_map(|s| clean_value(&s))
            .prop_filter("non-empty", |s| !s.is_empty())
    }

    fn body() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("From the start".to_string()),
                Just(">From quoted".to_string()),
                Just(">>>From deep".to_string()),
                Just(String::new()),
                "[^\\r\\n\\x00-\\x08\\x0b-\\x1f]{0,80}",
            ],
            0..12,
        )
        .prop_map(|lines| normalize_body(&lines.join("\n")))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]
        #[test]
        fn round_trip(
            subjects in prop::collection::vec(header_value(), 1..4),
            long in prop::option::of(prop::collection::vec("[a-zé]{1,12}", 100..250)),
            customs in prop::collection::vec(("[A-Za-z0-9]{1,10}", header_value()), 0..20),
            bodies in prop::collection::vec(body(), 1..5),
        ) {
            let mut msgs = Vec::new();
            for (i, b) in bodies.iter().enumerate() {
                let mut m = Msg { headers: base(&format!("01JBXK00000000000000000{i:03}")), body: b.clone() };
                m.set("Subject", &subjects[i % subjects.len()]);
                if i == 0 {
                    if let Some(words) = &long { m.set("Comments", &words.join(" ")); }
                    let mut seen = std::collections::HashSet::new();
                    for (k, v) in &customs {
                        let k = format!("X-{k}");
                        if seen.insert(k.to_ascii_lowercase()) { m.set(&k, v); }
                    }
                }
                m.set("Content-Type", CONTENT_TYPE);
                m.set("Content-Transfer-Encoding", CTE);
                m.headers = m.ordered_headers();
                msgs.push(m);
            }
            let text = serialize(&msgs);
            for line in text.lines() { prop_assert!(line.len() <= MAX_LINE + 20 || !line.contains(' ')); }
            let back = parse_mbox(text.as_bytes()).unwrap();
            prop_assert_eq!(&back, &msgs);
            prop_assert_eq!(serialize(&back), text);
        }
    }
}
