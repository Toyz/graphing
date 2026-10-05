//! The active palette as a gpui global, and the bridge that dresses
//! gpui-component (menus, inputs, tooltips, scrollbars) in the same colors so
//! nothing on screen comes from a different design.

use gpui_kit::component::theme::{Theme, ThemeMode, ThemeTokens};
use gpui_kit::{App, Global, Window};

use crate::tokens::*;

pub struct UiTheme {
    pub colors: Colors,
}

impl Global for UiTheme {}

pub trait UiExt {
    /// The active semantic colors.
    fn ui(&self) -> Colors;
}

impl UiExt for App {
    fn ui(&self) -> Colors {
        self.try_global::<UiTheme>().map_or_else(Colors::dark, |t| t.colors)
    }
}

/// Switch to light or dark everywhere.
pub fn install(dark: bool, window: Option<&mut Window>, cx: &mut App) {
    let mode = if dark { ThemeMode::Dark } else { ThemeMode::Light };
    Theme::change(mode, window, cx);
    let k = Colors::for_mode(dark);
    cx.set_global(UiTheme { colors: k });

    let t = Theme::global_mut(cx);
    t.font_size = TEXT_MD;
    t.radius = ROUND_SM;
    t.radius_lg = ROUND_LG;
    t.shadow = true;
    t.mono_font_size = TEXT_MD;
    // The source editor sits among the panels, so it wears panel colors:
    // chrome background and gutter, faint line numbers, a quiet active line.
    let mut hl = (*t.highlight_theme).clone();
    hl.style.editor_background = Some(k.chrome);
    hl.style.editor_foreground = Some(k.text);
    hl.style.editor_gutter_background = Some(k.chrome);
    hl.style.editor_line_number = Some(k.text_faint);
    hl.style.editor_active_line_number = Some(k.text_muted);
    hl.style.editor_active_line = Some(k.hover.opacity(0.55));
    hl.style.editor_invisible = Some(k.text_faint.opacity(0.5));
    t.highlight_theme = std::sync::Arc::new(hl);
    let s = &mut t.colors;
    s.background = k.bg;
    s.foreground = k.text;
    s.border = k.border;
    s.input = k.border_strong;
    s.ring = k.accent;
    s.caret = k.accent;
    s.selection = k.selection;
    s.muted = k.hover;
    s.muted_foreground = k.text_muted;
    s.accent = k.hover;
    s.accent_foreground = k.text;
    s.primary = k.accent;
    s.primary_hover = k.accent.opacity(0.9);
    s.primary_active = k.accent.opacity(0.8);
    s.primary_foreground = k.on_accent;
    s.secondary = k.raised;
    s.secondary_hover = k.hover;
    s.secondary_active = k.active;
    s.secondary_foreground = k.text;
    s.button = k.raised;
    s.button_hover = k.hover;
    s.button_active = k.active;
    s.button_foreground = k.text;
    s.button_primary = k.accent;
    s.button_primary_hover = k.accent.opacity(0.9);
    s.button_primary_active = k.accent.opacity(0.8);
    s.button_primary_foreground = k.on_accent;
    s.danger = k.danger;
    s.danger_foreground = k.on_accent;
    s.warning = k.warning;
    s.success = k.success;
    s.popover = k.raised;
    s.popover_foreground = k.text;
    s.list = k.raised;
    s.list_hover = k.hover;
    s.list_active = k.accent_soft;
    s.list_active_border = k.accent;
    s.sidebar = k.chrome;
    s.sidebar_border = k.border;
    s.sidebar_foreground = k.text;
    s.sidebar_accent = k.hover;
    s.sidebar_accent_foreground = k.text;
    s.title_bar = k.chrome;
    s.title_bar_border = k.border;
    s.status_bar = k.chrome;
    s.status_bar_border = k.border;
    s.tab_bar = k.chrome;
    s.tab = k.chrome;
    s.tab_active = k.bg;
    s.tab_foreground = k.text_muted;
    s.tab_active_foreground = k.text;
    s.scrollbar = gpui_kit::transparent_black();
    s.scrollbar_thumb = k.text_faint.opacity(0.5);
    s.scrollbar_thumb_hover = k.text_faint;
    s.overlay = k.scrim;
    s.window_border = k.border;
    s.link = k.accent;
    s.drag_border = k.accent;
    s.drop_target = k.accent_soft;
    t.tokens = ThemeTokens::from(t.colors);
}
