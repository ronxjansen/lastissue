//! lastissue desktop app: a Tauri shell over the core library.
//!
//! `lastissue-gui [ROOT]`: open ROOT (an issues root), else `LASTISSUE_DIR`,
//! else the root discovered from the current directory.
//! `lastissue-gui lastissue://repo/<label>/issue/<id>`: open a link.

mod commands;
mod dto;
mod format;
mod keymap;
mod link;
mod state;

use state::workspace::{self, Hub, Sink, Workspace, events};
use std::sync::Arc;
use tauri::{Emitter as _, Manager as _};
use tauri_plugin_deep_link::DeepLinkExt as _;

struct TauriSink(tauri::AppHandle);

impl Sink for TauriSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if let Err(e) = self.0.emit(event, payload) {
            eprintln!("lastissue-gui: cannot emit {event}: {e}");
        }
    }
}

fn main() {
    // A `lastissue://` URL as the argument is opened like one the system
    // hands over (handy before the bundle is registered).
    let arg = std::env::args_os().nth(1);
    let (root, url) = match arg {
        Some(a) if link::is_link(&a.to_string_lossy()) => {
            (None, Some(a.to_string_lossy().into_owned()))
        }
        // Ignore flags the macOS launcher may pass (`-psn_…`).
        Some(a) if !a.to_string_lossy().starts_with('-') => (Some(a.into()), None),
        _ => (None, None),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            let ws = Workspace::new(state::draft::Drafts::platform(), true);
            let hub = Hub::new(ws, Arc::new(TauriSink(handle.clone())));
            workspace::start(&hub, root);

            let keymap_path = keymap::user_path();
            let user_keymap = keymap::load(keymap_path.as_deref());
            let watcher = keymap_path.and_then(|p| {
                let h = handle.clone();
                keymap::watch(p, move |km| {
                    if let Some(k) = h.try_state::<commands::Keymap>() {
                        *k.current
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = km.clone();
                    }
                    if let Err(e) = h.emit(events::KEYMAP, km) {
                        eprintln!("lastissue-gui: cannot emit {}: {e}", events::KEYMAP);
                    }
                })
                .ok()
            });
            app.manage(commands::Keymap {
                current: std::sync::Mutex::new(user_keymap),
                _watcher: watcher,
            });

            let h = hub.clone();
            app.deep_link().on_open_url(move |ev| {
                for u in ev.urls() {
                    workspace::open_url(&h, u.as_str());
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for u in urls {
                    workspace::open_url(&hub, u.as_str());
                }
            }
            if let Some(u) = url.as_deref() {
                workspace::open_url(&hub, u);
            }
            app.manage(hub);
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("run lastissue");
}
