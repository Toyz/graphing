//! File > New from Template: a diagram to start from in every notation
//! graphing ships, from the examples (so they stay tested and current).

use gpui_kit::{
    AnyElement, Context, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, deferred, div,
};
use graphing_dsl::Document;
use graphing_ui::UiExt;
use graphing_ui::kit::{self, Lucide};
use graphing_ui::tokens::*;

use crate::workspace::Workspace;

pub(crate) struct Template {
    pub name: &'static str,
    /// The pack whose icon the card shows.
    pub pack: &'static str,
    pub blurb: &'static str,
    pub src: &'static str,
}

macro_rules! example {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/", $path))
    };
}

pub(crate) const TEMPLATES: &[Template] = &[
    Template { name: "Blank", pack: "core", blurb: "An empty canvas", src: "diagram \"Untitled\"\n" },
    Template { name: "Guided explainer", pack: "c4", blurb: "A request, step by step: plays and exports as GIF or video", src: example!("animated/request.gph") },
    Template { name: "Flowchart", pack: "core", blurb: "Steps, decisions and stores", src: example!("auth.gph") },
    Template { name: "C4 containers", pack: "c4", blurb: "Who uses a system, what it is made of", src: example!("notations/c4-container.gph") },
    Template { name: "UML classes", pack: "uml", blurb: "Classes, interfaces and how they relate", src: example!("notations/uml-class.gph") },
    Template { name: "Database schema", pack: "er", blurb: "Tables and crow's foot relationships", src: example!("notations/er.gph") },
    Template { name: "BPMN process", pack: "bpmn", blurb: "Events, tasks and gateways in a pool", src: example!("notations/bpmn.gph") },
    Template { name: "Threat model", pack: "dfd", blurb: "Data flows across trust boundaries", src: example!("notations/threat-model.gph") },
    Template { name: "SysML blocks", pack: "sysml", blurb: "Block definition: parts, values, specialisation", src: example!("sysml/bdd-vehicle.gph") },
    Template { name: "SysML internal blocks", pack: "sysml", blurb: "Parts, ports and connectors inside a block", src: example!("hil-ibd.gph") },
    Template { name: "Requirements", pack: "sysml", blurb: "Requirements and what satisfies them", src: example!("sysml/req-range.gph") },
    Template { name: "Control loop", pack: "control", blurb: "Gains, sums and a transfer function", src: example!("notations/control-loop.gph") },
    Template { name: "Timing", pack: "timing", blurb: "Clocks, levels and buses over time (WaveDrom letters)", src: example!("notations/timing.gph") },
    Template { name: "Fault tree", pack: "fta", blurb: "How failures combine into a hazard", src: example!("notations/fault-tree.gph") },
    Template { name: "ArchiMate", pack: "archimate", blurb: "Business down to technology, layer by layer", src: example!("notations/archimate.gph") },
    Template { name: "Event storming", pack: "es", blurb: "Domain events and what causes them", src: example!("notations/event-storming.gph") },
    Template { name: "Network", pack: "net", blurb: "Devices, links and zones", src: example!("notations/network.gph") },
    Template { name: "Org chart", pack: "org", blurb: "People and teams, top down", src: example!("notations/org-chart.gph") },
    Template { name: "Mind map", pack: "org", blurb: "Ideas around one topic", src: example!("notations/mind-map.gph") },
];

impl Workspace {
    pub(crate) fn open_templates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.templates = Some(focus);
        cx.notify();
    }

    fn pick_template(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.templates = None;
        if let Some(t) = TEMPLATES.get(i) {
            self.new_tab(Document::parse(t.src), None, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn render_templates(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let focus = self.templates.clone()?;
        let k = cx.ui();
        let icons: Vec<gpui_kit::component::Icon> = {
            let reg = graphing_scene::stencils::registry();
            TEMPLATES
                .iter()
                .map(|t| reg.packs.iter().find(|p| p.id == t.pack).and_then(|p| p.icon.as_deref()).map(kit::icon_named).unwrap_or_else(|| Lucide::Shapes.into()))
                .collect()
        };
        let cards = TEMPLATES.iter().zip(icons).enumerate().map(|(i, (t, icon))| {
            div()
                .id(("template", i))
                .w(gpui_kit::relative(1.0 / 3.0))
                .p(GAP_1)
                .child(
                    div()
                        .h_full()
                        .flex()
                        .flex_col()
                        .gap(GAP_2)
                        .p(GAP_3)
                        .rounded(ROUND_MD)
                        .border_1()
                        .border_color(k.border)
                        .bg(k.bg)
                        .cursor_pointer()
                        .hover(|d| d.border_color(k.accent).bg(k.accent_soft))
                        .child(div().size(HIT_LG).rounded(ROUND_SM).bg(k.accent_soft).flex().items_center().justify_center().child(icon.size(ICON_LG).text_color(k.accent)))
                        .child(div().text_size(TEXT_MD).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.heading).child(t.name))
                        .child(div().text_size(TEXT_SM).text_color(k.text_muted).line_clamp(2).child(t.blurb)),
                )
                .on_click(cx.listener(move |ws, _, window, cx| ws.pick_template(i, window, cx)))
        });
        let card = kit::raised(cx)
            .id("templates")
            .w(TEMPLATES_W)
            .max_h(gpui_kit::relative(0.8))
            .flex()
            .flex_col()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(GAP_3)
                    .px(GAP_5)
                    .py(GAP_4)
                    .border_b_1()
                    .border_color(k.border)
                    .child(div().flex_1().text_size(TEXT_XL).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.heading).child("New from template"))
                    .child(div().text_size(TEXT_XS).text_color(k.text_faint).child("Esc to close")),
            )
            .child(div().id("templates-grid").flex_1().min_h_0().overflow_y_scroll().p(GAP_4).flex().flex_wrap().children(cards));
        let overlay = div()
            .id("templates-backdrop")
            .absolute()
            .inset_0()
            .bg(k.modal_scrim)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&focus)
            .on_key_down(cx.listener(|ws, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    ws.templates = None;
                    ws.focus_canvas(window, cx);
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|ws, _, _, cx| {
                ws.templates = None;
                cx.notify();
            }))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(graphing_ui::menu::animate(card, "templates-anim"));
        Some(deferred(overlay).with_priority(100).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_parses_clean() {
        for t in TEMPLATES {
            let d = Document::parse(t.src);
            assert!(d.diags().is_empty(), "{}: {:?}", t.name, d.diags());
            assert!(graphing_scene::stencils::registry().packs.iter().any(|p| p.id == t.pack), "{}: no pack {}", t.name, t.pack);
        }
    }
}
