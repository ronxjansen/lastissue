//! `lastissue issue …`

use super::common::{self, Ctx};
use super::{CreateArgs, IssueCmd, ListArgs, SortKey, UpdateArgs};
use crate::error::{Error, Result};
use crate::format;
use crate::model::{self, Priority, State};
use crate::store::mbox::{self, Msg};
use crate::store::{self, IssueRef, Project};
use std::cmp::Reverse;
use std::collections::HashMap;

pub fn run(ctx: &mut Ctx, cmd: IssueCmd) -> Result<()> {
    match cmd {
        IssueCmd::Create(a) => create(ctx, *a),
        IssueCmd::List(a) => list(ctx, *a),
        IssueCmd::Show { id } => show(ctx, &id),
        IssueCmd::Update(a) => update(ctx, *a),
        IssueCmd::Close {
            id,
            cancel,
            duplicate_of,
            comment,
        } => close(ctx, &id, cancel, duplicate_of, comment),
        IssueCmd::Reopen { id } => reopen(ctx, &id),
        IssueCmd::Move {
            id,
            project,
            no_project,
        } => move_issue(ctx, &id, project, no_project),
        IssueCmd::Delete { id } => delete(ctx, &id),
    }
}

// ---------------------------------------------------------------- helpers

pub fn state_of(m: &Msg) -> State {
    m.get("X-Issue-State")
        .and_then(State::parse)
        .unwrap_or(State::Backlog)
}

pub fn priority_of(m: &Msg) -> Option<Priority> {
    m.get("X-Priority-Level").and_then(Priority::parse)
}

pub fn labels_of(m: &Msg) -> Vec<String> {
    m.get("Keywords")
        .map(|k| model::normalize_labels([k]))
        .unwrap_or_default()
}

fn list_of(m: &Msg, h: &str) -> Vec<String> {
    m.get(h).map(model::split_list).unwrap_or_default()
}

fn set_list(m: &mut Msg, h: &str, items: &[String]) {
    m.set_opt(h, model::join_list(items).as_deref());
}

/// Message-ID for an ID argument; refuses self-references.
fn relation_target(ctx: &Ctx, id: &str, own: Option<&str>) -> Result<String> {
    let mid = ctx.issue_message_id(id)?;
    if Some(mid.as_str()) == own {
        return Err(Error::validation("self-reference")
            .with("Id", id)
            .with("Hint", "an issue cannot relate to itself"));
    }
    Ok(mid)
}

/// For removals: resolve if possible, else match stored IDs by ULID prefix
/// (the target issue may have been deleted).
fn removal_target(ctx: &Ctx, id: &str, existing: &[String]) -> Option<String> {
    if let Ok(mid) = ctx.issue_message_id(id) {
        return Some(mid);
    }
    let q = id
        .trim()
        .trim_start_matches('<')
        .split('@')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    existing
        .iter()
        .find(|e| !q.is_empty() && mbox::ulid_of(e).to_ascii_uppercase().starts_with(&q))
        .cloned()
}

fn apply_custom_headers(m: &mut Msg, headers: &[String]) -> Result<()> {
    for kv in headers {
        let (k, v) = model::parse_custom_header(kv)?;
        m.set(&k, &v);
    }
    Ok(())
}

/// After header changes: enforce the duplicate rule (§3.1).
fn validate_duplicate(m: &mut Msg) -> Result<()> {
    if state_of(m) == State::Duplicate {
        if !m.has("References") {
            return Err(Error::validation("duplicate-needs-original").with(
                "Hint",
                "use: lastissue issue close <id> --duplicate-of <original-id>",
            ));
        }
    } else {
        m.remove("References");
    }
    Ok(())
}

/// Append one event listing every changed header of message #1 (if any)
/// and write the file. Returns the indices of messages to print.
fn commit(
    ctx: &mut Ctx,
    r: &IssueRef,
    msgs: &mut Vec<Msg>,
    before: &Msg,
    kind: &str,
    body_changed: bool,
) -> Result<Vec<usize>> {
    let lines = model::diff_lines(before, &msgs[0]);
    let mut printed = vec![0];
    if lines.is_empty() && !body_changed {
        return Ok(printed);
    }
    if !lines.is_empty() {
        let ev = model::event(ctx.actor()?, &msgs[0], kind, &lines);
        msgs.push(ev);
        printed.push(msgs.len() - 1);
    }
    ctx.store.write(&r.path, msgs)?;
    Ok(printed)
}

