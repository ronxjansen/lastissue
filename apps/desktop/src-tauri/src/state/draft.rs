//! Draft bookkeeping for the open issue's title and body,
//! plain data so it tests directly.
//!
//! A [`Field`] knows the text last seen on disk (`saved`), the user's
//! unsaved text (`draft`) and, when the file changed underneath a draft, the
//! disk's new text (`conflict`). Values are compared normalized (as core
//! stores them), so a save that core trims doesn't look like an external
//! change. While a conflict is open nothing autosaves; the user picks
//! "keep mine" or "use theirs" in the banner.
//!
//! [`Drafts`] is the crash guard: unsaved body drafts mirrored to
//! `<cache>/drafts/<ulid>.md` and offered back on the next open.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Title,
    Body,
}

impl FieldKind {
    pub fn normalize(self, s: &str) -> String {
        match self {
            FieldKind::Title => lastissue::store::mbox::clean_value(s),
            FieldKind::Body => lastissue::store::mbox::normalize_body(s),
        }
    }
}

/// What a disk read means for a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskChange {
    /// Same as what we have (including our own write coming back).
    None,
    /// No draft: the editor should show the new text.
    Adopt,
    /// A draft is in the way: the banner asks.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub kind: FieldKind,
    /// Normalized text on disk, as last read or written.
    pub saved: String,
    /// The editor's text when it differs from `saved` (raw, as typed).
    pub draft: Option<String>,
    /// Normalized disk text that arrived while a draft was open.
    pub conflict: Option<String>,
}

impl Field {
    pub fn new(kind: FieldKind, disk: &str) -> Self {
        Field {
            kind,
            saved: kind.normalize(disk),
            draft: None,
            conflict: None,
        }
    }

    /// The text the editor shows.
    pub fn value(&self) -> &str {
        self.draft.as_deref().unwrap_or(&self.saved)
    }

    pub fn is_dirty(&self) -> bool {
        self.draft.is_some()
    }

    /// The draft to write now: dirty, no open conflict, and valid (a title
    /// can't be empty; core refuses it).
    pub fn to_save(&self) -> Option<String> {
        let d = self.draft.as_ref()?;
        if self.conflict.is_some() || (self.kind == FieldKind::Title && d.trim().is_empty()) {
            return None;
        }
        Some(d.clone())
    }

    /// The user typed: returns whether a save is now pending.
    pub fn edit(&mut self, text: &str) -> bool {
        if self.kind.normalize(text) == self.saved {
            self.draft = None;
        } else {
            self.draft = Some(text.to_string());
        }
        self.is_dirty()
    }

    /// `text` reached the disk. A draft typed since stays a draft.
    pub fn saved_as(&mut self, text: &str) {
        self.saved = self.kind.normalize(text);
        if self
            .draft
            .as_deref()
            .is_some_and(|d| self.kind.normalize(d) == self.saved)
        {
            self.draft = None;
        }
    }

    /// The file was re-read after someone else wrote it.
    pub fn disk(&mut self, disk: &str) -> DiskChange {
        let disk = self.kind.normalize(disk);
        if disk == self.saved {
            return DiskChange::None;
        }
        match &self.draft {
            None => {
                self.saved = disk;
                self.conflict = None;
                DiskChange::Adopt
            }
            // They wrote exactly what we have.
            Some(d) if self.kind.normalize(d) == disk => {
                self.saved = disk;
                self.draft = None;
                self.conflict = None;
                DiskChange::None
            }
            Some(_) => {
                self.conflict = Some(disk);
                DiskChange::Conflict
            }
        }
    }

    /// Banner: overwrite theirs with the draft (the next save does it).
    pub fn keep_mine(&mut self) {
        if let Some(c) = self.conflict.take() {
            self.saved = c;
        }
    }

    /// Banner: drop the draft and show the disk text.
    pub fn use_theirs(&mut self) {
        if let Some(c) = self.conflict.take() {
            self.saved = c;
        }
        self.draft = None;
    }
}

/// Unsaved body drafts on disk, one file per issue.
#[derive(Debug, Clone)]
pub struct Drafts {
    dir: PathBuf,
}

