//! Repo registry (§2.7): a plain file of issue-root paths, so a GUI (or the
//! CLI's `--repo`) can pick a root by label. Pointers only, never an index.
//!
//! Format: one canonical absolute path per line, insertion order; `#` comments
//! and blank lines are ignored (and preserved on rewrite). The global root is
//! always listed implicitly as `global` and never written to the file.

use crate::error::{Error, Result};
use crate::store::{self, ISSUES_DIR, MARKER};
use std::fs;
use std::path::{Path, PathBuf};

/// Env var overriding the registry file location.
pub const ENV: &str = "LASTISSUE_REPOS";
/// Label of the implicit global root.
pub const GLOBAL: &str = "global";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub label: String,
    pub root: PathBuf,
    /// The implicit global root (not stored in the file).
    pub global: bool,
}

impl Repo {
    fn at(root: PathBuf) -> Self {
        Repo {
            label: label_of(&root),
            root,
            global: false,
        }
    }

    /// False when the root (or its `.lastissue` marker) no longer exists.
    pub fn exists(&self) -> bool {
        self.root.join(MARKER).is_file()
    }

    /// `X-Repo` / `X-Root` / `X-Repo-Status` header fields.
    pub fn fields(&self) -> Vec<(String, String)> {
        vec![
            ("X-Repo".into(), self.label.clone()),
            ("X-Root".into(), self.root.display().to_string()),
            (
                "X-Repo-Status".into(),
                if self.exists() { "ok" } else { "missing" }.into(),
            ),
        ]
    }
}

/// Label of a root: the folder containing `issues/`, or the root's own folder
/// name when it isn't called `issues`.
pub fn label_of(root: &Path) -> String {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let own = name(root);
    if own.as_deref() == Some(ISSUES_DIR)
        && let Some(parent) = root.parent().and_then(name)
    {
        return parent;
    }
    own.unwrap_or_else(|| root.display().to_string())
}

/// Registry file: `$LASTISSUE_REPOS`, else `<config dir>/repos`.
pub fn path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os(ENV).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    store::project_dirs()
        .map(|d| d.config_dir().join("repos"))
        .ok_or_else(|| {
            Error::not_found("no-config-dir")
                .with("Message", "cannot determine the platform config directory")
        })
}

