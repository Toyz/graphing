//! Right panel: the inspector. A header card names what is selected; tabs
//! split Properties, Style and Arrange. Each property is a label line over an
//! editor that edits in place by kind: text, choice, list, color, number.

use std::collections::HashSet;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, AppContext, Context, Div, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder,
};
use graphing_model::{Arrow, Diagram, Op, Placement, Size, Value};
use graphing_scene::Shape;
use graphing_scene::notation::{self, PropKind};
use graphing_scene::stencils::registry;
use graphing_ui::kit::{self, IconButton, Lucide, Segment, Segmented, TextButton};
use graphing_ui::tokens::*;
use graphing_ui::{Colors, UiExt};

use crate::ops::Align;
use crate::workspace::{RightTab, Workspace, shape_glyph, shape_icon};

/// Prop keys that belong to the Style tab, not Properties.
const STYLE_KEYS: &[&str] = &["fill", "stroke", "color", "line", "width"];
/// Target id for diagram-level fields.
const DIAGRAM: &str = "@diagram";

pub struct Field {
    pub input: Entity<InputState>,
    _sub: Subscription,
}

fn ident_like(s: &str) -> bool {
    !s.is_empty() && s.starts_with(|c: char| c.is_alphabetic() || c == '_') && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// A value as the user typed it: identifiers stay bare, the rest quoted.
fn typed(s: &str) -> Value {
    if ident_like(s) { Value::Ident(s.into()) } else { Value::Str(s.into()) }
}

fn humanize(stencil: Option<&str>) -> String {
    let reg = registry();
    let s = reg.resolve(stencil);
    let pack = reg.packs.iter().find(|p| p.id == s.pack).map_or(s.pack.clone(), |p| p.name.clone());
    if s.pack == "core" { format!("{} \u{b7} {}", s.title, s.category) } else { format!("{} \u{b7} {pack}", s.title) }
}

impl Workspace {
    /// The input for `target`/`key`, created on first use and refreshed from
    /// the model whenever it is not being typed in.
    fn field(&mut self, target: &str, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let id = format!("{target}\u{1f}{key}");
        if let Some(f) = self.fields.get(&id) {
            let input = f.input.clone();
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused && input.read(cx).value().as_ref() != value {
                let v = value.to_string();
                input.update(cx, |s, cx| s.set_value(v, window, cx));
            }
            return input;
        }
        let v = value.to_string();
        let placeholder = match key {
            "addport" => "Add port: name or name : Type",
            k if k.starts_with("add:") => "Add item, enter to keep",
            _ => "",
        };
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(placeholder);
            s.set_value(v, window, cx);
            s
        });
        let fid = id.clone();
        let sub = cx.subscribe_in(&input, window, move |ws: &mut Self, input, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let text = input.read(cx).value().to_string();
                ws.commit_field(&fid, text, input.clone(), window, cx);
            }
        });
        self.fields.insert(id, Field { input: input.clone(), _sub: sub });
        input
    }

    fn commit_field(&mut self, id: &str, text: String, input: Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, key)) = id.split_once('\u{1f}') else { return };
        let d = self.view().read(cx).doc().diagram().clone();
        let text = text.trim().to_string();
        let op = if target == DIAGRAM {
            match key {
                "title" => {
                    let title = (!text.is_empty()).then(|| text.clone());
                    (title != d.title).then_some(Op::SetTitle { title })
                }
                key => {
                    let value = (!text.is_empty()).then(|| typed(&text));
                    (value.as_ref() != d.prop(key)).then(|| Op::SetDiagramProp { key: key.into(), value })
                }
            }
        } else if let Some(old) = key.strip_prefix("port:") {
            if text.is_empty() || text == old {
                return;
            }
            crate::ops::rename_port(&d, target, old, &text)
        } else if key == "addport" {
            if text.is_empty() {
                return;
            }
            input.update(cx, |s, cx| s.set_value("", window, cx));
            crate::ops::add_port(&d, target, &text)
        } else if let Some(list) = key.strip_prefix("add:") {
            if text.is_empty() {
                return;
            }
            let mut items = current_prop(&d, target, list).and_then(|v| v.as_list().map(<[Value]>::to_vec)).unwrap_or_default();
            items.push(Value::Str(text.clone()));
            input.update(cx, |s, cx| s.set_value("", window, cx));
            Some(Op::SetProp { id: target.into(), key: list.into(), value: Some(Value::List(items)) })
        } else if key == "label" {
            let label = (!text.is_empty()).then(|| text.replace("\\n", "\n"));
            let current = d.node(target).map(|n| n.label.clone()).or_else(|| d.edge(target).map(|e| e.label.clone())).or_else(|| d.group(target).map(|g| g.label.clone()));
            (current.is_some_and(|c| c != label)).then(|| Op::SetLabel { id: target.into(), label })
        } else if let Some(axis) = ["x", "y", "w", "h"].iter().find(|a| **a == key) {
            let Ok(v) = text.parse::<f64>() else { return };
            let scene = self.view().read(cx).scene();
            let r = scene.rect_of(target);
            let place = d.layout.get(target).copied().or_else(|| r.map(|r| Placement { pos: r.origin, size: None }));
            let (Some(mut p), Some(r)) = (place, r) else { return };
            match *axis {
                "x" => p.pos.x = v,
                "y" => p.pos.y = v,
                "w" => p.size = Some(Size::new(v.max(20.0), p.size.map_or(r.size.h, |s| s.h))),
                _ => p.size = Some(Size::new(p.size.map_or(r.size.w, |s| s.w), v.max(20.0))),
            }
            Some(Op::SetPlacement { id: target.into(), placement: Some(p) })
        } else {
            let value = (!text.is_empty()).then(|| if key == "stereotype" { typed(&text) } else { Value::Str(text.clone()) });
            (value.as_ref() != current_prop(&d, target, key)).then(|| Op::SetProp { id: target.into(), key: key.into(), value })
        };
        if let Some(op) = op {
            self.edit(op, cx);
        }
    }

    pub fn render_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let view = self.view().read(cx);
        let d = view.doc().diagram().clone();
        // The SysML frame stands for the diagram itself.
        let sel: Vec<String> = view.selection().iter().filter(|id| *id != crate::view::FRAME_ID).cloned().collect();
        let src = view.doc().source().to_string();
        let diags: Vec<(usize, String)> = view.doc().diags().iter().map(|g| (src[..g.span.start].matches('\n').count() + 1, g.message.clone())).collect();

        let (header, tabs, body): (AnyElement, Vec<RightTab>, AnyElement) = match sel.as_slice() {
            [] => (self.header_diagram(&d, cx), vec![RightTab::Properties], self.diagram_props(&d, &diags, window, cx)),
            [id] if d.node(id).is_some() => {
                let tabs = vec![RightTab::Properties, RightTab::Style, RightTab::Arrange];
                let body = match self.right_tab {
                    RightTab::Style => self.style_tab(id, &d, cx),
                    RightTab::Arrange => self.arrange_tab(id, &d, window, cx),
                    RightTab::Properties => self.node_props(id, &d, window, cx),
                };
                (self.header_node(id, &d, cx), tabs, body)
            }
            [id] if d.edge(id).is_some() => {
                let tabs = vec![RightTab::Properties, RightTab::Style];
                let body = match self.right_tab {
                    RightTab::Style => self.style_tab(id, &d, cx),
                    _ => self.edge_props(id, &d, window, cx),
                };
                (self.header_edge(id, &d, cx), tabs, body)
            }
            [id] => {
                let tabs = vec![RightTab::Properties, RightTab::Style, RightTab::Arrange];
                let body = match self.right_tab {
                    RightTab::Style => self.style_tab(id, &d, cx),
                    RightTab::Arrange => self.arrange_tab(id, &d, window, cx),
                    RightTab::Properties => self.group_props(id, &d, window, cx),
                };
                (self.header_simple(Lucide::Group, d.group(id).and_then(|g| g.label.clone()).unwrap_or_else(|| id.clone()), "Group".into(), cx), tabs, body)
            }
            many => (self.header_simple(Lucide::Layers, format!("{} selected", many.len()), "Multiple items".into(), cx), vec![RightTab::Arrange], self.multi_tab(cx)),
        };
        let active = if tabs.contains(&self.right_tab) { self.right_tab } else { tabs[0] };
        let segments = (tabs.len() > 1).then(|| {
            Segmented::new(
                "right-tabs",
                tabs.iter()
                    .map(|t| {
                        let t = *t;
                        Segment::new(t.title(), t == active, cx.listener(move |ws, _, _, cx| {
                            ws.right_tab = t;
                            cx.notify();
                        }))
                    })
                    .collect(),
            )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(k.chrome)
            .child(div().flex_none().flex().flex_col().gap(GAP_3).p(PANEL_PAD).child(header).when_some(segments, |d, s| d.child(s)))
            .child(kit::divider_h(cx))
            .child(div().id("inspector-body").flex_1().min_h_0().overflow_y_scroll().pb(GAP_5).child(body))
            .into_any_element()
    }

    // ---- headers ----

    fn header_card(&self, glyph: AnyElement, title: String, subtitle: String, id: Option<String>, cx: &Context<Self>) -> AnyElement {
        let k = cx.ui();
        div()
            .flex()
            .items_center()
            .gap(GAP_3)
            .child(div().flex_none().size(HIT_LG + GAP_3).rounded(ROUND_MD).bg(k.raised).border_1().border_color(k.border).flex().items_center().justify_center().child(glyph))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(GAP_0)
                    .child(
                        div()
                            .id("inspector-title")
                            .text_size(TEXT_LG)
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(k.heading)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .tooltip(kit::tip(title.clone(), Some(subtitle.clone().into())))
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(GAP_2)
                            .text_size(TEXT_SM)
                            .text_color(k.text_muted)
                            .child(subtitle)
                            .when_some(id, |d, id| d.child(div().px(GAP_1).rounded(ROUND_XS).bg(k.hover).text_size(TEXT_XS).font_family(cx.mono()).text_color(k.text_muted).child(id))),
                    ),
            )
            .into_any_element()
    }

    fn header_simple(&self, icon: Lucide, title: String, subtitle: String, cx: &Context<Self>) -> AnyElement {
        let k = cx.ui();
        self.header_card(Icon::new(icon).size(ICON_LG).text_color(k.accent).into_any_element(), title, subtitle, None, cx)
    }

    fn header_node(&self, id: &str, d: &Diagram, cx: &Context<Self>) -> AnyElement {
        let k = cx.ui();
        let n = d.node(id).expect("node");
        let shape = Shape::from_stencil(n.stencil.as_deref());
        let _ = shape;
        self.header_card(shape_glyph(n.stencil.as_deref().unwrap_or("rect"), k).into_any_element(), n.text().replace('\n', " "), humanize(n.stencil.as_deref()), Some(id.to_string()), cx)
    }

    fn header_edge(&self, id: &str, d: &Diagram, cx: &Context<Self>) -> AnyElement {
        let e = d.edge(id).expect("edge");
        let kind = d.edge_prop(e, "kind").map(Value::text).unwrap_or_else(|| "connection".into());
        let title = e.label.clone().unwrap_or_else(|| format!("{} \u{2192} {}", e.from, e.to));
        let k = cx.ui();
        self.header_card(Icon::new(Lucide::Spline).size(ICON_LG).text_color(k.accent).into_any_element(), title, kind, Some(format!("{} \u{2192} {}", e.from, e.to)), cx)
    }

    fn header_diagram(&self, d: &Diagram, cx: &Context<Self>) -> AnyElement {
        let kind = d.prop("kind").map(Value::text).and_then(|k| registry().diagram_kind(&k).map(|dk| format!("{} diagram", dk.name))).unwrap_or_else(|| "Diagram".into());
        let title = d.title.clone().unwrap_or_else(|| self.view().read(cx).title());
        self.header_simple(Lucide::Waypoints, title, kind, cx)
    }

    // ---- rows ----

    /// Label line (icon, name, trailing control) over an editor.
    fn row(&self, icon: Lucide, label: &str, trailing: Option<AnyElement>, editor: impl IntoElement, cx: &Context<Self>) -> Div {
        let k = cx.ui();
        div()
            .flex()
            .flex_col()
            .gap(GAP_1)
            .px(PANEL_PAD)
            .py(GAP_2 + GAP_0)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP_1)
                    .h(HIT_SM)
                    .child(Icon::new(icon).size(ICON_SM).text_color(k.text_faint))
                    .child(div().flex_1().text_size(TEXT_SM).text_color(k.text_muted).child(label.to_string()))
                    .children(trailing),
            )
            .child(editor)
    }

    /// `head` is the row's icon and label.
    /// Removable once it has a value (or is being added).
    fn text_row(&mut self, target: &str, key: &str, head: (Lucide, &str), value: &str, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (icon, label) = head;
        let removable = !value.is_empty() || self.adding.contains(key);
        let input = self.field(target, key, value, window, cx);
        let trailing = removable.then(|| self.remove_button(target, key, cx));
        self.row(icon, label, trailing, kit::text_input(&input), cx)
    }

    fn remove_button(&self, target: &str, key: &str, cx: &Context<Self>) -> AnyElement {
        let (target, key) = (target.to_string(), key.to_string());
        IconButton::new(SharedString::from(format!("rm-{target}-{key}")), Lucide::X)
            .small()
            .tooltip("Remove property")
            .on_click(cx.listener(move |ws, _, _, cx| {
                ws.adding.remove(&key);
                ws.edit(Op::SetProp { id: target.clone(), key: key.clone(), value: None }, cx)
            }))
            .into_any_element()
    }

    fn list_row(&mut self, target: &str, key: &str, label: &str, items: &[Value], window: &mut Window, cx: &mut Context<Self>) -> Div {
        let k = cx.ui();
        let add = self.field(target, &format!("add:{key}"), "", window, cx);
        let lines: Vec<_> = items.iter().enumerate().map(|(i, v)| {
            let (target, key) = (target.to_string(), key.to_string());
            let all = items.to_vec();
            div()
                .id(SharedString::from(format!("li-{key}-{i}")))
                .group("li")
                .min_h(INPUT_H)
                .px(GAP_2)
                .flex()
                .items_center()
                .gap(GAP_1)
                .rounded(ROUND_SM)
                .bg(k.bg)
                .border_1()
                .border_color(k.border)
                .child(div().flex_1().min_w_0().text_size(TEXT_SM).font_family(cx.mono()).text_color(k.text).overflow_hidden().text_ellipsis().whitespace_nowrap().child(v.text()))
                .child(div().invisible().group_hover("li", |d| d.visible()).child(IconButton::new(SharedString::from(format!("lix-{key}-{i}")), Lucide::X).small().tooltip("Remove").on_click(cx.listener(move |ws, _, _, cx| {
                    let mut rest = all.clone();
                    rest.remove(i);
                    ws.edit(Op::SetProp { id: target.clone(), key: key.clone(), value: Some(Value::List(rest)) }, cx)
                }))))
        }).collect();
        let trailing = Some(self.remove_button(target, key, cx));
        self.row(Lucide::List, label, trailing, div().flex().flex_col().gap(GAP_1).children(lines).child(kit::text_input(&add)), cx)
    }

    /// One-click chips for known props the item lacks.
    fn add_chips(&self, target: &str, missing: Vec<(String, String, PropKind)>, cx: &Context<Self>) -> Option<Div> {
        if missing.is_empty() {
            return None;
        }
        let k = cx.ui();
        let chips = missing.into_iter().map(|(key, label, kind)| {
            let target = target.to_string();
            div()
                .id(SharedString::from(format!("add-{key}")))
                .h(HIT_SM)
                .px(GAP_2)
                .flex()
                .items_center()
                .gap(GAP_1)
                .rounded(ROUND_PILL)
                .border_1()
                .border_color(k.border_strong)
                .text_size(TEXT_SM)
                .text_color(k.text_muted)
                .cursor_pointer()
                .hover(|d| d.bg(k.hover).text_color(k.text))
                .on_click(cx.listener({
                    let key = key.clone();
                    move |ws, _, _, cx| {
                        if kind == PropKind::List {
                            ws.edit(Op::SetProp { id: target.clone(), key: key.clone(), value: Some(Value::List(Vec::new())) }, cx);
                        } else {
                            ws.adding.insert(key.clone());
                            cx.notify();
                        }
                    }
                }))
                .child(Icon::new(Lucide::Plus).size(ICON_XS).text_color(k.text_faint))
                .child(label)
        });
        Some(div().px(PANEL_PAD).pt(GAP_3).flex().flex_col().gap(GAP_2).child(kit::caption("Add property", cx)).child(div().flex().flex_wrap().gap(GAP_1).children(chips)))
    }

    // ---- bodies ----

    fn node_props(&mut self, id: &str, d: &Diagram, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let n = d.node(id).expect("node").clone();
        let current = n.stencil.clone().unwrap_or_else(|| "rect".into());
        let k = cx.ui();
        let options: Vec<SelectOption> = registry()
            .catalog()
            .into_iter()
            .flat_map(|(cat, entries)| {
                entries.into_iter().map(move |s| SelectOption {
                    value: s.id.clone(),
                    label: s.title.clone().into(),
                    description: None,
                    detail: Some(s.id.clone().into()),
                    icon: stencil_icon(s),
                    group: Some(cat.clone().into()),
                })
            })
            .collect();
        let target = id.to_string();
        let shape_picker = self.select_box(
            "shape",
            (stencil_icon(registry().resolve(Some(&current))), humanize(Some(&current)).into()),
            options,
            &current,
            std::rc::Rc::new(move |ws, value, cx| ws.edit(Op::SetStencil { id: target.clone(), stencil: Some(value.to_string()) }, cx)),
            cx,
        );
        let label_input = self.field(id, "label", &n.label.clone().unwrap_or_default().replace('\n', "\\n"), window, cx);
        let mut body = div().flex().flex_col().pt(GAP_1);
        let is_part = n.stencil.as_deref().is_some_and(|s| s.ends_with(".part"));
        body = body.child(self.row(Lucide::Type, if is_part { "Type" } else { "Label" }, None, kit::text_input(&label_input), cx));
        body = body.child(self.row(Lucide::Shapes, "Shape", None, shape_picker, cx));
        body = body.child(self.group_row(id, d, cx));
        let schema = notation::props_for(n.stencil.as_deref());
        let mut shown: HashSet<String> = HashSet::new();
        // Ports get their own editor: declared and connected ones together.
        shown.insert("ports".into());
        let ported = schema.iter().any(|p| p.key == "ports");
        let ports = crate::ops::ports_of(d, id);
        if ported || !ports.is_empty() {
            body = body.child(self.ports_row(id, &ports, window, cx));
        }
        let mut missing = Vec::new();
        for spec in schema.iter().filter(|p| p.key != "ports") {
            shown.insert(spec.key.clone());
            let value = d.node_prop(&n, &spec.key);
            match (spec.kind, value) {
                (PropKind::List, Some(v)) => {
                    let items = v.as_list().map(<[Value]>::to_vec).unwrap_or_else(|| vec![v.clone()]);
                    body = body.child(self.list_row(id, &spec.key, &spec.label, &items, window, cx));
                }
                (_, Some(v)) => {
                    let text = v.text();
                    body = body.child(self.text_row(id, &spec.key, (Lucide::TextCursor, &spec.label), &text, window, cx));
                }
                (kind, None) if self.adding.contains(&spec.key) && kind != PropKind::List => {
                    body = body.child(self.text_row(id, &spec.key, (Lucide::TextCursor, &spec.label), "", window, cx));
                }
                (kind, None) => missing.push((spec.key.clone(), spec.label.clone(), kind)),
            }
        }
        // Anything else the node carries, so nothing in the file is hidden.
        for (key, value) in n.props.iter().filter(|(k, _)| !shown.contains(k.as_str()) && !STYLE_KEYS.contains(&k.as_str())) {
            body = match value.as_list() {
                Some(items) => {
                    let items = items.to_vec();
                    body.child(self.list_row(id, key, key, &items, window, cx))
                }
                None => body.child(self.text_row(id, key, (Lucide::Hash, key), &value.text(), window, cx)),
            };
        }
        let _ = k;
        body.children(self.add_chips(id, missing, cx)).into_any_element()
    }

    fn edge_props(&mut self, id: &str, d: &Diagram, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let e = d.edge(id).expect("edge").clone();
        let kind = d.edge_prop(&e, "kind").map(Value::text).unwrap_or_default();
        let mut options = vec![SelectOption { value: String::new(), label: "plain".into(), description: Some("Line with the arrow above".into()), detail: None, icon: Lucide::Minus.into(), group: None }];
        for k in registry().edge_kinds.iter() {
            options.push(SelectOption {
                value: k.name.clone(),
                label: k.name.clone().into(),
                description: (!k.description.is_empty()).then(|| k.description.clone().into()),
                detail: None,
                icon: Lucide::Spline.into(),
                group: Some(if k.group.is_empty() { "Other".into() } else { k.group.clone().into() }),
            });
        }
        let target = id.to_string();
        let kind_picker = self.select_box(
            "edge-kind",
            (Lucide::Spline.into(), if kind.is_empty() { "plain".into() } else { kind.clone().into() }),
            options,
            &kind,
            std::rc::Rc::new(move |ws, value, cx| {
                let value = (!value.is_empty()).then(|| Value::Ident(value.to_string()));
                ws.edit(Op::SetProp { id: target.clone(), key: "kind".into(), value }, cx)
            }),
            cx,
        );
        let arrows = [(Arrow::Forward, "\u{2192}"), (Arrow::Back, "\u{2190}"), (Arrow::Both, "\u{2194}"), (Arrow::None, "\u{2014}")]
            .into_iter()
            .map(|(a, t)| {
                let id = id.to_string();
                Segment::new(t, e.arrow == a, cx.listener(move |ws, _, _, cx| ws.edit(Op::SetArrow { id: id.clone(), arrow: a }, cx)))
            })
            .collect();
        let label = self.field(id, "label", &e.label.clone().unwrap_or_default(), window, cx);
        let stereo = d.edge_prop(&e, "stereotype").map(Value::text).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .pt(GAP_1)
            .child(self.row(Lucide::Type, "Label", None, kit::text_input(&label), cx))
            .child(self.row(Lucide::Workflow, "Kind", None, kind_picker, cx))
            .child(self.row(Lucide::ArrowRightLeft, "Direction", None, Segmented::new("arrow", arrows), cx))
            .child(self.text_row(id, "stereotype", (Lucide::Quote, "Stereotype"), &stereo, window, cx))
            .child(self.row(Lucide::Cable, "Ends", None, kit::muted(format!("{}{}  \u{2192}  {}{}", e.from, e.from_port.as_ref().map(|p| format!(".{p}")).unwrap_or_default(), e.to, e.to_port.as_ref().map(|p| format!(".{p}")).unwrap_or_default()), cx), cx))
            .into_any_element()
    }

    fn group_props(&mut self, id: &str, d: &Diagram, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let g = d.group(id).expect("group").clone();
        let k = cx.ui();
        let label = self.field(id, "label", &g.label.clone().unwrap_or_default(), window, cx);
        // The kind's fields (CIDR, region, CPU...), each a text row.
        let kind_props: Vec<(String, String)> = graphing_model::find_prop(&g.props, "kind")
            .map(Value::text)
            .and_then(|k| graphing_scene::stencils::registry().group_kind(&k).map(|k| k.props.iter().map(|p| (p.key.clone(), p.label.clone())).collect()))
            .unwrap_or_default();
        let kind_rows: Vec<Div> = kind_props
            .into_iter()
            .map(|(key, label)| {
                let current = graphing_model::find_prop(&g.props, &key).map(Value::text).unwrap_or_default();
                let input = self.field(id, &key, &current, window, cx);
                self.row(Lucide::Tag, &label, None, kit::text_input(&input), cx)
            })
            .collect();
        let add = self.member_picker(id, d, cx);
        let links = self.connections(id, d, cx);
        let members = g.members.iter().enumerate().map(|(i, m)| {
            let (gid, member) = (id.to_string(), m.clone());
            let member_id = m.clone();
            let name = d.node(m).map(|n| n.text().replace('\n', " ")).or_else(|| d.group(m).and_then(|g| g.label.clone())).unwrap_or_else(|| m.clone());
            let icon = match d.node(m) {
                Some(n) => shape_icon(Shape::from_stencil(n.stencil.as_deref())),
                None => Lucide::Group,
            };
            div()
                .id(SharedString::from(format!("member-{i}")))
                .group("member")
                .min_h(INPUT_H)
                .px(GAP_2)
                .flex()
                .items_center()
                .gap(GAP_2)
                .rounded(ROUND_SM)
                .bg(k.bg)
                .border_1()
                .border_color(k.border)
                .child(Icon::new(icon).size(ICON_SM).text_color(k.text_muted))
                .tooltip(kit::tip(name.clone(), Some(member_id.clone().into())))
                .child(div().flex_1().min_w_0().text_size(TEXT_SM).text_color(k.text).overflow_hidden().text_ellipsis().whitespace_nowrap().child(name))
                .child(div().invisible().group_hover("member", |d| d.visible()).child(IconButton::new(SharedString::from(format!("mx-{i}")), Lucide::X).small().tooltip("Remove from group").on_click(cx.listener(move |ws, _, _, cx| {
                    let d = ws.view().read(cx).doc().diagram().clone();
                    if let Some(op) = crate::ops::set_group(&d, &member, None).filter(|_| d.node(&member).is_some()) {
                        ws.edit(op, cx);
                    } else if let Some(g) = d.group(&gid) {
                        let members = g.members.iter().filter(|x| **x != member).cloned().collect();
                        ws.edit(Op::SetMembers { group: gid.clone(), members }, cx);
                    }
                }))))
        });
        let ungroup = TextButton::new("ungroup", "Ungroup").icon(Lucide::Ungroup).on_click(cx.listener(|ws, _, _, cx| ws.with_view(cx, |v, cx| v.ungroup_selection(cx))));
        div()
            .flex()
            .flex_col()
            .pt(GAP_1)
            .child(self.row(Lucide::Type, "Label", None, kit::text_input(&label), cx))
            .children(kind_rows)
            .child(self.row(Lucide::Users, &format!("Members  {}", g.members.len()), None, div().flex().flex_col().gap(GAP_1).children(members).child(add), cx))
            .when_some(links, |el, links| el.child(links))
            .child(div().px(PANEL_PAD).pt(GAP_2).child(ungroup))
            .into_any_element()
    }

    /// Adds a node or group that is not already inside this one.
    fn member_picker(&mut self, id: &str, d: &Diagram, cx: &mut Context<Self>) -> AnyElement {
        let g = d.group(id).expect("group");
        // Never offer this group's ancestors: that would make a cycle.
        let mut ancestors = vec![id.to_string()];
        while let Some(p) = crate::ops::parent_group(d, ancestors.last().expect("non-empty")) {
            if ancestors.contains(&p.id) {
                break;
            }
            ancestors.push(p.id.clone());
        }
        let free = |m: &str| !g.members.iter().any(|x| x == m) && !ancestors.iter().any(|a| a == m);
        let mut options: Vec<SelectOption> = d
            .nodes
            .iter()
            .filter(|n| free(&n.id))
            .map(|n| SelectOption {
                value: n.id.clone(),
                label: n.text().replace('\n', " ").into(),
                description: crate::ops::parent_group(d, &n.id).map(|p| format!("in {}", p.label.clone().unwrap_or_else(|| p.id.clone())).into()),
                detail: Some(n.id.clone().into()),
                icon: shape_icon(Shape::from_stencil(n.stencil.as_deref())).into(),
                group: Some("Shapes".into()),
            })
            .collect();
        options.extend(d.groups.iter().filter(|x| free(&x.id)).map(|x| SelectOption {
            value: x.id.clone(),
            label: x.label.clone().unwrap_or_else(|| x.id.clone()).into(),
            description: Some(format!("{} members", x.members.len()).into()),
            detail: Some(x.id.clone().into()),
            icon: Lucide::Group.into(),
            group: Some("Groups".into()),
        }));
        let target = id.to_string();
        self.select_box(
            "group-add",
            (Lucide::Plus.into(), "Add member".into()),
            options,
            "",
            std::rc::Rc::new(move |ws, value, cx| {
                let d = ws.view().read(cx).doc().diagram().clone();
                if let Some(op) = crate::ops::set_group(&d, value, Some(&target)) {
                    ws.edit(op, cx);
                }
            }),
            cx,
        )
    }

    /// Edges that start or end at `id`; clicking one selects it.
    fn connections(&mut self, id: &str, d: &Diagram, cx: &mut Context<Self>) -> Option<Div> {
        let k = cx.ui();
        let edges: Vec<_> = d.edges.iter().filter(|e| e.from == id || e.to == id).collect();
        if edges.is_empty() {
            return None;
        }
        let name = |x: &str| d.node(x).map(|n| n.text().replace('\n', " ")).or_else(|| d.group(x).and_then(|g| g.label.clone())).unwrap_or_else(|| x.to_string());
        let rows = edges.iter().enumerate().map(|(i, e)| {
            let (icon, other) = if e.from == id { (Lucide::ArrowRight, name(&e.to)) } else { (Lucide::ArrowLeft, name(&e.from)) };
            let edge = e.id.clone();
            div()
                .id(SharedString::from(format!("link-{i}")))
                .min_h(INPUT_H)
                .px(GAP_2)
                .flex()
                .items_center()
                .gap(GAP_2)
                .rounded(ROUND_SM)
                .cursor_pointer()
                .hover(|s| s.bg(k.hover))
                .child(Icon::new(icon).size(ICON_SM).text_color(k.text_muted))
                .tooltip(kit::tip(other.clone(), e.label.clone().map(|l| l.replace('\n', " ").into())))
                .child(div().flex_1().min_w_0().text_size(TEXT_SM).text_color(k.text).overflow_hidden().text_ellipsis().whitespace_nowrap().child(other))
                .when_some(e.label.clone(), |el, l| el.child(div().flex_none().text_size(TEXT_XS).text_color(k.text_faint).child(l.replace('\n', " "))))
                .on_click(cx.listener(move |ws, _, _, cx| {
                    let edge = edge.clone();
                    ws.with_view(cx, |v, cx| v.select(vec![edge], cx));
                }))
        }).collect::<Vec<_>>();
        Some(self.row(Lucide::Spline, &format!("Connections  {}", edges.len()), None, div().flex().flex_col().children(rows), cx))
    }

    /// Which group a node sits in, as a select.
    fn group_row(&mut self, id: &str, d: &Diagram, cx: &mut Context<Self>) -> Div {
        let current = crate::ops::parent_group(d, id).map(|g| g.id.clone()).unwrap_or_default();
        let label = d.group(&current).map(|g| g.label.clone().unwrap_or_else(|| g.id.clone())).unwrap_or_else(|| "None".into());
        let mut options = vec![SelectOption { value: String::new(), label: "None".into(), description: Some("Not in a group".into()), detail: None, icon: Lucide::Square.into(), group: None }];
        options.extend(d.groups.iter().map(|g| SelectOption {
            value: g.id.clone(),
            label: g.label.clone().unwrap_or_else(|| g.id.clone()).into(),
            description: Some(format!("{} members", g.members.len()).into()),
            detail: Some(g.id.clone().into()),
            icon: Lucide::Group.into(),
            group: Some("Groups".into()),
        }));
        let target = id.to_string();
        let picker = self.select_box(
            "node-group",
            (Lucide::Group.into(), label.into()),
            options,
            &current,
            std::rc::Rc::new(move |ws, value, cx| {
                let d = ws.view().read(cx).doc().diagram().clone();
                if let Some(op) = crate::ops::set_group(&d, &target, (!value.is_empty()).then_some(value)) {
                    ws.edit(op, cx);
                }
            }),
            cx,
        );
        self.row(Lucide::Group, "Group", None, picker, cx)
    }

    /// Ports: each renames in place (connections follow), removes, and new
    /// ones add as `name` or `name : Type`.
    fn ports_row(&mut self, id: &str, ports: &[(String, Option<String>, usize)], window: &mut Window, cx: &mut Context<Self>) -> Div {
        let k = cx.ui();
        let mut list = div().flex().flex_col().gap(GAP_1);
        for (i, (name, ty, links)) in ports.iter().enumerate() {
            let input = self.field(id, &format!("port:{name}"), name, window, cx);
            let (target, port) = (id.to_string(), name.clone());
            list = list.child(
                div()
                    .group("port")
                    .flex()
                    .items_center()
                    .gap(GAP_1)
                    .child(div().size(ICON_SM).flex_none().border_1().border_color(k.text_muted).bg(k.bg))
                    .child(div().flex_1().child(kit::text_input(&input)))
                    .when_some(ty.clone(), |d, t| d.child(div().flex_none().font_family(cx.mono()).text_size(TEXT_XS).text_color(k.text_faint).child(t)))
                    .child(div().flex_none().w(HIT_LG).text_right().text_size(TEXT_XS).text_color(k.text_faint).child(if *links == 0 { String::new() } else { format!("{links}\u{d7}") }))
                    .child(IconButton::new(SharedString::from(format!("prm-{i}")), Lucide::X).small().tooltip("Remove port").on_click(cx.listener(move |ws, _, _, cx| {
                        let d = ws.view().read(cx).doc().diagram().clone();
                        if let Some(op) = crate::ops::remove_port(&d, &target, &port) {
                            ws.edit(op, cx);
                        }
                    }))),
            );
        }
        let add = self.field(id, "addport", "", window, cx);
        list = list.child(kit::text_input(&add));
        self.row(Lucide::Cable, &format!("Ports  {}", ports.len()), None, list, cx)
    }

    fn diagram_props(&mut self, d: &Diagram, diags: &[(usize, String)], window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let kind = d.prop("kind").map(Value::text).unwrap_or_default();
        let kind_label = registry().diagram_kind(&kind).map_or("None".to_string(), |dk| dk.name.clone());
        let mut options = vec![SelectOption { value: String::new(), label: "None".into(), description: Some("No frame".into()), detail: None, icon: Lucide::Square.into(), group: None }];
        options.extend(registry().diagram_kinds.iter().map(|dk| SelectOption {
            value: dk.id.clone(),
            label: dk.name.clone().into(),
            description: (!dk.description.is_empty()).then(|| dk.description.clone().into()),
            detail: Some(dk.id.clone().into()),
            icon: diagram_icon(&dk.id),
            group: Some("SysML".into()),
        }));
        let kind_picker = self.select_box(
            "diagram-kind",
            (diagram_icon(&kind), kind_label.into()),
            options,
            &kind,
            std::rc::Rc::new(|ws, value, cx| {
                if value.is_empty() {
                    ws.edit(Op::SetDiagramProp { key: "kind".into(), value: None }, cx);
                    return;
                }
                let d = ws.view().read(cx).doc().diagram().clone();
                let mut ops = vec![Op::SetDiagramProp { key: "kind".into(), value: Some(Value::Ident(value.to_string())) }];
                let ctx = registry().diagram_kind(value).map(|dk| dk.context.clone()).filter(|c| !c.is_empty());
                if d.prop("context").is_none()
                    && let Some(ctx) = ctx
                {
                    ops.push(Op::SetDiagramProp { key: "context".into(), value: Some(Value::Ident(ctx)) });
                }
                ws.edit(Op::Batch(ops), cx)
            }),
            cx,
        );
        let technical = graphing_scene::notation::technical(d);
        let looks = [("Default", false), ("Technical", true)]
            .into_iter()
            .map(|(t, on)| {
                Segment::new(t, technical == on, cx.listener(move |ws, _, _, cx| {
                    let value = Some(Value::Ident(if on { "technical" } else { "default" }.into()));
                    ws.edit(Op::SetDiagramProp { key: "look".into(), value }, cx)
                }))
            })
            .collect();
        // Straight lines, or elbows routed round shapes.
        let elbow = matches!(d.prop("routing").map(Value::text).as_deref(), Some("orthogonal" | "elbow" | "manhattan"));
        let routing = [("Straight", false), ("Elbow", true)]
            .into_iter()
            .map(|(t, on)| {
                Segment::new(t, elbow == on, cx.listener(move |ws, _, _, cx| {
                    let value = on.then(|| Value::Ident("orthogonal".into()));
                    ws.edit(Op::SetDiagramProp { key: "routing".into(), value }, cx)
                }))
            })
            .collect();
        let title = self.field(DIAGRAM, "title", &d.title.clone().unwrap_or_default(), window, cx);
        let context = self.field(DIAGRAM, "context", &d.prop("context").map(Value::text).unwrap_or_default(), window, cx);
        let view_name = self.field(DIAGRAM, "view", &d.prop("view").map(Value::text).unwrap_or_default(), window, cx);
        let mut body = div()
            .flex()
            .flex_col()
            .pt(GAP_1)
            .child(self.row(Lucide::Type, "Title", None, kit::text_input(&title), cx))
            .child(self.row(Lucide::Frame, "Diagram kind", None, kind_picker, cx))
            .child(self.row(Lucide::Box, "Context", None, kit::text_input(&context), cx))
            .child(self.row(Lucide::Eye, "View", None, kit::text_input(&view_name), cx))
            .child(self.row(Lucide::Palette, "Look", None, Segmented::new("look", looks), cx))
            .child(self.row(Lucide::Spline, "Lines", None, Segmented::new("routing", routing), cx))
            .child(self.row(Lucide::Package, "Packs", None, kit::muted(if d.packs.is_empty() { "none".to_string() } else { d.packs.join(", ") }, cx), cx))
            .child(
                div().px(PANEL_PAD).pt(GAP_3).flex().gap(GAP_4).children([
                    stat(d.nodes.len(), "nodes", k),
                    stat(d.edges.len(), "edges", k),
                    stat(d.groups.len(), "groups", k),
                ]),
            );
        if !diags.is_empty() {
            let list = diags.iter().map(|(line, msg)| {
                div()
                    .flex()
                    .gap(GAP_2)
                    .px(GAP_2)
                    .py(GAP_1)
                    .rounded(ROUND_SM)
                    .bg(k.warning.opacity(0.08))
                    .text_size(TEXT_SM)
                    .child(div().flex_none().font_family(cx.mono()).text_color(k.warning).child(format!("L{line}")))
                    .child(div().text_color(k.text).child(msg.clone()))
            });
            body = body.child(div().px(PANEL_PAD).pt(GAP_4).flex().flex_col().gap(GAP_1).child(kit::caption(format!("Problems  {}", diags.len()), cx)).children(list));
        }
        body.into_any_element()
    }

    fn style_tab(&mut self, id: &str, d: &Diagram, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div().flex().flex_col().pt(GAP_1);
        if d.node(id).is_some() {
            for (key, label, icon) in [("fill", "Fill", Lucide::PaintBucket), ("stroke", "Border", Lucide::Square), ("color", "Text", Lucide::Type)] {
                let field = self.color_field(id, key, d, cx);
                body = body.child(self.row(icon, label, None, field, cx));
            }
        } else if let Some(e) = d.edge(id) {
            let dashed = matches!(d.edge_prop(e, "line").map(Value::text).as_deref(), Some("dashed"));
            let lines = [("Solid", false), ("Dashed", true)]
                .into_iter()
                .map(|(t, on)| {
                    let id = id.to_string();
                    Segment::new(t, dashed == on, cx.listener(move |ws, _, _, cx| {
                        let value = on.then(|| Value::Ident("dashed".into()));
                        ws.edit(Op::SetProp { id: id.clone(), key: "line".into(), value }, cx)
                    }))
                })
                .collect();
            body = body.child(self.row(Lucide::Minus, "Pattern", None, Segmented::new("pattern", lines), cx));
            // This line's route: the diagram's, or its own.
            let route = d.edge_prop(e, "route").map(Value::text);
            let routes = [("Diagram", None), ("Straight", Some("straight")), ("Elbow", Some("orthogonal"))]
                .into_iter()
                .map(|(t, v)| {
                    let id = id.to_string();
                    let on = route.as_deref() == v || (v == Some("orthogonal") && route.as_deref() == Some("elbow"));
                    Segment::new(t, on, cx.listener(move |ws, _, _, cx| {
                        let value = v.map(|v| Value::Ident(v.into()));
                        ws.edit(Op::SetProp { id: id.clone(), key: "route".into(), value }, cx)
                    }))
                })
                .collect();
            body = body.child(self.row(Lucide::Spline, "Route", None, Segmented::new("route", routes), cx));
            let field = self.color_field(id, "stroke", d, cx);
            body = body.child(self.row(Lucide::Palette, "Color", None, field, cx));
        } else if let Some(g) = d.group(id) {
            let current = graphing_scene::GroupLook::of(&g.props, graphing_scene::notation::technical(d));
            let options = graphing_scene::GroupLook::ALL
                .iter()
                .map(|l| SelectOption {
                    value: l.name().into(),
                    label: l.title().into(),
                    description: Some(l.description().into()),
                    detail: None,
                    icon: group_look_icon(*l).into(),
                    group: None,
                })
                .collect();
            let target = id.to_string();
            let picker = self.select_box(
                "group-look",
                (group_look_icon(current).into(), current.title().into()),
                options,
                current.name(),
                std::rc::Rc::new(move |ws, value, cx| {
                    let d = ws.view().read(cx).doc().diagram().clone();
                    let mut ops = vec![Op::SetProp { id: target.clone(), key: "look".into(), value: Some(Value::Ident(value.into())) }];
                    // `look` replaces the older `line: solid`.
                    if d.group(&target).is_some_and(|g| graphing_model::find_prop(&g.props, "line").is_some()) {
                        ops.push(Op::SetProp { id: target.clone(), key: "line".into(), value: None });
                    }
                    ws.edit(Op::Batch(ops), cx);
                }),
                cx,
            );
            let kind_picker = self.group_kind_picker(id, d, cx);
            body = body.child(self.row(Lucide::Shapes, "Kind", None, kind_picker, cx));
            body = body.child(self.row(Lucide::LayoutTemplate, "Design", None, picker, cx));
            for (key, label, icon) in [("fill", "Fill", Lucide::PaintBucket), ("stroke", "Border", Lucide::Square), ("color", "Title", Lucide::Type)] {
                let field = self.color_field(id, key, d, cx);
                body = body.child(self.row(icon, label, None, field, cx));
            }
        }
        body.into_any_element()
    }

    fn arrange_tab(&mut self, id: &str, d: &Diagram, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let scene = self.view().read(cx).scene();
        let Some(r) = scene.rect_of(id) else { return div().into_any_element() };
        let k = cx.ui();
        let auto = d.layout.get(id).and_then(|p| p.size).is_none();
        let mut grid = div().flex().flex_wrap().gap(GAP_2);
        for (key, label, value) in [("x", "X", r.origin.x), ("y", "Y", r.origin.y), ("w", "W", r.size.w), ("h", "H", r.size.h)] {
            let input = self.field(id, key, &graphing_dsl::fmt_num(value), window, cx);
            grid = grid.child(
                div()
                    .w(INSPECTOR_W / 2.0 - PANEL_PAD - GAP_1)
                    .flex()
                    .items_center()
                    .gap(GAP_1)
                    .child(div().w(ICON_LG).text_size(TEXT_SM).text_color(k.text_faint).child(label))
                    .child(div().flex_1().child(kit::text_input(&input))),
            );
        }
        let reset = (!auto).then(|| {
            let id = id.to_string();
            let text = if d.group(&id).is_some() { "Fit members" } else { "Auto size" };
            TextButton::new("auto-size", text).icon(Lucide::Scaling).on_click(cx.listener(move |ws, _, _, cx| {
                let d = ws.view().read(cx).doc().diagram().clone();
                if let Some(p) = d.layout.get(&id) {
                    ws.edit(Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos: p.pos, size: None }) }, cx);
                }
            }))
        });
        div()
            .flex()
            .flex_col()
            .pt(GAP_1)
            .child(self.row(Lucide::Move, "Position and size", reset.map(IntoElement::into_any_element), grid, cx))
            .child(div().px(PANEL_PAD).pt(GAP_4).child(
                TextButton::new("delete-node", "Delete").icon(Lucide::Trash).danger().on_click(cx.listener(|ws, _, _, cx| ws.with_view(cx, |v, cx| v.delete_selected(cx)))),
            ))
            .into_any_element()
    }

    fn multi_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let btn = |id: &'static str, icon: Lucide, tip: &'static str, how: Align| {
            IconButton::new(id, icon).tooltip(tip).on_click(cx.listener(move |ws, _, _, cx| ws.with_view(cx, |v, cx| v.align(how, cx))))
        };
        div()
            .flex()
            .flex_col()
            .pt(GAP_1)
            .child(self.row(
                Lucide::AlignStartVertical,
                "Align",
                None,
                div()
                    .flex()
                    .gap(GAP_0)
                    .child(btn("al", Lucide::AlignStartVertical, "Align left", Align::Left))
                    .child(btn("ac", Lucide::AlignCenterVertical, "Align center", Align::CenterX))
                    .child(btn("ar", Lucide::AlignEndVertical, "Align right", Align::Right))
                    .child(kit::divider_v(cx).mx(GAP_1))
                    .child(btn("at", Lucide::AlignStartHorizontal, "Align top", Align::Top))
                    .child(btn("am", Lucide::AlignCenterHorizontal, "Align middle", Align::CenterY))
                    .child(btn("ab", Lucide::AlignEndHorizontal, "Align bottom", Align::Bottom)),
                cx,
            ))
            .child(self.row(
                Lucide::AlignHorizontalSpaceAround,
                "Distribute",
                None,
                div()
                    .flex()
                    .gap(GAP_0)
                    .child(btn("sh", Lucide::AlignHorizontalSpaceAround, "Distribute horizontally", Align::SpreadX))
                    .child(btn("sv", Lucide::AlignVerticalSpaceAround, "Distribute vertically", Align::SpreadY))
                    .child(IconButton::new("ss", Lucide::Scaling).tooltip("Make same size").on_click(cx.listener(|ws, _, _, cx| ws.with_view(cx, |v, cx| v.same_size(cx))))),
                cx,
            ))
            .child(div().px(PANEL_PAD).pt(GAP_3).child(
                TextButton::new("group-sel", "Group selection").icon(Lucide::Group).on_click(cx.listener(|ws, _, _, cx| ws.with_view(cx, |v, cx| v.group_selection(cx)))),
            ))
            .child(div().px(PANEL_PAD).pt(GAP_2).child(
                TextButton::new("delete-many", "Delete selection").icon(Lucide::Trash).danger().on_click(cx.listener(|ws, _, _, cx| ws.with_view(cx, |v, cx| v.delete_selected(cx)))),
            ))
            .into_any_element()
    }
}

