//! The `⌘K` palette's entries (GUI-SPEC §7): every command from the
//! registry, plus one entry per repo (its `⌘N` action) and per sidebar
//! scope (projects). `g p` opens the same list narrowed to scopes. Plain
//! data, matched with the pickers' scorer.

use super::picker::score;
use super::view_state::Scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteMode {
    /// Commands, repos and scopes.
    #[default]
    All,
    /// "Go to project…" (`g p`): scopes only.
    Scopes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Index into [`crate::actions::commands`].
    Command(usize),
    /// Index into the workspace's repos.
    Repo(usize),
    Scope(Scope),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub item: Item,
    pub title: String,
    pub detail: Option<String>,
}

/// What the palette can offer right now.
pub struct Sources<'a> {
    pub commands: &'a [&'a str],
    /// (label, root) per repo.
    pub repos: &'a [(String, String)],
    /// (short ID, display name) per project.
    pub projects: &'a [(String, String)],
}

pub fn entries(src: &Sources, mode: PaletteMode) -> Vec<Entry> {
    let mut out = Vec::new();
    let scope = |item: Scope, title: String| Entry {
        item: Item::Scope(item),
        title,
        detail: None,
    };
    if mode == PaletteMode::All {
        out.extend(src.commands.iter().enumerate().map(|(i, t)| Entry {
            item: Item::Command(i),
            title: (*t).to_string(),
            detail: None,
        }));
        out.extend(
            src.repos
                .iter()
                .enumerate()
                .map(|(i, (label, root))| Entry {
                    item: Item::Repo(i),
                    title: format!("Switch to repo {label}"),
                    detail: Some(root.clone()),
                }),
        );
    }
    let prefix = if mode == PaletteMode::All {
        "Go to project "
    } else {
        ""
    };
    out.extend(
        src.projects
            .iter()
            .map(|(id, name)| scope(Scope::Project(id.clone()), format!("{prefix}{name}"))),
    );
    if mode == PaletteMode::Scopes {
        out.push(scope(Scope::NoProject, "No project".into()));
        out.push(scope(Scope::All, "All issues".into()));
        out.push(scope(Scope::Mine, "My issues".into()));
    }
    out
}

/// Entries matching `query`, best first (ties keep list order).
pub fn filter(entries: Vec<Entry>, query: &str) -> Vec<Entry> {
    let mut scored: Vec<(usize, usize, Entry)> = entries
        .into_iter()
        .enumerate()
        .filter_map(|(i, e)| score(query, &e.title).map(|s| (s, i, e)))
        .collect();
    scored.sort_by_key(|(s, i, _)| (*s, *i));
    scored.into_iter().map(|(_, _, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src<'a>(
        commands: &'a [&'a str],
        repos: &'a [(String, String)],
        projects: &'a [(String, String)],
    ) -> Sources<'a> {
        Sources {
            commands,
            repos,
            projects,
        }
    }

    #[test]
    fn all_lists_commands_repos_projects() {
        let repos = [("work".to_string(), "/w".to_string())];
        let projects = [("p1".to_string(), "auth flow".to_string())];
        let s = src(&["New issue", "Show board"], &repos, &projects);
        let e = entries(&s, PaletteMode::All);
        let titles: Vec<&str> = e.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "New issue",
                "Show board",
                "Switch to repo work",
                "Go to project auth flow"
            ]
        );
        let f = filter(e, "board");
        assert_eq!(f[0].title, "Show board");
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn scopes_mode_is_projects_and_sidebar_items() {
        let projects = [("p1".to_string(), "auth".to_string())];
        let s = src(&["New issue"], &[], &projects);
        let e = entries(&s, PaletteMode::Scopes);
        assert_eq!(e[0].item, Item::Scope(Scope::Project("p1".into())));
        assert_eq!(e.len(), 4);
        let f = filter(e, "no");
        assert_eq!(f[0].item, Item::Scope(Scope::NoProject));
    }

    #[test]
    fn prefix_beats_scattered() {
        let s = src(&["Set priority…", "Show list", "Search issues"], &[], &[]);
        let f = filter(entries(&s, PaletteMode::All), "s");
        // All start with "s": list order is kept.
        assert_eq!(f.len(), 3);
        let f = filter(entries(&s, PaletteMode::All), "list");
        assert_eq!(f[0].title, "Show list");
    }
}
