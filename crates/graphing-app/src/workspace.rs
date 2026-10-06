//! Window root: custom titlebar with menus, the dock (diagram tabs and the
//! shapes, outline, inspector, source and problems panes), status bar,
//! command palette.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::component::input::{Editor, EditorState, InputEvent, InputState};
use gpui_kit::base::Placement;
use gpui_kit::component::dock::{DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, InsertTarget, PanelId};
use gpui_kit::component::{IconName, TitleBar};
use gpui_kit::{
    Action, AnyElement, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, WindowAppearance, canvas, div, prelude::FluentBuilder,
};
use graphing_ui::kit::{self, IconButton, Lucide};
use graphing_ui::tokens::*;
use graphing_ui::{Colors, UiExt};

use crate::dock::{Pane, PaneKind, Restore, Tool};
use crate::ops::Align;
use crate::palette::{Command, Palette, PaletteEvent};
use crate::settings::{Settings, State, ThemeChoice};
use crate::view::DiagramView;
use crate::*;
use graphing_dsl::Document;
use graphing_model::Op;
use graphing_scene::Shape;

mod debug;


struct Tab {
    view: Entity<DiagramView>,
    pane: Entity<Pane>,
    editor: Entity<EditorState>,
    /// Source text last pushed between canvas and editor, to break loops.
    synced: String,
    /// The view's last status message, so each new one toasts once.
    last_status: SharedString,
    /// Selection and dirty flag last seen, to skip redraws that change nothing.
    last_sel: Vec<String>,
    last_dirty: bool,
    _subs: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Shapes,
    Outline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Properties,
    Style,
    Arrange,
}

impl RightTab {
    pub fn title(self) -> &'static str {
        match self {
            RightTab::Properties => "Properties",
            RightTab::Style => "Style",
            RightTab::Arrange => "Arrange",
        }
    }
}

pub struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    focus: FocusHandle,
    pub(crate) dock: Entity<DockArea>,
    /// Tool panes, created on first use; whether one shows is the dock's call.
    pub(crate) tools: HashMap<Tool, Entity<Pane>>,
    /// Debounces layout writes while a split or dock is dragged.
    layout_gen: usize,
    /// Messages floating over the corner, newest last, each with its id.
    pub(crate) notices: Vec<crate::notices::Notice>,
    pub(crate) notice_seq: usize,
    pub(crate) theme: ThemeChoice,
    pub(crate) settings: Settings,
    /// Settings tab: shortcut search, the action being rebound, key focus.
    pub(crate) settings_query: Entity<InputState>,
    pub(crate) recording: Option<String>,
    pub(crate) settings_focus: FocusHandle,
    pub(crate) settings_section: crate::settings_pane::Section,
    /// The Open Source page's rows (built only while on screen), and the
    /// crates it was last filled with.
    pub(crate) oss_list: gpui_kit::ListState,
    pub(crate) oss_shown: Vec<usize>,
    /// The crate expanded in Settings > Open Source, by index.
    pub(crate) oss_open: Option<usize>,
    /// Saved blocks, for the Shapes pane.
    pub(crate) blocks: Vec<crate::blocks::Block>,
    /// The sequence strip's export menu, open at this point.
    pub(crate) export_menu: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    /// A block tile's right-click menu: block name and where.
    pub(crate) block_menu: Option<(String, gpui_kit::Point<gpui_kit::Pixels>)>,
    /// The pin open in the inspector's pin editor: node, side, name.
    pub(crate) pin_open: Option<(String, graphing_scene::pins::PinDir, String)>,
    /// The custom color picker while a color field is open.
    pub(crate) color_edit: Option<crate::color::ColorEdit>,
    /// The confirmation dialog being shown, if any.
    pub(crate) confirm: Option<crate::confirm::Confirm>,
    /// Quitting was confirmed (or nothing was unsaved): let the window go.
    pub(crate) quitting: bool,
    palette: Option<Entity<Palette>>,
    /// Selection the inspector last showed; fields reset when it changes.
    inspected: Option<Vec<String>>,
    pub(crate) recent: Vec<PathBuf>,
    pub(crate) menu: crate::menubar::MenuState,
    pub(crate) right_tab: RightTab,
    pub(crate) library_query: Entity<InputState>,
    /// Collapsed library categories.
    pub(crate) collapsed: HashSet<String>,
    /// Shapes pane: shapes, containers or both.
    pub(crate) library_filter: crate::library::Kinds,
    /// Shapes pane notation: empty for the diagram's own, `*` for all, or a pack id.
    pub(crate) library_scope: String,
    /// Stencils added lately, newest first.
    pub(crate) recent_stencils: Vec<String>,
    /// Sequence strip shown even before the diagram has steps.
    pub(crate) sequence_open: bool,
    /// A step title being edited in its chip, and the step menu (index, where).
    pub(crate) step_edit: Option<crate::sequence::StepEdit>,
    pub(crate) step_menu: Option<(usize, gpui_kit::Point<gpui_kit::Pixels>)>,
    /// The template picker, open with its keyboard focus.
    pub(crate) templates: Option<FocusHandle>,
    /// Inspector inputs by `target\x1fkey`.
    pub(crate) fields: HashMap<String, crate::inspector::Field>,
    /// Text props being added (shown with an empty field until committed).
    pub(crate) adding: HashSet<String>,
    /// Palette commands run most recently, newest first.
    recent_commands: Vec<SharedString>,
    /// The inspector select that is open, if any.
    pub(crate) select: Option<crate::inspector::OpenSelect>,
    pub(crate) plugins: Vec<crate::plugins::Running>,
    /// The plugin message poll loop is running.
    pub(crate) polling: bool,
    _subs: Vec<Subscription>,
}