impl Drafts {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Drafts { dir: dir.into() }
    }

    /// `<platform cache dir>/drafts` (`~/Library/Caches/lastissue/drafts`).
    pub fn platform() -> Option<Self> {
        let d = directories::ProjectDirs::from("", "", "lastissue")?;
        Some(Drafts::new(d.cache_dir().join("drafts")))
    }

    fn path(&self, message_id: &str) -> PathBuf {
        let ulid = lastissue::store::mbox::ulid_of(message_id);
        let name: String = ulid
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.')
            .collect();
        self.dir.join(format!("{name}.md"))
    }

    pub fn save(&self, message_id: &str, text: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_atomic(&self.path(message_id), text)
    }

    pub fn load(&self, message_id: &str) -> Option<String> {
        std::fs::read_to_string(self.path(message_id)).ok()
    }

    pub fn clear(&self, message_id: &str) {
        let _ = std::fs::remove_file(self.path(message_id));
    }
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// A draft left from an earlier session worth offering back: there is one
/// and it differs from what's on disk now.
pub fn leftover(drafts: &Drafts, message_id: &str, disk_body: &str) -> Option<String> {
    let d = drafts.load(message_id)?;
    if FieldKind::Body.normalize(&d) == FieldKind::Body.normalize(disk_body) {
        drafts.clear(message_id);
        return None;
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_back_to_saved_clears_the_draft() {
        let mut f = Field::new(FieldKind::Body, "Hello\n");
        assert_eq!(f.saved, "Hello");
        assert!(f.edit("Hello!"));
        assert_eq!(f.to_save().as_deref(), Some("Hello!"));
        assert!(!f.edit("Hello  \n"));
        assert_eq!(f.to_save(), None);
    }

    #[test]
    fn save_keeps_newer_typing() {
        let mut f = Field::new(FieldKind::Body, "a");
        f.edit("ab");
        let sent = f.to_save().unwrap();
        f.edit("abc"); // typed while the save was in flight
        f.saved_as(&sent);
        assert_eq!(f.saved, "ab");
        assert_eq!(f.to_save().as_deref(), Some("abc"));
        f.saved_as("abc");
        assert!(!f.is_dirty());
    }

    #[test]
    fn external_change_without_draft_is_adopted() {
        let mut f = Field::new(FieldKind::Title, "Old");
        assert_eq!(f.disk("Old"), DiskChange::None);
        assert_eq!(f.disk("New"), DiskChange::Adopt);
        assert_eq!(f.value(), "New");
    }

    #[test]
    fn external_change_under_a_draft_conflicts_and_pauses_saving() {
        let mut f = Field::new(FieldKind::Body, "base");
        f.edit("mine");
        assert_eq!(f.disk("theirs"), DiskChange::Conflict);
        assert_eq!(f.to_save(), None, "no autosave over a conflict");
        assert_eq!(f.value(), "mine", "the draft stays in the editor");

        let mut keep = f.clone();
        keep.keep_mine();
        assert_eq!(keep.saved, "theirs");
        assert_eq!(keep.to_save().as_deref(), Some("mine"));

        let mut theirs = f.clone();
        theirs.use_theirs();
        assert_eq!(theirs.value(), "theirs");
        assert!(!theirs.is_dirty() && theirs.conflict.is_none());
    }

    #[test]
    fn external_write_equal_to_draft_is_no_conflict() {
        let mut f = Field::new(FieldKind::Body, "base");
        f.edit("same\n");
        assert_eq!(f.disk("same"), DiskChange::None);
        assert!(!f.is_dirty());
    }

    #[test]
    fn empty_title_is_not_saved() {
        let mut f = Field::new(FieldKind::Title, "T");
        f.edit("   ");
        assert!(f.is_dirty());
        assert_eq!(f.to_save(), None);
    }

    #[test]
    fn drafts_round_trip_and_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let d = Drafts::new(dir.path().join("drafts"));
        let mid = "<01JABCDEF@lastissue>";
        assert_eq!(d.load(mid), None);
        d.save(mid, "unsaved").unwrap();
        assert_eq!(d.load(mid).as_deref(), Some("unsaved"));
        assert_eq!(leftover(&d, mid, "on disk").as_deref(), Some("unsaved"));
        // Same as disk: nothing to offer, and the stale file goes.
        assert_eq!(leftover(&d, mid, "unsaved\n"), None);
        assert_eq!(d.load(mid), None);
        d.save(mid, "x").unwrap();
        d.clear(mid);
        assert_eq!(d.load(mid), None);
    }
}
