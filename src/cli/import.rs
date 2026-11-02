//! `lastissue import linear`. The one place with interactive prompts (§6.2):
//! on a TTY every missing value is asked for, otherwise it fails fast
//! naming the flag. Prompts only fill the same values as the flags.

use super::common::Ctx;
use super::issue::{labels_of, priority_of, state_of};
use super::{Existing, ImportCmd, LinearArgs};
use crate::error::{Error, Result};
use crate::format;
use crate::linear::{Client, Source, SourceKind, map};
use crate::model::{self, Actor};
use crate::store::{self, IssueRef, Project, mbox::Msg};
use std::collections::HashMap;
use std::io::IsTerminal;

pub fn run(ctx: &mut Ctx, cmd: ImportCmd) -> Result<()> {
    match cmd {
        ImportCmd::Linear(a) => linear(ctx, *a),
    }
}

// ---------------------------------------------------------------- prompts

fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn missing(flag: &str) -> Error {
    Error::usage("missing-value").with("Flag", flag).with(
        "Hint",
        "not a terminal, so nothing can be prompted for; pass the flag",
    )
}

fn prompt_err(e: inquire::InquireError) -> Error {
    match e {
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            Error::usage("aborted")
        }
        e => Error::usage("prompt-failed").with("Message", e.to_string()),
    }
}

fn select(msg: &str, options: Vec<String>) -> Result<usize> {
    if options.is_empty() {
        return Err(Error::not_found("nothing-to-choose").with("Prompt", msg));
    }
    Ok(inquire::Select::new(msg, options)
        .with_page_size(15)
        .raw_prompt()
        .map_err(prompt_err)?
        .index)
}

// ---------------------------------------------------------------- flow

/// Where new issues go. `New` is created lazily, and reuses a local project
/// with the same slug so re-runs don't pile up folders.
enum Into {
    New(String),
    Existing(Project),
    None,
}

#[derive(Default)]
struct Tally {
    imported: usize,
    updated: usize,
    skipped: usize,
}

impl Tally {
    fn fields(&self) -> Vec<(String, String)> {
        vec![
            ("X-Imported".into(), self.imported.to_string()),
            ("X-Updated".into(), self.updated.to_string()),
            ("X-Skipped".into(), self.skipped.to_string()),
        ]
    }
}

