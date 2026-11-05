//! User keymap overrides (GUI-SPEC §7): `gui-keymap.json` in the config
//! dir, read here and watched, and pushed to the front-end, which owns the
//! defaults and the dispatch.
//!
//! The format is flat: context → key sequence → action, with `null`
//! unbinding a default.
//!
//! ```json
//! { "issues": { "shift-x": "toggleSelect", "x": null } }
//! ```

use notify::{RecursiveMode, Watcher as _};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const USER_FILE: &str = "gui-keymap.json";

/// What a new user keymap file starts as.
pub const TEMPLATE: &str = r#"{
  "//": "Overrides for the default keymap: context -> keys -> action (null unbinds). Contexts: global, workspace, issues, list, board, detail, picker, modal. The command palette (cmd-k) lists every action with its binding.",
  "issues": {}
}
"#;

pub fn user_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "lastissue").map(|d| d.config_dir().join(USER_FILE))
}

/// context → keys → action (`None`: unbind).
pub type Bindings = BTreeMap<String, BTreeMap<String, Option<String>>>;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserKeymap {
    pub path: Option<String>,
    pub bindings: Bindings,
    /// Problems with the file (shown in settings and the palette).
    pub errors: Vec<String>,
}

/// Parse a user keymap's text. A shape problem is reported, and the rest
/// still loads.
pub fn parse(text: &str) -> (Bindings, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return (out, vec![format!("{USER_FILE}: {e}")]),
    };
    let Some(obj) = v.as_object() else {
        return (out, vec![format!("{USER_FILE}: expected an object of contexts")]);
    };
    for (ctx, binds) in obj {
        if ctx.starts_with("//") {
            continue;
        }
        let Some(binds) = binds.as_object() else {
            errors.push(format!("{ctx}: expected an object of key → action"));
            continue;
        };
        let mut m = BTreeMap::new();
        for (keys, action) in binds {
            match action {
                serde_json::Value::String(a) => {
                    m.insert(keys.clone(), Some(a.clone()));
                }
                serde_json::Value::Null => {
                    m.insert(keys.clone(), None);
                }
                _ => errors.push(format!("{ctx} / {keys}: expected an action name or null")),
            }
        }
        out.insert(ctx.clone(), m);
    }
    (out, errors)
}

pub fn load(path: Option<&Path>) -> UserKeymap {
    let Some(path) = path else {
        return UserKeymap::default();
    };
    let mut km = UserKeymap {
        path: Some(path.display().to_string()),
        ..UserKeymap::default()
    };
    match std::fs::read_to_string(path) {
        Ok(text) => (km.bindings, km.errors) = parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => km.errors.push(format!("{}: {e}", path.display())),
    }
    km
}

/// Call `changed` with the reloaded keymap whenever the file changes.
/// Watches the parent directory, so creating the file counts too.
pub fn watch(
    path: PathBuf,
    changed: impl Fn(UserKeymap) + Send + 'static,
) -> notify::Result<notify::RecommendedWatcher> {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&dir).map_err(notify::Error::io)?;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut w = notify::recommended_watcher(tx)?;
    w.watch(&dir, RecursiveMode::NonRecursive)?;
    std::thread::Builder::new()
        .name("lastissue-keymap".into())
        .spawn(move || {
            let mut last = load(Some(&path));
            while let Ok(ev) = rx.recv() {
                let ev: notify::Result<notify::Event> = ev;
                let hit = ev.map_or(true, |e| {
                    e.paths.iter().any(|p| p.file_name() == path.file_name())
                });
                if !hit {
                    continue;
                }
                // Editors save in bursts; let it settle.
                std::thread::sleep(std::time::Duration::from_millis(50));
                while rx.try_recv().is_ok() {}
                let km = load(Some(&path));
                if km != last {
                    last = km.clone();
                    changed(km);
                }
            }
        })
        .map_err(notify::Error::io)?;
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bindings_unbinds_and_reports_bad_entries() {
        let (b, e) = parse(r#"{"issues": {"x": "toggleSelect", "j": null, "k": 3}, "bad": 1}"#);
        assert_eq!(b["issues"]["x"].as_deref(), Some("toggleSelect"));
        assert_eq!(b["issues"]["j"], None);
        assert!(!b["issues"].contains_key("k"));
        assert_eq!(e.len(), 2, "{e:?}");
        let (b, e) = parse("nope");
        assert!(b.is_empty() && e.len() == 1);
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let km = load(Some(&dir.path().join(USER_FILE)));
        assert!(km.bindings.is_empty() && km.errors.is_empty());
    }
}
