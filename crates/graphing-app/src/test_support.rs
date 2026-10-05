//! Shared setup for gpui interaction tests.

use std::path::PathBuf;

use gpui_kit::{AppContext, Entity, Render, TestAppContext, VisualTestContext};

use crate::settings::Settings;
use crate::workspace::Workspace;

/// What the app sets up at start, with config in a scratch folder so tests
/// never read or write real settings.
pub(crate) fn init(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("graphing-test-config-{}", std::process::id()));
    // SAFETY: every test in this binary sets the same value.
    unsafe { std::env::set_var("GRAPHING_CONFIG_DIR", &dir) };
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::keymap::apply(cx, &[]);
        crate::palette::bind_keys(cx);
        graphing_ui::install(true, None, cx);
    });
}

/// A window holding what `build` makes, wrapped in `Root` like the real
/// window (inputs rely on it), drawn once.
pub(crate) fn window<V: Render>(cx: &mut TestAppContext, build: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> Entity<V>) -> (Entity<V>, &mut VisualTestContext) {
    init(cx);
    let mut made = None;
    let window = cx.add_window(|window, cx| {
        let v = build(window, cx);
        made = Some(v.clone());
        gpui_kit::base::Root::new(v, window, cx)
    });
    let cx = VisualTestContext::from_window(*window, cx).into_mut();
    cx.run_until_parked();
    (made.expect("built"), cx)
}

/// A workspace window with `files` open as tabs.
pub(crate) fn workspace(cx: &mut TestAppContext, files: Vec<PathBuf>) -> (Entity<Workspace>, &mut VisualTestContext) {
    window(cx, |window, cx| cx.new(|cx| Workspace::new(files, Settings::default(), Vec::new(), window, cx)))
}
