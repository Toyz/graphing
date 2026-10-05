//! Design tokens: every color, size, radius and gap the UI uses.
//!
//! Pick from these scales; never type a number or a hex color in a view.
//! A value picked afresh at each place is how two panels side by side end up
//! a pixel apart and nobody can say which is right. The house style test in
//! graphing-app fails on raw `px(..)` and `rgb(..)` outside this crate and the
//! canvas painter.

use gpui_kit::{Hsla, Pixels, Rgba, px, rgb, rgba};

// ---- sizes ----

/// Icon glyphs: five steps, two apart.
pub const ICON_XS: Pixels = px(10.0);
pub const ICON_SM: Pixels = px(12.0);
pub const ICON_MD: Pixels = px(14.0);
pub const ICON_LG: Pixels = px(16.0);
pub const ICON_XL: Pixels = px(18.0);

/// Square hit targets for icon buttons.
pub const HIT_SM: Pixels = px(22.0);
pub const HIT_MD: Pixels = px(28.0);
pub const HIT_LG: Pixels = px(32.0);

pub const ROUND_XS: Pixels = px(4.0);
pub const ROUND_SM: Pixels = px(6.0);
pub const ROUND_MD: Pixels = px(9.0);
pub const ROUND_LG: Pixels = px(12.0);
pub const ROUND_PILL: Pixels = px(999.0);

/// Spacing scale, 4px based.
pub const GAP_0: Pixels = px(2.0);
pub const GAP_1: Pixels = px(4.0);
pub const GAP_2: Pixels = px(8.0);
pub const GAP_3: Pixels = px(12.0);
pub const GAP_4: Pixels = px(16.0);
pub const GAP_5: Pixels = px(24.0);

/// Type scale.
pub const TEXT_XS: Pixels = px(11.0);
pub const TEXT_SM: Pixels = px(12.0);
pub const TEXT_MD: Pixels = px(13.0);
pub const TEXT_LG: Pixels = px(15.0);
pub const TEXT_XL: Pixels = px(18.0);

pub const TITLEBAR_H: Pixels = px(40.0);
pub const STATUS_H: Pixels = px(26.0);
pub const TAB_H: Pixels = px(30.0);
pub const TAB_MAX_W: Pixels = px(200.0);
/// Dock tab bars: every panel and diagram tab.
pub const DOCK_TAB_H: Pixels = px(34.0);
pub const DOCK_TAB_MAX_W: Pixels = px(200.0);
/// The accent line under the active tab of the focused group.
pub const DOCK_INDICATOR: Pixels = px(2.0);
pub const ROW_H: Pixels = px(28.0);
pub const PANEL_HEADER_H: Pixels = px(40.0);
pub const PANEL_PAD: Pixels = px(12.0);
pub const SIDEBAR_W: Pixels = px(248.0);
pub const INSPECTOR_W: Pixels = px(280.0);
pub const SOURCE_W: Pixels = px(440.0);
pub const BOTTOM_DOCK_H: Pixels = px(200.0);
/// The sequence strip of animation steps under a diagram.
pub const SEQUENCE_H: Pixels = px(48.0);
/// The canvas minimap.
pub const MINIMAP_W: Pixels = px(200.0);
pub const MINIMAP_H: Pixels = px(130.0);
/// Readable width of the settings page.
pub const SETTINGS_W: Pixels = px(820.0);
/// Tallest a license text shows before it scrolls (Open Source settings).
pub const LICENSE_TEXT_H: Pixels = px(280.0);
/// Reopening the bottom dock never lands below this.
pub const BOTTOM_DOCK_MIN_H: Pixels = px(150.0);
/// Height of a closed bottom dock (the dock engine's strip).
pub const DOCK_STRIP_H: Pixels = px(29.0);
pub const PALETTE_W: Pixels = px(640.0);
pub const PALETTE_INPUT_H: Pixels = px(52.0);
pub const PALETTE_ROW_H: Pixels = px(38.0);
pub const PALETTE_LIST_H: Pixels = px(440.0);
pub const MENU_W: Pixels = px(240.0);
pub const MENU_ROW_H: Pixels = px(32.0);
pub const MENU_ROW_TALL_H: Pixels = px(48.0);
pub const SELECT_LIST_H: Pixels = px(360.0);
/// A select whose options need more room than its trigger (the library's
/// notation picker opens out over the canvas).
pub const SELECT_WIDE_W: Pixels = px(340.0);
/// Popover open animation.
pub const OPEN_ANIM: std::time::Duration = std::time::Duration::from_millis(140);
/// Text inputs in panels.
pub const INPUT_H: Pixels = px(32.0);
/// Node label size on the canvas at 100% zoom.
pub const CANVAS_LABEL: Pixels = px(14.0);
pub const SWATCH: Pixels = px(20.0);
pub const TILE_W: Pixels = px(64.0);
pub const TILE_GLYPH_W: Pixels = px(40.0);
pub const TILE_GLYPH_H: Pixels = px(26.0);
pub const DOT: Pixels = px(7.0);
pub const HAIRLINE: Pixels = px(1.0);
pub const WINDOW_W: Pixels = px(1440.0);
pub const WINDOW_H: Pixels = px(900.0);
pub const WINDOW_MIN_W: Pixels = px(720.0);
pub const WINDOW_MIN_H: Pixels = px(480.0);
/// Scroll distance of one wheel line.
pub const SCROLL_LINE: Pixels = px(20.0);
pub const FOCUS_RING: Pixels = px(2.0);

