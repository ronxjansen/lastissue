//! Bootstrap: gpui-component, keymap, the window, and `lastissue://` URLs
//! (GUI-SPEC §10) from the system or the command line.

use crate::state::workspace::Workspace;
use crate::views::WorkspaceView;
use futures::StreamExt as _;
use gpui::{App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme};
use std::path::PathBuf;

pub fn run(root: Option<PathBuf>, url: Option<String>) {
    // URLs can arrive before the window exists (a cold launch by a link):
    // queue them and drain the queue once the workspace is up.
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<String>();
    if let Some(u) = url {
        let _ = tx.unbounded_send(u);
    }
    let app = gpui_platform::application();
    app.on_open_urls(move |urls| {
        for u in urls {
            let _ = tx.unbounded_send(u);
        }
    });
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        crate::keymap::install(crate::keymap::user_path(), cx);
        cx.on_action(|_: &crate::actions::Quit, cx| cx.quit());
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let mut workspace = None;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("lastissue".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
                let ws = cx.new(|cx| Workspace::new(root, cx));
                workspace = Some(ws.clone());
                let view = cx.new(|cx| WorkspaceView::new(ws, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("open window");
        cx.activate(true);
        let ws = workspace.expect("window built the workspace");
        cx.spawn(async move |cx| {
            while let Some(url) = rx.next().await {
                ws.update(cx, |w, cx| w.open_url(&url, cx));
                cx.update(|cx| cx.activate(true));
            }
        })
        .detach();
    });
}
