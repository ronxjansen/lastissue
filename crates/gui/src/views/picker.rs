//! The property picker (GUI-SPEC §5, §7): a fuzzy list over the options
//! from `state::picker`. Opened by a click on a property row (shown under
//! it) or by its key (`s`, `p`, `a`, `l`, `d`, `m`) for the open issue,
//! the cursor's, or the whole selection (a bulk edit; shown floating).
//! Choosing commits at once through `Workspace::apply_to`.

use super::WorkspaceView;
use super::fuzzy::{self, Row};
use crate::state::picker::{self, PickOption, PickerKind};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, IntoElement, ScrollHandle, Subscription, Window,
    anchored, deferred, prelude::*, px,
};
use gpui_component::input::{InputEvent, InputState};
use lastissue::source::IssueSummary;
use std::rc::Rc;

pub(super) struct PickerUi {
    pub kind: PickerKind,
    /// Short IDs the choice applies to.
    pub issues: Vec<String>,
    input: Entity<InputState>,
    highlight: usize,
    scroll: ScrollHandle,
    return_focus: FocusHandle,
    _sub: Subscription,
}

impl WorkspaceView {
    pub(super) fn open_picker(
        &mut self,
        kind: PickerKind,
        issues: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if issues.is_empty() {
            return;
        }
        if self
            .picker
            .as_ref()
            .is_some_and(|p| p.kind == kind && p.issues == issues)
        {
            self.close_picker(window, cx);
            return;
        }
        self.close_overlays(cx);
        let return_focus = self.return_focus(window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(kind.placeholder()));
        let sub = cx.subscribe_in(
            &input,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    if let Some(p) = &mut this.picker {
                        p.highlight = 0;
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.pick(None, window, cx),
                _ => {}
            },
        );
        input.update(cx, |i, cx| i.focus(window, cx));
        self.picker = Some(PickerUi {
            kind,
            issues,
            input,
            highlight: 0,
            scroll: ScrollHandle::new(),
            return_focus,
            _sub: sub,
        });
        cx.notify();
    }

    pub(super) fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.picker.take() {
            window.focus(&p.return_focus, cx);
        }
        cx.notify();
    }

    /// The picker's visible options right now.
    fn picker_options(&self, cx: &Context<Self>) -> Vec<PickOption> {
        let Some(p) = &self.picker else {
            return Vec::new();
        };
        let ws = self.ws.read(cx);
        let Some(repo) = &ws.repo else {
            return Vec::new();
        };
        let ix = &repo.read(cx).index;
        let targets: Vec<&IssueSummary> = p.issues.iter().filter_map(|i| ix.by_id(i)).collect();
        if targets.is_empty() {
            return Vec::new();
        }
        let me = ws.identity.actor().ok().map(|a| a.from.clone());
        let q = p.input.read(cx).value().to_string();
        picker::filtered(p.kind, &q, &targets, ix, me.as_deref())
    }

    /// Commit option `i` (or the highlighted one).
    fn pick(&mut self, i: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let opts = self.picker_options(cx);
        let Some(p) = &self.picker else { return };
        let Some(o) = opts.get(i.unwrap_or(p.highlight)) else {
            return;
        };
        let (issues, kind, input) = (p.issues.clone(), p.kind, p.input.clone());
        let edit = o.edit.clone();
        self.ws.update(cx, |w, cx| w.apply_to(&issues, &edit, cx));
        if kind.multi() {
            input.update(cx, |i, cx| i.set_value("", window, cx));
            if let Some(p) = &mut self.picker {
                p.highlight = 0;
            }
            cx.notify();
        } else {
            self.close_picker(window, cx);
        }
    }

    pub(super) fn step_picker(&mut self, step: isize, cx: &mut Context<Self>) {
        let n = self.picker_options(cx).len();
        if let Some(p) = &mut self.picker {
            p.highlight = picker::step(p.highlight, n, step);
            p.scroll.scroll_to_item(p.highlight);
            cx.notify();
        }
    }

    /// The picker shows under its property row: it is for exactly the
    /// issue open in the detail pane.
    pub(super) fn picker_inline(&self, cx: &gpui::App) -> bool {
        let open = self.ws.read(cx).view.selected.as_deref();
        self.picker
            .as_ref()
            .is_some_and(|p| open.is_some() && p.issues.len() == 1 && p.issues[0] == open.unwrap())
    }

    fn picker_panel(&mut self, cx: &mut Context<Self>) -> Option<gpui::Stateful<gpui::Div>> {
        let opts = self.picker_options(cx);
        let p = self.picker.as_ref()?;
        let header = (p.issues.len() > 1).then(|| format!("{} issues", p.issues.len()).into());
        let rows = opts
            .into_iter()
            .map(|o| Row {
                label: o.label.into(),
                detail: o.detail.map(Into::into),
                checked: Some(o.checked),
                ..Row::default()
            })
            .collect();
        Some(fuzzy::panel(
            fuzzy::Panel {
                id: "picker",
                input: &p.input,
                rows,
                highlight: p.highlight,
                scroll: &p.scroll,
                width: 300.,
                empty: "No matches",
                header,
                footer: None,
                on_pick: Rc::new(|this, i, window, cx| this.pick(Some(i), window, cx)),
            },
            cx,
        ))
    }

    /// The popover under property row `kind`, when the picker is inline.
    pub(super) fn render_picker(
        &mut self,
        kind: PickerKind,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.picker_inline(cx) || self.picker.as_ref()?.kind != kind {
            return None;
        }
        let panel = self
            .picker_panel(cx)?
            .mt_1()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_picker(window, cx)));
        Some(
            deferred(anchored().snap_to_window_with_margin(px(8.)).child(panel))
                .with_priority(1)
                .into_any_element(),
        )
    }

    /// The picker for keyboard targets not shown in the detail pane.
    pub(super) fn render_floating_picker(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.picker.is_none() || self.picker_inline(cx) {
            return None;
        }
        let panel = self.picker_panel(cx)?;
        Some(fuzzy::backdrop("picker-backdrop", panel, cx))
    }
}
