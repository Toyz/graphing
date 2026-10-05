//! The Settings tab, laid out like VS Code's settings editor: one search box
//! over every setting and shortcut, a table of contents, and rows titled
//! `Section: Name` with a description, their settings.json key and a mark
//! when changed from the default. Every change writes settings.json at once
//! and applies live; the file stays hand-editable and unknown keys survive.

use gpui_kit::component::Icon;
use gpui_kit::component::switch::Switch;
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};
use graphing_ui::UiExt;
use graphing_ui::kit::{self, IconButton, Lucide, Segment, Segmented, TextButton};
use graphing_ui::tokens::*;

use crate::keymap;
use crate::settings::{ConfirmDelete, Play, Settings, ThemeChoice};
use crate::workspace::Workspace;
use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Appearance,
    Canvas,
    Panels,
    Shortcuts,
    Extensions,
    OpenSource,
}

impl Section {
    pub(crate) const ALL: [Section; 6] = [Section::Appearance, Section::Canvas, Section::Panels, Section::Shortcuts, Section::Extensions, Section::OpenSource];

    fn title(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Canvas => "Canvas",
            Section::Panels => "Panels",
            Section::Shortcuts => "Keyboard Shortcuts",
            Section::Extensions => "Extensions",
            Section::OpenSource => "Open Source",
        }
    }

    fn icon(self) -> Lucide {
        match self {
            Section::Appearance => Lucide::Palette,
            Section::Canvas => Lucide::Grid3x3,
            Section::Panels => Lucide::LayoutDashboard,
            Section::Shortcuts => Lucide::Keyboard,
            Section::Extensions => Lucide::Puzzle,
            Section::OpenSource => Lucide::Scale,
        }
    }
}

type Act = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);

enum Control {
    /// `(label, value)` choices, with getter and setter by value.
    Choice(&'static [(&'static str, &'static str)], fn(&Settings) -> &'static str, fn(&mut Settings, &str)),
    Toggle(fn(&Settings) -> bool, fn(&mut Settings, bool)),
    /// Buttons that do something rather than hold a value.
    Actions(&'static [(&'static str, Lucide, Act)]),
}

struct SettingDef {
    /// Path in settings.json, or empty for actions.
    key: &'static str,
    section: Section,
    title: &'static str,
    description: &'static str,
    control: Control,
}

