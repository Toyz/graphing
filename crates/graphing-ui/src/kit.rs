//! The component kit. Build new UI out of these instead of styling divs by
//! hand, so every surface has one look and the same pieces can later be
//! offered to plugins.

use std::rc::Rc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName};

/// Any Lucide icon (the full catalog ships with graphing).
pub use gpui_kit::assets::IconName as Lucide;
use gpui_kit::{
    Action, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::theme::UiExt;
use crate::tokens::*;

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type Plain = Rc<dyn Fn(&mut Window, &mut App)>;

/// A Lucide icon by its name as packs write it (`RectangleHorizontal` or
/// `rectangle-horizontal`).
pub fn icon_named(name: &str) -> Icon {
    Icon::empty().path(icon_path(name))
}

/// The asset path of a Lucide icon by name, for painting it directly.
pub fn icon_path(name: &str) -> SharedString {
    let mut kebab = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c.is_uppercase() {
            if prev.is_some() {
                kebab.push('-');
            }
            kebab.extend(c.to_lowercase());
        } else if c.is_ascii_digit() && prev.is_some_and(char::is_alphabetic) && !kebab.ends_with('-') {
            kebab.push('-');
            kebab.push(c);
        } else {
            kebab.push(c);
        }
        prev = Some(c);
    }
    format!("icons/{kebab}.svg").into()
}

fn icon(name: impl Into<Icon>, size: Pixels, color: Hsla) -> Icon {
    Icon::new(name).size(size).text_color(color)
}

// ---- text ----

/// Small uppercase label above a group of controls.
pub fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    let k = cx.ui();
    div().text_size(TEXT_XS).font_weight(FontWeight::SEMIBOLD).text_color(k.text_faint).child(text.into().to_uppercase())
}

pub fn heading(text: impl Into<SharedString>, cx: &App) -> Div {
    let k = cx.ui();
    div().text_size(TEXT_MD).font_weight(FontWeight::SEMIBOLD).text_color(k.heading).child(text.into())
}

pub fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_size(TEXT_SM).text_color(cx.ui().text_muted).child(text.into())
}

// ---- surfaces ----

/// A floating surface: palette, popovers, canvas toolbars.
pub fn raised(cx: &App) -> Div {
    let k = cx.ui();
    div().bg(k.raised).border_1().border_color(k.border_strong).rounded(ROUND_LG).shadow_lg()
}

pub fn divider_v(cx: &App) -> Div {
    div().w(HAIRLINE).h(ICON_LG).bg(cx.ui().border_strong)
}

pub fn divider_h(cx: &App) -> Div {
    div().h(HAIRLINE).w_full().bg(cx.ui().border)
}

// ---- icon button ----

/// Square icon button with a tooltip that shows the action's shortcut.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: Icon,
    tooltip: Option<SharedString>,
    action: Option<Box<dyn Action>>,
    on_click: Option<Handler>,
    active: bool,
    size: Pixels,
    glyph: Pixels,
    danger: bool,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: impl Into<Icon>) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            tooltip: None,
            action: None,
            on_click: None,
            active: false,
            size: HIT_MD,
            glyph: ICON_MD,
            danger: false,
        }
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Dispatch `action` on click; its keybinding shows in the tooltip.
    pub fn action(mut self, action: Box<dyn Action>) -> Self {
        self.action = Some(action);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn small(mut self) -> Self {
        self.size = HIT_SM;
        self.glyph = ICON_SM;
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let k = cx.ui();
        let color = if self.active {
            k.accent
        } else if self.danger {
            k.danger
        } else {
            k.text_muted
        };
        let tip = self.tooltip.clone();
        let tip_action = self.action.as_ref().map(|a| a.boxed_clone());
        let action = self.action;
        let on_click = self.on_click;
        div()
            .id(self.id)
            .size(self.size)
            .flex()
            .items_center()
            .justify_center()
            .rounded(ROUND_SM)
            .cursor_pointer()
            .when(self.active, |d| d.bg(k.accent_soft))
            .when(!self.active, |d| d.hover(|d| d.bg(k.hover)))
            .active(|d| d.bg(k.active))
            // Buttons in the title bar must not start a window drag.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |ev, window, cx| {
                if let Some(f) = &on_click {
                    f(ev, window, cx);
                }
                if let Some(a) = &action {
                    window.dispatch_action(a.boxed_clone(), cx);
                }
            })
            .when_some(tip, move |d, tip| {
                d.tooltip(move |window, cx| {
                    let mut t = Tooltip::new(tip.clone());
                    if let Some(a) = &tip_action {
                        t = t.action(a.as_ref(), None);
                    }
                    t.build(window, cx)
                })
            })
            .child(icon(self.icon, self.glyph, color))
    }
}

