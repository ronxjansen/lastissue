//! The main view's toolbar (List | Board, filter bar, sort, group) and the
//! status line under it.

use super::{WorkspaceView, muted};
use crate::state::view_state::{Mode, SORTS};
use gpui::{ClickEvent, Context, IntoElement, ParentElement, Styled, div, prelude::*, px};
use gpui_component::input::Input;
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex};
use lastissue::query::Grouping;

impl WorkspaceView {
    pub(super) fn render_toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let view = self.ws.read(cx).view.clone();

        let seg = |id: &'static str, label: &'static str, on: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .px_2()
                .py_0p5()
                .rounded_md()
                .cursor_pointer()
                .when(on, |el| el.bg(theme.secondary_active))
                .hover(|el| el.bg(theme.secondary_hover))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.ws.update(cx, |w, cx| {
                        w.update_view(
                            |v| match id {
                                "mode-list" => v.mode = Mode::List,
                                "mode-board" => v.mode = Mode::Board,
                                "sort" => {
                                    let i =
                                        SORTS.iter().position(|(s, _)| *s == v.sort).unwrap_or(0);
                                    v.sort = SORTS[(i + 1) % SORTS.len()].0;
                                }
                                _ => {
                                    v.grouping = match v.grouping {
                                        Grouping::State => Grouping::Priority,
                                        Grouping::Priority => Grouping::State,
                                    }
                                }
                            },
                            cx,
                        );
                    });
                }))
        };
        let sort_label = SORTS
            .iter()
            .find(|(s, _)| *s == view.sort)
            .map_or("Priority", |(_, l)| l);
        let group_label = match view.grouping {
            Grouping::State => "State",
            Grouping::Priority => "Priority",
        };

        h_flex()
            .flex_none()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .p_0p5()
                    .rounded_md()
                    .bg(theme.secondary)
                    .child(seg("mode-list", "List", view.mode == Mode::List, cx))
                    .child(seg("mode-board", "Board", view.mode == Mode::Board, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .child(Input::new(&self.filter).small()),
            )
            .child(muted("Sort", cx))
            .child(seg("sort", sort_label, false, cx))
            .child(muted("Group", cx))
            .child(seg("group", group_label, false, cx))
    }

    pub(super) fn render_status(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ws = self.ws.read(cx);
        let mut parts: Vec<String> = Vec::new();
        let mut warn: Option<String> = ws.visible.error.clone();
        if let Some(repo) = &ws.repo {
            let r = repo.read(cx);
            if r.loaded {
                parts.push(format!(
                    "{} shown · {} in repo",
                    ws.visible.total,
                    r.index.len()
                ));
            } else {
                parts.push("Loading…".into());
            }
            if !r.errors.is_empty() {
                parts.push(format!("{} unreadable", r.errors.len()));
            }
            if let Some(f) = &r.failure {
                warn.get_or_insert_with(|| f.clone());
            }
            parts.push(r.store.root.display().to_string());
        }
        if ws.filter_scanning() {
            parts.insert(1, "searching bodies and comments…".into());
        }
        if !ws.selection.is_empty() {
            parts.insert(0, format!("{} selected (esc clears)", ws.selection.len()));
        }
        if let Some(e) = crate::keymap::errors(cx).first() {
            warn.get_or_insert_with(|| format!("keymap: {e}"));
        }
        h_flex()
            .flex_none()
            .gap_3()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(theme.border)
            .child(muted(parts.join("  ·  "), cx).flex_1().truncate())
            .children(warn.map(|w| {
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .truncate()
                    .max_w(px(480.))
                    .child(w)
            }))
    }
}
