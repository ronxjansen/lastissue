//! `lastissue project …`: args → `lastissue::ops` / `query` → output.

use super::ProjectCmd;
use super::common::Ctx;
use crate::error::Result;
use crate::format;
use crate::store::Project;
use lastissue::{ops, query};

pub fn run(ctx: &mut Ctx, cmd: ProjectCmd) -> Result<()> {
    let text = match cmd {
        ProjectCmd::Create { name } => block(ctx, &ops::create_project(&ctx.store, &name)?, &[]),
        ProjectCmd::List => list(ctx)?,
        ProjectCmd::Rename { project, name } => {
            block(ctx, &ops::rename_project(&ctx.store, &project, &name)?, &[])
        }
        ProjectCmd::Delete { project, force } => block(
            ctx,
            &ops::delete_project(&ctx.store, &project, force)?,
            &[("X-Deleted", "yes".into())],
        ),
    };
    ctx.paint(&text, true);
    Ok(())
}

fn block(ctx: &Ctx, p: &Project, extra: &[(&str, String)]) -> String {
    let mut fields = vec![
        ("List-Id", p.list_id()),
        ("X-Short-Id", p.id.clone()),
        ("X-Path", format!("{}/", ctx.store.rel(&p.dir))),
    ];
    fields.extend_from_slice(extra);
    format::block(&fields)
}

fn list(ctx: &Ctx) -> Result<String> {
    let mut blocks = vec![format::block(&[(
        "X-Root",
        ctx.store.root.display().to_string(),
    )])];
    for p in ctx.store.projects()? {
        let n = query::project_counts(&ctx.store, &p)?;
        blocks.push(block(
            ctx,
            &p,
            &[
                ("X-Issue-Count", n.issues.to_string()),
                ("X-Open-Count", n.open.to_string()),
            ],
        ));
    }
    Ok(blocks.join("\n"))
}
