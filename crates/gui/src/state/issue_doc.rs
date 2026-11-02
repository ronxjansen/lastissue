//! The issue open in the detail pane (GUI-SPEC §3, §5.2): its full
//! messages, loaded lazily off the UI thread, plus the title and body
//! drafts, their ~300 ms debounced save, the crash-guard mirror, and the
//! "changed on disk" conflict state. The view owns the text inputs and
//! syncs them through [`IssueDoc::edit`] and [`DocEvent::SetText`].

use super::draft::{self, DiskChange, Drafts, Field, FieldKind};
use super::edit::{Edit, Op};
use super::repo_model::RepoModel;
use gpui::{AppContext as _, Context, Entity, EventEmitter, Task, WeakEntity};
use lastissue::identity::Identity;
use lastissue::store::mbox::Msg;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Title and body edits save this long after the last keystroke.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(300);

/// The editor for `FieldKind` should now show this text (the disk changed
/// and there was no draft, or a banner choice replaced the draft).
pub enum DocEvent {
    SetText(FieldKind, String),
}

pub struct IssueDoc {
    /// Short ID (stable across moves).
    pub id: String,
    pub path: PathBuf,
    /// `None` until the first read finishes.
    pub msgs: Option<Arc<Vec<Msg>>>,
    pub error: Option<String>,
    /// `None` until the first read finishes.
    pub title: Option<Field>,
    pub body: Option<Field>,
    /// The last save failed (the draft is kept and mirrored).
    pub save_error: Option<String>,
    /// A body draft left by an earlier session, offered back.
    pub leftover: Option<String>,
    pub saving: bool,
    repo: WeakEntity<RepoModel>,
    me: Arc<Identity>,
    drafts: Option<Drafts>,
    resave: bool,
    /// The read in flight; replacing it cancels the previous one.
    #[allow(dead_code)]
    load: Option<Task<()>>,
    /// The pending debounce; replacing it restarts the timer.
    #[allow(dead_code)]
    debounce: Option<Task<()>>,
}

impl EventEmitter<DocEvent> for IssueDoc {}

impl IssueDoc {
    pub fn new(
        id: String,
        path: PathBuf,
        repo: &Entity<RepoModel>,
        me: Arc<Identity>,
        drafts: Option<Drafts>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut doc = IssueDoc {
            id,
            path: path.clone(),
            msgs: None,
            error: None,
            title: None,
            body: None,
            save_error: None,
            leftover: None,
            saving: false,
            repo: repo.downgrade(),
            me,
            drafts,
            resave: false,
            load: None,
            debounce: None,
        };
        doc.reload(path, cx);
        doc
    }

    pub fn message_id(&self) -> Option<String> {
        Some(self.msgs.as_ref()?.first()?.message_id().to_string())
    }