// ---- colors ----

/// Semantic colors. Views ask for a role (`text_muted`), never a hue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub dark: bool,
    /// The canvas and editor surface.
    pub bg: Hsla,
    /// Panels, title bar, status bar.
    pub chrome: Hsla,
    /// Floating surfaces: palette, menus, toolbars.
    pub raised: Hsla,
    pub hover: Hsla,
    pub active: Hsla,
    pub border: Hsla,
    /// Borders that must read against `raised` (floating toolbars).
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub heading: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub selection: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub shadow: Hsla,
    pub scrim: Hsla,
    /// Behind modal dialogs: darker than the palette's scrim.
    pub modal_scrim: Hsla,
    /// Canvas specifics.
    pub grid: Hsla,
    pub node_fill: Hsla,
    pub node_stroke: Hsla,
    pub edge: Hsla,
    pub group_fill: Hsla,
    pub group_stroke: Hsla,
    /// Item flow stereotypes and other notation accents (`« flow »`).
    pub flow: Hsla,
}

fn c(v: u32) -> Hsla {
    rgb(v).into()
}

fn ca(v: u32, a: u8) -> Hsla {
    let x: Rgba = rgba((v << 8) | a as u32);
    x.into()
}

impl Colors {
    /// "Midnight", shared with notesy.
    pub fn dark() -> Self {
        let bg = c(0x19191d);
        Self {
            dark: true,
            bg,
            chrome: c(0x121215),
            raised: c(0x202025),
            hover: c(0x24242b),
            active: c(0x2c2c35),
            border: c(0x26262d),
            border_strong: c(0x34343d),
            text: c(0xdcdce2),
            text_muted: c(0x8c8c98),
            text_faint: c(0x555561),
            heading: c(0xf2f2f6),
            accent: c(0xa78bfa),
            accent_soft: ca(0xa78bfa, 0x26),
            selection: ca(0xa78bfa, 0x40),
            on_accent: bg,
            danger: c(0xf26d6d),
            success: c(0x6fcf8e),
            warning: c(0xf0a35e),
            shadow: ca(0x000000, 0x96),
            scrim: ca(0x000000, 0x78),
            modal_scrim: ca(0x000000, 0xa8),
            grid: c(0x2a2a31),
            node_fill: c(0x222228),
            node_stroke: c(0x4a4a55),
            edge: c(0x7c7c88),
            group_fill: ca(0xffffff, 0x06),
            group_stroke: c(0x3a3a44),
            flow: c(0xff8a3d),
        }
    }

    /// "Paper", shared with notesy.
    pub fn light() -> Self {
        let bg = c(0xfbfbfa);
        Self {
            dark: false,
            bg,
            chrome: c(0xf1f1ef),
            raised: c(0xffffff),
            hover: c(0xe9e9e6),
            active: c(0xe0e0dc),
            border: c(0xe2e2de),
            border_strong: c(0xd2d2cc),
            text: c(0x2c2c30),
            text_muted: c(0x6c6c74),
            text_faint: c(0xa2a2aa),
            heading: c(0x111114),
            accent: c(0x7457f0),
            accent_soft: ca(0x7457f0, 0x1e),
            selection: ca(0x7457f0, 0x33),
            on_accent: bg,
            danger: c(0xd93f3f),
            success: c(0x23914f),
            warning: c(0xd46b12),
            shadow: ca(0x000000, 0x40),
            scrim: ca(0x000000, 0x30),
            modal_scrim: ca(0x101014, 0x60),
            grid: c(0xdeded8),
            node_fill: c(0xffffff),
            node_stroke: c(0xb4b4ba),
            edge: c(0x8a8a92),
            group_fill: ca(0x000000, 0x05),
            group_stroke: c(0xcfcfc8),
            flow: c(0xc24e00),
        }
    }