fn linear(ctx: &mut Ctx, a: LinearArgs) -> Result<()> {
    let tty = interactive();

    // API key
    let key = match a
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        Some(k) => k.to_string(),
        None if tty => inquire::Password::new("Linear API key:")
            .without_confirmation()
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .prompt()
            .map_err(prompt_err)?,
        None => return Err(missing("--api-key or LINEAR_API_KEY")),
    };
    let client = Client::new(&key);
    let viewer = client.viewer()?;
    eprintln!(
        "Authenticated as {} @ {}",
        if viewer.name.is_empty() {
            &viewer.email
        } else {
            &viewer.name
        },
        viewer.org
    );

    // Source
    let src = if let Some(q) = &a.project {
        find_source(client.projects()?, q, "--project")?
    } else if let Some(q) = &a.team {
        find_source(client.teams()?, q, "--team")?
    } else if tty {
        let by = select("Import by:", vec!["Project".into(), "Team".into()])?;
        let mut list = if by == 0 {
            client.projects()?
        } else {
            client.teams()?
        };
        list.retain(|s| a.all || !s.closed);
        list.sort_by_key(|s| s.name.to_lowercase());
        let i = select(
            if by == 0 { "Project:" } else { "Team:" },
            list.iter().map(Source::label).collect(),
        )?;
        list.swap_remove(i)
    } else {
        return Err(missing("--project <name|slug|uuid> or --team <key>"));
    };

    // Destination
    let new_slug = store::slugify(&src.name, 2, "linear");
    let into = if let Some(p) = &a.into {
        Into::Existing(ctx.store.resolve_project(p)?)
    } else if a.no_project {
        Into::None
    } else if tty {
        let projects = ctx.store.projects()?;
        let mut opts = vec![format!("New \"{new_slug}\"")];
        if !projects.is_empty() {
            opts.push("Existing…".into());
        }
        opts.push("No project".into());
        match select("Import into:", opts.clone())? {
            0 => Into::New(new_slug),
            i if i == opts.len() - 1 => Into::None,
            _ => {
                let i = select("Project:", projects.iter().map(|p| p.dir_name()).collect())?;
                Into::Existing(projects[i].clone())
            }
        }
    } else {
        Into::New(new_slug)
    };

    // What's already here: Message-ID and X-Linear-Id → file.
    let mut by_mid: HashMap<String, IssueRef> = HashMap::new();
    let mut by_linear: HashMap<String, IssueRef> = HashMap::new();
    let mut ids: Vec<String> = Vec::new();
    for r in ctx.store.issues()? {
        let head = ctx.store.read_head(&r)?;
        if let Some(l) = head.get("X-Linear-Id") {
            by_linear.insert(l.to_string(), r.clone());
            by_mid.insert(head.message_id().to_string(), r.clone());
        }
        ids.push(r.id.clone());
    }

    // Pre-pass (only when a prompt needs it): count + already-imported matches.
    let mut count = None;
    let mut existing = a.existing;
    if tty && (!a.yes || existing.is_none()) {
        let all = client.issue_ids(&src)?;
        let matches = all
            .iter()
            .filter(|(_, ident)| by_linear.contains_key(ident))
            .count();
        count = Some(all.len());
        if existing.is_none() && matches > 0 {
            let i = select(
                &format!("{matches} already imported:"),
                vec!["Skip".into(), "Update".into()],
            )?;
            existing = Some(if i == 0 {
                Existing::Skip
            } else {
                Existing::Update
            });
        }
    }
    let existing = existing.unwrap_or(Existing::Skip);

    if !a.yes {
        if !tty {
            return Err(missing("--yes"));
        }
        let what = match count {
            Some(n) => format!("{n} issues"),
            None => "issues".into(),
        };
        let dest = match &into {
            Into::New(s) => format!("new project \"{s}\""),
            Into::Existing(p) => format!("project {}", p.dir_name()),
            Into::None => "the root (no project)".into(),
        };
        let ok = inquire::Confirm::new(&format!("Import {what} from {} into {dest}?", src.label()))
            .with_default(false)
            .prompt()
            .map_err(prompt_err)?;
        if !ok {
            return Err(Error::usage("aborted"));
        }
    }

    // Stream: fetch a page, write a page.
    let actor = Actor {
        from: ctx.actor()?.from.clone(),
        sender: Some(map::SENDER.into()),
    };
    let mut run = Run {
        into,
        project: None,
        existing,
        actor,
        ids,
        tally: Tally::default(),
        touched: Vec::new(),
    };
    let mut after: Option<String> = None;
    let mut page = 0;
    loop {
        let res = client
            .issue_page(&src, after.as_deref())
            .and_then(|(nodes, next)| {
                for n in &nodes {
                    let m = map::issue(n, &src);
                    let found = by_mid
                        .get(m.issue.message_id())
                        .or_else(|| by_linear.get(&m.identifier))
                        .cloned();
                    run.one(ctx, m, found)?;
                }
                Ok((nodes.len(), next))
            });
        let (n, next) = match res {
            Ok(x) => x,
            Err(mut e) => {
                e.fields.extend(
                    run.tally
                        .fields()
                        .into_iter()
                        .map(|(k, v)| (k.trim_start_matches("X-").to_string(), v)),
                );
                e.fields.retain(|(k, _)| k != "Hint");
                return Err(e.with(
                    "Hint",
                    "what was written is kept; re-run with --existing skip to continue",
                ));
            }
        };
        page += 1;
        let t = &run.tally;
        eprintln!(
            "Page {page}: {n} issues ({} imported, {} updated, {} skipped)",
            t.imported, t.updated, t.skipped
        );
        match next {
            Some(c) => after = Some(c),
            None => break,
        }
    }

    // Summary
    let mut fields = vec![("X-Root".into(), ctx.store.root.display().to_string())];
    fields.push(("X-Source".into(), format!("linear {}", src.label())));
    if let Some(p) = &run.project {
        fields.push(("List-Id".into(), p.list_id()));
    }
    fields.extend(run.tally.fields());
    let mut out = format::block(&fields);
    for (r, head) in &run.touched {
        out.push_str(&format::brief_line(
            &r.id,
            state_of(head).as_str(),
            priority_of(head).map(|p| p.as_str()),
            &labels_of(head),
            head.get("Subject").unwrap_or(""),
            ctx.color,
        ));
    }
    ctx.print(&out);
    Ok(())
}

