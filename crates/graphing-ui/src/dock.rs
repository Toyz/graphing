//! Chrome for the dock: tab bars, tabs, the drop zone and the drag preview.
//! The app's dock skin assembles these; behavior (drag, drop, split) comes
//! from the dock engine.

use gpui_kit::component::Icon;
use gpui_kit::{
    App, Div, ElementId, Hsla, InteractiveElement, ParentElement, SharedString, Stateful, Styled, div,
    prelude::FluentBuilder,
};

use crate::kit::Lucide;
use crate::theme::UiExt;
use crate::tokens::*;

/// The strip tabs sit in. `strip` fits a closed bottom dock.
pub fn tab_bar(strip: bool, cx: &App) -> Div {
    let k = cx.ui();
    div()
        .h(if strip { DOCK_STRIP_H } else { DOCK_TAB_H })
        .flex_none()
        .flex()
        .items_end()
        .bg(k.chrome)
        .border_b_1()
        .border_color(k.border)
}

/// One tab. `current` is the displayed tab of its group; `focused` marks the
/// group that holds the active diagram or keyboard focus, which gets the
/// accent line. The caller attaches clicks, drag and drop.
pub fn tab(id: impl Into<ElementId>, group: impl Into<SharedString>, icon: Icon, title: impl Into<SharedString>, current: bool, focused: bool, cx: &App) -> Stateful<Div> {
    let k = cx.ui();
    let group: SharedString = group.into();
    div()
        .id(id)
        .group(group)
        .relative()
        .h_full()
        .max_w(DOCK_TAB_MAX_W)
        .flex_none()
        .pl(GAP_3)
        .pr(GAP_1)
        .flex()
        .items_center()
        .gap(GAP_2)
        .text_size(TEXT_SM)
        .cursor_pointer()
        .border_r_1()
        .border_color(k.border)
        .when(current, |d| d.bg(k.bg).text_color(k.heading))
        .when(!current, |d| d.text_color(k.text_muted).hover(|d| d.bg(k.hover).text_color(k.text)))
        .child(icon.size(ICON_SM).text_color(if current { k.accent } else { k.text_faint }))
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title.into()))
        .when(current && focused, |d| d.child(div().absolute().left_0().right_0().top_0().h(DOCK_INDICATOR).bg(k.accent)))
        // The active tab covers the bar's bottom border so it joins its content.
        .when(current, |d| d.child(div().absolute().left_0().right_0().bottom(-HAIRLINE).h(HAIRLINE).bg(k.bg)))
}

/// The close cross, or a dirty dot that turns into the cross on hover.
pub fn tab_close(id: impl Into<ElementId>, group: impl Into<SharedString>, dirty: bool, current: bool, cx: &App) -> Stateful<Div> {
    let k = cx.ui();
    let group: SharedString = group.into();
    div()
        .id(id)
        .size(HIT_SM)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(ROUND_XS)
        .hover(|d| d.bg(k.active))
        .when(dirty, |d| {
            d.child(div().size(DOT).rounded_full().bg(k.text_muted).group_hover(group.clone(), |d| d.hidden()))
                .child(div().hidden().group_hover(group.clone(), |d| d.flex()).child(Icon::new(Lucide::X).size(ICON_SM).text_color(k.text_muted)))
        })
        .when(!dirty, |d| {
            d.child(
                div()
                    .when(!current, |d| d.invisible().group_hover(group.clone(), |d| d.visible()))
                    .child(Icon::new(Lucide::X).size(ICON_SM).text_color(k.text_muted)),
            )
        })
}

/// Empty bar space after the last tab; a drop target.
pub fn tab_filler(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).h_full().flex_1().min_w(HIT_LG)
}

/// Highlight while a drag hovers a drop target.
pub fn drop_tint(cx: &App) -> Hsla {
    cx.ui().accent_soft
}

/// Where a dragged panel will land.
pub fn drop_zone(cx: &App) -> Div {
    let k = cx.ui();
    div().absolute().bg(k.accent_soft).border_2().border_color(k.accent).rounded(ROUND_SM)
}

/// What follows the pointer while a tab is dragged.
pub fn drag_preview(icon: Icon, title: impl Into<SharedString>, cx: &App) -> Div {
    let k = cx.ui();
    crate::kit::raised(cx)
        .flex()
        .items_center()
        .gap(GAP_2)
        .px(GAP_3)
        .h(DOCK_TAB_H)
        .text_size(TEXT_SM)
        .text_color(k.heading)
        .opacity(0.94)
        .child(icon.size(ICON_SM).text_color(k.accent))
        .child(div().whitespace_nowrap().child(title.into()))
}