/// Dialog button styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Secondary,
    Primary,
    Danger,
}

/// A full-size button with an optional key hint (`Esc`, `Enter`). The caller
/// attaches `on_click`.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, kind: ButtonKind, hint: Option<&'static str>, cx: &App) -> gpui_kit::Stateful<Div> {
    let k = cx.ui();
    let (bg, fg, border, hover) = match kind {
        ButtonKind::Secondary => (k.raised, k.text, k.border_strong, k.hover),
        ButtonKind::Primary => (k.accent, k.on_accent, k.accent, k.accent.opacity(0.88)),
        ButtonKind::Danger => (k.danger, k.on_accent, k.danger, k.danger.opacity(0.88)),
    };
    div()
        .id(id)
        .h(BUTTON_H)
        .px(GAP_4)
        .flex()
        .items_center()
        .gap(GAP_2)
        .rounded(ROUND_MD)
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_size(TEXT_MD)
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label.into())
        .when_some(hint, |d, h| d.child(div().px(GAP_1).rounded(ROUND_XS).bg(fg.opacity(0.14)).text_size(TEXT_XS).text_color(fg.opacity(0.8)).child(h)))
}

/// A modal dialog: tinted icon and title, message, optional body (`extra`),
/// and a footer with a key hint on the left and buttons on the right.
#[allow(clippy::too_many_arguments)]
pub fn dialog_card(
    icon_name: Lucide,
    tone: Hsla,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    extra: Option<AnyElement>,
    hint: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
    cx: &App,
) -> Div {
    let k = cx.ui();
    div()
        .w(DIALOG_W)
        .flex()
        .flex_col()
        .bg(k.raised)
        .border_1()
        .border_color(k.border_strong)
        .rounded(ROUND_LG)
        .shadow_2xl()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(GAP_4)
                .p(GAP_5)
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(GAP_3)
                        .child(div().flex_none().size(HIT_LG + GAP_1).rounded_full().bg(tone.opacity(0.16)).flex().items_center().justify_center().child(icon(icon_name, ICON_LG, tone)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(GAP_1)
                                .pt(GAP_0)
                                .child(div().text_size(TEXT_XL).font_weight(FontWeight::SEMIBOLD).text_color(k.heading).child(title.into()))
                                .child(div().text_size(TEXT_MD).text_color(k.text_muted).child(message.into())),
                        ),
                )
                .children(extra),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(GAP_3)
                .px(GAP_5)
                .py(GAP_3)
                .bg(k.chrome)
                .border_t_1()
                .border_color(k.border)
                .child(div().flex_1().min_w_0().text_size(TEXT_XS).text_color(k.text_faint).child(hint.into()))
                .child(div().flex_none().flex().gap(GAP_2).children(buttons)),
        )
}

/// One option in a choice dialog: icon, title, a line of explanation, and
/// an optional badge ("Recommended"). `highlighted` is the keyboard pick.
pub fn choice_card(
    id: impl Into<ElementId>,
    icon_name: Lucide,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    badge: Option<SharedString>,
    highlighted: bool,
    cx: &App,
) -> gpui_kit::Stateful<Div> {
    let k = cx.ui();
    div()
        .id(id)
        .flex()
        .items_start()
        .gap(GAP_3)
        .p(GAP_3)
        .rounded(ROUND_MD)
        .border_1()
        .border_color(if highlighted { k.accent } else { k.border })
        .bg(if highlighted { k.accent_soft } else { k.bg })
        .cursor_pointer()
        .hover(|s| s.border_color(k.accent))
        .child(
            div()
                .flex_none()
                .size(HIT_MD)
                .rounded(ROUND_SM)
                .bg(if highlighted { k.accent.opacity(0.18) } else { k.hover })
                .flex()
                .items_center()
                .justify_center()
                .child(icon(icon_name, ICON_MD, if highlighted { k.accent } else { k.text_muted })),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(GAP_0)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(GAP_2)
                        .child(div().text_size(TEXT_MD).font_weight(FontWeight::MEDIUM).text_color(k.heading).child(title.into()))
                        .when_some(badge, |d, b| {
                            d.child(div().px(GAP_1 + GAP_0).rounded(ROUND_PILL).bg(k.accent.opacity(0.16)).text_size(TEXT_XS).text_color(k.accent).child(b))
                        }),
                )
                .child(div().text_size(TEXT_SM).text_color(k.text_muted).child(description.into())),
        )
}

