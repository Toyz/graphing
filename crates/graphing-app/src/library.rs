//! Left panel: the shape library (search, collapsible categories, tiles you
//! click or drag onto the canvas) and the outline of the open diagram.

use gpui_kit::{
    AnyElement, App, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};
use graphing_scene::Shape;
use graphing_scene::stencils::registry;
use graphing_ui::kit::{self, Lucide, Row, Segment, Segmented};
use graphing_ui::tokens::*;
use graphing_ui::{Colors, UiExt};

use crate::inspector::SelectOption;
use crate::workspace::{LeftTab, Workspace, shape_glyph, shape_icon};

/// Library tiles of one section: (stencil or `group:<kind>`, title, detail).
type Tiles = Vec<(String, String, String)>;

/// How many stencils "Recently used" keeps.
const RECENT_MAX: usize = 8;

/// A library tile for a stencil id or `group:<kind>`, if it still exists.
fn tile_for(stencil: &str) -> Option<(String, String, String)> {
    let reg = registry();
    match stencil.strip_prefix("group:") {
        Some(kind) => reg.group_kind(kind).map(|g| (stencil.to_string(), g.title.clone(), g.description.clone())),
        None => reg.get(stencil).map(|s| (s.id.clone(), s.title.clone(), s.id.clone())),
    }
}

/// A category without its notation's name in front: "ArchiMate business"
/// reads "Business" under the ArchiMate header.
fn short_category(category: &str, pack: &str) -> String {
    let rest = category.strip_prefix(pack).map(str::trim_start).filter(|r| !r.is_empty());
    match rest {
        Some(r) => {
            let mut c = r.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
        None => category.to_string(),
    }
}

/// Which parts the Shapes pane lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Kinds {
    #[default]
    All,
    Shapes,
    Containers,
}

/// Payload while a library tile is dragged onto the canvas.
#[derive(Clone)]
pub struct StencilDrag {
    pub stencil: SharedString,
    pub title: SharedString,
}

/// What follows the pointer during a drag.
pub struct DragGhost {
    title: SharedString,
    stencil: SharedString,
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = cx.ui();
        kit::raised(cx)
            .flex()
            .items_center()
            .gap(GAP_2)
            .px(GAP_2)
            .py(GAP_1)
            .opacity(0.92)
            .child(shape_glyph(&self.stencil, k))
            .child(div().text_size(TEXT_SM).text_color(k.text).child(self.title.clone()))
    }
}