fn defs() -> Vec<SettingDef> {
    use Control::*;
    vec![
        SettingDef {
            key: "theme",
            section: Section::Appearance,
            title: "Theme",
            description: "Follow the system, or always use the light or dark palette.",
            control: Choice(
                &[("System", "system"), ("Light", "light"), ("Dark", "dark")],
                |s| match s.theme {
                    ThemeChoice::System => "system",
                    ThemeChoice::Light => "light",
                    ThemeChoice::Dark => "dark",
                },
                |s, v| {
                    s.theme = match v {
                        "light" => ThemeChoice::Light,
                        "dark" => ThemeChoice::Dark,
                        _ => ThemeChoice::System,
                    }
                },
            ),
        },
        SettingDef {
            key: "canvas.grid",
            section: Section::Canvas,
            title: "Grid Size",
            description: "Diagram units between snap points.",
            control: Choice(
                &[("5", "5"), ("10", "10"), ("20", "20"), ("40", "40")],
                |s| match s.grid as u32 {
                    5 => "5",
                    20 => "20",
                    40 => "40",
                    _ => "10",
                },
                |s, v| s.grid = v.parse().unwrap_or(10.0),
            ),
        },
        SettingDef {
            key: "canvas.snap",
            section: Section::Canvas,
            title: "Snap To Grid",
            description: "Snap shapes to the grid while dragging. Hold Alt to place freely.",
            control: Toggle(|s| s.snap, |s, v| s.snap = v),
        },
        SettingDef {
            key: "canvas.show_grid",
            section: Section::Canvas,
            title: "Show Grid",
            description: "Draw grid dots behind the diagram.",
            control: Toggle(|s| s.show_grid, |s, v| s.show_grid = v),
        },
        SettingDef {
            key: "canvas.animations",
            section: Section::Canvas,
            title: "Moving Pictures",
            description: "When animated GIF, WebP and AVIF pictures play: whenever the window is in front, only the one under the pointer, or never.",
            control: Choice(
                &[("Always", "always"), ("On hover", "hover"), ("Never", "never")],
                |s| match s.play {
                    Play::Always => "always",
                    Play::Hover => "hover",
                    Play::Never => "never",
                },
                |s, v| {
                    s.play = match v {
                        "hover" => Play::Hover,
                        "never" => Play::Never,
                        _ => Play::Always,
                    }
                },
            ),
        },
        SettingDef {
            key: "editing.confirm_delete",
            section: Section::Canvas,
            title: "Confirm Delete",
            description: "Ask before deleting: always, only for groups and several items at once, or never. Undo works either way.",
            control: Choice(
                &[("Always", "always"), ("Groups and multiple", "groups"), ("Never", "never")],
                |s| match s.confirm_delete {
                    ConfirmDelete::Always => "always",
                    ConfirmDelete::Groups => "groups",
                    ConfirmDelete::Never => "never",
                },
                |s, v| {
                    s.confirm_delete = match v {
                        "groups" => ConfirmDelete::Groups,
                        "never" => ConfirmDelete::Never,
                        _ => ConfirmDelete::Always,
                    }
                },
            ),
        },
        SettingDef {
            key: "panels.shapes",
            section: Section::Panels,
            title: "Shapes And Outline",
            description: "Open the shapes and outline panes in the left dock when no layout is saved.",
            control: Toggle(|s| s.shapes_panel, |s, v| s.shapes_panel = v),
        },
        SettingDef {
            key: "panels.inspector",
            section: Section::Panels,
            title: "Inspector",
            description: "Open the inspector in the right dock when no layout is saved.",
            control: Toggle(|s| s.inspector, |s, v| s.inspector = v),
        },
        SettingDef {
            key: "panels.source",
            section: Section::Panels,
            title: "Source",
            description: "Open the source editor beside the diagram when no layout is saved.",
            control: Toggle(|s| s.source_panel, |s, v| s.source_panel = v),
        },
        SettingDef {
            key: "",
            section: Section::Panels,
            title: "Layout",
            description: "Put every pane back where it started.",
            control: Actions(&[("Reset layout", Lucide::LayoutDashboard, |ws, window, cx| ws.confirm_reset_layout(window, cx))]),
        },
        SettingDef {
            key: "",
            section: Section::Extensions,
            title: "Shape Packs",
            description: "JSON packs of shapes, edge kinds and group kinds, from the packs folder.",
            control: Actions(&[
                ("Open folder", Lucide::FolderOpen, |_, window, cx| window.dispatch_action(Box::new(OpenPacksFolder), cx)),
                ("Reload", Lucide::RefreshCw, |_, window, cx| window.dispatch_action(Box::new(ReloadPacks), cx)),
            ]),
        },
        SettingDef {
            key: "",
            section: Section::Extensions,
            title: "Plugins",
            description: "Rune scripts that add commands and shapes, from the plugins folder.",
            control: Actions(&[
                ("Open folder", Lucide::FolderOpen, |_, window, cx| window.dispatch_action(Box::new(OpenPluginsFolder), cx)),
                ("Reload", Lucide::RefreshCw, |_, window, cx| window.dispatch_action(Box::new(ReloadPlugins), cx)),
            ]),
        },
    ]
}

/// Every word of `query` appears in one of `fields` (case-insensitive).
/// `ctrl+s` matches `ctrl-s`, as in VS Code.
pub(crate) fn matches(query: &str, fields: &[&str]) -> bool {
    let hay = fields.join(" ").to_lowercase().replace('+', "-");
    query.to_lowercase().replace('+', "-").split_whitespace().all(|w| hay.contains(w))
}

fn def_matches(d: &SettingDef, q: &str) -> bool {
    matches(q, &[d.section.title(), d.title, d.description, d.key])
}

