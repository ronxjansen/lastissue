//! Colors and glyphs derived from the active gpui-component theme, so light
//! and dark switch in one place. M4 adds the prose `EditorTheme` and the
//! markdown `TextView` style here (GUI-SPEC §5.3).

use gpui::Hsla;
use gpui_component::Theme;
use lastissue::model::{Priority, State};

pub fn state_color(s: State, t: &Theme) -> Hsla {
    match s {
        State::Backlog => t.muted_foreground,
        State::Todo => t.foreground,
        State::InProgress => t.yellow,
        State::InReview => t.blue,
        State::Done => t.green,
        State::Cancelled | State::Duplicate => t.muted_foreground,
    }
}

/// A one-character state marker.
pub fn state_glyph(s: State) -> &'static str {
    match s {
        State::Backlog => "◌",
        State::Todo => "○",
        State::InProgress => "◐",
        State::InReview => "◑",
        State::Done => "●",
        State::Cancelled | State::Duplicate => "⊘",
    }
}

pub fn priority_glyph(p: Option<Priority>) -> &'static str {
    match p {
        Some(Priority::Urgent) => "!!",
        Some(Priority::High) => "▃▅▇",
        Some(Priority::Medium) => "▃▅",
        Some(Priority::Low) => "▃",
        None => "–",
    }
}

pub fn priority_color(p: Option<Priority>, t: &Theme) -> Hsla {
    match p {
        Some(Priority::Urgent) => t.red,
        Some(Priority::High) => t.foreground,
        _ => t.muted_foreground,
    }
}