impl Workspace {
    /// The Shapes or Outline pane: a search field over its list.
    pub fn render_library(&mut self, tab: LeftTab, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let query = self.library_query.read(cx).value().to_lowercase();
        let body = match tab {
            LeftTab::Shapes => self.shapes(&query, k, cx),
            LeftTab::Outline => self.outline(&query, k, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(k.chrome)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(GAP_2)
                    .p(PANEL_PAD)
                    .child(
                        kit::text_input(&self.library_query)
                            .prefix(gpui_kit::component::Icon::new(Lucide::Search).size(ICON_SM).text_color(k.text_faint)),
                    )
                    .when(tab == LeftTab::Shapes, |d| d.child(self.notation_picker(cx)))
                    .when(tab == LeftTab::Shapes, |d| {
                        let filter = self.library_filter;
                        let segs = [(Kinds::All, "All"), (Kinds::Shapes, "Shapes"), (Kinds::Containers, "Containers")]
                            .into_iter()
                            .map(|(v, label)| {
                                Segment::new(label, filter == v, cx.listener(move |ws, _, _, cx| {
                                    ws.library_filter = v;
                                    cx.notify();
                                }))
                            })
                            .collect();
                        d.child(Segmented::new("library-filter", segs))
                    }),
            )
            .child(div().id("library-body").flex_1().min_h_0().overflow_y_scroll().pb(GAP_4).child(body))
            .into_any_element()
    }

    /// The notations the Shapes pane lists: `*` all, one pack id, or (empty)
    /// the ones this diagram uses: its `use` line, the packs of its shapes
    /// and containers, and core.
    pub(crate) fn library_packs(&self, cx: &App) -> Vec<String> {
        self.packs_in(&self.library_scope, cx)
    }

    fn packs_in(&self, scope: &str, cx: &App) -> Vec<String> {
        let reg = registry();
        let all = || reg.packs.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        match scope {
            "*" => all(),
            "" => {
                let d = self.view().read(cx).doc().diagram();
                let mut used: Vec<String> = vec!["core".into()];
                used.extend(d.packs.iter().cloned());
                used.extend(d.nodes.iter().map(|n| reg.resolve(n.stencil.as_deref()).pack.clone()));
                used.extend(d.groups.iter().filter_map(|g| {
                    let kind = graphing_model::find_prop(&g.props, "kind").map(graphing_model::Value::text)?;
                    reg.group_kind(&kind).map(|k| k.pack.clone())
                }));
                // The diagram's own notations first, in registry order; core,
                // always there, last.
                let mut packs: Vec<String> = all().into_iter().filter(|p| p != "core" && used.contains(p)).collect();
                packs.push("core".into());
                packs
            }
            one => vec![one.to_string()],
        }
    }

    /// The notation picker over the Shapes pane.
    fn notation_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let scope = self.library_scope.clone();
        let in_diagram = self.packs_in("", cx);
        let reg = registry();
        let count = |pack: &str| reg.stencils().filter(|s| s.pack == pack && !s.hidden).count() + reg.group_kinds.iter().filter(|k| k.pack == pack).count();
        let icon_of = |p: &graphing_scene::stencils::PackInfo| p.icon.as_deref().map(kit::icon_named).unwrap_or_else(|| Lucide::Shapes.into());
        let names: Vec<String> = reg.packs.iter().filter(|p| in_diagram.contains(&p.id)).map(|p| p.name.clone()).collect();
        let mut options = vec![
            SelectOption {
                value: String::new(),
                label: "In this diagram".into(),
                description: Some(names.join(", ").into()),
                detail: Some(format!("{}", in_diagram.len()).into()),
                icon: Lucide::Pin.into(),
                group: None,
            },
            SelectOption {
                value: "*".into(),
                label: "All notations".into(),
                description: Some(format!("{} notations, everything graphing draws", reg.packs.len()).into()),
                detail: None,
                icon: Lucide::LayoutGrid.into(),
                group: None,
            },
        ];
        options.extend(reg.packs.iter().map(|p| SelectOption {
            value: p.id.clone(),
            label: p.name.clone().into(),
            description: (!p.description.is_empty()).then(|| p.description.clone().into()),
            detail: Some(count(&p.id).to_string().into()),
            icon: icon_of(p),
            group: Some("Notations".into()),
        }));
        let trigger = match scope.as_str() {
            "" => {
                // Name the diagram's own notations; core goes without saying.
                let own: Vec<&str> = reg.packs.iter().filter(|p| p.id != "core" && in_diagram.contains(&p.id)).map(|p| p.name.as_str()).collect();
                let label = if own.is_empty() { "This diagram: Core".to_string() } else { format!("This diagram: {}", own.join(", ")) };
                (gpui_kit::component::Icon::from(Lucide::Pin), SharedString::from(label))
            }
            "*" => (Lucide::LayoutGrid.into(), "All notations".into()),
            id => match reg.packs.iter().find(|p| p.id == id) {
                Some(p) => (icon_of(p), p.name.clone().into()),
                None => (Lucide::Shapes.into(), id.to_string().into()),
            },
        };
        drop(reg);
        self.select_box_sized(
            "library-notation",
            trigger,
            options,
            &scope,
            std::rc::Rc::new(|ws, value, cx| {
                ws.library_scope = value.to_string();
                cx.notify();
            }),
            Some(SELECT_WIDE_W),
            cx,
        )
    }