/// A shortcut row: the command and what is bound to it.
struct ShortcutRow {
    name: SharedString,
    group: &'static str,
    icon: Icon,
    action: String,
    keys: Vec<keymap::Effective>,
    changed: bool,
}

impl ShortcutRow {
    fn matches(&self, q: &str) -> bool {
        let keys: Vec<&str> = self.keys.iter().map(|e| e.keys.as_str()).collect();
        let ctx: Vec<&str> = self.keys.iter().filter_map(|e| e.context.as_deref()).collect();
        matches(q, &[&self.name, self.group, &self.action, &keys.join(" "), &ctx.join(" "), "keyboard shortcut keybinding"])
    }
}

impl Workspace {
    /// Change settings, save the file and apply what changed.
    pub(crate) fn change_settings(&mut self, f: impl FnOnce(&mut Settings), window: &mut Window, cx: &mut Context<Self>) {
        f(&mut self.settings);
        self.settings.save();
        self.theme = self.settings.theme;
        self.apply_theme(window, cx);
        self.apply_canvas_settings(cx);
        let errors = keymap::apply(cx, &self.settings.keybindings);
        if !errors.is_empty() {
            self.toast(errors.join("; "), window, cx);
        }
        cx.notify();
    }

    fn shortcut_rows(&self, window: &Window, cx: &App) -> Vec<ShortcutRow> {
        let eff = keymap::effective(&self.settings.keybindings);
        self.commands(window, cx)
            .into_iter()
            .map(|c| {
                let action = c.action.name().to_string();
                let keys: Vec<keymap::Effective> = eff.iter().filter(|e| e.action == action).cloned().collect();
                let changed = keys.iter().any(|e| e.user)
                    || self.settings.keybindings.iter().any(|b| b.action.is_none() && keymap::defaults().any(|(dk, da, _)| da == action && dk == keymap::canonical(&b.keys)));
                ShortcutRow { name: c.name, group: c.group, icon: c.icon, action, keys, changed }
            })
            .collect()
    }

    pub(crate) fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let query = self.settings_query.read(cx).value().to_string();
        let defs = defs();
        let shortcuts: Vec<ShortcutRow> = self.shortcut_rows(window, cx).into_iter().filter(|r| r.matches(&query)).collect();
        let crates = oss_matches(&query);
        // Sections with something to show, in order.
        let shown: Vec<Section> = Section::ALL
            .into_iter()
            .filter(|s| match s {
                Section::Shortcuts => !shortcuts.is_empty(),
                Section::OpenSource => !crates.is_empty(),
                _ => defs.iter().any(|d| d.section == *s && def_matches(d, &query)),
            })
            .collect();
        let found = defs.iter().filter(|d| def_matches(d, &query)).count() + shortcuts.len() + crates.len();

        let search = kit::text_input(&self.settings_query).prefix(Icon::new(Lucide::Search).size(ICON_SM).text_color(k.text_faint));
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(GAP_3)
            .px(GAP_5)
            .py(GAP_3)
            .border_b_1()
            .border_color(k.border)
            .child(div().flex_1().min_w_0().child(search))
            .child(div().flex_none().text_size(TEXT_XS).text_color(k.text_faint).child(if query.is_empty() { String::new() } else { format!("{found} found") }))
            .child(TextButton::new("set-open-file", "Edit JSON").icon(Lucide::FileCode).on_click(cx.listener(|ws, _, window, cx| ws.open_settings_file(window, cx))));

