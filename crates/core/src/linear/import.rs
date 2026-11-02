//! The Linear import (SPEC §6.5) without the prompts: what is already here,
//! where new issues go, and writing one fetched issue at a time. A
//! front-end drives it page by page (so it can report progress) and owns
//! every question asked of the user.

use super::map::{self, Mapped};
use super::{Source, SourceKind};
use crate::error::{Error, Result};
use crate::model::{self, Actor};
use crate::store::mbox::Msg;
use crate::store::{self, IssueRef, Project, Store};
use serde_json::Value;
use std::collections::HashMap;

/// What to do with issues imported before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Existing {
    /// Leave them untouched.
    #[default]
    Skip,
    /// Linear wins on the fields it owns; local-only headers and comments stay.
    Update,
}

/// Where new issues go.
#[derive(Debug, Clone)]
pub enum Destination {
    /// A new project with this slug, created lazily; a local project with the
    /// same slug is reused so re-runs don't pile up folders.
    New(String),
    Existing(Project),
    /// The root, outside any project.
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// What's already in the store: imported issues by Message-ID and by
/// `X-Linear-Id`, plus every short ID (for collision-free new IDs).
#[derive(Debug, Clone, Default)]
pub struct Index {
    by_mid: HashMap<String, IssueRef>,
    by_linear: HashMap<String, IssueRef>,
    ids: Vec<String>,
}

impl Index {
    pub fn scan(store: &Store) -> Result<Index> {
        let mut ix = Index::default();
        for r in store.issues()? {
            let head = store.read_head(&r)?;
            if let Some(l) = head.get("X-Linear-Id") {
                ix.by_linear.insert(l.to_string(), r.clone());
                ix.by_mid.insert(head.message_id().to_string(), r.clone());
            }
            ix.ids.push(r.id.clone());
        }
        Ok(ix)
    }

    /// How many of these `(uuid, identifier)` pairs were imported before.
    pub fn already_imported(&self, ids: &[(String, String)]) -> usize {
        ids.iter()
            .filter(|(_, ident)| self.by_linear.contains_key(ident))
            .count()
    }

    fn find(&self, m: &Mapped) -> Option<IssueRef> {
        self.by_mid
            .get(m.issue.message_id())
            .or_else(|| self.by_linear.get(&m.identifier))
            .cloned()
    }
}

/// One import run.
#[derive(Debug)]
pub struct Import {
    into: Destination,
    /// Resolved destination project (after lazy creation).
    project: Option<Project>,
    existing: Existing,
    actor: Actor,
    index: Index,
    tally: Tally,
    touched: Vec<(IssueRef, Msg)>,
}

impl Import {
    /// `from` is the importing user; messages get `Sender: lastissue-import`.
    pub fn new(index: Index, into: Destination, existing: Existing, from: String) -> Import {
        Import {
            into,
            project: None,
            existing,
            actor: Actor {
                from,
                sender: Some(map::SENDER.into()),
            },
            index,
            tally: Tally::default(),
            touched: Vec::new(),
        }
    }

    pub fn tally(&self) -> Tally {
        self.tally
    }

    /// The destination project, once something was written to it.
    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    /// Issues imported or updated, with their new message #1, in order.
    pub fn touched(&self) -> &[(IssueRef, Msg)] {
        &self.touched
    }

    fn destination(&mut self, store: &Store) -> Result<Option<Project>> {
        if self.project.is_none() {
            self.project = match &self.into {
                Destination::None => return Ok(None),
                Destination::Existing(p) => Some(p.clone()),
                Destination::New(slug) => {
                    let projects = store.projects()?;
                    if let Some(p) = projects.into_iter().find(|p| &p.slug == slug) {
                        Some(p)
                    } else {
                        let existing: Vec<String> =
                            store.projects()?.into_iter().map(|p| p.id).collect();
                        let (_, id) = store::fresh_id(&existing);
                        let p = Project {
                            dir: store.root.join(format!("{slug}-{id}")),
                            id,
                            slug: slug.clone(),
                        };
                        std::fs::create_dir(&p.dir)?;
                        Some(p)
                    }
                }
            };
        }
        Ok(self.project.clone())
    }

