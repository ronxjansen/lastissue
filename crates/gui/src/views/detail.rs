//! The detail pane (GUI-SPEC §5, §5.2): the selected issue's inline title,
//! property pickers, body (rendered; a click or `e` edits it in place) and
//! activity with the comment composer. Banners surface a conflict with a
//! change on disk, a draft left by an earlier session, and save errors.

use super::format::{date_time, display_name};
use super::{WorkspaceView, muted};
use crate::state::issue_doc::IssueDoc;
use crate::state::picker::PickerKind;
use crate::state::view_state::{priority_label, state_label};
use crate::theme;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, IntoElement, ParentElement, SharedString, Styled,
    Window, div, prelude::*, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, Textarea};
use gpui_component::text::TextView;
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use lastissue::index::Index;
use lastissue::source::IssueSummary;

fn prop(label: &'static str, value: impl IntoElement, cx: &App) -> gpui::Div {
    h_flex()
        .gap_2()
        .items_start()
        .child(muted(label, cx).w(px(84.)).flex_none().pt_0p5())
        .child(div().flex_1().min_w(px(0.)).child(value))
}

/// A one-line notice with actions.
fn banner(
    text: impl Into<SharedString>,
    color: gpui::Hsla,
    actions: Vec<Button>,
    cx: &App,
) -> AnyElement {
    h_flex()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded_md()
        .border_1()
        .border_color(color.opacity(0.6))
        .bg(color.opacity(0.08))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(text.into()),
        )
        .children(actions)
        .into_any_element()
}

/// `abc123 Title` for a related issue's Message-ID, or the raw ID when it
/// isn't loaded (deleted, another repo).
fn relation(ix: &Index, mid: &str) -> (Option<String>, String) {
    match ix.by_message_id(mid) {
        Some(s) => (Some(s.id.clone()), format!("{}  {}", s.id, s.title)),
        None => (None, mid.to_string()),
    }
}

