//! The `⌘K` palette and the `/` search box (GUI-SPEC §6, §7), both fuzzy
//! lists over the shared panel, plus the delete confirmation.
//!
//! The palette lists every command with its current binding (read from
//! the live keymap, so user overrides show) and runs the chosen one by
//! dispatching its action to whatever had focus before the palette
//! opened. Search shows tiered results as they stream in: `Enter` opens
//! one, `⌘Enter` makes the query the view's filter.

use super::WorkspaceView;
use super::fuzzy::{self, Row};
use crate::actions::{self, CONFIRM, ModalCancel, ModalSubmit};
use crate::state::palette::{self, Entry, Item, PaletteMode, Sources};
use crate::theme;
use gpui::{
    Action, AnyElement, App, ClickEvent, Context, Entity, FocusHandle, IntoElement,
    KeybindingKeystroke, ParentElement, ScrollHandle, SharedString, Styled, Subscription, Window,
    div, prelude::*, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use std::rc::Rc;

/// Results the search box shows at most.
const SHOWN: usize = 200;

pub(super) struct PaletteUi {
    mode: PaletteMode,
    input: Entity<InputState>,
    highlight: usize,
    scroll: ScrollHandle,
    return_focus: FocusHandle,
    _sub: Subscription,
}

pub(super) struct SearchUi {
    input: Entity<InputState>,
    highlight: usize,
    scroll: ScrollHandle,
    return_focus: FocusHandle,
    _sub: Subscription,
}

pub(super) struct ConfirmUi {
    pub ids: Vec<String>,
    focus: FocusHandle,
    return_focus: FocusHandle,
}

/// `⌘K`, `⇧h`, `g i`, `↵`: one keystroke as the spec writes it (letters
/// lowercase unless ⌘/⌃/⌥ is held).
pub fn key_label(k: &KeybindingKeystroke) -> String {
    let m = k.modifiers();
    let key = k.key();
    let shown = match key {
        "enter" => "↵".to_string(),
        "space" => "␣".to_string(),
        _ => k.to_string(),
    };
    let chord = m.platform || m.control || m.alt;
    if !chord && key.len() == 1 {
        shown.to_lowercase()
    } else if key == "enter" {
        let mut s = k.to_string();
        s.truncate(s.len() - "enter".len());
        s + "↵"
    } else {
        shown
    }
}

/// The binding shown for `action`: the keymap's last (highest precedence).
pub fn binding_label(action: &dyn Action, cx: &App) -> Option<String> {
    let km = cx.key_bindings();
    let km = km.borrow();
    let b = km.bindings_for_action(action).last()?;
    Some(
        b.keystrokes()
            .iter()
            .map(key_label)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

impl WorkspaceView {
    /// Where focus goes back to when an overlay closes.
    pub(super) fn return_focus(&self, window: &Window, cx: &App) -> FocusHandle {
        if self.detail_focus.contains_focused(window, cx) {
            self.detail_focus.clone()
        } else {
            self.focus.clone()
        }
    }

    /// Close the palette and search (one overlay at a time).
    pub(super) fn close_overlays(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        self.palette = None;
        if self.search.take().is_some() {
            self.ws.update(cx, |w, _| w.close_search());
        }
    }

    pub(super) fn overlay_step(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            self.step_picker(step, cx);
        } else if self.palette.is_some() {
            let n = self.palette_entries(cx).len();
            if let Some(p) = &mut self.palette {
                p.highlight = crate::state::picker::step(p.highlight, n, step);
                p.scroll.scroll_to_item(p.highlight);
            }
        } else if self.search.is_some() {
            let n = self.ws.read(cx).search.hits.len().min(SHOWN);
            if let Some(s) = &mut self.search {
                s.highlight = crate::state::picker::step(s.highlight, n, step);
                s.scroll.scroll_to_item(s.highlight);
            }
        }
        cx.notify();
    }

    pub(super) fn overlay_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            self.close_picker(window, cx);
            return;
        }
        let back = self
            .palette
            .as_ref()
            .map(|p| p.return_focus.clone())
            .or_else(|| self.search.as_ref().map(|s| s.return_focus.clone()));
        self.close_overlays(cx);
        if let Some(f) = back {
            window.focus(&f, cx);
        }
        cx.notify();
    }

    // ---------------------------------------------------------- palette

    pub(super) fn open_palette(
        &mut self,
        mode: PaletteMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.as_ref().is_some_and(|p| p.mode == mode) {
            self.overlay_cancel(window, cx);
            return;
        }
        let return_focus = self
            .palette
            .as_ref()
            .map(|p| p.return_focus.clone())
            .unwrap_or_else(|| self.return_focus(window, cx));
        self.close_overlays(cx);
        let placeholder = match mode {
            PaletteMode::All => "Type a command…",
            PaletteMode::Scopes => "Go to project…",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let sub = cx.subscribe_in(
            &input,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    if let Some(p) = &mut this.palette {
                        p.highlight = 0;
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.run_palette(None, window, cx),
                _ => {}
            },
        );
        input.update(cx, |i, cx| i.focus(window, cx));
        self.palette = Some(PaletteUi {
            mode,
            input,
            highlight: 0,
            scroll: ScrollHandle::new(),
            return_focus,
            _sub: sub,
        });
        cx.notify();
    }

    fn palette_entries(&self, cx: &App) -> Vec<Entry> {
        let Some(p) = &self.palette else {
            return Vec::new();
        };
        let ws = self.ws.read(cx);
        let titles: Vec<&str> = self.commands.iter().map(|c| c.title).collect();
        let repos: Vec<(String, String)> = ws
            .repos
            .iter()
            .map(|r| (r.label.clone(), r.root.display().to_string()))
            .collect();
        let projects: Vec<(String, String)> = ws.repo.as_ref().map_or_else(Vec::new, |r| {
            let mut v: Vec<(String, String)> = r
                .read(cx)
                .index
                .projects()
                .iter()
                .map(|p| (p.id.clone(), p.slug.replace('-', " ")))
                .collect();
            v.sort_by(|a, b| a.1.cmp(&b.1));
            v
        });
        let src = Sources {
            commands: &titles,
            repos: &repos,
            projects: &projects,
        };
        let q = p.input.read(cx).value().to_string();
        palette::filter(palette::entries(&src, p.mode), &q)
    }

    /// The action an entry runs, if it is one.
    fn entry_action(&self, item: &Item) -> Option<Box<dyn Action>> {
        match item {
            Item::Command(i) => self.commands.get(*i).map(|c| (c.build)()),
            Item::Repo(i) => actions::switch_repo(*i),
            Item::Scope(_) => None,
        }
    }

    fn run_palette(&mut self, i: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.palette_entries(cx);
        let Some(p) = &self.palette else { return };
        let Some(e) = entries.get(i.unwrap_or(p.highlight)) else {
            return;
        };
        let back = p.return_focus.clone();
        let action = self.entry_action(&e.item);
        let item = e.item.clone();
        self.close_overlays(cx);
        window.focus(&back, cx);
        match (action, item) {
            (Some(a), _) => window.dispatch_action(a, cx),
            (None, Item::Repo(i)) => {
                let repo = self.ws.read(cx).repos.get(i).cloned();
                if let Some(r) = repo.filter(|r| r.exists()) {
                    self.ws.update(cx, |w, cx| w.open_repo(r, cx));
                }
            }
            (None, Item::Scope(s)) => {
                self.ws
                    .update(cx, |w, cx| w.update_view(|v| v.scope = s, cx));
                window.focus(&self.focus, cx);
            }
            (None, Item::Command(_)) => {}
        }
        cx.notify();
    }

    pub(super) fn render_palette(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let entries = self.palette_entries(cx);
        let p = self.palette.as_ref()?;
        let rows = entries
            .iter()
            .map(|e| Row {
                label: e.title.clone().into(),
                detail: e.detail.clone().map(Into::into),
                hint: self
                    .entry_action(&e.item)
                    .and_then(|a| binding_label(&*a, cx))
                    .map(Into::into),
                ..Row::default()
            })
            .collect();
        let panel = fuzzy::panel(
            fuzzy::Panel {
                id: "palette",
                input: &p.input,
                rows,
                highlight: p.highlight,
                scroll: &p.scroll,
                width: 560.,
                empty: "No matching commands",
                header: None,
                footer: None,
                on_pick: Rc::new(|this, i, window, cx| this.run_palette(Some(i), window, cx)),
            },
            cx,
        );
        Some(fuzzy::backdrop("palette-backdrop", panel, cx))
    }

    // ----------------------------------------------------------- search

    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws.read(cx).repo.is_none() {
            return;
        }
        if let Some(s) = &self.search {
            let input = s.input.clone();
            input.update(cx, |i, cx| i.focus(window, cx));
            return;
        }
        let return_focus = self.return_focus(window, cx);
        self.close_overlays(cx);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search titles, bodies, comments… (state:todo label:bug @me)")
        });
        let sub = cx.subscribe_in(
            &input,
            window,
            |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let q = input.read(cx).value().to_string();
                    if let Some(s) = &mut this.search {
                        s.highlight = 0;
                    }
                    this.ws.update(cx, |w, cx| w.set_search(&q, cx));
                }
                InputEvent::PressEnter {
                    secondary: true, ..
                } => {
                    this.apply_search(window, cx);
                }
                InputEvent::PressEnter { .. } => this.open_hit(None, window, cx),
                _ => {}
            },
        );
        input.update(cx, |i, cx| i.focus(window, cx));
        self.search = Some(SearchUi {
            input,
            highlight: 0,
            scroll: ScrollHandle::new(),
            return_focus,
            _sub: sub,
        });
        cx.notify();
    }

    /// `Enter`: open the highlighted result in the detail pane.
    fn open_hit(&mut self, i: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.search else { return };
        let i = i.unwrap_or(s.highlight);
        let Some(id) = self.ws.read(cx).search.hits.get(i).map(|h| h.id.clone()) else {
            return;
        };
        self.close_overlays(cx);
        self.ws.update(cx, |w, cx| w.select(Some(id), cx));
        window.focus(&self.detail_focus, cx);
        cx.notify();
    }

    /// `⌘Enter`: the query becomes the current view's filter.
    fn apply_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let q = self.ws.read(cx).search.query.clone();
        self.close_overlays(cx);
        self.filter
            .update(cx, |f, cx| f.set_value(q.clone(), window, cx));
        self.ws
            .update(cx, |w, cx| w.update_view(|v| v.filter = q, cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn render_search(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = self.search.as_ref()?;
        let theme = cx.theme().clone();
        let ws = self.ws.read(cx);
        let st = &ws.search;
        let ix = &ws.repo.as_ref()?.read(cx).index;
        let rows: Vec<Row> = st
            .hits
            .iter()
            .take(SHOWN)
            .filter_map(|h| {
                let sum = ix.get(&h.path)?;
                let leading = h_flex()
                    .gap_2()
                    .flex_none()
                    .child(
                        div()
                            .text_color(theme::state_color(sum.state, &theme))
                            .child(theme::state_glyph(sum.state)),
                    )
                    .child(
                        div()
                            .w(px(56.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(sum.id.clone()),
                    )
                    .into_any_element();
                Some(Row {
                    label: sum.title.clone().into(),
                    detail: Some(h.tier.label().into()),
                    leading: Some(leading),
                    lines: h
                        .snippets
                        .iter()
                        .map(|sn| SharedString::from(format!("{}: {}", sn.loc, sn.text)))
                        .collect(),
                    ..Row::default()
                })
            })
            .collect();
        let mut status = vec![match st.hits.len() {
            0 if st.query.trim().is_empty() => "Type to search".to_string(),
            n if n > SHOWN => format!("{SHOWN} of {n} results"),
            1 => "1 result".to_string(),
            n => format!("{n} results"),
        }];
        if st.scanning {
            status.push("searching bodies and comments…".into());
        }
        let footer = h_flex()
            .gap_2()
            .child(super::muted(status.join(" · "), cx).flex_1())
            .children(
                st.error
                    .clone()
                    .map(|e| div().text_xs().text_color(theme.danger).truncate().child(e)),
            )
            .child(super::muted("↵ open · ⌘↵ filter view · esc close", cx))
            .into_any_element();
        let panel = fuzzy::panel(
            fuzzy::Panel {
                id: "search",
                input: &s.input,
                rows,
                highlight: s.highlight,
                scroll: &s.scroll,
                width: 680.,
                empty: if st.scanning {
                    "Searching…"
                } else {
                    "No results"
                },
                header: None,
                footer: Some(footer),
                on_pick: Rc::new(|this, i, window, cx| this.open_hit(Some(i), window, cx)),
            },
            cx,
        );
        Some(fuzzy::backdrop("search-backdrop", panel, cx))
    }

    // ---------------------------------------------------------- confirm

    /// `⌘⌫`: ask before deleting `ids`.
    pub(super) fn confirm_delete(
        &mut self,
        ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ids.is_empty() {
            return;
        }
        let return_focus = self.return_focus(window, cx);
        self.close_overlays(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.confirm = Some(ConfirmUi {
            ids,
            focus,
            return_focus,
        });
        cx.notify();
    }

    fn close_confirm(&mut self, delete: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.confirm.take() else { return };
        if delete {
            self.ws.update(cx, |w, cx| w.delete(&c.ids, cx));
            window.focus(&self.focus, cx);
        } else {
            window.focus(&c.return_focus, cx);
        }
        cx.notify();
    }

    pub(super) fn render_confirm(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.confirm.as_ref()?;
        let theme = cx.theme().clone();
        let what = match c.ids.as_slice() {
            [one] => {
                let ws = self.ws.read(cx);
                let title = ws
                    .repo
                    .as_ref()
                    .and_then(|r| r.read(cx).index.by_id(one).map(|s| s.title.clone()))
                    .unwrap_or_default();
                format!("Delete {one} “{title}”?")
            }
            many => format!("Delete {} issues?", many.len()),
        };
        let panel = v_flex()
            .id("confirm")
            .key_context(CONFIRM)
            .track_focus(&c.focus)
            .on_action(cx.listener(|this, _: &ModalSubmit, window, cx| {
                this.close_confirm(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ModalCancel, window, cx| {
                this.close_confirm(false, window, cx);
            }))
            .occlude()
            .w(px(420.))
            .p_4()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg()
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(what))
            .child(super::muted(
                "The file is removed from disk. This can't be undone here.",
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(super::muted("↵ delete · esc cancel", cx).flex_1())
                    .child(
                        Button::new("confirm-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.close_confirm(false, window, cx);
                            })),
                    )
                    .child(
                        Button::new("confirm-delete")
                            .small()
                            .danger()
                            .label("Delete")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.close_confirm(true, window, cx);
                            })),
                    ),
            );
        Some(
            div()
                .id("confirm-overlay")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .bg(gpui::black().opacity(0.35))
                .child(panel)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(keys: &str) -> String {
        keys.split_whitespace()
            .map(|k| {
                key_label(&KeybindingKeystroke::new_with_mapper(
                    gpui::Keystroke::parse(k).unwrap(),
                    false,
                    &gpui::DummyKeyboardMapper,
                ))
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn keys_read_like_the_spec() {
        assert_eq!(label("cmd-k"), "⌘K");
        assert_eq!(label("g i"), "g i");
        assert_eq!(label("shift-h"), "⇧h");
        assert_eq!(label("cmd-backspace"), "⌘⌫");
        assert_eq!(label("enter"), "↵");
        assert_eq!(label("cmd-enter"), "⌘↵");
        assert_eq!(label("/"), "/");
    }
}
