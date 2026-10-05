//! Menus and selects share one popover: rows with an icon column, a label,
//! an optional detail and shortcut chips, check marks, captions and
//! separators. Hosts own the open state; these pieces only draw.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::Icon;
use gpui_kit::{
    Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Pixels, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, canvas, deferred, div, relative,
    ease_out_quint, prelude::FluentBuilder,
};

use crate::kit::{self, Lucide};
use crate::theme::UiExt;
use crate::tokens::*;

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
/// Opens a submenu.
type OnOpen = Rc<dyn Fn(&mut Window, &mut App)>;

pub enum MenuRow {
    Item {
        icon: Option<Box<Icon>>,
        label: SharedString,
        /// Muted text after the label (`ibd`, a path, a hint).
        detail: Option<SharedString>,
        /// Second line under the label (selects explain each choice).
        description: Option<SharedString>,
        /// Shortcut as written in keymaps (`ctrl-shift-p`); empty for none.
        keys: SharedString,
        checked: bool,
        on_click: Handler,
    },
    Caption(SharedString),
    Separator,
    /// A row that opens more rows beside it.
    Sub {
        icon: Option<Box<Icon>>,
        label: SharedString,
        rows: Vec<MenuRow>,
        open: bool,
        /// Called when the pointer reaches the row (or it is clicked).
        on_open: OnOpen,
        /// Where the row and its open panel are, for the menu's pointer
        /// tracking.
        row_zone: Rc<Cell<Option<Bounds<Pixels>>>>,
        panel_zone: Rc<Cell<Option<Bounds<Pixels>>>>,
    },
}

impl MenuRow {
    pub fn item(label: impl Into<SharedString>, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        MenuRow::Item {
            icon: None,
            label: label.into(),
            detail: None,
            description: None,
            keys: SharedString::default(),
            checked: false,
            on_click: Rc::new(f),
        }
    }

    /// A submenu: `rows` open beside this row while `open`.
    pub fn sub(
        label: impl Into<SharedString>,
        rows: Vec<MenuRow>,
        open: bool,
        on_open: impl Fn(&mut Window, &mut App) + 'static,
        row_zone: Rc<Cell<Option<Bounds<Pixels>>>>,
        panel_zone: Rc<Cell<Option<Bounds<Pixels>>>>,
    ) -> Self {
        MenuRow::Sub { icon: None, label: label.into(), rows, open, on_open: Rc::new(on_open), row_zone, panel_zone }
    }

    pub fn icon(mut self, i: impl Into<Icon>) -> Self {
        if let MenuRow::Item { icon, .. } | MenuRow::Sub { icon, .. } = &mut self {
            *icon = Some(Box::new(i.into()));
        }
        self
    }

    pub fn detail(mut self, d: impl Into<SharedString>) -> Self {
        if let MenuRow::Item { detail, .. } = &mut self {
            *detail = Some(d.into());
        }
        self
    }

    pub fn description(mut self, d: impl Into<SharedString>) -> Self {
        if let MenuRow::Item { description, .. } = &mut self {
            *description = Some(d.into());
        }
        self
    }