impl WorkspaceView {
    fn relation_links(
        &mut self,
        label: &'static str,
        mids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if mids.is_empty() {
            return None;
        }
        let rels: Vec<(Option<String>, String)> = {
            let ws = self.ws.read(cx);
            let ix = &ws.repo.as_ref()?.read(cx).index;
            mids.iter().map(|m| relation(ix, m)).collect()
        };
        let theme = cx.theme().clone();
        let mut col = v_flex();
        for (i, (id, text)) in rels.into_iter().enumerate() {
            let el = div()
                .id(SharedString::from(format!("rel-{label}-{i}")))
                .truncate()
                .child(text);
            col = col.child(match id {
                Some(id) => el
                    .text_color(theme.link)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.select(id.clone(), window, cx);
                    }))
                    .into_any_element(),
                None => el.text_color(theme.muted_foreground).into_any_element(),
            });
        }
        Some(prop(label, col, cx).into_any_element())
    }

    /// A property row that opens `kind`'s picker when clicked.
    fn editable_prop(
        &mut self,
        label: &'static str,
        kind: PickerKind,
        issue: &str,
        value: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let open = self.picker_inline(cx) && self.picker.as_ref().is_some_and(|p| p.kind == kind);
        let id = issue.to_string();
        let row = prop(label, value, cx)
            .id(SharedString::from(format!("prop-{label}")))
            .px_1()
            .mx_neg_1()
            .rounded_md()
            .cursor_pointer()
            .when(open, |el| el.bg(theme.list_active))
            .hover(|el| el.bg(theme.list_hover))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_picker(kind, vec![id.clone()], window, cx);
            }));
        v_flex()
            .child(row)
            .children(self.render_picker(kind, cx))
            .into_any_element()
    }

    fn doc_banners(&mut self, doc: &Entity<IssueDoc>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let d = doc.read(cx);
        let mut out = Vec::new();
        if d.has_conflict() {
            let (a, b) = (doc.clone(), doc.clone());
            out.push(banner(
                "This issue changed on disk while you were editing it.",
                theme.warning,
                vec![
                    Button::new("keep-mine")
                        .xsmall()
                        .primary()
                        .label("Keep mine")
                        .on_click(move |_, _, cx| a.update(cx, |d, cx| d.keep_mine(cx))),
                    Button::new("use-theirs")
                        .xsmall()
                        .label("Use theirs")
                        .on_click(move |_, _, cx| b.update(cx, |d, cx| d.use_theirs(cx))),
                ],
                cx,
            ));
        }
        if d.leftover.is_some() {
            let (a, b) = (doc.clone(), doc.clone());
            out.push(banner(
                "An unsaved description draft from an earlier session was found.",
                theme.info,
                vec![
                    Button::new("restore-draft")
                        .xsmall()
                        .primary()
                        .label("Restore")
                        .on_click(move |_, _, cx| a.update(cx, |d, cx| d.restore_leftover(cx))),
                    Button::new("discard-draft")
                        .xsmall()
                        .label("Discard")
                        .on_click(move |_, _, cx| b.update(cx, |d, cx| d.discard_leftover(cx))),
                ],
                cx,
            ));
        }
        if let Some(e) = &d.save_error {
            out.push(banner(
                format!("Couldn't save (your draft is kept): {e}"),
                theme.danger,
                Vec::new(),
                cx,
            ));
        }
        out
    }

    pub(super) fn render_detail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _ = window;
        let theme = cx.theme().clone();
        let ws = self.ws.read(cx);
        let Some(doc) = ws.doc.clone() else {
            return div().into_any_element();
        };
        let summary: Option<IssueSummary> = ws
            .repo
            .as_ref()
            .and_then(|r| r.read(cx).index.by_id(&doc.read(cx).id).cloned());
        let (msgs, error, saving, dirty) = {
            let d = doc.read(cx);
            (d.msgs.clone(), d.error.clone(), d.saving, d.is_dirty())
        };
        if self.expanded_for.as_deref() != Some(doc.read(cx).id.as_str()) {
            self.expanded_for = Some(doc.read(cx).id.clone());
            self.expanded_runs.clear();
        }

        let close = div()
            .id("detail-close")
            .px_1()
            .cursor_pointer()
            .text_color(theme.muted_foreground)
            .hover(|el| el.text_color(theme.foreground))
            .child("✕")
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.ws.update(cx, |w, cx| w.select(None, cx));
                window.focus(&this.focus, cx);
            }));

        let mut pane = v_flex()
            .id("detail")
            .w(px(440.))
            .flex_none()
            .h_full()
            .border_l_1()
            .border_color(theme.border)
            .overflow_y_scroll()
            .p_4()
            .gap_3();

        let Some(s) = summary else {
            return pane
                .child(h_flex().child(div().flex_1()).child(close))
                .child(muted("This issue is no longer on disk.", cx))
                .into_any_element();
        };

        let crumbs = match &s.project_slug {
            Some(p) => format!("{} › {}", p.replace('-', " "), s.id),
            None => s.id.clone(),
        };
        let status = if saving {
            "Saving…"
        } else if dirty {
            "Unsaved"
        } else {
            ""
        };
        pane = pane.child(
            h_flex()
                .gap_2()
                .child(muted(crumbs, cx).flex_1())
                .child(muted(status, cx))
                .child(close),
        );
        pane = pane.children(self.doc_banners(&doc, cx));

        let editors = self
            .editors
            .as_ref()
            .filter(|e| e.doc == doc)
            .map(|e| (e.title.clone(), e.body.clone(), e.editing_body));
        pane = pane.child(match &editors {
            Some((title, _, _)) => div()
                .text_xl()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .mx_neg_2()
                .child(Input::new(title).large().appearance(false))
                .into_any_element(),
            None => div()
                .text_xl()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(s.title.clone())
                .into_any_element(),
        });

        // Properties.
        let state = h_flex()
            .gap_1()
            .child(
                div()
                    .text_color(theme::state_color(s.state, &theme))
                    .child(theme::state_glyph(s.state)),
            )
            .child(state_label(s.state))
            .into_any_element();
        let none = |cx: &App| muted("—", cx).into_any_element();
        let priority = h_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::priority_color(s.priority, &theme))
                    .child(theme::priority_glyph(s.priority)),
            )
            .child(priority_label(s.priority))
            .into_any_element();
        let assignee = s.assignee.as_deref().map_or_else(
            || none(cx),
            |a| div().child(display_name(a)).into_any_element(),
        );
        let labels = if s.labels.is_empty() {
            none(cx)
        } else {
            h_flex()
                .flex_wrap()
                .gap_1()
                .children(s.labels.iter().map(|l| super::list::chip(l, cx)))
                .into_any_element()
        };
        let due = s.due.map_or_else(
            || none(cx),
            |d| div().child(date_time(d)).into_any_element(),
        );
        let project = s.project_slug.as_ref().map_or_else(
            || none(cx),
            |p| div().child(p.replace('-', " ")).into_any_element(),
        );
        let id = s.id.clone();
        let mut props = v_flex()
            .gap_0p5()
            .child(self.editable_prop("State", PickerKind::State, &id, state, cx))
            .child(self.editable_prop("Priority", PickerKind::Priority, &id, priority, cx))
            .child(self.editable_prop("Assignee", PickerKind::Assignee, &id, assignee, cx))
            .child(self.editable_prop("Labels", PickerKind::Labels, &id, labels, cx))
            .child(self.editable_prop("Due", PickerKind::Due, &id, due, cx))
            .child(self.editable_prop("Project", PickerKind::Project, &id, project, cx));
        if let Some(e) = s.expires {
            props = props.child(prop("Expires", date_time(e), cx));
        }
        if !s.watchers.is_empty() {
            let names: Vec<String> = s.watchers.iter().map(|w| display_name(w)).collect();
            props = props.child(prop("Watchers", names.join(", "), cx));
        }
        let parent = s.parent.clone().into_iter().collect();
        props = props
            .children(self.relation_links("Parent", parent, cx))
            .children(self.relation_links("Blocked by", s.blocked_by.clone(), cx))
            .children(self.relation_links("Related", s.related.clone(), cx));
        let author = match &s.sender {
            Some(agent) => format!("{} (via {})", display_name(&s.author), display_name(agent)),
            None => display_name(&s.author),
        };
        props = props.child(prop(
            "Created",
            format!(
                "{} · {}",
                s.created.map(date_time).unwrap_or_default(),
                author
            ),
            cx,
        ));
        pane = pane.child(props);

        // Body and activity.
        match (msgs, error) {
            (_, Some(e)) => {
                pane = pane.child(div().text_color(theme.danger).child(e));
            }
            (None, None) => pane = pane.child(muted("Loading…", cx)),
            (Some(msgs), None) => {
                pane = pane.child(div().h(px(1.)).bg(theme.border));
                pane = pane.child(match &editors {
                    Some((_, body, true)) => Textarea::new(body).into_any_element(),
                    _ => {
                        let text = doc
                            .read(cx)
                            .body
                            .as_ref()
                            .map_or_else(|| msgs[0].body.clone(), |b| b.value().to_string());
                        let text = text.trim().to_string();
                        div()
                            .id("body-view")
                            .min_h(px(48.))
                            .p_1()
                            .mx_neg_1()
                            .rounded_md()
                            .cursor_text()
                            .hover(|el| el.bg(theme.list_hover.opacity(0.5)))
                            .child(if text.is_empty() {
                                muted("Add a description…", cx).into_any_element()
                            } else {
                                TextView::markdown(
                                    SharedString::from(format!("body-{}", msgs[0].message_id())),
                                    text,
                                )
                                .into_any_element()
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.edit_body(window, cx);
                            }))
                            .into_any_element()
                    }
                });
                pane = pane.child(self.render_activity(&msgs, cx));
            }
        }
        pane.into_any_element()
    }
}
