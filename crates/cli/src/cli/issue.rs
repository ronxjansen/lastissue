//! `lastissue issue …`: args → `lastissue::ops` / `query` → output.

use super::common::{self, Ctx};
use super::{CreateArgs, IssueCmd, ListArgs, UpdateArgs};
use crate::error::Result;
use crate::format;
use crate::model::{self, Activity};
use lastissue::ops::{self, CloseAs, IssueUpdate, NewIssue};
use lastissue::query::{self, ListQuery};

pub fn run(ctx: &mut Ctx, cmd: IssueCmd) -> Result<()> {
    match cmd {
        IssueCmd::Create(a) => create(ctx, *a),
        IssueCmd::List(a) => list(ctx, *a),
        IssueCmd::Show { id, no_events } => show(ctx, &id, no_events),
        IssueCmd::Update(a) => update(ctx, *a),
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
            ops::close_issue(&ctx.store, &ctx.me, &id, &how, comment.as_deref())
                .map(|o| print_outcome(ctx, &o))
        }
        IssueCmd::Reopen { id } => {
            ops::reopen_issue(&ctx.store, &ctx.me, &id).map(|o| print_outcome(ctx, &o))
        }
        IssueCmd::Move {
            id,
            project,
            no_project,
        } => {
            let target = if no_project { None } else { project };
            ops::move_issue(&ctx.store, &ctx.me, &id, target.as_deref())
                .map(|o| print_outcome(ctx, &o))
        }
        IssueCmd::Delete { id } => delete(ctx, &id),
    }
}

fn print_outcome(ctx: &mut Ctx, o: &ops::Outcome) {
    ctx.print_indices(&o.issue, &o.msgs, &o.shown());
}

// ---------------------------------------------------------------- create

fn create(ctx: &mut Ctx, a: CreateArgs) -> Result<()> {
    let body = common::optional_body(&a.body)?;
    let new = NewIssue {
        title: a.title,
        project: a.project,
        body,
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
    };
    let o = ops::create_issue(&ctx.store, &ctx.me, &new)?;
    print_outcome(ctx, &o);
    Ok(())
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
    let next_page = page.next_offset.map(|n| format!("--offset {n}"));

    let root = ctx.store.root.display().to_string();
    if a.brief {
        let mut out = format::block(&[("X-Root".into(), root)]);
        for row in &page.rows {
            out.push_str(&format::brief(&row.issue, &row.head, ctx.color));
        }
        if let Some(n) = next_page {
            out.push_str(&format::block(&[("X-Next".into(), n)]));
        }
        ctx.print(&out);
        return Ok(());
    }

    let mut blocks: Vec<String> = vec![format::block(&[("X-Root".into(), root)])];
    for row in &page.rows {
        let mut fields: Vec<(String, String)> = vec![
            ("Message-ID".into(), row.head.message_id().to_string()),
            ("X-Short-Id".into(), row.issue.id.clone()),
        ];
        for (k, v) in row.head.ordered_headers() {
            let lk = k.to_ascii_lowercase();
            if lk == "message-id" || lk == "content-type" || lk == "content-transfer-encoding" {
                continue;
            }
            fields.push((k, v));
        }
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
    if let Some(n) = next_page {
        blocks.push(format::block(&[("X-Next".into(), n)]));
    }
    let text = blocks.join("\n");
    let painted = format::paint(&text, ctx.color, true);
    ctx.print(&painted);
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
    let painted = format::paint(&out, ctx.color, false);
    ctx.print(&painted);
    Ok(())
}

// ---------------------------------------------------------------- update

fn update(ctx: &mut Ctx, a: UpdateArgs) -> Result<()> {
    let body = common::explicit_body(&a.body)?;
    let u = IssueUpdate {
        title: a.title,
        body,
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
    };
    let o = ops::update_issue(&ctx.store, &ctx.me, &a.id, &u)?;
    print_outcome(ctx, &o);
    Ok(())
}

/// clap makes each set/unset flag pair exclusive: `Some(None)` = unset.
#[allow(clippy::option_option, reason = "IssueUpdate's set/unset fields")]
fn set_or_clear<T>(v: Option<T>, clear: bool) -> Option<Option<T>> {
    if clear { Some(None) } else { v.map(Some) }
}

// ---------------------------------------------------------------- delete

fn delete(ctx: &mut Ctx, id: &str) -> Result<()> {
    let d = ops::delete_issue(&ctx.store, id)?;
    let mut fields = vec![
        ("Message-ID".to_string(), d.head.message_id().to_string()),
        (
            "Subject".to_string(),
            d.head.get("Subject").unwrap_or("").to_string(),
        ),
    ];
    fields.extend(ctx.ident(&d.issue).map(|(k, v)| (k.to_string(), v)));
    fields.push(("X-Deleted".to_string(), "yes".to_string()));
    let text = format::paint(&format::block(&fields), ctx.color, true);
    ctx.print(&text);
    Ok(())
}
