//! What every action does (GUI-SPEC §7). All handlers sit on the window
//! root, so an action works from wherever it is dispatched (a key in any
//! focused pane, or the palette); the keymap's contexts decide which keys
//! reach which action.

use super::WorkspaceView;
use crate::actions::{
    AssignToMe, ClearSelection, CloseIssue, ColumnNext, ColumnPrev, CopyIssueLink, CursorDown,
    CursorUp, CycleSort, DeleteIssue, EditBody, GoAllIssues, GoBoard, GoList, GoMyIssues,
    GoProjects, MoveCardNext, MoveCardPrev, MoveToProject, NewIssue, OpenIssue, OpenPalette,
    OpenSearch, OpenSettings, Rename, SelectAll, SetAssignee, SetDue, SetLabels, SetPriority,
    SetState, SwitchRepo1, SwitchRepo2, SwitchRepo3, SwitchRepo4, SwitchRepo5, SwitchRepo6,
    SwitchRepo7, SwitchRepo8, SwitchRepo9, ToggleGrouping, ToggleSelect,
};
use crate::state::draft::FieldKind;
use crate::state::edit::Edit;
use crate::state::palette::PaletteMode;
use crate::state::picker::PickerKind;
use crate::state::view_state::{ListRow, Mode, SORTS, Scope};
use gpui::{Context, Div, InteractiveElement, ScrollStrategy, Stateful, Window};
use lastissue::query::Grouping;

macro_rules! on {
    ($el:ident, $cx:ident, $action:ty, |$this:ident, $window:ident, $c:ident| $body:expr) => {
        $el = $el.on_action($cx.listener(|$this, _: &$action, $window, $c| $body));
    };
}