fn print_indices(ctx: &mut Ctx, r: &IssueRef, msgs: &[Msg], idx: &[usize]) {
    let refs: Vec<&Msg> = idx.iter().map(|&i| &msgs[i]).collect();
    ctx.print_msgs(r, &refs);
}

// ---------------------------------------------------------------- create

fn create(ctx: &mut Ctx, a: CreateArgs) -> Result<()> {
    let title = mbox::clean_value(&a.title);
    if title.is_empty() {
        return Err(Error::validation("empty-title").with("Hint", "pass --title \"…\""));
    }
    let state = a.state.unwrap_or(State::Backlog);
    if state == State::Duplicate {
        return Err(Error::validation("duplicate-needs-original").with(
            "Hint",
            "create the issue, then: lastissue issue close <id> --duplicate-of <original-id>",
        ));
    }
    let project = a
        .project
        .as_deref()
        .map(|p| ctx.store.resolve_project(p))
        .transpose()?;

    // Resolve everything before touching disk.
    let assignee = a.assign.as_deref().map(|w| ctx.who(w)).transpose()?;
    let mut watchers = Vec::new();
    for w in &a.watch {
        let w = ctx.who(w)?;
        if !watchers.contains(&w) {
            watchers.push(w);
        }
    }
    let due = a.due.as_deref().map(|d| ctx.date(d)).transpose()?;
    let expires = a.expires.as_deref().map(|d| ctx.date(d)).transpose()?;
    let parent = a
        .parent
        .as_deref()
        .map(|p| relation_target(ctx, p, None))
        .transpose()?;
    let mut blocked = Vec::new();
    for b in &a.blocked_by {
        let mid = relation_target(ctx, b, None)?;
        if !blocked.contains(&mid) {
            blocked.push(mid);
        }
    }
    let mut related = Vec::new();
    for b in &a.relate {
        let mid = relation_target(ctx, b, None)?;
        if !related.contains(&mid) {
            related.push(mid);
        }
    }
    let body = common::optional_body(&a.body)?;
    let actor = ctx.actor()?.clone();

    let existing: Vec<String> = ctx.store.issues()?.into_iter().map(|r| r.id).collect();
    let (ulid, id) = store::fresh_id(&existing);
    let mut m = Msg::default();
    m.set("Message-ID", &model::message_id(ulid));
    m.set("Date", &model::date_header(model::now()));
    m.set("From", &actor.from);
    m.set_opt("Sender", actor.sender.as_deref());
    m.set_opt("To", assignee.as_deref());
    set_list(&mut m, "Cc", &watchers);
    m.set("Subject", &title);
    m.set("X-Issue-State", state.as_str());
    m.set_opt("X-Priority-Level", a.priority.map(|p| p.as_str()));
    set_list(
        &mut m,
        "Keywords",
        &model::normalize_labels(a.label.iter().map(|s| s.as_str())),
    );
    m.set_opt("Reply-By", due.as_deref());
    m.set_opt("Expires", expires.as_deref());
    m.set_opt("List-Id", project.as_ref().map(|p| p.list_id()).as_deref());
    m.set_opt("X-Parent", parent.as_deref());
    set_list(&mut m, "X-Blocked-By", &blocked);
    set_list(&mut m, "X-Related", &related);
    apply_custom_headers(&mut m, &a.header)?;
    model::set_content_headers(&mut m);
    m.body = mbox::normalize_body(&body);

    let slug = store::slugify(&title, 4, "issue");
    let path = ctx.store.issue_path(project.as_ref(), &slug, &id);
    let r = IssueRef { id, path, project };
    let msgs = vec![m];
    ctx.store.write(&r.path, &msgs)?;
    print_indices(ctx, &r, &msgs, &[0]);
    Ok(())
}

// ---------------------------------------------------------------- list

struct Row {
    r: IssueRef,
    head: Msg,
    full: Option<Vec<Msg>>,
    stats: Option<(usize, Option<chrono::DateTime<chrono::FixedOffset>>)>,
}

