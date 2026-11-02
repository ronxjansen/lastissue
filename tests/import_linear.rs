//! `lastissue import linear` against a mock Linear API (`mockito`), the
//! binary pointed at it through the hidden `LINEAR_API_URL` env var.
//! Message-IDs are deterministic, so snapshots need few filters.

use assert_cmd::Command;
use mockito::{Matcher, Mock, Server, ServerGuard};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    server: ServerGuard,
}

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

fn r(id: &str, created: &str) -> Value {
    json!({ "id": id, "createdAt": created, "project": { "id": "proj-1" }, "team": { "id": "team-1" } })
}

/// A Linear issue node as the API returns it (nested connections complete).
fn issue(n: u32, title: &str) -> Value {
    json!({
        "id": format!("uuid-{n}"), "identifier": format!("ENG-{n}"),
        "url": format!("https://linear.app/acme/issue/ENG-{n}"),
        "title": title, "description": format!("Description of {title}."),
        "createdAt": format!("2026-10-0{n}T10:00:00.000Z"), "priority": n % 5, "dueDate": null,
        "creator": { "name": "Ron Jansen", "email": "ron@acme.com" },
        "assignee": null,
        "state": { "name": "Todo", "type": "unstarted" },
        "project": { "id": "proj-1", "name": "Billing Revamp" }, "team": { "id": "team-1" },
        "parent": null,
        "labels": { "nodes": [{ "name": "Bug" }], "pageInfo": { "hasNextPage": false } },
        "subscribers": { "nodes": [], "pageInfo": { "hasNextPage": false } },
        "attachments": { "nodes": [], "pageInfo": { "hasNextPage": false } },
        "relations": { "nodes": [], "pageInfo": { "hasNextPage": false } },
        "inverseRelations": { "nodes": [], "pageInfo": { "hasNextPage": false } },
        "comments": { "nodes": [], "pageInfo": { "hasNextPage": false } }
    })
}

fn comment(id: &str, body: &str, created: &str) -> Value {
    json!({ "id": id, "body": body, "createdAt": created, "editedAt": null,
            "user": { "name": "Ann", "email": "ann@acme.com" }, "parent": null })
}

fn page1() -> Vec<Value> {
    let mut a = issue(1, "Invoices round wrong");
    a["comments"]["nodes"] = json!([comment(
        "c1",
        "Seen on EUR too.",
        "2026-10-05T09:00:00.000Z"
    )]);
    a["inverseRelations"]["nodes"] =
        json!([{ "type": "blocks", "issue": r("uuid-2", "2026-10-02T10:00:00.000Z") }]);
    let mut b = issue(2, "Switch to integer cents");
    b["state"] = json!({ "name": "In Review", "type": "started" });
    b["assignee"] = json!({ "name": "Ann", "email": "ann@acme.com" });
    vec![a, b]
}

fn page2() -> Vec<Value> {
    let mut c = issue(3, "Rounding bug again");
    c["state"] = json!({ "name": "Duplicate", "type": "canceled" });
    c["relations"]["nodes"] =
        json!([{ "type": "duplicate", "relatedIssue": r("uuid-1", "2026-10-01T10:00:00.000Z") }]);
    vec![c]
}

