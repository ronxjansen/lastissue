//! `lastissue issue …`: args → `lastissue::ops` / `query` → output.

use super::common::{self, Ctx};
use super::{CreateArgs, IssueCmd, ListArgs, UpdateArgs};
use crate::error::Result;
use crate::format;
use crate::model::{self, labels_of, priority_of, state_of};
use crate::store::IssueRef;
use crate::store::mbox::Msg;
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
        state: a.state.map(Into::into),
        priority: a.priority.map(Into::into),
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
        states: a.state.into_iter().map(Into::into).collect(),
        all: a.all,
        labels: a.label,
        assignee: a.assignee,
        author: a.author,
        watching: a.watching,
        agent: a.agent,
        parent: a.parent,
        ready: a.ready,
        expired: a.expired,
        blocked: false,
        priorities: Vec::new(),
        due_before: None,
        due_after: None,
        search: a.search,
        regex: a.regex,
        sort: a.sort.map(Into::into),
        limit: a.limit,
        offset: a.offset,
        // Stats only for the rows actually printed in full.
        stats: !a.brief,
    };
    for f in &a.filter {
        q.filter(f)?;
    }
    let page = query::list(&ctx.store, &ctx.me, &q)?;
    let next_page = page.next_offset.map(|n| format!("--offset {n}"));

    let root = ctx.store.root.display().to_string();
    if a.brief {
        let mut out = format!("X-Root: {root}\n");
        for row in &page.rows {
            out.push_str(&brief(&row.issue, &row.head, ctx.color));
        }
        if let Some(n) = next_page {
            out.push_str(&format!("X-Next: {n}\n"));
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

/// One `--brief` line for an issue head.
pub fn brief(r: &IssueRef, head: &Msg, color: bool) -> String {
    format::brief_line(
        &r.id,
        state_of(head).as_str(),
        priority_of(head).map(|p| p.as_str()),
        &labels_of(head),
        head.get("Subject").unwrap_or(""),
        color,
    )
}

// ---------------------------------------------------------------- show

/// Computed headers for comment `i` (X-Comment, X-Replying-To).
pub fn comment_extras(
    r: &IssueRef,
    msgs: &[Msg],
    nums: &[Option<usize>],
    i: usize,
) -> Vec<(&'static str, String)> {
    let mut extra = Vec::new();
    if let Some(n) = nums[i] {
        extra.push(("X-Comment", format!("{}/{n}", r.id)));
        let irt = msgs[i].get("In-Reply-To").unwrap_or("");
        if irt != msgs[0].message_id() {
            let target = msgs
                .iter()
                .position(|m| m.message_id() == irt)
                .and_then(|j| nums[j]);
            let marker = match target {
                Some(t) => format!("↳ replying to /{t}"),
                None => "↳ replying to (deleted)".to_string(),
            };
            extra.push(("X-Replying-To", marker));
        }
    }
    extra
}

fn show(ctx: &mut Ctx, id: &str, no_events: bool) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let msgs = ctx.store.read(&r)?;
    let nums = model::comment_numbers(&msgs);
    let updated = msgs.iter().filter_map(|m| m.date()).max();
    let mut out = String::new();
    for (i, m) in msgs.iter().enumerate() {
        if no_events && model::is_event(m) {
            continue;
        }
        if i == 0 {
            let mut extra = vec![
                ("X-Short-Id", r.id.clone()),
                ("X-Path", ctx.store.rel(&r.path)),
                ("X-Root", ctx.store.root.display().to_string()),
                ("X-Comment-Count", nums.iter().flatten().count().to_string()),
            ];
            if let Some(u) = updated {
                extra.push(("X-Updated", u.to_rfc2822()));
            }
            out.push_str(&format::message_with(m, &extra));
        } else {
            out.push_str(&format::message_with(
                m,
                &comment_extras(&r, &msgs, &nums, i),
            ));
        }
    }
    let painted = format::paint(&out, ctx.color, false);
    ctx.print(&painted);
    Ok(())
}

// ---------------------------------------------------------------- update

fn update(ctx: &mut Ctx, a: UpdateArgs) -> Result<()> {
    let body = common::explicit_body(&a.body)?;
    // clap makes each set/unset pair exclusive: `Some(None)` = unset.
    let set_or_clear = |v: Option<String>, clear: bool| {
        if clear { Some(None) } else { v.map(Some) }
    };
    let u = IssueUpdate {
        title: a.title,
        body,
        state: a.state.map(Into::into),
        priority: if a.no_priority {
            Some(None)
        } else {
            a.priority.map(|p| Some(p.into()))
        },
        add_labels: a.add_label,
        remove_labels: a.remove_label,
        assignee: set_or_clear(a.assign, a.unassign),
        watch: a.watch,
        unwatch: a.unwatch,
        due: set_or_clear(a.due, a.no_due),
        expires: a.expires.map(|d| (!d.trim().is_empty()).then_some(d)),
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

// ---------------------------------------------------------------- delete

fn delete(ctx: &mut Ctx, id: &str) -> Result<()> {
    let d = ops::delete_issue(&ctx.store, id)?;
    let fields = vec![
        ("Message-ID".to_string(), d.head.message_id().to_string()),
        (
            "Subject".to_string(),
            d.head.get("Subject").unwrap_or("").to_string(),
        ),
        ("X-Short-Id".to_string(), d.issue.id.clone()),
        ("X-Path".to_string(), ctx.store.rel(&d.issue.path)),
        ("X-Deleted".to_string(), "yes".to_string()),
    ];
    let text = format::paint(&format::block(&fields), ctx.color, true);
    ctx.print(&text);
    Ok(())
}