impl Row {
    fn created(&self) -> i64 {
        self.head.date().map(|d| d.timestamp()).unwrap_or(0)
    }
    fn updated(&self) -> i64 {
        self.stats
            .as_ref()
            .and_then(|s| s.1)
            .map(|d| d.timestamp())
            .unwrap_or_else(|| self.created())
    }
    fn due(&self) -> Option<i64> {
        self.head
            .get("Reply-By")
            .and_then(mbox::parse_date)
            .map(|d| d.timestamp())
    }
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

fn list(ctx: &mut Ctx, a: ListArgs) -> Result<()> {
    let project = a
        .project
        .as_deref()
        .map(|p| ctx.store.resolve_project(p))
        .transpose()?;
    let parent = a
        .parent
        .as_deref()
        .map(|p| ctx.issue_message_id(p))
        .transpose()?;
    let assignee = a
        .assignee
        .as_deref()
        .map(|w| ctx.who_pattern(w))
        .transpose()?;
    let author = a
        .author
        .as_deref()
        .map(|w| ctx.who_pattern(w))
        .transpose()?;
    let watching = a
        .watching
        .as_deref()
        .map(|w| ctx.who_pattern(w))
        .transpose()?;
    let want_labels = model::normalize_labels(a.label.iter().map(|s| s.as_str()));
    let now = model::now().timestamp();

    // Headers of every issue (needed for readiness across projects).
    let mut heads: Vec<(IssueRef, Msg)> = Vec::new();
    for r in ctx.store.issues()? {
        let in_scope = match (&project, a.no_project) {
            (Some(p), _) => r.project.as_ref().is_some_and(|rp| rp.id == p.id),
            (None, true) => r.project.is_none(),
            _ => true,
        };
        if !in_scope && !a.ready {
            continue;
        }
        let head = ctx.store.read_head(&r)?;
        heads.push((r, head));
    }
    let states: HashMap<String, State> = if a.ready {
        ctx.store.states_by_message_id(&heads)
    } else {
        HashMap::new()
    };

    let mut rows: Vec<Row> = Vec::new();
    for (r, head) in heads {
        match (&project, a.no_project) {
            (Some(p), _) if r.project.as_ref().is_none_or(|rp| rp.id != p.id) => continue,
            (None, true) if r.project.is_some() => continue,
            _ => {}
        }
        let st = state_of(&head);
        if !a.state.is_empty() {
            if !a.state.contains(&st) {
                continue;
            }
        } else if !a.all && !st.is_open() {
            continue;
        }
        if !want_labels.is_empty() {
            let have = labels_of(&head);
            if !want_labels.iter().all(|l| have.contains(l)) {
                continue;
            }
        }
        if let Some(w) = &assignee
            && !head.get("To").is_some_and(|v| contains_ci(v, w))
        {
            continue;
        }
        if let Some(w) = &author
            && !head.get("From").is_some_and(|v| contains_ci(v, w))
        {
            continue;
        }
        if let Some(w) = &watching
            && !list_of(&head, "Cc").iter().any(|v| contains_ci(v, w))
        {
            continue;
        }
        if a.agent && !head.has("Sender") {
            continue;
        }
        if let Some(p) = &parent
            && head.get("X-Parent") != Some(p.as_str())
        {
            continue;
        }
        if a.ready {
            let blockers_open = list_of(&head, "X-Blocked-By")
                .iter()
                .any(|b| states.get(b).is_some_and(|s| s.is_open()));
            if st != State::Todo || blockers_open {
                continue;
            }
        }
        if a.expired
            && head
                .get("Expires")
                .and_then(mbox::parse_date)
                .is_none_or(|d| d.timestamp() >= now)
        {
            continue;
        }
        rows.push(Row {
            r,
            head,
            full: None,
            stats: None,
        });
    }

    if let Some(text) = &a.search {
        let needle = text.to_lowercase();
        let mut kept = Vec::new();
        for mut row in rows {
            let msgs = ctx.store.read(&row.r)?;
            let hit = msgs.iter().enumerate().any(|(i, m)| {
                (i == 0
                    && m.get("Subject")
                        .is_some_and(|s| s.to_lowercase().contains(&needle)))
                    || (!model::is_event(m) && m.body.to_lowercase().contains(&needle))
            });
            if hit {
                row.full = Some(msgs);
                kept.push(row);
            }
        }
        rows = kept;
    }

    if a.sort == Some(SortKey::Updated) || !a.brief {
        for row in rows.iter_mut() {
            row.stats = Some(ctx.store.stats(&row.r)?);
        }
    }

    let default_key = |r: &Row| {
        (
            state_of(&r.head).sort_rank(),
            Priority::rank(priority_of(&r.head)),
            Reverse(r.created()),
        )
    };
    match a.sort {
        None => rows.sort_by_key(default_key),
        Some(SortKey::Created) => rows.sort_by_key(|r| Reverse(r.created())),
        Some(SortKey::Updated) => rows.sort_by_key(|r| Reverse(r.updated())),
        Some(SortKey::Priority) => {
            rows.sort_by_key(|r| (Priority::rank(priority_of(&r.head)), Reverse(r.created())))
        }
        Some(SortKey::Due) => rows.sort_by_key(|r| (r.due().is_none(), r.due(), default_key(r))),
    }

    let total = rows.len();
    if a.limit > 0 {
        rows.truncate(a.limit);
    }

    let root = ctx.store.root.display().to_string();
    let mut out = String::new();
    if a.brief {
        out.push_str(&format!("X-Root: {root}\n"));
        for row in &rows {
            out.push_str(&format::brief_line(
                &row.r.id,
                state_of(&row.head).as_str(),
                priority_of(&row.head).map(|p| p.as_str()),
                &labels_of(&row.head),
                row.head.get("Subject").unwrap_or(""),
                ctx.color,
            ));
        }
        if total > rows.len() {
            out.push_str(&format!("X-Truncated: {} of {total}\n", rows.len()));
        }
        ctx.print(&out);
        return Ok(());
    }

    let mut blocks: Vec<String> = vec![format::block(&[("X-Root".into(), root)])];
    for row in &rows {
        let mut fields: Vec<(String, String)> = vec![
            ("Message-ID".into(), row.head.message_id().to_string()),
            ("X-Short-Id".into(), row.r.id.clone()),
        ];
        for (k, v) in row.head.ordered_headers() {
            let lk = k.to_ascii_lowercase();
            if lk == "message-id" || lk == "content-type" || lk == "content-transfer-encoding" {
                continue;
            }
            fields.push((k, v));
        }
        if let Some((count, updated)) = &row.stats {
            fields.push(("X-Comment-Count".into(), count.to_string()));
            if let Some(u) = updated {
                fields.push(("X-Updated".into(), u.to_rfc2822()));
            }
        }
        fields.push(("X-Path".into(), ctx.store.rel(&row.r.path)));
        blocks.push(format::block(&fields));
    }
    if total > rows.len() {
        blocks.push(format::block(&[(
            "X-Truncated".into(),
            format!("{} of {total}", rows.len()),
        )]));
    }
    let text = blocks.join("\n");
    let painted = format::paint(&text, ctx.color, true);
    ctx.print(&painted);
    Ok(())
}

// ---------------------------------------------------------------- show

/// 1-based comment numbers: message index → n (events excluded).
pub fn comment_numbers(msgs: &[Msg]) -> Vec<Option<usize>> {
    let mut n = 0;
    msgs.iter()
        .enumerate()
        .map(|(i, m)| {
            if i == 0 || model::is_event(m) {
                None
            } else {
                n += 1;
                Some(n)
            }
        })
        .collect()
}

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

fn show(ctx: &mut Ctx, id: &str) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let msgs = ctx.store.read(&r)?;
    let nums = comment_numbers(&msgs);
    let updated = msgs.iter().filter_map(|m| m.date()).max();
    let mut out = String::new();
    for (i, m) in msgs.iter().enumerate() {
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
    let r = ctx.store.resolve_issue(&a.id)?;
    let mut msgs = ctx.store.read(&r)?;
    let before = msgs[0].clone();
    let own = before.message_id().to_string();
    let new_body = common::explicit_body(&a.body)?;

    // Resolve all inputs first so a bad one leaves the file untouched.
    let assignee = a.assign.as_deref().map(|w| ctx.who(w)).transpose()?;
    let watch: Vec<String> = a.watch.iter().map(|w| ctx.who(w)).collect::<Result<_>>()?;
    let unwatch: Vec<String> = a
        .unwatch
        .iter()
        .map(|w| ctx.who_pattern(w))
        .collect::<Result<_>>()?;
    let due = a.due.as_deref().map(|d| ctx.date(d)).transpose()?;
    let expires = match a.expires.as_deref() {
        Some(d) if d.trim().is_empty() => Some(None),
        Some(d) => Some(Some(ctx.date(d)?)),
        None => None,
    };
    let parent = a
        .parent
        .as_deref()
        .map(|p| relation_target(ctx, p, Some(&own)))
        .transpose()?;
    let add_blocked: Vec<String> = a
        .blocked_by
        .iter()
        .map(|b| relation_target(ctx, b, Some(&own)))
        .collect::<Result<_>>()?;
    let add_related: Vec<String> = a
        .relate
        .iter()
        .map(|b| relation_target(ctx, b, Some(&own)))
        .collect::<Result<_>>()?;

    let m = &mut msgs[0];
    if let Some(t) = &a.title {
        let t = mbox::clean_value(t);
        if t.is_empty() {
            return Err(Error::validation("empty-title"));
        }
        m.set("Subject", &t);
    }
    if let Some(s) = a.state {
        m.set("X-Issue-State", s.as_str());
    }
    if let Some(p) = a.priority {
        m.set("X-Priority-Level", p.as_str());
    }
    if a.no_priority {
        m.remove("X-Priority-Level");
    }
    if !a.add_label.is_empty() || !a.remove_label.is_empty() {
        let remove = model::normalize_labels(a.remove_label.iter().map(|s| s.as_str()));
        let mut labels = labels_of(m);
        labels.extend(model::normalize_labels(
            a.add_label.iter().map(|s| s.as_str()),
        ));
        let labels: Vec<String> = model::normalize_labels(labels.iter().map(|s| s.as_str()))
            .into_iter()
            .filter(|l| !remove.contains(l))
            .collect();
        set_list(m, "Keywords", &labels);
    }
    if let Some(w) = &assignee {
        m.set("To", w);
    }
    if a.unassign {
        m.remove("To");
    }
    if !watch.is_empty() || !unwatch.is_empty() {
        let mut cc = list_of(m, "Cc");
        for w in watch {
            if !cc.iter().any(|c| model::email_of(c) == model::email_of(&w)) {
                cc.push(w);
            }
        }
        cc.retain(|c| {
            !unwatch
                .iter()
                .any(|u| model::email_of(c) == model::email_of(u) || contains_ci(c, u))
        });
        set_list(m, "Cc", &cc);
    }
    if let Some(d) = &due {
        m.set("Reply-By", d);
    }
    if a.no_due {
        m.remove("Reply-By");
    }
    if let Some(e) = &expires {
        m.set_opt("Expires", e.as_deref());
    }
    if let Some(p) = &parent {
        m.set("X-Parent", p);
    }
    if a.no_parent {
        m.remove("X-Parent");
    }
    for (header, add, remove) in [
        ("X-Blocked-By", &add_blocked, &a.unblock),
        ("X-Related", &add_related, &a.unrelate),
    ] {
        if add.is_empty() && remove.is_empty() {
            continue;
        }
        let mut cur = list_of(m, header);
        for x in add {
            if !cur.contains(x) {
                cur.push(x.clone());
            }
        }
        for id in remove.iter() {
            match removal_target(ctx, id, &cur) {
                Some(mid) => cur.retain(|c| c != &mid),
                None => {
                    return Err(Error::not_found("relation-not-found")
                        .with("Id", id.as_str())
                        .with("Header", header));
                }
            }
        }
        set_list(m, header, &cur);
    }
    apply_custom_headers(m, &a.header)?;
    validate_duplicate(m)?;

    let mut body_changed = false;
    if let Some(b) = new_body {
        let b = mbox::normalize_body(&b);
        if b != m.body {
            m.body = b;
            model::mark_edited(m, ctx.actor()?);
            body_changed = true;
        }
    }
    let printed = commit(ctx, &r, &mut msgs, &before, "update", body_changed)?;
    print_indices(ctx, &r, &msgs, &printed);
    Ok(())
}

fn close(
    ctx: &mut Ctx,
    id: &str,
    cancel: bool,
    duplicate_of: Option<String>,
    comment: Option<String>,
) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let mut msgs = ctx.store.read(&r)?;
    let before = msgs[0].clone();
    let original = duplicate_of
        .as_deref()
        .map(|d| relation_target(ctx, d, Some(before.message_id())))
        .transpose()?;
    if let Some(c) = &comment
        && c.trim().is_empty()
    {
        return Err(Error::validation("empty-body").with("Hint", "--comment must not be empty"));
    }
    let state = if original.is_some() {
        State::Duplicate
    } else if cancel {
        State::Cancelled
    } else {
        State::Done
    };
    msgs[0].set("X-Issue-State", state.as_str());
    msgs[0].set_opt("References", original.as_deref());
    validate_duplicate(&mut msgs[0])?;
    let lines = model::diff_lines(&before, &msgs[0]);
    let mut printed = vec![0];
    if !lines.is_empty() {
        msgs.push(model::event(ctx.actor()?, &msgs[0], "update", &lines));
        printed.push(msgs.len() - 1);
    }
    if let Some(c) = &comment {
        msgs.push(model::comment(ctx.actor()?, &msgs[0], None, c));
        printed.push(msgs.len() - 1);
    }
    if printed.len() > 1 {
        ctx.store.write(&r.path, &msgs)?;
    }
    print_indices(ctx, &r, &msgs, &printed);
    Ok(())
}

