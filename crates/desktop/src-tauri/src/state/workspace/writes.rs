//! Writes through core: one op or a batch, under the repo's write lock,
//! folded straight into the index.

use super::repo::apply;
use super::{SharedHub, events};
use crate::dto::{self, DocDto, MetaDto};
use crate::format::error_line;
use crate::state::edit::{self, Applied, Edit, Op};
use crate::state::view_state::Mode;
use lastissue::error::Result;
use lastissue::index::{self, Reload};
use lastissue::query::GroupKey;
use lastissue::store::Store;
use std::sync::PoisonError;

/// Run a write through core, off the UI thread and outside the workspace
/// lock (but under the repo's write lock). The result is folded into the
/// index at once, so the watcher's echo is a no-op; the open issue takes
/// its new messages. A failure lands in the status line and is returned.
pub fn run(hub: &SharedHub, op: Op) -> std::result::Result<Applied, String> {
    let (mut done, err) = run_all(hub, vec![op]);
    done.pop().ok_or_else(|| err.unwrap_or_default())
}

/// [`run`] for several writes under one write lock, folded in and pushed
/// once. Every op is tried; the first failure is reported.
fn run_all(hub: &SharedHub, ops: Vec<Op>) -> (Vec<Applied>, Option<String>) {
    let (store, lock, me, generation) = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else {
            return (Vec::new(), Some("no repo is open".into()));
        };
        (
            r.store.clone(),
            r.write_lock.clone(),
            ws.identity.clone(),
            r.generation,
        )
    };
    let mut done = Vec::new();
    let mut first_err = None;
    {
        let _g = lock.lock().unwrap_or_else(PoisonError::into_inner);
        for op in &ops {
            match edit::run(&store, &me, op) {
                Ok(a) => done.push(a),
                Err(e) => {
                    first_err.get_or_insert_with(|| error_line(&e));
                }
            }
        }
    }
    if !done.is_empty() {
        let (mut loaded, mut gone) = (Vec::new(), Vec::new());
        for a in &done {
            if let Reload::Paths {
                loaded: l, gone: g, ..
            } = &a.reload
            {
                loaded.extend(l.iter().cloned());
                gone.extend(g.iter().cloned());
            }
        }
        let reload = Reload::Paths {
            loaded,
            gone,
            errors: Vec::new(),
        };
        apply(hub, generation, Ok(reload), true);
    }
    let mut ws = hub.lock();
    ws.error.clone_from(&first_err);
    for a in &done {
        if a.deleted && ws.doc.as_ref().is_some_and(|d| d.id == a.id) {
            ws.doc = None;
        }
        if let Some(doc) = ws.doc.as_mut()
            && a.written
            && doc.id == a.id
        {
            doc.adopt_own(a.path.clone(), a.msgs.clone());
        }
    }
    hub.emit(events::META, &MetaDto::new(&ws));
    hub.emit(events::DOC, &DocDto::new(&ws));
    (done, first_err)
}

fn run_all_ok(hub: &SharedHub, ops: Vec<Op>) -> std::result::Result<(), String> {
    run_all(hub, ops).1.map_or(Ok(()), Err)
}

/// The same change to several issues (pickers, bulk edit).
pub fn apply_to(hub: &SharedHub, ids: &[String], edit: &Edit) -> std::result::Result<(), String> {
    let ops = hub
        .lock()
        .existing(ids)
        .into_iter()
        .map(|id| Op::Update {
            id,
            edit: edit.clone(),
        })
        .collect();
    run_all_ok(hub, ops)
}

/// A card dropped on a column (state or priority, per the grouping).
pub fn drop_card(
    hub: &SharedHub,
    ids: &[String],
    target: GroupKey,
) -> std::result::Result<(), String> {
    let ops = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else { return Ok(()) };
        ids.iter()
            .filter_map(|id| {
                let s = r.index.by_id(id)?;
                let edit = edit::drop_edit(target, s.state, s.priority)?;
                Some(Op::Update {
                    id: id.clone(),
                    edit,
                })
            })
            .collect()
    };
    run_all_ok(hub, ops)
}

/// `⇧h` / `⇧l` on the board: move cards one column each.
pub fn move_cards(hub: &SharedHub, ids: &[String], step: isize) -> std::result::Result<(), String> {
    let ops = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else { return Ok(()) };
        if ws.view.mode != Mode::Board {
            return Ok(());
        }
        let cols: Vec<_> = ws.visible.groups.iter().map(|g| g.key).collect();
        ids.iter()
            .filter_map(|id| {
                let s = r.index.by_id(id)?;
                let cur = GroupKey::of(ws.view.grouping, s);
                let target = edit::neighbour(&cols, cur, step)?;
                let edit = edit::drop_edit(target, s.state, s.priority)?;
                Some(Op::Update {
                    id: id.clone(),
                    edit,
                })
            })
            .collect()
    };
    run_all_ok(hub, ops)
}

pub fn delete(hub: &SharedHub, ids: &[String]) -> std::result::Result<(), String> {
    let ops = hub
        .lock()
        .existing(ids)
        .into_iter()
        .map(|id| Op::Delete { id })
        .collect();
    run_all_ok(hub, ops)
}

/// Create, rename (or delete) a project: a structural change, so the
/// index rescans right away.
pub fn project_op(
    hub: &SharedHub,
    f: impl FnOnce(&Store) -> Result<lastissue::store::Project>,
) -> std::result::Result<dto::ProjectDto, String> {
    let (store, lock, generation) = {
        let ws = hub.lock();
        let Some(r) = &ws.repo else {
            return Err("no repo is open".into());
        };
        (r.store.clone(), r.write_lock.clone(), r.generation)
    };
    let p = {
        let _g = lock.lock().unwrap_or_else(PoisonError::into_inner);
        f(&store).map_err(|e| error_line(&e))?
    };
    let r = index::scan(&store);
    apply(hub, generation, r, true);
    Ok(dto::ProjectDto::from(&p))
}

#[cfg(test)]
mod tests {
    use super::super::doc::select;
    use super::super::events;
    use super::super::fixture::{fx, id_of};
    use super::*;
    use lastissue::model::State;

    #[test]
    fn edits_go_through_core_and_update_the_index() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        apply_to(
            &f.hub,
            std::slice::from_ref(&id),
            &Edit::State(State::InProgress),
        )
        .unwrap();
        let ws = f.hub.lock();
        let s = ws.repo.as_ref().unwrap().index.by_id(&id).unwrap();
        assert_eq!(s.state, State::InProgress);
        let vis = f.rec.last(events::VISIBLE).unwrap();
        let first = usize::try_from(vis["list"][0].as_u64().unwrap()).unwrap();
        assert_eq!(vis["groups"][first]["key"]["value"], "in-progress");
    }

    #[test]
    fn delete_closes_the_open_issue() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        select(&f.hub, Some(id.clone()));
        delete(&f.hub, std::slice::from_ref(&id)).unwrap();
        let ws = f.hub.lock();
        assert!(ws.doc.is_none());
        assert_eq!(ws.repo.as_ref().unwrap().index.len(), 0);
    }
}
