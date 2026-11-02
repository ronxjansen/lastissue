//! Rendering (GUI-SPEC §5). [`WorkspaceView`] is the window's root: it owns
//! view-only state (scroll handles, text inputs, the open picker or modal,
//! expanded event runs) and renders from [`Workspace`]; each pane lives in
//! its own module as an `impl WorkspaceView` block. Every write goes
//! through `Workspace` (core `ops`), never straight to a file.

mod activity;
mod board;
mod create;
mod detail;
pub mod format;
mod fuzzy;
mod keys;
mod list;
pub mod palette;
mod picker;
mod sidebar;
mod toolbar;

use crate::actions::{ISSUE_DETAIL, ISSUES, WORKSPACE};
use crate::state::draft::FieldKind;
use crate::state::issue_doc::{DocEvent, IssueDoc};
use crate::state::view_state::Mode;
use crate::state::workspace::Workspace;
use gpui::{
    App, Context, Entity, EntityId, FocusHandle, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, UniformListScrollHandle, Window, div, prelude::*, px,
};
use gpui_component::input::{InputEvent, InputState, TextareaState};
use gpui_component::{ActiveTheme as _, Theme, h_flex, v_flex};
use std::collections::HashSet;

/// The text inputs of the open issue, created when it has loaded and
/// dropped when another issue opens.
pub(super) struct Editors {
    doc: Entity<IssueDoc>,
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    /// The body shows as an editor (after a click or `e`), else rendered.
    editing_body: bool,
    composer: Entity<TextareaState>,
    /// The comment being edited (Message-ID) and its editor.
    comment_edit: Option<(String, Entity<TextareaState>)>,
    /// A comment whose delete awaits confirmation.
    confirm_delete: Option<String>,
    subs: Vec<Subscription>,
}

pub struct WorkspaceView {
    ws: Entity<Workspace>,
    /// The list or board: where focus rests (context `Issues`).
    focus: FocusHandle,
    /// The detail pane (context `IssueDetail`), after `Enter`/`o`.
    detail_focus: FocusHandle,
    filter: Entity<InputState>,
    list_scroll: UniformListScrollHandle,
    board_scroll: Vec<UniformListScrollHandle>,
    commands: Vec<crate::actions::Command>,
    /// Focus this field once the open issue's editors exist (`r`, `e`).
    pending_focus: Option<FieldKind>,
    palette: Option<palette::PaletteUi>,
    search: Option<palette::SearchUi>,
    confirm: Option<palette::ConfirmUi>,
    repo_menu_open: bool,
    /// Event runs expanded in the activity timeline (index of the run's
    /// first message), reset when another issue opens.
    expanded_runs: HashSet<usize>,
    expanded_for: Option<String>,
    editors: Option<Editors>,
    /// Re-render when the open doc changes (it loads after it opens).
    doc_watch: Option<(EntityId, Subscription)>,
    picker: Option<picker::PickerUi>,
    create: Option<create::CreateUi>,
    _subs: Vec<Subscription>,
}

