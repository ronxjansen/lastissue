//! Linear issue JSON → lastissue messages. Pure: no I/O, so every row of
//! the field-mapping table is unit-tested here.
//!
//! Message-IDs are deterministic ULIDs: timestamp = Linear `createdAt`,
//! random part = a hash of the Linear UUID. Re-runs and relations resolve
//! without lookups.

use super::{Source, SourceKind, str_of};
use crate::model::{self, Priority, State};
use crate::store::mbox::{self, Msg};
use chrono::{DateTime, NaiveDate};
use serde_json::Value;
use ulid::Ulid;

/// `Sender:` on everything the importer writes.
pub const SENDER: &str = "lastissue-import";
/// Used when Linear has no author email (bots, deleted users).
const FALLBACK_FROM: &str = "Linear <noreply@linear.app>";

/// Headers of message #1 that Linear owns on `--existing update`.
pub const OWNED: &[&str] = &[
    "From",
    "To",
    "Cc",
    "Subject",
    "X-Issue-State",
    "X-Priority-Level",
    "Keywords",
    "Reply-By",
    "References",
    "X-Parent",
    "X-Blocked-By",
    "X-Related",
    "X-Linear-Id",
    "X-Linear-Url",
];

pub struct Mapped {
    pub issue: Msg,
    pub comments: Vec<Msg>,
    pub ulid: Ulid,
    pub identifier: String,
}

/// splitmix64 finalizer: full avalanche over 64 bits.
fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

/// Stable 80-bit hash (unlike `DefaultHasher`, fixed across Rust versions):
/// FNV-1a 64, then two splitmix rounds so every input bit reaches every
/// output bit (short IDs are the ULID's tail, so the low bits matter).
fn hash80(s: &str) -> u128 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let lo = mix64(h);
    let hi = mix64(lo ^ 0x9e3779b97f4a7c15);
    (((hi as u128) << 64) | lo as u128) & ((1u128 << 80) - 1)
}

/// Deterministic ULID for a Linear object.
pub fn ulid_for(created_at: &str, uuid: &str) -> Ulid {
    let ms = DateTime::parse_from_rfc3339(created_at)
        .map(|d| d.timestamp_millis().max(0) as u64)
        .unwrap_or(0);
    Ulid::from_parts(ms, hash80(uuid))
}

fn mid(node: &Value) -> String {
    model::message_id(ulid_for(
        node["createdAt"].as_str().unwrap_or(""),
        node["id"].as_str().unwrap_or(""),
    ))
}

fn date(iso: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|d| d.to_rfc2822())
}

