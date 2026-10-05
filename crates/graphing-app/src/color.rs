//! The color field: a compact trigger (chip, hex, chevron) that opens a
//! picker with a Default option, the palette, colors already used in the
//! diagram, and a custom picker (saturation/brightness square, hue bar, hex
//! input). A drag in the custom picker previews live and applies on release,
//! so it is one undo step.

use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, AppContext, Bounds, Context, DispatchPhase, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, hsla, linear_color_stop,
    linear_gradient, prelude::FluentBuilder, relative,
};
use graphing_model::{Diagram, Op, Value};
use graphing_ui::UiExt;
use graphing_ui::kit::{self, Lucide};
use graphing_ui::tokens::*;

use crate::inspector::OpenSelect;
use crate::workspace::Workspace;

/// Select id per prop, so each field opens its own picker.
fn picker_id(key: &str) -> &'static str {
    match key {
        "fill" => "color-fill",
        "stroke" => "color-stroke",
        _ => "color-text",
    }
}

/// `#4dabf7`, `4dabf7` or `#fff` as a color value.
pub(crate) fn parse_hex(text: &str) -> Option<u32> {
    let t = text.trim().trim_start_matches('#');
    let full = match t.len() {
        3 => t.chars().flat_map(|c| [c, c]).collect(),
        6 => t.to_string(),
        _ => return None,
    };
    u32::from_str_radix(&full, 16).ok()
}

/// Colors already set on anything in the diagram, first use first.
fn used_colors(d: &Diagram) -> Vec<u32> {
    let mut out = Vec::new();
    let props = d.nodes.iter().map(|n| &n.props).chain(d.groups.iter().map(|g| &g.props)).chain(d.edges.iter().map(|e| &e.props));
    for p in props {
        for (k, v) in p {
            if matches!(k.as_str(), "fill" | "stroke" | "color")
                && let Value::Color(c) = v
                && let Some(c) = parse_hex(c)
                && !out.contains(&c)
            {
                out.push(c);
            }
        }
    }
    out.truncate(10);
    out
}

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

/// Hue in degrees, saturation and value in 0..1, to `0xrrggbb`.
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