/// A count with an icon, as in "3 shapes".
pub fn stat_pill(icon_name: Lucide, text: impl Into<SharedString>, cx: &App) -> Div {
    let k = cx.ui();
    div()
        .flex()
        .items_center()
        .gap(GAP_1)
        .h(HIT_SM + GAP_0)
        .px(GAP_2)
        .rounded(ROUND_PILL)
        .bg(k.bg)
        .border_1()
        .border_color(k.border)
        .text_size(TEXT_SM)
        .text_color(k.text)
        .child(icon(icon_name, ICON_SM, k.text_muted))
        .child(text.into())
}

/// A color chip: the color, or a "no color" slash for `None`. `selected`
/// adds an accent ring around it.
pub fn color_chip(id: impl Into<ElementId>, color: Option<Hsla>, selected: bool, cx: &App) -> gpui_kit::Stateful<Div> {
    let k = cx.ui();
    div()
        .id(id)
        .flex_none()
        .size(SWATCH + GAP_1)
        .p(GAP_0)
        .rounded(ROUND_SM)
        .border_1()
        .border_color(if selected { k.accent } else { gpui_kit::transparent_black() })
        .child(
            div()
                .size_full()
                .rounded(ROUND_XS)
                .border_1()
                .border_color(k.border_strong)
                .map(|d| match color {
                    Some(c) => d.bg(c),
                    None => d.bg(k.bg).flex().items_center().justify_center().child(div().w(SWATCH).h(HAIRLINE).bg(k.danger).opacity(0.8)),
                }),
        )
}

/// A tooltip with a title and an optional muted second line. Give one to
/// anything whose text can be cut off, so the full name is a hover away.
pub fn tip(title: impl Into<SharedString>, detail: Option<SharedString>) -> impl Fn(&mut Window, &mut App) -> gpui_kit::AnyView + 'static {
    let title: SharedString = title.into();
    move |window, cx| {
        let (title, detail) = (title.clone(), detail.clone());
        Tooltip::element(move |_, cx| {
            let k = cx.ui();
            div()
                .flex()
                .flex_col()
                .gap(GAP_0)
                .max_w(MENU_W * 1.5)
                .child(div().text_size(TEXT_SM).text_color(k.text).child(title.clone()))
                .when_some(detail.clone(), |d, s| d.child(div().text_size(TEXT_XS).text_color(k.text_muted).child(s)))
        })
        .build(window, cx)
    }
}

// ---- text button ----

/// Compact text button. `primary` fills with the accent.
#[derive(IntoElement)]
pub struct TextButton {
    id: ElementId,
    label: SharedString,
    icon: Option<Icon>,
    on_click: Option<Handler>,
    primary: bool,
    danger: bool,
    selected: bool,
}

impl TextButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), label: label.into(), icon: None, on_click: None, primary: false, danger: false, selected: false }
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for TextButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let k = cx.ui();
        let (bg, fg, border) = if self.primary {
            (k.accent, k.on_accent, k.accent)
        } else if self.selected {
            (k.accent_soft, k.accent, k.accent.opacity(0.4))
        } else if self.danger {
            (k.raised, k.danger, k.border_strong)
        } else {
            (k.raised, k.text, k.border_strong)
        };
        let on_click = self.on_click;
        div()
            .id(self.id)
            .h(HIT_SM)
            .px(GAP_2)
            .flex()
            .items_center()
            .gap(GAP_1)
            .rounded(ROUND_SM)
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_size(TEXT_SM)
            .text_color(fg)
            .cursor_pointer()
            .hover(|d| d.opacity(0.88))
            .when_some(on_click, |d, f| d.on_click(move |ev, w, cx| f(ev, w, cx)))
            .when_some(self.icon, |d, i| d.child(icon(i, ICON_SM, fg)))
            .child(self.label)
    }
}

