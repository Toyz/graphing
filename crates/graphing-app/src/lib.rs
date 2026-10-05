//! gpui shell for graphing.

mod color;
mod combine;
mod confirm;
mod dock;
pub mod export;
mod files;
mod media;
mod inspector;
pub mod keymap;
mod library;
mod license;
mod menubar;
mod open_source;
mod ops;
mod paint;
mod palette;
mod plugins;
pub mod settings;
mod sequence;
mod settings_pane;
mod templates;
mod source;
#[cfg(test)]
mod test_support;
mod view;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::{Action, App, AppContext, WindowBounds, WindowDecorations, WindowOptions, actions, size};
use graphing_ui::tokens::{WINDOW_H, WINDOW_MIN_H, WINDOW_MIN_W, WINDOW_W};
use schemars::JsonSchema;
use serde::Deserialize;

use workspace::Workspace;

actions!(
    graphing,
    [
        // Files and tabs
        NewFile,
        NewFromTemplate,
        OpenFile,
        Save,
        SaveAs,
        CloseTab,
        NextTab,
        PrevTab,
        ExportSvg,
        ExportPng,
        ExportSysml,
        InsertImage,
        ImportFile,
        OpenSettings,
        ReloadSettings,
        Quit,
        // Panels and look
        ToggleLeft,
        ToggleRight,
        ToggleSource,
        ToggleOutline,
        ToggleProblems,
        ResetLayout,
        CycleTheme,
        CommandPalette,
        // Editing
        Undo,
        Redo,
        Delete,
        Copy,
        Cut,
        Paste,
        Duplicate,
        SelectAll,
        Escape,
        Rename,
        NudgeLeft,
        NudgeRight,
        NudgeUp,
        NudgeDown,
        NudgeLeftBig,
        NudgeRightBig,
        NudgeUpBig,
        NudgeDownBig,
        // View
        FitView,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        // Animation
        PlayAnimation,
        ToggleSequence,
        NewStep,
        ExportAnimation,
        // Arrange
        Relayout,
        AlignLeft,
        AlignCenter,
        AlignRight,
        AlignTop,
        AlignMiddle,
        AlignBottom,
        SpreadH,
        SpreadV,
        SameSize,
        GroupSelection,
        Ungroup,
        ReloadPacks,
        OpenPacksFolder,
        ReloadPlugins,
        OpenPluginsFolder,
    ]
);

/// Import a file as one format (`mermaid`, `drawio`, `sysml`, `visio`).
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = graphing)]
pub struct ImportAs {
    pub format: String,
}

/// Run a command a plugin registered.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = graphing)]
pub struct RunPluginCommand {
    pub plugin: String,
    pub command: String,
}

/// Load shape packs from `<config>/packs` (and any extra dirs). One message
/// per pack that failed to load.
pub fn load_packs(extra: &[PathBuf]) -> Vec<String> {
    let mut errors = graphing_scene::stencils::load_dir(&settings::config_dir().join("packs"));
    for d in extra {
        errors.extend(graphing_scene::stencils::load_dir(d));
    }
    errors
}

/// Add a node of `stencil` in the middle of the canvas.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = graphing)]
pub struct AddShape {
    pub stencil: String,
}

/// Open entry `index` of the recent files list.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = graphing)]
pub struct OpenRecent {
    pub index: usize,
}

/// Open a window with `files` as tabs (or the last session when empty).
pub fn run(files: Vec<PathBuf>) -> anyhow::Result<()> {
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(move |cx: &mut App| {
        gpui_kit::init(cx);
        palette::bind_keys(cx);
        let (settings, mut errors) = settings::Settings::load();
        errors.extend(load_packs(&[]));
        errors.extend(keymap::apply(cx, &settings.keybindings));
        for e in &errors {
            tracing::warn!("{e}");
        }
        cx.on_action(|_: &Quit, cx| cx.quit());
        let bounds = gpui_kit::Bounds::centered(None, size(WINDOW_W, WINDOW_H), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_decorations: Some(WindowDecorations::Client),
            window_min_size: Some(size(WINDOW_MIN_W, WINDOW_MIN_H)),
            ..TitleBar::window_options()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(files, settings, errors, window, cx))
        });
        if let Err(e) = opened {
            tracing::error!("failed to open window: {e:#}");
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
    Ok(())
}

/// `GRAPHING_TRACE=1` prints how long renders and paints take (debugging
/// slow frames).
pub(crate) fn trace(label: &str, since: std::time::Instant) {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("GRAPHING_TRACE").is_some());
    if *ON {
        eprintln!("[trace] {label}: {:?}", since.elapsed());
    }
}
