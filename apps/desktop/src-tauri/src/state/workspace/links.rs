//! `lastissue://` links (GUI-SPEC §10).

use super::doc::select_locked;
use super::repo::open_repo;
use super::{SharedHub, Workspace, events};
use crate::dto::MetaDto;
use crate::link::Link;

/// Open a `lastissue://` URL (GUI-SPEC §10): switch to the labelled repo
/// if needed, then select the issue and focus the detail pane, at once or
/// as soon as the repo has loaded.
pub fn open_url(hub: &SharedHub, url: &str) {
    if let Err(e) = crate::link::parse(url).and_then(|l| open_link(hub, l)) {
        let mut ws = hub.lock();
        ws.error = Some(e);
        hub.emit(events::META, &MetaDto::new(&ws));
    }
}

fn open_link(hub: &SharedHub, link: Link) -> std::result::Result<(), String> {
    let (repo, switch) = {
        let ws = hub.lock();
        let repo = ws
            .repos
            .iter()
            .find(|r| r.label == link.repo)
            .or_else(|| {
                ws.repos
                    .iter()
                    .find(|r| r.label.eq_ignore_ascii_case(&link.repo))
            })
            .cloned()
            .ok_or_else(|| format!("no registered repo labelled {:?}", link.repo))?;
        let current = ws.repo.as_ref().map(|r| r.repo.root.clone());
        let switch = current.as_ref() != Some(&repo.root);
        (repo, switch)
    };
    if switch {
        open_repo(hub, repo)?;
    }
    let mut ws = hub.lock();
    ws.pending_link = link.issue;
    open_pending_link(hub, &mut ws)
}

/// Select the pending linked issue if the repo has loaded: an exact short
/// ID, else a unique prefix.
pub(super) fn open_pending_link(
    hub: &SharedHub,
    ws: &mut Workspace,
) -> std::result::Result<(), String> {
    let Some(r) = &ws.repo else { return Ok(()) };
    if !r.loaded {
        return Ok(());
    }
    let Some(want) = ws.pending_link.take() else {
        return Ok(());
    };
    let id = if let Some(s) = r.index.by_id(&want) {
        s.id.clone()
    } else {
        let mut m = r.index.iter().filter(|s| s.id.starts_with(&want));
        match (m.next(), m.next()) {
            (Some(s), None) => s.id.clone(),
            (None, _) => return Err(format!("no issue {want} in {}", r.repo.label)),
            (Some(_), Some(_)) => return Err(format!("issue ID {want} is ambiguous")),
        }
    };
    select_locked(hub, ws, Some(id.clone()));
    hub.emit(events::FOCUS_DETAIL, &id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{fx, id_of};
    use super::*;

    #[test]
    fn links_select_and_focus() {
        let f = fx(&["one"]);
        let id = id_of(&f, "one");
        let label = f.hub.lock().repos[0].label.clone();
        open_url(
            &f.hub,
            &Link {
                repo: label,
                issue: Some(id.clone()),
            }
            .to_url(),
        );
        assert_eq!(
            f.rec.last(events::FOCUS_DETAIL).unwrap(),
            serde_json::json!(id)
        );
        assert_eq!(
            f.hub.lock().doc.as_ref().map(|d| d.id.as_str()),
            Some(id.as_str())
        );
    }
}