        // Table of contents: jump to a section.
        let toc = div().flex_none().w(SIDEBAR_W * 0.8).pt(GAP_4).px(GAP_2).flex().flex_col().gap(GAP_0).children(Section::ALL.into_iter().map(|s| {
            let ix = shown.iter().position(|x| *x == s);
            let active = self.settings_section == Some(s) && ix.is_some();
            let count = match s {
                Section::Shortcuts => shortcuts.len(),
                Section::OpenSource => crates.len(),
                _ => defs.iter().filter(|d| d.section == s && def_matches(d, &query)).count(),
            };
            let mut row = kit::Row::new(SharedString::from(format!("toc-{}", s.title())), s.title()).icon(s.icon()).selected(active);
            if !query.is_empty() {
                row = row.meta(count.to_string());
            }
            row.on_click(cx.listener(move |ws, _, _, cx| {
                if let Some(ix) = ix {
                    ws.settings_section = Some(s);
                    ws.settings_scroll.scroll_to_top_of_item(ix);
                    cx.notify();
                }
            }))
        }));

        let mut body: Vec<AnyElement> = Vec::new();
        for s in &shown {
            let content = if *s == Section::Shortcuts {
                self.render_shortcuts(shortcuts.as_slice(), cx)
            } else if *s == Section::OpenSource {
                self.render_open_source(&crates, cx)
            } else {
                let rows: Vec<AnyElement> = defs.iter().filter(|d| d.section == *s && def_matches(d, &query)).map(|d| self.setting_row(d, cx)).collect();
                div().flex().flex_col().gap(GAP_1).children(rows).into_any_element()
            };
            body.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(GAP_2)
                    .pb(GAP_5)
                    .child(div().text_size(TEXT_LG).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.heading).child(s.title()))
                    .child(content)
                    .into_any_element(),
            );
        }
        if body.is_empty() {
            body.push(kit::empty_state(Lucide::SearchX, "No settings found", "Try another word, a setting key like canvas.grid, or keys like ctrl+s", cx).into_any_element());
        }

        div()
            .size_full()
            .bg(k.bg)
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(toc)
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.settings_scroll)
                            .px(GAP_5)
                            .pt(GAP_4)
                            .children(body.into_iter().map(|b| div().max_w(SETTINGS_W).child(b))),
                    ),
            )
            .into_any_element()
    }

    /// `Section: Title`, description, control; an accent bar and a reset
    /// button when the value differs from the default.
    fn setting_row(&self, d: &SettingDef, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let defaults = Settings::default();
        let (control, modified): (AnyElement, bool) = match &d.control {
            Control::Choice(options, get, set) => {
                let (current, set) = (get(&self.settings), *set);
                let segs = options
                    .iter()
                    .map(|(label, value)| {
                        let value = *value;
                        Segment::new(*label, current == value, cx.listener(move |ws, _, window, cx| ws.change_settings(move |s| set(s, value), window, cx)))
                    })
                    .collect();
                (Segmented::new(SharedString::from(format!("set-{}", d.key)), segs).into_any_element(), current != get(&defaults))
            }
            Control::Toggle(get, set) => {
                let (on, set) = (get(&self.settings), *set);
                let switch = Switch::new(SharedString::from(format!("set-{}", d.key))).checked(on).on_change(cx.listener(move |ws, v: &bool, window, cx| {
                    let v = *v;
                    ws.change_settings(move |s| set(s, v), window, cx);
                }));
                (switch.into_any_element(), on != get(&defaults))
            }
            Control::Actions(buttons) => (
                div()
                    .flex()
                    .gap(GAP_2)
                    .children(buttons.iter().enumerate().map(|(i, (label, icon, act))| {
                        let act = *act;
                        TextButton::new(SharedString::from(format!("set-{}-{i}", d.title)), *label).icon(*icon).on_click(cx.listener(move |ws, _, window, cx| act(ws, window, cx)))
                    }))
                    .into_any_element(),
                false,
            ),
        };
        let reset = modified.then(|| {
            let key = d.key;
            IconButton::new(SharedString::from(format!("reset-{key}")), Lucide::RotateCcw).small().tooltip("Reset to default").on_click(cx.listener(move |ws, _, window, cx| {
                ws.change_settings(move |s| reset_key(s, key), window, cx);
            }))
        });
        let group: SharedString = format!("setting-{}-{}", d.section.title(), d.title).into();
        div()
            .id(group.clone())
            .relative()
            .flex()
            .flex_col()
            .gap(GAP_1)
            .pl(GAP_4)
            .pr(GAP_3)
            .py(GAP_3)
            .rounded(ROUND_SM)
            .hover(|s| s.bg(k.chrome))
            // Changed from the default: an accent bar, like VS Code.
            .when(modified, |el| el.child(div().absolute().left_0().top(GAP_3).bottom(GAP_3).w(DOCK_INDICATOR).rounded(ROUND_PILL).bg(k.accent)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP_1)
                    .text_size(TEXT_MD)
                    .child(div().text_color(k.text_muted).child(format!("{}:", d.section.title())))
                    .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.text).child(d.title))
                    .when(modified, |el| el.child(div().pl(GAP_1).text_size(TEXT_XS).text_color(k.accent).child("Modified")))
                    .child(div().flex_1())
                    .when(!d.key.is_empty(), |el| el.child(div().text_size(TEXT_XS).font_family(cx.mono()).text_color(k.text_faint).child(d.key)))
                    .children(reset),
            )
            .child(div().text_size(TEXT_SM).text_color(k.text_muted).child(d.description))
            // A row, so controls keep their natural width instead of stretching.
            .child(div().pt(GAP_1).flex().child(control))
            .into_any_element()
    }

    /// Command, keybinding, when, source; double-click or the pencil to
    /// record new keys.
    fn render_shortcuts(&self, rows: &[ShortcutRow], cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let recording = self.recording.clone();
        let head = |t: &'static str| div().text_size(TEXT_XS).text_color(k.text_faint).child(t);
        let header = div()
            .h(ROW_H)
            .px(GAP_3)
            .flex()
            .items_center()
            .gap(GAP_3)
            .border_b_1()
            .border_color(k.border)
            .child(div().flex_1().min_w_0().child(head("Command")))
            .child(div().flex_none().w(SIDEBAR_W).child(head("Keybinding")))
            .child(div().flex_none().w(HIT_LG * 3.0).child(head("When")))
            .child(div().flex_none().w(HIT_LG * 2.0).child(head("Source")))
            .child(div().flex_none().w(HIT_SM * 2.0 + GAP_1));
        let body: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let is_rec = recording.as_deref() == Some(r.action.as_str());
                let keys: AnyElement = if is_rec {
                    div().text_size(TEXT_SM).text_color(k.accent).child("Press keys, Esc cancels").into_any_element()
                } else if r.keys.is_empty() {
                    div().text_size(TEXT_SM).text_color(k.text_faint).child("\u{2014}").into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(GAP_2)
                        .children(r.keys.iter().map(|e| div().flex().gap(GAP_0).children(graphing_ui::menu::key_chips(&e.keys).into_iter().map(|chip| kit::kbd(chip, cx)))))
                        .into_any_element()
                };
                let when = r.keys.first().and_then(|e| e.context.clone()).or_else(|| keymap::context_of(&r.action)).unwrap_or_else(|| "Global".into());
                let source = if r.changed {
                    "User"
                } else if r.keys.is_empty() {
                    ""
                } else {
                    "Default"
                };
                let action = r.action.clone();
                let record = {
                    let action = action.clone();
                    IconButton::new(SharedString::from(format!("rec-{i}")), Lucide::Pencil).small().tooltip("Change keybinding").on_click(cx.listener(move |ws, _, window, cx| ws.start_recording(&action, window, cx)))
                };
                let reset = r.changed.then(|| {
                    let action = action.clone();
                    IconButton::new(SharedString::from(format!("kreset-{i}")), Lucide::RotateCcw).small().tooltip("Reset keybinding").on_click(cx.listener(move |ws, _, window, cx| {
                        let action = action.clone();
                        ws.change_settings(move |s| keymap::reset(&mut s.keybindings, &action), window, cx);
                    }))
                });
                let group: SharedString = format!("shortcut-{i}").into();
                div()
                    .id(group.clone())
                    .group(group.clone())
                    .relative()
                    .min_h(ROW_H + GAP_2)
                    .px(GAP_3)
                    .flex()
                    .items_center()
                    .gap(GAP_3)
                    .rounded(ROUND_SM)
                    .when(is_rec, |d| d.bg(k.accent_soft))
                    .when(!is_rec, |d| d.hover(|d| d.bg(k.chrome)))
                    .when(r.changed, |el| el.child(div().absolute().left_0().top(GAP_2).bottom(GAP_2).w(DOCK_INDICATOR).rounded(ROUND_PILL).bg(k.accent)))
                    .on_click(cx.listener(move |ws, ev: &gpui_kit::ClickEvent, window, cx| {
                        if ev.click_count() >= 2 {
                            ws.start_recording(&action, window, cx);
                        }
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(GAP_2)
                            .child(r.icon.clone().size(ICON_SM).text_color(k.text_muted))
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(TEXT_SM).text_color(k.text).child(r.name.clone()))
                            .child(div().flex_none().text_size(TEXT_XS).text_color(k.text_faint).child(r.group)),
                    )
                    .child(div().flex_none().w(SIDEBAR_W).child(keys))
                    .child(div().flex_none().w(HIT_LG * 3.0).text_size(TEXT_XS).font_family(cx.mono()).text_color(k.text_muted).child(when))
                    .child(div().flex_none().w(HIT_LG * 2.0).text_size(TEXT_XS).text_color(if r.changed { k.accent } else { k.text_faint }).child(source))
                    .child(
                        div()
                            .flex_none()
                            .w(HIT_SM * 2.0 + GAP_1)
                            .flex()
                            .gap(GAP_1)
                            .justify_end()
                            .when(!is_rec && !r.changed, |d| d.invisible().group_hover(group, |d| d.visible()))
                            .children(reset)
                            .child(record),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("shortcuts")
            .track_focus(&self.settings_focus)
            .capture_key_down(cx.listener(|ws, ev: &KeyDownEvent, window, cx| ws.record_key(ev, window, cx)))
            .flex()
            .flex_col()
            .child(header)
            .children(body)
            .into_any_element()
    }

    /// What graphing is built from: a summary of the licenses, then every
    /// crate with its license; open one for its source and license texts.
    fn render_open_source(&self, list: &[usize], cx: &mut Context<Self>) -> AnyElement {
        use crate::open_source::{self as oss, CRATES};
        let k = cx.ui();
        let counts = oss::family_counts();
        let named = oss::named_families(&counts, 5);
        let other: usize = counts.iter().filter(|(f, _)| !named.contains(f)).map(|(_, n)| n).sum();
        // A license's pill filters the list to it.
        let mut pills: Vec<AnyElement> = counts
            .iter()
            .filter(|(f, _)| named.contains(f))
            .map(|&(f, n)| {
                div()
                    .id(SharedString::from(format!("oss-family-{f}")))
                    .cursor_pointer()
                    .rounded(ROUND_PILL)
                    .hover(|d| d.opacity(0.8))
                    .on_click(cx.listener(move |ws, _, window, cx| ws.settings_query.update(cx, |s, cx| s.set_value(f.to_string(), window, cx))))
                    .child(kit::stat_pill(Lucide::Scale, format!("{f}  {n}"), cx))
                    .into_any_element()
            })
            .collect();
        if other > 0 {
            pills.push(kit::stat_pill(Lucide::Scale, format!("{}  {other}", oss::OTHER), cx).into_any_element());
        }
        let build = if oss::COMMIT.is_empty() { format!("graphing {}", oss::VERSION) } else { format!("graphing {} ({})", oss::VERSION, oss::COMMIT) };
        let intro = div()
            .flex()
            .flex_col()
            .gap(GAP_2)
            .px(GAP_3)
            .pb(GAP_2)
            .child(
                div()
                    .text_size(TEXT_SM)
                    .text_color(k.text_muted)
                    .child(format!("{build} is built from {} open source crates. Each is listed with its license and where its source lives; MPL-2.0 crates are used unmodified.", CRATES.len())),
            )
            .child(div().flex().flex_wrap().gap(GAP_2).children(pills));

        let rows = list.iter().map(|&i| {
            let c = &CRATES[i];
            let open = self.oss_open == Some(i);
            let family = &oss::families()[i];
            let strict = oss::has_conditions(family);
            let repo = c.repository;
            let head = div()
                .id(SharedString::from(format!("oss-{i}")))
                .h(ROW_H + GAP_1)
                .px(GAP_3)
                .flex()
                .items_center()
                .gap(GAP_2)
                .rounded(ROUND_SM)
                .cursor_pointer()
                .hover(|d| d.bg(k.chrome))
                .on_click(cx.listener(move |ws, _, _, cx| {
                    ws.oss_open = if ws.oss_open == Some(i) { None } else { Some(i) };
                    cx.notify();
                }))
                .child(Icon::new(if open { Lucide::ChevronDown } else { Lucide::ChevronRight }).size(ICON_SM).text_color(k.text_faint))
                .child(div().flex_none().text_size(TEXT_SM).text_color(k.text).child(c.name))
                .child(div().flex_none().text_size(TEXT_XS).text_color(k.text_faint).child(c.version))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(TEXT_XS).text_color(k.text_faint).child(c.description))
                .child(div().flex_none().text_size(TEXT_XS).font_family(cx.mono()).text_color(if strict { k.warning } else { k.text_muted }).child(family.clone()))
                .when(!repo.is_empty(), |el| {
                    el.child(IconButton::new(SharedString::from(format!("oss-src-{i}")), Lucide::ExternalLink).small().tooltip(repo).on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cx.open_url(repo);
                    }))
                });
            let body = open.then(|| {
                let field = |label: &'static str, value: String| {
                    div()
                        .flex()
                        .gap(GAP_2)
                        .text_size(TEXT_SM)
                        .child(div().flex_none().w(HIT_LG * 2.5).text_color(k.text_faint).child(label))
                        .child(div().flex_1().min_w_0().text_color(k.text).child(value))
                };
                let texts = oss::texts_for(c, family);
                div()
                    .flex()
                    .flex_col()
                    .gap(GAP_2)
                    .pl(GAP_5)
                    .pr(GAP_3)
                    .pb(GAP_3)
                    .child(field("License", c.license.to_string()))
                    .when(c.license != family.as_str(), |el| el.child(field("Taken under", family.clone())))
                    .when(!c.authors.is_empty(), |el| el.child(field("Authors", c.authors.to_string())))
                    .when(!repo.is_empty(), |el| el.child(field("Source", format!("{repo} (version {})", c.version))))
                    .when(strict, |el| el.child(field("Note", "Used unmodified; its source is at the address above.".to_string())))
                    .when(texts.is_empty(), |el| el.child(div().text_size(TEXT_SM).text_color(k.text_faint).child("This crate ships no license file; its license is the one named above.")))
                    .children(texts.into_iter().enumerate().map(|(t, (name, text))| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(GAP_1)
                            .child(div().flex().items_center().gap(GAP_1).text_size(TEXT_XS).text_color(k.text_muted).child(Icon::new(Lucide::FileText).size(ICON_SM)).child(name))
                            .child(
                                div()
                                    .id(SharedString::from(format!("oss-text-{i}-{t}")))
                                    .max_h(LICENSE_TEXT_H)
                                    .overflow_y_scroll()
                                    .p(GAP_3)
                                    .rounded(ROUND_SM)
                                    .bg(k.chrome)
                                    .border_1()
                                    .border_color(k.border)
                                    .text_size(TEXT_XS)
                                    .font_family(cx.mono())
                                    .text_color(k.text_muted)
                                    .child(text),
                            )
                    }))
            });
            div().flex().flex_col().child(head).children(body)
        });
        div().flex().flex_col().gap(GAP_0).child(intro).children(rows).into_any_element()
    }

    fn start_recording(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = Some(action.to_string());
        window.focus(&self.settings_focus, cx);
        cx.notify();
    }

    /// While recording, the next keystroke becomes the shortcut.
    fn record_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.recording.clone() else { return };
        cx.stop_propagation();
        let keys = ev.keystroke.unparse();
        self.recording = None;
        if ev.keystroke.key == "escape" && !ev.keystroke.modifiers.modified() {
            cx.notify();
            return;
        }
        if let Some(other) = keymap::conflict(&self.settings.keybindings, &action, &keys) {
            let other = other.trim_start_matches("graphing::").to_string();
            self.toast(format!("{keys} was {other}; now {}", action.trim_start_matches("graphing::")), window, cx);
        }
        self.change_settings(move |s| keymap::rebind(&mut s.keybindings, &action, &keys), window, cx);
    }

    pub(crate) fn open_settings_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = Settings::ensure_file();
        cx.open_with_system(&path);
        self.toast(format!("opened {}; run Reload Settings after editing", path.display()), window, cx);
    }
}

