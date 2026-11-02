//! The activity timeline (GUI-SPEC §5): comments and events merged in file
//! order; events are one compact line each, and a run of three or more
//! collapses into a single expandable line. Comments can be edited in place
//! (an editor swaps in for that comment only) and deleted (with a confirm);
//! the composer at the end adds one (`⌘Enter`).

use super::format::{ago, display_name};
use super::{WorkspaceView, muted};
use crate::state::edit::Op;
use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, Window, div,
    prelude::*, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{InputEvent, Textarea, TextareaState};
use gpui_component::text::TextView;
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use lastissue::model::{self, Activity};
use lastissue::store::mbox::Msg;

/// Runs of this many events or more start collapsed.
const COLLAPSE_RUN: usize = 3;

fn who(m: &Msg) -> String {
    let from = display_name(m.get("From").unwrap_or("?"));
    match m.get("Sender") {
        Some(s) => format!("{from} (via {})", display_name(s)),
        None => from,
    }
}

/// A small text action (`Edit`, `Delete`).
fn link(
    id: SharedString,
    label: &'static str,
    cx: &Context<WorkspaceView>,
) -> gpui::Stateful<gpui::Div> {
    let t = cx.theme();
    div()
        .id(id)
        .text_xs()
        .text_color(t.muted_foreground)
        .cursor_pointer()
        .hover(|el| el.text_color(t.foreground))
        .child(label)
}

impl WorkspaceView {
    /// The open issue's short ID.
    fn doc_id(&self, cx: &Context<Self>) -> Option<String> {
        Some(self.ws.read(cx).doc.as_ref()?.read(cx).id.clone())
    }

