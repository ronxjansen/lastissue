//! The issue in the detail pane: selecting, loading and saving it.

use super::writes::run;
use super::{SharedHub, Workspace, events};
use crate::dto::DocDto;
use crate::format::error_line;
use crate::state::doc::Doc;
use crate::state::draft::FieldKind;
use crate::state::edit::{Edit, Op};
use serde::Deserialize;
use std::path::PathBuf;

/// Open an issue in the detail pane (`None` closes it).
pub fn select(hub: &SharedHub, id: Option<String>) {
    let mut ws = hub.lock();
    select_locked(hub, &mut ws, id);
}

pub(super) fn select_locked(hub: &SharedHub, ws: &mut Workspace, id: Option<String>) {
    if id.is_some() && ws.doc.as_ref().map(|d| &d.id) == id.as_ref() {
        return;
    }
    if ws.doc.as_ref().is_some_and(Doc::is_dirty) {
        save_doc_locked(hub, ws);
    }
    ws.doc = None;
    if let (Some(id), Some(r)) = (id, &ws.repo)
        && let Some(s) = r.index.by_id(&id)
    {
        let path = s.path.clone();
        ws.doc_generation += 1;
        ws.doc = Some(Doc::new(ws.doc_generation, id, path.clone()));
        load_doc(hub, ws, path);
    }
    hub.emit(events::DOC, &DocDto::new(ws));
}

/// Read the open issue from `path` in the background.
pub(super) fn load_doc(hub: &SharedHub, ws: &mut Workspace, path: PathBuf) {
    let (Some(doc), Some(r)) = (ws.doc.as_mut(), &ws.repo) else {
        return;
    };
    doc.path.clone_from(&path);
    let (generation, store) = (doc.generation, r.store.clone());
    let hub = hub.clone();
    std::thread::spawn(move || {
        let r = store
            .issue_ref_for(&path)
            .ok_or_else(|| {
                lastissue::error::Error::not_found("issue-not-found")
                    .with("Path", path.display().to_string())
            })
            .and_then(|r| store.read(&r));
        let mut ws = hub.lock();
        let drafts = ws.drafts.clone();
        let Some(doc) = ws.doc.as_mut().filter(|d| d.generation == generation) else {
            return;
        };
        match r {
            Ok(msgs) if !msgs.is_empty() => doc.loaded(msgs, drafts.as_ref()),
            Ok(_) => doc.error = Some("the issue file is empty".into()),
            Err(e) => doc.error = Some(error_line(&e)),
        }
        hub.emit(events::DOC, &DocDto::new(&ws));
    });
}

/// The editor's text changed. Returns whether a save is due (the front-end
/// then calls [`save_doc`] after its debounce).
pub fn doc_edit(hub: &SharedHub, id: &str, kind: FieldKind, text: &str) -> bool {
    let mut ws = hub.lock();
    let drafts = ws.drafts.clone();
    let Some(doc) = ws.doc.as_mut().filter(|d| d.id == id) else {
        return false;
    };
    let was = doc.is_dirty();
    let due = doc.edit(kind, text, drafts.as_ref());
    if due != was {
        hub.emit(events::DOC, &DocDto::new(&ws));
    }
    due
}

pub fn save_doc(hub: &SharedHub) {
    let mut ws = hub.lock();
    save_doc_locked(hub, &mut ws);
}

/// Write the open issue's drafts (title, then body). Runs on a thread;
/// each write goes through [`run`].
fn save_doc_locked(hub: &SharedHub, ws: &mut Workspace) {
    let drafts = ws.drafts.clone();
    let Some(doc) = ws.doc.as_ref() else { return };
    doc.mirror(drafts.as_ref());
    let edits = doc.to_save();
    if edits.is_empty() {
        return;
    }
    let (id, generation) = (doc.id.clone(), doc.generation);
    let hub = hub.clone();
    std::thread::spawn(move || {
        let mut error = None;
        for (kind, text) in edits {
            let edit = match kind {
                FieldKind::Title => Edit::Title(text.clone()),
                FieldKind::Body => Edit::Body(text.clone()),
            };
            match run(
                &hub,
                Op::Update {
                    id: id.clone(),
                    edit,
                },
            ) {
                Ok(_) => {
                    let mut ws = hub.lock();
                    let drafts = ws.drafts.clone();
                    if let Some(d) = ws.doc.as_mut().filter(|d| d.generation == generation) {
                        d.saved(kind, &text, drafts.as_ref());
                    }
                }
                Err(e) => error = Some(e),
            }
        }
        let mut ws = hub.lock();
        if let Some(d) = ws.doc.as_mut().filter(|d| d.generation == generation) {
            d.save_error = error;
        }
        hub.emit(events::DOC, &DocDto::new(&ws));
    });
}

/// The conflict banner's and the leftover-draft banner's buttons.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DocAction {
    KeepMine,
    UseTheirs,
    RestoreLeftover,
    DiscardLeftover,
}

pub fn doc_action(hub: &SharedHub, action: DocAction) {
    let mut ws = hub.lock();
    let drafts = ws.drafts.clone();
    let Some(doc) = ws.doc.as_mut() else {
        return;
    };
    let save = match action {
        DocAction::KeepMine => {
            doc.keep_mine();
            true
        }
        DocAction::UseTheirs => {
            doc.use_theirs(drafts.as_ref());
            false
        }
        DocAction::RestoreLeftover => doc.restore_leftover(drafts.as_ref()),
        DocAction::DiscardLeftover => {
            doc.discard_leftover(drafts.as_ref());
            false
        }
    };
    if save {
        save_doc_locked(hub, &mut ws);
    }
    hub.emit(events::DOC, &DocDto::new(&ws));
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{fx, id_of, wait};
    use super::*;

    #[test]
    fn doc_edits_save_and_coalesce() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        select(&f.hub, Some(id.clone()));
        wait("doc load", || {
            f.hub.lock().doc.as_ref().is_some_and(|d| d.msgs.is_some())
        });
        assert!(doc_edit(&f.hub, &id, FieldKind::Body, "new body\n"));
        assert!(doc_edit(&f.hub, &id, FieldKind::Title, "renamed"));
        save_doc(&f.hub);
        wait("save", || !f.hub.lock().doc.as_ref().unwrap().is_dirty());
        let r = f.store.resolve_issue(&id).unwrap();
        let msgs = f.store.read(&r).unwrap();
        assert_eq!(msgs[0].get("Subject"), Some("renamed"));
        assert_eq!(msgs[0].body, "new body");
        // Title and body edits coalesce into one event.
        let events: Vec<_> = msgs
            .iter()
            .filter(|m| lastissue::model::is_event(m))
            .collect();
        assert_eq!(events.len(), 1, "{msgs:?}");
    }
}
