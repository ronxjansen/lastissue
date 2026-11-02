//! Sidebar (GUI-SPEC §5): repo switcher, My issues / All, projects with
//! open counts, No project.

use super::{WorkspaceView, muted};
use crate::state::view_state::Scope;
use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

impl WorkspaceView {
    pub(super) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ws_entity = self.ws.clone();
        let ws = ws_entity.read(cx);
        let scope = ws.view.scope.clone();
        let current = ws.repo.as_ref().map(|r| r.read(cx).repo.clone());
        let (projects, counts, none_count, all_count) = match &ws.repo {
            Some(r) => {
                let ix = &r.read(cx).index;
                let (per, none) = ix.open_counts();
                let per: Vec<(String, usize)> =
                    per.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
                let total = per.iter().map(|(_, n)| n).sum::<usize>() + none;
                (ix.projects().to_vec(), per, none, total)
            }
            None => (Vec::new(), Vec::new(), 0, 0),
        };
        let repos = ws.repos.clone();

        let item = |id: SharedString,
                    label: String,
                    count: Option<usize>,
                    target: Scope,
                    cx: &mut Context<Self>| {
            let selected = scope == target;
            h_flex()
                .id(id)
                .px_2()
                .py_1()
                .mx_1()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |el| {
                    el.bg(theme.sidebar_accent)
                        .text_color(theme.sidebar_accent_foreground)
                })
                .hover(|el| el.bg(theme.sidebar_accent))
                .child(div().flex_1().truncate().child(label))
                .children(count.map(|n| muted(n.to_string(), cx)))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let target = target.clone();
                    this.ws
                        .update(cx, |w, cx| w.update_view(|v| v.scope = target, cx));
                }))
        };

        let section = |title: &'static str| {
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(title)
        };

        // Repo switcher: the open repo, expanding to the registry.
        let label = current
            .as_ref()
            .map_or_else(|| "No repo".to_string(), |r| r.label.clone());
        let mut switcher = v_flex().child(
            h_flex()
                .id("repo-switcher")
                .px_3()
                .py_2()
                .gap_1()
                .cursor_pointer()
                .hover(|el| el.bg(theme.sidebar_accent))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(label),
                )
                .child(muted(if self.repo_menu_open { "▴" } else { "▾" }, cx))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.repo_menu_open = !this.repo_menu_open;
                    cx.notify();
                })),
        );
        if self.repo_menu_open {
            for (i, repo) in repos.into_iter().enumerate() {
                let is_current = current.as_ref().is_some_and(|c| c.root == repo.root);
                let exists = repo.exists();
                let name = repo.label.clone();
                let path = repo.root.display().to_string();
                switcher = switcher.child(
                    v_flex()
                        .id(("repo", i))
                        .px_4()
                        .py_1()
                        .cursor_pointer()
                        .when(is_current, |el| el.bg(theme.sidebar_accent))
                        .hover(|el| el.bg(theme.sidebar_accent))
                        .child(div().truncate().child(if exists {
                            name
                        } else {
                            format!("{name} (missing)")
                        }))
                        .child(muted(path, cx).truncate())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.repo_menu_open = false;
                            if exists && !is_current {
                                let repo = repo.clone();
                                this.ws.update(cx, |w, cx| w.open_repo(repo, cx));
                            }
                            cx.notify();
                        })),
                );
            }
        }

        let mut list = v_flex()
            .id("sidebar-scroll")
            .flex_1()
            .overflow_y_scroll()
            .child(item(
                "mine".into(),
                "My issues".into(),
                None,
                Scope::Mine,
                cx,
            ))
            .child(item(
                "all".into(),
                "All issues".into(),
                Some(all_count),
                Scope::All,
                cx,
            ))
            .child(section("Projects"));
        for p in projects {
            let n = counts
                .iter()
                .find(|(k, _)| *k == p.id)
                .map_or(0, |(_, n)| *n);
            let name = p.slug.replace('-', " ");
            list = list.child(item(
                SharedString::from(format!("project-{}", p.id)),
                name,
                Some(n),
                Scope::Project(p.id.clone()),
                cx,
            ));
        }
        list = list.child(item(
            "no-project".into(),
            "No project".into(),
            Some(none_count),
            Scope::NoProject,
            cx,
        ));

        v_flex()
            .w(px(220.))
            .flex_none()
            .h_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(switcher)
            .child(list)
    }
}
