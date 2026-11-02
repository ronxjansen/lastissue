//! The keymap (GUI-SPEC §7): defaults embedded from
//! `default-keymap.json`, user overrides from `gui-keymap.json` in the
//! config dir, both in Zed's format, hot-reloaded when the user file
//! changes:
//!
//! ```json
//! [{ "context": "Issues", "bindings": { "j": "lastissue::CursorDown", "x": null } }]
//! ```
//!
//! A `null` action unbinds the key in that context. User bindings are
//! loaded last, so they win over the defaults at the same depth.

use futures::StreamExt as _;
use gpui::{App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate, NoAction};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

pub const DEFAULT_KEYMAP: &str = include_str!("default-keymap.json");
pub const USER_FILE: &str = "gui-keymap.json";

/// One `"keys": action` line of a keymap file.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub keys: String,
    /// `None` for `null` (unbind).
    pub action: Option<String>,
    pub data: Option<serde_json::Value>,
    pub context: Option<String>,
}

/// Parse a keymap file. Errors name the offending section or key.
pub fn parse(json: &str) -> Result<Vec<Entry>, String> {
    let v: serde_json::Value =
        serde_json::from_str(&strip_comments(json)).map_err(|e| e.to_string())?;
    let sections = v.as_array().ok_or("a keymap is an array of sections")?;
    let mut out = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        let context = match sec.get("context") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(format!("section {i}: \"context\" must be a string")),
        };
        let Some(bindings) = sec.get("bindings").and_then(|b| b.as_object()) else {
            return Err(format!("section {i}: missing \"bindings\" object"));
        };
        for (keys, action) in bindings {
            let (action, data) = match action {
                serde_json::Value::Null => (None, None),
                serde_json::Value::String(s) => (Some(s.clone()), None),
                serde_json::Value::Array(a) => match a.as_slice() {
                    [serde_json::Value::String(s), data] => (Some(s.clone()), Some(data.clone())),
                    [serde_json::Value::String(s)] => (Some(s.clone()), None),
                    _ => return Err(format!("\"{keys}\": expected [\"action\", {{…}}]")),
                },
                _ => return Err(format!("\"{keys}\": expected an action name or null")),
            };
            out.push(Entry {
                keys: keys.clone(),
                action,
                data,
                context: context.clone(),
            });
        }
    }
    Ok(out)
}

/// Drop `//` line comments outside strings (Zed keymaps are JSONC).
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let (mut in_str, mut escaped) = (false, false);
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
        } else if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            in_str = c == '"';
            out.push(c);
        }
    }
    out
}

/// Turn entries into bindings. Bad entries are skipped and reported.
pub fn bindings(entries: &[Entry], cx: &App) -> (Vec<KeyBinding>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for e in entries {
        let action = match &e.action {
            None => Box::new(NoAction),
            Some(name) => match cx.build_action(name, e.data.clone()) {
                Ok(a) => a,
                Err(err) => {
                    errors.push(format!("\"{}\": {err}", e.keys));
                    continue;
                }
            },
        };
        let predicate = match e.context.as_deref().map(KeyBindingContextPredicate::parse) {
            None => None,
            Some(Ok(p)) => Some(Rc::new(p)),
            Some(Err(err)) => {
                errors.push(format!("context {:?}: {err}", e.context));
                continue;
            }
        };
        match KeyBinding::load(
            &e.keys,
            action,
            predicate,
            false,
            None,
            &DummyKeyboardMapper,
        ) {
            Ok(b) => out.push(b),
            Err(err) => errors.push(format!("\"{}\": {err}", e.keys)),
        }
    }
    (out, errors)
}

/// The user's keymap file: `<config dir>/gui-keymap.json`
/// (`~/Library/Application Support/lastissue/gui-keymap.json`).
pub fn user_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "lastissue").map(|d| d.config_dir().join(USER_FILE))
}

/// What the keymap was built from, kept to rebuild it on reload.
struct State {
    /// Bindings registered before ours (gpui-component's), restored first.
    base: Vec<KeyBinding>,
    user: Option<PathBuf>,
    /// Problems in the last load (bad JSON, unknown actions).
    errors: Vec<String>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Global for State {}

/// Install the keymap. Call after every other crate has bound its keys.
/// With a `user` path, watch it and reload whenever it changes.
pub fn install(user: Option<PathBuf>, cx: &mut App) {
    let base: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    let watcher = user.as_deref().and_then(|p| watch(p, cx));
    cx.set_global(State {
        base,
        user,
        errors: Vec::new(),
        _watcher: watcher,
    });
    reload(cx);
}

/// Rebuild the keymap: base, defaults, then the user file.
pub fn reload(cx: &mut App) {
    let (base, user) = {
        let s = cx.global::<State>();
        (s.base.clone(), s.user.clone())
    };
    let defaults = parse(DEFAULT_KEYMAP).expect("default keymap parses");
    let (mut all, mut errors) = bindings(&defaults, cx);
    debug_assert!(errors.is_empty(), "default keymap: {errors:?}");
    if let Some(path) = &user {
        match std::fs::read_to_string(path) {
            Ok(text) => match parse(&text) {
                Ok(entries) => {
                    let (b, e) = bindings(&entries, cx);
                    all.extend(b);
                    errors.extend(e);
                }
                Err(e) => errors.push(e),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => errors.push(e.to_string()),
        }
    }
    for e in &errors {
        eprintln!("lastissue-gui: {}: {e}", user_label(user.as_deref()));
    }
    cx.clear_key_bindings();
    cx.bind_keys(base);
    cx.bind_keys(all);
    cx.global_mut::<State>().errors = errors;
}

fn user_label(p: Option<&Path>) -> String {
    p.map_or_else(|| USER_FILE.to_string(), |p| p.display().to_string())
}

/// Problems in the last keymap load, for the status line.
pub fn errors(cx: &App) -> &[String] {
    cx.try_global::<State>().map_or(&[], |s| &s.errors)
}

/// Watch the user file's directory (the file may not exist yet).
fn watch(path: &Path, cx: &mut App) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher as _;
    let dir = path.parent()?.to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    let name = path.file_name()?.to_os_string();
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
    let mut w = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
        if let Ok(ev) = ev
            && ev.paths.iter().any(|p| p.file_name() == Some(&name))
        {
            let _ = tx.unbounded_send(());
        }
    })
    .ok()?;
    w.watch(&dir, notify::RecursiveMode::NonRecursive).ok()?;
    cx.spawn(async move |cx| {
        while rx.next().await.is_some() {
            // Editors save in several steps; settle first.
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            while rx.try_recv().is_ok() {}
            cx.update(reload);
        }
    })
    .detach();
    Some(w)
}