/// `0xrrggbb` to (hue degrees, saturation, value).
pub(crate) fn rgb_to_hsv(c: u32) -> (f32, f32, f32) {
    let (r, g, b) = (((c >> 16) & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Part {
    Square,
    Hue,
}

/// The custom picker's state while it is open.
pub(crate) struct ColorEdit {
    h: f32,
    s: f32,
    v: f32,
    drag: Option<Part>,
    target: String,
    key: &'static str,
    square: Rc<Cell<Bounds<Pixels>>>,
    hue: Rc<Cell<Bounds<Pixels>>>,
}

impl ColorEdit {
    fn rgb(&self) -> u32 {
        hsv_to_rgb(self.h, self.s, self.v)
    }
}

fn frac(v: Pixels, start: Pixels, len: Pixels) -> f32 {
    if f32::from(len) <= 0.0 { 0.0 } else { (f32::from(v - start) / f32::from(len)).clamp(0.0, 1.0) }
}

impl Workspace {
    /// The color of `key` on `id`, as stored.
    fn color_of(d: &Diagram, id: &str, key: &str) -> Option<u32> {
        d.node(id)
            .and_then(|n| d.node_prop(n, key))
            .or_else(|| d.edge(id).and_then(|e| d.edge_prop(e, key)))
            .or_else(|| d.group(id).and_then(|g| graphing_model::find_prop(&g.props, key)))
            .and_then(graphing_scene::color)
    }

    fn set_color(&mut self, id: &str, key: &str, color: Option<u32>, cx: &mut Context<Self>) {
        self.apply_color(id, key, color, cx);
        self.select = None;
        self.color_edit = None;
        cx.notify();
    }

    fn apply_color(&mut self, id: &str, key: &str, color: Option<u32>, cx: &mut Context<Self>) {
        let value = color.map(|c| Value::Color(hex(c)));
        self.edit(Op::SetProp { id: id.to_string(), key: key.into(), value }, cx);
    }

    /// Pointer at `p` while dragging `part`: move the picked color.
    fn drag_color(&mut self, part: Part, p: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &mut self.color_edit else { return };
        match part {
            Part::Square => {
                let b = e.square.get();
                e.s = frac(p.x, b.origin.x, b.size.width);
                e.v = 1.0 - frac(p.y, b.origin.y, b.size.height);
            }
            Part::Hue => {
                let b = e.hue.get();
                e.h = frac(p.x, b.origin.x, b.size.width) * 360.0;
            }
        }
        let text = hex(e.rgb()).trim_start_matches('#').to_string();
        if let Some(sel) = &self.select {
            sel.query.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        cx.notify();
    }

    /// Release: the dragged color becomes the value (one undo step).
    fn end_color_drag(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &mut self.color_edit else { return };
        if e.drag.take().is_none() {
            return;
        }
        let (target, key, rgb) = (e.target.clone(), e.key, e.rgb());
        self.apply_color(&target, key, Some(rgb), cx);
        cx.notify();
    }

    pub(crate) fn toggle_color(&mut self, id: &str, key: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let pid = picker_id(key);
        if self.select.as_ref().is_some_and(|s| s.id.as_ref() == pid) {
            self.select = None;
            cx.notify();
            return;
        }
        let start = Self::color_of(self.view().read(cx).doc().diagram(), id, key);
        let (h, s, v) = rgb_to_hsv(start.unwrap_or(0x4dabf7));
        self.color_edit = Some(ColorEdit { h, s, v, drag: None, target: id.to_string(), key, square: Rc::default(), hue: Rc::default() });
        let input = cx.new(|cx| {
            let mut st = InputState::new(window, cx).placeholder("Hex, like 4dabf7");
            if let Some(c) = start {
                st.set_value(hex(c).trim_start_matches('#').to_string(), window, cx);
            }
            st
        });
        let target = id.to_string();
        let sub = cx.subscribe_in(&input, window, move |ws: &mut Self, input, ev: &InputEvent, _, cx| match ev {
            InputEvent::PressEnter { .. } => {
                if let Some(c) = parse_hex(&input.read(cx).value()) {
                    ws.set_color(&target, key, Some(c), cx);
                }
            }
            InputEvent::Change => {
                // Typing a full hex moves the square and hue bar to it.
                if let Some(c) = parse_hex(&input.read(cx).value())
                    && let Some(e) = &mut ws.color_edit
                    && e.drag.is_none()
                {
                    (e.h, e.s, e.v) = rgb_to_hsv(c);
                }
                cx.notify()
            }
            _ => {}
        });
        self.select = Some(OpenSelect { id: pid.into(), query: input, _sub: sub });
        cx.notify();
    }

    /// The field: chip, hex (or "Default"), chevron; the picker below it
    /// while open.
    pub(crate) fn color_field(&mut self, id: &str, key: &'static str, d: &Diagram, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let current = Self::color_of(d, id, key);
        let pid = picker_id(key);
        let open = self.select.as_ref().is_some_and(|s| s.id.as_ref() == pid);
        let target = id.to_string();
        let trigger = div()
            .id(SharedString::from(format!("{pid}-trigger")))
            .h(INPUT_H)
            .w_full()
            .px(GAP_2)
            .flex()
            .items_center()
            .gap(GAP_2)
            .rounded(ROUND_SM)
            .border_1()
            .border_color(if open { k.accent } else { k.border_strong })
            .bg(k.bg)
            .cursor_pointer()
            .hover(|s| s.border_color(k.text_faint))
            .on_click(cx.listener(move |ws, _, window, cx| ws.toggle_color(&target, key, window, cx)))
            .child(kit::color_chip(SharedString::from(format!("{pid}-chip")), current.map(swatch), false, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(TEXT_SM)
                    .when_some(current, |d, c| d.font_family("monospace").text_color(k.text).child(hex(c)))
                    .when(current.is_none(), |d| d.text_color(k.text_muted).child("Default")),
            )
            .child(Icon::new(if open { Lucide::ChevronUp } else { Lucide::ChevronDown }).size(ICON_SM).text_color(k.text_faint));

        let popover = open.then(|| self.color_picker(id, key, current, d, cx));
        div()
            .relative()
            .w_full()
            .child(trigger)
            .when_some(popover, |el, p| {
                el.child(div().absolute().top(INPUT_H + GAP_1).right_0().child(gpui_kit::deferred(p).with_priority(3)))
            })
            .into_any_element()
    }

    fn color_picker(&mut self, id: &str, key: &'static str, current: Option<u32>, d: &Diagram, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let pid = picker_id(key);
        let cell = |ws: &Workspace, cx: &mut Context<Workspace>, name: String, c: Option<u32>, tip: String| {
            let _ = ws;
            let target = id.to_string();
            kit::color_chip(SharedString::from(name), c.map(swatch), c == current, cx)
                .cursor_pointer()
                .hover(|s| s.border_color(k.text_muted))
                .tooltip(kit::tip(tip, c.map(|c| hex(c).into())))
                .on_click(cx.listener(move |ws, _, _, cx| ws.set_color(&target, key, c, cx)))
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        // Default: clear the prop, the theme (or the shape's pack) decides.
        let target = id.to_string();
        rows.push(
            div()
                .id(SharedString::from(format!("{pid}-default")))
                .flex()
                .items_center()
                .gap(GAP_2)
                .px(GAP_1)
                .py(GAP_0)
                .rounded(ROUND_SM)
                .cursor_pointer()
                .when(current.is_none(), |d| d.bg(k.accent_soft))
                .hover(|s| s.bg(k.hover))
                .on_click(cx.listener(move |ws, _, _, cx| ws.set_color(&target, key, None, cx)))
                .child(kit::color_chip(SharedString::from(format!("{pid}-none")), None, false, cx))
                .child(div().text_size(TEXT_SM).text_color(k.text).child("Default"))
                .child(div().text_size(TEXT_XS).text_color(k.text_faint).child("from the theme"))
                .into_any_element(),
        );
        rows.push(div().pt(GAP_1).child(kit::caption("Palette", cx)).into_any_element());
        for (r, row) in PALETTE.iter().enumerate() {
            let cells: Vec<_> = row
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let name = if r == 0 { format!("Gray {i}") } else { format!("{} {}", PALETTE_HUES[i], ["", "light", "mid", "strong"][r]) };
                    cell(self, cx, format!("{pid}-{r}-{i}"), Some(*c), name)
                })
                .collect();
            rows.push(div().flex().gap(GAP_0).children(cells).into_any_element());
        }
        let used = used_colors(d);
        if !used.is_empty() {
            rows.push(div().pt(GAP_1).child(kit::caption("In this diagram", cx)).into_any_element());
            let cells: Vec<_> = used.iter().enumerate().map(|(i, c)| cell(self, cx, format!("{pid}-used-{i}"), Some(*c), "Used here".into())).collect();
            rows.push(div().flex().gap(GAP_0).children(cells).into_any_element());
        }
        // Custom: square, hue bar, preview and hex (Enter applies).
        if let (Some(sel), Some(e)) = (&self.select, &self.color_edit) {
            let input = kit::text_input(&sel.query).prefix(div().text_size(TEXT_SM).text_color(k.text_faint).child("#"));
            let preview = e.rgb();
            rows.push(div().pt(GAP_1).child(kit::caption("Custom", cx)).into_any_element());
            rows.push(self.color_square(pid, cx));
            rows.push(self.hue_bar(pid, cx));
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP_2)
                    .pt(GAP_1)
                    .child(kit::color_chip(SharedString::from(format!("{pid}-typed")), Some(swatch(preview)), false, cx))
                    .child(div().flex_1().child(input))
                    .into_any_element(),
            );
        }
        // While dragging, follow the pointer anywhere in the window.
        let dragging = self.color_edit.as_ref().and_then(|e| e.drag);
        let ws = cx.weak_entity();
        let tracker = dragging.map(|part| {
            canvas(|_, _, _| {}, move |_, (), window, _| {
                let (ws_move, ws_up) = (ws.clone(), ws.clone());
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        ws_move.update(cx, |ws, cx| ws.drag_color(part, ev.position, window, cx)).ok();
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        ws_up.update(cx, |ws, cx| ws.end_color_drag(cx)).ok();
                    }
                });
            })
        });
        let surface = kit::raised(cx)
            .id(SharedString::from(format!("{pid}-picker")))
            // Clicks land on the picker only, never on fields behind it.
            .occlude()
            .children(tracker)
            .w(COLOR_PICKER_W)
            .p(GAP_2)
            .flex()
            .flex_col()
            .gap(GAP_1)
            .children(rows)
            .on_mouse_down_out(cx.listener(|ws, _, _, cx| {
                ws.select = None;
                cx.notify();
            }));
        graphing_ui::menu::animate(surface, SharedString::from(format!("{pid}-anim"))).into_any_element()
    }

    /// Saturation left to right, brightness bottom to top, at the current hue.
    fn color_square(&mut self, pid: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(e) = &self.color_edit else { return div().into_any_element() };
        let (h, s, v) = (e.h, e.s, e.v);
        let bounds = e.square.clone();
        let hue = swatch(hsv_to_rgb(h, 1.0, 1.0));
        div()
            .id(SharedString::from(format!("{pid}-square")))
            .relative()
            .w_full()
            .h(COLOR_SQUARE_H)
            .rounded(ROUND_SM)
            .overflow_hidden()
            .cursor_crosshair()
            .bg(hue)
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
            .child(div().absolute().size_full().bg(linear_gradient(90.0, linear_color_stop(hsla(0.0, 0.0, 1.0, 1.0), 0.0), linear_color_stop(hsla(0.0, 0.0, 1.0, 0.0), 1.0))))
            .child(div().absolute().size_full().bg(linear_gradient(180.0, linear_color_stop(hsla(0.0, 0.0, 0.0, 0.0), 0.0), linear_color_stop(hsla(0.0, 0.0, 0.0, 1.0), 1.0))))
            .child(knob(relative(s), relative(1.0 - v), swatch(hsv_to_rgb(h, s, v))))
            .on_mouse_down(MouseButton::Left, cx.listener(|ws, ev: &MouseDownEvent, window, cx| {
                if let Some(e) = &mut ws.color_edit {
                    e.drag = Some(Part::Square);
                }
                ws.drag_color(Part::Square, ev.position, window, cx);
            }))
            .into_any_element()
    }

    /// The hue spectrum in six two-stop segments.
    fn hue_bar(&mut self, pid: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(e) = &self.color_edit else { return div().into_any_element() };
        let h = e.h;
        let bounds = e.hue.clone();
        let segs = (0..6).map(|i| {
            let (a, b) = (swatch(hsv_to_rgb(i as f32 * 60.0, 1.0, 1.0)), swatch(hsv_to_rgb((i + 1) as f32 * 60.0, 1.0, 1.0)));
            div().flex_1().h_full().bg(linear_gradient(90.0, linear_color_stop(a, 0.0), linear_color_stop(b, 1.0)))
        });
        div()
            .id(SharedString::from(format!("{pid}-hue")))
            .relative()
            .w_full()
            .h(COLOR_HUE_H)
            .mt(GAP_1)
            .cursor_pointer()
            .child(div().size_full().rounded(ROUND_PILL).overflow_hidden().flex().children(segs))
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
            .child(knob(relative(h / 360.0), relative(0.5), swatch(hsv_to_rgb(h, 1.0, 1.0))))
            .on_mouse_down(MouseButton::Left, cx.listener(|ws, ev: &MouseDownEvent, window, cx| {
                if let Some(e) = &mut ws.color_edit {
                    e.drag = Some(Part::Hue);
                }
                ws.drag_color(Part::Hue, ev.position, window, cx);
            }))
            .into_any_element()
    }
}

