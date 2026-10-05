//! Docking, VS Code style. Every diagram and every tool (shapes, outline,
//! inspector, source, problems) is a dock pane: drag a tab into another
//! group, onto an edge to split, or into the left, right or bottom dock. The
//! layout engine is gpui-component's dock; this file supplies the panes and
//! the skin (tabs drawn with `graphing_ui::dock`). The layout persists in
//! state.json.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::ResizeHandleContext;
use gpui_kit::component::Icon;
use gpui_kit::component::dock::{
    AnyDrag, BasePanel, BasePanelView, DockArea, DockAreaRenderer, DockContext, DockPlacement, DockSkin, DragPanel,
    DropIndicator, NodeId, PanelEvent, PanelId, PanelInfo, PanelState, TabGroupContext, TabGroupRenderer,
    register_panel,
};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Axis, Context, Div, Empty, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, size,
};
use graphing_ui::kit::Lucide;
use graphing_ui::tokens::*;
use graphing_ui::{UiExt, dock as chrome};

use crate::view::DiagramView;
use crate::workspace::Workspace;

/// Bumped when the persisted layout format changes; older layouts are dropped.
pub const LAYOUT_VERSION: usize = 1;

/// A tool panel. Each exists at most once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Shapes,
    Outline,
    Inspector,
    Source,
    Problems,
    Settings,
}

impl Tool {
    pub const ALL: [Tool; 6] = [Tool::Shapes, Tool::Outline, Tool::Inspector, Tool::Source, Tool::Problems, Tool::Settings];

    /// Persisted panel name.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Shapes => "shapes",
            Tool::Outline => "outline",
            Tool::Inspector => "inspector",
            Tool::Source => "source",
            Tool::Problems => "problems",
            Tool::Settings => "settings",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tool::Shapes => "Shapes",
            Tool::Outline => "Outline",
            Tool::Inspector => "Inspector",
            Tool::Source => "Source",
            Tool::Problems => "Problems",
            Tool::Settings => "Settings",
        }
    }

    pub fn icon(self) -> Lucide {
        match self {
            Tool::Shapes => Lucide::Shapes,
            Tool::Outline => Lucide::ListTree,
            Tool::Inspector => Lucide::SlidersHorizontal,
            Tool::Source => Lucide::Code,
            Tool::Problems => Lucide::TriangleAlert,
            Tool::Settings => Lucide::Settings,
        }
    }

    /// Where the tool goes when shown and it has no place yet.
    pub fn home(self) -> DockPlacement {
        match self {
            Tool::Shapes | Tool::Outline => DockPlacement::Left,
            Tool::Inspector => DockPlacement::Right,
            Tool::Source | Tool::Settings => DockPlacement::Center,
            Tool::Problems => DockPlacement::Bottom,
        }
    }
}

#[derive(Clone)]
pub enum PaneKind {
    Diagram(Entity<DiagramView>),
    Tool(Tool),
    /// A saved layout named something this run cannot rebuild (a diagram
    /// whose file is gone). Removed right after the layout loads.
    Missing,
}

/// One dockable pane. Its content is drawn by the workspace, which owns all
/// the state the panes show.
pub struct Pane {
    pub kind: PaneKind,
    ws: WeakEntity<Workspace>,
    focus: FocusHandle,
    /// Re-render when the workspace changes. Panes are cached, so a canvas
    /// redraw (a drag) leaves the other panes alone.
    _ws_sub: Option<gpui_kit::Subscription>,
}

impl Pane {
    pub fn new(kind: PaneKind, ws: WeakEntity<Workspace>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self { kind, ws, focus: cx.focus_handle(), _ws_sub: None })
    }

    pub fn id(pane: &Entity<Pane>) -> PanelId {
        PanelId::from(pane.entity_id())
    }

    /// Icon, title and dirty flag for its tab.
    fn label(&self, cx: &App) -> (Lucide, SharedString, bool) {
        match &self.kind {
            PaneKind::Diagram(v) => {
                let v = v.read(cx);
                (Lucide::Waypoints, v.title().into(), v.is_dirty())
            }
            PaneKind::Tool(t) => (t.icon(), t.title().into(), false),
            PaneKind::Missing => (Lucide::FileX, "Missing".into(), false),
        }
    }
}

impl EventEmitter<PanelEvent> for Pane {}

