//! Helpers shared by the CLI test crates (each uses a subset).

use assert_cmd::Command;
use std::path::Path;

/// The binary with the repo registry redirected away from the real one
/// (`init` auto-registers). Tests that inspect the registry override it.
pub fn bin() -> Command {
    let mut cmd = Command::cargo_bin("lastissue").unwrap();
    cmd.env(
        "LASTISSUE_REPOS",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-repos"),
    );
    cmd
}

/// `bin()` in `root` as Ron, without colour or an inherited Sender.
pub fn at(root: &Path) -> Command {
    let mut cmd = bin();
    cmd.current_dir(root)
        .env("LASTISSUE_DIR", root)
        .env("LASTISSUE_FROM", "Ron <ron@example.com>")
        .env_remove("LASTISSUE_SENDER")
        .env("NO_COLOR", "1");
    cmd
}

pub struct Out {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

impl Out {
    pub fn of(cmd: &mut Command) -> Out {
        let o = cmd.output().unwrap();
        Out {
            stdout: String::from_utf8(o.stdout).unwrap(),
            stderr: String::from_utf8(o.stderr).unwrap(),
            code: o.status.code().unwrap(),
        }
    }

    /// Expect success, return stdout.
    pub fn ok(self, args: &[&str]) -> String {
        assert_eq!(self.code, 0, "args {args:?} failed:\n{}", self.stderr);
        self.stdout
    }
}
