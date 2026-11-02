//! The board: one column per state (or priority band), each a virtualized
//! list of fixed-height cards (GUI-SPEC §5.1). Cards sort by the active
//! sort key; there is no manual order. Dropping a card on another column
//! changes its state (or priority, when grouped by priority) through
//! core `ops`; dropping it on its own column does nothing.

use super::format::{ago, display_name};
use super::list::Marks;
use super::{WorkspaceView, muted};
use crate::state::edit;
use crate::state::view_state::group_label;
use crate::theme;
use gpui::{
    AnyElement, App, ClickEvent, Context, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use lastissue::model::{Priority, State};
use lastissue::query::GroupKey;
use lastissue::source::IssueSummary;

const CARD_HEIGHT: f32 = 76.;
const COLUMN_WIDTH: f32 = 280.;

/// What a card carries while dragged; also its drag preview.
#[derive(Clone)]
pub(super) struct DraggedCard {
    id: String,
    title: String,
    state: State,
    priority: Option<Priority>,
}

impl Render for DraggedCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        h_flex()
            .w(px(COLUMN_WIDTH - 16.))
            .gap_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(t.ring)
            .bg(t.background.opacity(0.95))
            .shadow_lg()
            .text_sm()
            .text_color(t.foreground)
            .child(
                div()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(self.id.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child(self.title.clone()),
            )
    }
}

fn card(s: &IssueSummary, marks: Marks, cx: &App) -> gpui::Stateful<gpui::Div> {
    let t = cx.theme();
    let now = chrono::Utc::now();
    // The card sits in a fixed-height slot; the gap is padding inside it.
    div()
        .id(gpui::SharedString::from(format!("card-{}", s.id)))
        .w_full()
        .h(px(CARD_HEIGHT))
        .px_2()
        .pb_2()
        .child(
            v_flex()
                .size_full()
                .p_2()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(if marks.cursor {
                    t.primary
                } else if marks.open {
                    t.ring
                } else {
                    t.border
                })
                .when(marks.cursor, |el| el.border_2())
                .bg(if marks.checked == Some(true) {
                    t.primary.opacity(0.08)
                } else {
                    t.background
                })
                .cursor_pointer()
                .hover(|el| el.bg(t.list_hover))
                .child(
                    h_flex()
                        .gap_2()
                        .children(marks.checked.map(|c| {
                            div().text_xs().text_color(t.primary).child(if c {
                                "✓"
                            } else {
                                "·"
                            })
                        }))
                        .child(muted(s.id.clone(), cx))
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::priority_color(s.priority, t))
                                .child(theme::priority_glyph(s.priority)),
                        ),
                )
                .child(div().truncate().child(s.title.clone()))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            muted(s.labels.join(" · "), cx)
                                .flex_1()
                                .min_w(px(0.))
                                .truncate(),
                        )
                        .child(muted(
                            s.assignee.as_deref().map(display_name).unwrap_or_default(),
                            cx,
                        ))
                        .child(muted(
                            s.updated.map(|d| ago(d, now)).unwrap_or_default(),
                            cx,
                        )),
                ),
        )
}

impl WorkspaceView {
    pub(super) fn render_board(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let groups: Vec<(GroupKey, usize)> = self
            .ws
            .read(cx)
            .visible
            .groups
            .iter()
            .map(|g| (g.key, g.items.len()))
            .collect();
        let mut board = h_flex()
            .id("board")
            .flex_1()
            .min_h(px(0.))
            .items_start()
            .gap_2()
            .p_2()
            .overflow_x_scroll();
        self.board_scroll
            .resize_with(groups.len(), gpui::UniformListScrollHandle::new);
        for (col, (key, count)) in groups.into_iter().enumerate() {
            let header = h_flex()
                .px_2()
                .py_1()
                .gap_2()
                .children(match key {
                    GroupKey::State(s) => Some(
                        div()
                            .text_color(theme::state_color(s, &theme))
                            .child(theme::state_glyph(s)),
                    ),
                    GroupKey::Priority(_) => None,
                })
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(group_label(key)),
                )
                .child(muted(count.to_string(), cx));
            let cards = uniform_list(
                ("board-col", col),
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let ws = this.ws.read(cx);
                    let (Some(repo), Some(group)) = (&ws.repo, ws.visible.groups.get(col)) else {
                        return Vec::new();
                    };
                    let ix = &repo.read(cx).index;
                    let paths = group.items[range.start..range.end.min(group.items.len())].to_vec();
                    paths
                        .into_iter()
                        .map(|p| -> AnyElement {
                            match ix.get(&p) {
                                Some(s) => {
                                    let id = s.id.clone();
                                    let dragged = DraggedCard {
                                        id: id.clone(),
                                        title: s.title.clone(),
                                        state: s.state,
                                        priority: s.priority,
                                    };
                                    card(s, Marks::of(&id, ws), cx)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.select(id.clone(), window, cx);
                                            },
                                        ))
                                        .on_drag(dragged, |d, _, _, cx| cx.new(|_| d.clone()))
                                        .into_any_element()
                                }
                                None => div().h(px(CARD_HEIGHT)).into_any_element(),
                            }
                        })
                        .collect()
                }),
            )
            .flex_1()
            .w_full()
            .track_scroll(&self.board_scroll[col]);
            let ring = theme.ring;
            board = board.child(
                v_flex()
                    .id(("column", col))
                    .w(px(COLUMN_WIDTH))
                    .flex_none()
                    .h_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .bg(theme.secondary)
                    .drag_over::<DraggedCard>(move |st, d, _, _| {
                        if edit::drop_edit(key, d.state, d.priority).is_some() {
                            st.border_color(ring)
                        } else {
                            st
                        }
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedCard, _, cx| {
                        if let Some(e) = edit::drop_edit(key, d.state, d.priority) {
                            let id = d.id.clone();
                            this.ws.update(cx, |w, cx| w.apply_edit(id, e, cx));
                        }
                    }))
                    .child(header)
                    .child(cards),
            );
        }
        board
    }
}
