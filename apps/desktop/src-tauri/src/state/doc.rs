//! The issue open in the detail pane: its messages,
//! the title and body drafts, the crash-guard mirror and the "changed on
//! disk" conflict state. Plain data: the workspace does the I/O and the
//! front-end owns the debounce timer (it calls save ~300 ms after the last
//! keystroke).

use super::draft::{self, DiskChange, Drafts, Field, FieldKind};
use lastissue::store::mbox::Msg;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Doc {
    /// Distinguishes this open from a later open of the same issue, so a
    /// slow load or save can tell it's stale.
    pub generation: u64,
    pub id: String,
    pub path: PathBuf,
    pub msgs: Option<Vec<Msg>>,
    pub error: Option<String>,
    pub title: Option<Field>,
    pub body: Option<Field>,
    pub save_error: Option<String>,
    pub leftover: Option<String>,
    /// Bumped whenever the editors must replace their text with the
    /// fields' values.
    pub reset: u64,
}

impl Doc {
    pub fn new(generation: u64, id: String, path: PathBuf) -> Self {
        Doc {
            generation,
            id,
            path,
            msgs: None,
            error: None,
            title: None,
            body: None,
            save_error: None,
            leftover: None,
            reset: 0,
        }
    }

    pub fn message_id(&self) -> Option<String> {
        Some(self.msgs.as_ref()?.first()?.message_id().to_string())
    }

    /// Fresh messages from disk: the first load sets the fields (and finds
    /// a leftover draft); later loads adopt disk changes or flag a conflict.
    pub fn loaded(&mut self, msgs: Vec<Msg>, drafts: Option<&Drafts>) {
        let head = &msgs[0];
        let disk_title = head.get("Subject").unwrap_or("").to_string();
        let disk_body = head.body.clone();
        let mid = head.message_id().to_string();
        self.error = None;
        self.msgs = Some(msgs);
        for (kind, disk) in [(FieldKind::Title, disk_title), (FieldKind::Body, disk_body)] {
            let slot = match kind {
                FieldKind::Title => &mut self.title,
                FieldKind::Body => &mut self.body,
            };
            match slot {
                None => {
                    *slot = Some(Field::new(kind, &disk));
                    self.reset += 1;
                    if kind == FieldKind::Body {
                        self.leftover = drafts.and_then(|d| draft::leftover(d, &mid, &disk));
                    }
                }
                Some(f) => {
                    if f.disk(&disk) == DiskChange::Adopt {
                        self.reset += 1;
                    }
                }
            }
        }
    }

    /// Our own write's result: new messages (and path, after a move).
    pub fn adopt_own(&mut self, path: PathBuf, msgs: Vec<Msg>) {
        self.path = path;
        self.msgs = Some(msgs);
    }

    pub fn field(&self, kind: FieldKind) -> Option<&Field> {
        match kind {
            FieldKind::Title => self.title.as_ref(),
            FieldKind::Body => self.body.as_ref(),
        }
    }