// ---- segmented control ----

pub struct Segment {
    pub label: SharedString,
    pub icon: Option<Icon>,
    pub selected: bool,
    pub on_click: Handler,
}

impl Segment {
    pub fn new(label: impl Into<SharedString>, selected: bool, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), icon: None, selected, on_click: Rc::new(f) }
    }
}

/// A row of mutually exclusive choices.
#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    items: Vec<Segment>,
}

impl Segmented {
    pub fn new(id: impl Into<SharedString>, items: Vec<Segment>) -> Self {
        Self { id: id.into(), items }
    }
}

impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let k = cx.ui();
        let id = self.id;
        div()
            .flex()
            .p(GAP_0)
            .gap(GAP_0)
            .rounded(ROUND_SM)
            .bg(k.hover)
            .children(self.items.into_iter().enumerate().map(move |(i, s)| {
                let f = s.on_click.clone();
                div()
                    .id((id.clone(), i))
                    .flex_1()
                    .h(HIT_SM)
                    .px(GAP_2)
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(GAP_1)
                    .rounded(ROUND_XS)
                    .text_size(TEXT_SM)
                    .cursor_pointer()
                    .when(s.selected, |d| d.bg(k.raised).text_color(k.heading).shadow_sm())
                    .when(!s.selected, |d| d.text_color(k.text_muted).hover(|d| d.text_color(k.text)))
                    .on_click(move |ev, w, cx| f(ev, w, cx))
                    .when_some(s.icon, |d, i| d.child(icon(i, ICON_SM, if s.selected { k.heading } else { k.text_muted })))
                    .child(s.label)
            }))
    }
}

// ---- rows, sections, fields ----

/// A list row: icon, label, trailing meta. For outlines, menus, results.
#[derive(IntoElement)]
pub struct Row {
    id: ElementId,
    icon: Option<Icon>,
    label: SharedString,
    meta: Option<SharedString>,
    selected: bool,
    highlighted: bool,
    on_click: Option<Handler>,
    tip: Option<(SharedString, Option<SharedString>)>,
}

impl Row {
    /// Full text (and a second line) on hover, for labels that ellipsize.
    pub fn tip(mut self, title: impl Into<SharedString>, detail: Option<SharedString>) -> Self {
        self.tip = Some((title.into(), detail));
        self
    }

    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), icon: None, label: label.into(), meta: None, selected: false, highlighted: false, on_click: None, tip: None }
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Keyboard focus in a list (palette), drawn like hover.
    pub fn highlighted(mut self, on: bool) -> Self {
        self.highlighted = on;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Row {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let k = cx.ui();
        let fg = if self.selected { k.heading } else { k.text };
        let on_click = self.on_click;
        div()
            .id(self.id)
            .h(ROW_H)
            .px(GAP_2)
            .flex()
            .items_center()
            .gap(GAP_2)
            .rounded(ROUND_SM)
            .text_size(TEXT_MD)
            .text_color(fg)
            .cursor_pointer()
            .when(self.selected, |d| d.bg(k.accent_soft))
            .when(self.highlighted && !self.selected, |d| d.bg(k.hover))
            .when(!self.selected, |d| d.hover(|d| d.bg(k.hover)))
            .when_some(on_click, |d, f| d.on_click(move |ev, w, cx| f(ev, w, cx)))
            .when_some(self.tip, |d, (t, detail)| d.tooltip(tip(t, detail)))
            .when_some(self.icon, |d, i| d.child(icon(i, ICON_MD, if self.selected { k.accent } else { k.text_muted })))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(self.label))
            .when_some(self.meta, |d, m| d.child(div().flex_none().text_size(TEXT_XS).text_color(k.text_faint).child(m)))
    }
}

/// A titled block of controls inside a panel.
pub fn section(title: impl Into<SharedString>, cx: &App) -> Div {
    div().flex().flex_col().gap(GAP_2).px(PANEL_PAD).py(GAP_3).child(caption(title, cx))
}