impl Workspace {
    pub fn new(files: Vec<PathBuf>, settings: Settings, errors: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Tests run in parallel against one config dir; never restore there.
        let state = if cfg!(test) { State::default() } else { State::load() };
        let library_query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let sub = cx.subscribe_in(&library_query, window, |_: &mut Self, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        let appearance = cx.observe_window_appearance(window, |ws: &mut Self, window, cx| {
            if ws.theme == ThemeChoice::System {
                ws.apply_theme(window, cx);
            }
        });
        crate::dock::register(cx);
        let dock = crate::dock::area(cx.weak_entity(), window, cx);
        let dock_sub = cx.subscribe_in(&dock, window, |ws: &mut Self, _, ev: &DockEvent, window, cx| {
            if matches!(ev, DockEvent::LayoutChanged) {
                ws.schedule_layout_save(window, cx);
            }
        });
        let settings_query = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings and shortcuts"));
        let query_sub = cx.subscribe_in(&settings_query, window, |_: &mut Self, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut ws = Self {
            tabs: Vec::new(),
            active: 0,
            focus: cx.focus_handle(),
            dock,
            tools: HashMap::new(),
            layout_gen: 0,
            notices: Vec::new(),
            notice_seq: 0,
            theme: settings.theme,
            settings_query,
            recording: None,
            settings_focus: cx.focus_handle(),
            settings_section: crate::settings_pane::Section::Appearance,
            oss_list: gpui_kit::ListState::new(0, gpui_kit::ListAlignment::Top, ROW_H * 10.0),
            oss_shown: Vec::new(),
            oss_open: None,
            pin_open: None,
            blocks: crate::blocks::list(),
            export_menu: None,
            block_menu: None,
            color_edit: None,
            confirm: None,
            quitting: false,
            settings,
            palette: None,
            inspected: None,
            recent: state.recent.clone(),
            menu: Default::default(),
            right_tab: RightTab::Properties,
            library_query,
            collapsed: HashSet::new(),
            library_filter: Default::default(),
            library_scope: String::new(),
            recent_stencils: Vec::new(),
            sequence_open: false,
            step_edit: None,
            step_menu: None,
            templates: None,
            fields: HashMap::new(),
            adding: HashSet::new(),
            recent_commands: Vec::new(),
            select: None,
            plugins: Vec::new(),
            polling: false,
            _subs: vec![sub, appearance, dock_sub, query_sub],
        };
        ws.apply_theme(window, cx);
        // The window manager's close (Alt+F4, the title bar on other
        // platforms) asks about unsaved changes too.
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |ws, cx| {
                if ws.quitting || ws.dirty_tabs(cx).is_empty() {
                    return true;
                }
                ws.request_quit(window, cx);
                false
            })
            .unwrap_or(true)
        });
        let restoring = files.is_empty();
        let files = if restoring { state.files.into_iter().filter(|p| p.exists()).collect() } else { files };
        for f in files {
            ws.open_path(f, window, cx);
        }
        if ws.tabs.is_empty() {
            ws.new_tab(Document::parse(""), None, window, cx);
        }
        if restoring {
            ws.active = state.active.min(ws.tabs.len() - 1);
        }
        let saved = state.dock.clone().and_then(|v| serde_json::from_value::<DockAreaState>(v).ok()).filter(|s| s.version == Some(crate::dock::LAYOUT_VERSION));
        if !saved.is_some_and(|s| ws.restore_layout(s, window, cx)) {
            ws.default_layout(window, cx);
        }
        let pid = Pane::id(&ws.tabs[ws.active].pane);
        ws.dock.update(cx, |d, cx| d.select_panel(pid, window, cx));
        if !errors.is_empty() {
            ws.toast(format!("settings: {}", errors.join("; ")), window, cx);
        }
        ws.focus_canvas(window, cx);
        ws.start_plugins(window, cx);
        ws.debug_open(window, cx);
        ws
    }

    /// Every open tab's canvas.
    pub(crate) fn views(&self) -> Vec<Entity<DiagramView>> {
        self.tabs.iter().map(|t| t.view.clone()).collect()
    }

    pub(crate) fn view(&self) -> &Entity<DiagramView> {
        &self.tabs[self.active].view
    }

    pub(crate) fn focus_canvas(&self, window: &mut Window, cx: &mut App) {
        let f = self.view().read(cx).focus_handle().clone();
        window.focus(&f, cx);
    }

    #[cfg(test)]
    pub(crate) fn editor_at(&self, i: usize) -> Entity<EditorState> {
        self.tabs[i].editor.clone()
    }

    // ---- tabs ----

    pub(crate) fn new_tab(&mut self, doc: Document, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let src = doc.source().to_string();
        let editor = cx.new(|cx| {
            let mut e = EditorState::new(window, cx).line_number(true);
            source::install(&mut e, cx);
            e.set_value(src.clone(), window, cx);
            source::set_diagnostics(&mut e, &doc, cx);
            e
        });
        let view = cx.new(|cx| DiagramView::new(doc, path, window, cx));
        let ed_sub = cx.subscribe_in(&editor, window, |ws: &mut Self, editor, ev: &InputEvent, _, cx| {
            if !matches!(ev, InputEvent::Change) {
                return;
            }
            let text = editor.read(cx).value().to_string();
            let Some(tab) = ws.tabs.iter_mut().find(|t| &t.editor == editor) else { return };
            if text == tab.synced {
                return;
            }
            tab.synced = text.clone();
            let view = tab.view.clone();
            view.update(cx, |v, cx| v.set_source(text, cx));
            let doc = view.read(cx).doc().clone();
            editor.update(cx, |e, cx| source::set_diagnostics(e, &doc, cx));
        });
        let view_sub = cx.observe_in(&view, window, |ws: &mut Self, view, window, cx| {
            // A canvas notifies on every pointer move; only what the other
            // panes show (text, selection, dirty flag, status) is worth a
            // workspace redraw.
            let Some(i) = ws.tabs.iter().position(|t| t.view.entity_id() == view.entity_id()) else { return };
            let v = view.read(cx);
            let tab = &ws.tabs[i];
            let src_changed = v.doc().source() != tab.synced;
            let status = v.status().clone();
            let changed = src_changed || v.selection() != tab.last_sel.as_slice() || v.is_dirty() != tab.last_dirty || status != tab.last_status;
            if !changed {
                return;
            }
            let sel = v.selection().to_vec();
            let dirty = v.is_dirty();
            if src_changed {
                let src = v.doc().source().to_string();
                let doc = v.doc().clone();
                ws.tabs[i].synced = src.clone();
                ws.tabs[i].editor.update(cx, |e, cx| {
                    e.set_value(src, window, cx);
                    source::set_diagnostics(e, &doc, cx);
                });
            }
            ws.tabs[i].last_sel = sel;
            ws.tabs[i].last_dirty = dirty;
            if status != ws.tabs[i].last_status {
                ws.tabs[i].last_status = status.clone();
                ws.toast(status, window, cx);
            }
            ws.sync_inspector(window, cx);
            cx.notify();
        });
        let event_sub = cx.subscribe_in(&view, window, |ws: &mut Self, view, ev: &crate::view::ViewEvent, window, cx| match ev {
            crate::view::ViewEvent::ConfirmDelete(ids) => ws.confirm_delete(view.clone(), ids.clone(), window, cx),
            crate::view::ViewEvent::AddImages(images, at) => ws.add_images(view.clone(), images.clone(), *at, window, cx),
            crate::view::ViewEvent::ChangedOnDisk => ws.ask_reload(view.clone(), window, cx),
            crate::view::ViewEvent::DiagramsDropped(paths, at) => ws.diagrams_dropped(view.clone(), paths.clone(), *at, window, cx),
            crate::view::ViewEvent::OpenDiagram(path, copy) => {
                if path.exists() {
                    ws.open_path(path.clone(), window, cx);
                } else if let Some(text) = copy {
                    // Not here (a shared package): its snapshot, unsaved.
                    ws.new_tab(Document::parse(text.clone()), None, window, cx);
                    ws.toast(format!("{} is not here; this is the copy saved in the package", path.display()), window, cx);
                } else {
                    ws.toast(format!("{} is missing", path.display()), window, cx);
                }
            }
        });
        // A canvas taking focus makes its diagram the active one, so split
        // views drive the inspector and source panes.
        let focus = view.read(cx).focus_handle().clone();
        let focus_sub = cx.on_focus_in(&focus, window, {
            let view = view.clone();
            move |ws: &mut Self, window, cx| ws.diagram_shown(&view, window, cx)
        });
        let pane = Pane::new(PaneKind::Diagram(view.clone()), cx.weak_entity(), cx);
        let target = self.active_group(cx);
        self.tabs.push(Tab { view, pane: pane.clone(), editor, synced: src, last_status: SharedString::default(), last_sel: Vec::new(), last_dirty: false, _subs: vec![ed_sub, view_sub, focus_sub, event_sub] });
        self.active = self.tabs.len() - 1;
        self.apply_canvas_settings(cx);
        let pid = Pane::id(&pane);
        self.dock.update(cx, |d, cx| {
            d.add_panel(pane, DockPlacement::Center, None, window, cx);
            if let Some(node) = target {
                d.move_panel(pid, InsertTarget::Tabs { node, ix: None, activate: true }, window, cx);
            }
        });
        self.save_session(cx);
        cx.notify();
    }

    // ---- dock ----

    /// The dock pane of the active diagram.
    pub(crate) fn active_pane_id(&self, _cx: &App) -> Option<PanelId> {
        self.tabs.get(self.active).map(|t| Pane::id(&t.pane))
    }

    /// The tab group holding the active diagram, where new diagrams open.
    fn active_group(&self, cx: &App) -> Option<gpui_kit::component::dock::NodeId> {
        let pid = self.active_pane_id(cx)?;
        let d = self.dock.read(cx);
        [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
            .into_iter()
            .find_map(|p| d.layout(p).and_then(|t| t.find_panel_node(pid)))
    }

    /// A diagram tab became the displayed or focused one.
    pub(crate) fn diagram_shown(&mut self, view: &Entity<DiagramView>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.tabs.iter().position(|t| &t.view == view) else { return };
        if i != self.active {
            self.active = i;
            self.inspected = None;
            self.sync_inspector(window, cx);
            self.save_session(cx);
        }
        cx.notify();
    }

    /// A pane left the dock (closed from its tab, or displaced by a layout
    /// load). Diagrams closed this way have no unsaved-changes prompt left to
    /// give, so only already-handled closes reach here with a tab still open.
    pub(crate) fn pane_removed(&mut self, kind: &PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        // Removal is reported after the fact; a pane put back since (a layout
        // load re-adding open diagrams) is not closed.
        if let PaneKind::Diagram(v) = kind
            && let Some(i) = self.tabs.iter().position(|t| &t.view == v)
            && self.placement_of(Pane::id(&self.tabs[i].pane), cx).is_none()
        {
            self.force_close(i, window, cx);
        }
        cx.notify();
    }

    /// Close the tab showing `view`, asking about unsaved changes.
    pub(crate) fn close_view(&mut self, view: &Entity<DiagramView>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.iter().position(|t| &t.view == view) {
            self.close_tab(i, window, cx);
        }
    }

    fn tool(&mut self, t: Tool, cx: &mut Context<Self>) -> Entity<Pane> {
        let ws = cx.weak_entity();
        self.tools.entry(t).or_insert_with(|| Pane::new(PaneKind::Tool(t), ws, cx)).clone()
    }

    /// Where a panel sits, if it is in the dock.
    fn placement_of(&self, pid: PanelId, cx: &App) -> Option<DockPlacement> {
        let d = self.dock.read(cx);
        [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
            .into_iter()
            .find(|p| d.layout(*p).is_some_and(|t| t.panels().any(|x| x == pid)))
    }

    #[cfg(test)]
    pub(crate) fn placement_of_tool(&self, t: Tool, cx: &App) -> Option<DockPlacement> {
        self.placement_of(Pane::id(self.tools.get(&t)?), cx)
    }

    /// The tool is in the dock and its dock is open.
    pub(crate) fn shown(&self, t: Tool, cx: &App) -> bool {
        let Some(pane) = self.tools.get(&t) else { return false };
        match self.placement_of(Pane::id(pane), cx) {
            None => false,
            Some(DockPlacement::Center) => true,
            Some(p) => self.dock.read(cx).is_dock_open(p),
        }
    }

    pub(crate) fn toggle_tool(&mut self, t: Tool, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown(t, cx) {
            let pane = self.tool(t, cx);
            // The bottom dock folds to its strip, like VS Code's panel;
            // elsewhere the tab goes away.
            if self.placement_of(Pane::id(&pane), cx) == Some(DockPlacement::Bottom) {
                self.dock.update(cx, |d, cx| d.toggle_dock(DockPlacement::Bottom, window, cx));
            } else {
                self.dock.update(cx, |d, cx| d.remove_panel(pane, window, cx));
            }
        } else {
            self.show_tool(t, window, cx);
        }
        cx.notify();
    }

    /// Bring a tool on screen: select it where it is, or put it in its home.
    pub(crate) fn show_tool(&mut self, t: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.tool(t, cx);
        let pid = Pane::id(&pane);
        if self.placement_of(pid, cx).is_some() {
            self.dock.update(cx, |d, cx| d.select_panel(pid, window, cx));
            self.open_dock_of(pid, window, cx);
            return;
        }
        let target = self.active_group(cx);
        self.dock.update(cx, |d, cx| match t.home() {
            // Settings opens as a tab beside the diagrams.
            DockPlacement::Center if t == Tool::Settings => {
                d.add_panel(pane, DockPlacement::Center, None, window, cx);
                if let Some(node) = target {
                    d.move_panel(pid, InsertTarget::Tabs { node, ix: None, activate: true }, window, cx);
                }
            }
            DockPlacement::Center => {
                d.add_panel(pane, DockPlacement::Center, None, window, cx);
                if let Some(node) = target {
                    d.move_panel(pid, InsertTarget::Split { node, placement: Placement::Right, size: Some(SOURCE_W) }, window, cx);
                }
            }
            home => d.add_panel(pane, home, Some(dock_size(home)), window, cx),
        });
        self.open_dock_of(pid, window, cx);
    }

    /// Open the side or bottom dock holding `pid`, at a usable size: a
    /// bottom dock dragged shut comes back at its last real height, or the
    /// default when that was only the floor.
    pub(crate) fn open_dock_of(&mut self, pid: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(side) = self.placement_of(pid, cx).filter(|p| *p != DockPlacement::Center) else { return };
        self.open_dock(side, window, cx);
    }

    fn open_dock(&mut self, side: DockPlacement, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |d, cx| {
            if side == DockPlacement::Bottom && d.dock_size(side).is_some_and(|s| s < BOTTOM_DOCK_MIN_H) {
                d.set_dock_size(side, BOTTOM_DOCK_H, window, cx);
            }
            if !d.is_dock_open(side) {
                d.toggle_dock(side, window, cx);
            }
        });
        cx.notify();
    }

    /// A side dock: collapse it if it has panes, otherwise show `fallback`.
    fn toggle_side(&mut self, side: DockPlacement, fallback: Tool, window: &mut Window, cx: &mut Context<Self>) {
        let has = self.dock.read(cx).layout(side).is_some_and(|t| t.panels().next().is_some());
        if has && self.dock.read(cx).is_dock_open(side) {
            self.dock.update(cx, |d, cx| d.toggle_dock(side, window, cx));
        } else if has {
            self.open_dock(side, window, cx);
        } else {
            self.show_tool(fallback, window, cx);
        }
        cx.notify();
    }

    /// Shapes and outline left, inspector right, problems below (closed),
    /// diagrams in the middle with the source beside them if wanted.
    pub(crate) fn default_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keep_settings = self.shown(Tool::Settings, cx);
        let [shapes, outline, inspector, source, problems, settings] = Tool::ALL.map(|t| self.tool(t, cx));
        let mut docs = DockLayout::tabs();
        for t in &self.tabs {
            docs = docs.panel(t.pane.clone());
        }
        // Resetting from the Settings tab keeps it open.
        if keep_settings {
            docs = docs.panel(settings);
        }
        let docs = docs.active_index(self.active);
        let center = if self.settings.source_panel {
            DockLayout::h_split().child(docs, None).child(DockLayout::tabs().panel(source), Some(SOURCE_W))
        } else {
            docs
        };
        let (left, right) = (self.settings.shapes_panel, self.settings.inspector);
        self.dock.update(cx, |d, cx| {
            d.set_center(center, window, cx);
            d.set_dock(DockPlacement::Left, DockLayout::tabs().panel(shapes).panel(outline), window, cx);
            d.set_dock_size(DockPlacement::Left, dock_size(DockPlacement::Left), window, cx);
            d.set_dock(DockPlacement::Right, DockLayout::tabs().panel(inspector), window, cx);
            d.set_dock_size(DockPlacement::Right, dock_size(DockPlacement::Right), window, cx);
            d.set_dock(DockPlacement::Bottom, DockLayout::tabs().panel(problems), window, cx);
            d.set_dock_size(DockPlacement::Bottom, dock_size(DockPlacement::Bottom), window, cx);
            for (side, open) in [(DockPlacement::Left, left), (DockPlacement::Right, right), (DockPlacement::Bottom, false)] {
                if d.is_dock_open(side) != open {
                    d.toggle_dock(side, window, cx);
                }
            }
        });
    }

    /// Rebuild a saved layout. False when it could not be read.
    pub(crate) fn restore_layout(&mut self, state: DockAreaState, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut panes = HashMap::new();
        for t in Tool::ALL {
            panes.insert(t.name().to_string(), self.tool(t, cx));
        }
        for tab in &self.tabs {
            if let Some(p) = tab.view.read(cx).path() {
                panes.insert(format!("diagram:{}", p.display()), tab.pane.clone());
            }
        }
        let restore = Restore { panes, ws: cx.weak_entity() };
        let dock = self.dock.clone();
        if crate::dock::with_restore(restore, || dock.update(cx, |d, cx| d.load(state, window, cx))).is_err() {
            return false;
        }
        // Drop panes for files that are gone; add open diagrams it lacks.
        let all: Vec<Entity<Pane>> = {
            let d = self.dock.read(cx);
            [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
                .into_iter()
                .filter_map(|p| d.layout(p))
                .flat_map(|t| t.panels().collect::<Vec<_>>())
                .filter_map(|pid| d.panel(pid).and_then(|v| v.view().downcast::<Pane>().ok()))
                .collect()
        };
        let missing: Vec<Entity<Pane>> = all.into_iter().filter(|p| matches!(p.read(cx).kind, PaneKind::Missing)).collect();
        for pane in missing {
            self.dock.update(cx, |d, cx| d.remove_panel(pane, window, cx));
        }
        let target = self.active_group(cx);
        for tab in &self.tabs {
            let pid = Pane::id(&tab.pane);
            if self.placement_of(pid, cx).is_none() {
                let pane = tab.pane.clone();
                self.dock.update(cx, |d, cx| {
                    d.add_panel(pane, DockPlacement::Center, None, window, cx);
                    if let Some(node) = target {
                        d.move_panel(pid, InsertTarget::Tabs { node, ix: None, activate: false }, window, cx);
                    }
                });
            }
        }
        true
    }

    /// Ask, then delete `ids` in `view`.
    fn confirm_delete(&mut self, view: Entity<DiagramView>, ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sum) = crate::ops::delete_summary(view.read(cx).doc().diagram(), &ids) else { return };
        // What goes besides the things named in the title.
        let named: Vec<String> = {
            let d = view.read(cx).doc().diagram();
            ids.iter().map(|id| d.node(id).map(|n| n.text().replace('\n', " ")).or_else(|| d.group(id).map(|g| g.label.clone().unwrap_or_else(|| g.id.clone()))).unwrap_or_default()).collect()
        };
        let single = ids.len() == 1;
        let action: crate::confirm::DialogAction = Box::new(move |_, _, cx| view.update(cx, |v, cx| v.delete_ids(&ids, cx)));
        let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut stats = Vec::new();
        // Only worth counting when more than the one thing named goes.
        if sum.nodes + sum.groups + sum.edges > 1 {
            if sum.groups > 0 {
                stats.push((Lucide::Group, count(sum.groups, "group", "groups").into()));
            }
            if sum.nodes > 0 {
                stats.push((Lucide::Shapes, count(sum.nodes, "shape", "shapes").into()));
            }
            if sum.edges > 0 {
                stats.push((Lucide::Spline, count(sum.edges, "connection", "connections").into()));
            }
        }
        let rest: Vec<(bool, String)> = if single { sum.names.into_iter().filter(|(_, n)| !named.contains(n)).collect() } else { sum.names };
        let items = if rest.len() > 1 || (single && !rest.is_empty()) { rest.into_iter().map(|(g, n)| (if g { Lucide::Group } else { Lucide::Square }, n.into())).collect() } else { Vec::new() };
        let c = crate::confirm::Confirm::danger(sum.title, sum.message, "Delete", action).summary(stats, items);
        self.ask(c, window, cx);
    }

    /// Pictures into `view`. Unsaved diagrams and packages keep them inside;
    /// a plain `.gph` asks whether to become a package or link the files.
    pub(crate) fn add_images(&mut self, view: Entity<DiagramView>, images: Vec<crate::view::IncomingImage>, at: graphing_model::Point, window: &mut Window, cx: &mut Context<Self>) {
        let path = view.read(cx).path().cloned();
        let plain = path.as_ref().is_some_and(|p| !crate::files::is_package_path(p));
        if !plain {
            view.update(cx, |v, cx| v.insert_images(images, at, true, window, cx));
            return;
        }
        let path = path.expect("plain has a path");
        let can_link = images.iter().all(|i| i.path.is_some());
        let package = path.with_extension("gphz");
        let package_name = package.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let one = images.len() == 1;
        let count = if one { "this picture".to_string() } else { format!("these {} pictures", images.len()) };
        let (view2, images2) = (view.clone(), images.clone());
        let mut choices = vec![crate::confirm::Choice {
            icon: Lucide::Package,
            title: format!("Save as {package_name}").into(),
            description: "One file with the diagram and its pictures.".into(),
            badge: None,
            action: Some(Box::new(move |ws, window, cx| {
                view.update(cx, |v, cx| v.insert_images(images, at, true, window, cx));
                ws.save_as(&view, package, window, cx);
            })),
        }];
        if can_link {
            choices.push(crate::confirm::Choice {
                icon: Lucide::Link,
                title: "Link the files".into(),
                description: format!("Moving or renaming the {} breaks the link.", if one { "picture" } else { "pictures" }).into(),
                badge: None,
                action: Some(Box::new(move |_, window, cx| view2.update(cx, |v, cx| v.insert_images(images2, at, false, window, cx)))),
            });
        }
        let it = if one { "it" } else { "them" };
        let message = if can_link { format!("Keep {it} with the diagram, or link to {it}.") } else { "Pasted pictures live inside the diagram.".to_string() };
        let c = crate::confirm::Confirm::choose(format!("Add {count}?"), message, choices);
        self.ask(c, window, cx);
    }

    /// Save `view` to `path` and make that its file (recents, session).
    pub(crate) fn save_as(&mut self, view: &Entity<DiagramView>, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let moved = view.read(cx).path() != Some(&path);
        match view.update(cx, |v, cx| v.save_to(path.clone(), cx)) {
            Ok(()) => {
                if moved {
                    self.toast(format!("saved {}", path.display()), window, cx);
                }
                self.recent.retain(|p| p != &path);
                self.recent.insert(0, path);
                self.save_session(cx);
            }
            Err(e) => self.toast(format!("save failed: {e}"), window, cx),
        }
    }

    /// File > Insert Image: pick pictures, add them at the canvas center.
    fn insert_image_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Insert".into()) });
        let view = self.view().clone();
        cx.spawn_in(window, async move |ws, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            ws.update_in(cx, |ws, window, cx| {
                let images: Vec<_> = paths.iter().filter_map(|p| crate::view::IncomingImage::from_path(p)).collect();
                if images.is_empty() {
                    ws.toast("no pictures in that selection (png, jpg, gif, webp, bmp, svg)", window, cx);
                    return;
                }
                let at = view.read(cx).center();
                ws.add_images(view.clone(), images, at, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn confirm_reset_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let c = crate::confirm::Confirm::danger(
            "Reset the layout?",
            "Every pane goes back to its starting dock and size. Open diagrams stay open.",
            "Reset layout",
            Box::new(|ws, window, cx| ws.default_layout(window, cx)),
        );
        self.ask(c, window, cx);
    }

    fn schedule_layout_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layout_gen += 1;
        let generation = self.layout_gen;
        cx.spawn_in(window, async move |ws, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(400)).await;
            ws.update(cx, |ws, cx| {
                if ws.layout_gen == generation {
                    ws.save_session(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = path.canonicalize().unwrap_or(path);
        if let Some(i) = self.tabs.iter().position(|t| t.view.read(cx).path() == Some(&path)) {
            self.activate(i, window, cx);
            return;
        }
        let (src, assets) = if path.exists() {
            match crate::files::load(&path) {
                Ok(loaded) => loaded,
                Err(e) => {
                    self.toast(format!("cannot open {}: {e}", path.display()), window, cx);
                    return;
                }
            }
        } else {
            Default::default()
        };
        // Replace a pristine untitled tab instead of piling up empties.
        let pristine = self.tabs.get(self.active).is_some_and(|t| {
            let v = t.view.read(cx);
            v.path().is_none() && !v.is_dirty() && v.doc().source().is_empty()
        });
        if pristine {
            let tab = self.tabs.remove(self.active);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            self.dock.update(cx, |d, cx| d.remove_panel(tab.pane, window, cx));
        }
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path.clone());
        self.recent.truncate(12);
        self.new_tab(Document::parse(src), Some(path), window, cx);
        if !assets.is_empty() {
            self.view().update(cx, |v, cx| v.set_assets(assets, cx));
        }
    }

    fn close_tab(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(i) else { return };
        if tab.view.read(cx).is_dirty() {
            let name = tab.view.read(cx).title();
            let view = tab.view.clone();
            let index = move |ws: &Workspace| ws.tabs.iter().position(|t| t.view == view);
            let index2 = index.clone();
            let c = crate::confirm::Confirm {
                title: format!("Save changes to {name}?").into(),
                message: "Your changes will be lost if you close without saving.".into(),
                stats: Vec::new(),
                items: Vec::new(),
                choices: Vec::new(),
                highlight: 0,
                cancel: None,
                icon: None,
                tone: crate::confirm::Tone::Warning,
                buttons: vec![
                    crate::confirm::DialogButton {
                        label: "Don't save".into(),
                        primary: false,
                        action: Some(Box::new(move |ws, window, cx| {
                            if let Some(i) = index2(ws) {
                                ws.force_close(i, window, cx);
                            }
                        })),
                    },
                    crate::confirm::DialogButton {
                        label: "Save".into(),
                        primary: true,
                        action: Some(Box::new(move |ws, window, cx| {
                            let Some(i) = index(ws) else { return };
                            ws.active = i;
                            ws.save(false, window, cx);
                            if !ws.tabs.get(i).is_some_and(|t| t.view.read(cx).is_dirty()) {
                                ws.force_close(i, window, cx);
                            }
                        })),
                    },
                ],
                input: None,
                focus: None,
            };
            self.ask(c, window, cx);
            return;
        }
        self.force_close(i, window, cx);
    }

    /// Tabs whose diagrams have unsaved changes.
    pub(crate) fn dirty_tabs(&self, cx: &App) -> Vec<usize> {
        (0..self.tabs.len()).filter(|&i| self.tabs[i].view.read(cx).is_dirty()).collect()
    }

    /// Quit, first asking about unsaved diagrams: save them all, or not.
    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.dirty_tabs(cx);
        if dirty.is_empty() || self.quitting {
            return self.quit_now(cx);
        }
        let names: Vec<String> = dirty.iter().map(|&i| self.tabs[i].view.read(cx).title().to_string()).collect();
        let title = match names.as_slice() {
            [one] => format!("Save changes to {one} before quitting?"),
            _ => format!("{} diagrams have unsaved changes", names.len()),
        };
        let items = if names.len() > 1 { names.into_iter().map(|n| (Lucide::FileText, n.into())).collect() } else { Vec::new() };
        let c = crate::confirm::Confirm {
            title: title.into(),
            message: "Your changes will be lost if you quit without saving.".into(),
            stats: Vec::new(),
            items,
            choices: Vec::new(),
            highlight: 0,
            cancel: None,
            icon: None,
            tone: crate::confirm::Tone::Warning,
            buttons: vec![
                crate::confirm::DialogButton { label: "Quit without saving".into(), primary: false, action: Some(Box::new(|ws, _, cx| ws.quit_now(cx))) },
                crate::confirm::DialogButton { label: "Save all and quit".into(), primary: true, action: Some(Box::new(|ws, window, cx| ws.save_all_then_quit(window, cx))) },
            ],
            input: None,
                focus: None,
        };
        self.ask(c, window, cx);
    }

    /// Diagram files dropped on a canvas: open them, put their contents in
    /// this diagram, or add links to them.
    pub(crate) fn diagrams_dropped(&mut self, view: Entity<DiagramView>, paths: Vec<PathBuf>, at: graphing_model::Point, window: &mut Window, cx: &mut Context<Self>) {
        use crate::confirm::Choice;
        let one = paths.len() == 1;
        let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let what = if one { name(&paths[0]) } else { format!("{} diagrams", paths.len()) };
        // Links point at graphing files; other formats are imported.
        let linkable = paths.iter().all(|p| crate::files::is_diagram_path(p));
        let (open, insert, link) = (paths.clone(), (paths.clone(), view.clone()), (paths, view));
        let mut choices = vec![
            Choice {
                icon: Lucide::PanelTop,
                title: if one { "Open in a new tab".into() } else { "Open each in a new tab".into() },
                description: "Work on it on its own.".into(),
                badge: None,
                action: Some(Box::new(move |ws, window, cx| {
                    for p in open {
                        ws.open_or_import(p, window, cx);
                    }
                })),
            },
            Choice {
                icon: Lucide::Combine,
                title: "Insert it here".into(),
                description: "Copy its shapes and lines into this diagram, as a group where you dropped it.".into(),
                badge: None,
                action: Some(Box::new(move |ws, window, cx| {
                    let (paths, view) = insert;
                    let mut at = at;
                    for p in paths {
                        ws.insert_diagram(&view, &p, at, window, cx);
                        at.y += 60.0;
                    }
                })),
            },
        ];
        if linkable {
            choices.push(Choice {
                icon: Lucide::Link,
                title: "Link to it".into(),
                description: "A card showing it small that stays up to date; double-click to open it.".into(),
                badge: None,
                action: Some(Box::new(move |_, _, cx| {
                    let (paths, view) = link;
                    view.update(cx, |v, cx| {
                        for (k, p) in paths.iter().enumerate() {
                            v.add_link(p, graphing_model::Point::new(at.x + k as f64 * 290.0, at.y), cx);
                        }
                    });
                })),
            });
        }
        let mut c = crate::confirm::Confirm::choose(format!("Add {what}?"), "Dropped on this diagram.", choices);
        c.icon = Some(Lucide::FileSymlink);
        self.ask(c, window, cx);
    }

    /// Open a graphing file, or import another format into a new tab.
    pub(crate) fn open_or_import(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if crate::files::is_diagram_path(&path) {
            return self.open_path(path, window, cx);
        }
        match crate::export::import(&path, None) {
            Ok((src, warnings)) => {
                self.new_tab(Document::parse(src), None, window, cx);
                if !warnings.is_empty() {
                    self.toast(format!("imported with {} warning(s): {}", warnings.len(), warnings.join("; ")), window, cx);
                }
            }
            Err(e) => self.toast(format!("cannot open {}: {e}", path.display()), window, cx),
        }
    }

    /// Copy the diagram at `path` into `view` at `at` (one undo step).
    pub(crate) fn insert_diagram(&mut self, view: &Entity<DiagramView>, path: &std::path::Path, at: graphing_model::Point, window: &mut Window, cx: &mut Context<Self>) {
        let src = if crate::files::is_diagram_path(path) {
            crate::files::load(path).map(|(src, _)| src).map_err(anyhow::Error::from)
        } else {
            crate::export::import(path, None).map(|(src, _)| src)
        };
        match src {
            Ok(src) => {
                let other = Document::parse(src);
                let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "inserted".into());
                let op = crate::combine::insert(view.read(cx).doc().diagram(), other.diagram(), &stem, at);
                view.update(cx, |v, cx| {
                    v.apply(op, cx);
                });
            }
            Err(e) => self.toast(format!("cannot insert {}: {e}", path.display()), window, cx),
        }
    }

    /// The file changed on disk under unsaved edits: reload it, or keep
    /// the edits (saving them then overwrites the other change).
    fn ask_reload(&mut self, view: Entity<DiagramView>, window: &mut Window, cx: &mut Context<Self>) {
        let name = view.read(cx).title();
        let c = crate::confirm::Confirm {
            title: format!("{name} changed on disk").into(),
            message: "Another program saved it while you have unsaved edits here. Reload it and lose your edits, or keep them?".into(),
            stats: Vec::new(),
            items: Vec::new(),
            choices: Vec::new(),
            highlight: 0,
            cancel: Some("Keep my edits".into()),
            icon: None,
            tone: crate::confirm::Tone::Warning,
            buttons: vec![crate::confirm::DialogButton { label: "Reload from disk".into(), primary: true, action: Some(Box::new(move |_, _, cx| view.update(cx, |v, cx| v.reload_from_disk(cx)))) }],
            input: None,
                focus: None,
        };
        self.ask(c, window, cx);
    }

    fn quit_now(&mut self, cx: &mut Context<Self>) {
        self.quitting = true;
        self.save_session(cx);
        cx.quit();
    }

    /// Save every unsaved diagram (untitled ones ask where, one by one),
    /// then quit; a cancelled or failed save stops the quit.
    fn save_all_then_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let views: Vec<Entity<DiagramView>> = self.dirty_tabs(cx).into_iter().map(|i| self.tabs[i].view.clone()).collect();
        cx.spawn_in(window, async move |ws, cx| {
            for view in views {
                let (path, (dir, name)) = view.read_with(cx, |v, _| (v.path().cloned(), v.dialog_start(v.save_ext())));
                let path = match path {
                    Some(p) => p,
                    None => {
                        let Ok(rx) = ws.update(cx, |_, cx| cx.prompt_for_new_path(&dir, Some(&name))) else { return };
                        match rx.await {
                            Ok(Ok(Some(p))) => p,
                            _ => {
                                ws.update_in(cx, |ws, window, cx| ws.toast("quit cancelled: a diagram is still unsaved", window, cx)).ok();
                                return;
                            }
                        }
                    }
                };
                let path = view.read_with(cx, |v, _| v.save_path(path));
                let saved = view.update(cx, |v, cx| v.save_to(path.clone(), cx));
                if let Err(e) = saved {
                    ws.update_in(cx, |ws, window, cx| ws.toast(format!("quit cancelled: saving {} failed: {e}", path.display()), window, cx)).ok();
                    return;
                }
            }
            ws.update(cx, |ws, cx| ws.quit_now(cx)).ok();
        })
        .detach();
    }

    fn force_close(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if i >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(i);
        self.dock.update(cx, |d, cx| d.remove_panel(tab.pane, window, cx));
        if self.tabs.is_empty() {
            self.new_tab(Document::parse(""), None, window, cx);
        }
        self.active = self.active.min(self.tabs.len() - 1);
        if i < self.active {
            self.active -= 1;
        }
        self.inspected = None;
        self.save_session(cx);
        self.focus_canvas(window, cx);
        cx.notify();
    }

    fn activate(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if i < self.tabs.len() {
            self.active = i;
            let pid = Pane::id(&self.tabs[i].pane);
            self.dock.update(cx, |d, cx| d.select_panel(pid, window, cx));
            self.inspected = None;
            self.sync_inspector(window, cx);
            self.focus_canvas(window, cx);
            self.save_session(cx);
            cx.notify();
        }
    }

    // ---- files ----

    fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |ws, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                ws.update_in(cx, |ws, window, cx| {
                    for p in paths {
                        ws.open_path(p, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    fn save(&mut self, force_dialog: bool, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.view().clone();
        match view.read(cx).path().cloned() {
            Some(path) if !force_dialog => {
                let path = view.read(cx).save_path(path);
                self.save_as(&view, path, window, cx);
            }
            _ => {
                let (dir, name) = view.read(cx).dialog_start(view.read(cx).save_ext());
                let rx = cx.prompt_for_new_path(&dir, Some(&name));
                cx.spawn_in(window, async move |ws, cx| {
                    let Ok(Ok(Some(path))) = rx.await else { return };
                    ws.update_in(cx, |ws, window, cx| {
                        let path = view.read(cx).save_path(path);
                        ws.save_as(&view, path, window, cx);
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    /// A short message in the corner that fades on its own.
    pub(crate) fn toast(&mut self, msg: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        let msg = msg.into();
        if !msg.is_empty() {
            self.notice(kit::NoticeTone::Info, msg, None, window, cx);
        }
    }

    // ---- theme and session ----

    pub(crate) fn apply_theme(&self, window: &mut Window, cx: &mut App) {
        let dark = match self.theme {
            ThemeChoice::System => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        };
        graphing_ui::install(dark, Some(window), cx);
        for tab in &self.tabs {
            tab.editor.update(cx, source::refresh);
        }
    }

    fn cycle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme = match self.theme {
            ThemeChoice::System => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
            ThemeChoice::Dark => ThemeChoice::System,
        };
        self.apply_theme(window, cx);
        self.save_session(cx);
        window.refresh();
        cx.notify();
    }

    /// Persist open files to state.json and UI toggles to settings.json.
    fn save_session(&mut self, cx: &App) {
        State {
            files: self.tabs.iter().filter_map(|t| t.view.read(cx).path().cloned()).collect(),
            active: self.active,
            recent: self.recent.clone(),
            dock: serde_json::to_value(self.dock.read(cx).dump(cx)).ok(),
        }
        .save();
        if self.settings.theme != self.theme {
            self.settings.theme = self.theme;
            self.settings.save();
        }
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_tool(Tool::Settings, window, cx);
    }

    /// Push grid and snap settings to every canvas.
    pub(crate) fn apply_canvas_settings(&mut self, cx: &mut Context<Self>) {
        let (grid, snap, show) = (self.settings.grid, self.settings.snap, self.settings.show_grid);
        let confirm = self.settings.confirm_delete;
        let play = self.settings.play;
        for t in &self.tabs {
            t.view.update(cx, |v, cx| {
                v.set_grid(grid, snap, show, cx);
                v.set_play(play, cx);
                v.set_confirm_delete(confirm);
            });
        }
    }

    fn reload_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (settings, mut errors) = Settings::load();
        errors.extend(crate::keymap::apply_user(cx, &settings.keybindings));
        self.theme = settings.theme;
        self.settings = settings;
        self.apply_theme(window, cx);
        self.apply_canvas_settings(cx);
        let msg = if errors.is_empty() { "settings reloaded".to_string() } else { format!("settings: {}", errors.join("; ")) };
        self.toast(msg, window, cx);
        cx.notify();
    }

    // ---- inspector ----


    /// Selection changed: drop the old fields so new ones load fresh values.
    fn sync_inspector(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let sel = self.view().read(cx).selection().to_vec();
        if self.inspected.as_ref() == Some(&sel) {
            return;
        }
        // The first sync only records the selection; later changes reset.
        if self.inspected.replace(sel).is_some() {
            self.select = None;
        }
        self.fields.clear();
        self.adding.clear();
    }

    pub(crate) fn edit(&mut self, op: Op, cx: &mut Context<Self>) {
        self.view().update(cx, |v, cx| {
            v.apply(op, cx);
        });
    }

    pub(crate) fn with_view(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut DiagramView, &mut Context<DiagramView>)) {
        self.view().update(cx, f);
    }

    // ---- palette ----

    pub(crate) fn commands(&self, window: &Window, cx: &App) -> Vec<Command> {
        use Lucide as L;
        let list: Vec<(&'static str, &'static str, L, Box<dyn Action>)> = vec![
            ("File", "New Diagram", L::FilePlus, Box::new(NewFile)),
            ("File", "Open...", L::FolderOpen, Box::new(OpenFile)),
            ("File", "Save", L::Save, Box::new(Save)),
            ("File", "Save As...", L::SaveAll, Box::new(SaveAs)),
            ("File", "Close Tab", L::X, Box::new(CloseTab)),
            ("File", "Next Tab", L::ArrowRight, Box::new(NextTab)),
            ("File", "Previous Tab", L::ArrowLeft, Box::new(PrevTab)),
            ("File", "Import Any Supported File...", L::FileInput, Box::new(ImportFile)),
            ("File", "Import Mermaid...", L::FileInput, Box::new(ImportAs { format: "mermaid".into() })),
            ("File", "Import draw.io...", L::FileInput, Box::new(ImportAs { format: "drawio".into() })),
            ("File", "Import SysML v2...", L::FileInput, Box::new(ImportAs { format: "sysml".into() })),
            ("File", "Import Visio...", L::FileInput, Box::new(ImportAs { format: "visio".into() })),
            ("File", "Export Animation as GIF...", L::Clapperboard, Box::new(crate::ExportAnimationAs { format: "gif".into() })),
            ("File", "Export Animation as WebM...", L::Film, Box::new(crate::ExportAnimationAs { format: "webm".into() })),
            ("File", "Export Animation as Animated PNG...", L::FileImage, Box::new(crate::ExportAnimationAs { format: "apng".into() })),
            ("File", "Export Animation as Animated SVG...", L::FileCode, Box::new(crate::ExportAnimationAs { format: "svg".into() })),
            ("File", "Export SVG...", L::FileOutput, Box::new(ExportSvg)),
            ("File", "Export PNG...", L::Image, Box::new(ExportPng)),
            ("File", "Export SysML v2...", L::FileCode, Box::new(ExportSysml)),
            ("Edit", "Insert Image...", L::ImagePlus, Box::new(InsertImage)),
            ("Edit", "Save Selection as Block...", L::BookmarkPlus, Box::new(SaveAsBlock)),
            ("File", "Open Settings", L::Settings, Box::new(OpenSettings)),
            ("File", "Reload Settings", L::RefreshCw, Box::new(ReloadSettings)),
            ("File", "Reload Shape Packs", L::PackageCheck, Box::new(ReloadPacks)),
            ("File", "Open Packs Folder", L::FolderOpen, Box::new(OpenPacksFolder)),
            ("File", "Reload Plugins", L::Puzzle, Box::new(ReloadPlugins)),
            ("File", "Open Plugins Folder", L::FolderOpen, Box::new(OpenPluginsFolder)),
            ("File", "Quit", L::LogOut, Box::new(Quit)),
            ("Edit", "Undo", L::Undo2, Box::new(Undo)),
            ("Edit", "Redo", L::Redo2, Box::new(Redo)),
            ("Edit", "Cut", L::Scissors, Box::new(Cut)),
            ("Edit", "Copy", L::Copy, Box::new(Copy)),
            ("Edit", "Paste", L::ClipboardPaste, Box::new(Paste)),
            ("Edit", "Duplicate", L::CopyPlus, Box::new(Duplicate)),
            ("Edit", "Delete", L::Trash, Box::new(Delete)),
            ("Edit", "Select All", L::SquareDashedMousePointer, Box::new(SelectAll)),
            ("Edit", "Rename", L::PencilLine, Box::new(Rename)),
            ("View", "Command Palette", L::Command, Box::new(CommandPalette)),
            ("View", "Toggle Library", L::PanelLeft, Box::new(ToggleLeft)),
            ("View", "Toggle Inspector", L::PanelRight, Box::new(ToggleRight)),
            ("View", "Toggle Source", L::Code, Box::new(ToggleSource)),
            ("View", "Cycle Theme", L::SunMoon, Box::new(CycleTheme)),
            ("View", "Fit to Window", L::Scan, Box::new(FitView)),
            ("View", "Zoom In", L::ZoomIn, Box::new(ZoomIn)),
            ("View", "Zoom Out", L::ZoomOut, Box::new(ZoomOut)),
            ("View", "Actual Size", L::Search, Box::new(ZoomReset)),
            ("Arrange", "Auto Layout", L::Workflow, Box::new(Relayout)),
            ("Arrange", "Align Left", L::AlignStartVertical, Box::new(AlignLeft)),
            ("Arrange", "Align Center", L::AlignCenterVertical, Box::new(AlignCenter)),
            ("Arrange", "Align Right", L::AlignEndVertical, Box::new(AlignRight)),
            ("Arrange", "Align Top", L::AlignStartHorizontal, Box::new(AlignTop)),
            ("Arrange", "Align Middle", L::AlignCenterHorizontal, Box::new(AlignMiddle)),
            ("Arrange", "Align Bottom", L::AlignEndHorizontal, Box::new(AlignBottom)),
            ("Arrange", "Distribute Horizontally", L::AlignHorizontalSpaceAround, Box::new(SpreadH)),
            ("Arrange", "Distribute Vertically", L::AlignVerticalSpaceAround, Box::new(SpreadV)),
            ("Arrange", "Make Same Size", L::Scaling, Box::new(SameSize)),
            ("Arrange", "Group Selection", L::Group, Box::new(GroupSelection)),
            ("Arrange", "Ungroup", L::Ungroup, Box::new(Ungroup)),
        ];
        let keys_for = |action: &dyn Action| self.keys_for(action, window, cx);
        let mut out: Vec<Command> = list
            .into_iter()
            .map(|(group, name, icon, action)| {
                let keys = keys_for(&*action);
                Command { name: name.into(), action, keys, icon: icon.into(), group }
            })
            .collect();
        for (plugin, command, title) in self.plugin_commands() {
            let action: Box<dyn Action> = Box::new(RunPluginCommand { plugin, command });
            let keys = keys_for(&*action);
            out.push(Command { name: title, action, keys, icon: Lucide::Puzzle.into(), group: "Plugins" });
        }
        let reg = graphing_scene::stencils::registry();
        for (_, entries) in reg.catalog() {
            for s in entries {
                let pack = reg.packs.iter().find(|p| p.id == s.pack).map_or(s.pack.as_str(), |p| p.name.as_str());
                let name: SharedString = if s.pack == "core" { format!("Insert {}", s.title) } else { format!("Insert {pack} {}", s.title) }.into();
                if out.iter().any(|c| c.name == name) {
                    continue;
                }
                let action: Box<dyn Action> = Box::new(AddShape { stencil: s.id.clone() });
                let keys = keys_for(&*action);
                out.push(Command { name, action, keys, icon: crate::inspector::stencil_icon(s), group: "Insert" });
            }
        }
        out
    }

    /// Shortcut for `action` as seen from the canvas, where shortcuts fire;
    /// empty when unbound.
    pub(crate) fn keys_for(&self, action: &dyn Action, window: &Window, cx: &App) -> SharedString {
        let canvas = self.view().read(cx).focus_handle().clone();
        window
            .bindings_for_action_in(action, &canvas)
            .last()
            .map(|b| b.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .into()
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.palette = None;
            self.focus_canvas(window, cx);
            cx.notify();
            return;
        }
        let commands = self.commands(window, cx);
        let recent = self.recent_commands.clone();
        let palette = cx.new(|cx| Palette::new(commands, recent, window, cx));
        cx.subscribe_in(&palette, window, |ws: &mut Self, _, ev: &PaletteEvent, window, cx| {
            ws.palette = None;
            ws.focus_canvas(window, cx);
            if let PaletteEvent::Run(name, action) = ev {
                ws.recent_commands.retain(|n| n != name);
                ws.recent_commands.insert(0, name.clone());
                ws.recent_commands.truncate(8);
                window.dispatch_action(action.boxed_clone(), cx);
            }
            cx.notify();
        })
        .detach();
        self.palette = Some(palette);
        cx.notify();
    }

    // ---- export / import ----

    /// Export to a file the user picks; its extension picks the format and
    /// `ext` is the one suggested. `animate` plays the steps (File > Export
    /// > Animation). Rendering runs off the UI thread.
    fn export(&mut self, ext: &'static str, animate: bool, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.view().read(cx);
        if animate && view.steps().is_empty() {
            self.sequence_open = true;
            self.toast("no steps to animate yet: select shapes and press + in the sequence strip", window, cx);
            return;
        }
        let (src, assets, base) = (view.doc().source().to_string(), view.assets().clone(), view.folder().map(PathBuf::from));
        let style = crate::export::Style { dark: cx.ui().dark, animate, scale: if animate { 1.0 } else { 2.0 } };
        let (dir, name) = view.dialog_start(ext);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |ws, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let file: SharedString = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().to_string()).into();
            let folder: Option<SharedString> = path.parent().map(|p| p.display().to_string().into());
            // A notice that follows the work: frames done, then the result.
            let progress = animate.then(|| std::sync::Arc::new(graphing_export::Progress::default()));
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let Ok(id) = ws.update_in(cx, |ws, window, cx| {
                let id = ws.notice(kit::NoticeTone::Working, format!("Exporting {file}"), folder.clone(), window, cx);
                ws.update_notice(id, |n| n.progress = progress.as_ref().map(|_| 0.0), window, cx);
                id
            }) else {
                return;
            };
            if let Some(p) = progress.clone() {
                let done = done.clone();
                ws.update_in(cx, |_, window, cx| {
                    cx.spawn_in(window, async move |ws, cx| {
                        while !done.load(std::sync::atomic::Ordering::Relaxed) {
                            cx.background_executor().timer(std::time::Duration::from_millis(120)).await;
                            let f = p.fraction();
                            if ws.update_in(cx, |ws, window, cx| ws.update_notice(id, |n| n.progress = n.progress.map(|_| f), window, cx)).is_err() {
                                break;
                            }
                        }
                    })
                    .detach();
                })
                .ok();
            }
            let target = path.clone();
            let result = cx.background_executor().spawn(async move { crate::export::write(&src, &assets, base.as_deref(), &target, style, progress) }).await;
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            ws.update_in(cx, |ws, window, cx| {
                ws.update_notice(
                    id,
                    |n| {
                        n.progress = None;
                        match &result {
                            Ok(()) => {
                                n.tone = kit::NoticeTone::Success;
                                n.title = format!("Exported {file}").into();
                                let (open, reveal) = (path.clone(), path.clone());
                                n.actions = vec![
                                    ("Show in folder".into(), Lucide::FolderOpen, std::rc::Rc::new(move |_: &mut Workspace, _: &mut Window, cx: &mut Context<Workspace>| cx.reveal_path(&reveal))),
                                    ("Open".into(), Lucide::ExternalLink, std::rc::Rc::new(move |_: &mut Workspace, _: &mut Window, cx: &mut Context<Workspace>| cx.open_with_system(&open))),
                                ];
                            }
                            Err(e) => {
                                n.tone = kit::NoticeTone::Error;
                                n.title = format!("Could not export {file}").into();
                                n.detail = Some(e.to_string().into());
                            }
                        }
                    },
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Pick a file and open it as a new diagram, parsed as `format` (or by
    /// what it looks like).
    fn import(&mut self, format: Option<graphing_import::Format>, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = match format {
            Some(f) => format!("Import {}", f.title()),
            None => "Import".to_string(),
        };
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(prompt.into()) });
        cx.spawn_in(window, async move |ws, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            ws.update_in(cx, |ws, window, cx| match crate::export::import(&path, format) {
                Ok((src, warnings)) => {
                    ws.new_tab(Document::parse(src), None, window, cx);
                    let n = warnings.len();
                    let msg = if n == 0 { format!("imported {}", path.display()) } else { format!("imported with {n} warning(s): {}", warnings.join("; ")) };
                    ws.toast(msg, window, cx);
                }
                Err(e) => ws.toast(format!("import failed: {e}"), window, cx),
            })
            .ok();
        })
        .detach();
    }

    // ---- rendering ----

    /// The content of one dock pane.
    pub(crate) fn render_pane(&mut self, kind: &PaneKind, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        match kind {
            PaneKind::Diagram(view) => {
                let toolbar = self.render_toolbar(view, cx);
                let focus = view.read(cx).focus_handle().clone();
                let sequence = self.render_sequence(view, cx);
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            // Using a split's toolbar acts on that split's diagram.
                            .capture_any_mouse_down(move |_, window, cx| window.focus(&focus, cx))
                            .child(view.clone())
                            .child(div().absolute().bottom(GAP_4).left_0().right_0().flex().justify_center().child(toolbar))
                            .child(self.render_hud(view, cx)),
                    )
                    .children(sequence)
                    .into_any_element()
            }
            PaneKind::Tool(Tool::Shapes) => self.render_library(LeftTab::Shapes, cx),
            PaneKind::Tool(Tool::Outline) => self.render_library(LeftTab::Outline, cx),
            PaneKind::Tool(Tool::Inspector) => self.render_inspector(window, cx),
            PaneKind::Tool(Tool::Source) => {
                let editor = self.tabs[self.active].editor.clone();
                div().size_full().bg(k.chrome).pt(GAP_1).child(Editor::new(&editor).h_full().bordered(false)).into_any_element()
            }
            PaneKind::Tool(Tool::Problems) => self.render_problems(cx),
            PaneKind::Tool(Tool::Settings) => self.render_settings(window, cx),
            PaneKind::Missing => div().into_any_element(),
        }
    }

    /// Quiet facts in the canvas corner: counts, selection, pointer.
    fn render_hud(&self, view: &Entity<DiagramView>, cx: &App) -> AnyElement {
        let k = cx.ui();
        let v = view.read(cx);
        let d = v.doc().diagram();
        let m = v.mouse();
        let sel = v.selection().len();
        let mut parts = vec![format!("{} nodes", d.nodes.len()), format!("{} edges", d.edges.len())];
        if !d.groups.is_empty() {
            parts.push(format!("{} groups", d.groups.len()));
        }
        if sel > 0 {
            parts.push(format!("{sel} selected"));
        }
        div()
            .absolute()
            .left(GAP_3)
            .bottom(GAP_3)
            .flex()
            .gap(GAP_3)
            .text_size(TEXT_XS)
            .text_color(k.text_faint)
            .child(parts.join("  \u{b7}  "))
            .child(format!("{:.0}, {:.0}", m.x, m.y))
            .into_any_element()
    }

    /// Parser warnings and errors for the active diagram; clicking one shows
    /// it in the source pane.
    fn render_problems(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let view = self.view().read(cx);
        let src = view.doc().source().to_string();
        let title = view.title();
        let line_of = |at: usize| src[..at.min(src.len())].matches('\n').count() + 1;
        let mut diags: Vec<(usize, usize, String, bool)> =
            view.doc().diags().iter().map(|d| (d.span.start, line_of(d.span.start), d.message.clone(), d.severity == graphing_dsl::Severity::Error)).collect();
        // Wiring problems (pins, loops) point at the statement they are about.
        for p in graphing_scene::pins::problems(view.doc().diagram()) {
            let at = view.doc().span_of(&p.id).map_or(0, |s| s.start);
            diags.push((at, line_of(at), p.message, true));
        }
        diags.sort_by_key(|d| d.0);
        if diags.is_empty() {
            return div().size_full().bg(k.chrome).child(kit::empty_state(Lucide::CircleCheck, "No problems", format!("{title} parses and wires cleanly"), cx)).into_any_element();
        }
        let rows = diags.into_iter().enumerate().map(|(i, (offset, line, message, error))| {
            div()
                .id(SharedString::from(format!("problem-{i}")))
                .h(ROW_H)
                .px(PANEL_PAD)
                .flex()
                .items_center()
                .gap(GAP_2)
                .cursor_pointer()
                .hover(|d| d.bg(k.hover))
                .child(gpui_kit::component::Icon::new(if error { Lucide::CircleX } else { Lucide::TriangleAlert }).size(ICON_SM).text_color(if error { k.danger } else { k.warning }))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(TEXT_SM).text_color(k.text).child(message))
                .child(div().flex_none().text_size(TEXT_XS).text_color(k.text_faint).child(format!("{title}  L{line}")))
                .on_click(cx.listener(move |ws, _, window, cx| ws.reveal_offset(offset, window, cx)))
        });
        div().id("problems").size_full().bg(k.chrome).overflow_y_scroll().py(GAP_1).children(rows).into_any_element()
    }

    /// Show the source pane with the cursor at `offset`.
    fn reveal_offset(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::input::RopeExt;
        self.show_tool(Tool::Source, window, cx);
        let editor = self.tabs[self.active].editor.clone();
        editor.update(cx, |e, cx| {
            let pos = e.text().offset_to_position(offset.min(e.text().len()));
            e.set_cursor_position(pos, window, cx);
        });
    }

    fn render_titlebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let (l, r, s) = (self.shown(Tool::Shapes, cx) || self.shown(Tool::Outline, cx), self.shown(Tool::Inspector, cx), self.shown(Tool::Source, cx));
        let p = self.shown(Tool::Problems, cx);
        let menubar = self.render_menubar(window, cx);
        // Breadcrumb: the folder, muted, then the file; a dot when unsaved.
        let (folder, name, dirty, full) = {
            let v = self.view().read(cx);
            let folder = v.path().and_then(|p| p.parent()).and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string());
            let full = v.path().map_or("Not saved yet".to_string(), |p| p.display().to_string());
            (folder, v.title(), v.is_dirty(), full)
        };
        let warn = {
            let doc = self.view().read(cx).doc();
            doc.diags().len() + graphing_scene::pins::problems(doc.diagram()).len()
        };
        let crumb = div()
            .id("crumb")
            .flex()
            .items_center()
            .gap(GAP_1)
            .min_w_0()
            .text_size(TEXT_SM)
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(full.clone()).build(window, cx))
            .when_some(folder, |d, f| d.child(div().text_color(k.text_faint).child(f)).child(div().text_color(k.text_faint).child("/")))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(k.text).child(name))
            .when(dirty, |d| d.child(div().size(DOT).rounded_full().bg(k.text_muted)));
        let theme_icon = match self.theme {
            ThemeChoice::Dark => IconName::Moon,
            ThemeChoice::Light => IconName::Sun,
            ThemeChoice::System => IconName::Palette,
        };
        let mark = div()
            .size(HIT_SM)
            .flex()
            .items_center()
            .justify_center()
            .rounded(ROUND_SM)
            .bg(k.accent)
            .child(gpui_kit::component::Icon::new(Lucide::Waypoints).size(ICON_SM).text_color(k.on_accent));

        TitleBar::new()
            .h(TITLEBAR_H)
            .bg(k.chrome)
            .border_color(k.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .size_full()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h_full()
                            .gap(GAP_2)
                            .min_w_0()
                            .child(mark)
                            .child(menubar)
                            .child(kit::divider_v(cx))
                            .child(crumb)
                            .child(IconButton::new("new-tab", IconName::Plus).small().tooltip("New diagram").action(Box::new(NewFile))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(GAP_0)
                            .pr(GAP_2)
                            .child(IconButton::new("palette", IconName::Search).tooltip("Command palette").action(Box::new(CommandPalette)))
                            .child(kit::divider_v(cx).mx(GAP_1))
                            .child(IconButton::new("t-left", IconName::PanelLeft).active(l).tooltip("Shapes & outline").action(Box::new(ToggleLeft)))
                            .child(IconButton::new("t-source", Lucide::Code).active(s).tooltip("Source").action(Box::new(ToggleSource)))
                            .child(
                                div()
                                    .relative()
                                    .child(IconButton::new("t-problems", Lucide::PanelBottom).active(p).tooltip("Problems").action(Box::new(ToggleProblems)))
                                    .when(warn > 0, |d| d.child(kit::count_badge(warn, k.warning, cx))),
                            )
                            .child(IconButton::new("t-right", IconName::PanelRight).active(r).tooltip("Inspector").action(Box::new(ToggleRight)))
                            .child(IconButton::new("theme", theme_icon).tooltip("Theme: system, light, dark").action(Box::new(CycleTheme))),
                    ),
            )
            .on_close_window(cx.listener(|ws, _, window, cx| ws.request_quit(window, cx)))
            .into_any_element()
    }


    /// Floating pill over the canvas: history, zoom, layout.
    fn render_toolbar(&mut self, view: &Entity<DiagramView>, cx: &mut Context<Self>) -> AnyElement {
        let zoom = view.read(cx).zoom();
        let source = self.shown(Tool::Source, cx);
        let sequence = self.sequence_open || !view.read(cx).steps().is_empty();
        let k = cx.ui();
        kit::raised(cx)
            .flex()
            .items_center()
            .gap(GAP_0)
            .p(GAP_1)
            .child(IconButton::new("tb-undo", Lucide::Undo2).tooltip("Undo").action(Box::new(Undo)))
            .child(IconButton::new("tb-redo", Lucide::Redo2).tooltip("Redo").action(Box::new(Redo)))
            .child(kit::divider_v(cx).mx(GAP_1))
            .child(IconButton::new("tb-zoom-out", Lucide::ZoomOut).tooltip("Zoom out").action(Box::new(ZoomOut)))
            .child(
                div()
                    .id("tb-zoom")
                    .w(HIT_LG + GAP_3)
                    .text_center()
                    .text_size(TEXT_SM)
                    .text_color(k.text)
                    .cursor_pointer()
                    .rounded(ROUND_SM)
                    .hover(|d| d.bg(k.hover))
                    .on_click(cx.listener(|ws, _, _, cx| ws.with_view(cx, |v, cx| v.zoom_reset(cx))))
                    .child(format!("{:.0}%", zoom * 100.0)),
            )
            .child(IconButton::new("tb-zoom-in", Lucide::ZoomIn).tooltip("Zoom in").action(Box::new(ZoomIn)))
            .child(IconButton::new("tb-fit", Lucide::Scan).tooltip("Fit to window").action(Box::new(FitView)))
            .child(kit::divider_v(cx).mx(GAP_1))
            .child(IconButton::new("tb-layout", Lucide::Workflow).tooltip("Auto layout").action(Box::new(Relayout)))
            .child(IconButton::new("tb-source", Lucide::Code).active(source).tooltip("Source").action(Box::new(ToggleSource)))
            .child(kit::divider_v(cx).mx(GAP_1))
            .child(IconButton::new("tb-sequence", Lucide::Clapperboard).active(sequence).tooltip("Sequence: step-by-step animation").action(Box::new(ToggleSequence)))
            .into_any_element()
    }

}

pub(crate) fn shape_icon(shape: Shape) -> Lucide {
    match shape {
        Shape::Rect => Lucide::Square,
        Shape::Rounded => Lucide::RectangleHorizontal,
        Shape::Ellipse => Lucide::Circle,
        Shape::Diamond => Lucide::Diamond,
        Shape::Cylinder => Lucide::Cylinder,
        Shape::Parallelogram => Lucide::Slash,
        Shape::Hexagon => Lucide::Hexagon,
        Shape::Note => Lucide::StickyNote,
        Shape::Actor => Lucide::UserRound,
        Shape::Block => Lucide::Box,
        Shape::Initial => Lucide::CircleDot,
        Shape::Final => Lucide::CircleStop,
        Shape::Bar => Lucide::Minus,
        Shape::Package => Lucide::Folder,
        Shape::Lifeline => Lucide::GitCommitVertical,
        Shape::Path => Lucide::Shapes,
    }
}

/// A stencil's glyph: its built-in outline or its pack's custom path.
pub(crate) fn shape_glyph(stencil: &str, k: Colors) -> AnyElement {
    let (fill, stroke) = (k.node_fill, k.text_muted);
    if let Some(kind) = stencil.strip_prefix("group:") {
        // A small frame in the kind's color (dashed, solid or tinted, with a
        // band or tab where the design has one) and its icon in the body.
        let (look, icon, color) = {
            let reg = graphing_scene::stencils::registry();
            let def = reg.group_kind(kind);
            let look = def.and_then(|k| k.look.as_deref()).or(Some(kind)).and_then(graphing_scene::GroupLook::parse).unwrap_or_default();
            let hex = def.and_then(|k| graphing_model::find_prop(&k.defaults, "stroke").or_else(|| graphing_model::find_prop(&k.defaults, "fill"))).and_then(|v| match v {
                graphing_model::Value::Color(c) => u32::from_str_radix(c.trim_start_matches('#'), 16).ok(),
                _ => None,
            });
            (look, def.and_then(|k| k.icon.clone()), hex)
        };
        let ink: gpui_kit::Hsla = color.map_or(k.text_muted, |c| gpui_kit::rgb(c).into());
        return canvas(
            |_, _, _| {},
            move |b, (), window, cx| {
                use graphing_scene::GroupLook as L;
                use gpui_kit::{BorderStyle, Bounds, point, quad, size};
                let r = Bounds { origin: point(b.origin.x + GAP_0, b.origin.y + GAP_0), size: size(b.size.width - GAP_0 * 2.0, b.size.height - GAP_0 * 2.0) };
                let (fill, border, style, radius) = match look {
                    L::Zone => (ink.opacity(0.22), gpui_kit::transparent_black(), BorderStyle::Solid, ROUND_XS),
                    L::Dashed => (ink.opacity(0.06), ink.opacity(0.8), BorderStyle::Dashed, ROUND_XS),
                    L::Package | L::Sysml => (ink.opacity(0.06), ink.opacity(0.8), BorderStyle::Solid, gpui_kit::Pixels::ZERO),
                    _ => (ink.opacity(0.06), ink.opacity(0.8), BorderStyle::Solid, ROUND_XS),
                };
                let mut body = r;
                let head = GAP_1 + GAP_0;
                match look {
                    L::Package => {
                        let tab = Bounds { origin: r.origin, size: size(r.size.width * 0.45, head) };
                        window.paint_quad(quad(tab, gpui_kit::Pixels::ZERO, fill, HAIRLINE, border, style));
                        body = Bounds { origin: point(r.origin.x, r.origin.y + head - HAIRLINE), size: size(r.size.width, r.size.height - head + HAIRLINE) };
                        window.paint_quad(quad(body, radius, fill, HAIRLINE, border, style));
                    }
                    L::Lane | L::Sysml | L::Card => {
                        window.paint_quad(quad(r, radius, fill, HAIRLINE, border, style));
                        let band = Bounds { origin: r.origin, size: size(r.size.width, head) };
                        let band_fill = if look == L::Lane { ink.opacity(0.35) } else { gpui_kit::transparent_black() };
                        window.paint_quad(quad(band, radius, band_fill, gpui_kit::Pixels::ZERO, gpui_kit::transparent_black(), style));
                        let rule = Bounds { origin: point(r.origin.x, r.origin.y + head), size: size(r.size.width, HAIRLINE) };
                        window.paint_quad(quad(rule, gpui_kit::Pixels::ZERO, border, gpui_kit::Pixels::ZERO, gpui_kit::transparent_black(), style));
                        body = Bounds { origin: point(r.origin.x, r.origin.y + head), size: size(r.size.width, r.size.height - head) };
                    }
                    _ => window.paint_quad(quad(r, radius, fill, HAIRLINE, border, style)),
                }
                if let Some(icon) = &icon {
                    let c = body.center();
                    let at = Bounds { origin: point(c.x - ICON_SM / 2.0, c.y - ICON_SM / 2.0), size: size(ICON_SM, ICON_SM) };
                    window.paint_svg(at, kit::icon_path(icon), None, gpui_kit::TransformationMatrix::unit(), ink, cx).ok();
                }
            },
        )
        .w(TILE_GLYPH_W)
        .h(TILE_GLYPH_H)
        .into_any_element();
    }
    let (shape, outline, def) = {
        let reg = graphing_scene::stencils::registry();
        let def = reg.resolve(Some(stencil));
        (def.shape(), def.path(), def.clone())
    };
    // A pack's own colors (C4 blue, sticky yellow) show on its tile.
    let own = |key: &str| match graphing_model::find_prop(&def.defaults, key) {
        Some(graphing_model::Value::Color(c)) => u32::from_str_radix(c.trim_start_matches('#'), 16).ok().map(|c| gpui_kit::Hsla::from(gpui_kit::rgb(c))),
        _ => None,
    };
    let (fill, stroke) = (own("fill").unwrap_or(fill), own("stroke").unwrap_or(stroke));
    // Width over height of the shape as packs size it.
    let aspect = def.size.map(|(w, h)| (w / h.max(1.0)) as f32).or_else(|| outline.as_ref().map(|o| (o.view.size.w / o.view.size.h.max(f64::EPSILON)) as f32));
    canvas(
        |_, _, _| {},
        move |b, (), window, cx| {
            let mut inset = gpui_kit::Bounds {
                origin: gpui_kit::point(b.origin.x + GAP_0, b.origin.y + GAP_0),
                size: gpui_kit::size(b.size.width - GAP_0 * 2.0, b.size.height - GAP_0 * 2.0),
            };
            // Keep the stencil's own proportions (a person stays a person,
            // a mux stays a bar) instead of filling the tile.
            if let Some(aspect) = aspect {
                let (w, h) = (inset.size.width, inset.size.height);
                let (fw, fh) = if aspect > f32::from(w) / f32::from(h) { (w, w / aspect) } else { (h * aspect, h) };
                inset = gpui_kit::Bounds { origin: gpui_kit::point(inset.origin.x + (w - fw) / 2.0, inset.origin.y + (h - fh) / 2.0), size: gpui_kit::size(fw, fh) };
            }
            match &outline {
                Some(o) => crate::paint::preview_path(o, inset, fill, stroke, window),
                None => crate::paint::preview(shape, inset, fill, stroke, window),
            }
            crate::paint::preview_overlays(&def, inset, stroke, window, cx);
        },
    )
    .w(TILE_GLYPH_W)
    .h(TILE_GLYPH_H)
    .into_any_element()
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let k = cx.ui();
        let title = format!("{}{} - graphing", if self.view().read(cx).is_dirty() { "* " } else { "" }, self.view().read(cx).title());
        window.set_window_title(&title);
        self.sync_inspector(window, cx);
        let titlebar = self.render_titlebar(window, cx);
        let confirm = self.render_confirm(cx);
        let templates = self.render_templates(cx);
        let block_menu = self.render_block_menu(cx);
        let notices = self.render_notices(cx);
        let body = self.dock.clone();

        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            // Undo works wherever focus is (inspector, toolbar, nothing);
            // text fields catch their own ctrl-z first.
            .on_action(cx.listener(|ws, _: &Undo, _, cx| ws.with_view(cx, |v, cx| v.undo_op(cx))))
            .on_action(cx.listener(|ws, _: &Redo, _, cx| ws.with_view(cx, |v, cx| v.redo_op(cx))))
            .on_action(cx.listener(|ws, _: &NewFile, w, cx| ws.new_tab(Document::parse(""), None, w, cx)))
            // Diagram files dropped anywhere but a canvas open.
            .on_drop(cx.listener(|ws, paths: &gpui_kit::ExternalPaths, window, cx| {
                for p in paths.paths().iter().filter(|p| crate::files::is_openable(p)) {
                    ws.open_or_import(p.clone(), window, cx);
                }
            }))
            .on_action(cx.listener(|ws, _: &OpenFile, w, cx| ws.open_dialog(w, cx)))
            .on_action(cx.listener(|ws, _: &Save, w, cx| ws.save(false, w, cx)))
            .on_action(cx.listener(|ws, _: &SaveAs, w, cx| ws.save(true, w, cx)))
            .on_action(cx.listener(|ws, _: &CloseTab, w, cx| ws.close_tab(ws.active, w, cx)))
            .on_action(cx.listener(|ws, _: &Quit, w, cx| ws.request_quit(w, cx)))
            .on_action(cx.listener(|ws, _: &NextTab, w, cx| ws.activate((ws.active + 1) % ws.tabs.len(), w, cx)))
            .on_action(cx.listener(|ws, _: &PrevTab, w, cx| ws.activate((ws.active + ws.tabs.len() - 1) % ws.tabs.len(), w, cx)))
            .on_action(cx.listener(|ws, _: &ToggleLeft, w, cx| ws.toggle_side(DockPlacement::Left, Tool::Shapes, w, cx)))
            .on_action(cx.listener(|ws, _: &ToggleRight, w, cx| ws.toggle_side(DockPlacement::Right, Tool::Inspector, w, cx)))
            .on_action(cx.listener(|ws, _: &ToggleSource, w, cx| ws.toggle_tool(Tool::Source, w, cx)))
            .on_action(cx.listener(|ws, _: &ToggleOutline, w, cx| ws.toggle_tool(Tool::Outline, w, cx)))
            .on_action(cx.listener(|ws, _: &ToggleProblems, w, cx| ws.toggle_tool(Tool::Problems, w, cx)))
            .on_action(cx.listener(|ws, _: &ResetLayout, w, cx| ws.confirm_reset_layout(w, cx)))
            .on_action(cx.listener(|ws, _: &CycleTheme, w, cx| ws.cycle_theme(w, cx)))
            .on_action(cx.listener(|ws, _: &CommandPalette, w, cx| ws.open_palette(w, cx)))
            .on_action(cx.listener(|ws, _: &Relayout, _, cx| ws.with_view(cx, |v, cx| v.relayout(cx))))
            .on_action(cx.listener(|ws, _: &AlignLeft, _, cx| ws.with_view(cx, |v, cx| v.align(Align::Left, cx))))
            .on_action(cx.listener(|ws, _: &AlignCenter, _, cx| ws.with_view(cx, |v, cx| v.align(Align::CenterX, cx))))
            .on_action(cx.listener(|ws, _: &AlignRight, _, cx| ws.with_view(cx, |v, cx| v.align(Align::Right, cx))))
            .on_action(cx.listener(|ws, _: &AlignTop, _, cx| ws.with_view(cx, |v, cx| v.align(Align::Top, cx))))
            .on_action(cx.listener(|ws, _: &AlignMiddle, _, cx| ws.with_view(cx, |v, cx| v.align(Align::CenterY, cx))))
            .on_action(cx.listener(|ws, _: &AlignBottom, _, cx| ws.with_view(cx, |v, cx| v.align(Align::Bottom, cx))))
            .on_action(cx.listener(|ws, _: &SpreadH, _, cx| ws.with_view(cx, |v, cx| v.align(Align::SpreadX, cx))))
            .on_action(cx.listener(|ws, _: &SpreadV, _, cx| ws.with_view(cx, |v, cx| v.align(Align::SpreadY, cx))))
            .on_action(cx.listener(|ws, _: &SameSize, _, cx| ws.with_view(cx, |v, cx| v.same_size(cx))))
            .on_action(cx.listener(|ws, _: &ExportSvg, w, cx| ws.export("svg", false, w, cx)))
            .on_action(cx.listener(|ws, _: &ExportPng, w, cx| ws.export("png", false, w, cx)))
            .on_action(cx.listener(|ws, _: &ExportAnimation, w, cx| ws.export("gif", true, w, cx)))
            .on_action(cx.listener(|ws, a: &crate::ExportAnimationAs, w, cx| {
                let ext = crate::ANIMATION_FORMATS.iter().find(|f| f.0 == a.format).map_or("gif", |f| f.3);
                ws.export(ext, true, w, cx);
            }))
            .on_action(cx.listener(|ws, _: &NewFromTemplate, w, cx| ws.open_templates(w, cx)))
            .on_action(cx.listener(|ws, _: &PlayAnimation, _, cx| {
                // Playing needs steps; the strip shows how to make them.
                ws.sequence_open = true;
                ws.with_view(cx, |v, cx| v.toggle_play(cx));
            }))
            .on_action(cx.listener(|ws, _: &ToggleSequence, _, cx| {
                ws.sequence_open = !ws.sequence_open;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &ExportSysml, w, cx| ws.export("sysml", false, w, cx)))
            .on_action(cx.listener(|ws, _: &ImportFile, w, cx| ws.import(None, w, cx)))
            .on_action(cx.listener(|ws, a: &ImportAs, w, cx| ws.import(graphing_import::Format::parse(&a.format), w, cx)))
            .on_action(cx.listener(|ws, _: &InsertImage, w, cx| ws.insert_image_dialog(w, cx)))
            .on_action(cx.listener(|ws, _: &SaveAsBlock, w, cx| ws.save_block(w, cx)))
            .on_action(cx.listener(|ws, a: &AddShape, w, cx| {
                let stencil = a.stencil.clone();
                ws.with_view(cx, |v, cx| v.add_shape(&stencil, w, cx));
            }))
            .on_action(cx.listener(|ws, a: &OpenRecent, w, cx| {
                if let Some(p) = ws.recent.get(a.index).cloned() {
                    ws.open_path(p, w, cx);
                }
            }))
            .on_action(cx.listener(|ws, _: &OpenSettings, w, cx| ws.open_settings(w, cx)))
            .on_action(cx.listener(|ws, _: &ReloadPacks, w, cx| {
                let errors = crate::load_packs(&[]);
                let n = graphing_scene::stencils::registry().packs.len();
                let msg = if errors.is_empty() { format!("{n} packs loaded") } else { format!("packs: {}", errors.join("; ")) };
                ws.toast(msg, w, cx);
                for t in &ws.tabs {
                    t.view.update(cx, |_, cx| cx.notify());
                }
                cx.notify();
            }))
            .on_action(cx.listener(|ws, a: &RunPluginCommand, w, cx| ws.run_plugin_command(&a.plugin, &a.command, w, cx)))
            .on_action(cx.listener(|ws, _: &ReloadPlugins, w, cx| {
                ws.start_plugins(w, cx);
                let n = ws.plugins.len();
                ws.toast(format!("{n} plugin(s) started"), w, cx);
            }))
            .on_action(cx.listener(|ws, _: &OpenPluginsFolder, w, cx| {
                let dir = crate::settings::config_dir().join("plugins");
                let _ = std::fs::create_dir_all(&dir);
                cx.open_with_system(&dir);
                ws.toast(format!("plugins live in {}; run Reload Plugins after adding one", dir.display()), w, cx);
            }))
            .on_action(cx.listener(|ws, _: &OpenPacksFolder, w, cx| {
                let dir = crate::settings::config_dir().join("packs");
                let _ = std::fs::create_dir_all(&dir);
                cx.open_with_system(&dir);
                ws.toast(format!("packs live in {}; run Reload Packs after adding one", dir.display()), w, cx);
            }))
            .on_action(cx.listener(|ws, _: &ReloadSettings, w, cx| ws.reload_settings(w, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(k.bg)
            .text_color(k.text)
            .text_size(TEXT_MD)
            .child(titlebar)
            .child(div().flex_1().min_h_0().child(body))
            .children(notices)
            .when_some(confirm, |d, c| d.child(c))
            .when_some(templates, |d, t| d.child(t))
            .children(block_menu)
            .when_some(self.palette.clone(), |d, p| {
                // The backdrop swallows scroll and clicks; clicking it closes.
                d.child(
                    div()
                        .id("palette-backdrop")
                        .absolute()
                        .inset_0()
                        .bg(k.scrim)
                        .occlude()
                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                        .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|ws, _, window, cx| {
                            ws.palette = None;
                            ws.focus_canvas(window, cx);
                            cx.notify();
                        }))
                        .flex()
                        .justify_center()
                        .items_start()
                        .pt(TITLEBAR_H * 2.0)
                        .child(div().on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(p)),
                )
            })
            .map(|el| {
                crate::trace("workspace render", started);
                el
            })
    }
}

/// Default size of a side or bottom dock.
fn dock_size(side: DockPlacement) -> gpui_kit::Pixels {
    match side {
        DockPlacement::Left => SIDEBAR_W,
        DockPlacement::Right => INSPECTOR_W,
        _ => BOTTOM_DOCK_H,
    }
}