fn issues_body(nodes: Vec<Value>, next: Option<&str>) -> String {
    json!({ "data": { "issues": { "nodes": nodes,
        "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next } } } })
    .to_string()
}

impl Env {
    fn new() -> Self {
        let env = Env {
            dir: TempDir::new().unwrap(),
            server: Server::new(),
        };
        fs::write(env.root().join(".lastissue"), "").unwrap();
        env
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// viewer + projects; the issue pages are mocked per test.
    fn base_mocks(&mut self) -> Vec<Mock> {
        vec![
            self.server
                .mock("POST", "/")
                .match_body(Matcher::Regex("viewer".into()))
                .with_body(r#"{"data":{"viewer":{"name":"Ron Jansen","email":"ron@acme.com","organization":{"name":"Acme"}}}}"#)
                .expect_at_least(1)
                .create(),
            self.server
                .mock("POST", "/")
                .match_body(Matcher::Regex(r"projects\(".into()))
                .with_body(json!({ "data": { "projects": { "nodes": [
                    { "id": "proj-1", "name": "Billing Revamp", "slugId": "a1b2c3", "state": "started" },
                    { "id": "proj-2", "name": "Old Stuff", "slugId": "d4e5f6", "state": "completed" }
                ], "pageInfo": { "hasNextPage": false, "endCursor": null } } } }).to_string())
                .expect_at_least(1)
                .create(),
        ]
    }

    fn page(&mut self, after: Option<&str>, status: usize, body: String) -> Mock {
        let after = match after {
            Some(a) => format!(r#""after":"{a}""#),
            None => r#""after":null"#.into(),
        };
        self.server
            .mock("POST", "/")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(r"issues\(first: 25".into()),
                Matcher::Regex(regex_escape(&after)),
            ]))
            .with_status(status)
            .with_body(body)
            .create()
    }

    fn pages(&mut self, p1: Vec<Value>, p2: Vec<Value>) -> Vec<Mock> {
        vec![
            self.page(None, 200, issues_body(p1, Some("c1"))),
            self.page(Some("c1"), 200, issues_body(p2, None)),
        ]
    }

    fn run(&self, extra: &[&str]) -> Out {
        let mut cmd = Command::cargo_bin("lastissue").unwrap();
        cmd.current_dir(self.root())
            .env("LASTISSUE_DIR", self.root())
            .env("LASTISSUE_FROM", "Ron <ron@example.com>")
            .env("LINEAR_API_URL", self.server.url())
            .env("LINEAR_API_KEY", "lin_api_test")
            .env("NO_COLOR", "1")
            .args(["import", "linear", "--project", "billing revamp", "--yes"])
            .args(extra)
            .write_stdin("");
        let o = cmd.output().unwrap();
        Out {
            stdout: String::from_utf8(o.stdout).unwrap(),
            stderr: String::from_utf8(o.stderr).unwrap(),
            code: o.status.code().unwrap(),
        }
    }

    fn ok(&self, extra: &[&str]) -> String {
        let o = self.run(extra);
        assert_eq!(o.code, 0, "{}", o.stderr);
        o.stdout
    }

    fn snap(&self, name: &str, text: &str) {
        let mut s = insta::Settings::clone_current();
        s.add_filter(&regex_escape(&self.root().display().to_string()), "[ROOT]");
        s.add_filter(
            r"billing-revamp-[0-9a-hjkmnp-tv-z]{6,8}",
            "billing-revamp-[P]",
        );
        s.add_filter(r"<[0-9a-hjkmnp-tv-z]{6,8}\.lastissue>", "<[P].lastissue>");
        // Messages written now (local comment, update event) have fresh IDs/dates.
        s.add_filter(
            r"(?m)^Date: .*\n(From: Ron <ron@example)",
            "Date: [NOW]\n$1",
        );
        s.add_filter(
            r"(?m)^Message-ID: <\w+@lastissue>\n(Date: \[NOW\])",
            "Message-ID: [NEW]\n$1",
        );
        s.add_filter(r"(?m)^X-Edited: .*", "X-Edited: [NOW]");
        s.add_filter(
            r"(?m)^From \w+@lastissue (.*)\n(Message-ID: \[NEW\])",
            "From [NEW]\n$2",
        );
        s.bind(|| insta::assert_snapshot!(name, text));
    }

    fn files(&self) -> Vec<PathBuf> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            for e in fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "mbox") {
                    out.push(p);
                }
            }
        }
        let mut out = Vec::new();
        walk(self.root(), &mut out);
        out.sort();
        out
    }

    fn file(&self, ident: &str) -> String {
        self.files()
            .into_iter()
            .map(|p| fs::read_to_string(p).unwrap())
            .find(|t| t.contains(&format!("X-Linear-Id: {ident}\n")))
            .expect("imported file")
    }
}

fn regex_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if "\\.+*?()|[]{}^$".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

#[test]
fn fresh_import() {
    let mut env = Env::new();
    let _m = env.base_mocks();
    let _p = env.pages(page1(), page2());
    let out = env.run(&[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stderr.contains("Authenticated as Ron Jansen @ Acme"));
    assert!(out.stderr.contains("Page 2: 1 issues"));
    env.snap("fresh_summary", &out.stdout);
    env.snap("fresh_eng1", &env.file("ENG-1"));
    env.snap("fresh_eng3", &env.file("ENG-3"));
    assert_eq!(env.files().len(), 3);
    // No import events on fresh create.
    assert!(!env.file("ENG-1").contains("X-Issue-Event"));
}

#[test]
fn rerun_skip() {
    let mut env = Env::new();
    let _m = env.base_mocks();
    let _p = env.pages(page1(), page2());
    env.ok(&[]);
    let before: Vec<String> = env
        .files()
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    let out = env.ok(&[]);
    assert!(
        out.contains("X-Imported: 0\nX-Updated: 0\nX-Skipped: 3\n"),
        "{out}"
    );
    let after: Vec<String> = env
        .files()
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    assert_eq!(before, after);
    // A re-run reuses the "new" project folder instead of creating another.
    let dirs = fs::read_dir(env.root())
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().is_dir())
        .count();
    assert_eq!(dirs, 1);
}

