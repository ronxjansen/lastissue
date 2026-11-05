//! The IPC surface: thin `#[tauri::command]`s over [`crate::state::workspace`].
//! Every command is `async`, so it runs off the main thread; results come
//! back as values, and everything else is pushed as events.

use crate::dto::{self, EditDto, GroupKeyDto, NewIssueDto, PickOptionDto, Snapshot, ViewDto};
use crate::keymap::UserKeymap;
use crate::link::Link;
use crate::state::draft::FieldKind;
use crate::state::edit::Op;
use crate::state::picker::PickerKind;
use crate::state::workspace::{self as ws, SharedHub};
use std::sync::Mutex;
use tauri::State;

type Hub<'a> = State<'a, SharedHub>;
type Res<T = ()> = Result<T, String>;

/// The user keymap as last loaded, and its file watcher.
pub struct Keymap(pub Mutex<UserKeymap>, #[allow(dead_code)] pub Option<notify::RecommendedWatcher>);

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        snapshot,
        set_view,
        select,
        doc_edit,
        doc_save,
        doc_action,
        apply_edit,
        drop_card,
        move_cards,
        delete_issues,
        create_issue,
        add_comment,
        edit_comment,
        delete_comment,
        create_project,
        rename_project,
        picker_options,
        search,
        open_repo,
        add_repo,
        open_url,
        issue_link,
        open_keymap_file,
    ]
}

#[tauri::command]
async fn snapshot(hub: Hub<'_>, keymap: State<'_, Keymap>) -> Res<Snapshot> {
    let km = keymap.0.lock().map_err(|e| e.to_string())?.clone();
    Ok(hub.lock().snapshot(km))
}

#[tauri::command]
async fn set_view(hub: Hub<'_>, view: ViewDto) -> Res {
    ws::set_view(&hub, &view)
}

#[tauri::command]
async fn select(hub: Hub<'_>, id: Option<String>) -> Res {
    ws::select(&hub, id);
    Ok(())
}

fn field(kind: &str) -> Res<FieldKind> {
    match kind {
        "title" => Ok(FieldKind::Title),
        "body" => Ok(FieldKind::Body),
        k => Err(format!("unknown field {k:?}")),
    }
}

/// Returns whether a save is due.
#[tauri::command]
async fn doc_edit(hub: Hub<'_>, id: String, field_kind: String, text: String) -> Res<bool> {
    Ok(ws::doc_edit(&hub, &id, field(&field_kind)?, &text))
}

#[tauri::command]
async fn doc_save(hub: Hub<'_>) -> Res {
    ws::save_doc(&hub);
    Ok(())
}

#[tauri::command]
async fn doc_action(hub: Hub<'_>, action: String) -> Res {
    ws::doc_action(&hub, &action)
}

#[tauri::command]
async fn apply_edit(hub: Hub<'_>, ids: Vec<String>, edit: EditDto) -> Res {
    ws::apply_to(&hub, &ids, &edit.to_edit()?)
}

#[tauri::command]
async fn drop_card(hub: Hub<'_>, ids: Vec<String>, target: GroupKeyDto) -> Res {
    ws::drop_card(&hub, &ids, target.to_key()?)
}

#[tauri::command]
async fn move_cards(hub: Hub<'_>, ids: Vec<String>, step: isize) -> Res {
    ws::move_cards(&hub, &ids, step)
}

#[tauri::command]
async fn delete_issues(hub: Hub<'_>, ids: Vec<String>) -> Res {
    ws::delete(&hub, &ids)
}

/// Returns the new issue's short ID (and opens it).
#[tauri::command]
async fn create_issue(hub: Hub<'_>, issue: NewIssueDto) -> Res<String> {
    let a = ws::run(&hub, Op::Create(Box::new(issue.to_new()?)))?;
    ws::select(&hub, Some(a.id.clone()));
    Ok(a.id)
}

#[tauri::command]
async fn add_comment(hub: Hub<'_>, id: String, body: String, reply_to: Option<String>) -> Res {
    ws::run(&hub, Op::AddComment { id, body, reply_to }).map(drop)
}

#[tauri::command]
async fn edit_comment(hub: Hub<'_>, id: String, message_id: String, body: String) -> Res {
    ws::run(&hub, Op::EditComment {
        id,
        message_id,
        body,
    })
    .map(drop)
}

#[tauri::command]
async fn delete_comment(hub: Hub<'_>, id: String, message_id: String) -> Res {
    ws::run(&hub, Op::DeleteComment { id, message_id }).map(drop)
}

#[tauri::command]
async fn create_project(hub: Hub<'_>, name: String) -> Res<dto::ProjectDto> {
    ws::project_op(&hub, |s| lastissue::ops::create_project(s, &name))
}

#[tauri::command]
async fn rename_project(hub: Hub<'_>, project: String, name: String) -> Res<dto::ProjectDto> {
    ws::project_op(&hub, |s| lastissue::ops::rename_project(s, &project, &name))
}

fn picker_kind(kind: &str) -> Res<PickerKind> {
    Ok(match kind {
        "state" => PickerKind::State,
        "priority" => PickerKind::Priority,
        "assignee" => PickerKind::Assignee,
        "labels" => PickerKind::Labels,
        "due" => PickerKind::Due,
        "project" => PickerKind::Project,
        k => return Err(format!("unknown picker {k:?}")),
    })
}

#[tauri::command]
async fn picker_options(
    hub: Hub<'_>,
    kind: String,
    ids: Vec<String>,
    query: String,
) -> Res<Vec<PickOptionDto>> {
    Ok(ws::picker_options(&hub, picker_kind(&kind)?, &ids, &query))
}

#[tauri::command]
async fn search(hub: Hub<'_>, query: String) -> Res {
    ws::set_search(&hub, &query);
    Ok(())
}

/// Switch to the registry's `index`th repo.
#[tauri::command]
async fn open_repo(hub: Hub<'_>, index: usize) -> Res {
    let repo = hub
        .lock()
        .repos
        .get(index)
        .cloned()
        .ok_or_else(|| format!("no repo #{}", index + 1))?;
    ws::open_repo(&hub, repo);
    Ok(())
}

/// Register a folder (its issues root is found as the CLI would) and open it.
#[tauri::command]
async fn add_repo(hub: Hub<'_>, path: String) -> Res {
    let root = lastissue::store::discover(Some(std::path::Path::new(&path)))
        .map_err(|e| e.to_string().trim().replace('\n', " · "))?;
    let repo = lastissue::registry::add(&root).map_err(|e| e.to_string().trim().replace('\n', " · "))?;
    hub.lock().repos = lastissue::registry::load().map_err(|e| e.to_string())?;
    ws::open_repo(&hub, repo);
    Ok(())
}

#[tauri::command]
async fn open_url(hub: Hub<'_>, url: String) -> Res {
    ws::open_url(&hub, &url);
    Ok(())
}

#[tauri::command]
async fn issue_link(hub: Hub<'_>, id: String) -> Res<String> {
    let ws = hub.lock();
    let r = ws.repo.as_ref().ok_or("no repo is open")?;
    Ok(Link {
        repo: r.repo.label.clone(),
        issue: Some(id),
    }
    .to_url())
}

/// Create the user keymap file if needed and open it in the default editor.
#[tauri::command]
async fn open_keymap_file() -> Res<String> {
    let path = crate::keymap::user_path().ok_or("no config directory")?;
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, crate::keymap::TEMPLATE).map_err(|e| e.to_string())?;
    }
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}