/// `Name <email>`, quoting the name when needed; None without an email.
pub fn address(user: &Value) -> Option<String> {
    let email = user["email"].as_str()?.trim();
    if !email.contains('@') {
        return None;
    }
    let name = user["name"].as_str().unwrap_or("").trim();
    if name.is_empty() || name.eq_ignore_ascii_case(email) {
        return Some(email.to_string());
    }
    let name = mbox::clean_value(name);
    if name.chars().any(|c| "\",;<>@()[]:\\".contains(c)) {
        Some(format!(
            "\"{}\" <{email}>",
            name.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    } else {
        Some(format!("{name} <{email}>"))
    }
}

fn nodes(v: &Value) -> &[Value] {
    v["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

pub fn in_scope(issue_ref: &Value, src: &Source) -> bool {
    let field = match src.kind {
        SourceKind::Project => "project",
        SourceKind::Team => "team",
    };
    !src.id.is_empty() && issue_ref[field]["id"].as_str() == Some(src.id.as_str())
}

pub fn state(state: &Value, duplicate: bool) -> State {
    let name = state["name"].as_str().unwrap_or("").to_lowercase();
    match state["type"].as_str().unwrap_or("") {
        "unstarted" => State::Todo,
        "started" if name.contains("review") => State::InReview,
        "started" => State::InProgress,
        "completed" => State::Done,
        "canceled" | "cancelled" if duplicate => State::Duplicate,
        "canceled" | "cancelled" => State::Cancelled,
        _ => State::Backlog, // triage, backlog
    }
}

pub fn priority(p: &Value) -> Option<Priority> {
    match p.as_i64()? {
        1 => Some(Priority::Urgent),
        2 => Some(Priority::High),
        3 => Some(Priority::Medium),
        4 => Some(Priority::Low),
        _ => None,
    }
}

fn due(d: &str) -> Option<String> {
    let day = NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok()?;
    Some(
        day.and_hms_opt(0, 0, 0)?
            .and_utc()
            .fixed_offset()
            .to_rfc2822(),
    )
}

fn body(issue: &Value) -> String {
    let mut b = mbox::normalize_body(issue["description"].as_str().unwrap_or(""));
    let links: Vec<String> = nodes(&issue["attachments"])
        .iter()
        .filter_map(|a| {
            let url = a["url"].as_str()?;
            let title = a["title"]
                .as_str()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(url);
            Some(format!("- [{}]({url})", title.trim()))
        })
        .collect();
    if !links.is_empty() {
        if !b.is_empty() {
            b.push_str("\n\n");
        }
        b.push_str("Attachments:\n\n");
        b.push_str(&links.join("\n"));
    }
    b
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
}

/// Map one fully-fetched Linear issue node.
pub fn issue(node: &Value, src: &Source) -> Mapped {
    let ulid = ulid_for(
        node["createdAt"].as_str().unwrap_or(""),
        node["id"].as_str().unwrap_or(""),
    );
    let issue_mid = model::message_id(ulid);
    let title = mbox::clean_value(node["title"].as_str().unwrap_or(""));
    let title = if title.is_empty() {
        str_of(&node["identifier"])
    } else {
        title
    };

    // Relations, only when the other end is in scope.
    let mut duplicate_of = None;
    let mut related = Vec::new();
    let mut blocked_by = Vec::new();
    for r in nodes(&node["relations"]) {
        let other = &r["relatedIssue"];
        if !in_scope(other, src) {
            continue;
        }
        match r["type"].as_str().unwrap_or("") {
            "duplicate" => duplicate_of = duplicate_of.or(Some(mid(other))),
            "related" => push_unique(&mut related, mid(other)),
            _ => {}
        }
    }
    for r in nodes(&node["inverseRelations"]) {
        let other = &r["issue"];
        if r["type"].as_str() == Some("blocks") && in_scope(other, src) {
            push_unique(&mut blocked_by, mid(other));
        }
    }
    let parent =
        (!node["parent"].is_null() && in_scope(&node["parent"], src)).then(|| mid(&node["parent"]));

    let st = state(&node["state"], duplicate_of.is_some());
    let mut labels: Vec<&str> = nodes(&node["labels"])
        .iter()
        .filter_map(|l| l["name"].as_str())
        .collect();
    if src.kind == SourceKind::Team
        && let Some(p) = node["project"]["name"].as_str()
    {
        labels.push(p);
    }
    let watchers: Vec<String> = {
        let mut w = Vec::new();
        for s in nodes(&node["subscribers"]) {
            if let Some(a) = address(s) {
                push_unique(&mut w, a);
            }
        }
        w
    };

    let mut m = Msg::default();
    m.set("Message-ID", &issue_mid);
    m.set_opt(
        "Date",
        date(node["createdAt"].as_str().unwrap_or("")).as_deref(),
    );
    m.set(
        "From",
        &address(&node["creator"]).unwrap_or_else(|| FALLBACK_FROM.into()),
    );
    m.set("Sender", SENDER);
    m.set_opt("To", address(&node["assignee"]).as_deref());
    m.set_opt("Cc", model::join_list(&watchers).as_deref());
    m.set("Subject", &title);
    m.set("X-Issue-State", st.as_str());
    m.set_opt(
        "X-Priority-Level",
        priority(&node["priority"]).map(|p| p.as_str()),
    );
    m.set_opt(
        "Keywords",
        model::join_list(&model::normalize_labels(labels)).as_deref(),
    );
    m.set_opt(
        "Reply-By",
        node["dueDate"].as_str().and_then(due).as_deref(),
    );
    if st == State::Duplicate {
        m.set_opt("References", duplicate_of.as_deref());
    }
    m.set_opt("X-Parent", parent.as_deref());
    m.set_opt("X-Blocked-By", model::join_list(&blocked_by).as_deref());
    m.set_opt("X-Related", model::join_list(&related).as_deref());
    m.set("X-Linear-Id", node["identifier"].as_str().unwrap_or(""));
    m.set("X-Linear-Url", node["url"].as_str().unwrap_or(""));
    model::set_content_headers(&mut m);
    m.body = body(node);

    Mapped {
        comments: comments(node, &issue_mid, &title),
        issue: m,
        ulid,
        identifier: str_of(&node["identifier"]),
    }
}

fn comment_author(c: &Value) -> String {
    address(&c["user"])
        .or_else(|| address(&c["externalUser"]))
        .or_else(|| {
            let bot = c["botActor"]["name"].as_str()?.trim();
            (!bot.is_empty()).then(|| format!("{} <noreply@linear.app>", mbox::clean_value(bot)))
        })
        .unwrap_or_else(|| FALLBACK_FROM.into())
}

/// Comments in creation order; threads become In-Reply-To / References.
fn comments(node: &Value, issue_mid: &str, title: &str) -> Vec<Msg> {
    let mut cs: Vec<&Value> = nodes(&node["comments"]).iter().collect();
    cs.sort_by(|a, b| {
        let key = |c: &Value| {
            DateTime::parse_from_rfc3339(c["createdAt"].as_str().unwrap_or(""))
                .map(|d| d.timestamp_millis())
                .unwrap_or(0)
        };
        key(a).cmp(&key(b))
    });
    let mut out: Vec<Msg> = Vec::new();
    for c in cs {
        let cmid = mid(c);
        let mut m = Msg::default();
        m.set("Message-ID", &cmid);
        m.set_opt(
            "Date",
            date(c["createdAt"].as_str().unwrap_or("")).as_deref(),
        );
        m.set("From", &comment_author(c));
        m.set("Sender", SENDER);
        m.set("Subject", &model::reply_subject(title));
        let parent_mid = (!c["parent"].is_null()).then(|| mid(&c["parent"]));
        let parent_refs = parent_mid.as_ref().and_then(|p| {
            out.iter()
                .find(|o| o.message_id() == p)
                .and_then(|o| o.get("References").map(str::to_string))
        });
        match parent_mid {
            Some(p) => {
                m.set("In-Reply-To", &p);
                let refs = parent_refs.unwrap_or_else(|| issue_mid.to_string());
                m.set("References", &format!("{refs} {p}"));
            }
            None => {
                m.set("In-Reply-To", issue_mid);
                m.set("References", issue_mid);
            }
        }
        m.set_opt("X-Edited", c["editedAt"].as_str().and_then(date).as_deref());
        m.set("X-Linear-Comment-Id", c["id"].as_str().unwrap_or(""));
        model::set_content_headers(&mut m);
        m.body = mbox::normalize_body(c["body"].as_str().unwrap_or(""));
        out.push(m);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn team() -> Source {
        Source {
            kind: SourceKind::Team,
            id: "team-1".into(),
            key: "ENG".into(),
            name: "Engineering".into(),
            closed: false,
        }
    }

    fn project() -> Source {
        Source {
            kind: SourceKind::Project,
            id: "proj-1".into(),
            ..team()
        }
    }

    fn r(id: &str, created: &str, team: &str) -> Value {
        json!({ "id": id, "createdAt": created, "project": { "id": "proj-1" }, "team": { "id": team } })
    }

    fn full() -> Value {
        json!({
            "id": "uuid-1", "identifier": "ENG-1", "url": "https://linear.app/acme/issue/ENG-1",
            "title": "Login crashes", "description": "Steps:\nFrom here\n",
            "createdAt": "2026-10-01T10:00:00.000Z", "priority": 2, "dueDate": "2026-11-20",
            "creator": { "name": "Ron Jansen", "email": "ron@acme.com" },
            "assignee": { "name": "Doe, J", "email": "j@acme.com" },
            "state": { "name": "In Progress", "type": "started" },
            "project": { "id": "proj-1", "name": "Billing Revamp" }, "team": { "id": "team-1" },
            "parent": r("uuid-p", "2026-09-01T00:00:00.000Z", "team-1"),
            "labels": { "nodes": [{ "name": "Bug" }, { "name": "auth" }] },
            "subscribers": { "nodes": [{ "name": "Ron Jansen", "email": "ron@acme.com" }, { "name": "", "email": "x@acme.com" }] },
            "attachments": { "nodes": [{ "title": "PR #4", "url": "https://github.com/a/b/pull/4" }] },
            "relations": { "nodes": [
                { "type": "related", "relatedIssue": r("uuid-r", "2026-09-02T00:00:00.000Z", "team-1") },
                { "type": "related", "relatedIssue": r("uuid-out", "2026-09-02T00:00:00.000Z", "team-2") }
            ] },
            "inverseRelations": { "nodes": [
                { "type": "blocks", "issue": r("uuid-b", "2026-09-03T00:00:00.000Z", "team-1") }
            ] },
            "comments": { "nodes": [
                { "id": "c2", "body": "reply", "createdAt": "2026-10-02T11:00:00.000Z", "editedAt": null,
                  "user": null, "externalUser": null, "botActor": { "name": "GitHub" },
                  "parent": { "id": "c1", "createdAt": "2026-10-02T10:00:00.000Z" } },
                { "id": "c1", "body": "first", "createdAt": "2026-10-02T10:00:00.000Z",
                  "editedAt": "2026-10-03T10:00:00.000Z",
                  "user": { "name": "Ann", "email": "ann@acme.com" }, "parent": null }
            ] }
        })
    }

    #[test]
    fn deterministic_ids() {
        let a = ulid_for("2026-10-01T10:00:00.000Z", "uuid-1");
        assert_eq!(a, ulid_for("2026-10-01T10:00:00.000Z", "uuid-1"));
        assert_ne!(a, ulid_for("2026-10-01T10:00:00.000Z", "uuid-2"));
        // Near-identical UUIDs must not share an ID tail (short IDs).
        let tails: std::collections::HashSet<String> = (0..1000)
            .map(|i| {
                let u = ulid_for("2026-10-01T10:00:00.000Z", &format!("uuid-{i}")).to_string();
                u[u.len() - 6..].to_string()
            })
            .collect();
        assert!(tails.len() > 990, "{}", tails.len());
        assert_eq!(a.timestamp_ms(), 1_790_848_800_000);
        // Pinned: changing the hash would orphan every earlier import.
        assert_eq!(a.to_string(), "01M3VEG580FBEDH6KWWH38X6PS");
    }

    #[test]
    fn issue_headers() {
        let m = issue(&full(), &team());
        let h = |k| m.issue.get(k).map(str::to_string);
        assert_eq!(h("Message-ID").unwrap(), mid(&full()));
        assert_eq!(h("Date").unwrap(), "Thu, 1 Oct 2026 10:00:00 +0000");
        assert_eq!(h("From").unwrap(), "Ron Jansen <ron@acme.com>");
        assert_eq!(h("Sender").unwrap(), SENDER);
        assert_eq!(h("To").unwrap(), "\"Doe, J\" <j@acme.com>");
        assert_eq!(h("Cc").unwrap(), "Ron Jansen <ron@acme.com>, x@acme.com");
        assert_eq!(h("Subject").unwrap(), "Login crashes");
        assert_eq!(h("X-Issue-State").unwrap(), "in-progress");
        assert_eq!(h("X-Priority-Level").unwrap(), "high");
        // team import: the Linear project name becomes a label
        assert_eq!(h("Keywords").unwrap(), "bug, auth, billing revamp");
        assert_eq!(h("Reply-By").unwrap(), "Fri, 20 Nov 2026 00:00:00 +0000");
        assert_eq!(
            h("X-Parent"),
            Some(mid(&r("uuid-p", "2026-09-01T00:00:00.000Z", "")))
        );
        assert_eq!(
            h("X-Related"),
            Some(mid(&r("uuid-r", "2026-09-02T00:00:00.000Z", "")))
        );
        assert_eq!(
            h("X-Blocked-By"),
            Some(mid(&r("uuid-b", "2026-09-03T00:00:00.000Z", "")))
        );
        assert_eq!(h("References"), None);
        assert_eq!(h("X-Linear-Id").unwrap(), "ENG-1");
        assert_eq!(
            h("X-Linear-Url").unwrap(),
            "https://linear.app/acme/issue/ENG-1"
        );
        assert_eq!(
            m.issue.body,
            "Steps:\nFrom here\n\nAttachments:\n\n- [PR #4](https://github.com/a/b/pull/4)"
        );
    }

    #[test]
    fn project_scope_and_labels() {
        let m = issue(&full(), &project());
        // Project import: no project label; all relation ends share proj-1.
        assert_eq!(m.issue.get("Keywords"), Some("bug, auth"));
        let mut n = full();
        n["parent"]["project"]["id"] = json!("other");
        assert_eq!(issue(&n, &project()).issue.get("X-Parent"), None);
    }

    #[test]
    fn comments_threaded() {
        let m = issue(&full(), &team());
        let [c1, c2] = &m.comments[..] else { panic!() };
        assert_eq!(c1.body, "first");
        assert_eq!(c1.get("From"), Some("Ann <ann@acme.com>"));
        assert_eq!(c1.get("In-Reply-To"), m.issue.get("Message-ID"));
        assert_eq!(c1.get("X-Edited"), Some("Sat, 3 Oct 2026 10:00:00 +0000"));
        assert_eq!(c1.get("X-Linear-Comment-Id"), Some("c1"));
        assert_eq!(c1.get("Subject"), Some("Re: Login crashes"));
        assert_eq!(c2.get("From"), Some("GitHub <noreply@linear.app>"));
        assert_eq!(c2.get("In-Reply-To"), c1.get("Message-ID"));
        assert_eq!(
            c2.get("References").unwrap(),
            format!("{} {}", m.issue.message_id(), c1.message_id())
        );
    }

    #[test]
    fn states() {
        let s = |t: &str, n: &str, dup| state(&json!({ "type": t, "name": n }), dup);
        assert_eq!(s("triage", "Triage", false), State::Backlog);
        assert_eq!(s("backlog", "Backlog", false), State::Backlog);
        assert_eq!(s("unstarted", "Todo", false), State::Todo);
        assert_eq!(s("started", "In Progress", false), State::InProgress);
        assert_eq!(s("started", "In Review", false), State::InReview);
        assert_eq!(s("completed", "Done", false), State::Done);
        assert_eq!(s("canceled", "Canceled", false), State::Cancelled);
        assert_eq!(s("canceled", "Duplicate", true), State::Duplicate);
    }

    #[test]
    fn duplicates() {
        let mut n = full();
        n["state"] = json!({ "name": "Duplicate", "type": "canceled" });
        n["relations"] = json!({ "nodes": [
            { "type": "duplicate", "relatedIssue": r("uuid-o", "2026-09-05T00:00:00.000Z", "team-1") }
        ] });
        let m = issue(&n, &team());
        assert_eq!(m.issue.get("X-Issue-State"), Some("duplicate"));
        assert_eq!(
            m.issue.get("References").unwrap(),
            mid(&r("uuid-o", "2026-09-05T00:00:00.000Z", ""))
        );
        // Original out of scope → plain cancelled, no References.
        n["relations"]["nodes"][0]["relatedIssue"]["team"]["id"] = json!("team-2");
        let m = issue(&n, &team());
        assert_eq!(m.issue.get("X-Issue-State"), Some("cancelled"));
        assert_eq!(m.issue.get("References"), None);
    }

    #[test]
    fn priorities() {
        let p = |v: i64| priority(&json!(v)).map(|p| p.as_str());
        assert_eq!(
            [p(0), p(1), p(2), p(3), p(4)],
            [
                None,
                Some("urgent"),
                Some("high"),
                Some("medium"),
                Some("low")
            ]
        );
    }

    #[test]
    fn missing_people() {
        let mut n = full();
        n["creator"] = Value::Null;
        n["assignee"] = Value::Null;
        n["subscribers"] = json!({ "nodes": [] });
        n["dueDate"] = Value::Null;
        n["priority"] = json!(0);
        let m = issue(&n, &team()).issue;
        assert_eq!(m.get("From"), Some(FALLBACK_FROM));
        for h in ["To", "Cc", "Reply-By", "X-Priority-Level"] {
            assert_eq!(m.get(h), None, "{h}");
        }
    }
}
