//! Synthetic issue stores for the scale benchmarks.
//!
//! Deterministic: the same `n` always yields the same files. Stores are
//! cached under [`base_dir`] (`LASTISSUE_BENCH_DIR`, else cargo's per-target
//! temp dir) and regenerated only when their stamp is missing.
//!
//! Shape, roughly a busy real repo: 25 projects holding ~70% of issues, a
//! weighted state mix (~45% closed), 0–3 labels from 40, an assignee on
//! 60%, a 3–20 line body, 0–6 comments (some replying to comments), 0–3
//! update events, a few `X-Blocked-By` links. Search needles: [`RARE`] in
//! ~0.5% of comments, [`COMMON`] in ~20% of bodies.

#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::range_plus_one
)]

use chrono::{DateTime, Duration, FixedOffset, TimeZone as _};
use lastissue::model::{self, State};
use lastissue::store::{self, mbox, mbox::Msg};
use rayon::prelude::*;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use ulid::Ulid;

/// Bump when the generated shape changes, to invalidate cached stores.
const VERSION: u32 = 1;
pub const RARE: &str = "zanzibar";
pub const COMMON: &str = "quokka";
pub const PROJECTS: usize = 25;

const WORDS: &[&str] = &[
    "login", "crash", "avatar", "upload", "sync", "cache", "token", "refresh", "board", "column",
    "drag", "drop", "search", "index", "render", "layout", "theme", "dark", "mode", "keyboard",
    "shortcut", "palette", "export", "import", "linear", "watcher", "editor", "draft", "save",
    "conflict", "banner", "filter", "label", "priority", "state", "project", "comment", "event",
    "timeline", "scroll", "virtual", "list", "detail", "pane", "sidebar", "repo", "registry",
    "identity", "config", "startup", "slow", "memory", "leak", "panic", "error", "retry",
    "timeout", "network", "offline", "parser", "header", "mbox", "encoding", "unicode",
];
const LABELS: &[&str] = &[
    "bug",
    "feature",
    "chore",
    "docs",
    "perf",
    "ux",
    "a11y",
    "security",
    "regression",
    "flaky",
    "infra",
    "ci",
    "release",
    "design",
    "backend",
    "frontend",
    "cli",
    "gui",
    "core",
    "search",
    "sync",
    "import",
    "export",
    "macos",
    "linux",
    "windows",
    "good-first-issue",
    "help-wanted",
    "blocked",
    "needs-triage",
    "p0",
    "customer",
    "tech-debt",
    "refactor",
    "test",
    "build",
    "deps",
    "api",
    "storage",
    "telemetry",
];
const PEOPLE: &[&str] = &[
    "Ada", "Bob", "Cy", "Dee", "Eve", "Fay", "Gus", "Hal", "Ivy", "Jo", "Kai", "Lu", "Mo", "Ned",
    "Oz", "Pia", "Quin", "Ray", "Sol", "Tia", "Uma", "Vic", "Wes", "Xan", "Yui",
];

/// splitmix64.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.next() % 1000 < pct * 10
    }
    pub fn permille(&mut self, pm: u64) -> bool {
        self.next() % 1000 < pm
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

fn person(i: usize) -> String {
    let p = PEOPLE[i % PEOPLE.len()];
    format!("{p} <{}@example.com>", p.to_lowercase())
}

fn sentence(r: &mut Rng, words: usize) -> String {
    (0..words)
        .map(|_| r.pick(WORDS))
        .collect::<Vec<_>>()
        .join(" ")
}

fn epoch() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(3600)
        .unwrap()
        .with_ymd_and_hms(2024, 1, 1, 9, 0, 0)
        .unwrap()
}

/// Issue `i`'s ULID: 1-minute steps from 2024-01-01; the low 40 bits (the
/// 8-char short ID) are a bijection of `i`, so IDs are unique, all 8 chars
/// long (no prefix collisions), and look random.
fn issue_ulid(i: usize) -> Ulid {
    let ms = epoch().timestamp_millis().cast_unsigned() + i as u64 * 60_000;
    let low = (i as u128).wrapping_mul(0x009E_3779_B97F) & ((1u128 << 40) - 1);
    Ulid::from_parts(ms, (0xA5A5u128 << 64) | low)
}

pub fn short_id(i: usize) -> String {
    let s = issue_ulid(i).to_string().to_lowercase();
    s[s.len() - 8..].to_string()
}

fn msg_id(u: Ulid) -> String {
    model::message_id(u)
}

fn base(from: &str, subject: &str, mid: &str, date: DateTime<FixedOffset>) -> Msg {
    let mut m = Msg::default();
    m.set("Message-ID", mid);
    m.set("Date", &date.to_rfc2822());
    m.set("From", from);
    m.set("Subject", subject);
    m
}

fn state(r: &mut Rng) -> State {
    match r.below(100) {
        0..25 => State::Backlog,
        25..45 => State::Todo,
        45..53 => State::InProgress,
        53..57 => State::InReview,
        57..92 => State::Done,
        _ => State::Cancelled,
    }
}