/// The crates `query` finds in Open Source: all of them when the query is
/// empty or names the section itself.
fn oss_matches(query: &str) -> Vec<usize> {
    let all = matches(query, &["open source licenses notices credits"]);
    crate::open_source::CRATES
        .iter()
        .enumerate()
        .filter(|(_, c)| all || crate_matches(c, query))
        .map(|(i, _)| i)
        .collect()
}

/// Every word of `query` is in the crate's name or license, or starts a
/// word of its description or authors (so `mpl` finds MPL-2.0, not "simple").
fn crate_matches(c: &crate::open_source::Crate, query: &str) -> bool {
    let (name, license) = (c.name.to_lowercase(), c.license.to_lowercase());
    let words: Vec<String> = format!("{} {}", c.description, c.authors).split(|ch: char| !ch.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    query.to_lowercase().split_whitespace().all(|q| name.contains(q) || license.contains(q) || words.iter().any(|w| w.starts_with(q)))
}

/// Put one setting back to its default.
fn reset_key(s: &mut Settings, key: &str) {
    let d = Settings::default();
    match key {
        "theme" => s.theme = d.theme,
        "canvas.grid" => s.grid = d.grid,
        "canvas.snap" => s.snap = d.snap,
        "canvas.show_grid" => s.show_grid = d.show_grid,
        "editing.confirm_delete" => s.confirm_delete = d.confirm_delete,
        "canvas.animations" => s.play = d.play,
        "panels.shapes" => s.shapes_panel = d.shapes_panel,
        "panels.inspector" => s.inspector = d.inspector,
        "panels.source" => s.source_panel = d.source_panel,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_settings_by_title_key_or_words() {
        let hits = |q: &str| defs().iter().filter(|d| def_matches(d, q)).map(|d| d.title).collect::<Vec<_>>();
        assert_eq!(hits("grid"), ["Grid Size", "Snap To Grid", "Show Grid"]);
        assert_eq!(hits("canvas.snap"), ["Snap To Grid"]);
        assert_eq!(hits("dark theme"), ["Theme"]);
        assert!(hits("nothing like this").is_empty());
        assert!(matches("ctrl+s", &["save", "ctrl-s"]));
    }

    #[test]
    fn crate_search_matches_words_not_fragments() {
        let c = crate::open_source::Crate { name: "mp4parse", version: "0.17.0", license: "MPL-2.0", repository: "", description: "Parser for ISO base media file format", authors: "Ralph Giles", texts: &[] };
        let simple = crate::open_source::Crate { description: "A simple implementation", license: "MIT", name: "x", ..c };
        assert!(crate_matches(&c, "mpl") && crate_matches(&c, "MPL-2.0") && crate_matches(&c, "mp4") && crate_matches(&c, "media"));
        assert!(!crate_matches(&simple, "mpl"));
    }

    #[test]
    fn every_key_resets() {
        let mut s = Settings::default();
        s.theme = ThemeChoice::Dark;
        s.grid = 40.0;
        s.snap = false;
        for d in defs() {
            reset_key(&mut s, d.key);
        }
        assert_eq!(s, Settings::default());
    }
}