    pub fn keys(mut self, k: impl Into<SharedString>) -> Self {
        if let MenuRow::Item { keys, .. } = &mut self {
            *keys = k.into();
        }
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        if let MenuRow::Item { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
}

/// `ctrl-shift-p` -> ["Ctrl", "Shift", "P"]; chords separated by spaces.
pub fn key_chips(keys: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chord in keys.split_whitespace() {
        let mut parts: Vec<&str> = chord.split('-').collect();
        // `ctrl--` is ctrl plus the minus key.
        if chord.ends_with("--") {
            parts.retain(|p| !p.is_empty());
            parts.push("-");
        }
        for p in parts.into_iter().filter(|p| !p.is_empty()) {
            out.push(match p {
                "ctrl" => "Ctrl".into(),
                "shift" => "Shift".into(),
                "alt" if cfg!(target_os = "macos") => "Option".into(),
                "alt" => "Alt".into(),
                "secondary" if cfg!(target_os = "macos") => "Cmd".into(),
                "secondary" => "Ctrl".into(),
                "cmd" | "super" | "platform" | "win" if cfg!(target_os = "macos") => "Cmd".into(),
                "cmd" | "super" | "platform" | "win" if cfg!(target_os = "windows") => "Win".into(),
                "cmd" | "super" | "platform" | "win" => "Super".into(),
                "enter" => "Enter".into(),
                "escape" => "Esc".into(),
                "delete" => "Del".into(),
                "backspace" => "Backspace".into(),
                "pageup" => "PgUp".into(),
                "pagedown" => "PgDn".into(),
                "tab" => "Tab".into(),
                "space" => "Space".into(),
                "left" => "\u{2190}".into(),
                "right" => "\u{2192}".into(),
                "up" => "\u{2191}".into(),
                "down" => "\u{2193}".into(),
                other if other.chars().count() == 1 => other.to_uppercase(),
                other => {
                    let mut c = other.chars();
                    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
                }
            });
        }
    }
    out
}

/// Shortcut chips, compact for menus.
pub fn keys(keys: &str, cx: &App) -> Div {
    div().flex_none().flex().gap(GAP_0).children(key_chips(keys).into_iter().map(|k| kit::kbd(k, cx)))
}

/// The popover surface with its rows: icon tiles, an accent pill on the
/// hovered and the checked row, a lit top edge. Graphing's popover signature,
/// shared with the palette.
pub fn menu_surface(id: impl Into<ElementId>, rows: Vec<MenuRow>, cx: &App) -> Stateful<Div> {
    menu_surface_with(id, None, rows, None, cx)
}

/// A popover with an optional header (a search field) above rows that
/// scroll past `max_h`.
pub fn menu_surface_with(id: impl Into<ElementId>, header: Option<AnyElement>, rows: Vec<MenuRow>, max_h: Option<Pixels>, cx: &App) -> Stateful<Div> {
    let k = cx.ui();
    let id: ElementId = id.into();
    let id_text: SharedString = format!("{id}").into();
    let rows = rows.into_iter().enumerate().map(|(j, row)| match row {
        MenuRow::Separator => div().flex_none().my(GAP_1).mx(GAP_3).child(kit::divider_h(cx)).into_any_element(),
        MenuRow::Sub { icon, label, rows, open, on_open, row_zone, panel_zone } => {
            let group: SharedString = format!("{id_text}-row-{j}").into();
            let (hover_open, click_open) = (on_open.clone(), on_open);
            let panel = open.then(|| {
                let surface = menu_surface(SharedString::from(format!("{id_text}-sub-{j}")), rows, cx).debug_selector(|| "menu-sub".into()).child(zone(panel_zone));
                // Beside the row, over the rows below it.
                div().absolute().left(relative(1.0)).top(-GAP_1).pl(GAP_1).child(deferred(animate(surface, SharedString::from(format!("{id_text}-sub-anim-{j}")))).with_priority(3))
            });
            div()
                .id(SharedString::from(format!("{id_text}-{j}")))
                .group(group.clone())
                .relative()
                .flex_none()
                .h(MENU_ROW_H)
                .mx(GAP_1)
                .pl(GAP_2)
                .pr(GAP_2)
                .flex()
                .items_center()
                .gap(GAP_2)
                .rounded(ROUND_MD)
                .text_size(TEXT_MD)
                .text_color(if open { k.heading } else { k.text })
                .cursor_pointer()
                .when(open, |d| d.bg(k.accent_soft))
                .hover(|d| d.bg(k.accent_soft).text_color(k.heading))
                .debug_selector({
                    let label = label.clone();
                    move || format!("menu-sub-row-{label}")
                })
                .on_hover(move |h, w, cx| {
                    if *h {
                        hover_open(w, cx)
                    }
                })
                .on_click(move |_, w, cx| click_open(w, cx))
                .child(zone(row_zone))
                .child(
                    div()
                        .flex_none()
                        .size(HIT_SM)
                        .rounded(ROUND_SM)
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(if open { k.raised } else { k.hover })
                        .group_hover(group.clone(), |d| d.bg(k.raised))
                        .map(|d| match icon {
                            Some(i) => d.child(i.size(ICON_SM).text_color(if open { k.accent } else { k.text_muted })),
                            None => d,
                        }),
                )
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
                .child(Icon::new(Lucide::ChevronRight).size(ICON_SM).text_color(k.text_faint))
                .children(panel)
                .into_any_element()
        }
        MenuRow::Caption(text) => div().flex_none().px(GAP_3).pt(GAP_3).pb(GAP_1).child(kit::caption(text, cx)).into_any_element(),
        MenuRow::Item { icon, label, detail, description, keys: shortcut, checked, on_click } => {
            let group: SharedString = format!("{id_text}-row-{j}").into();
            let tall = description.is_some();
            div()
                .id(SharedString::from(format!("{id_text}-{j}")))
                .group(group.clone())
                .relative()
                // Never squeezed by a scrolling list: rows would overlap.
                .flex_none()
                .when(tall, |d| d.min_h(MENU_ROW_TALL_H).py(GAP_2))
                .when(!tall, |d| d.h(MENU_ROW_H))
                .mx(GAP_1)
                .pl(GAP_2)
                .pr(GAP_2)
                .flex()
                .items_center()
                .gap(GAP_2)
                .rounded(ROUND_MD)
                .text_size(TEXT_MD)
                .text_color(if checked { k.heading } else { k.text })
                .cursor_pointer()
                .when(checked, |d| d.bg(k.accent_soft.opacity(0.6)))
                .hover(|d| d.bg(k.accent_soft).text_color(k.heading))
                .on_click(move |ev, w, cx| on_click(ev, w, cx))
                // The signature pill: on the checked row, and on hover.
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(GAP_2)
                        .bottom(GAP_2)
                        .w(FOCUS_RING)
                        .rounded(ROUND_PILL)
                        .bg(k.accent)
                        .when(!checked, |d| d.invisible().group_hover(group.clone(), |d| d.visible())),
                )
                .child(
                    div()
                        .flex_none()
                        .size(if tall { HIT_MD } else { HIT_SM })
                        .rounded(ROUND_SM)
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(if checked { k.raised } else { k.hover })
                        .group_hover(group.clone(), |d| d.bg(k.raised))
                        .map(|d| match icon {
                            Some(i) => d.child(i.size(ICON_SM).text_color(if checked { k.accent } else { k.text_muted })),
                            None => d,
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(GAP_0)
                        .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(if tall { FontWeight::MEDIUM } else { FontWeight::NORMAL }).child(label))
                        .when_some(description, |d, t| {
                            // Descriptions wrap instead of cutting off mid-word.
                            d.child(div().text_size(TEXT_SM).line_height(TEXT_SM * 1.35).text_color(k.text_muted).child(t))
                        }),
                )
                // Detail as a small mono chip (`ibd`, `sysml.block`).
                .when_some(detail, |d, t| {
                    d.child(
                        div()
                            .flex_none()
                            .px(GAP_1 + GAP_0)
                            .rounded(ROUND_XS)
                            .bg(k.hover)
                            .border_1()
                            .border_color(k.border)
                            .font_family(cx.mono())
                            .text_size(TEXT_XS)
                            .text_color(k.text_muted)
                            .group_hover(group.clone(), |d| d.bg(k.raised))
                            .child(t),
                    )
                })
                .when(!shortcut.is_empty(), |d| d.child(keys(&shortcut, cx)))
                .when(checked, |d| d.child(Icon::new(Lucide::Check).size(ICON_SM).text_color(k.accent)))
                .into_any_element()
        }
    });
    div()
        .id(id)
        .min_w(MENU_W)
        .p(GAP_1)
        .flex()
        .flex_col()
        .bg(k.raised)
        .border_1()
        .border_color(k.border_strong)
        .rounded(ROUND_LG)
        .shadow_2xl()
        .occlude()
        .relative()
        // Lit top edge: a hairline lighter than the surface.
        .child(div().absolute().top_0().left(GAP_3).right(GAP_3).h(HAIRLINE).bg(k.heading.opacity(if k.dark { 0.08 } else { 0.0 })))
        .when_some(header, |d, h| d.child(div().p(GAP_1).pb(GAP_2).child(h)).child(div().mx(GAP_1).mb(GAP_1).child(kit::divider_h(cx))))
        .child({
            let list = div().id(SharedString::from(format!("{id_text}-list"))).flex().flex_col().children(rows.collect::<Vec<AnyElement>>());
            match max_h {
                Some(h) => list.max_h(h).overflow_y_scroll(),
                None => list,
            }
        })
}

/// Fade and slide a popover in as it opens.
pub fn animate(el: impl IntoElement + 'static, id: impl Into<ElementId>) -> AnyElement {
    div()
        .child(el)
        .with_animation(id, Animation::new(OPEN_ANIM).with_easing(ease_out_quint()), |d, t| d.opacity(t).mt(GAP_1 * (1.0 - t)))
        .into_any_element()
}

/// A select's closed state: looks like an input, opens a menu.
pub fn select_trigger(
    id: impl Into<ElementId>,
    icon: Option<Icon>,
    label: impl Into<SharedString>,
    open: bool,
    f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let k = cx.ui();
    div()
        .id(id)
        .h(INPUT_H)
        .w_full()
        .px(GAP_3)
        .flex()
        .items_center()
        .gap(GAP_2)
        .rounded(ROUND_SM)
        .border_1()
        .border_color(if open { k.accent } else { k.border_strong })
        .bg(k.bg)
        .text_size(TEXT_MD)
        .text_color(k.text)
        .cursor_pointer()
        .hover(|d| d.border_color(k.text_faint))
        .on_click(f)
        .when_some(icon, |d, i| d.child(i.size(ICON_SM).text_color(k.text_muted)))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label.into()))
        .child(Icon::new(if open { Lucide::ChevronUp } else { Lucide::ChevronDown }).size(ICON_SM).text_color(k.text_faint))
}

/// Records an element's bounds each frame, for hit tests that must not
/// depend on hover events (a menu staying open while the pointer is on it).
pub fn zone(slot: Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, (), _, _| {}).absolute().top_0().left_0().size_full()
}
