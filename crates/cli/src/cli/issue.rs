//! `lastissue issue …`: args → `lastissue::ops` / `query` → output.

use super::common::{self, Ctx};
use super::{CreateArgs, IssueCmd, ListArgs, UpdateArgs};
use crate::error::Result;
use crate::format;
use crate::model::{self, Activity};
use lastissue::ops::{self, CloseAs, IssueUpdate, NewIssue};
use lastissue::query::{self, ListQuery};

pub fn run(ctx: &mut Ctx, cmd: IssueCmd) -> Result<()> {
    let (store, me) = (&ctx.store, &ctx.me);
    let o = match cmd {
        IssueCmd::Create(a) => ops::create_issue(store, me, &new_issue(*a)?)?,
        IssueCmd::List(a) => return list(ctx, *a),
        IssueCmd::Show { id, no_events } => return show(ctx, &id, no_events),
        IssueCmd::Update(a) => {
            let id = a.id.clone();
            ops::update_issue(store, me, &id, &issue_update(*a)?)?
        }
        IssueCmd::Close {
            id,
            cancel,
            duplicate_of,
            comment,
        } => {
            let how = match duplicate_of {
                Some(d) => CloseAs::DuplicateOf(d),
                None if cancel => CloseAs::Cancelled,
                None => CloseAs::Done,
            };
            ops::close_issue(store, me, &id, &how, comment.as_deref())?
        }
        IssueCmd::Reopen { id } => ops::reopen_issue(store, me, &id)?,
        IssueCmd::Move {
            id,
            project,
            no_project,
        } => {
            let target = if no_project { None } else { project };
            ops::move_issue(store, me, &id, target.as_deref())?
        }
        IssueCmd::Delete { id } => return delete(ctx, &id),
    };
    ctx.print_indices(&o.issue, &o.msgs, &o.shown());
    Ok(())
}

fn new_issue(a: CreateArgs) -> Result<NewIssue> {
    Ok(NewIssue {
        title: a.title,
        project: a.project,
        body: common::given_body(&a.body)?.unwrap_or_default(),
        state: a.state,
        priority: a.priority,
        labels: a.label,
        assignee: a.assign,
        watchers: a.watch,
        due: a.due,
        expires: a.expires,
        parent: a.parent,
        blocked_by: a.blocked_by,
        related: a.relate,
        headers: a.header,
    })
}

fn issue_update(a: UpdateArgs) -> Result<IssueUpdate> {
    Ok(IssueUpdate {
        title: a.title,
        body: common::explicit_body(&a.body)?,
        state: a.state,
        priority: set_or_clear(a.priority, a.no_priority),
        add_labels: a.add_label,
        remove_labels: a.remove_label,
        assignee: set_or_clear(a.assign, a.unassign),
        watch: a.watch,
        unwatch: a.unwatch,
        due: set_or_clear(a.due, a.no_due),
        expires: set_or_clear(a.expires, a.no_expires),
        parent: set_or_clear(a.parent, a.no_parent),
        add_blocked_by: a.blocked_by,
        remove_blocked_by: a.unblock,
        add_related: a.relate,
        remove_related: a.unrelate,
        headers: a.header,
    })
}

/// clap makes each set/unset flag pair exclusive: `Some(None)` = unset.
#[allow(clippy::option_option, reason = "IssueUpdate's set/unset fields")]
fn set_or_clear<T>(v: Option<T>, clear: bool) -> Option<Option<T>> {
    if clear { Some(None) } else { v.map(Some) }
}

// ---------------------------------------------------------------- list

fn list(ctx: &mut Ctx, a: ListArgs) -> Result<()> {
    let mut q = ListQuery {
        project: a.project,
        no_project: a.no_project,
        states: a.state,
        all: a.all,
        labels: a.label,
        assignee: a.assignee,
        author: a.author,
        watching: a.watching,
        agent: a.agent,
        parent: a.parent,
        ready: a.ready,
        expired: a.expired,
        search: a.search,
        regex: a.regex,
        sort: a.sort.map(Into::into),
        limit: a.limit,
        offset: a.offset,
        // Stats only for the rows actually printed in full.
        stats: !a.brief,
        ..Default::default()
    };
    for f in &a.filter {
        q.filter(f)?;
    }
    let page = query::list(&ctx.store, &ctx.me, &q)?;
    let root = format::block(&[("X-Root", ctx.store.root.display().to_string())]);
    let next = page
        .next_offset
        .map(|n| format::block(&[("X-Next", format!("--offset {n}"))]));
    if a.brief {
        let mut out = root;
        for row in &page.rows {
            out.push_str(&format::brief(&row.issue, &row.head, ctx.color));
        }
        out.extend(next);
        ctx.print(&out);
        return Ok(());
    }

    let mut blocks = vec![root];
    for row in &page.rows {
        let mut fields = vec![
            ("Message-ID".into(), row.head.message_id().to_string()),
            ("X-Short-Id".into(), row.issue.id.clone()),
        ];
        fields.extend(row.head.ordered_headers().into_iter().filter(|(k, _)| {
            !["Message-ID", "Content-Type", "Content-Transfer-Encoding"]
                .iter()
                .any(|h| h.eq_ignore_ascii_case(k))
        }));
        if let Some(st) = &row.stats {
            fields.push(("X-Comment-Count".into(), st.comments.to_string()));
            if let Some(u) = st.updated {
                fields.push(("X-Updated".into(), u.to_rfc2822()));
            }
        }
        if let Some(m) = &row.matches {
            for s in &m.shown {
                fields.push(("X-Match".into(), s.clone()));
            }
            fields.push(("X-Match-Count".into(), m.count.to_string()));
        }
        fields.push(("X-Path".into(), ctx.store.rel(&row.issue.path)));
        blocks.push(format::block(&fields));
    }
    blocks.extend(next);
    ctx.paint(&blocks.join("\n"), true);
    Ok(())
}

fn show(ctx: &mut Ctx, id: &str, no_events: bool) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let msgs = ctx.store.read(&r)?;
    let head = &msgs[0];
    let act = model::activity(&msgs);
    let comments = act
        .iter()
        .filter(|a| matches!(a, Activity::Comment { .. }))
        .count();
    let mut extra = ctx.ident(&r).to_vec();
    extra.push(("X-Root", ctx.store.root.display().to_string()));
    extra.push(("X-Comment-Count", comments.to_string()));
    if let Some(u) = msgs.iter().filter_map(|m| m.date()).max() {
        extra.push(("X-Updated", u.to_rfc2822()));
    }
    let mut out = format::message_with(head, &extra);
    for a in &act {
        if no_events && matches!(a, Activity::Event { .. }) {
            continue;
        }
        out.push_str(&format::message_with(
            a.msg(),
            &format::comment_headers(&r, head, a),
        ));
    }
    ctx.paint(&out, false);
    Ok(())
}

// ---------------------------------------------------------------- delete

fn delete(ctx: &mut Ctx, id: &str) -> Result<()> {
    let d = ops::delete_issue(&ctx.store, id)?;
    let [short, path] = ctx.ident(&d.issue);
    let text = format::block(&[
        ("Message-ID", d.head.message_id().to_string()),
        ("Subject", d.head.get("Subject").unwrap_or("").to_string()),
        short,
        path,
        ("X-Deleted", "yes".into()),
    ]);
    ctx.paint(&text, true);
    Ok(())
}