    pub fn for_mode(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}

/// Swatches offered for fills, borders and text, as 0xRRGGBB. `None`
/// clears the prop. Soft fills first, then strong ink colors.
/// The color picker's palette (Open Color): a neutral row, then light, mid
/// and strong rows of ten hues. Light suits fills, strong suits lines.
pub const PALETTE: [[u32; 10]; 4] = [
    [0xffffff, 0xf1f3f5, 0xdee2e6, 0xced4da, 0xadb5bd, 0x868e96, 0x495057, 0x343a40, 0x212529, 0x000000],
    [0xffe3e3, 0xffe8cc, 0xfff3bf, 0xe9fac8, 0xd3f9d8, 0xc3fae8, 0xc5f6fa, 0xd0ebff, 0xe5dbff, 0xffdeeb],
    [0xff8787, 0xffa94d, 0xffd43b, 0xa9e34b, 0x69db7c, 0x38d9a9, 0x3bc9db, 0x4dabf7, 0x9775fa, 0xf783ac],
    [0xf03e3e, 0xf76707, 0xf59f00, 0x74b816, 0x37b24d, 0x0ca678, 0x1098ad, 0x1c7ed6, 0x7048e8, 0xd6336c],
];

/// Hue names for the palette columns (row 0 is neutrals).
pub const PALETTE_HUES: [&str; 10] = ["Red", "Orange", "Yellow", "Lime", "Green", "Teal", "Cyan", "Blue", "Violet", "Pink"];

/// Width of the color picker popover.
pub const COLOR_PICKER_W: Pixels = px(268.0);
/// Confirmation dialogs.
pub const DIALOG_W: Pixels = px(460.0);
/// File > New from Template.
pub const TEMPLATES_W: Pixels = px(760.0);
/// Dialog footer buttons.
pub const BUTTON_H: Pixels = px(34.0);
/// Custom color picker: the saturation/brightness square, hue bar and knob.
pub const COLOR_SQUARE_H: Pixels = px(132.0);
pub const COLOR_HUE_H: Pixels = px(12.0);
pub const COLOR_KNOB: Pixels = px(14.0);

/// A progress bar's track.
pub const PROGRESS_H: Pixels = px(4.0);
/// Notices (toasts) at most this wide.
pub const NOTICE_W: Pixels = px(360.0);

/// Swatch color for painting a cell.
pub fn swatch(v: u32) -> Hsla {
    c(v)
}

/// Source editor syntax colors, one per token role.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Syntax {
    pub keyword: Hsla,
    /// Ids a statement declares.
    pub def: Hsla,
    /// Ids referred to (edge endpoints, members, layout).
    pub reference: Hsla,
    pub port: Hsla,
    pub stencil: Hsla,
    pub class: Hsla,
    pub key: Hsla,
    pub value: Hsla,
    pub string: Hsla,
    pub number: Hsla,
    pub color: Hsla,
    pub comment: Hsla,
    pub punct: Hsla,
    pub arrow: Hsla,
    pub error: Hsla,
}

impl Syntax {
    pub fn for_mode(dark: bool) -> Self {
        if dark {
            // Midnight: violet for structure (keywords, arrows), soft cool
            // and warm accents for the rest, ids by weight not color.
            Self {
                keyword: c(0xa78bfa),
                def: c(0xf2f2f6),
                reference: c(0xc9c9d2),
                port: c(0x8bd5ca),
                stencil: c(0x7dcfff),
                class: c(0xf0a35e),
                key: c(0x9aa5ce),
                value: c(0xe0af68),
                string: c(0x9ece6a),
                number: c(0xff9e64),
                color: c(0xff8a3d),
                comment: c(0x5c5c6b),
                punct: c(0x6c6c7a),
                arrow: c(0xa78bfa),
                error: c(0xf26d6d),
            }
        } else {
            Self {
                keyword: c(0x6d28d9),
                def: c(0x1d4ed8),
                reference: c(0x1f1f24),
                port: c(0x0f766e),
                stencil: c(0xa16207),
                class: c(0xc2410c),
                key: c(0x0369a1),
                value: c(0xb45309),
                string: c(0x15803d),
                number: c(0xc2410c),
                color: c(0xc24e00),
                comment: c(0x8a8a96),
                punct: c(0x6b6b78),
                arrow: c(0x7c3aed),
                error: c(0xdc2626),
            }
        }
    }
}