    /// Re-read from `path` (the issue may have moved). The previous
    /// messages stay visible until the new ones arrive.
    pub fn reload(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.path = path.clone();
        let Some(store) = self.repo.upgrade().map(|r| r.read(cx).store.clone()) else {
            return;
        };
        self.load = Some(cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move {
                    let r = store.issue_ref_for(&path).ok_or_else(|| {
                        lastissue::error::Error::not_found("issue-not-found")
                            .with("Path", path.display().to_string())
                    })?;
                    store.read(&r)
                })
                .await;
            let _ = this.update(cx, |doc, cx| {
                match r {
                    Ok(msgs) => doc.loaded(msgs, cx),
                    Err(e) => doc.error = Some(e.to_string().trim().replace('\n', " · ")),
                }
                cx.notify();
            });
        }));
    }

    /// Fresh messages from disk: someone else's write (or the first read).
    fn loaded(&mut self, msgs: Vec<Msg>, cx: &mut Context<Self>) {
        let head = &msgs[0];
        let disk_title = head.get("Subject").unwrap_or("").to_string();
        let disk_body = head.body.clone();
        let mid = head.message_id().to_string();
        self.error = None;
        self.msgs = Some(Arc::new(msgs));
        for (kind, disk) in [(FieldKind::Title, disk_title), (FieldKind::Body, disk_body)] {
            let slot = match kind {
                FieldKind::Title => &mut self.title,
                FieldKind::Body => &mut self.body,
            };
            match slot {
                None => {
                    *slot = Some(Field::new(kind, &disk));
                    if kind == FieldKind::Body {
                        self.leftover = self
                            .drafts
                            .as_ref()
                            .and_then(|d| draft::leftover(d, &mid, &disk));
                    }
                }
                Some(f) => {
                    if let DiskChange::Adopt(text) = f.disk(&disk) {
                        cx.emit(DocEvent::SetText(kind, text));
                    }
                }
            }
        }
    }

    /// The result of the GUI's own write (a picker, a comment): new
    /// messages, maybe a new path. Drafts are left alone.
    pub fn adopt_own(&mut self, path: PathBuf, msgs: Vec<Msg>, cx: &mut Context<Self>) {
        self.path = path;
        self.msgs = Some(Arc::new(msgs));
        cx.notify();
    }

    fn field_mut(&mut self, kind: FieldKind) -> Option<&mut Field> {
        match kind {
            FieldKind::Title => self.title.as_mut(),
            FieldKind::Body => self.body.as_mut(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.title.as_ref().is_some_and(Field::is_dirty)
            || self.body.as_ref().is_some_and(Field::is_dirty)
    }

    pub fn has_conflict(&self) -> bool {
        self.title.as_ref().is_some_and(|f| f.conflict.is_some())
            || self.body.as_ref().is_some_and(|f| f.conflict.is_some())
    }

    /// The user typed into the title or body editor.
    pub fn edit(&mut self, kind: FieldKind, text: &str, cx: &mut Context<Self>) {
        let Some(f) = self.field_mut(kind) else {
            return;
        };
        if f.value() == text {
            return;
        }
        if f.edit(text) {
            self.schedule(cx);
        } else if kind == FieldKind::Body {
            self.clear_mirror();
        }
        cx.notify();
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            let _ = this.update(cx, |d, cx| d.save_now(cx));
        }));
    }

    fn clear_mirror(&self) {
        if let (Some(d), Some(mid)) = (&self.drafts, self.message_id()) {
            d.clear(&mid);
        }
    }

    /// Write pending drafts now (debounce fired, or the issue is closing).
    /// The save runs detached, so closing the pane doesn't cancel it.
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.debounce = None;
        if self.saving {
            self.resave = true;
            return;
        }
        // Crash guard: mirror the body draft before trying to write it.
        if let (Some(d), Some(mid), Some(b)) = (&self.drafts, self.message_id(), &self.body) {
            match &b.draft {
                Some(text) => {
                    let _ = d.save(&mid, text);
                }
                None => d.clear(&mid),
            }
        }
        let mut edits = Vec::new();
        if let Some(t) = self.title.as_ref().and_then(Field::to_save) {
            edits.push((FieldKind::Title, t));
        }
        if let Some(b) = self.body.as_ref().and_then(Field::to_save) {
            edits.push((FieldKind::Body, b));
        }
        let Some(repo) = self.repo.upgrade() else {
            return;
        };
        if edits.is_empty() {
            return;
        }
        self.saving = true;
        let (id, me) = (self.id.clone(), self.me.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let mut error = None;
            for (kind, text) in edits {
                let edit = match kind {
                    FieldKind::Title => Edit::Title(text.clone()),
                    FieldKind::Body => Edit::Body(text.clone()),
                };
                let op = Op::Update {
                    id: id.clone(),
                    edit,
                };
                let task = repo.update(cx, |r, cx| r.run(me.clone(), op, cx));
                match task.await {
                    Ok(a) => {
                        let _ = this.update(cx, |d, cx| {
                            if let Some(f) = d.field_mut(kind) {
                                f.saved_as(&text);
                            }
                            if kind == FieldKind::Body
                                && !d.body.as_ref().is_some_and(Field::is_dirty)
                            {
                                d.clear_mirror();
                            }
                            d.adopt_own(a.path, a.msgs, cx);
                        });
                    }
                    Err(e) => error = Some(e.to_string().trim().replace('\n', " · ")),
                }
            }
            let _ = this.update(cx, |d, cx| {
                d.saving = false;
                d.save_error = error;
                if std::mem::take(&mut d.resave) && d.is_dirty() {
                    d.save_now(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Banner: the disk changed under a draft; overwrite it with the draft.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        for k in [FieldKind::Title, FieldKind::Body] {
            if let Some(f) = self.field_mut(k) {
                f.keep_mine();
            }
        }
        self.save_now(cx);
        cx.notify();
    }

    /// Banner: drop the conflicting drafts and show what's on disk.
    pub fn use_theirs(&mut self, cx: &mut Context<Self>) {
        for k in [FieldKind::Title, FieldKind::Body] {
            let Some(f) = self.field_mut(k) else { continue };
            if f.conflict.is_some() {
                let text = f.use_theirs();
                cx.emit(DocEvent::SetText(k, text));
                if k == FieldKind::Body {
                    self.clear_mirror();
                }
            }
        }
        cx.notify();
    }

    /// Banner: bring back the draft from an earlier session.
    pub fn restore_leftover(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.leftover.take() {
            cx.emit(DocEvent::SetText(FieldKind::Body, text.clone()));
            self.edit(FieldKind::Body, &text, cx);
        }
    }

    pub fn discard_leftover(&mut self, cx: &mut Context<Self>) {
        self.leftover = None;
        self.clear_mirror();
        cx.notify();
    }
}