/// Icon per diagram kind, from its pack; a frame when none is given.
pub fn diagram_icon(kind: &str) -> Icon {
    registry().diagram_kind(kind).and_then(|dk| dk.icon.as_deref().map(kit::icon_named)).unwrap_or_else(|| Lucide::Frame.into())
}

/// A stencil's list icon: from its pack, else from its outline.
pub fn stencil_icon(s: &graphing_scene::stencils::StencilDef) -> Icon {
    s.icon.as_deref().map(kit::icon_named).unwrap_or_else(|| shape_icon(s.shape()).into())
}

fn stat(n: usize, what: &str, k: Colors) -> Div {
    div()
        .flex()
        .flex_col()
        .child(div().text_size(TEXT_XL).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.heading).child(n.to_string()))
        .child(div().text_size(TEXT_XS).text_color(k.text_faint).child(what.to_string()))
}

fn current_prop<'a>(d: &'a Diagram, target: &str, key: &str) -> Option<&'a Value> {
    d.node(target)
        .and_then(|n| graphing_model::find_prop(&n.props, key))
        .or_else(|| d.edge(target).and_then(|e| graphing_model::find_prop(&e.props, key)))
        .or_else(|| d.group(target).and_then(|g| graphing_model::find_prop(&g.props, key)))
}

