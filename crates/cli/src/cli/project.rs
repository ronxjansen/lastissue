//! `lastissue project …`: args → `lastissue::ops` / `query` → output.

use super::ProjectCmd;
use super::common::Ctx;
use crate::error::Result;
use crate::format;
use crate::store::Project;
use lastissue::{ops, query};

pub fn run(ctx: &mut Ctx, cmd: ProjectCmd) -> Result<()> {
    let text = match cmd {
        ProjectCmd::Create { name } => block(ctx, &ops::create_project(&ctx.store, &name)?, vec![]),
        ProjectCmd::List => list(ctx)?,
        ProjectCmd::Rename { project, name } => block(
            ctx,
            &ops::rename_project(&ctx.store, &project, &name)?,
            vec![],
        ),
        ProjectCmd::Delete { project, force } => block(
            ctx,
            &ops::delete_project(&ctx.store, &project, force)?,
            vec![("X-Deleted".into(), "yes".into())],
        ),
    };
    ctx.print(&text);
    Ok(())
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

fn list(ctx: &Ctx) -> Result<String> {
    let mut blocks = vec![format::paint(
        &format::block(&[("X-Root".into(), ctx.store.root.display().to_string())]),
        ctx.color,
        true,
    )];
    for p in ctx.store.projects()? {
        let n = query::project_counts(&ctx.store, &p)?;
        blocks.push(block(
            ctx,
            &p,
            vec![
                ("X-Issue-Count".into(), n.issues.to_string()),
                ("X-Open-Count".into(), n.open.to_string()),
            ],
        ));
    }
    Ok(blocks.join("\n"))
}