fn find_source(list: Vec<Source>, q: &str, flag: &str) -> Result<Source> {
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

struct Run {
    into: Into,
    /// Resolved destination project (after lazy creation).
    project: Option<Project>,
    existing: Existing,
    actor: Actor,
    ids: Vec<String>,
    tally: Tally,
    touched: Vec<(IssueRef, Msg)>,
}

impl Run {
    fn destination(&mut self, ctx: &Ctx) -> Result<Option<Project>> {
        if self.project.is_none() {
            self.project = match &self.into {
                Into::None => return Ok(None),
                Into::Existing(p) => Some(p.clone()),
                Into::New(slug) => {
                    let projects = ctx.store.projects()?;
                    match projects.into_iter().find(|p| &p.slug == slug) {
                        Some(p) => Some(p),
                        None => {
                            let existing: Vec<String> =
                                ctx.store.projects()?.into_iter().map(|p| p.id).collect();
                            let (_, id) = store::fresh_id(&existing);
                            let p = Project {
                                dir: ctx.store.root.join(format!("{slug}-{id}")),
                                id,
                                slug: slug.clone(),
                            };
                            std::fs::create_dir(&p.dir)?;
                            Some(p)
                        }
                    }
                }
            };
        }
        Ok(self.project.clone())
    }

    fn one(&mut self, ctx: &Ctx, m: map::Mapped, found: Option<IssueRef>) -> Result<()> {
        match (found, self.existing) {
            (Some(_), Existing::Skip) => self.tally.skipped += 1,
            (Some(r), Existing::Update) => {
                if let Some(head) = update(ctx, &r, m, &self.actor)? {
                    self.tally.updated += 1;
                    self.touched.push((r, head));
                } else {
                    self.tally.skipped += 1;
                }
            }
            (None, _) => {
                let project = self.destination(ctx)?;
                let mut issue = m.issue;
                issue.set_opt("List-Id", project.as_ref().map(|p| p.list_id()).as_deref());
                let id = store::short_id_for(m.ulid, &self.ids).ok_or_else(|| {
                    Error::validation("id-collision")
                        .with("Linear-Id", m.identifier.clone())
                        .with("Message-ID", issue.message_id())
                })?;
                let slug = store::slugify(issue.get("Subject").unwrap_or(""), 4, "issue");
                let path = ctx.store.issue_path(project.as_ref(), &slug, &id);
                self.ids.push(id.clone());
                let r = IssueRef { id, path, project };
                let mut msgs = vec![issue];
                msgs.extend(m.comments);
                ctx.store.write(&r.path, &msgs)?;
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
fn update(ctx: &Ctx, r: &IssueRef, m: map::Mapped, actor: &Actor) -> Result<Option<Msg>> {
    let mut msgs = ctx.store.read(r)?;
    let before = msgs[0].clone();
    for h in map::OWNED {
        msgs[0].set_opt(h, m.issue.get(h));
    }
    let mut lines = model::diff_lines(&before, &msgs[0]);
    if msgs[0].body != m.issue.body {
        msgs[0].body = m.issue.body.clone();
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
                x.body = c.body.clone();
                let when = c
                    .get("X-Edited")
                    .map(str::to_string)
                    .unwrap_or_else(|| model::date_header(model::now()));
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
    ctx.store.write(&r.path, &msgs)?;
    Ok(Some(msgs.swap_remove(0)))
}
