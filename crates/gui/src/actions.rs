//! Actions and the command registry (GUI-SPEC §7). Every user-facing
//! action is an `actions!` type, listed in [`commands`] for the `⌘K`
//! palette; its binding comes from the keymap (`default-keymap.json` plus
//! the user's `gui-keymap.json`, see [`crate::keymap`]).

use gpui::{Action, actions};

actions!(
    lastissue,
    [
        Quit,
        // Global.
        OpenPalette,
        OpenSearch,
        NewIssue,
        GoMyIssues,
        GoAllIssues,
        GoProjects,
        GoBoard,
        GoList,
        SwitchRepo1,
        SwitchRepo2,
        SwitchRepo3,
        SwitchRepo4,
        SwitchRepo5,
        SwitchRepo6,
        SwitchRepo7,
        SwitchRepo8,
        SwitchRepo9,
        OpenSettings,
        ToggleGrouping,
        CycleSort,
        // List and board.
        CursorDown,
        CursorUp,
        ColumnPrev,
        ColumnNext,
        OpenIssue,
        ToggleSelect,
        SelectAll,
        ClearSelection,
        MoveCardPrev,
        MoveCardNext,
        // Issue (the open one, the selection, or the cursor's).
        SetState,
        SetPriority,
        SetAssignee,
        AssignToMe,
        SetLabels,
        SetDue,
        MoveToProject,
        Rename,
        EditBody,
        CloseIssue,
        DeleteIssue,
        CopyIssueLink,
        // Pickers, palette, search, modals (not in the palette).
        PickerUp,
        PickerDown,
        PickerCancel,
        ModalSubmit,
        ModalCancel,
    ]
);

/// The window root's key context.
pub const WORKSPACE: &str = "Workspace";
/// The list or board (with `mode=list|board`), when it has focus.
pub const ISSUES: &str = "Issues";
/// The detail pane, when it has focus.
pub const ISSUE_DETAIL: &str = "IssueDetail";
/// A property picker, the palette or the search box.
pub const PICKER: &str = "Picker";
/// The create-issue modal.
pub const CREATE_MODAL: &str = "CreateModal";
/// A confirmation dialog (delete).
pub const CONFIRM: &str = "Confirm";

/// One palette entry for an action.
pub struct Command {
    pub title: &'static str,
    pub build: fn() -> Box<dyn Action>,
}

macro_rules! cmds {
    ($($title:literal => $action:ident),* $(,)?) => {
        vec![$(Command { title: $title, build: || Box::new($action) }),*]
    };
}

/// Every action the palette offers, in display order (ties in matching
/// keep this order). Repo switching is offered per repo instead (see
/// [`switch_repo`]); picker and modal plumbing isn't offered at all.
pub fn commands() -> Vec<Command> {
    cmds![
        "Search issues" => OpenSearch,
        "New issue" => NewIssue,
        "Go to my issues" => GoMyIssues,
        "Go to all issues" => GoAllIssues,
        "Go to project…" => GoProjects,
        "Show board" => GoBoard,
        "Show list" => GoList,
        "Toggle grouping (state / priority)" => ToggleGrouping,
        "Cycle sort order" => CycleSort,
        "Change state…" => SetState,
        "Set priority…" => SetPriority,
        "Assign to…" => SetAssignee,
        "Assign to me" => AssignToMe,
        "Edit labels…" => SetLabels,
        "Set due date…" => SetDue,
        "Move to project…" => MoveToProject,
        "Rename issue" => Rename,
        "Edit description" => EditBody,
        "Copy issue link (lastissue://)" => CopyIssueLink,
        "Delete issue…" => DeleteIssue,
        "Move card to previous state" => MoveCardPrev,
        "Move card to next state" => MoveCardNext,
        "Open issue" => OpenIssue,
        "Close issue pane" => CloseIssue,
        "Next issue" => CursorDown,
        "Previous issue" => CursorUp,
        "Next column" => ColumnNext,
        "Previous column" => ColumnPrev,
        "Toggle selection" => ToggleSelect,
        "Select all" => SelectAll,
        "Clear selection / back" => ClearSelection,
        "Open settings (keymap)" => OpenSettings,
        "Command palette" => OpenPalette,
        "Quit" => Quit,
    ]
}

/// The `⌘1..9` action for repo `i` (0-based).
pub fn switch_repo(i: usize) -> Option<Box<dyn Action>> {
    let a: Box<dyn Action> = match i {
        0 => Box::new(SwitchRepo1),
        1 => Box::new(SwitchRepo2),
        2 => Box::new(SwitchRepo3),
        3 => Box::new(SwitchRepo4),
        4 => Box::new(SwitchRepo5),
        5 => Box::new(SwitchRepo6),
        6 => Box::new(SwitchRepo7),
        7 => Box::new(SwitchRepo8),
        8 => Box::new(SwitchRepo9),
        _ => return None,
    };
    Some(a)
}

/// Actions bound in the keymap but deliberately not in the palette.
#[cfg(test)]
pub const INTERNAL: &[&str] = &[
    "lastissue::PickerUp",
    "lastissue::PickerDown",
    "lastissue::PickerCancel",
    "lastissue::ModalSubmit",
    "lastissue::ModalCancel",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_actions_are_distinct() {
        let names: Vec<&str> = (0..9).map(|i| switch_repo(i).unwrap().name()).collect();
        assert_eq!(names[0], "lastissue::SwitchRepo1");
        assert_eq!(names[8], "lastissue::SwitchRepo9");
        assert!(switch_repo(9).is_none());
    }

    #[test]
    fn command_titles_are_unique() {
        let mut t: Vec<&str> = commands().iter().map(|c| c.title).collect();
        t.sort_unstable();
        let n = t.len();
        t.dedup();
        assert_eq!(t.len(), n);
    }
}
