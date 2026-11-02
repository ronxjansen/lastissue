//! The one fuzzy-list component (GUI-SPEC §7): a filter input over rows,
//! `↑↓` to move the highlight, `Enter` to pick, `Esc` to close. Property
//! pickers, the `⌘K` palette and the `/` search box all render through
//! [`panel`]; the key handling is wired once, to whichever is open.

use super::WorkspaceView;
use crate::actions::{PICKER, PickerCancel, PickerDown, PickerUp};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, IntoElement, ParentElement, ScrollHandle,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use std::rc::Rc;

/// One row of a fuzzy list.
#[derive(Default)]
pub(super) struct Row {
    pub label: SharedString,
    /// Muted text after the label (an address, a tier).
    pub detail: Option<SharedString>,
    /// `Some` reserves a check column (pickers).
    pub checked: Option<bool>,
    /// Right-aligned key binding (palette).
    pub hint: Option<SharedString>,
    /// Before the label (search: state glyph and ID).
    pub leading: Option<AnyElement>,
    /// Muted lines under the label (search snippets).
    pub lines: Vec<SharedString>,
}

pub(super) type OnPick =
    Rc<dyn Fn(&mut WorkspaceView, usize, &mut Window, &mut Context<WorkspaceView>)>;

pub(super) struct Panel<'a> {
    pub id: &'static str,
    pub input: &'a Entity<InputState>,
    pub rows: Vec<Row>,
    pub highlight: usize,
    pub scroll: &'a ScrollHandle,
    pub width: f32,
    pub empty: &'static str,
    pub header: Option<SharedString>,
    pub footer: Option<AnyElement>,
    pub on_pick: OnPick,
}

pub(super) fn panel(p: Panel, cx: &mut Context<WorkspaceView>) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme().clone();
    let mut list = v_flex()
        .id(SharedString::from(format!("{}-rows", p.id)))
        .track_scroll(p.scroll)
        .max_h(px(360.))
        .overflow_y_scroll()
        .py_1();
    if p.rows.is_empty() {
        list = list.child(super::muted(p.empty, cx).px_3().py_1());
    }
    for (i, r) in p.rows.into_iter().enumerate() {
        let on_pick = p.on_pick.clone();
        let mut label = h_flex()
            .gap_2()
            .min_w(px(0.))
            .children(r.checked.map(|c| {
                div()
                    .w(px(14.))
                    .flex_none()
                    .text_color(theme.primary)
                    .child(if c { "✓" } else { "" })
            }))
            .children(r.leading)
            .child(div().flex_1().min_w(px(0.)).truncate().child(r.label))
            .children(r.detail.map(|d| super::muted(d, cx)))
            .children(r.hint.map(|h| {
                div()
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .bg(theme.secondary)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(h)
            }));
        if r.lines.is_empty() {
            label = label.w_full();
        }
        list = list.child(
            v_flex()
                .id((p.id, i))
                .px_3()
                .py_1()
                .mx_1()
                .rounded_md()
                .cursor_pointer()
                .when(i == p.highlight, |el| el.bg(theme.list_active))
                .hover(|el| el.bg(theme.list_hover))
                .child(label.w_full())
                .children(
                    r.lines
                        .into_iter()
                        .map(|l| super::muted(l, cx).truncate().pl(px(4.))),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    on_pick(this, i, window, cx);
                })),
        );
    }
    v_flex()
        .id(p.id)
        .key_context(PICKER)
        .on_action(cx.listener(|this, _: &PickerUp, _, cx| this.overlay_step(-1, cx)))
        .on_action(cx.listener(|this, _: &PickerDown, _, cx| this.overlay_step(1, cx)))
        .on_action(cx.listener(|this, _: &PickerCancel, window, cx| {
            this.overlay_cancel(window, cx);
        }))
        .occlude()
        .w(px(p.width))
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .shadow_lg()
        .child(
            div()
                .p_1()
                .border_b_1()
                .border_color(theme.border)
                .child(Input::new(p.input).small().appearance(false)),
        )
        .children(p.header.map(|h| super::muted(h, cx).px_3().pt_1()))
        .child(list)
        .children(p.footer.map(|f| {
            div()
                .px_3()
                .py_1()
                .border_t_1()
                .border_color(theme.border)
                .child(f)
        }))
}

/// A full-window layer that centers `panel` near the top and closes the
/// overlay on a click outside it.
pub(super) fn backdrop(
    id: &'static str,
    panel: impl IntoElement,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .justify_center()
        .items_start()
        .pt(px(80.))
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.overlay_cancel(window, cx);
        }))
        .child(panel)
        .into_any_element()
}
