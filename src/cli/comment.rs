//! `lastissue comment …`

use super::CommentCmd;
use super::common::{self, Ctx};
use super::issue::{comment_extras, comment_numbers};
use crate::error::{Error, Result};
use crate::format;
use crate::model;
use crate::store::IssueRef;
use crate::store::mbox::{self, Msg};

pub fn run(ctx: &mut Ctx, cmd: CommentCmd) -> Result<()> {
    match cmd {
        CommentCmd::Create { id, body, reply_to } => create(ctx, &id, &body, reply_to.as_deref()),
        CommentCmd::List { id } => list(ctx, &id),
        CommentCmd::Update { comment, body } => update(ctx, &comment, &body),
        CommentCmd::Delete { comment } => delete(ctx, &comment),
    }
}

/// A resolved comment: the issue, its messages, and the comment's index.
struct Found {
    r: IssueRef,
    msgs: Vec<Msg>,
    idx: usize,
}

fn usage_hint() -> &'static str {
    "address a comment as <issue>/<n> (n from 1, see: lastissue comment list <issue>) or by Message-ID prefix"
}

/// `<issue>/<n>`, or a comment Message-ID (ULID) prefix.
fn resolve(ctx: &Ctx, q: &str) -> Result<Found> {
    let q = q.trim();
    if let Some((issue, n)) = q.rsplit_once('/') {
        if issue.is_empty() {
            return Err(Error::usage("invalid-comment-ref")
                .with("Value", q)
                .with("Hint", usage_hint()));
        }
        let n: usize = n.parse().map_err(|_| {
            Error::usage("invalid-comment-ref")
                .with("Value", q)
                .with("Hint", usage_hint())
        })?;
        let r = ctx.store.resolve_issue(issue)?;
        let msgs = ctx.store.read(&r)?;
        let nums = comment_numbers(&msgs);
        let count = nums.iter().flatten().count();
        let idx = nums.iter().position(|x| *x == Some(n)).ok_or_else(|| {
            Error::not_found("comment-not-found")
                .with("Comment", format!("{}/{n}", r.id))
                .with(
                    "Valid",
                    if count == 0 {
                        "(no comments)".to_string()
                    } else {
                        format!("1..{count}")
                    },
                )
        })?;
        return Ok(Found { r, msgs, idx });
    }
    let needle =
        model::normalize_id_input(q.trim_start_matches('<').split('@').next().unwrap_or(""));
    if needle.is_empty() || !model::is_short_id(&needle) {
        return Err(Error::usage("invalid-comment-ref")
            .with("Value", q)
            .with("Hint", usage_hint()));
    }
    let mut hits: Vec<(IssueRef, Vec<Msg>, usize, String)> = Vec::new();
    for r in ctx.store.issues()? {
        let msgs = ctx.store.read(&r)?;
        let nums = comment_numbers(&msgs);
        for (i, n) in nums.iter().enumerate() {
            let Some(n) = n else { continue };
            if mbox::ulid_of(msgs[i].message_id())
                .to_lowercase()
                .starts_with(&needle)
            {
                hits.push((r.clone(), msgs.clone(), i, format!("{}/{n}", r.id)));
            }
        }
    }
    match hits.len() {
        0 => Err(Error::not_found("comment-not-found")
            .with("Comment", q)
            .with("Hint", usage_hint())),
        1 => {
            let (r, msgs, idx, _) = hits.remove(0);
            Ok(Found { r, msgs, idx })
        }
        _ => Err(Error::ambiguous(hits.into_iter().map(|h| h.3).collect())),
    }
}

fn print_comment(ctx: &mut Ctx, f: &Found, idx: usize) {
    let nums = comment_numbers(&f.msgs);
    let text = format::message_with(&f.msgs[idx], &comment_extras(&f.r, &f.msgs, &nums, idx));
    let painted = format::paint(&text, ctx.color, false);
    ctx.print(&painted);
}

fn create(ctx: &mut Ctx, id: &str, body: &super::BodyArgs, reply_to: Option<&str>) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let parent_id = match reply_to {
        Some(rt) => {
            // Allow "/n" or "n" as shorthand for a comment on this same issue.
            let rt = rt.trim();
            let rt = if rt.starts_with('/') || rt.chars().all(|c| c.is_ascii_digit()) {
                format!("{}/{}", r.id, rt.trim_start_matches('/'))
            } else {
                rt.to_string()
            };
            let f = resolve(ctx, &rt)?;
            if f.r.path != r.path {
                return Err(Error::validation("reply-to-other-issue").with(
                    "Hint",
                    format!("--reply-to must reference a comment on {}", r.id),
                ));
            }
            Some(f.msgs[f.idx].message_id().to_string())
        }
        None => None,
    };
    let text = common::required_body(body, "")?;
    let mut msgs = ctx.store.read(&r)?;
    let parent = parent_id
        .as_deref()
        .and_then(|p| msgs.iter().find(|m| m.message_id() == p))
        .cloned();
    let c = model::comment(ctx.actor()?, &msgs[0], parent.as_ref(), &text);
    msgs.push(c);
    ctx.store.write(&r.path, &msgs)?;
    let idx = msgs.len() - 1;
    print_comment(ctx, &Found { r, msgs, idx }, idx);
    Ok(())
}

fn list(ctx: &mut Ctx, id: &str) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let msgs = ctx.store.read(&r)?;
    let f = Found { r, msgs, idx: 0 };
    let nums = comment_numbers(&f.msgs);
    for (i, n) in nums.iter().enumerate() {
        if n.is_some() {
            print_comment(ctx, &f, i);
        }
    }
    Ok(())
}

fn update(ctx: &mut Ctx, q: &str, body: &super::BodyArgs) -> Result<()> {
    let mut f = resolve(ctx, q)?;
    let current = f.msgs[f.idx].body.clone();
    let text = mbox::normalize_body(&common::required_body(body, &current)?);
    if text != current {
        let actor = ctx.actor()?.clone();
        let m = &mut f.msgs[f.idx];
        m.body = text;
        model::mark_edited(m, &actor);
        ctx.store.write(&f.r.path, &f.msgs)?;
    }
    let idx = f.idx;
    print_comment(ctx, &f, idx);
    Ok(())
}

fn delete(ctx: &mut Ctx, q: &str) -> Result<()> {
    let mut f = resolve(ctx, q)?;
    let nums = comment_numbers(&f.msgs);
    let n = nums[f.idx].unwrap_or(0);
    let removed = f.msgs.remove(f.idx);
    let line = format!(
        "deleted: /{n} {} by {}",
        removed.message_id(),
        removed.get("From").unwrap_or("(unknown)")
    );
    let ev = model::event(ctx.actor()?, &f.msgs[0], "comment-deleted", &[line]);
    f.msgs.push(ev);
    ctx.store.write(&f.r.path, &f.msgs)?;
    let last = f.msgs.len() - 1;
    ctx.print_msgs(&f.r, &[&f.msgs[last]]);
    Ok(())
}
