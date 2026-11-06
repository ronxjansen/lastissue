//! `lastissue comment …`: args → `lastissue::ops` → output.

use super::common::{self, Ctx};
use super::{BodyArgs, CommentCmd};
use crate::error::Result;
use crate::format;
use crate::model::{self, Activity};
use crate::store::{IssueRef, mbox::Msg};
use lastissue::ops;

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

/// Print the comments whose message index passes `keep`.
fn print_comments(ctx: &mut Ctx, r: &IssueRef, msgs: &[Msg], keep: impl Fn(usize) -> bool) {
    for a in model::activity(msgs) {
        if let Activity::Comment { index, msg, .. } = a
            && keep(index)
        {
            let text = format::message_with(msg, &format::comment_headers(r, &msgs[0], &a));
            ctx.paint(&text, false);
        }
    }
}

fn create(ctx: &mut Ctx, id: &str, body: &BodyArgs, reply_to: Option<&str>) -> Result<()> {
    // Validate the target before asking for a body (which may open $EDITOR).
    let target = ops::comment_target(&ctx.store, id, reply_to)?;
    let text = common::required_body(body, "")?;
    let c = ops::add_comment(&ctx.store, &ctx.me, &target, &text)?;
    print_comments(ctx, &c.issue, &c.msgs, |i| i == c.idx);
    Ok(())
}

fn list(ctx: &mut Ctx, id: &str) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let msgs = ctx.store.read(&r)?;
    print_comments(ctx, &r, &msgs, |_| true);
    Ok(())
}

fn update(ctx: &mut Ctx, q: &str, body: &BodyArgs) -> Result<()> {
    let c = ops::find_comment(&ctx.store, q)?;
    let text = common::required_body(body, &c.msgs[c.idx].body)?;
    let c = ops::edit_comment(&ctx.store, &ctx.me, c, &text)?;
    print_comments(ctx, &c.issue, &c.msgs, |i| i == c.idx);
    Ok(())
}