    pub(super) fn submit_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(e), Some(id)) = (&self.editors, self.doc_id(cx)) else {
            return;
        };
        let composer = e.composer.clone();
        let body = composer.read(cx).value().to_string();
        if body.trim().is_empty() {
            return;
        }
        let task = self.ws.update(cx, |w, cx| {
            w.run(
                Op::AddComment {
                    id,
                    body,
                    reply_to: None,
                },
                cx,
            )
        });
        cx.spawn_in(window, async move |_, cx| {
            if task.await.is_some() {
                let _ = composer.update_in(cx, |c, window, cx| c.set_value("", window, cx));
            }
        })
        .detach();
    }

    fn start_comment_edit(
        &mut self,
        mid: String,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(e) = &mut self.editors else { return };
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 20)
                .default_value(body)
        });
        input.update(cx, |i, cx| i.focus(window, cx));
        e.comment_edit = Some((mid, input.clone()));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = ev
            {
                this.save_comment_edit(window, cx);
            }
        });
        e.subs.push(sub);
        cx.notify();
    }

    fn save_comment_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.doc_id(cx) else { return };
        let Some(e) = &mut self.editors else { return };
        let Some((mid, input)) = e.comment_edit.take() else {
            return;
        };
        let body = input.read(cx).value().to_string();
        if body.trim().is_empty() {
            // Empty means delete; ask instead of guessing.
            e.comment_edit = Some((mid.clone(), input));
            e.confirm_delete = Some(mid);
            cx.notify();
            return;
        }
        self.ws
            .update(cx, |w, cx| {
                w.run(
                    Op::EditComment {
                        id,
                        message_id: mid,
                        body,
                    },
                    cx,
                )
            })
            .detach();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn delete_comment(&mut self, mid: String, cx: &mut Context<Self>) {
        let Some(id) = self.doc_id(cx) else { return };
        if let Some(e) = &mut self.editors {
            e.confirm_delete = None;
            e.comment_edit = None;
        }
        self.ws
            .update(cx, |w, cx| {
                w.run(
                    Op::DeleteComment {
                        id,
                        message_id: mid,
                    },
                    cx,
                )
            })
            .detach();
    }

    fn render_composer(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let composer = self.editors.as_ref()?.composer.clone();
        Some(
            v_flex()
                .gap_1()
                .pt_1()
                .child(Textarea::new(&composer))
                .child(
                    h_flex().child(div().flex_1()).child(
                        Button::new("comment-send")
                            .small()
                            .primary()
                            .label("Comment")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.submit_comment(window, cx);
                            })),
                    ),
                )
                .into_any_element(),
        )
    }

    /// One comment: header with actions, then its text or its editor.
    fn comment_card(
        &mut self,
        number: usize,
        reply_to: Option<usize>,
        msg: &Msg,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let now = chrono::Utc::now();
        let mid = msg.message_id().to_string();
        let editing = self
            .editors
            .as_ref()
            .and_then(|e| e.comment_edit.as_ref())
            .filter(|(m, _)| *m == mid)
            .map(|(_, i)| i.clone());
        let confirming = self
            .editors
            .as_ref()
            .and_then(|e| e.confirm_delete.as_deref())
            == Some(mid.as_str());
        let mut header = h_flex()
            .gap_2()
            .child(div().font_weight(gpui::FontWeight::MEDIUM).child(who(msg)))
            .child(muted(format!("/{number}"), cx));
        if let Some(r) = reply_to {
            header = header.child(muted(format!("↳ /{r}"), cx));
        }
        if msg.has("X-Edited") {
            header = header.child(muted("edited", cx));
        }
        header = header.child(div().flex_1());
        let key = |s: &str| SharedString::from(format!("{s}-{mid}"));
        if confirming {
            let (m1, m2) = (mid.clone(), mid.clone());
            header = header
                .child(muted("Delete this comment?", cx).text_color(theme.danger))
                .child(
                    link(key("c-del-yes"), "Delete", cx)
                        .text_color(theme.danger)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.delete_comment(m1.clone(), cx)
                        })),
                )
                .child(link(key("c-del-no"), "Cancel", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        if let Some(e) = &mut this.editors
                            && e.confirm_delete.as_deref() == Some(m2.as_str())
                        {
                            e.confirm_delete = None;
                        }
                        cx.notify();
                    },
                )));
        } else if editing.is_none() {
            let (m1, m2, body) = (mid.clone(), mid.clone(), msg.body.clone());
            header = header
                .child(link(key("c-edit"), "Edit", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.start_comment_edit(m1.clone(), body.clone(), window, cx);
                    },
                )))
                .child(link(key("c-del"), "Delete", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        if let Some(e) = &mut this.editors {
                            e.confirm_delete = Some(m2.clone());
                        }
                        cx.notify();
                    },
                )));
        }
        header = header.child(muted(
            msg.date().map(|d| ago(d, now)).unwrap_or_default(),
            cx,
        ));
        let content = match editing {
            Some(input) => v_flex()
                .gap_1()
                .child(Textarea::new(&input))
                .child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1())
                        .child(
                            Button::new(key("c-cancel"))
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    if let Some(e) = &mut this.editors {
                                        e.comment_edit = None;
                                    }
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(key("c-save"))
                                .xsmall()
                                .primary()
                                .label("Save")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.save_comment_edit(window, cx);
                                })),
                        ),
                )
                .into_any_element(),
            None => TextView::markdown(
                SharedString::from(format!("c-{mid}")),
                msg.body.trim().to_string(),
            )
            .selectable(true)
            .into_any_element(),
        };
        v_flex()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .child(header)
            .child(content)
            .into_any_element()
    }

    fn event_line(&self, kind: &str, m: &Msg, cx: &Context<Self>) -> AnyElement {
        let now = chrono::Utc::now();
        let lines: Vec<&str> = m.body.lines().filter(|l| !l.trim().is_empty()).collect();
        let what = if lines.is_empty() {
            kind.to_string()
        } else {
            lines.join(" · ")
        };
        h_flex()
            .gap_2()
            .items_start()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(div().flex_none().child("•"))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(format!("{} {what}", who(m))),
            )
            .child(
                div()
                    .flex_none()
                    .child(m.date().map(|d| ago(d, now)).unwrap_or_default()),
            )
            .into_any_element()
    }

    pub(super) fn render_activity(&mut self, msgs: &[Msg], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let items = model::activity(msgs);
        let mut col = v_flex().gap_2().child(
            div()
                .pt_2()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Activity"),
        );
        if items.is_empty() {
            return col
                .child(muted("No activity yet", cx))
                .children(self.render_composer(cx))
                .into_any_element();
        }
        let mut i = 0;
        while i < items.len() {
            match items[i] {
                Activity::Event { index, .. } => {
                    let run_end = items[i..]
                        .iter()
                        .position(|a| matches!(a, Activity::Comment { .. }))
                        .map_or(items.len(), |n| i + n);
                    let run = &items[i..run_end];
                    if run.len() >= COLLAPSE_RUN && !self.expanded_runs.contains(&index) {
                        let people: Vec<String> = {
                            let mut v: Vec<String> = run.iter().map(|a| who(a.msg())).collect();
                            v.dedup();
                            v
                        };
                        col = col.child(
                            div()
                                .id(("run", index))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .cursor_pointer()
                                .hover(|el| el.text_color(theme.foreground))
                                .child(format!("▸ {} changes by {}", run.len(), people.join(", ")))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.expanded_runs.insert(index);
                                    cx.notify();
                                })),
                        );
                    } else {
                        for a in run {
                            if let Activity::Event { kind, msg, .. } = a {
                                col = col.child(self.event_line(kind, msg, cx));
                            }
                        }
                    }
                    i = run_end;
                }
                Activity::Comment {
                    number,
                    reply_to,
                    msg,
                    ..
                } => {
                    col = col.child(self.comment_card(number, reply_to, msg, cx));
                    i += 1;
                }
            }
        }
        col = col.children(self.render_composer(cx));
        col.into_any_element()
    }
}
