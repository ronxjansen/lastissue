//! lastissue GUI (GUI-SPEC). M3: read-only views kept live by a watcher.
//!
//! `lastissue-gui [ROOT]`: open ROOT (an issues root), else `LASTISSUE_DIR`,
//! else the root discovered from the current directory.
//! `lastissue-gui lastissue://repo/<label>/issue/<id>`: open a link (§10).

mod actions;
mod app;
mod keymap;
mod link;
mod state;
mod theme;
mod views;

fn main() {
    // A `lastissue://` URL as the argument is opened like one the system
    // hands over (handy before the bundle is registered).
    let arg = std::env::args_os().nth(1);
    let (root, url) = match arg {
        Some(a) if link::is_link(&a.to_string_lossy()) => {
            (None, Some(a.to_string_lossy().into_owned()))
        }
        a => (a.map(std::path::PathBuf::from), None),
    };
    app::run(root, url);
}
