//! `lastissue project …`

use super::ProjectCmd;
use super::common::Ctx;
use super::issue::state_of;
use crate::error::{Error, Result};
use crate::format;
use crate::store::{self, Project};
use std::fs;

pub fn run(ctx: &mut Ctx, cmd: ProjectCmd) -> Result<()> {
    match cmd {
        ProjectCmd::Create { name } => create(ctx, &name),
        ProjectCmd::List => list(ctx),
        ProjectCmd::Rename { project, name } => rename(ctx, &project, &name),
        ProjectCmd::Delete { project, force } => delete(ctx, &project, force),
    }
}

fn block(ctx: &Ctx, p: &Project, extra: Vec<(String, String)>) -> String {
    let mut fields = vec![
        ("List-Id".to_string(), p.list_id()),
        ("X-Short-Id".to_string(), p.id.clone()),
        ("X-Path".to_string(), format!("{}/", ctx.store.rel(&p.dir))),
    ];
    fields.extend(extra);
    format::paint(&format::block(&fields), ctx.color, true)
}

fn create(ctx: &mut Ctx, name: &str) -> Result<()> {
    let slug = store::slugify(name, 2, "project");
    let existing: Vec<String> = ctx.store.projects()?.into_iter().map(|p| p.id).collect();
    let (_, id) = store::fresh_id(&existing);
    let p = Project {
        dir: ctx.store.root.join(format!("{slug}-{id}")),
        id,
        slug,
    };
    fs::create_dir(&p.dir)?;
    let text = block(ctx, &p, vec![]);
    ctx.print(&text);
    Ok(())
}

fn list(ctx: &mut Ctx) -> Result<()> {
    let mut blocks = vec![format::paint(
        &format::block(&[("X-Root".into(), ctx.store.root.display().to_string())]),
        ctx.color,
        true,
    )];
    for p in ctx.store.projects()? {
        let issues = ctx.store.project_issues(&p)?;
        let mut open = 0;
        for r in &issues {
            if state_of(&ctx.store.read_head(r)?).is_open() {
                open += 1;
            }
        }
        blocks.push(block(
            ctx,
            &p,
            vec![
                ("X-Issue-Count".into(), issues.len().to_string()),
                ("X-Open-Count".into(), open.to_string()),
            ],
        ));
    }
    ctx.print(&blocks.join("\n"));
    Ok(())
}

fn rename(ctx: &mut Ctx, project: &str, name: &str) -> Result<()> {
    let old = ctx.store.resolve_project(project)?;
    let slug = store::slugify(name, 2, "project");
    let new = Project {
        dir: ctx.store.root.join(format!("{slug}-{}", old.id)),
        id: old.id.clone(),
        slug,
    };
    if new.dir != old.dir {
        if new.dir.exists() {
            return Err(Error::validation("project-exists").with("Path", ctx.store.rel(&new.dir)));
        }
        fs::rename(&old.dir, &new.dir)?;
    }
    // Keep List-Id display names in step with the folder (the source of truth).
    for r in ctx.store.project_issues(&new)? {
        let mut msgs = ctx.store.read(&r)?;
        if msgs[0].get("List-Id") != Some(new.list_id().as_str()) {
            msgs[0].set("List-Id", &new.list_id());
            ctx.store.write(&r.path, &msgs)?;
        }
    }
    let text = block(ctx, &new, vec![]);
    ctx.print(&text);
    Ok(())
}

fn delete(ctx: &mut Ctx, project: &str, force: bool) -> Result<()> {
    let p = ctx.store.resolve_project(project)?;
    let count = fs::read_dir(&p.dir)?.count();
    if count > 0 && !force {
        let issues = ctx.store.project_issues(&p)?.len();
        return Err(Error::validation("project-not-empty")
            .with("Project", p.dir_name())
            .with("Issues", issues.to_string())
            .with(
                "Hint",
                format!(
                    "move the issues out first, or: lastissue project delete {} --force",
                    p.id
                ),
            ));
    }
    fs::remove_dir_all(&p.dir)?;
    let text = block(ctx, &p, vec![("X-Deleted".into(), "yes".into())]);
    ctx.print(&text);
    Ok(())
}
