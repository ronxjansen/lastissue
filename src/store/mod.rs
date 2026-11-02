//! Root discovery, layout, slugs, ID generation/resolution, atomic writes.

pub mod mbox;

use crate::error::{Error, Result};
use crate::model::{self, State};
use mbox::Msg;
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use ulid::Ulid;

pub const MARKER: &str = ".lastissue";
/// Conventional local root subdirectory; preferred over its parent (§2.1).
pub const ISSUES_DIR: &str = "issues";

// ---------------------------------------------------------------- Root

pub fn global_root() -> Result<PathBuf> {
    directories::ProjectDirs::from("", "", "lastissue")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or_else(|| {
            Error::not_found("no-global-dir")
                .with("Message", "cannot determine the platform data directory")
        })
}

fn not_initialized(root: &Path, hint: &str) -> Error {
    Error::not_found("not-initialized")
        .with("Root", root.display().to_string())
        .with("Hint", hint)
}

/// Resolve the issues root (§2.1): `--root` / `LASTISSUE_DIR` (both arrive
/// via `flag`), nearest `.lastissue` upwards (at each level `issues/.lastissue`
/// wins over `.lastissue`), then the global root.
pub fn discover(flag: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = flag {
        let p = absolute(p);
        if !p.join(MARKER).is_file() {
            return Err(not_initialized(
                &p,
                &format!("lastissue --root {} init", p.display()),
            ));
        }
        return Ok(p);
    }
    let cwd = std::env::current_dir()?;
    for dir in cwd.ancestors() {
        for cand in [dir.join(ISSUES_DIR), dir.to_path_buf()] {
            if cand.join(MARKER).is_file() {
                return Ok(cand);
            }
        }
    }
    let g = global_root()?;
    if !g.join(MARKER).is_file() {
        return Err(not_initialized(
            &g,
            "lastissue init (local, in the current directory) or lastissue init --global",
        ));
    }
    Ok(g)
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

/// Directory a plain `lastissue init` uses: `./issues` if it exists, else cwd.
pub fn default_local_root(cwd: &Path) -> PathBuf {
    let issues = cwd.join(ISSUES_DIR);
    if issues.is_dir() {
        issues
    } else {
        cwd.to_path_buf()
    }
}

/// Create a root (marker file). Idempotent.
pub fn init(dir: &Path) -> Result<PathBuf> {
    let dir = absolute(dir);
    fs::create_dir_all(&dir)?;
    let marker = dir.join(MARKER);
    if !marker.exists() {
        File::create(&marker)?;
    }
    Ok(dir)
}

// ---------------------------------------------------------------- Slugs & IDs

/// Kebab-case slug of at most `max_words` words.
pub fn slugify(text: &str, max_words: usize, fallback: &str) -> String {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(max_words)
        .map(|w| w.to_lowercase().chars().take(24).collect())
        .collect();
    if words.is_empty() {
        fallback.to_string()
    } else {
        words.join("-")
    }
}

/// Split `<slug>-<id>` (filename stem or folder name) into (slug, id).
pub fn split_name(name: &str) -> Option<(&str, &str)> {
    let (slug, id) = match name.rsplit_once('-') {
        Some((s, i)) => (s, i),
        None => ("", name),
    };
    (id.len() >= 6 && id.len() <= 8 && model::is_short_id(id)).then_some((slug, id))
}

/// Pick the shortest (6–8 chars) lowercase suffix of a fresh ULID that does
/// not prefix-collide with any existing ID.
pub fn fresh_id(existing: &[String]) -> (Ulid, String) {
    loop {
        let u = model::new_ulid();
        let s = u.to_string().to_lowercase();
        for len in 6..=8 {
            let cand = &s[s.len() - len..];
            if !existing
                .iter()
                .any(|e| e.starts_with(cand) || cand.starts_with(e.as_str()))
            {
                return (u, cand.to_string());
            }
        }
    }
}

/// Short ID for a given (e.g. deterministic) ULID: its shortest 6–8 char
/// suffix that does not prefix-collide with `existing`; None if even 8 do.
pub fn short_id_for(u: Ulid, existing: &[String]) -> Option<String> {
    let s = u.to_string().to_lowercase();
    for len in 6..=8 {
        let cand = &s[s.len() - len..];
        if !existing
            .iter()
            .any(|e| e.starts_with(cand) || cand.starts_with(e.as_str()))
        {
            return Some(cand.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------- Layout

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub dir: PathBuf,
}

impl Project {
    pub fn dir_name(&self) -> String {
        format!("{}-{}", self.slug, self.id)
    }

    /// `new feature <45aef68.lastissue>`
    pub fn list_id(&self) -> String {
        format!("{} <{}.lastissue>", self.slug.replace('-', " "), self.id)
    }
}

#[derive(Debug, Clone)]
pub struct IssueRef {
    pub id: String,
    pub path: PathBuf,
    pub project: Option<Project>,
}

pub struct Store {
    pub root: PathBuf,
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Store { root }
    }

    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .display()
            .to_string()
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut out = Vec::new();
        for e in fs::read_dir(&self.root)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if is_hidden(&name) || !e.file_type()?.is_dir() {
                continue;
            }
            if let Some((slug, id)) = split_name(&name)
                && !slug.is_empty()
            {
                out.push(Project {
                    id: id.to_string(),
                    slug: slug.to_string(),
                    dir: e.path(),
                });
            }
        }
        out.sort_by_key(|a| a.dir_name());
        Ok(out)
    }

    fn issues_in(
        &self,
        dir: &Path,
        project: Option<&Project>,
        out: &mut Vec<IssueRef>,
    ) -> Result<()> {
        for e in fs::read_dir(dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if is_hidden(&name) || !e.file_type()?.is_file() {
                continue;
            }
            let Some(stem) = name.strip_suffix(".mbox") else {
                continue;
            };
            if let Some((_, id)) = split_name(stem) {
                out.push(IssueRef {
                    id: id.to_string(),
                    path: e.path(),
                    project: project.cloned(),
                });
            }
        }
        Ok(())
    }

    /// All issues: root-level first, then each project.
    pub fn issues(&self) -> Result<Vec<IssueRef>> {
        let mut out = Vec::new();
        self.issues_in(&self.root, None, &mut out)?;
        for p in self.projects()? {
            self.issues_in(&p.dir, Some(&p), &mut out)?;
        }
        Ok(out)
    }

    pub fn project_issues(&self, p: &Project) -> Result<Vec<IssueRef>> {
        let mut out = Vec::new();
        self.issues_in(&p.dir, Some(p), &mut out)?;
        Ok(out)
    }

    pub fn read(&self, r: &IssueRef) -> Result<Vec<Msg>> {
        let bytes = fs::read(&r.path)?;
        let msgs = mbox::parse_mbox(&bytes)?;
        if msgs.is_empty() {
            return Err(Error::validation("corrupt-file").with("Path", self.rel(&r.path)));
        }
        Ok(msgs)
    }

    pub fn read_head(&self, r: &IssueRef) -> Result<Msg> {
        mbox::read_head(BufReader::new(File::open(&r.path)?))
    }

    pub fn stats(
        &self,
        r: &IssueRef,
    ) -> Result<(usize, Option<chrono::DateTime<chrono::FixedOffset>>)> {
        mbox::scan_stats(BufReader::new(File::open(&r.path)?))
    }

    /// Atomic write: temp file in the same directory, then rename.
    pub fn write(&self, path: &Path, msgs: &[Msg]) -> Result<()> {
        let dir = path.parent().unwrap_or(&self.root);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = dir.join(format!(".{name}.{}.tmp", model::new_ulid()));
        let res = (|| -> std::io::Result<()> {
            let mut f = File::create(&tmp)?;
            f.write_all(mbox::serialize(msgs).as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, path)
        })();
        if res.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        Ok(res?)
    }

    pub fn issue_path(&self, project: Option<&Project>, slug: &str, id: &str) -> PathBuf {
        let dir = project
            .map(|p| p.dir.clone())
            .unwrap_or_else(|| self.root.clone());
        dir.join(format!("{slug}-{id}.mbox"))
    }

    // ------------------------------------------------------------ resolution

    /// Resolve an issue by short-ID prefix, full ULID / Message-ID, or a
    /// `slug-id` / filename (the ID suffix is what counts).
    pub fn resolve_issue(&self, query: &str) -> Result<IssueRef> {
        let all = self.issues()?;
        let raw = query.trim();
        let mut q = raw.trim_start_matches('<').trim_end_matches('>');
        q = q.split('@').next().unwrap_or(q);
        q = q.rsplit('/').next().unwrap_or(q);
        q = q.strip_suffix(".mbox").unwrap_or(q);
        let q = q.rsplit('-').next().unwrap_or(q);
        let q = model::normalize_id_input(q);
        let not_found = || {
            Error::not_found("issue-not-found")
                .with("Id", raw)
                .with("Hint", "lastissue issue list --all")
        };
        if !model::is_short_id(&q) {
            return Err(not_found());
        }
        if q.len() == 26 {
            // Full ULID: the short ID is one of its 6–8 char suffixes; confirm via Message-ID.
            for r in &all {
                if q.ends_with(&r.id) {
                    let head = self.read_head(r)?;
                    if mbox::ulid_of(head.message_id()).eq_ignore_ascii_case(&q) {
                        return Ok(r.clone());
                    }
                }
            }
            return Err(not_found());
        }
        let mut hits: Vec<IssueRef> = all.into_iter().filter(|r| r.id.starts_with(&q)).collect();
        if let Some(exact) = hits.iter().position(|r| r.id == q) {
            return Ok(hits.swap_remove(exact));
        }
        match hits.len() {
            0 => Err(not_found()),
            1 => Ok(hits.remove(0)),
            _ => Err(Error::ambiguous(hits.into_iter().map(|r| r.id).collect())),
        }
    }

    pub fn resolve_project(&self, query: &str) -> Result<Project> {
        let raw = query.trim();
        let q = raw.trim_end_matches('/');
        let q = q.rsplit('/').next().unwrap_or(q);
        let q = model::normalize_id_input(q.rsplit('-').next().unwrap_or(q));
        let mut hits: Vec<Project> = self
            .projects()?
            .into_iter()
            .filter(|p| p.id.starts_with(&q))
            .collect();
        if let Some(exact) = hits.iter().position(|p| p.id == q) {
            return Ok(hits.swap_remove(exact));
        }
        match hits.len() {
            0 => Err(Error::not_found("project-not-found")
                .with("Project", raw)
                .with("Hint", "lastissue project list")),
            1 => Ok(hits.remove(0)),
            _ => Err(Error::ambiguous(hits.into_iter().map(|p| p.id).collect())),
        }
    }

    /// Map Message-ID → state over every issue (for readiness checks).
    pub fn states_by_message_id(
        &self,
        heads: &[(IssueRef, Msg)],
    ) -> std::collections::HashMap<String, State> {
        heads
            .iter()
            .filter_map(|(_, h)| {
                let st = State::parse(h.get("X-Issue-State").unwrap_or("backlog"))?;
                Some((h.message_id().to_string(), st))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slugify("Login crashes on empty password!", 4, "issue"),
            "login-crashes-on-empty"
        );
        assert_eq!(slugify("New Feature", 2, "p"), "new-feature");
        assert_eq!(slugify("¿¡!!", 4, "issue"), "issue");
        assert_eq!(slugify("Über größe", 4, "issue"), "über-größe");
    }

    #[test]
    fn names() {
        assert_eq!(
            split_name("login-crashes-7hq2mz"),
            Some(("login-crashes", "7hq2mz"))
        );
        assert_eq!(split_name("src"), None);
        assert_eq!(split_name("foo-bar"), None);
    }

    #[test]
    fn ids_avoid_prefix_collisions() {
        let mut ids = Vec::new();
        for _ in 0..200 {
            let (_, id) = fresh_id(&ids);
            assert!((6..=8).contains(&id.len()));
            assert!(
                !ids.iter()
                    .any(|e: &String| e.starts_with(&id) || id.starts_with(e.as_str()))
            );
            ids.push(id);
        }
    }
}
