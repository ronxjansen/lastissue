//! `lastissue comment …`: args → `lastissue::ops` → output.

use super::common::{self, Ctx};
use super::issue::comment_extras;
use super::{BodyArgs, CommentCmd};
use crate::error::Result;
use crate::format;
use crate::model;
use lastissue::ops::{self, CommentRef};

pub fn run(ctx: &mut Ctx, cmd: CommentCmd) -> Result<()> {
    match cmd {
        CommentCmd::Create { id, body, reply_to } => create(ctx, &id, &body, reply_to.as_deref()),
        CommentCmd::List { id } => list(ctx, &id),
        CommentCmd::Update { comment, body } => update(ctx, &comment, &body),
        CommentCmd::Delete { comment } => {
            let c = ops::find_comment(&ctx.store, &comment)?;
            let o = ops::delete_comment(&ctx.store, &ctx.me, c)?;
            ctx.print_indices(&o.issue, &o.msgs, &o.appended);
            Ok(())
        }
    }
}

fn print_comment(ctx: &mut Ctx, c: &CommentRef, idx: usize) {
    let nums = model::comment_numbers(&c.msgs);
    let text = format::message_with(&c.msgs[idx], &comment_extras(&c.issue, &c.msgs, &nums, idx));
    let painted = format::paint(&text, ctx.color, false);
    ctx.print(&painted);
}

fn create(ctx: &mut Ctx, id: &str, body: &BodyArgs, reply_to: Option<&str>) -> Result<()> {
    // Validate the target before asking for a body (which may open $EDITOR).
    let target = ops::comment_target(&ctx.store, id, reply_to)?;
    let text = common::required_body(body, "")?;
    let c = ops::add_comment(&ctx.store, &ctx.me, &target, &text)?;
    print_comment(ctx, &c, c.idx);
    Ok(())
}

fn list(ctx: &mut Ctx, id: &str) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let msgs = ctx.store.read(&r)?;
    let c = CommentRef {
        issue: r,
        msgs,
        idx: 0,
    };
    let nums = model::comment_numbers(&c.msgs);
    for (i, n) in nums.iter().enumerate() {
        if n.is_some() {
            print_comment(ctx, &c, i);
        }
    }
    Ok(())
}

fn update(ctx: &mut Ctx, q: &str, body: &BodyArgs) -> Result<()> {
    let c = ops::find_comment(&ctx.store, q)?;
    let text = common::required_body(body, &c.msgs[c.idx].body)?;
    let c = ops::edit_comment(&ctx.store, &ctx.me, c, &text)?;
    print_comment(ctx, &c, c.idx);
    Ok(())
}