impl Focusable for Pane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.kind {
            PaneKind::Diagram(v) => v.read(cx).focus_handle().clone(),
            _ => self.focus.clone(),
        }
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The workspace may still be under construction when the pane is
        // made, so subscribe on first render.
        if self._ws_sub.is_none()
            && let Some(ws) = self.ws.upgrade()
        {
            self._ws_sub = Some(cx.observe(&ws, |_, _, cx| cx.notify()));
        }
        let kind = self.kind.clone();
        let started = std::time::Instant::now();
        let body = self.ws.update(cx, |ws, cx| ws.render_pane(&kind, window, cx)).ok();
        crate::trace(BasePanel::panel_name(self), started);
        div().size_full().track_focus(&self.focus).children(body)
    }
}

impl BasePanel for Pane {
    fn panel_name(&self) -> &'static str {
        match &self.kind {
            PaneKind::Diagram(_) => "diagram",
            PaneKind::Tool(t) => t.name(),
            PaneKind::Missing => "missing",
        }
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let (true, PaneKind::Diagram(v)) = (active, &self.kind) {
            let (ws, v) = (self.ws.clone(), v.clone());
            window.defer(cx, move |window, cx| {
                ws.update(cx, |ws, cx| ws.diagram_shown(&v, window, cx)).ok();
            });
        }
    }

    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.ws.clone();
        let kind = self.kind.clone();
        window.defer(cx, move |window, cx| {
            ws.update(cx, |ws, cx| ws.pane_removed(&kind, window, cx)).ok();
        });
    }

    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(self.panel_name());
        if let PaneKind::Diagram(v) = &self.kind {
            let path = v.read(cx).path().map(|p| p.display().to_string());
            state.info = PanelInfo::panel(serde_json::json!({ "path": path }));
        }
        state
    }
}

/// The key a pane is found under while a saved layout loads.
pub fn restore_key(state: &PanelState) -> String {
    match (state.panel_name.as_str(), &state.info) {
        ("diagram", PanelInfo::Panel(info)) => format!("diagram:{}", info.get("path").and_then(|p| p.as_str()).unwrap_or_default()),
        (name, _) => name.to_string(),
    }
}

/// Panes a loading layout may use, by [`restore_key`]. Builders registered
/// with the dock read it; anything not found becomes a `Missing` pane.
pub struct Restore {
    pub panes: HashMap<String, Entity<Pane>>,
    pub ws: WeakEntity<Workspace>,
}

thread_local! {
    static RESTORE: RefCell<Option<Restore>> = const { RefCell::new(None) };
}

/// Run `f` (a `DockArea::load`) with `restore` available to the builders.
pub fn with_restore<R>(restore: Restore, f: impl FnOnce() -> R) -> R {
    RESTORE.with(|r| *r.borrow_mut() = Some(restore));
    let out = f();
    RESTORE.with(|r| *r.borrow_mut() = None);
    out
}

/// Register the builders a saved layout is rebuilt through. Idempotent.
pub fn register(cx: &mut App) {
    for name in ["diagram", "missing"].into_iter().chain(Tool::ALL.map(Tool::name)) {
        register_panel(cx, name, |build, _, cx| {
            let key = restore_key(build.state());
            let (found, ws) = RESTORE.with(|r| {
                let r = r.borrow();
                let r = r.as_ref();
                (r.and_then(|r| r.panes.get(&key).cloned()), r.map(|r| r.ws.clone()).unwrap_or_else(WeakEntity::new_invalid))
            });
            let pane = found.unwrap_or_else(|| Pane::new(PaneKind::Missing, ws, cx));
            Arc::new(pane) as Arc<dyn BasePanelView>
        });
    }
}

/// The dock area, wearing the graphing skin.
pub fn area(ws: WeakEntity<Workspace>, window: &mut Window, cx: &mut App) -> Entity<DockArea> {
    cx.new(|cx| {
        let inner = DockSkin::new(cx);
        DockArea::new("graphing", Some(LAYOUT_VERSION), window, cx).with_renderer(Rc::new(Skin { inner, ws }))
    })
}

struct Skin {
    inner: Rc<DockSkin>,
    ws: WeakEntity<Workspace>,
}