    /// Import one fetched Linear issue node.
    pub fn node(&mut self, store: &Store, node: &Value, src: &Source) -> Result<()> {
        let m = map::issue(node, src);
        let found = self.index.find(&m);
        match (found, self.existing) {
            (Some(_), Existing::Skip) => self.tally.skipped += 1,
            (Some(r), Existing::Update) => {
                if let Some(head) = update(store, &r, m, &self.actor)? {
                    self.tally.updated += 1;
                    self.touched.push((r, head));
                } else {
                    self.tally.skipped += 1;
                }
            }
            (None, _) => {
                let project = self.destination(store)?;
                let mut issue = m.issue;
                issue.set_opt("List-Id", project.as_ref().map(Project::list_id).as_deref());
                let id = store::short_id_for(m.ulid, &self.index.ids).ok_or_else(|| {
                    Error::validation("id-collision")
                        .with("Linear-Id", m.identifier.clone())
                        .with("Message-ID", issue.message_id())
                })?;
                let slug = store::slugify(issue.get("Subject").unwrap_or(""), 4, "issue");
                let path = store.issue_path(project.as_ref(), &slug, &id);
                self.index.ids.push(id.clone());
                let r = IssueRef { id, path, project };
                let mut msgs = vec![issue];
                msgs.extend(m.comments);
                store.write(&r.path, &msgs)?;
                self.tally.imported += 1;
                self.touched.push((r, msgs.swap_remove(0)));
            }
        }
        Ok(())
    }
}

/// `--existing update`: Linear wins on the headers it owns and the body;
/// Linear comments are appended or edited in place; everything local stays.
/// Appends one update event. Returns the new head if anything changed.
fn update(store: &Store, r: &IssueRef, m: Mapped, actor: &Actor) -> Result<Option<Msg>> {
    let mut msgs = store.read(r)?;
    let before = msgs[0].clone();
    for h in map::OWNED {
        msgs[0].set_opt(h, m.issue.get(h));
    }
    let mut lines = model::diff_lines(&before, &msgs[0]);
    if msgs[0].body != m.issue.body {
        msgs[0].body.clone_from(&m.issue.body);
        msgs[0].set("X-Edited", &model::date_header(model::now()));
        msgs[0].set("X-Edited-By", map::SENDER);
        lines.push("body: updated from Linear".into());
    }

    let (mut added, mut edited) = (0, 0);
    for c in m.comments {
        let cid = c.get("X-Linear-Comment-Id").unwrap_or("");
        match msgs
            .iter_mut()
            .skip(1)
            .find(|x| x.get("X-Linear-Comment-Id") == Some(cid))
        {
            Some(x) if x.body != c.body => {
                x.body.clone_from(&c.body);
                let when = c
                    .get("X-Edited")
                    .map_or_else(|| model::date_header(model::now()), str::to_string);
                x.set("X-Edited", &when);
                edited += 1;
            }
            Some(_) => {}
            None => {
                msgs.push(c);
                added += 1;
            }
        }
    }
    if added > 0 {
        lines.push(format!("comments: {added} added from Linear"));
    }
    if edited > 0 {
        lines.push(format!("comments: {edited} updated from Linear"));
    }
    if lines.is_empty() {
        return Ok(None);
    }
    let ev = model::event(actor, &msgs[0], "update", &lines);
    msgs.push(ev);
    store.write(&r.path, &msgs)?;
    Ok(Some(msgs.swap_remove(0)))
}

/// Pick the one source `q` names. `flag` names the input in errors.
pub fn find_source(list: &[Source], q: &str, flag: &str) -> Result<Source> {
    let mut hits: Vec<Source> = list.iter().filter(|s| s.matches(q)).cloned().collect();
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => {
            let mut names: Vec<String> = list
                .iter()
                .map(|s| match s.kind {
                    SourceKind::Project => s.name.clone(),
                    SourceKind::Team => s.key.clone(),
                })
                .collect();
            names.sort();
            Err(Error::not_found("linear-not-found")
                .with("Flag", flag)
                .with("Value", q)
                .with("Valid", names.join(", ")))
        }
        _ => Err(
            Error::ambiguous(hits.iter().map(|s| s.id.clone()).collect())
                .with("Hint", format!("pass the UUID to {flag}")),
        ),
    }
}