impl WorkspaceView {
    pub fn new(ws: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter: state:todo p:high label:bug @me is:ready due:<7d …")
        });
        let subs = vec![
            cx.observe_in(&ws, window, |this, ws, window, cx| {
                // A `lastissue://` link opened an issue.
                if ws.read(cx).focus_detail {
                    ws.update(cx, |w, _| w.focus_detail = false);
                    if this.ws.read(cx).doc.is_some() {
                        window.focus(&this.detail_focus, cx);
                    }
                }
                cx.notify();
            }),
            cx.subscribe_in(
                &filter,
                window,
                |this, input, ev: &InputEvent, window, cx| {
                    match ev {
                        InputEvent::Change => {
                            let text = input.read(cx).value().to_string();
                            this.ws
                                .update(cx, |w, cx| w.update_view(|v| v.filter = text, cx));
                        }
                        // Back to the list, filter applied.
                        InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
                        _ => {}
                    }
                },
            ),
            cx.observe_window_appearance(window, |_, window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
                cx.notify();
            }),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        WorkspaceView {
            ws,
            focus,
            detail_focus: cx.focus_handle(),
            filter,
            list_scroll: UniformListScrollHandle::new(),
            board_scroll: Vec::new(),
            commands: crate::actions::commands(),
            pending_focus: None,
            palette: None,
            search: None,
            confirm: None,
            repo_menu_open: false,
            expanded_runs: HashSet::new(),
            expanded_for: None,
            editors: None,
            doc_watch: None,
            picker: None,
            create: None,
            _subs: subs,
        }
    }

    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.ws.update(cx, |w, cx| w.select(Some(id), cx));
        // Take keyboard focus back from any input, so `⇧h`/`e`/… work.
        window.focus(&self.focus, cx);
    }

    /// Keep [`Editors`] in step with the open doc.
    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.ws.read(cx).doc.clone();
        let Some(doc) = doc else {
            self.editors = None;
            self.doc_watch = None;
            return;
        };
        if self.doc_watch.as_ref().map(|(id, _)| *id) != Some(doc.entity_id()) {
            let sub = cx.observe(&doc, |_, _, cx| cx.notify());
            self.doc_watch = Some((doc.entity_id(), sub));
            self.editors = None;
            self.picker = None;
        }
        if self.editors.is_some() {
            return;
        }
        let (title_text, body_text) = {
            let d = doc.read(cx);
            match (&d.title, &d.body) {
                (Some(t), Some(b)) => (t.value().to_string(), b.value().to_string()),
                _ => return, // still loading
            }
        };
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(title_text)
                .placeholder("Issue title")
        });
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(6, 40)
                .default_value(body_text)
                .placeholder("Add a description… (Markdown)")
        });
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 16)
                .placeholder("Leave a comment… (⌘Enter to send)")
        });
        let d1 = doc.clone();
        let d2 = doc.clone();
        let subs = vec![
            cx.subscribe_in(
                &title,
                window,
                move |this, input, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let t = input.read(cx).value().to_string();
                        d1.update(cx, |d, cx| d.edit(FieldKind::Title, &t, cx));
                    }
                    InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
                    InputEvent::Blur => d1.update(cx, |d, cx| d.save_now(cx)),
                    InputEvent::Focus => {}
                },
            ),
            cx.subscribe_in(
                &body,
                window,
                move |this, input, ev: &InputEvent, window, cx| match ev {
                    // `⌘Enter`: done editing (saves on blur).
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => window.focus(&this.detail_focus, cx),
                    InputEvent::Change => {
                        let t = input.read(cx).value().to_string();
                        d2.update(cx, |d, cx| d.edit(FieldKind::Body, &t, cx));
                    }
                    InputEvent::Blur => {
                        d2.update(cx, |d, cx| d.save_now(cx));
                        if let Some(e) = &mut this.editors {
                            e.editing_body = false;
                        }
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&composer, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = ev
                {
                    this.submit_comment(window, cx);
                }
            }),
            cx.subscribe_in(&doc, window, |this, _, ev: &DocEvent, window, cx| {
                let DocEvent::SetText(kind, text) = ev;
                let Some(e) = &this.editors else { return };
                match kind {
                    FieldKind::Title => e
                        .title
                        .update(cx, |i, cx| i.set_value(text.clone(), window, cx)),
                    FieldKind::Body => e
                        .body
                        .update(cx, |i, cx| i.set_value(text.clone(), window, cx)),
                }
            }),
        ];
        self.editors = Some(Editors {
            doc,
            title,
            body,
            editing_body: false,
            composer,
            comment_edit: None,
            confirm_delete: None,
            subs,
        });
        if let Some(kind) = self.pending_focus.take() {
            self.focus_field(kind, window, cx);
        }
    }

    fn edit_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_field(FieldKind::Body, window, cx);
    }

    /// Put the caret in the open issue's title (selected, for `r`) or
    /// body (`e`), now or once its editors exist.
    fn focus_field(&mut self, kind: FieldKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &mut self.editors else {
            self.pending_focus = Some(kind);
            return;
        };
        match kind {
            FieldKind::Title => {
                let t = e.title.clone();
                t.update(cx, |t, cx| {
                    t.focus(window, cx);
                    t.select_all(window, cx);
                });
            }
            FieldKind::Body => {
                e.editing_body = true;
                let body = e.body.clone();
                body.update(cx, |b, cx| b.focus(window, cx));
            }
        }
        cx.notify();
    }
}

/// A muted, small, single-line label.
fn muted(text: impl Into<SharedString>, cx: &App) -> gpui::Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .whitespace_nowrap()
        .child(text.into())
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_editors(window, cx);
        let theme = cx.theme().clone();
        let ws = self.ws.read(cx);
        let has_doc = ws.doc.is_some();
        let repo_open = ws.repo.is_some();
        let notice = ws.notice.clone();
        let mode = ws.view.mode;
        let main: gpui::AnyElement = if repo_open {
            let body = match mode {
                Mode::List => self.render_list(cx).into_any_element(),
                Mode::Board => self.render_board(cx).into_any_element(),
            };
            let context = match mode {
                Mode::List => format!("{ISSUES} mode=list"),
                Mode::Board => format!("{ISSUES} mode=board"),
            };
            v_flex()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .child(self.render_toolbar(cx))
                .child(
                    v_flex()
                        .id("issues")
                        .key_context(context.as_str())
                        .track_focus(&self.focus)
                        .flex_1()
                        .min_h(px(0.))
                        .child(body),
                )
                .child(self.render_status(cx))
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_lg().child("No issues root open"))
                .child(muted(
                    notice.unwrap_or_else(|| "Run `lastissue init`, or pass a root path.".into()),
                    cx,
                ))
                .into_any_element()
        };
        let sidebar = self.render_sidebar(cx).into_any_element();
        let detail = has_doc.then(|| {
            div()
                .id("detail-focus")
                .key_context(ISSUE_DETAIL)
                .track_focus(&self.detail_focus)
                .h_full()
                .flex_none()
                .child(self.render_detail(window, cx))
                .into_any_element()
        });
        let modal = self
            .create
            .is_some()
            .then(|| self.render_create(cx).into_any_element());
        let overlays: Vec<gpui::AnyElement> = [
            self.render_floating_picker(cx),
            self.render_palette(cx),
            self.render_search(cx),
            self.render_confirm(cx),
        ]
        .into_iter()
        .flatten()
        .collect();
        let root = h_flex()
            .id("workspace")
            .key_context(WORKSPACE)
            .relative()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_sm()
            .child(sidebar)
            .child(main)
            .children(detail)
            .children(modal)
            .children(overlays);
        self.bind_actions(root, cx)
    }
}