/// One choice in a searchable select.
pub struct SelectOption {
    pub value: String,
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub detail: Option<SharedString>,
    pub icon: Icon,
    pub group: Option<SharedString>,
}

pub type OnPick = std::rc::Rc<dyn Fn(&mut Workspace, &str, &mut Context<Workspace>)>;

/// The open select: which one, and its search field.
pub struct OpenSelect {
    pub id: SharedString,
    pub query: Entity<InputState>,
    pub(crate) _sub: Subscription,
}

impl Workspace {
    /// A searchable select: input-styled trigger, popover with a search field
    /// and grouped options. `current` is the checked value.
    pub(crate) fn select_box(
        &mut self,
        id: &'static str,
        trigger: (Icon, SharedString),
        options: Vec<SelectOption>,
        current: &str,
        on_pick: OnPick,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.select_box_sized(id, trigger, options, current, on_pick, None, cx)
    }

    /// [`Self::select_box`] whose popover is at least `min_w` wide.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn select_box_sized(
        &mut self,
        id: &'static str,
        trigger: (Icon, SharedString),
        options: Vec<SelectOption>,
        current: &str,
        on_pick: OnPick,
        min_w: Option<gpui_kit::Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_open = self.select.as_ref().is_some_and(|s| s.id.as_ref() == id);
        let button = graphing_ui::menu::select_trigger(
            SharedString::from(format!("{id}-trigger")),
            Some(trigger.0),
            trigger.1,
            is_open,
            cx.listener(move |ws, _, window, cx| ws.toggle_select(id, window, cx)),
            cx,
        );
        let popover = is_open.then(|| {
            let query = self.select.as_ref().map(|s| s.query.read(cx).value().to_lowercase()).unwrap_or_default();
            let mut rows = Vec::new();
            let mut group = None;
            // Only the first option with the current value shows checked.
            let mut checked_once = false;
            for o in options.into_iter().filter(|o| {
                query.is_empty() || o.label.to_lowercase().contains(&query) || o.value.to_lowercase().contains(&query) || o.group.as_ref().is_some_and(|g| g.to_lowercase().contains(&query))
            }) {
                if o.group != group && o.group.is_some() {
                    group = o.group.clone();
                    rows.push(graphing_ui::menu::MenuRow::Caption(o.group.clone().unwrap_or_default()));
                }
                let checked = !checked_once && o.value == current;
                checked_once |= checked;
                let (value, pick) = (o.value.clone(), on_pick.clone());
                let mut row = graphing_ui::menu::MenuRow::item(o.label, cx.listener(move |ws, _, window, cx| {
                    ws.select = None;
                    pick(ws, &value, cx);
                    ws.focus_canvas(window, cx);
                    cx.notify();
                }))
                .icon(o.icon)
                .checked(checked);
                if let Some(d) = o.description {
                    row = row.description(d);
                }
                if let Some(d) = o.detail {
                    row = row.detail(d);
                }
                rows.push(row);
            }
            if rows.is_empty() {
                rows.push(graphing_ui::menu::MenuRow::Caption("No matches".into()));
            }
            let search = self.select.as_ref().map(|s| s.query.clone()).expect("open");
            let header = kit::text_input(&search)
                .prefix(Icon::new(Lucide::Search).size(ICON_SM).text_color(cx.ui().text_faint))
                .into_any_element();
            let surface = graphing_ui::menu::menu_surface_with(SharedString::from(format!("{id}-pop")), Some(header), rows, Some(SELECT_LIST_H), cx)
                .w_full()
                .on_mouse_down_out(cx.listener(|ws, _, _, cx| {
                    ws.select = None;
                    cx.notify();
                }));
            graphing_ui::menu::animate(surface, SharedString::from(format!("{id}-anim")))
        });
        div()
            .relative()
            .w_full()
            .child(button)
            .when_some(popover, |d, p| {
                let at = div().absolute().top(INPUT_H + GAP_1).left_0();
                let at = match min_w {
                    Some(w) => at.w(w),
                    None => at.right_0(),
                };
                d.child(at.child(gpui_kit::deferred(p).with_priority(3)))
            })
            .into_any_element()
    }

    pub(crate) fn toggle_select(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if self.select.as_ref().is_some_and(|s| s.id.as_ref() == id) {
            self.select = None;
            cx.notify();
            return;
        }
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let sub = cx.subscribe_in(&query, window, |_: &mut Self, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        query.update(cx, |s, cx| s.focus(window, cx));
        self.select = Some(OpenSelect { id: id.into(), query, _sub: sub });
        cx.notify();
    }
}

