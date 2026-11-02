//! The list view: one virtualized list of group headers and issue rows
//! (GUI-SPEC §3, §5). Rows have a fixed height, so render cost doesn't grow
//! with issue count.

use super::format::{ago, display_name};
use super::{WorkspaceView, muted};
use crate::state::view_state::{ListRow, group_label};
use crate::theme;
use gpui::{
    AnyElement, App, ClickEvent, Context, IntoElement, ParentElement, Styled, div, prelude::*, px,
    uniform_list,
};
use gpui_component::{ActiveTheme as _, h_flex};
use lastissue::source::IssueSummary;

pub(super) const ROW_HEIGHT: f32 = 32.;

/// A label chip at its natural width.
pub(super) fn chip(label: &str, cx: &App) -> gpui::Div {
    let t = cx.theme();
    div()
        .flex_none()
        .whitespace_nowrap()
        .px_1p5()
        .rounded_md()
        .border_1()
        .border_color(t.border)
        .text_xs()
        .text_color(t.muted_foreground)
        .child(label.to_string())
}

/// How a row or card is marked: open in the detail pane, under the
/// keyboard cursor, in the multi-selection (`Some` while one exists).
#[derive(Clone, Copy, Default)]
pub(super) struct Marks {
    pub open: bool,
    pub cursor: bool,
    pub checked: Option<bool>,
}

impl Marks {
    pub fn of(id: &str, ws: &crate::state::workspace::Workspace) -> Self {
        Marks {
            open: ws.view.selected.as_deref() == Some(id),
            cursor: ws.cursor.as_deref() == Some(id),
            checked: (!ws.selection.is_empty()).then(|| ws.selection.contains(id)),
        }
    }
}

/// One issue row.
/// `compact` (detail pane open): drop labels and project to keep the
/// title readable.
pub(super) fn issue_row(
    s: &IssueSummary,
    marks: Marks,
    compact: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let t = cx.theme();
    let now = chrono::Utc::now();
    h_flex()
        .id(gpui::SharedString::from(format!("row-{}", s.id)))
        .w_full()
        .h(px(ROW_HEIGHT))
        .pl_1()
        .pr_3()
        .gap_3()
        .cursor_pointer()
        .border_b_1()
        .border_color(t.border.opacity(0.4))
        // The keyboard cursor: a bar at the row's start.
        .child(
            div()
                .w(px(3.))
                .h(px(18.))
                .flex_none()
                .rounded_sm()
                .when(marks.cursor, |el| el.bg(t.primary)),
        )
        .when(marks.checked == Some(true), |el| {
            el.bg(t.primary.opacity(0.08))
        })
        .when(marks.open, |el| el.bg(t.list_active))
        .hover(|el| el.bg(t.list_hover))
        .children(marks.checked.map(|c| {
            div()
                .w(px(12.))
                .flex_none()
                .text_color(t.primary)
                .child(if c { "✓" } else { "·" })
        }))
        .child(
            div()
                .w(px(28.))
                .flex_none()
                .whitespace_nowrap()
                .text_xs()
                .text_color(theme::priority_color(s.priority, t))
                .child(theme::priority_glyph(s.priority)),
        )
        .child(muted(s.id.clone(), cx).w(px(64.)).flex_none())
        .child(
            div()
                .flex_none()
                .text_color(theme::state_color(s.state, t))
                .child(theme::state_glyph(s.state)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(160.))
                .truncate()
                .child(s.title.clone()),
        )
        // Chips and project keep their natural width; the title gives way
        // first, and the project truncates only once the title is at its
        // minimum.
        .children(
            s.labels
                .iter()
                .take(if compact { 0 } else { 3 })
                .map(|l| chip(l, cx)),
        )
        .children(s.project_slug.as_ref().filter(|_| !compact).map(|p| {
            muted(p.replace('-', " "), cx)
                .flex_shrink(1.)
                .min_w(px(32.))
                .truncate()
        }))
        .child(
            muted(
                s.assignee.as_deref().map(display_name).unwrap_or_default(),
                cx,
            )
            .w(px(96.))
            .flex_none()
            .truncate(),
        )
        .child(
            muted(s.updated.map(|d| ago(d, now)).unwrap_or_default(), cx)
                .w(px(72.))
                .flex_none(),
        )
}

impl WorkspaceView {
    pub(super) fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.ws.read(cx).visible.rows.len();
        if count == 0 {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(muted("No matching issues", cx))
                .into_any_element();
        }
        uniform_list(
            "issue-list",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                let ws = this.ws.read(cx);
                let Some(repo) = &ws.repo else {
                    return Vec::new();
                };
                let ix = &repo.read(cx).index;
                let compact = ws.doc.is_some();
                let rows: Vec<ListRow> = ws.visible.rows[range].to_vec();
                rows.into_iter()
                    .map(|row| -> AnyElement {
                        match row {
                            ListRow::Header { key, count } => h_flex()
                                .w_full()
                                .h(px(ROW_HEIGHT))
                                .px_3()
                                .gap_2()
                                .bg(cx.theme().secondary)
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(group_label(key)),
                                )
                                .child(muted(count.to_string(), cx))
                                .into_any_element(),
                            ListRow::Issue(p) => match ix.get(&p) {
                                Some(s) => {
                                    let id = s.id.clone();
                                    issue_row(s, Marks::of(&id, ws), compact, cx)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.select(id.clone(), window, cx);
                                            },
                                        ))
                                        .into_any_element()
                                }
                                None => div().h(px(ROW_HEIGHT)).into_any_element(),
                            },
                        }
                    })
                    .collect()
            }),
        )
        .flex_1()
        .w_full()
        .track_scroll(&self.list_scroll)
        .into_any_element()
    }
}