fn read_lines(file: &Path) -> Result<Vec<String>> {
    match fs::read_to_string(file) {
        Ok(s) => Ok(s.lines().map(str::to_string).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// The path a registry line holds, if it isn't a comment or blank.
fn entry(line: &str) -> Option<PathBuf> {
    let l = line.trim();
    (!l.is_empty() && !l.starts_with('#')).then(|| PathBuf::from(l))
}

fn write_lines(file: &Path, lines: &[String]) -> Result<()> {
    if let Some(d) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(d)?;
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    store::atomic_write(file, text.as_bytes())
}

/// Registered repos in insertion order, followed by the implicit global root
/// (when the platform has one). Missing paths are kept, never pruned.
pub fn load() -> Result<Vec<Repo>> {
    load_from(&path()?)
}

pub fn load_from(file: &Path) -> Result<Vec<Repo>> {
    let mut out: Vec<Repo> = Vec::new();
    for root in read_lines(file)?.iter().filter_map(|l| entry(l)) {
        if !out.iter().any(|r| r.root == root) {
            out.push(Repo::at(root));
        }
    }
    if let Ok(g) = store::global_root() {
        out.push(Repo {
            label: GLOBAL.into(),
            root: g,
            global: true,
        });
    }
    Ok(out)
}

/// Turn `path` (a root, or a dir containing `issues/`) into a canonical root.
fn root_for(path: &Path) -> Result<PathBuf> {
    for cand in [path.to_path_buf(), path.join(ISSUES_DIR)] {
        if cand.join(MARKER).is_file() {
            return Ok(fs::canonicalize(&cand)?);
        }
    }
    Err(Error::validation("not-a-root")
        .with("Path", path.display().to_string())
        .with("Message", "no .lastissue marker here or in ./issues")
        .with("Hint", "lastissue init"))
}

/// Register a root (or a dir containing `issues/`). Idempotent.
pub fn add(path: &Path) -> Result<Repo> {
    add_to(&self::path()?, path)
}

pub fn add_to(file: &Path, path: &Path) -> Result<Repo> {
    let root = root_for(path)?;
    let mut lines = read_lines(file)?;
    if !lines.iter().filter_map(|l| entry(l)).any(|p| p == root) {
        lines.push(root.display().to_string());
        write_lines(file, &lines)?;
    }
    Ok(Repo::at(root))
}

/// Unregister a repo by label (prefix) or path. `global` can't be removed.
pub fn remove(query: &str) -> Result<Repo> {
    remove_from(&path()?, query)
}

pub fn remove_from(file: &Path, query: &str) -> Result<Repo> {
    let repo = resolve_in(&load_from(file)?, query)?;
    if repo.global {
        return Err(Error::validation("cannot-remove-global").with(
            "Message",
            "the global root is always listed and is not stored in the registry",
        ));
    }
    let lines: Vec<String> = read_lines(file)?
        .into_iter()
        .filter(|l| entry(l).as_deref() != Some(repo.root.as_path()))
        .collect();
    write_lines(file, &lines)?;
    Ok(repo)
}

/// Resolve `--repo` / `repo remove` input: a path (anything containing a
/// separator, or `.`), else a label, exact first and then by prefix.
pub fn resolve(query: &str) -> Result<Repo> {
    resolve_in(&load()?, query)
}

pub fn resolve_in(repos: &[Repo], query: &str) -> Result<Repo> {
    let q = query.trim();
    let labels = || {
        repos
            .iter()
            .map(|r| r.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let not_found = || {
        Error::not_found("repo-not-found")
            .with("Repo", q)
            .with("Repos", labels())
            .with("Hint", "lastissue repo list")
    };
    if q.contains(std::path::MAIN_SEPARATOR) || q == "." || q == ".." {
        let p = Path::new(q);
        let mut cands = vec![std::path::absolute(p)?];
        if let Ok(c) = root_for(p) {
            cands.push(c);
        }
        if let Ok(c) = fs::canonicalize(p) {
            cands.push(c);
        }
        return repos
            .iter()
            .find(|r| cands.contains(&r.root))
            .cloned()
            .ok_or_else(not_found);
    }
    match store::matching(repos, |r| r.label.as_str(), q).as_slice() {
        [] => Err(not_found()),
        [one] => Ok((*one).clone()),
        many => Err(Error::ambiguous_as(
            "ambiguous-repo",
            many.iter()
                .map(|r| format!("{} ({})", r.label, r.root.display()))
                .collect(),
        )
        .with("Hint", "pass the root path instead, or use --root")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{EXIT_AMBIGUOUS, EXIT_NOT_FOUND, EXIT_VALIDATION};
    use tempfile::TempDir;

    fn mkroot(dir: &Path) -> PathBuf {
        store::init(dir).unwrap();
        fs::canonicalize(dir).unwrap()
    }

    fn user_repos(file: &Path) -> Vec<Repo> {
        load_from(file)
            .unwrap()
            .into_iter()
            .filter(|r| !r.global)
            .collect()
    }

    #[test]
    fn labels() {
        assert_eq!(label_of(Path::new("/a/lastissue/issues")), "lastissue");
        assert_eq!(label_of(Path::new("/a/notes")), "notes");
        assert_eq!(label_of(Path::new("/issues")), "issues");
    }

    #[test]
    fn add_is_idempotent_and_accepts_parent_dir() {
        let t = TempDir::new().unwrap();
        let file = t.path().join("cfg/repos");
        let root = mkroot(&t.path().join("proj/issues"));
        let a = add_to(&file, &t.path().join("proj")).unwrap();
        let b = add_to(&file, &root).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.label, "proj");
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("{}\n", root.display())
        );
        let g = load_from(&file).unwrap();
        assert_eq!(g.last().unwrap().label, GLOBAL);
        assert!(g.last().unwrap().global);
    }

    #[test]
    fn add_requires_marker() {
        let t = TempDir::new().unwrap();
        let e = add_to(&t.path().join("repos"), t.path()).unwrap_err();
        assert_eq!(e.code, EXIT_VALIDATION);
        assert_eq!(e.kind, "not-a-root");
    }

    #[test]
    fn comments_kept_missing_shown() {
        let t = TempDir::new().unwrap();
        let file = t.path().join("repos");
        let root = mkroot(&t.path().join("alpha"));
        fs::write(&file, "# my repos\n\n/nowhere/beta/issues\n").unwrap();
        add_to(&file, &root).unwrap();
        let repos = user_repos(&file);
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].label, "beta");
        assert!(!repos[0].exists());
        assert_eq!(repos[0].fields()[2].1, "missing");
        assert_eq!(repos[1].fields()[2].1, "ok");
        remove_from(&file, "beta").unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("# my repos\n\n{}\n", root.display())
        );
    }

    #[test]
    fn resolution() {
        let t = TempDir::new().unwrap();
        let file = t.path().join("repos");
        let a = mkroot(&t.path().join("x/web/issues"));
        let b = mkroot(&t.path().join("y/web/issues"));
        let c = mkroot(&t.path().join("worker"));
        for r in [&a, &b, &c] {
            add_to(&file, r).unwrap();
        }
        let repos = load_from(&file).unwrap();
        assert_eq!(resolve_in(&repos, "wo").unwrap().root, c);
        let e = resolve_in(&repos, "web").unwrap_err();
        assert_eq!(e.code, EXIT_AMBIGUOUS);
        assert_eq!(e.kind, "ambiguous-repo");
        let e = resolve_in(&repos, "zzz").unwrap_err();
        assert_eq!(e.code, EXIT_NOT_FOUND);
        assert!(
            e.fields
                .iter()
                .any(|(k, v)| k == "Repos" && v.contains("worker"))
        );
        assert_eq!(
            resolve_in(&repos, &b.display().to_string()).unwrap().root,
            b
        );
        let parent = t.path().join("x/web");
        assert_eq!(
            resolve_in(&repos, &parent.display().to_string())
                .unwrap()
                .root,
            a
        );
    }

    #[test]
    fn global_cannot_be_removed() {
        let t = TempDir::new().unwrap();
        let file = t.path().join("repos");
        if store::global_root().is_err() {
            return;
        }
        let e = remove_from(&file, GLOBAL).unwrap_err();
        assert_eq!(e.code, EXIT_VALIDATION);
        assert!(!file.exists());
    }
}