impl Workspace {
    /// Group presets from the packs (containers, SysML, compute, networking).
    fn group_kind_picker(&mut self, id: &str, d: &Diagram, cx: &mut Context<Self>) -> AnyElement {
        let g = d.group(id).expect("group");
        let current = graphing_model::find_prop(&g.props, "kind").map(Value::text).unwrap_or_default();
        let reg = graphing_scene::stencils::registry();
        let label = reg.group_kind(&current).map_or("None".to_string(), |k| k.title.clone());
        let icon = reg.group_kind(&current).and_then(|k| k.icon.as_deref().map(kit::icon_named)).unwrap_or_else(|| Lucide::Group.into());
        let mut options = vec![SelectOption { value: String::new(), label: "None".into(), description: Some("Plain group".into()), detail: None, icon: Lucide::Square.into(), group: None }];
        options.extend(reg.group_kinds.iter().map(|k| SelectOption {
            value: k.name.clone(),
            label: k.title.clone().into(),
            description: (!k.description.is_empty()).then(|| k.description.clone().into()),
            detail: Some(k.name.clone().into()),
            icon: k.icon.as_deref().map(kit::icon_named).unwrap_or_else(|| Lucide::Group.into()),
            group: Some(k.category.clone().into()),
        }));
        drop(reg);
        let target = id.to_string();
        self.select_box(
            "group-kind",
            (icon, label.into()),
            options,
            &current,
            std::rc::Rc::new(move |ws, value, cx| {
                let d = ws.view().read(cx).doc().diagram().clone();
                let value = (!value.is_empty()).then(|| Value::Ident(value.into()));
                let mut ops = vec![Op::SetProp { id: target.clone(), key: "kind".into(), value }];
                // A kind brings its own design; drop an explicit one.
                if d.group(&target).is_some_and(|g| graphing_model::find_prop(&g.props, "look").is_some()) {
                    ops.push(Op::SetProp { id: target.clone(), key: "look".into(), value: None });
                }
                ws.edit(Op::Batch(ops), cx);
            }),
            cx,
        )
    }
}

/// Icon for a group design in pickers.
pub(crate) fn group_look_icon(look: graphing_scene::GroupLook) -> Lucide {
    use graphing_scene::GroupLook as L;
    match look {
        L::Dashed => Lucide::SquareDashed,
        L::Solid => Lucide::Square,
        L::Package => Lucide::Folder,
        L::Lane => Lucide::PanelTop,
        L::Zone => Lucide::Layers,
        L::Card => Lucide::PanelsTopLeft,
        L::Sysml => Lucide::Box,
    }
}