/// Label above a control.
pub fn field(label: impl Into<SharedString>, control: impl IntoElement, cx: &App) -> Div {
    div().flex().flex_col().gap(GAP_1).child(div().text_size(TEXT_XS).text_color(cx.ui().text_muted).child(label.into())).child(control)
}

/// Label left, value right, on one line.
pub fn prop_row(label: impl Into<SharedString>, value: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(GAP_2)
        .min_h(HIT_SM)
        .child(div().text_size(TEXT_SM).text_color(cx.ui().text_muted).child(label.into()))
        .child(div().text_size(TEXT_SM).text_color(cx.ui().text).child(value))
}

/// Side panel: header with title and actions, scrolling body.
pub fn panel(id: impl Into<ElementId>, title: impl Into<SharedString>, actions: Vec<AnyElement>, body: impl IntoElement, cx: &App) -> impl IntoElement {
    let k = cx.ui();
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(k.chrome)
        .child(
            div()
                .h(PANEL_HEADER_H)
                .flex_none()
                .px(PANEL_PAD)
                .flex()
                .items_center()
                .justify_between()
                .child(heading(title, cx))
                .child(div().flex().items_center().gap(GAP_0).children(actions)),
        )
        .child(div().id(id).flex_1().min_h_0().overflow_y_scroll().child(body))
}

/// A single-line text field at the house height. gpui-component's medium
/// input pads 8px top and bottom, which at our height leaves less than a line
/// and clips descenders; this keeps the padding to fit the line.
pub fn text_input(state: &gpui_kit::Entity<gpui_kit::component::input::InputState>) -> gpui_kit::component::input::Input {
    gpui_kit::component::input::Input::new(state).h(INPUT_H).py(GAP_1)
}

/// Keyboard shortcut chip.
pub fn kbd(keys: impl Into<SharedString>, cx: &App) -> Div {
    let k = cx.ui();
    let keys: SharedString = keys.into();
    div()
        .flex_none()
        .px(GAP_1)
        .rounded(ROUND_XS)
        .border_1()
        .border_color(k.border_strong)
        .bg(k.chrome)
        .text_size(TEXT_XS)
        .text_color(k.text_muted)
        .child(keys)
}

/// Empty state: icon, title, hint.
pub fn empty_state(icon_name: impl Into<Icon>, title: impl Into<SharedString>, hint: impl Into<SharedString>, cx: &App) -> Div {
    let k = cx.ui();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(GAP_2)
        .p(GAP_5)
        .child(icon(icon_name, ICON_XL, k.text_faint))
        .child(div().text_size(TEXT_MD).text_color(k.text_muted).child(title.into()))
        .child(div().text_size(TEXT_SM).text_color(k.text_faint).text_center().child(hint.into()))
}

// ---- tabs ----

/// Document tab for the title bar strip.
#[derive(IntoElement)]
pub struct TabItem {
    id: SharedString,
    title: SharedString,
    active: bool,
    dirty: bool,
    on_click: Option<Handler>,
    on_close: Option<Handler>,
    on_middle: Option<Plain>,
}

impl TabItem {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self { id: id.into(), title: title.into(), active: false, dirty: false, on_click: None, on_close: None, on_middle: None }
    }

    pub fn active(mut self, on: bool) -> Self {
        self.active = on;
        self
    }

    pub fn dirty(mut self, on: bool) -> Self {
        self.dirty = on;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    pub fn on_close(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }

    pub fn on_middle_click(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_middle = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for TabItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let k = cx.ui();
        let group: SharedString = format!("tab-{}", self.id).into();
        let on_click = self.on_click;
        let on_close = self.on_close;
        let on_middle = self.on_middle;
        let close_id: ElementId = (self.id.clone(), 1usize).into();
        let id: ElementId = self.id.into();
        div()
            .id(id)
            .group(group.clone())
            .h(TAB_H)
            .max_w(TAB_MAX_W)
            .pl(GAP_3)
            .pr(GAP_1)
            .flex()
            .items_center()
            .gap(GAP_1)
            .rounded(ROUND_SM)
            .text_size(TEXT_MD)
            .cursor_pointer()
            .when(self.active, |d| d.bg(k.bg).text_color(k.heading).shadow_sm())
            .when(!self.active, |d| d.text_color(k.text_muted).hover(|d| d.bg(k.hover).text_color(k.text)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Middle, move |_, w, cx| {
                cx.stop_propagation();
                if let Some(f) = &on_middle {
                    f(w, cx);
                }
            })
            .when_some(on_click, |d, f| d.on_click(move |ev, w, cx| f(ev, w, cx)))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(self.title))
            .child(
                div()
                    .id(close_id)
                    .size(HIT_SM)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(ROUND_XS)
                    .hover(|d| d.bg(k.active))
                    .when_some(on_close, |d, f| {
                        d.on_click(move |ev, w, cx| {
                            cx.stop_propagation();
                            f(ev, w, cx)
                        })
                    })
                    // Dirty dot, swapped for the close cross on hover.
                    .when(self.dirty, |d| {
                        d.child(div().size(DOT).rounded_full().bg(k.text_muted).group_hover(group.clone(), |d| d.invisible()))
                    })
                    .when(!self.dirty, |d| {
                        d.child(
                            div()
                                .when(!self.active, |d| d.invisible())
                                .group_hover(group.clone(), |d| d.visible())
                                .child(icon(IconName::Close, ICON_SM, k.text_muted)),
                        )
                    }),
            )
    }
}