/// The ring marking the picked spot, centered on (`x`, `y`).
fn knob(x: gpui_kit::DefiniteLength, y: gpui_kit::DefiniteLength, fill: gpui_kit::Hsla) -> gpui_kit::Div {
    div()
        .absolute()
        .left(x)
        .top(y)
        .ml(-(COLOR_KNOB / 2.0))
        .mt(-(COLOR_KNOB / 2.0))
        .size(COLOR_KNOB)
        .rounded_full()
        .border_2()
        .border_color(hsla(0.0, 0.0, 1.0, 1.0))
        .shadow_sm()
        .bg(fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for c in [0x4dabf7u32, 0xff0000, 0x00ff00, 0x0000ff, 0x7048e8, 0x000000, 0xffffff, 0x868e96] {
            let (h, s, v) = rgb_to_hsv(c);
            assert_eq!(hsv_to_rgb(h, s, v), c, "{c:06x}");
        }
    }

    #[test]
    fn hex_forms() {
        assert_eq!(parse_hex("#4dabf7"), Some(0x4dabf7));
        assert_eq!(parse_hex("4DABF7"), Some(0x4dabf7));
        assert_eq!(parse_hex("#fff"), Some(0xffffff));
        assert_eq!(parse_hex("#ffff"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
    }

    #[test]
    fn used_colors_are_unique_and_in_order() {
        let d = graphing_dsl::Document::parse("a { fill: #ff0000 }\nb { fill: #00ff00, stroke: #ff0000 }\n");
        assert_eq!(used_colors(d.diagram()), [0xff0000, 0x00ff00]);
    }
}