#[test]
fn rerun_update() {
    let mut env = Env::new();
    let _m = env.base_mocks();
    let p = env.pages(page1(), page2());
    env.ok(&[]);
    // Local-only additions must survive.
    let path = env
        .files()
        .into_iter()
        .find(|p| {
            fs::read_to_string(p)
                .unwrap()
                .contains("X-Linear-Id: ENG-1\n")
        })
        .unwrap();
    let text = fs::read_to_string(&path).unwrap().replacen(
        "X-Linear-Id: ENG-1\n",
        "X-Linear-Id: ENG-1\nX-Estimate: 3\n",
        1,
    );
    fs::write(&path, text).unwrap();
    let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
    let short = stem.rsplit('-').next().unwrap();
    let o = Command::cargo_bin("lastissue")
        .unwrap()
        .env("LASTISSUE_DIR", env.root())
        .env("LASTISSUE_FROM", "Ron <ron@example.com>")
        .args(["comment", "create", short, "--body", "Local note."])
        .write_stdin("")
        .output()
        .unwrap();
    assert!(o.status.success());
    drop(p);

    let mut p1 = page1();
    p1[0]["title"] = json!("Invoices round wrong (EUR)");
    p1[0]["state"] = json!({ "name": "In Progress", "type": "started" });
    p1[0]["comments"]["nodes"] = json!([
        comment("c1", "Seen on EUR and GBP.", "2026-10-05T09:00:00.000Z"),
        comment("c2", "Fix is up.", "2026-10-06T09:00:00.000Z"),
    ]);
    let _p = env.pages(p1, page2());
    let out = env.ok(&["--existing", "update"]);
    assert!(
        out.contains("X-Imported: 0\nX-Updated: 1\nX-Skipped: 2\n"),
        "{out}"
    );
    let eng1 = env.file("ENG-1");
    assert!(eng1.contains("X-Estimate: 3\n"));
    env.snap("update_eng1", &eng1);
}

#[test]
fn server_error_then_resume() {
    let mut env = Env::new();
    let _m = env.base_mocks();
    let p1 = env.page(None, 200, issues_body(page1(), Some("c1")));
    let bad = env.page(Some("c1"), 500, r#"{"errors":[{"message":"boom"}]}"#.into());
    let out = env.run(&[]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Error: linear-api\n"), "{}", out.stderr);
    assert!(out.stderr.contains("Imported: 2\n"), "{}", out.stderr);
    assert!(out.stderr.contains("--existing skip"), "{}", out.stderr);
    assert_eq!(env.files().len(), 2);
    drop((p1, bad));

    let _p = env.pages(page1(), page2());
    let out = env.ok(&["--existing", "skip"]);
    assert!(
        out.contains("X-Imported: 1\nX-Updated: 0\nX-Skipped: 2\n"),
        "{out}"
    );
    assert_eq!(env.files().len(), 3);
}

#[test]
fn rate_limit_retry() {
    let mut env = Env::new();
    let _m = env.base_mocks();
    // Matching mocks that still miss hits win, in creation order: the 429 first.
    let limited = env
        .server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#"issues\(first: 25.*"after":null"#.into()))
        .with_status(429)
        .with_header("Retry-After", "0")
        .with_body(
            r#"{"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}"#,
        )
        .expect(1)
        .create();
    let _p = env.pages(page1(), page2());
    let out = env.run(&[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stderr
            .contains("Rate limited by Linear; retrying in 0s")
    );
    assert!(out.stdout.contains("X-Imported: 3\n"));
    limited.assert();
}

#[test]
fn non_tty_missing_values_fail_fast() {
    let env = Env::new();
    let mut cmd = Command::cargo_bin("lastissue").unwrap();
    let o = cmd
        .env("LASTISSUE_DIR", env.root())
        .env_remove("LINEAR_API_KEY")
        .args(["import", "linear", "--project", "x"])
        .write_stdin("")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(
        err.contains("Error: missing-value\nFlag: --api-key or LINEAR_API_KEY\n"),
        "{err}"
    );
}

#[test]
fn non_tty_requires_yes() {
    let mut env = Env::new();
    let _m = env.base_mocks();
    let mut cmd = Command::cargo_bin("lastissue").unwrap();
    let o = cmd
        .env("LASTISSUE_DIR", env.root())
        .env("LASTISSUE_FROM", "Ron <ron@example.com>")
        .env("LINEAR_API_URL", env.server.url())
        .env("LINEAR_API_KEY", "k")
        .args(["import", "linear", "--project", "a1b2c3"])
        .write_stdin("")
        .output()
        .unwrap();
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(err.contains("Flag: --yes\n"), "{err}");
}

#[test]
fn unknown_project_lists_valid() {
    let mut env = Env::new();
    let _m = env.base_mocks();
    let mut cmd = Command::cargo_bin("lastissue").unwrap();
    let o = cmd
        .env("LASTISSUE_DIR", env.root())
        .env("LINEAR_API_URL", env.server.url())
        .env("LINEAR_API_KEY", "k")
        .args(["import", "linear", "--project", "nope", "--yes"])
        .write_stdin("")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(err.contains("Valid: Billing Revamp, Old Stuff\n"), "{err}");
}

/// Live smoke test: `LINEAR_API_KEY=… LINEAR_SMOKE_TEAM=ENG cargo test -- --ignored`.
#[test]
#[ignore]
fn live_smoke() {
    let (Ok(key), Ok(team)) = (
        std::env::var("LINEAR_API_KEY"),
        std::env::var("LINEAR_SMOKE_TEAM"),
    ) else {
        return;
    };
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".lastissue"), "").unwrap();
    let o = Command::cargo_bin("lastissue")
        .unwrap()
        .env("LASTISSUE_DIR", dir.path())
        .env("LASTISSUE_FROM", "Smoke <smoke@example.com>")
        .env("LINEAR_API_KEY", key)
        .env_remove("LINEAR_API_URL")
        .args(["import", "linear", "--team", &team, "--yes"])
        .write_stdin("")
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