    pub fn field_mut(&mut self, kind: FieldKind) -> Option<&mut Field> {
        match kind {
            FieldKind::Title => self.title.as_mut(),
            FieldKind::Body => self.body.as_mut(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.title.as_ref().is_some_and(Field::is_dirty)
            || self.body.as_ref().is_some_and(Field::is_dirty)
    }

    /// The editor's text changed. Returns whether a save is due.
    pub fn edit(&mut self, kind: FieldKind, text: &str, drafts: Option<&Drafts>) -> bool {
        let mid = self.message_id();
        let Some(f) = self.field_mut(kind) else {
            return false;
        };
        if f.value() == text {
            return f.is_dirty();
        }
        let dirty = f.edit(text);
        if !dirty && kind == FieldKind::Body {
            clear_mirror(drafts, mid.as_deref());
        }
        dirty
    }

    /// Mirror the body draft to the crash guard (or clear it).
    pub fn mirror(&self, drafts: Option<&Drafts>) {
        if let (Some(d), Some(mid), Some(b)) = (drafts, self.message_id(), &self.body) {
            match &b.draft {
                Some(text) => {
                    if let Err(e) = d.save(&mid, text) {
                        eprintln!("lastissue-gui: cannot keep the draft of {}: {e}", self.id);
                    }
                }
                None => d.clear(&mid),
            }
        }
    }

    /// What a save writes now. Nothing while a conflict is open.
    pub fn to_save(&self) -> Vec<(FieldKind, String)> {
        [FieldKind::Title, FieldKind::Body]
            .into_iter()
            .filter_map(|k| Some((k, self.field(k)?.to_save()?)))
            .collect()
    }

    /// A save of `kind` as `text` went through.
    pub fn saved(&mut self, kind: FieldKind, text: &str, drafts: Option<&Drafts>) {
        if let Some(f) = self.field_mut(kind) {
            f.saved_as(text);
        }
        if kind == FieldKind::Body && !self.body.as_ref().is_some_and(Field::is_dirty) {
            clear_mirror(drafts, self.message_id().as_deref());
        }
    }

    pub fn keep_mine(&mut self) {
        for k in [FieldKind::Title, FieldKind::Body] {
            if let Some(f) = self.field_mut(k) {
                f.keep_mine();
            }
        }
    }

    pub fn use_theirs(&mut self, drafts: Option<&Drafts>) {
        let mid = self.message_id();
        for k in [FieldKind::Title, FieldKind::Body] {
            let Some(f) = self.field_mut(k) else { continue };
            if f.conflict.is_some() {
                f.use_theirs();
                if k == FieldKind::Body {
                    clear_mirror(drafts, mid.as_deref());
                }
            }
        }
        self.reset += 1;
    }

    /// Put the leftover draft into the body. Returns whether a save is due.
    pub fn restore_leftover(&mut self, drafts: Option<&Drafts>) -> bool {
        let Some(text) = self.leftover.take() else {
            return false;
        };
        let dirty = self.edit(FieldKind::Body, &text, drafts);
        self.reset += 1;
        dirty
    }

    pub fn discard_leftover(&mut self, drafts: Option<&Drafts>) {
        self.leftover = None;
        clear_mirror(drafts, self.message_id().as_deref());
    }
}

fn clear_mirror(drafts: Option<&Drafts>, mid: Option<&str>) {
    if let (Some(d), Some(mid)) = (drafts, mid) {
        d.clear(mid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::store::mbox;

    fn msgs(title: &str, body: &str) -> Vec<Msg> {
        let mut m = Msg::default();
        m.headers
            .push(("Message-ID".into(), "<01JTEST@lastissue>".into()));
        m.headers.push(("Subject".into(), title.into()));
        m.body = mbox::normalize_body(body);
        vec![m]
    }

    #[test]
    fn load_edit_save_and_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = Drafts::new(dir.path());
        let mut d = Doc::new(1, "abc".into(), "a.mbox".into());
        d.loaded(msgs("T", "one\n"), Some(&drafts));
        assert_eq!(d.reset, 2);
        assert!(!d.is_dirty());

        assert!(d.edit(FieldKind::Body, "two\n", Some(&drafts)));
        d.mirror(Some(&drafts));
        let save = d.to_save();
        assert_eq!(save, [(FieldKind::Body, "two\n".to_string())]);
        // Mirrored to the crash guard until saved.
        assert_eq!(drafts.load("<01JTEST@lastissue>").as_deref(), Some("two\n"));
        d.saved(FieldKind::Body, "two\n", Some(&drafts));
        assert!(!d.is_dirty());
        assert!(drafts.load("<01JTEST@lastissue>").is_none());

        // An external change with no draft is adopted; with one, a conflict.
        d.loaded(msgs("T", "three\n"), Some(&drafts));
        assert_eq!(d.body.as_ref().unwrap().value(), "three");
        assert_eq!(d.reset, 3);
        d.edit(FieldKind::Body, "mine\n", Some(&drafts));
        d.loaded(msgs("T", "theirs\n"), Some(&drafts));
        assert!(d.body.as_ref().unwrap().conflict.is_some());
        assert!(d.to_save().is_empty());
        d.use_theirs(Some(&drafts));
        assert_eq!(d.body.as_ref().unwrap().value(), "theirs");
        assert!(!d.is_dirty());
    }

    #[test]
    fn leftover_drafts_are_offered_back() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = Drafts::new(dir.path());
        drafts.save("<01JTEST@lastissue>", "unsaved\n").unwrap();
        let mut d = Doc::new(1, "abc".into(), "a.mbox".into());
        d.loaded(msgs("T", "disk\n"), Some(&drafts));
        assert_eq!(d.leftover.as_deref(), Some("unsaved\n"));
        assert!(d.restore_leftover(Some(&drafts)));
        assert_eq!(d.body.as_ref().unwrap().value(), "unsaved\n");
    }
}