    fn shapes(&mut self, query: &str, k: Colors, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.library_filter;
        let searching = !query.is_empty();
        // Search always covers every notation.
        let packs: Vec<String> = if searching { registry().packs.iter().map(|p| p.id.clone()).collect() } else { self.library_packs(cx) };
        let hit = |category: &str, (stencil, name, detail): &(String, String, String)| {
            !searching || [name.as_str(), stencil.as_str(), category, detail.as_str()].iter().any(|f| f.to_lowercase().contains(query))
        };
        // Per notation: its shape categories and its containers.
        struct Notation {
            id: String,
            name: String,
            icon: Option<String>,
            cats: Vec<(String, Tiles)>,
            containers: Tiles,
        }
        let notations: Vec<Notation> = {
            let reg = registry();
            // In the scope's order (the diagram's notations before core).
            packs
                .iter()
                .filter_map(|id| reg.packs.iter().find(|p| &p.id == id))
                .map(|p| {
                    let mut cats: Vec<(String, Tiles)> = Vec::new();
                    if filter != Kinds::Containers {
                        for (c, list) in reg.catalog() {
                            let tiles: Tiles = list.into_iter().filter(|s| s.pack == p.id).map(|s| (s.id.clone(), s.title.clone(), s.id.clone())).filter(|t| hit(&c, t)).collect();
                            if !tiles.is_empty() {
                                cats.push((c, tiles));
                            }
                        }
                    }
                    let containers: Tiles = if filter == Kinds::Shapes {
                        Vec::new()
                    } else {
                        reg.group_kinds
                            .iter()
                            .filter(|g| g.pack == p.id)
                            .map(|g| (format!("group:{}", g.name), g.title.clone(), g.description.clone()))
                            .filter(|t| hit(&p.name, t))
                            .collect()
                    };
                    Notation { id: p.id.clone(), name: p.name.clone(), icon: p.icon.clone(), cats, containers }
                })
                .filter(|n| !n.cats.is_empty() || !n.containers.is_empty())
                .collect()
        };

        let any = !notations.is_empty();
        let mut out = div().flex().flex_col();
        // Your saved blocks first; they match searches by name too.
        let blocks: Vec<&crate::blocks::Block> = self.blocks.iter().filter(|b| filter != Kinds::Containers && (!searching || b.title.to_lowercase().contains(query))).collect();
        if !blocks.is_empty() {
            let collapsed = !searching && self.collapsed.contains("blocks");
            out = out.child(self.category_header("blocks", "My blocks", blocks.len(), collapsed, k, cx));
            if !collapsed {
                let tiles: Vec<(String, String, String, String)> = blocks
                    .iter()
                    .map(|b| (b.name.clone(), b.title.clone(), b.stencil.clone().unwrap_or_else(|| "rect".into()), if b.count == 1 { "Saved block".to_string() } else { format!("Saved block, {} shapes", b.count) }))
                    .collect();
                out = out.child(self.block_tiles(tiles, k, cx));
            }
        }
        if searching {
            let total: usize = notations.iter().map(|n| n.cats.iter().map(|c| c.1.len()).sum::<usize>() + n.containers.len()).sum();
            out = out.child(div().px(PANEL_PAD).pt(GAP_2).text_size(TEXT_XS).text_color(k.text_faint).child(format!("{total} found across all notations")));
        } else if !self.recent_stencils.is_empty() && filter == Kinds::All {
            let recent: Tiles = self.recent_stencils.iter().filter_map(|s| tile_for(s)).collect();
            if !recent.is_empty() {
                out = out.child(self.category_header("recent", "Recently used", recent.len(), false, k, cx)).child(self.shape_tiles("recent", recent, k, cx));
            }
        }
        let single = notations.len() == 1;
        for n in notations {
            let count = n.cats.iter().map(|c| c.1.len()).sum::<usize>() + n.containers.len();
            let pack_key = format!("pack:{}", n.id);
            let folded = !searching && !single && self.collapsed.contains(&pack_key);
            if !single {
                out = out.child(self.notation_header(&pack_key, &n.name, n.icon.as_deref(), count, folded, k, cx));
                if folded {
                    continue;
                }
            }
            let only_one_cat = n.cats.len() == 1 && n.containers.is_empty();
            for (category, tiles) in n.cats {
                let key = format!("{}/{category}", n.id);
                let shown = short_category(&category, &n.name);
                if !only_one_cat || shown != category {
                    let collapsed = !searching && self.collapsed.contains(&key);
                    out = out.child(self.category_header(&key, &shown, tiles.len(), collapsed, k, cx));
                    if collapsed {
                        continue;
                    }
                }
                out = out.child(self.shape_tiles(&key, tiles, k, cx));
            }
            if !n.containers.is_empty() {
                let key = format!("{}/containers", n.id);
                let collapsed = !searching && self.collapsed.contains(&key);
                out = out.child(self.category_header(&key, "Containers", n.containers.len(), collapsed, k, cx));
                if !collapsed {
                    out = out.child(self.container_rows(&key, n.containers, k, cx));
                }
            }
        }
        if !any {
            out = out.child(kit::empty_state(Lucide::SearchX, "No shapes match", "Try another word", cx));
        }
        out.into_any_element()
    }

