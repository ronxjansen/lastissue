//! CLI snapshot tests against a temp root (§7.3).

use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    ids: Vec<(String, String)>,
}

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

impl Env {
    fn new() -> Self {
        let env = Env {
            dir: TempDir::new().unwrap(),
            ids: Vec::new(),
        };
        let out = env.run(&["init"], None);
        assert_eq!(out.code, 0, "{}", out.stderr);
        env
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn run(&self, args: &[&str], stdin: Option<&str>) -> Out {
        let mut cmd = Command::cargo_bin("lastissue").unwrap();
        cmd.current_dir(self.root())
            .env("LASTISSUE_DIR", self.root())
            .env("LASTISSUE_FROM", "Ron <ron@example.com>")
            .env_remove("LASTISSUE_SENDER")
            .env("NO_COLOR", "1")
            .args(args)
            .write_stdin(stdin.unwrap_or(""));
        let o = cmd.output().unwrap();
        Out {
            stdout: String::from_utf8(o.stdout).unwrap(),
            stderr: String::from_utf8(o.stderr).unwrap(),
            code: o.status.code().unwrap(),
        }
    }

    /// Run, expect success, return stdout.
    fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args, None);
        assert_eq!(o.code, 0, "args {args:?} failed:\n{}", o.stderr);
        o.stdout
    }

    /// Remember an ID under a stable placeholder for snapshots.
    fn name(&mut self, id: &str, label: &str) {
        self.ids.push((id.to_string(), format!("[{label}]")));
    }

    fn snap(&self, name: &str, text: &str) {
        let mut s = insta::Settings::clone_current();
        s.add_filter(&regex_escape(&self.root().display().to_string()), "[ROOT]");
        for (id, label) in &self.ids {
            s.add_filter(id, label.as_str());
        }
        s.add_filter(r"[0-9A-HJKMNP-TV-Z]{26}", "[ULID]");
        s.add_filter(
            r"(?m)^From [0-9a-hjkmnp-tv-z]{6}@lastissue",
            "From [MSG]@lastissue",
        );
        s.add_filter(
            r"@lastissue \w{3} \w{3} [ \d]\d \d\d:\d\d:\d\d \d{4}",
            "@lastissue [ASCTIME]",
        );
        s.add_filter(
            r"\w{3}, \d{1,2} \w{3} \d{4} \d\d:\d\d:\d\d [+-]\d{4}",
            "[DATE]",
        );
        s.bind(|| insta::assert_snapshot!(name, text));
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

/// `X-Short-Id: abc123` from mutation output.
fn short_id(out: &str) -> String {
    out.lines()
        .find_map(|l| l.strip_prefix("X-Short-Id: "))
        .expect("X-Short-Id in output")
        .to_string()
}

fn issue_file(root: &Path, id: &str) -> PathBuf {
    fn walk(dir: &Path, id: &str) -> Option<PathBuf> {
        for e in fs::read_dir(dir).ok()? {
            let p = e.ok()?.path();
            if p.is_dir() {
                if let Some(f) = walk(&p, id) {
                    return Some(f);
                }
            } else if p.to_string_lossy().ends_with(&format!("-{id}.mbox")) {
                return Some(p);
            }
        }
        None
    }
    walk(root, id).expect("issue file")
}

#[test]
fn issue_lifecycle() {
    let mut env = Env::new();
    let p = short_id(&env.ok(&["project", "create", "New feature work"]));
    env.name(&p, "P");

    let out = env.ok(&[
        "issue",
        "create",
        "--title",
        "Login crashes on empty password",
        "--project",
        &p,
        "--label",
        "Bug, auth",
        "--priority",
        "hi",
        "--assign",
        "me",
        "--body",
        "From the start\n>From quoted\nSteps to reproduce.",
    ]);
    let a = short_id(&out);
    env.name(&a, "A");
    env.snap("create", &out);

    // Slug is the first 4 title words; the file is valid mboxrd on disk.
    let file = issue_file(env.root(), &a);
    assert!(file.ends_with(format!("new-feature-{p}/login-crashes-on-empty-{a}.mbox")));
    let raw = fs::read_to_string(&file).unwrap();
    assert!(raw.contains("\n>From the start\n>>From quoted\n"));

    let out = env.run(
        &[
            "--sender",
            "Claude Code <claude@agent>",
            "issue",
            "update",
            &a,
            "--state",
            "In Progress",
            "--assign",
            "claude@agent",
            "--title",
            "Login crashes when password is empty",
        ],
        None,
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    env.snap("update", &out.stdout);
    // Slug is frozen on retitle.
    assert!(issue_file(env.root(), &a).ends_with(format!("login-crashes-on-empty-{a}.mbox")));

    env.ok(&["comment", "create", &a, "--body", "Reproduced on main."]);
    let o = env.run(
        &[
            "comment",
            "create",
            &a,
            "--reply-to",
            &format!("{a}/1"),
            "-",
        ],
        Some("Same here.\n"),
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    env.ok(&[
        "comment",
        "update",
        &format!("{a}/1"),
        "--body",
        "Reproduced on main (debug build).",
    ]);

    env.snap("show", &env.ok(&["issue", "show", &a]));
    env.snap("comment_list", &env.ok(&["comment", "list", &a]));
    env.snap("list", &env.ok(&["issue", "list"]));

    env.ok(&["comment", "delete", &format!("{a}/1")]);
    let show = env.ok(&["issue", "show", &a]);
    assert!(show.contains("X-Issue-Event: comment-deleted"));
    assert!(show.contains("X-Replying-To: ↳ replying to (deleted)"));

    let out = env.ok(&["issue", "close", &a, "--comment", "Fixed."]);
    assert!(out.contains("state: in-progress → done"));
    assert!(
        env.ok(&["issue", "list", "--brief"]).lines().count() == 1,
        "closed issues hidden"
    );
    assert!(env.ok(&["issue", "list", "--brief", "--all"]).contains(&a));

    env.snap("reopen", &env.ok(&["issue", "reopen", &a]));

    let out = env.ok(&["issue", "move", &a, "--no-project"]);
    assert!(out.contains("X-Issue-Event: move"));
    assert!(
        env.root()
            .join(format!("login-crashes-on-empty-{a}.mbox"))
            .is_file()
    );

    env.ok(&["issue", "delete", &a]);
    assert_eq!(env.run(&["issue", "show", &a], None).code, 2);
}

#[test]
fn readiness_and_relations() {
    let mut env = Env::new();
    let blocker = short_id(&env.ok(&["issue", "create", "--title", "Blocker", "--state", "todo"]));
    env.name(&blocker, "B");
    let a = short_id(&env.ok(&[
        "issue",
        "create",
        "--title",
        "Blocked work",
        "--state",
        "todo",
        "--blocked-by",
        &blocker,
        "--priority",
        "urgent",
    ]));
    env.name(&a, "A");
    let child = short_id(&env.ok(&["issue", "create", "--title", "Child", "--parent", &a]));
    env.name(&child, "C");

    env.snap(
        "ready_before",
        &env.ok(&["issue", "list", "--ready", "--brief"]),
    );
    env.ok(&["issue", "close", &blocker]);
    env.snap(
        "ready_after",
        &env.ok(&["issue", "list", "--ready", "--brief"]),
    );

    assert!(
        env.ok(&["issue", "list", "--parent", &a, "--brief"])
            .contains(&child)
    );
    let out = env.ok(&["issue", "update", &a, "--unblock", &blocker]);
    assert!(out.contains("blocked-by: <"));
    assert_eq!(
        env.run(&["issue", "update", &a, "--parent", &a], None).code,
        4
    );
}

#[test]
fn list_filters_sort_and_truncation() {
    let env = Env::new();
    for (t, prio, label) in [
        ("Low thing", "low", "x"),
        ("Urgent thing", "urgent", "y"),
        ("No priority", "", "x"),
    ] {
        let mut args = vec![
            "issue", "create", "--title", t, "--state", "todo", "--label", label,
        ];
        if !prio.is_empty() {
            args.extend(["--priority", prio]);
        }
        env.ok(&args);
    }
    env.ok(&[
        "--sender",
        "Bot <bot@agent>",
        "issue",
        "create",
        "--title",
        "Agent filed",
        "--body",
        "the needle is here",
    ]);

    let titles = |out: String| -> Vec<String> {
        out.lines()
            .filter(|l| !l.starts_with("X-"))
            .map(|l| l.split("  ").last().unwrap().trim().to_string())
            .collect()
    };
    assert_eq!(
        titles(env.ok(&["issue", "list", "--brief"])),
        ["Urgent thing", "Low thing", "No priority", "Agent filed"]
    );
    assert_eq!(
        titles(env.ok(&[
            "issue", "list", "--brief", "--label", "x", "--sort", "priority"
        ])),
        ["Low thing", "No priority"]
    );
    assert_eq!(
        titles(env.ok(&["issue", "list", "--brief", "--agent"])),
        ["Agent filed"]
    );
    assert_eq!(
        titles(env.ok(&["issue", "list", "--brief", "--search", "NEEDLE"])),
        ["Agent filed"]
    );
    assert_eq!(
        titles(env.ok(&["issue", "list", "--brief", "--author", "bot@agent"])),
        Vec::<String>::new()
    );
    let out = env.ok(&["issue", "list", "--brief", "--limit", "2"]);
    assert!(out.ends_with("X-Truncated: 2 of 4\n"), "{out}");
    let out = env.ok(&["issue", "list", "--limit", "0"]);
    assert!(out.starts_with("X-Root: "));
    assert!(!out.contains("X-Truncated"));
}

#[test]
fn errors_and_exit_codes() {
    let env = Env::new();
    let a = short_id(&env.ok(&["issue", "create", "--title", "One"]));

    // Ambiguous prefix: two files whose IDs share a prefix.
    let src = issue_file(env.root(), &a);
    fs::copy(&src, env.root().join("x-7hq2mz.mbox")).unwrap();
    fs::copy(&src, env.root().join("x-7hq9ab.mbox")).unwrap();
    let o = env.run(&["issue", "show", "7hq"], None);
    assert_eq!(o.code, 3);
    insta::assert_snapshot!("ambiguous", o.stderr);
    assert_eq!(env.run(&["issue", "show", "7hq2"], None).code, 0);

    let o = env.run(&["issue", "update", &a, "--state", "bogus"], None);
    assert_eq!(o.code, 4);
    insta::assert_snapshot!("invalid_state", o.stderr);

    let o = env.run(&["issue", "show", "zzzzzz"], None);
    assert_eq!(o.code, 2);
    assert!(o.stderr.starts_with("Error: issue-not-found\n"));

    let o = env.run(&["issue", "frobnicate"], None);
    assert_eq!(o.code, 1);
    assert!(o.stderr.starts_with("Error: usage\n"));

    let o = env.run(&["issue", "update", &a, "--due", "someday"], None);
    assert_eq!(o.code, 4);
    assert!(o.stderr.contains("Valid: YYYY-MM-DD"));

    let o = env.run(
        &["issue", "update", &a, "--header", "X-Issue-State=done"],
        None,
    );
    assert_eq!(o.code, 4);
    assert!(o.stderr.contains("Use: --state"));

    let o = env.run(&["issue", "update", &a, "--state", "duplicated"], None);
    assert_eq!(o.code, 4, "duplicate requires --duplicate-of");

    let o = env.run(&["comment", "create", &a], None);
    assert_eq!(o.code, 4, "empty piped body: {}", o.stderr);

    let o = env.run(
        &["--from", "ron", "issue", "update", &a, "--state", "todo"],
        None,
    );
    assert_eq!(o.code, 4);
    assert!(o.stderr.starts_with("Error: invalid-address\n"));

    let p = short_id(&env.ok(&["project", "create", "Stuff"]));
    env.ok(&["issue", "move", &a, "--project", &p]);
    let o = env.run(&["project", "delete", &p], None);
    assert_eq!(o.code, 4);
    assert!(o.stderr.starts_with("Error: project-not-empty\n"));
    env.ok(&["project", "delete", &p, "--force"]);
}

#[test]
fn projects_rename_keeps_id() {
    let mut env = Env::new();
    let p = short_id(&env.ok(&["project", "create", "Old name here"]));
    env.name(&p, "P");
    let a = short_id(&env.ok(&["issue", "create", "--title", "Thing", "--project", &p]));
    env.ok(&["project", "rename", &p, "Shiny new"]);
    assert!(env.root().join(format!("shiny-new-{p}")).is_dir());
    let show = env.ok(&["issue", "show", &a]);
    assert!(show.contains(&format!("List-Id: shiny new <{p}.lastissue>")));
    env.snap("project_list", &env.ok(&["project", "list"]));
}

#[test]
fn root_discovery() {
    let env = Env::new();
    let sub = env.root().join("a/b");
    fs::create_dir_all(&sub).unwrap();
    let mut cmd = Command::cargo_bin("lastissue").unwrap();
    let o = cmd
        .current_dir(&sub)
        .env_remove("LASTISSUE_DIR")
        .args(["issue", "list", "--brief"])
        .write_stdin("")
        .output()
        .unwrap();
    assert!(o.status.success());
    let out = String::from_utf8(o.stdout).unwrap();
    let canon = |p: &Path| fs::canonicalize(p).unwrap();
    let reported = out
        .lines()
        .next()
        .unwrap()
        .strip_prefix("X-Root: ")
        .unwrap();
    assert_eq!(canon(Path::new(reported)), canon(env.root()));
}

#[test]
fn issues_dir_root() {
    let tmp = TempDir::new().unwrap();
    let canon = |p: &Path| fs::canonicalize(p).unwrap();
    let run = |cwd: &Path, args: &[&str]| {
        let o = Command::cargo_bin("lastissue")
            .unwrap()
            .current_dir(cwd)
            .env_remove("LASTISSUE_DIR")
            .env("LASTISSUE_FROM", "Ron <ron@example.com>")
            .args(args)
            .write_stdin("")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8(o.stdout).unwrap();
        PathBuf::from(out.lines().next().unwrap().strip_prefix("X-Root: ").unwrap())
    };
    let issues = tmp.path().join("issues");
    fs::create_dir(&issues).unwrap();
    // init picks ./issues when it exists
    assert_eq!(canon(&run(tmp.path(), &["init"])), canon(&issues));
    assert!(issues.join(".lastissue").is_file());
    assert!(!tmp.path().join(".lastissue").exists());
    // issues/.lastissue wins over a sibling root marker, also from a subdir
    fs::File::create(tmp.path().join(".lastissue")).unwrap();
    let sub = tmp.path().join("src/deep");
    fs::create_dir_all(&sub).unwrap();
    let got = run(&sub, &["issue", "list", "--brief"]);
    assert_eq!(canon(&got), canon(&issues));
}