/// The messages of issue `i`, with `extra` additional comments on top of
/// the random 0–6 (for the activity-feed benchmark).
pub fn issue(i: usize, project: Option<&store::Project>, extra: usize) -> (String, Vec<Msg>) {
    let mut r = Rng::new(i as u64 ^ 0x5EED);
    let u = issue_ulid(i);
    let mid = msg_id(u);
    let created = epoch() + Duration::minutes(i as i64);
    let title = {
        let t = {
            let w = 3 + r.below(5);
            sentence(&mut r, w)
        };
        let mut c = t.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or(t)
    };
    let author = person(r.below(PEOPLE.len()));
    let mut head = base(&author, &title, &mid, created);
    if r.chance(60) {
        head.set("To", &person(r.below(PEOPLE.len())));
    }
    if r.chance(15) {
        head.set("Cc", &person(r.below(PEOPLE.len())));
    }
    head.set("X-Issue-State", state(&mut r).as_str());
    if r.chance(70) {
        head.set(
            "X-Priority-Level",
            ["urgent", "high", "medium", "low"][r.below(4)],
        );
    }
    let nl = r.below(4);
    let labels: Vec<&str> = (0..nl).map(|_| r.pick(LABELS)).collect();
    model::set_list(
        &mut head,
        "Keywords",
        &model::normalize_labels(labels.iter().copied()),
    );
    if r.chance(10) {
        let due = created + Duration::days(r.below(120) as i64);
        head.set("Reply-By", &due.to_rfc2822());
    }
    if let Some(p) = project {
        head.set("List-Id", &p.list_id());
    }
    if i > 10 && r.chance(5) {
        head.set("X-Blocked-By", &msg_id(issue_ulid(i - 1 - r.below(10))));
    }
    if i > 10 && r.chance(5) {
        head.set("X-Parent", &msg_id(issue_ulid(i - 1 - r.below(10))));
    }
    model::set_content_headers(&mut head);
    let mut body: Vec<String> = (0..3 + r.below(18))
        .map(|_| {
            let w = 6 + r.below(8);
            sentence(&mut r, w)
        })
        .collect();
    if r.chance(20) {
        let at = r.below(body.len());
        let _ = write!(body[at], " {COMMON}");
    }
    head.body = body.join("\n");

    let reply = model::reply_subject(&title);
    let mut msgs = vec![head];
    let mut at = created;
    let comments = r.below(7) + extra;
    let events = r.below(4);
    let mut cmids: Vec<String> = Vec::new();
    for k in 0..comments + events {
        at += Duration::minutes(5 + r.below(3000) as i64);
        let mu = Ulid::from_parts(
            at.timestamp_millis().cast_unsigned(),
            (u.random() & !0xFFFF_FFFF) | (k as u128 + 1),
        );
        let cmid = msg_id(mu);
        let who = person(r.below(PEOPLE.len()));
        let mut m = base(&who, &reply, &cmid, at);
        if k < comments {
            // A third reply to an earlier comment.
            if let (false, 0) = (cmids.is_empty(), r.below(3)) {
                let p = cmids[r.below(cmids.len())].clone();
                m.set("In-Reply-To", &p);
                m.set("References", &format!("{mid} {p}"));
            } else {
                m.set("In-Reply-To", &mid);
                m.set("References", &mid);
            }
            model::set_content_headers(&mut m);
            let mut lines: Vec<String> = (0..1 + r.below(6))
                .map(|_| {
                    let w = 5 + r.below(10);
                    sentence(&mut r, w)
                })
                .collect();
            if r.permille(5) {
                let _ = write!(lines[0], " {RARE}");
            }
            m.body = lines.join("\n");
            cmids.push(cmid);
        } else {
            m.set("In-Reply-To", &mid);
            m.set("References", &mid);
            m.set("X-Issue-Event", "update");
            model::set_content_headers(&mut m);
            m.body = format!("state: backlog -> {}", state(&mut r).as_str());
        }
        msgs.push(m);
    }
    let slug = store::slugify(&title, 4, "issue");
    (format!("{slug}-{}.mbox", short_id(i)), msgs)
}

/// Where stores are generated.
pub fn base_dir() -> PathBuf {
    std::env::var_os("LASTISSUE_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lastissue-bench"))
}

fn project_list(root: &Path) -> Vec<store::Project> {
    (0..PROJECTS)
        .map(|p| {
            let s = store::Project {
                // Same ID space as issues, far past any issue index.
                id: short_id(1 << 30 | p),
                slug: format!("project-{}", WORDS[p % WORDS.len()]),
                dir: PathBuf::new(),
            };
            store::Project {
                dir: root.join(s.dir_name()),
                ..s
            }
        })
        .collect()
}

/// A store of `n` issues (generated once, then reused). Returns its root.
pub fn ensure(n: usize) -> PathBuf {
    let root = base_dir().join(format!("store-{n}"));
    let stamp = root.join(format!(".synth-v{VERSION}"));
    if stamp.is_file() {
        return root;
    }
    let _ = std::fs::remove_dir_all(&root);
    let root = store::init(&root).expect("init store");
    let projects = project_list(&root);
    for p in &projects {
        std::fs::create_dir_all(&p.dir).unwrap();
    }
    (0..n).into_par_iter().for_each(|i| {
        let mut r = Rng::new(i as u64 ^ 0xF00D);
        let project = r.chance(70).then(|| &projects[r.below(PROJECTS)]);
        let (name, msgs) = issue(i, project, 0);
        let dir = project.map_or(root.as_path(), |p| p.dir.as_path());
        std::fs::write(dir.join(name), mbox::serialize(&msgs)).unwrap();
    });
    std::fs::write(&stamp, "").unwrap();
    root
}

/// One issue file with `comments` extra comments, as raw bytes.
pub fn big_issue(comments: usize) -> Vec<u8> {
    let (_, msgs) = issue(7, None, comments);
    mbox::serialize(&msgs).into_bytes()
}