/// A starter user keymap, written by "Open settings" when there is none.
pub const USER_TEMPLATE: &str = r#"// lastissue keymap overrides (Zed format). Saved changes apply at once.
// Bindings here win over the defaults; map a key to null to unbind it.
// Contexts: Workspace, Issues (mode == list | board), IssueDetail, Picker,
// CreateModal, Confirm. Add "&& !Input" to skip text fields.
[
  {
    "context": "Issues",
    "bindings": {
      // "n": "lastissue::NewIssue"
    }
  }
]
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{INTERNAL, commands, switch_repo};

    #[test]
    fn parses_strings_nulls_and_args() {
        let e = parse(
            r#"[{"bindings": {"cmd-q": "a::Quit"}},
                {"context": "Issues", "bindings": {"x": null, "y": ["a::B", {"n": 1}]}}]"#,
        )
        .unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].context, None);
        assert_eq!(e[1].action, None);
        assert_eq!(e[2].data, Some(serde_json::json!({"n": 1})));
        assert!(parse("{}").is_err());
        // Comments (JSONC), but not inside strings.
        let e = parse("[{\"bindings\": {\"g //\": \"a::B\" // trailing\n}}]").unwrap();
        assert_eq!(e[0].keys, "g //");
        assert!(parse(USER_TEMPLATE).unwrap().is_empty());
        assert!(parse(r#"[{"bindings": {"x": 3}}]"#).is_err());
    }

    /// Every default binding names a palette command (or plumbing), and
    /// every palette command has a default binding unless it is
    /// palette-only.
    #[test]
    fn defaults_cover_the_command_registry() {
        let entries = parse(DEFAULT_KEYMAP).unwrap();
        let cmds: Vec<String> = commands()
            .iter()
            .map(|c| (c.build)().name().to_string())
            .chain((0..9).map(|i| switch_repo(i).unwrap().name().to_string()))
            .collect();
        for e in &entries {
            let a = e.action.as_deref().unwrap();
            assert!(
                cmds.iter().any(|c| c == a) || INTERNAL.contains(&a),
                "{a} is bound but not a command"
            );
            if let Some(c) = &e.context {
                KeyBindingContextPredicate::parse(c).unwrap();
            }
        }
        let palette_only = [
            "lastissue::ToggleGrouping",
            "lastissue::CycleSort",
            "lastissue::CopyIssueLink",
        ];
        for c in &cmds {
            assert!(
                palette_only.contains(&c.as_str())
                    || entries.iter().any(|e| e.action.as_deref() == Some(c)),
                "{c} has no default binding"
            );
        }
    }

    #[gpui::test]
    fn user_file_overrides_and_reloads(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join(USER_FILE);
        std::fs::write(
            &user,
            r#"[{"context": "Issues", "bindings": {"n": "lastissue::NewIssue", "x": null, "q": "nope::Nope"}}]"#,
        )
        .unwrap();
        cx.update(|cx| {
            install(Some(user.clone()), cx);
            let km = cx.key_bindings();
            let km = km.borrow();
            let keys = |a: &dyn gpui::Action| -> Vec<String> {
                km.bindings_for_action(a)
                    .map(|b| {
                        b.keystrokes()
                            .iter()
                            .map(|k| k.key().to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect()
            };
            assert_eq!(keys(&crate::actions::NewIssue), ["c", "n"]);
            // `null` unbinds `x` in that context.
            assert!(keys(&crate::actions::ToggleSelect).is_empty());
            drop(km);
            assert_eq!(errors(cx).len(), 1, "{:?}", errors(cx));
        });
        std::fs::write(&user, "[]").unwrap();
        cx.update(|cx| {
            reload(cx);
            let km = cx.key_bindings();
            let n = km
                .borrow()
                .bindings_for_action(&crate::actions::NewIssue)
                .count();
            assert_eq!(n, 1);
            assert!(errors(cx).is_empty());
        });
    }
}