impl DockAreaRenderer for Skin {
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.frame(window, cx).bg(cx.ui().border)
    }

    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.center_frame(window, cx)
    }

    fn split_frame(&self, node: NodeId, axis: Axis, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.split_frame(node, axis, window, cx).bg(cx.ui().border)
    }

    fn render_split_handle(&self, handle: &ResizeHandleContext, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.inner.render_split_handle(handle, window, cx)
    }

    fn render_dock(&self, dock: &DockContext, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        self.inner.render_dock(dock, content, window, cx)
    }

    fn build_placeholder(&self, _: &PanelState, _: &mut Window, cx: &mut App) -> Option<Arc<dyn BasePanelView>> {
        Some(Arc::new(Pane::new(PaneKind::Missing, self.ws.clone(), cx)))
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(Tabs { ws: self.ws.clone() })
    }
}

struct Tabs {
    ws: WeakEntity<Workspace>,
}

fn pane_of(panel: &Arc<dyn BasePanelView>) -> Option<Entity<Pane>> {
    panel.view().downcast::<Pane>().ok()
}

/// Follows the pointer while a tab is dragged.
struct TabPreview {
    icon: Lucide,
    title: SharedString,
}

impl Render for TabPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        chrome::drag_preview(Icon::new(self.icon), self.title.clone(), cx)
    }
}

fn is_diagram(pane: &Entity<Pane>, cx: &App) -> bool {
    matches!(pane.read(cx).kind, PaneKind::Diagram(_))
}

impl TabGroupRenderer for Tabs {
    fn frame(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        let k = cx.ui();
        let diagram = group.panels().iter().filter_map(pane_of).any(|p| is_diagram(&p, cx));
        div().id(("dock-group", group.node().as_u64())).bg(if diagram { k.bg } else { k.chrome })
    }

    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        let node = group.node().as_u64();
        let displayed = group.active_panel().map(|p| p.panel_id(cx));
        let collapsed = group.is_collapsed();
        let focused = self.ws.upgrade().and_then(|ws| ws.read(cx).active_pane_id(cx)).is_some_and(|id| group.panels().iter().any(|p| p.panel_id(cx) == id));
        let droppable = group.is_droppable();
        let count = group.panels().len();
        let mut bar = chrome::tab_bar(collapsed, cx).id(("dock-tabs", node)).overflow_x_scroll();
        for (ix, panel) in group.panels().iter().enumerate() {
            if !panel.visible(cx) {
                continue;
            }
            let Some(pane) = pane_of(panel) else { continue };
            let (icon, title, dirty) = pane.read(cx).label(cx);
            let pid = panel.panel_id(cx);
            let current = !collapsed && Some(pid) == displayed;
            let detail: Option<SharedString> = match &pane.read(cx).kind {
                PaneKind::Diagram(v) => Some(v.read(cx).path().map_or("Not saved yet".into(), |p| p.display().to_string().into())),
                _ => None,
            };
            let name: SharedString = format!("dock-tab-{node}-{ix}").into();
            let close = {
                let (group, ws, pane) = (group.clone(), self.ws.clone(), pane.clone());
                move |window: &mut Window, cx: &mut App| close_pane(&group, &ws, &pane, pid, window, cx)
            };
            let close_click = close.clone();
            let mut tab = chrome::tab(name.clone(), name.clone(), Icon::new(icon), title.clone(), current, focused, cx)
                .on_click({
                    let (group, ws) = (group.clone(), self.ws.clone());
                    move |_, window, cx| {
                        group.select_tab(ix, window, cx);
                        // A closed dock's strip is the way back in.
                        if collapsed {
                            ws.update(cx, |ws, cx| ws.open_dock_of(pid, window, cx)).ok();
                        }
                    }
                })
                .tooltip(graphing_ui::kit::tip(title.clone(), detail))
                .on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                    cx.stop_propagation();
                    close(window, cx);
                })
                .child(chrome::tab_close((name.clone(), 1usize), name.clone(), dirty, current, cx).on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    close_click(window, cx);
                }));
            if !collapsed
                && group.is_draggable()
                && let Some(drag) = group.drag_panel(ix, cx)
            {
                tab = tab.on_drag(drag, move |drag: &DragPanel, offset, _, cx| {
                    cx.stop_propagation();
                    drag.set_drag_offset(offset);
                    drag.set_preview_size(size(DOCK_TAB_MAX_W, DOCK_TAB_H));
                    cx.new(|_| TabPreview { icon, title: title.clone() })
                });
            }
            if droppable {
                let tint = chrome::drop_tint(cx);
                tab = tab
                    .drag_over::<DragPanel>(move |s, _, _, _| s.bg(tint))
                    .on_drop({
                        let group = group.clone();
                        move |drag: &DragPanel, window, cx| group.drop_panel(drag.clone(), Some(ix), true, window, cx)
                    })
                    .drag_over::<AnyDrag>(move |s, _, _, _| s.bg(tint))
                    .on_drop({
                        let group = group.clone();
                        move |item: &AnyDrag, window, cx| group.drop_item(item.clone(), None, window, cx)
                    });
            }
            bar = bar.child(tab);
        }
        let mut filler = chrome::tab_filler(("dock-tab-filler", node));
        if droppable {
            let tint = chrome::drop_tint(cx);
            filler = filler
                .drag_over::<DragPanel>(move |s, _, _, _| s.bg(tint))
                .on_drop({
                    let group = group.clone();
                    let node = group.node();
                    move |drag: &DragPanel, window, cx| {
                        // Past its own last tab: the last slot; from elsewhere: appended.
                        let ix = (drag.source() == node).then(|| count.saturating_sub(1));
                        group.drop_panel(drag.clone(), ix, true, window, cx);
                    }
                })
                .drag_over::<AnyDrag>(move |s, _, _, _| s.bg(tint))
                .on_drop({
                    let group = group.clone();
                    move |item: &AnyDrag, window, cx| group.drop_item(item.clone(), None, window, cx)
                });
        }
        bar.child(filler).into_any_element()
    }

    fn render_active_panel(&self, panel: AnyView, group: &TabGroupContext, _: &mut Window, _: &mut App) -> AnyElement {
        if group.is_collapsed() {
            return Empty.into_any_element();
        }
        div().flex_1().min_h_0().relative().child(panel.cached(gpui_kit::StyleRefinement::default().absolute().size_full())).into_any_element()
    }

    fn render_drop_indicator(&self, indicator: DropIndicator, _: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let to = indicator.to();
        let (o, s) = (to.origin(), to.size());
        Some(chrome::drop_zone(cx).left(o.x).top(o.y).w(s.width).h(s.height).into_any_element())
    }
}