fn reopen(ctx: &mut Ctx, id: &str) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let mut msgs = ctx.store.read(&r)?;
    let before = msgs[0].clone();
    msgs[0].set("X-Issue-State", State::Todo.as_str());
    validate_duplicate(&mut msgs[0])?;
    let printed = commit(ctx, &r, &mut msgs, &before, "update", false)?;
    print_indices(ctx, &r, &msgs, &printed);
    Ok(())
}

fn move_issue(ctx: &mut Ctx, id: &str, project: Option<String>, no_project: bool) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let target: Option<Project> = if no_project {
        None
    } else {
        project
            .as_deref()
            .map(|p| ctx.store.resolve_project(p))
            .transpose()?
    };
    let mut msgs = ctx.store.read(&r)?;
    let before = msgs[0].clone();
    let file_name = r.path.file_name().expect("issue file name").to_owned();
    let dir = target
        .as_ref()
        .map(|p| p.dir.clone())
        .unwrap_or_else(|| ctx.store.root.clone());
    let new_path = dir.join(file_name);
    msgs[0].set_opt("List-Id", target.as_ref().map(|p| p.list_id()).as_deref());
    let new_ref = IssueRef {
        id: r.id.clone(),
        path: new_path.clone(),
        project: target,
    };
    if new_path == r.path && before == msgs[0] {
        print_indices(ctx, &r, &msgs, &[0]);
        return Ok(());
    }
    let mut lines = model::diff_lines(&before, &msgs[0]);
    lines.push(format!(
        "path: {} → {}",
        ctx.store.rel(&r.path),
        ctx.store.rel(&new_path)
    ));
    msgs.push(model::event(ctx.actor()?, &msgs[0], "move", &lines));
    ctx.store.write(&new_path, &msgs)?;
    if new_path != r.path {
        std::fs::remove_file(&r.path)?;
    }
    let last = msgs.len() - 1;
    print_indices(ctx, &new_ref, &msgs, &[0, last]);
    Ok(())
}

fn delete(ctx: &mut Ctx, id: &str) -> Result<()> {
    let r = ctx.store.resolve_issue(id)?;
    let head = ctx.store.read_head(&r)?;
    std::fs::remove_file(&r.path)?;
    let fields = vec![
        ("Message-ID".to_string(), head.message_id().to_string()),
        (
            "Subject".to_string(),
            head.get("Subject").unwrap_or("").to_string(),
        ),
        ("X-Short-Id".to_string(), r.id.clone()),
        ("X-Path".to_string(), ctx.store.rel(&r.path)),
        ("X-Deleted".to_string(), "yes".to_string()),
    ];
    let text = format::paint(&format::block(&fields), ctx.color, true);
    ctx.print(&text);
    Ok(())
}
