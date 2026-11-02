//! The create-issue modal (GUI-SPEC §5, §5.2): `c` opens it; the file is
//! written only on submit (`⌘Enter` or the button), so there are no
//! half-made files. Defaults come from the current view: the sidebar's
//! project, "assigned to me" under My issues, the board column state.

use super::{WorkspaceView, muted};
use crate::actions::{CREATE_MODAL, ModalCancel, ModalSubmit};
use crate::state::edit::Op;
use crate::state::view_state::{Scope, priority_label, state_label};
use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div,
    prelude::*, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use lastissue::model::{Priority, State};
use lastissue::ops::NewIssue;

const STATES: [State; 4] = [
    State::Backlog,
    State::Todo,
    State::InProgress,
    State::InReview,
];
const PRIORITIES: [Option<Priority>; 5] = [
    None,
    Some(Priority::Urgent),
    Some(Priority::High),
    Some(Priority::Medium),
    Some(Priority::Low),
];

pub(super) struct CreateUi {
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    state: State,
    priority: Option<Priority>,
    /// Project short ID and its display name.
    project: Option<(String, String)>,
    assign_me: bool,
    submitting: bool,
    _subs: Vec<gpui::Subscription>,
}

impl CreateUi {
    /// The issue to create from the modal's fields.
    fn new_issue(&self, cx: &gpui::App) -> NewIssue {
        NewIssue {
            title: self.title.read(cx).value().to_string(),
            body: self.body.read(cx).value().to_string(),
            state: Some(self.state),
            priority: self.priority,
            project: self.project.as_ref().map(|(id, _)| id.clone()),
            assignee: self.assign_me.then(|| "me".to_string()),
            ..NewIssue::default()
        }
    }
}

impl WorkspaceView {
    pub(super) fn open_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.create.is_some() || self.ws.read(cx).repo.is_none() {
            return;
        }
        let ws = self.ws.read(cx);
        let project = match &ws.view.scope {
            Scope::Project(p) => ws.repo.as_ref().and_then(|r| {
                r.read(cx)
                    .index
                    .projects()
                    .iter()
                    .find(|x| &x.id == p)
                    .map(|x| (x.id.clone(), x.slug.replace('-', " ")))
            }),
            _ => None,
        };
        let assign_me = ws.view.scope == Scope::Mine;
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Issue title"));
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(5, 20)
                .placeholder("Description (Markdown)…")
        });
        let subs = vec![
            cx.subscribe_in(&title, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.submit_create(window, cx);
                }
            }),
            cx.subscribe_in(&body, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = ev
                {
                    this.submit_create(window, cx);
                }
            }),
        ];
        title.update(cx, |t, cx| t.focus(window, cx));
        self.create = Some(CreateUi {
            title,
            body,
            state: State::Todo,
            priority: None,
            project,
            assign_me,
            submitting: false,
            _subs: subs,
        });
        self.ws.update(cx, |w, _| w.error = None);
        cx.notify();
    }

    fn close_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.create = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn submit_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &mut self.create else { return };
        if c.submitting {
            return;
        }
        let new = c.new_issue(cx);
        if new.title.trim().is_empty() {
            c.title.update(cx, |t, cx| t.focus(window, cx));
            return;
        }
        c.submitting = true;
        let task = self
            .ws
            .update(cx, |w, cx| w.run(Op::Create(Box::new(new)), cx));
        cx.spawn_in(window, async move |this, cx| {
            let a = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match a {
                    Some(a) => {
                        this.close_create(window, cx);
                        this.select(a.id, window, cx);
                    }
                    None => {
                        if let Some(c) = &mut this.create {
                            c.submitting = false;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_create(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let error = self.ws.read(cx).error.clone();
        let Some(c) = &self.create else {
            return div().into_any_element();
        };
        let seg = |id: SharedString, label: &'static str, on: bool| {
            div()
                .id(id)
                .px_2()
                .py_0p5()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .when(on, |el| el.bg(theme.secondary_active))
                .hover(|el| el.bg(theme.secondary_hover))
                .child(label)
        };
        let mut states = h_flex().gap_0p5().child(muted("State", cx).w(px(60.)));
        for st in STATES {
            states = states.child(
                seg(
                    format!("cs-{}", st.as_str()).into(),
                    state_label(st),
                    c.state == st,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(c) = &mut this.create {
                        c.state = st;
                    }
                    cx.notify();
                })),
            );
        }
        let mut prios = h_flex().gap_0p5().child(muted("Priority", cx).w(px(60.)));
        for (i, p) in PRIORITIES.into_iter().enumerate() {
            prios = prios.child(
                seg(format!("cp-{i}").into(), priority_label(p), c.priority == p).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(c) = &mut this.create {
                            c.priority = p;
                        }
                        cx.notify();
                    }),
                ),
            );
        }
        let mut extras = h_flex().gap_2();
        if let Some((_, name)) = &c.project {
            extras = extras.child(
                seg("c-project".into(), "", true)
                    .child(format!("Project: {name}  ✕"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(c) = &mut this.create {
                            c.project = None;
                        }
                        cx.notify();
                    })),
            );
        }
        extras = extras.child(
            seg(
                "c-me".into(),
                if c.assign_me {
                    "Assigned to me ✓"
                } else {
                    "Assign to me"
                },
                c.assign_me,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(c) = &mut this.create {
                    c.assign_me = !c.assign_me;
                }
                cx.notify();
            })),
        );
        let panel = v_flex()
            .id("create-modal")
            .key_context(CREATE_MODAL)
            .on_action(cx.listener(|this, _: &ModalSubmit, window, cx| {
                this.submit_create(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ModalCancel, window, cx| {
                this.close_create(window, cx);
            }))
            .occlude()
            .w(px(560.))
            .p_4()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("New issue"),
            )
            .child(Input::new(&c.title).large())
            .child(Textarea::new(&c.body))
            .child(states)
            .child(prios)
            .child(extras)
            .children(error.map(|e| div().text_xs().text_color(theme.danger).child(e)))
            .child(
                h_flex()
                    .gap_2()
                    .child(muted("⌘Enter to create · Esc to cancel", cx).flex_1())
                    .child(
                        Button::new("create-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.close_create(window, cx);
                            })),
                    )
                    .child(
                        Button::new("create-submit")
                            .small()
                            .primary()
                            .loading(c.submitting)
                            .label("Create issue")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.submit_create(window, cx);
                            })),
                    ),
            );
        div()
            .id("create-overlay")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(96.))
            .bg(gpui::black().opacity(0.35))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.close_create(window, cx);
            }))
            .child(panel)
            .into_any_element()
    }
}