// ---- status bar ----

/// How a notice reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeTone {
    Info,
    /// Work going on (an export); shows its progress.
    Working,
    Success,
    Error,
}

/// A notice: icon by tone, a title, an optional second line, a progress
/// bar while work goes on, buttons for what to do next, and a close
/// button. `actions` are already-built buttons (`TextButton`s).
#[allow(clippy::too_many_arguments)]
pub fn notice(id: impl Into<ElementId>, tone: NoticeTone, title: impl Into<SharedString>, detail: Option<SharedString>, progress: Option<f32>, actions: Vec<AnyElement>, on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, cx: &App) -> gpui_kit::Stateful<Div> {
    let k = cx.ui();
    let (glyph, color) = match tone {
        NoticeTone::Info => (Lucide::Info, k.accent),
        NoticeTone::Working => (Lucide::LoaderCircle, k.accent),
        NoticeTone::Success => (Lucide::CircleCheck, k.success),
        NoticeTone::Error => (Lucide::CircleX, k.danger),
    };
    let id = id.into();
    raised(cx)
        .id(id.clone())
        .w(NOTICE_W)
        .flex()
        .flex_col()
        .gap(GAP_2)
        .p(GAP_3)
        .child(
            div()
                .flex()
                .items_start()
                .gap(GAP_2)
                .child(div().flex_none().pt(GAP_0).child(icon(glyph, ICON_SM, color)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(GAP_0)
                        .child(div().text_size(TEXT_SM).text_color(k.heading).child(title.into()))
                        .when_some(detail, |d, t| d.child(div().text_size(TEXT_XS).text_color(k.text_muted).overflow_hidden().text_ellipsis().whitespace_nowrap().child(t))),
                )
                .child(IconButton::new((id.clone(), "close"), Lucide::X).small().tooltip("Dismiss").on_click(on_close)),
        )
        .when_some(progress, |d, f| {
            d.child(
                div()
                    .h(PROGRESS_H)
                    .w_full()
                    .rounded_full()
                    .bg(k.hover)
                    .child(div().h_full().rounded_full().bg(k.accent).w(gpui_kit::relative(f.clamp(0.02, 1.0)))),
            )
        })
        .when(!actions.is_empty(), |d| d.child(div().flex().justify_end().gap(GAP_2).children(actions)))
}

/// A small count pill, pinned to the corner of the element it decorates.
pub fn count_badge(n: usize, color: Hsla, cx: &App) -> Div {
    let k = cx.ui();
    div()
        .absolute()
        .top(-GAP_0)
        .right(-GAP_0)
        .min_w(ICON_MD)
        .h(ICON_MD)
        .px(GAP_0)
        .rounded(ROUND_PILL)
        .bg(color)
        .flex()
        .items_center()
        .justify_center()
        .text_size(TEXT_XS)
        .text_color(k.on_accent)
        .child(if n > 99 { "99+".to_string() } else { n.to_string() })
}

pub fn status_item(content: impl IntoElement, cx: &App) -> Div {
    div().flex().items_center().gap(GAP_1).text_size(TEXT_XS).text_color(cx.ui().text_muted).child(content)
}