/// Diagrams close through the workspace (it asks about unsaved changes);
/// tools just leave the dock.
fn close_pane(group: &TabGroupContext, ws: &WeakEntity<Workspace>, pane: &Entity<Pane>, pid: PanelId, window: &mut Window, cx: &mut App) {
    if let PaneKind::Diagram(v) = &pane.read(cx).kind {
        let v = v.clone();
        ws.update(cx, |ws, cx| ws.close_view(&v, window, cx)).ok();
    } else {
        group.close(pid, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext};

    fn open(cx: &mut TestAppContext, files: Vec<std::path::PathBuf>) -> (Entity<Workspace>, &mut VisualTestContext) {
        crate::test_support::workspace(cx, files)
    }

    #[gpui_kit::test]
    fn saving_an_empty_diagram_after_a_cancelled_save(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Vec::new());
        let view = ws.read_with(cx, |w, _| w.view().clone());
        cx.update(|window, cx| {
            let f = view.read(cx).focus_handle().clone();
            window.focus(&f, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("secondary-s");
        cx.run_until_parked();
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        cx.simulate_keystrokes("secondary-a");
        cx.run_until_parked();
        cx.simulate_keystrokes("secondary-s");
        cx.run_until_parked();
        let path = std::env::temp_dir().join(format!("graphing-empty-save-{}.gph", std::process::id()));
        let to = path.clone();
        cx.simulate_new_path_selection(move |_| Some(to));
        cx.run_until_parked();
        assert!(path.exists());
        std::fs::remove_file(&path).ok();
    }

    #[gpui_kit::test]
    fn dropped_diagrams_open_insert_or_link(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("graphing-drop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (host, other) = (dir.join("host.gph"), dir.join("auth.gph"));
        std::fs::write(&host, "a\n").unwrap();
        std::fs::write(&other, "diagram \"Auth\"\nlogin\ncheck\nlogin -> check\n").unwrap();
        let (ws, cx) = open(cx, vec![host.clone()]);
        let at = graphing_model::Point::new(400.0, 300.0);
        // The dialog offers all three for a graphing file.
        ws.update_in(cx, |ws, window, cx| {
            let view = ws.view().clone();
            ws.diagrams_dropped(view, vec![other.clone()], at, window, cx);
            let titles: Vec<String> = ws.confirm.as_ref().unwrap().choices.iter().map(|c| c.title.to_string()).collect();
            assert_eq!(titles, ["Open in a new tab", "Insert it here", "Link to it"]);
            ws.confirm = None;
        });
        // Link: a card pointing at the file, relative to the host's folder.
        ws.update(cx, |ws, cx| ws.view().update(cx, |v, cx| v.add_link(&other, at, cx)));
        ws.update(cx, |ws, cx| {
            let v = ws.view().read(cx);
            assert!(v.doc().source().contains("auth_link: ref { src: \"auth.gph\" }"), "{}", v.doc().source());
            let scene = v.scene();
            let target = v.link_target(&scene, "auth_link").expect("resolves");
            assert_eq!(target.canonicalize().unwrap(), other.canonicalize().unwrap());
        });
        // Insert: its shapes come in, wrapped in a group named after it.
        ws.update_in(cx, |ws, window, cx| {
            let view = ws.view().clone();
            ws.insert_diagram(&view, &other, at, window, cx);
            let d = view.read(cx).doc().diagram();
            // `auth_link` already uses the `auth` prefix, so this copy is `auth2`.
            assert!(d.node("auth2_login").is_some() && d.edge("auth2_login->auth2_check").is_some(), "{}", view.read(cx).doc().source());
            assert_eq!(d.group("auth2").and_then(|g| g.label.as_deref()), Some("Auth"));
        });
        // Open: a tab of its own.
        ws.update_in(cx, |ws, window, cx| {
            ws.open_or_import(other.clone(), window, cx);
            assert_eq!(ws.view().read(cx).path().cloned(), Some(other.canonicalize().unwrap()));
        });
    }

    #[gpui_kit::test]
    fn a_package_carries_its_links_along(cx: &mut TestAppContext) {
        let base = std::env::temp_dir().join(format!("graphing-pkglink-test-{}", std::process::id()));
        let (here, there) = (base.join("here"), base.join("there"));
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(&there).unwrap();
        std::fs::write(here.join("auth.gph"), "diagram \"Auth\"\nlogin\n").unwrap();
        let host = here.join("map.gphz");
        let (ws, cx) = open(cx, Vec::new());
        // Save a diagram with a link as a package beside the linked file.
        ws.update(cx, |ws, cx| {
            ws.view().update(cx, |v, cx| {
                v.apply(graphing_model::Op::AddNode { node: graphing_model::Node { id: "auth".into(), stencil: Some("ref".into()), label: None, classes: Vec::new(), props: vec![("src".into(), graphing_model::Value::Str("auth.gph".into()))] }, index: 0 }, cx);
                v.save_to(host.clone(), cx).unwrap();
            })
        });
        // Take only the package elsewhere: the link still shows Auth.
        let moved = there.join("map.gphz");
        std::fs::copy(&host, &moved).unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_path(moved.clone(), window, cx));
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            let v = ws.view().read(cx);
            let linked = v.linked.borrow();
            let auth = linked.get("auth.gph").and_then(|(_, l)| l.clone()).expect("shown from the package's copy");
            assert_eq!(auth.title, "Auth");
        });
    }

    #[gpui_kit::test]
    fn quitting_with_unsaved_changes_asks_first(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Vec::new());
        ws.update_in(cx, |ws, window, cx| {
            ws.view().update(cx, |v, cx| {
                v.apply(graphing_model::Op::AddNode { node: graphing_model::Node::new("x"), index: 0 }, cx);
            });
            assert_eq!(ws.dirty_tabs(cx).len(), 1);
            ws.request_quit(window, cx);
            let c = ws.confirm.as_ref().expect("asks before quitting");
            assert!(c.title.contains("before quitting"), "{}", c.title);
            assert_eq!(c.buttons.iter().map(|b| b.label.to_string()).collect::<Vec<_>>(), ["Quit without saving", "Save all and quit"]);
            assert!(!ws.quitting);
        });
    }

    #[gpui_kit::test]
    fn a_file_changed_under_unsaved_edits_asks_which_to_keep(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("graphing-reload-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.gph");
        std::fs::write(&path, "a\n").unwrap();
        let (ws, cx) = open(cx, vec![path.clone()]);
        ws.update(cx, |ws, cx| {
            ws.view().update(cx, |v, cx| {
                v.apply(graphing_model::Op::AddNode { node: graphing_model::Node::new("mine"), index: 0 }, cx);
            });
        });
        // Someone else saves; a second later so the time differs.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&path, "theirs\n").unwrap();
        ws.update(cx, |ws, cx| ws.view().update(cx, |v, cx| v.check_disk(cx)));
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            let c = ws.confirm.as_ref().expect("asks which to keep");
            assert_eq!(c.cancel.as_deref(), Some("Keep my edits"));
            assert!(ws.view().read(cx).doc().source().contains("mine"), "nothing changes until answered");
        });
        // Reloading takes theirs and drops the edits.
        ws.update(cx, |ws, cx| ws.view().update(cx, |v, cx| v.reload_from_disk(cx)));
        ws.update(cx, |ws, cx| {
            let v = ws.view().read(cx);
            // Theirs, placed like any opened file; the edit is gone.
            assert!(v.doc().source().starts_with("theirs\n") && !v.doc().source().contains("mine"), "{}", v.doc().source());
        });
    }

    #[gpui_kit::test]
    fn shapes_pane_lists_the_diagrams_notations_first(cx: &mut TestAppContext) {
        let path = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/notations/bpmn.gph"));
        let (ws, cx) = open(cx, vec![path]);
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.library_packs(cx), ["bpmn", "core"], "the diagram's notation, then core");
            ws.library_scope = "*".into();
            assert!(ws.library_packs(cx).len() >= 14);
            ws.library_scope = "fta".into();
            assert_eq!(ws.library_packs(cx), ["fta"]);
            ws.note_recent("bpmn.task");
            ws.note_recent("group:pool");
            ws.note_recent("bpmn.task");
            assert_eq!(ws.recent_stencils, ["bpmn.task", "group:pool"]);
        });
    }

    #[gpui_kit::test]
    fn default_layout_and_tool_toggles(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Vec::new());
        let shown = |ws: &Entity<Workspace>, cx: &mut VisualTestContext, t| ws.read_with(cx, |w, cx| w.shown(t, cx));
        assert!(shown(&ws, cx, Tool::Shapes) && shown(&ws, cx, Tool::Inspector));
        assert!(!shown(&ws, cx, Tool::Problems) && !shown(&ws, cx, Tool::Source));

        cx.update(|window, cx| ws.update(cx, |w, cx| w.toggle_tool(Tool::Source, window, cx)));
        cx.run_until_parked();
        assert!(shown(&ws, cx, Tool::Source));
        // Beside the diagrams, not as another tab in their group.
        let (diagram_node, source_node) = ws.read_with(cx, |w, cx| {
            let d = w.dock.read(cx);
            let tree = d.layout(DockPlacement::Center).unwrap();
            (tree.find_panel_node(w.active_pane_id(cx).unwrap()), tree.find_panel_node(Pane::id(&w.tools[&Tool::Source])))
        });
        assert!(source_node.is_some() && source_node != diagram_node);

        cx.update(|window, cx| ws.update(cx, |w, cx| w.toggle_tool(Tool::Problems, window, cx)));
        cx.update(|window, cx| ws.update(cx, |w, cx| w.toggle_tool(Tool::Source, window, cx)));
        cx.run_until_parked();
        assert!(shown(&ws, cx, Tool::Problems) && !shown(&ws, cx, Tool::Source));
    }

    #[gpui_kit::test]
    fn diagrams_open_and_close_as_dock_tabs(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Vec::new());
        let count = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| {
            ws.read_with(cx, |w, cx| w.dock.read(cx).layout(DockPlacement::Center).map_or(0, |t| t.panels().count()))
        };
        assert_eq!(count(&ws, cx), 1);
        cx.update(|window, cx| ws.update(cx, |w, cx| w.open_path(std::env::temp_dir().join("graphing-dock-a.gph"), window, cx)));
        cx.update(|window, cx| ws.update(cx, |w, cx| w.open_path(std::env::temp_dir().join("graphing-dock-b.gph"), window, cx)));
        cx.run_until_parked();
        // The pristine untitled tab was replaced by the first file.
        assert_eq!(count(&ws, cx), 2);
        let view = ws.read_with(cx, |w, _| w.view().clone());
        cx.update(|window, cx| ws.update(cx, |w, cx| w.close_view(&view, window, cx)));
        cx.run_until_parked();
        assert_eq!(count(&ws, cx), 1);
    }

    #[gpui_kit::test]
    fn layout_round_trips_through_its_saved_state(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Vec::new());
        cx.update(|window, cx| ws.update(cx, |w, cx| {
            w.toggle_tool(Tool::Source, window, cx);
            w.toggle_tool(Tool::Outline, window, cx);
        }));
        cx.run_until_parked();
        let saved = ws.read_with(cx, |w, cx| w.dock.read(cx).dump(cx));
        cx.update(|window, cx| ws.update(cx, |w, cx| w.default_layout(window, cx)));
        cx.run_until_parked();
        assert!(ws.read_with(cx, |w, cx| !w.shown(Tool::Source, cx) && w.shown(Tool::Outline, cx)));
        let ok = cx.update(|window, cx| ws.update(cx, |w, cx| w.restore_layout(saved, window, cx)));
        cx.run_until_parked();
        assert!(ok);
        assert!(ws.read_with(cx, |w, cx| w.shown(Tool::Source, cx) && !w.shown(Tool::Outline, cx)));
    }

    #[gpui_kit::test]
    fn clicking_a_group_in_the_workspace_inspects_it(cx: &mut TestAppContext) {
        let file = std::env::temp_dir().join(format!("graphing-dock-groups-{}.gph", std::process::id()));
        std::fs::write(&file, "a\nb\ngroup g1 \"A\" { a }\nlayout {\n  a 40 60\n  b 640 60\n  g1 0 0 300x200\n}\n").unwrap();
        let (ws, cx) = open(cx, vec![file.clone()]);
        let view = ws.read_with(cx, |w, _| w.view().clone());
        let p = view.read_with(cx, |v, _| v.screen_point(graphing_model::Point::new(220.0, 170.0)));
        cx.simulate_mouse_move(p, None, gpui_kit::Modifiers::default());
        cx.simulate_click(p, gpui_kit::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selection().to_vec()), ["g1"]);
        let title = view.read_with(cx, |v, _| v.screen_point(graphing_model::Point::new(60.0, 8.0)));
        cx.simulate_click(title, gpui_kit::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selection().to_vec()), ["g1"]);
        let _ = std::fs::remove_file(&file);
    }

    #[gpui_kit::test]
    fn files_from_the_command_line_survive_a_saved_layout(cx: &mut TestAppContext) {
        // A saved layout that knows a different file.
        let (ws, cx) = open(cx, vec![std::env::temp_dir().join("graphing-dock-old.gph")]);
        let saved = ws.read_with(cx, |w, cx| w.dock.read(cx).dump(cx));
        let files = ["graphing-dock-new1.gph", "graphing-dock-new2.gph"].map(|f| std::env::temp_dir().join(f));
        for f in &files {
            std::fs::write(f, "a -> b\n").unwrap();
        }
        cx.update(|window, cx| {
            ws.update(cx, |w, cx| {
                for f in &files {
                    w.open_path(f.clone(), window, cx);
                }
                w.restore_layout(saved, window, cx);
            })
        });
        cx.run_until_parked();
        let titles = ws.read_with(cx, |w, cx| w.views().iter().map(|v| v.read(cx).title()).collect::<Vec<_>>());
        assert!(titles.iter().any(|t| t.contains("new1")) && titles.iter().any(|t| t.contains("new2")), "{titles:?}");
        let in_dock = ws.read_with(cx, |w, cx| w.dock.read(cx).layout(DockPlacement::Center).map_or(0, |t| t.panels().count()));
        assert!(in_dock >= 2);
    }

    #[gpui_kit::test]
    fn bottom_dock_reopens_at_a_usable_height(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Vec::new());
        let size = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| ws.read_with(cx, |w, cx| w.dock.read(cx).dock_size(DockPlacement::Bottom));
        cx.update(|window, cx| ws.update(cx, |w, cx| w.toggle_tool(Tool::Problems, window, cx)));
        assert_eq!(size(&ws, cx), Some(BOTTOM_DOCK_H));
        // Dragged down to the floor, then folded away.
        cx.update(|window, cx| ws.update(cx, |w, cx| {
            w.dock.update(cx, |d, cx| d.set_dock_size(DockPlacement::Bottom, gpui_kit::px(100.0), window, cx));
            w.toggle_tool(Tool::Problems, window, cx);
        }));
        assert!(ws.read_with(cx, |w, cx| !w.shown(Tool::Problems, cx) && w.placement_of_tool(Tool::Problems, cx).is_some()));
        cx.update(|window, cx| ws.update(cx, |w, cx| w.toggle_tool(Tool::Problems, window, cx)));
        assert_eq!(size(&ws, cx), Some(BOTTOM_DOCK_H));
        assert!(ws.read_with(cx, |w, cx| w.shown(Tool::Problems, cx)));
    }

    #[gpui_kit::test]
    fn settings_tab_records_a_shortcut(cx: &mut TestAppContext) {
        let (ws, cx) = open(cx, Vec::new());
        cx.update(|window, cx| ws.update(cx, |w, cx| w.show_tool(Tool::Settings, window, cx)));
        cx.run_until_parked();
        assert!(ws.read_with(cx, |w, cx| w.shown(Tool::Settings, cx)));
        cx.update(|window, cx| {
            ws.update(cx, |w, cx| {
                w.recording = Some("graphing::NextTab".into());
                window.focus(&w.settings_focus.clone(), cx);
            })
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("alt-l");
        let (rec, eff) = ws.read_with(cx, |w, _| (w.recording.clone(), crate::keymap::effective(&w.settings.keybindings)));
        assert_eq!(rec, None);
        let keys: Vec<_> = eff.iter().filter(|e| e.action == "graphing::NextTab").map(|e| e.keys.clone()).collect();
        assert_eq!(keys, ["alt-l"]);
    }

    /// Timing probe for dragging a group through the whole workspace. Run
    /// with `cargo test -p graphing-app drag_timing -- --ignored --nocapture`.
    #[gpui_kit::test]
    #[ignore]
    fn drag_timing(cx: &mut TestAppContext) {
        let file = std::env::temp_dir().join("graphing-drag-timing.gph");
        std::fs::write(&file, include_str!("testdata/bench_infra.gph")).unwrap();
        let (ws, cx) = open(cx, vec![file]);
        let view = ws.read_with(cx, |w, _| w.view().clone());
        let p = |x: f64, y: f64, cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.screen_point(graphing_model::Point::new(x, y)));
        for (what, x, y) in [("node", 120.0, 330.0), ("vpc group", 60.0, 320.0)] {
            let start = p(x, y, cx);
            cx.simulate_mouse_move(start, None, gpui_kit::Modifiers::default());
            cx.simulate_mouse_down(start, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
            let t = std::time::Instant::now();
            let steps = 60;
            for i in 1..=steps {
                let at = gpui_kit::point(start.x + gpui_kit::px(i as f32 * 4.0), start.y);
                cx.simulate_mouse_move(at, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
                cx.run_until_parked();
            }
            let el = t.elapsed();
            let end = gpui_kit::point(start.x + gpui_kit::px(steps as f32 * 4.0), start.y);
            cx.simulate_mouse_up(end, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
            cx.simulate_keystrokes("secondary-z");
            println!("{what}: {:?} per move", el / steps);
        }
    }

    #[gpui_kit::test]
    fn deleting_a_group_asks_first(cx: &mut TestAppContext) {
        let file = std::env::temp_dir().join(format!("graphing-confirm-{}.gph", std::process::id()));
        std::fs::write(&file, "a\nb\ngroup g \"Backend\" { a b }\nlayout {\n  a 0 0\n  b 200 0\n}\n").unwrap();
        let (ws, cx) = open(cx, vec![file.clone()]);
        let view = ws.read_with(cx, |w, _| w.view().clone());
        cx.update(|window, cx| {
            view.update(cx, |v, cx| v.select(vec!["g".into()], cx));
            let f = view.read(cx).focus_handle().clone();
            window.focus(&f, cx);
        });
        cx.simulate_keystrokes("delete");
        let title = ws.read_with(cx, |w, _| w.confirm.as_ref().map(|c| c.title.to_string()));
        assert_eq!(title.as_deref(), Some("Delete group \"Backend\"?"));
        assert!(view.read_with(cx, |v, _| v.doc().diagram().group("g").is_some()), "nothing deleted before answering");
        // Escape cancels; asking again and Enter deletes.
        cx.simulate_keystrokes("escape");
        assert!(ws.read_with(cx, |w, _| w.confirm.is_none()));
        cx.update(|window, cx| {
            let f = view.read(cx).focus_handle().clone();
            window.focus(&f, cx);
        });
        cx.simulate_keystrokes("delete");
        cx.simulate_keystrokes("enter");
        let d = view.read_with(cx, |v, _| v.doc().diagram().clone());
        assert!(d.group("g").is_none() && d.nodes.is_empty(), "{:?}", d);
        let _ = std::fs::remove_file(&file);
    }
}