    /// A notation's name, icon and size; click to fold it.
    #[allow(clippy::too_many_arguments)]
    fn notation_header(&self, key: &str, name: &str, icon: Option<&str>, count: usize, folded: bool, k: Colors, cx: &mut Context<Self>) -> AnyElement {
        let key = key.to_string();
        let icon = icon.map(kit::icon_named).unwrap_or_else(|| Lucide::Shapes.into());
        div()
            .id(SharedString::from(format!("notation-{key}")))
            .mt(GAP_1)
            .h(ROW_H + GAP_2)
            .px(PANEL_PAD)
            .flex()
            .items_center()
            .gap(GAP_2)
            .border_t_1()
            .border_color(k.border)
            .cursor_pointer()
            .hover(|d| d.bg(k.hover))
            .on_click(cx.listener(move |ws, _, _, cx| {
                if !ws.collapsed.remove(&key) {
                    ws.collapsed.insert(key.clone());
                }
                cx.notify();
            }))
            .child(icon.size(ICON_MD).text_color(k.accent))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(TEXT_MD).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.heading).child(name.to_string()))
            .child(div().text_size(TEXT_XS).text_color(k.text_faint).child(count.to_string()))
            .child(gpui_kit::component::Icon::new(if folded { Lucide::ChevronRight } else { Lucide::ChevronDown }).size(ICON_SM).text_color(k.text_faint))
            .into_any_element()
    }

    /// A collapsible category inside a notation.
    fn category_header(&self, key: &str, category: &str, count: usize, collapsed: bool, k: Colors, cx: &mut Context<Self>) -> AnyElement {
        let key = key.to_string();
        div()
            .id(SharedString::from(format!("cat-{key}")))
            .h(ROW_H)
            .px(PANEL_PAD)
            .flex()
            .items_center()
            .gap(GAP_1)
            .cursor_pointer()
            .hover(|d| d.bg(k.hover))
            .on_click(cx.listener(move |ws, _, _, cx| {
                if !ws.collapsed.remove(&key) {
                    ws.collapsed.insert(key.clone());
                }
                cx.notify();
            }))
            .child(gpui_kit::component::Icon::new(if collapsed { Lucide::ChevronRight } else { Lucide::ChevronDown }).size(ICON_SM).text_color(k.text_faint))
            .child(kit::caption(category.to_string(), cx))
            .child(div().flex_1())
            .child(div().text_size(TEXT_XS).text_color(k.text_faint).child(count.to_string()))
            .into_any_element()
    }

    /// Clickable, draggable stencil or container: adds it at the view's centre.
    fn tile_base(&self, id: String, stencil: &str, name: &str, detail: String, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let drag = StencilDrag { stencil: SharedString::from(stencil.to_string()), title: SharedString::from(name.to_string()) };
        let click = stencil.to_string();
        let ws = cx.entity().downgrade();
        let k = cx.ui();
        div()
            .id(SharedString::from(id))
            .rounded(ROUND_MD)
            .cursor_grab()
            .hover(|d| d.bg(k.hover))
            .active(|d| d.bg(k.active))
            .on_click(cx.listener(move |ws, _, window, cx| {
                let s = click.clone();
                ws.note_recent(&s);
                ws.with_view(cx, |v, cx| v.add_shape(&s, window, cx));
            }))
            .tooltip(kit::tip(name.to_string(), (!detail.is_empty()).then(|| detail.into())))
            .on_drag(drag, move |d: &StencilDrag, _, _, cx: &mut App| {
                let stencil = d.stencil.to_string();
                ws.update(cx, |ws, _| ws.note_recent(&stencil)).ok();
                cx.new(|_| DragGhost { title: d.title.clone(), stencil: d.stencil.clone() })
            })
    }

    /// Saved blocks: a click drops a copy at the centre, a drag where it
    /// lands; right-click to rename or delete.
    fn block_tiles(&self, tiles: Vec<(String, String, String, String)>, k: Colors, cx: &mut Context<Self>) -> AnyElement {
        let tiles: Vec<AnyElement> = tiles
            .into_iter()
            .enumerate()
            .map(|(i, (name, title, stencil, detail))| {
                let menu_for = name.clone();
                self.tile_base(format!("block-{i}"), &format!("block:{name}"), &title, detail, cx)
                    .w(gpui_kit::relative(1.0 / 3.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(GAP_1)
                    .py(GAP_2)
                    .child(shape_glyph(&stencil, k))
                    .child(div().w_full().px(GAP_0).text_center().text_size(TEXT_XS).line_height(TEXT_XS * 1.25).text_color(k.text_muted).line_clamp(2).child(title))
                    .on_mouse_down(gpui_kit::MouseButton::Right, cx.listener(move |ws, ev: &gpui_kit::MouseDownEvent, _, cx| {
                        ws.block_menu = Some((menu_for.clone(), ev.position));
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        div().flex().flex_wrap().px(PANEL_PAD).children(tiles).into_any_element()
    }

    /// Remember a stencil for the "Recently used" row.
    pub(crate) fn note_recent(&mut self, stencil: &str) {
        if stencil.starts_with("block:") {
            return;
        }
        self.recent_stencils.retain(|s| s != stencil);
        self.recent_stencils.insert(0, stencil.to_string());
        self.recent_stencils.truncate(RECENT_MAX);
    }

    /// Shapes: a grid of glyph tiles.
    fn shape_tiles(&self, key: &str, tiles: Tiles, k: Colors, cx: &mut Context<Self>) -> AnyElement {
        let tiles: Vec<AnyElement> = tiles
            .into_iter()
            .enumerate()
            .map(|(i, (stencil, name, detail))| {
                // Three to a row across the pane, however wide it is.
                self.tile_base(format!("tile-{key}-{i}"), &stencil, &name, detail, cx)
                    .w(gpui_kit::relative(1.0 / 3.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(GAP_1)
                    .py(GAP_2)
                    .child(shape_glyph(&stencil, k))
                    // Two lines before it clips: "Message start", not "Messag...".
                    .child(div().w_full().px(GAP_0).text_center().text_size(TEXT_XS).line_height(TEXT_XS * 1.25).text_color(k.text_muted).line_clamp(2).child(name))
                    .into_any_element()
            })
            .collect();
        div().flex().flex_wrap().px(GAP_2).pb(GAP_2).children(tiles).into_any_element()
    }

    /// Containers: rows with the frame's look, its name and what it is for,
    /// so they never pass for shapes.
    fn container_rows(&self, key: &str, tiles: Tiles, k: Colors, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<AnyElement> = tiles
            .into_iter()
            .enumerate()
            .map(|(i, (stencil, name, detail))| {
                let kind = stencil.trim_start_matches("group:").to_string();
                self.tile_base(format!("tile-{key}-{i}"), &stencil, &name, format!("Container \u{b7} kind: {kind}"), cx)
                    .flex()
                    .items_center()
                    .gap(GAP_3)
                    .px(GAP_2)
                    .py(GAP_1)
                    .child(div().flex_none().child(shape_glyph(&stencil, k)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_size(TEXT_SM).text_color(k.text).overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                            .when(!detail.is_empty(), |d| {
                                d.child(div().text_size(TEXT_XS).text_color(k.text_faint).overflow_hidden().whitespace_nowrap().text_ellipsis().child(detail))
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        div().flex().flex_col().gap(GAP_0).px(GAP_2).pb(GAP_2).children(rows).into_any_element()
    }

    fn outline(&mut self, query: &str, k: Colors, cx: &mut Context<Self>) -> AnyElement {
        let view = self.view().read(cx);
        let d = view.doc().diagram();
        let sel = view.selection().to_vec();
        let matches = |s: &str| query.is_empty() || s.to_lowercase().contains(query);
        let mut groups: Vec<(String, String, Lucide, Option<String>)> = Vec::new();
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for g in &d.groups {
            let text = g.label.clone().unwrap_or_else(|| g.id.clone());
            if matches(&text) || matches(&g.id) {
                groups.push((g.id.clone(), text, Lucide::Group, Some(g.members.len().to_string())));
            }
        }
        for n in &d.nodes {
            let text = n.text().replace('\n', " ");
            if matches(&text) || matches(&n.id) {
                nodes.push((n.id.clone(), text, shape_icon(Shape::from_stencil(n.stencil.as_deref())), Some(n.id.clone())));
            }
        }
        for e in &d.edges {
            let text = match &e.label {
                Some(l) => format!("{} \u{2192} {}  {l}", e.from, e.to),
                None => format!("{} \u{2192} {}", e.from, e.to),
            };
            if matches(&text) {
                edges.push((e.id.clone(), text, Lucide::Spline, None));
            }
        }
        if groups.is_empty() && nodes.is_empty() && edges.is_empty() {
            let (title, hint) = if query.is_empty() {
                ("Nothing here yet", "Double-click the canvas or drag a shape in")
            } else {
                ("No matches", "Try another word")
            };
            return kit::empty_state(Lucide::Shapes, title, hint, cx).into_any_element();
        }
        let mut out = div().flex().flex_col();
        let mut n = 0usize;
        for (title, list) in [("Groups", groups), ("Nodes", nodes), ("Connections", edges)] {
            if list.is_empty() {
                continue;
            }
            out = out.child(
                div()
                    .px(PANEL_PAD)
                    .pt(GAP_3)
                    .pb(GAP_1)
                    .flex()
                    .justify_between()
                    .child(kit::caption(title, cx))
                    .child(div().text_size(TEXT_XS).text_color(k.text_faint).child(list.len().to_string())),
            );
            let rows = list.into_iter().map(|(id, text, icon, meta)| {
                n += 1;
                let mut row = Row::new(("outline", n), text.clone()).icon(icon).tip(text, Some(id.clone().into())).selected(sel.contains(&id)).on_click(cx.listener(move |ws, _, _, cx| {
                    let id = id.clone();
                    ws.with_view(cx, |v, cx| v.reveal(&id, cx));
                }));
                if let Some(m) = meta {
                    row = row.meta(m);
                }
                row
            });
            out = out.child(div().px(GAP_2).flex().flex_col().gap(GAP_0).children(rows));
        }
        out.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::short_category;

    #[test]
    fn categories_drop_their_notations_name() {
        assert_eq!(short_category("ArchiMate business", "ArchiMate"), "Business");
        assert_eq!(short_category("UML class", "UML"), "Class");
        assert_eq!(short_category("Event storming", "Event storming"), "Event storming");
        assert_eq!(short_category("Flowchart", "Core"), "Flowchart");
    }
}