impl WorkspaceView {
    pub(super) fn bind_actions(
        &mut self,
        mut el: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        // Global.
        on!(el, cx, OpenPalette, |this, window, cx| this.open_palette(
            PaletteMode::All,
            window,
            cx
        ));
        on!(el, cx, GoProjects, |this, window, cx| this.open_palette(
            PaletteMode::Scopes,
            window,
            cx
        ));
        on!(el, cx, OpenSearch, |this, window, cx| this
            .open_search(window, cx));
        on!(el, cx, NewIssue, |this, window, cx| this
            .open_create(window, cx));
        on!(el, cx, GoMyIssues, |this, window, cx| this.go_scope(
            Scope::Mine,
            window,
            cx
        ));
        on!(el, cx, GoAllIssues, |this, window, cx| this.go_scope(
            Scope::All,
            window,
            cx
        ));
        on!(el, cx, GoBoard, |this, window, cx| this.go_mode(
            Mode::Board,
            window,
            cx
        ));
        on!(el, cx, GoList, |this, window, cx| this.go_mode(
            Mode::List,
            window,
            cx
        ));
        on!(el, cx, OpenSettings, |this, _window, cx| this
            .open_settings(cx));
        on!(el, cx, ToggleGrouping, |this, _window, cx| this.ws.update(
            cx,
            |w, cx| {
                w.update_view(
                    |v| {
                        v.grouping = match v.grouping {
                            Grouping::State => Grouping::Priority,
                            Grouping::Priority => Grouping::State,
                        }
                    },
                    cx,
                );
            }
        ));
        on!(el, cx, CycleSort, |this, _window, cx| this.ws.update(
            cx,
            |w, cx| {
                w.update_view(
                    |v| {
                        let i = SORTS.iter().position(|(s, _)| *s == v.sort).unwrap_or(0);
                        v.sort = SORTS[(i + 1) % SORTS.len()].0;
                    },
                    cx,
                );
            }
        ));
        macro_rules! repo {
            ($($a:ident = $i:expr),*) => {
                $(on!(el, cx, $a, |this, window, cx| this.switch_repo($i, window, cx));)*
            };
        }
        repo!(
            SwitchRepo1 = 0,
            SwitchRepo2 = 1,
            SwitchRepo3 = 2,
            SwitchRepo4 = 3,
            SwitchRepo5 = 4,
            SwitchRepo6 = 5,
            SwitchRepo7 = 6,
            SwitchRepo8 = 7,
            SwitchRepo9 = 8
        );

        // List and board.
        on!(el, cx, CursorDown, |this, _window, cx| this
            .move_cursor(true, 1, cx));
        on!(el, cx, CursorUp, |this, _window, cx| this
            .move_cursor(true, -1, cx));
        on!(el, cx, ColumnNext, |this, _window, cx| this
            .move_cursor(false, 1, cx));
        on!(el, cx, ColumnPrev, |this, _window, cx| this
            .move_cursor(false, -1, cx));
        on!(el, cx, OpenIssue, |this, window, cx| {
            this.ws.update(cx, |w, cx| w.open_cursor(cx));
            if this.ws.read(cx).doc.is_some() {
                window.focus(&this.detail_focus, cx);
            }
        });
        on!(el, cx, ToggleSelect, |this, _window, cx| this
            .ws
            .update(cx, |w, cx| w.toggle_select(cx)));
        on!(el, cx, SelectAll, |this, _window, cx| this
            .ws
            .update(cx, |w, cx| w.select_all(cx)));
        on!(el, cx, ClearSelection, |this, window, cx| {
            if this.detail_focus.contains_focused(window, cx) {
                window.focus(&this.focus, cx);
            } else {
                this.ws.update(cx, |w, cx| w.clear(cx));
            }
        });
        on!(el, cx, MoveCardPrev, |this, _window, cx| this
            .move_card(-1, cx));
        on!(el, cx, MoveCardNext, |this, _window, cx| this
            .move_card(1, cx));

        // Issue.
        on!(el, cx, SetState, |this, window, cx| this.pick_for(
            PickerKind::State,
            window,
            cx
        ));
        on!(el, cx, SetPriority, |this, window, cx| this.pick_for(
            PickerKind::Priority,
            window,
            cx
        ));
        on!(el, cx, SetAssignee, |this, window, cx| this.pick_for(
            PickerKind::Assignee,
            window,
            cx
        ));
        on!(el, cx, SetLabels, |this, window, cx| this.pick_for(
            PickerKind::Labels,
            window,
            cx
        ));
        on!(el, cx, SetDue, |this, window, cx| this.pick_for(
            PickerKind::Due,
            window,
            cx
        ));
        on!(el, cx, MoveToProject, |this, window, cx| this.pick_for(
            PickerKind::Project,
            window,
            cx
        ));
        on!(el, cx, AssignToMe, |this, window, cx| {
            let ids = this.targets(window, cx);
            let e = Edit::Assignee(Some("me".into()));
            this.ws.update(cx, |w, cx| w.apply_to(&ids, &e, cx));
        });
        on!(el, cx, CopyIssueLink, |this, window, cx| {
            let ids = this.targets(window, cx);
            let ws = this.ws.read(cx);
            let (Some(id), Some(repo)) = (ids.first(), &ws.repo) else {
                return;
            };
            let url = crate::link::Link {
                repo: repo.read(cx).repo.label.clone(),
                issue: Some(id.clone()),
            }
            .to_url();
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(url));
        });
        on!(el, cx, Rename, |this, window, cx| this.open_field(
            FieldKind::Title,
            window,
            cx
        ));
        on!(el, cx, EditBody, |this, window, cx| this.open_field(
            FieldKind::Body,
            window,
            cx
        ));
        on!(el, cx, CloseIssue, |this, window, cx| {
            this.ws.update(cx, |w, cx| w.select(None, cx));
            window.focus(&this.focus, cx);
        });
        on!(el, cx, DeleteIssue, |this, window, cx| {
            let ids = this.targets(window, cx);
            this.confirm_delete(ids, window, cx);
        });
        el
    }

    /// The issues an issue command acts on (see `Workspace::targets`).
    pub(super) fn targets(&self, window: &Window, cx: &Context<Self>) -> Vec<String> {
        let detail = self.detail_focus.contains_focused(window, cx);
        self.ws.read(cx).targets(detail)
    }

    fn pick_for(&mut self, kind: PickerKind, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.targets(window, cx);
        self.open_picker(kind, ids, window, cx);
    }

    /// `r` / `e`: open the target issue and put the caret in its title or
    /// body.
    fn open_field(&mut self, kind: FieldKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.targets(window, cx).into_iter().next() else {
            return;
        };
        let already = self.ws.read(cx).view.selected.as_deref() == Some(id.as_str());
        if !already {
            self.ws.update(cx, |w, cx| w.select(Some(id), cx));
            self.editors = None;
        }
        self.focus_field(kind, window, cx);
    }

    fn go_scope(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        self.ws
            .update(cx, |w, cx| w.update_view(|v| v.scope = scope, cx));
        window.focus(&self.focus, cx);
    }

    fn go_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.ws
            .update(cx, |w, cx| w.update_view(|v| v.mode = mode, cx));
        window.focus(&self.focus, cx);
    }

    fn switch_repo(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.ws.read(cx).repos.get(i).cloned();
        let Some(repo) = repo.filter(|r| r.exists()) else {
            return;
        };
        let current = self
            .ws
            .read(cx)
            .repo
            .as_ref()
            .map(|r| r.read(cx).repo.root.clone());
        if current.as_ref() != Some(&repo.root) {
            self.close_overlays(cx);
            self.ws.update(cx, |w, cx| w.open_repo(repo, cx));
        }
        window.focus(&self.focus, cx);
    }

    /// `⌘,`: open the keymap overrides in the default editor, creating
    /// them from a template first. (There is no settings window in v1.)
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        let Some(path) = crate::keymap::user_path() else {
            return;
        };
        if !path.exists() {
            let made = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, crate::keymap::USER_TEMPLATE));
            if let Err(e) = made {
                self.ws.update(cx, |w, cx| {
                    w.error = Some(format!("{}: {e}", path.display()));
                    cx.notify();
                });
                return;
            }
        }
        cx.open_with_system(&path);
    }

    /// `j`/`k`/`h`/`l`, then keep the cursor in view.
    fn move_cursor(&mut self, vertical: bool, step: isize, cx: &mut Context<Self>) {
        let mode = self.ws.read(cx).view.mode;
        if !vertical && mode != Mode::Board {
            return;
        }
        self.ws
            .update(cx, |w, cx| w.move_cursor(vertical, step, cx));
        self.scroll_to_cursor(cx);
    }

    pub(super) fn scroll_to_cursor(&mut self, cx: &mut Context<Self>) {
        let ws = self.ws.read(cx);
        let (Some(cur), Some(repo)) = (ws.cursor.as_deref(), &ws.repo) else {
            return;
        };
        let ix = &repo.read(cx).index;
        let is = |p: &std::path::PathBuf| ix.get(p).is_some_and(|s| s.id == cur);
        match ws.view.mode {
            Mode::List => {
                if let Some(i) = ws
                    .visible
                    .rows
                    .iter()
                    .position(|r| matches!(r, ListRow::Issue(p) if is(p)))
                {
                    self.list_scroll.scroll_to_item(i, ScrollStrategy::Nearest);
                }
            }
            Mode::Board => {
                for (c, g) in ws.visible.groups.iter().enumerate() {
                    if let Some(r) = g.items.iter().position(is)
                        && let Some(h) = self.board_scroll.get(c)
                    {
                        h.scroll_to_item(r, ScrollStrategy::Nearest);
                    }
                }
            }
        }
    }

    /// `⇧h` / `⇧l`: the board's keyboard equivalent of dragging.
    fn move_card(&mut self, step: isize, cx: &mut Context<Self>) {
        self.ws.update(cx, |w, cx| {
            if w.view.mode == Mode::Board {
                w.move_selected(step, cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::search::Tier;
    use crate::state::workspace::{FULL_TEXT_DEBOUNCE, Workspace};
    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use lastissue::identity::Identity;
    use lastissue::model::{Actor, State};
    use lastissue::ops::{self, NewIssue};
    use lastissue::store::{self, Store};
    use std::time::Duration;

    struct Fx {
        _dir: tempfile::TempDir,
        s: Store,
        ws: Entity<Workspace>,
        view: Entity<WorkspaceView>,
    }

    fn me() -> Identity {
        Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        })
    }

    fn setup(cx: &mut TestAppContext) -> (Fx, &mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(&dir.path().join("repo")).unwrap();
        let s = Store::new(root.clone());
        for (title, body) in [
            ("Alpha", "first"),
            ("Beta", "second"),
            ("Gamma", "has a needle inside"),
        ] {
            let n = NewIssue {
                title: title.into(),
                body: body.into(),
                state: Some(State::Todo),
                ..NewIssue::default()
            };
            ops::create_issue(&s, &me(), &n).unwrap();
        }
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::keymap::install(None, cx);
        });
        let drafts = dir.path().join("drafts");
        let ws = cx.new(|cx| {
            let mut w = Workspace::new(Some(root), cx);
            w.for_test(me(), &drafts);
            w
        });
        cx.run_until_parked();
        let mut inner = None;
        let (_root, vcx) = cx.add_window_view(|window, cx| {
            let v = cx.new(|cx| WorkspaceView::new(ws.clone(), window, cx));
            inner = Some(v.clone());
            gpui_component::Root::new(v, window, cx)
        });
        let fx = Fx {
            _dir: dir,
            s,
            ws,
            view: inner.unwrap(),
        };
        (fx, vcx)
    }

    fn state_of(s: &Store, id: &str) -> State {
        let r = s.resolve_issue(id).unwrap();
        lastissue::model::state_of(&s.read_head(&r).unwrap())
    }

    fn id_of(f: &Fx, cx: &VisualTestContext, title: &str) -> String {
        f.ws.read_with(cx, |w, cx| {
            w.repo
                .as_ref()
                .unwrap()
                .read(cx)
                .index
                .iter()
                .find(|s| s.title == title)
                .unwrap()
                .id
                .clone()
        })
    }

    /// A triage session with no mouse: move, multi-select, bulk-set state
    /// through the picker, the palette, the board, search with the
    /// full-text tier, `⌘Enter` to filter, open, labels, delete.
    #[gpui::test]
    fn keyboard_triage(cx: &mut TestAppContext) {
        let (f, cx) = setup(cx);
        let col = |cx: &VisualTestContext| f.ws.read_with(cx, |w, cx| w.columns(cx));
        let cols = col(cx);
        assert_eq!(cols[0].len(), 3);

        // j j x k x: select the first two rows.
        cx.simulate_keystrokes("j j x k x");
        let (sel, cursor) =
            f.ws.read_with(cx, |w, _| (w.selection.ids().to_vec(), w.cursor.clone()));
        assert_eq!(sel, [cols[0][1].clone(), cols[0][0].clone()]);
        assert_eq!(cursor.as_deref(), Some(cols[0][0].as_str()));

        // `s` opens one picker for both; type, Enter: a bulk edit.
        cx.simulate_keystrokes("s");
        let targets = f
            .view
            .read_with(cx, |v, _| v.picker.as_ref().map(|p| p.issues.clone()));
        assert_eq!(targets.as_deref(), Some(sel.as_slice()));
        cx.simulate_input("in prog");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(f.view.read_with(cx, |v, _| v.picker.is_none()));
        for id in &sel {
            assert_eq!(state_of(&f.s, id), State::InProgress);
        }
        cx.simulate_keystrokes("escape");
        assert!(f.ws.read_with(cx, |w, _| w.selection.is_empty()));

        // ⌘K → "show board" → Enter runs the action.
        cx.simulate_keystrokes("cmd-k");
        assert!(f.view.read_with(cx, |v, _| v.palette.is_some()));
        cx.simulate_input("show board");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(f.view.read_with(cx, |v, _| v.palette.is_none()));
        assert_eq!(f.ws.read_with(cx, |w, _| w.view.mode), Mode::Board);

        // Board: `h` to the Todo column (Gamma), `⇧l` moves the card on.
        let gamma = id_of(&f, cx, "Gamma");
        cx.simulate_keystrokes("h");
        assert_eq!(
            f.ws.read_with(cx, |w, _| w.cursor.clone()),
            Some(gamma.clone())
        );
        cx.simulate_keystrokes("shift-l");
        cx.run_until_parked();
        assert_eq!(state_of(&f.s, &gamma), State::InProgress);

        // `g l` back to the list; `/` searches bodies after the debounce.
        cx.simulate_keystrokes("g l");
        assert_eq!(f.ws.read_with(cx, |w, _| w.view.mode), Mode::List);
        cx.simulate_keystrokes("/");
        assert!(f.view.read_with(cx, |v, _| v.search.is_some()));
        cx.simulate_input("needle");
        cx.run_until_parked();
        assert!(f.ws.read_with(cx, |w, _| w.search.hits.is_empty() && w.search.scanning));
        cx.executor()
            .advance_clock(FULL_TEXT_DEBOUNCE + Duration::from_millis(10));
        cx.run_until_parked();
        let hits = f.ws.read_with(cx, |w, _| w.search.hits.clone());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].id.as_str(), hits[0].tier),
            (gamma.as_str(), Tier::Body)
        );
        assert_eq!(hits[0].snippets[0].text, "has a needle inside");

        // ⌘Enter: the query becomes the list's filter, body tier included.
        cx.simulate_keystrokes("cmd-enter");
        cx.executor()
            .advance_clock(FULL_TEXT_DEBOUNCE + Duration::from_millis(10));
        cx.run_until_parked();
        assert!(f.view.read_with(cx, |v, _| v.search.is_none()));
        assert_eq!(f.ws.read_with(cx, |w, _| w.view.filter.clone()), "needle");
        assert_eq!(col(cx), [vec![gamma.clone()]]);

        // Enter opens it and focuses the detail pane, where `l` is labels.
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(
            f.ws.read_with(cx, |w, _| w.view.selected.clone()),
            Some(gamma.clone())
        );
        cx.simulate_keystrokes("l");
        let inline = f.view.read_with(cx, |v, cx| {
            v.picker.as_ref().map(|p| p.kind) == Some(PickerKind::Labels) && v.picker_inline(cx)
        });
        assert!(inline);
        cx.simulate_input("triaged");
        cx.simulate_keystrokes("enter escape");
        cx.run_until_parked();
        let labels = f.ws.read_with(cx, |w, cx| {
            w.repo
                .as_ref()
                .unwrap()
                .read(cx)
                .index
                .by_id(&gamma)
                .unwrap()
                .labels
                .clone()
        });
        assert_eq!(labels, ["triaged"]);

        // ⌘⌫ asks first; Enter confirms; the file is gone.
        cx.simulate_keystrokes("cmd-backspace");
        assert!(f.view.read_with(cx, |v, _| v.confirm.is_some()));
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(f.s.resolve_issue(&gamma).is_err());
        assert!(f.ws.read_with(cx, |w, _| w.doc.is_none()));
    }

    /// `c` and `/` stay text inside an input; `g p` opens the project
    /// list; `i` assigns the cursor's issue to me; `r` focuses the title.
    #[gpui::test]
    fn single_letters_skip_inputs(cx: &mut TestAppContext) {
        let (f, cx) = setup(cx);
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("c/");
        assert!(
            f.view
                .read_with(cx, |v, _| v.create.is_none() && v.search.is_none())
        );
        cx.simulate_keystrokes("escape");
        assert!(f.view.read_with(cx, |v, _| v.palette.is_none()));

        cx.simulate_keystrokes("j i");
        cx.run_until_parked();
        let cur = f.ws.read_with(cx, |w, _| w.cursor.clone().unwrap());
        let a = f.ws.read_with(cx, |w, cx| {
            w.repo
                .as_ref()
                .unwrap()
                .read(cx)
                .index
                .by_id(&cur)
                .unwrap()
                .assignee
                .clone()
        });
        assert_eq!(a.as_deref(), Some("Ada <ada@example.com>"));

        cx.simulate_keystrokes("r");
        cx.run_until_parked();
        assert_eq!(
            f.ws.read_with(cx, |w, _| w.view.selected.clone()),
            Some(cur)
        );
        // Typing now edits the title, not the keymap.
        cx.simulate_keystrokes("x");
        assert!(f.ws.read_with(cx, |w, _| w.selection.is_empty()));

        cx.simulate_keystrokes("escape escape g p");
        assert!(f.view.read_with(cx, |v, _| v.palette.is_some()));
    }
}
