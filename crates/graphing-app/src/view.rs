//! The diagram canvas: owns one document, its camera, selection, drag state
//! and undo history. Editing commands live in `ops`; this file is input.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use graphing_ui::UiExt;
use gpui_kit::{
    AppContext, Bounds, ClipboardItem, Context, CursorStyle, Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollWheelEvent,
    SharedString, Styled, Subscription, Window, canvas, deferred, div, point, prelude::FluentBuilder, px, Action,
};
use graphing_ui::kit::Lucide;
use graphing_ui::menu::{self, MenuRow};

use crate::ops::{self, Align};
use crate::paint::{self, Frame, Overlay, Palette};
use crate::{
    Copy, Cut, Delete, Duplicate, Escape, FitView, GroupSelection, Ungroup, NudgeDown, NudgeDownBig, NudgeLeft, NudgeLeftBig, NudgeRight,
    NudgeRightBig, NudgeUp, NudgeUpBig, Paste, Redo, Rename, SelectAll, Undo, ZoomIn, ZoomOut, ZoomReset,
};
use graphing_dsl::Document;
use graphing_model::{Op, Placement, Rect, Size};
use graphing_scene::{self as scene, Hit, Scene};

const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 6.0;
/// Screen pixels around a handle that still count as a hit.
const HANDLE_HIT: f32 = 7.0;
const MIN_W: f64 = 40.0;
const MIN_H: f64 = 30.0;

type WPoint = graphing_model::Point;

use crate::settings::ConfirmDelete;

/// What the canvas asks of the workspace.
pub enum ViewEvent {
    /// Delete these ids once the user confirms.
    ConfirmDelete(Vec<String>),
    /// Pictures dropped or pasted at a point; the workspace decides how
    /// they are stored (packaged or linked).
    AddImages(Vec<IncomingImage>, WPoint),
    /// Another program saved the file while this one has unsaved edits.
    ChangedOnDisk,
    /// Diagram files dropped on the canvas at a point: open, insert or link.
    DiagramsDropped(Vec<PathBuf>, WPoint),
    /// Open this diagram (a link was double-clicked); when the file is not
    /// there, the copy this package keeps of it.
    OpenDiagram(PathBuf, Option<String>),
}


/// A picture on its way into the diagram.
#[derive(Debug, Clone)]
pub struct IncomingImage {
    /// File name (for the asset name), or the path when it came from disk.
    pub name: String,
    /// Where it came from on disk, if anywhere (so it can be linked).
    pub path: Option<PathBuf>,
    pub bytes: Vec<u8>,
}

impl IncomingImage {
    /// An image file from disk, if it decodes as one.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        crate::media::sniff(&bytes)?;
        Some(Self { name: path.file_name()?.to_string_lossy().to_string(), path: Some(path.to_path_buf()), bytes })
    }
}

impl gpui_kit::EventEmitter<ViewEvent> for DiagramView {}

/// Selection id of the diagram frame (SysML diagrams). Selecting it shows
/// the diagram in the inspector; renaming it sets the title.
pub const FRAME_ID: &str = "@frame";

/// Pan and zoom. Shared with the canvas closures so the first painted frame
/// can fit the content without waiting for another render.
#[derive(Clone, Copy)]
struct Camera {
    offset: Point<f32>,
    zoom: f32,
    /// Fit content on next prepaint.
    fit: bool,
    /// Keep fitting when the canvas resizes, until the user pans or zooms.
    auto: bool,
}

/// Camera that centers `content` in `bounds`, or `None` if nothing to fit.
fn fit_camera(bounds: Bounds<Pixels>, content: Option<Rect>) -> Option<Camera> {
    let r = content?;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let pad = 60.0;
    let zoom = ((w - pad * 2.0) / r.size.w as f32).min((h - pad * 2.0) / r.size.h as f32).clamp(MIN_ZOOM, 1.5);
    let offset = point(
        (w - r.size.w as f32 * zoom) / 2.0 - r.origin.x as f32 * zoom,
        (h - r.size.h as f32 * zoom) / 2.0 - r.origin.y as f32 * zoom,
    );
    Some(Camera { offset, zoom, fit: false, auto: true })
}

enum Drag {
    Pan { last: Point<Pixels> },
    Move {
        items: Vec<Item>,
        grab: WPoint,
        delta: WPoint,
        /// Scene before the drag, for regrouping; built once.
        before: std::rc::Rc<Scene>,
        /// The diagram with the drop's group changes applied and the group
        /// the nodes land in, recomputed per pointer move (None when whole
        /// groups move, which never regroups).
        preview: Option<Box<(graphing_model::Diagram, Option<String>)>>,
        regroup: bool,
    },
    Marquee { start: WPoint, current: WPoint, base: Vec<String> },
    Link { from: String, from_port: Option<String>, current: WPoint },
    Resize { id: String, anchor: WPoint, current: WPoint },
}

/// Screen pixels the pointer must travel before a press becomes a drag, so
/// a click never nudges, resizes or links anything.
const DRAG_START: f32 = 3.0;

/// One box moving with the drag.
struct Item {
    id: String,
    start: WPoint,
    size: Option<Size>,
}

/// Largest size a new picture gets, in diagram units (aspect kept).
const IMAGE_MAX_W: f64 = 360.0;
const IMAGE_MAX_H: f64 = 280.0;

/// Size of a group added from the library, in diagram units.
const NEW_GROUP_W: f64 = 360.0;
const NEW_GROUP_H: f64 = 220.0;

/// Typing pauses longer than this start a new undo step.
const TEXT_UNDO_BURST: std::time::Duration = std::time::Duration::from_millis(800);

/// One undo step: the source text from before an edit (or a burst of typing
/// in the source panel).
#[derive(Debug, Clone)]
enum Step {
    Text(String),
}

pub struct DiagramView {
    doc: Document,
    path: Option<PathBuf>,
    mtime: Option<SystemTime>,
    focus: FocusHandle,
    cam: Rc<Cell<Camera>>,
    /// Canvas bounds from the last paint, for mouse -> world mapping.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<Drag>,
    /// Where the current press started, until it passes `DRAG_START`.
    press: Option<Point<Pixels>>,
    selected: Vec<String>,
    /// Node under the mouse, for connection ports.
    hover: Option<String>,
    mouse: WPoint,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// When the last text edit landed, so typing coalesces into one step.
    last_text_edit: Option<std::time::Instant>,
    dirty: bool,
    status: SharedString,
    /// Whether deleting asks first; standalone views (tests) never do.
    confirm_delete: ConfirmDelete,
    /// Images carried in the package (`asset:<name>` -> bytes).
    assets: crate::files::Assets,
    /// When moving pictures play, and the clock their frames follow.
    play: crate::settings::Play,
    started: std::time::Instant,
    /// A repaint for the next animation frame is already on its way.
    anim_pending: bool,
    /// Animation playback or preview; `None` while editing.
    pub(crate) player: Option<crate::sequence::Player>,
    pub(crate) tick_pending: bool,
    /// Diagrams that links point at, by `src`: when last read, and what.
    pub(crate) linked: Rc<std::cell::RefCell<paint::Linked>>,
    /// The minimap's mapping and whether it is being dragged.
    minimap: Rc<Cell<minimap::MiniMap>>,
    minimap_drag: bool,
    /// Decoded pictures by `src`, shared with the paint closure.
    images: std::rc::Rc<std::cell::RefCell<HashMap<String, Option<std::sync::Arc<gpui_kit::RenderImage>>>>>,
    /// Grid step in diagram units, whether drags snap to it, whether it shows.
    grid: f64,
    snap_on: bool,
    show_grid: bool,
    /// Inline label editor and the id it edits.
    rename: Entity<InputState>,
    renaming: Option<String>,
    /// The right-click menu, at this window position.
    menu: Option<Point<Pixels>>,
    /// Where a right press started: released in place it opens the menu,
    /// dragged it pans.
    right_press: Option<Point<Pixels>>,
    _subs: Vec<Subscription>,
}

impl DiagramView {
    pub fn new(doc: Document, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let mtime = path.as_ref().and_then(|p| std::fs::metadata(p).ok()?.modified().ok());
        let rename = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_text_align(gpui_kit::TextAlign::Center, cx);
            s
        });
        let sub = cx.subscribe_in(&rename, window, |v: &mut Self, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => v.commit_rename(window, cx),
            _ => {}
        });
        let mut view = Self {
            doc,
            path,
            mtime,
            focus,
            cam: Rc::new(Cell::new(Camera { offset: point(0.0, 0.0), zoom: 1.0, fit: true, auto: true })),
            bounds: Rc::new(Cell::new(Bounds::default())),
            drag: None,
            press: None,
            menu: None,
            right_press: None,
            selected: Vec::new(),
            hover: None,
            mouse: WPoint::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_text_edit: None,
            dirty: false,
            status: SharedString::default(),
            confirm_delete: ConfirmDelete::Never,
            assets: Default::default(),
            play: crate::settings::Play::default(),
            started: std::time::Instant::now(),
            anim_pending: false,
            player: None,
            tick_pending: false,
            linked: Default::default(),
            minimap: Rc::new(Cell::new(minimap::MiniMap::default())),
            minimap_drag: false,
            images: Default::default(),
            grid: scene::GRID,
            snap_on: true,
            show_grid: true,
            rename,
            renaming: None,
            _subs: vec![sub],
        };
        view.place_missing();
        view.watch(cx);
        view
    }

    // ---- accessors for the workspace ----

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Window position of a diagram point, for tests that click the canvas.
    #[cfg(test)]
    pub(crate) fn screen_point(&self, p: WPoint) -> gpui_kit::Point<gpui_kit::Pixels> {
        let o = self.bounds.get().origin;
        let c = self.cam.get();
        gpui_kit::point(o.x + gpui_kit::px(c.offset.x + p.x as f32 * c.zoom), o.y + gpui_kit::px(c.offset.y + p.y as f32 * c.zoom))
    }

    pub fn selection(&self) -> &[String] {
        &self.selected
    }

    /// Debug bench: drag the selection `step` grid steps right, as a pointer
    /// drag would (`GRAPHING_DEBUG_OPEN=drag-bench:<id>`).
    pub(crate) fn bench_drag(&mut self, step: usize, cx: &mut Context<Self>) {
        if !matches!(self.drag, Some(Drag::Move { .. })) {
            let scene = self.scene();
            let items = self.drag_items(&scene);
            let regroup = !self.selected.iter().any(|id| self.doc.diagram().group(id).is_some());
            let before = std::rc::Rc::new(self.scene_static());
            self.drag = Some(Drag::Move { items, grab: WPoint::default(), delta: WPoint::default(), before, preview: None, regroup });
        }
        let step_size = self.grid;
        if let Some(Drag::Move { delta, .. }) = &mut self.drag {
            *delta = WPoint::new(step as f64 * step_size, 0.0);
        }
        cx.notify();
    }

    /// Canvas settings: grid step, snapping, grid dots.
    pub fn set_grid(&mut self, step: f64, snap_on: bool, show: bool, cx: &mut Context<Self>) {
        self.grid = step.max(1.0);
        self.snap_on = snap_on;
        self.show_grid = show;
        cx.notify();
    }

    pub fn status(&self) -> &SharedString {
        &self.status
    }

    pub fn zoom(&self) -> f32 {
        self.cam.get().zoom
    }

    pub fn mouse(&self) -> WPoint {
        self.mouse
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn title(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or("untitled".into(), |n| n.to_string_lossy().to_string())
    }

    pub fn set_status(&mut self, s: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = s.into();
        cx.notify();
    }

    /// Text edited in the source panel.
    pub fn set_source(&mut self, src: String, cx: &mut Context<Self>) {
        if src == self.doc.source() {
            return;
        }
        // Source panel typing is one history with canvas edits: a burst of
        // keystrokes becomes one step holding the text from before it.
        let burst = self.last_text_edit.is_some_and(|t| t.elapsed() < TEXT_UNDO_BURST);
        if !burst || !matches!(self.undo.last(), Some(Step::Text(_))) {
            self.undo.push(Step::Text(self.doc.source().to_string()));
        }
        self.redo.clear();
        self.last_text_edit = Some(std::time::Instant::now());
        self.doc.set_source(src);
        self.place_missing();
        let d = self.doc.diagram();
        self.selected.retain(|id| id == FRAME_ID || d.node(id).is_some() || d.edge(id).is_some() || d.group(id).is_some());
        self.dirty = true;
        cx.notify();
    }

    /// Apply an edit with undo. Returns false if it did not apply.
    pub fn apply(&mut self, op: Op, cx: &mut Context<Self>) -> bool {
        // Undo restores the text exactly (statement order, formatting), which
        // replaying an op's inverse cannot always do.
        let before = self.doc.source().to_string();
        match self.doc.apply(&op) {
            Some(_) => {
                self.undo.push(Step::Text(before));
                self.redo.clear();
                self.last_text_edit = None;
                self.dirty = true;
                cx.notify();
                true
            }
            None => false,
        }
    }

    pub fn select(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.selected = ids;
        cx.notify();
    }

    /// Select and scroll so `id` is in the middle of the canvas.
    pub fn reveal(&mut self, id: &str, cx: &mut Context<Self>) {
        let s = self.scene();
        let r = s.rect_of(id).or_else(|| {
            let e = s.edges.iter().find(|e| e.id == id)?;
            Some(scene::rect_from(e.points[0], *e.points.last()?))
        });
        let Some(r) = r else { return };
        let b = self.bounds.get();
        let mut c = self.cam.get();
        let center = r.center();
        c.offset.x = f32::from(b.size.width) / 2.0 - center.x as f32 * c.zoom;
        c.offset.y = f32::from(b.size.height) / 2.0 - center.y as f32 * c.zoom;
        self.cam.set(c);
        self.selected = vec![id.to_string()];
        cx.notify();
    }

    /// New node of `stencil` in the middle of the visible area.
    pub fn add_shape(&mut self, stencil: &str, window: &mut Window, cx: &mut Context<Self>) {
        let b = self.bounds.get();
        let center = self.to_world(b.center());
        self.add_shape_at(stencil, center, window, cx);
    }

    /// New node of `stencil` centered on `center` (world units).
    pub fn add_shape_at(&mut self, stencil: &str, center: WPoint, window: &mut Window, cx: &mut Context<Self>) {
        // Library "Containers" tiles are `group:<look>`.
        if let Some(kind) = stencil.strip_prefix("group:") {
            let (w, h) = (NEW_GROUP_W, NEW_GROUP_H);
            let rect = Rect::new((center.x - w / 2.0).round(), (center.y - h / 2.0).round(), w, h);
            let (id, op) = ops::add_group(self.doc.diagram(), &self.scene_static(), kind, rect);
            if self.apply(op, cx) {
                self.selected = vec![id.clone()];
                self.start_rename(&id, window, cx);
            }
            return;
        }
        let probe = graphing_model::Node { stencil: Some(stencil.to_string()), ..graphing_model::Node::new("probe") };
        let (w, h) = graphing_scene::notation::node_spec(self.doc.diagram(), &probe).min;
        let at = WPoint::new(center.x - w / 2.0, center.y - h / 2.0);
        let (id, op) = ops::add_node(self.doc.diagram(), Some(stencil), at);
        if self.apply(op, cx) {
            self.selected = vec![id.clone()];
            self.start_rename(&id, window, cx);
        }
    }

    pub fn group_selection(&mut self, cx: &mut Context<Self>) {
        if let Some((id, op)) = ops::group_selection(self.doc.diagram(), &self.selected)
            && self.apply(op, cx)
        {
            self.selected = vec![id];
        }
    }

    pub fn ungroup_selection(&mut self, cx: &mut Context<Self>) {
        let groups: Vec<String> = self.selected.iter().filter(|id| self.doc.diagram().group(id).is_some()).cloned().collect();
        let mut members = Vec::new();
        let mut all = Vec::new();
        for g in &groups {
            members.extend(self.doc.diagram().group(g).map(|g| g.members.clone()).unwrap_or_default());
            let mut probe = self.doc.diagram().clone();
            for op in &all {
                probe.apply(op);
            }
            if let Some(op) = ops::ungroup(&probe, g) {
                all.push(op);
            }
        }
        if !all.is_empty() && self.apply(Op::Batch(all), cx) {
            self.selected = members;
        }
    }

    pub fn align(&mut self, how: Align, cx: &mut Context<Self>) {
        let scene = self.scene();
        if let Some(op) = ops::align(&scene, self.doc.diagram(), &self.selected, how) {
            self.apply(op, cx);
        }
    }

    pub fn same_size(&mut self, cx: &mut Context<Self>) {
        let scene = self.scene();
        if let Some(op) = ops::same_size(&scene, self.doc.diagram(), &self.selected) {
            self.apply(op, cx);
        }
    }

    /// Re-run auto placement for every node (one-shot layout).
    pub fn relayout(&mut self, cx: &mut Context<Self>) {
        let mut cleared = self.doc.diagram().clone();
        cleared.layout.retain(|id, _| cleared.groups.iter().any(|g| &g.id == id));
        let ops: Vec<Op> = scene::auto_place(&cleared)
            .into_iter()
            .map(|(id, pos)| {
                let size = self.doc.diagram().layout.get(&id).and_then(|p| p.size);
                Op::SetPlacement { id, placement: Some(Placement { pos, size }) }
            })
            .collect();
        if !ops.is_empty() && self.apply(Op::Batch(ops), cx) {
            self.request_fit(cx);
        }
    }

    pub fn request_fit(&mut self, cx: &mut Context<Self>) {
        let mut c = self.cam.get();
        c.fit = true;
        self.cam.set(c);
        cx.notify();
    }

    pub fn undo_op(&mut self, cx: &mut Context<Self>) {
        if let Some(step) = self.undo.pop()
            && let Some(inv) = self.replay(step)
        {
            self.redo.push(inv);
            self.dirty = true;
            cx.notify();
        }
    }

    pub fn redo_op(&mut self, cx: &mut Context<Self>) {
        if let Some(step) = self.redo.pop()
            && let Some(inv) = self.replay(step)
        {
            self.undo.push(inv);
            self.dirty = true;
            cx.notify();
        }
    }

    /// Apply a history step, returning the step that reverses it.
    fn replay(&mut self, step: Step) -> Option<Step> {
        self.last_text_edit = None;
        let inv = match step {
            Step::Text(src) => {
                let before = self.doc.source().to_string();
                self.doc.set_source(src);
                Step::Text(before)
            }
        };
        let d = self.doc.diagram();
        self.selected.retain(|id| id == FRAME_ID || d.node(id).is_some() || d.edge(id).is_some() || d.group(id).is_some());
        Some(inv)
    }

    pub fn save_to(&mut self, path: PathBuf, cx: &mut Context<Self>) -> std::io::Result<()> {
        crate::files::save(&path, self.doc.source(), &mut self.assets)?;
        self.mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        let first = self.path.is_none();
        self.path = Some(path);
        self.dirty = false;
        self.status = "saved".into();
        if first {
            self.watch(cx);
        }
        cx.notify();
        Ok(())
    }

    pub fn scene(&self) -> Scene {
        // Previewing a step shows (and hit-tests) shapes where it moved them.
        let mut moved = self.preview_moved();
        let mut sized = None;
        match &self.drag {
            Some(Drag::Move { items, delta, preview, .. }) => {
                for it in items {
                    moved.insert(it.id.clone(), WPoint::new(it.start.x + delta.x, it.start.y + delta.y));
                }
                // Show the drop's group changes live: a node dragged out of
                // its group leaves it (the group stops growing to follow).
                if let Some(p) = preview {
                    return scene::build(&p.0, &moved);
                }
            }
            Some(Drag::Resize { id, anchor, current }) => sized = Some((id.clone(), resize_rect(*anchor, *current))),
            _ => {}
        }
        // A live resize goes into the layout before building, so edges,
        // ports and fitted groups follow it instead of jumping on release.
        match sized {
            Some((id, r)) => {
                let mut d = self.doc.diagram().clone();
                d.layout.insert(id, Placement { pos: r.origin, size: Some(Size::new(r.size.w, r.size.h)) });
                scene::build(&d, &moved)
            }
            None => scene::build(self.doc.diagram(), &moved),
        }
    }

    /// The diagram as a move would leave it (group joins and leaves
    /// applied), and the group the first moved node would land in.
    fn move_preview(&self, before: &Scene, items: &[Item], delta: WPoint) -> (graphing_model::Diagram, Option<String>) {
        let centers: Vec<(String, WPoint)> = items
            .iter()
            .filter_map(|it| {
                let r = before.rect_of(&it.id)?;
                Some((it.id.clone(), WPoint::new(it.start.x + delta.x + r.size.w / 2.0, it.start.y + delta.y + r.size.h / 2.0)))
            })
            .collect();
        let mut d = self.doc.diagram().clone();
        for op in ops::regroup_after_move(&d, before, &centers) {
            d.apply(&op);
        }
        let target = centers.iter().find(|(id, _)| d.node(id).is_some()).and_then(|(id, _)| ops::parent_group(&d, id).map(|g| g.id.clone()));
        (d, target)
    }

    // ---- loading and disk ----

    /// Nodes without a layout entry get one now; written on next save.
    fn place_missing(&mut self) {
        let placed = scene::auto_place(self.doc.diagram());
        if placed.is_empty() {
            return;
        }
        let n = placed.len();
        let ops = placed
            .into_iter()
            .map(|(id, pos)| Op::SetPlacement { id, placement: Some(Placement { pos, size: None }) })
            .collect();
        self.doc.apply(&Op::Batch(ops));
        self.status = format!("placed {n} new node(s)").into();
    }

    /// Reload when the file changes on disk and we have no unsaved edits, so
    /// editing the text elsewhere updates the canvas live.
    fn watch(&self, cx: &mut Context<Self>) {
        if self.path.is_none() {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let Ok(()) = this.update(cx, |v, cx| v.check_disk(cx)) else { break };
            }
        })
        .detach();
    }

    pub(crate) fn check_disk(&mut self, cx: &mut Context<Self>) {
        let Some(path) = &self.path else { return };
        let Some(m) = std::fs::metadata(path).ok().and_then(|m| m.modified().ok()) else { return };
        if Some(m) == self.mtime {
            return;
        }
        self.mtime = Some(m);
        if self.dirty {
            // Unsaved edits here: the workspace asks which to keep.
            cx.emit(ViewEvent::ChangedOnDisk);
            return;
        }
        self.reload_from_disk(cx);
    }

    /// Replace the diagram with what is on disk, dropping unsaved edits.
    pub fn reload_from_disk(&mut self, cx: &mut Context<Self>) {
        let Some(path) = &self.path else { return };
        let Ok((src, assets)) = crate::files::load(path) else { return };
        // Whatever was unsaved is gone now: the file is the truth again.
        self.dirty = false;
        cx.notify();
        if src != self.doc.source() || assets != self.assets {
            if assets != self.assets {
                self.assets = assets;
                self.images.borrow_mut().clear();
            }
            self.doc.set_source(src);
            self.place_missing();
            self.undo.clear();
            self.redo.clear();
            let d = self.doc.diagram();
            self.selected.retain(|id| id == FRAME_ID || d.node(id).is_some() || d.edge(id).is_some() || d.group(id).is_some());
            self.status = "reloaded".into();
            cx.notify();
        }
    }

    // ---- coordinates ----

    fn to_world(&self, p: Point<Pixels>) -> WPoint {
        let o = self.bounds.get().origin;
        let c = self.cam.get();
        WPoint::new(((f32::from(p.x - o.x) - c.offset.x) / c.zoom) as f64, ((f32::from(p.y - o.y) - c.offset.y) / c.zoom) as f64)
    }

    fn to_screen(&self, p: WPoint) -> Point<Pixels> {
        let o = self.bounds.get().origin;
        let c = self.cam.get();
        point(o.x + px(c.offset.x + p.x as f32 * c.zoom), o.y + px(c.offset.y + p.y as f32 * c.zoom))
    }

    /// World distance covering `screen` pixels at the current zoom.
    fn tol(&self, screen: f32) -> f64 {
        (screen / self.cam.get().zoom) as f64
    }

    fn zoom_at(&mut self, factor: f32, at: Point<Pixels>) {
        let o = self.bounds.get().origin;
        let (sx, sy) = (f32::from(at.x - o.x), f32::from(at.y - o.y));
        let mut c = self.cam.get();
        let z = (c.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        // Keep the world point under the cursor fixed.
        c.offset.x = sx - (sx - c.offset.x) * z / c.zoom;
        c.offset.y = sy - (sy - c.offset.y) * z / c.zoom;
        c.zoom = z;
        c.auto = false;
        self.cam.set(c);
    }

    fn pan(&self, dx: f32, dy: f32) {
        let mut c = self.cam.get();
        c.offset.x += dx;
        c.offset.y += dy;
        c.auto = false;
        self.cam.set(c);
    }

    pub fn zoom_center(&mut self, factor: f32, cx: &mut Context<Self>) {
        let b = self.bounds.get();
        self.zoom_at(factor, b.center());
        cx.notify();
    }

    pub fn zoom_reset(&mut self, cx: &mut Context<Self>) {
        let b = self.bounds.get();
        self.zoom_at(1.0 / self.cam.get().zoom, b.center());
        cx.notify();
    }

    // ---- rename ----

    fn label_of(&self, id: &str) -> Option<Option<String>> {
        let d = self.doc.diagram();
        if id == FRAME_ID {
            return Some(d.title.clone());
        }
        d.node(id)
            .map(|n| n.label.clone())
            .or_else(|| d.edge(id).map(|e| e.label.clone()))
            .or_else(|| d.group(id).map(|g| g.label.clone()))
    }

    pub fn start_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.label_of(id) else { return };
        // Multi-line labels edit with `\n` escapes in a single-line field.
        let text = current.unwrap_or_default().replace('\n', "\\n");
        let align = if self.label_spot(id).is_some_and(|s| s.left) { gpui_kit::TextAlign::Left } else { gpui_kit::TextAlign::Center };
        self.renaming = Some(id.to_string());
        self.rename.update(cx, |s, cx| {
            s.set_text_align(align, cx);
            s.set_value(text, window, cx);
            s.select_all(window, cx);
        });
        // After the current event: the canvas takes focus on mouse down,
        // which would otherwise land after this and steal it back.
        cx.defer_in(window, |v, window, cx| {
            v.rename.update(cx, |s, cx| s.focus(window, cx));
        });
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.renaming.take() else { return };
        let text = self.rename.read(cx).value().to_string().replace("\\n", "\n");
        let label = if text.trim().is_empty() { None } else { Some(text) };
        if self.label_of(&id).is_some_and(|c| c != label) {
            let op = if id == FRAME_ID { Op::SetTitle { title: label } } else { Op::SetLabel { id, label } };
            self.apply(op, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // ---- context menu ----

    fn right_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.commit_rename(window, cx);
        }
        window.focus(&self.focus, cx);
        self.menu = None;
        self.right_press = Some(ev.position);
        self.drag = Some(Drag::Pan { last: ev.position });
        cx.notify();
    }

    /// Released where it was pressed: the menu for what is under the
    /// pointer (selecting it first, as file managers do). Dragged: a pan.
    fn right_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let press = self.right_press.take();
        self.drag = None;
        let still = press.is_some_and(|p| f32::from(p.x - ev.position.x).hypot(f32::from(p.y - ev.position.y)) < DRAG_START);
        if still {
            self.open_menu(ev.position, cx);
        }
        cx.notify();
    }

    pub(crate) fn open_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let world = self.to_world(at);
        let scene = self.scene();
        let hit = match scene.hit(world) {
            Some(Hit::Node(id)) => Some(id),
            other => scene.hit_edge(world, self.tol(6.0)).or_else(|| other.map(|h| h.id().to_string())),
        };
        let hit = hit.or_else(|| Self::on_frame_edge(&scene, world, self.tol(6.0)).then(|| FRAME_ID.to_string()));
        match hit {
            Some(id) if !self.selected.contains(&id) => self.selected = vec![id],
            Some(_) => {}
            None => self.selected.clear(),
        }
        self.menu = Some(at);
        cx.notify();
    }

    /// Debug hook: right-click the middle of `id` (or the canvas's middle).
    pub(crate) fn debug_context(&mut self, id: &str, cx: &mut Context<Self>) {
        let b = self.bounds.get();
        let at = self.scene().rect_of(id).map(|r| self.to_screen(WPoint::new(r.origin.x + r.size.w / 2.0, r.origin.y + r.size.h / 2.0))).unwrap_or_else(|| b.center());
        self.open_menu(at, cx);
    }

    fn run_menu(&mut self, action: Box<dyn Action>, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        window.focus(&self.focus, cx);
        window.dispatch_action(action, cx);
        cx.notify();
    }

    /// What the menu offers for the selection (or the empty canvas).
    fn menu_rows(&self, window: &Window, cx: &mut Context<Self>) -> Vec<MenuRow> {
        use Lucide as L;
        let d = self.doc.diagram();
        let sel = &self.selected;
        let nodes = sel.iter().filter(|id| d.node(id).is_some()).count();
        let groups = sel.iter().filter(|id| d.group(id).is_some()).count();
        let focus = self.focus.clone();
        let item = |label: &'static str, icon: Lucide, action: Box<dyn Action>| {
            let keys: SharedString = window
                .bindings_for_action_in(&*action, &focus)
                .last()
                .map(|b| b.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" "))
                .unwrap_or_default()
                .into();
            MenuRow::item(label, cx.listener(move |v, _, window, cx| v.run_menu(action.boxed_clone(), window, cx))).icon(icon).keys(keys)
        };
        let mut sections: Vec<Vec<MenuRow>> = Vec::new();
        if sel.is_empty() {
            sections.push(vec![item("Paste", L::ClipboardPaste, Box::new(Paste)), item("Select All", L::SquareDashedMousePointer, Box::new(SelectAll))]);
            sections.push(vec![item("Insert Image...", L::ImagePlus, Box::new(crate::InsertImage))]);
            sections.push(vec![
                item("Fit to Window", L::Scan, Box::new(FitView)),
                item("Actual Size", L::Search, Box::new(ZoomReset)),
                item("Auto Layout", L::Workflow, Box::new(crate::Relayout)),
            ]);
        } else if sel.len() == 1 && sel[0] == FRAME_ID {
            sections.push(vec![item("Rename", L::PencilLine, Box::new(Rename))]);
        } else {
            let mut edit = Vec::new();
            if sel.len() == 1 {
                edit.push(item("Rename", L::PencilLine, Box::new(Rename)));
            }
            edit.push(item("Cut", L::Scissors, Box::new(Cut)));
            edit.push(item("Copy", L::Copy, Box::new(Copy)));
            if nodes + groups > 0 {
                edit.push(item("Duplicate", L::CopyPlus, Box::new(Duplicate)));
            }
            sections.push(edit);
            if nodes + groups >= 2 {
                sections.push(vec![
                    item("Align Left", L::AlignStartVertical, Box::new(crate::AlignLeft)),
                    item("Align Center", L::AlignCenterVertical, Box::new(crate::AlignCenter)),
                    item("Align Top", L::AlignStartHorizontal, Box::new(crate::AlignTop)),
                    item("Align Middle", L::AlignCenterHorizontal, Box::new(crate::AlignMiddle)),
                ]);
            }
            let mut arrange = Vec::new();
            if nodes > 0 {
                arrange.push(item("Group Selection", L::Group, Box::new(crate::GroupSelection)));
            }
            if groups > 0 {
                arrange.push(item("Ungroup", L::Ungroup, Box::new(crate::Ungroup)));
            }
            sections.push(arrange);
            sections.push(vec![item("Delete", L::Trash, Box::new(Delete))]);
        }
        let mut rows = Vec::new();
        for section in sections.into_iter().filter(|s| !s.is_empty()) {
            if !rows.is_empty() {
                rows.push(MenuRow::Separator);
            }
            rows.extend(section);
        }
        rows
    }

    fn context_menu(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        use graphing_ui::tokens::*;
        let at = self.menu?;
        let rows = self.menu_rows(window, cx);
        let b = self.bounds.get();
        // Keep it on the canvas: open leftward or upward near an edge.
        let items = rows.iter().filter(|r| matches!(r, MenuRow::Item { .. })).count() as f32;
        let tall = MENU_ROW_H * items + GAP_4 * 2.0;
        let mut left = at.x - b.origin.x;
        let mut top = at.y - b.origin.y;
        if left + MENU_W > b.size.width {
            left = (left - MENU_W).max(Pixels::ZERO);
        }
        if top + tall > b.size.height {
            top = (top - tall).max(Pixels::ZERO);
        }
        let surface = menu::menu_surface("canvas-menu", rows, cx).debug_selector(|| "canvas-menu".into()).on_mouse_down_out(cx.listener(|v, _, _, cx| {
            v.menu = None;
            cx.notify();
        }));
        Some(div().absolute().left(left).top(top).child(deferred(menu::animate(surface, "canvas-menu-anim")).with_priority(2)))
    }

    // ---- mouse ----

    fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            self.commit_rename(window, cx);
        }
        // A click stops playback; while previewing a step, shapes can still
        // be picked (dragging one records a move), empty canvas ends it.
        if self.playing() {
            self.stop_animation(cx);
        }
        window.focus(&self.focus, cx);
        let world = self.to_world(ev.position);
        let scene = self.scene();
        let shift = ev.modifiers.shift;
        let double = ev.click_count >= 2;
        self.press = Some(ev.position);

        // Resize handles of a single selected node or group.
        if let [id] = self.selected.as_slice()
            && let Some(r) = scene.rect_of(id)
            && let Some(anchor) = self.corner_hit(r, world)
        {
            self.drag = Some(Drag::Resize { id: id.clone(), anchor, current: world });
            cx.notify();
            return;
        }
        // A port square: connect from that port.
        let port_tol = self.tol(HANDLE_HIT).max(graphing_scene::PORT / 2.0);
        if let Some((node, port)) = scene.nodes.iter().rev().find_map(|n| {
            n.ports.iter().find(|p| (p.at.x - world.x).abs() <= port_tol && (p.at.y - world.y).abs() <= port_tol).map(|p| (n.id.clone(), p.name.clone()))
        }) {
            self.drag = Some(Drag::Link { from: node, from_port: Some(port), current: world });
            cx.notify();
            return;
        }
        // Connection handles of the hovered node.
        if let Some(h) = self.hover.clone()
            && let Some(r) = scene.rect_of(&h)
            && scene::ports(r).iter().any(|p| (p.x - world.x).hypot(p.y - world.y) <= self.tol(HANDLE_HIT))
        {
            self.drag = Some(Drag::Link { from: h, from_port: None, current: world });
            cx.notify();
            return;
        }

        let hit = match scene.hit(world) {
            Some(Hit::Node(id)) => Some(Hit::Node(id)),
            other => scene.hit_edge(world, self.tol(6.0)).map(Hit::Edge).or(other),
        };
        // Double-click in a group's open area adds a node inside it; only its
        // title strip renames it.
        let hit = match hit {
            Some(Hit::Group(g)) if double && !Self::on_group_title(&scene, &g, world) => {
                let at = WPoint::new(world.x - 60.0, world.y - 28.0);
                let (id, add) = ops::add_node(self.doc.diagram(), None, at);
                let mut probe = self.doc.diagram().clone();
                probe.apply(&add);
                let join = ops::set_group(&probe, &id, Some(&g));
                let op = Op::Batch(std::iter::once(add).chain(join).collect());
                if self.apply(op, cx) {
                    self.selected = vec![id.clone()];
                    self.start_rename(&id, window, cx);
                }
                return;
            }
            other => other,
        };
        // The frame's header strip and border select the diagram itself.
        if hit.is_none() && Self::on_frame_edge(&scene, world, self.tol(6.0)) {
            self.selected = vec![FRAME_ID.to_string()];
            if double {
                self.start_rename(FRAME_ID, window, cx);
            }
            cx.notify();
            return;
        }
        match hit {
            Some(hit) => {
                let id = hit.id().to_string();
                // Double-clicking a diagram link opens what it points at.
                if double && let Some(path) = self.link_target(&scene, &id) {
                    let src = scene.nodes.iter().find(|n| n.id == id).and_then(|n| n.reference.clone()).unwrap_or_default();
                    let copy = self.assets.get(&graphing_package::linked_name(&src)).map(|b| String::from_utf8_lossy(b).into_owned());
                    cx.emit(ViewEvent::OpenDiagram(path, copy));
                    return;
                }
                if double {
                    self.selected = vec![id.clone()];
                    self.start_rename(&id, window, cx);
                    return;
                }
                if shift {
                    if let Some(i) = self.selected.iter().position(|s| *s == id) {
                        self.selected.remove(i);
                    } else {
                        self.selected.push(id);
                    }
                } else {
                    if !self.selected.contains(&id) {
                        self.selected = vec![id.clone()];
                    }
                    if !matches!(hit, Hit::Edge(_)) {
                        let items = self.drag_items(&scene);
                        // Moving whole groups never changes membership.
                        let regroup = !self.selected.iter().any(|id| self.doc.diagram().group(id).is_some());
                        let before = std::rc::Rc::new(self.scene_static());
                        self.drag = Some(Drag::Move { items, grab: world, delta: WPoint::default(), before, preview: None, regroup });
                    }
                }
            }
            None if double => {
                let at = WPoint::new(world.x - 60.0, world.y - 28.0);
                let (id, op) = ops::add_node(self.doc.diagram(), None, at);
                if self.apply(op, cx) {
                    self.selected = vec![id.clone()];
                    self.start_rename(&id, window, cx);
                }
                return;
            }
            None => {
                self.stop_animation(cx);
                let base = if shift { self.selected.clone() } else { Vec::new() };
                self.selected = base.clone();
                self.drag = Some(Drag::Marquee { start: world, current: world, base });
            }
        }
        cx.notify();
    }

    fn on_frame_edge(scene: &Scene, p: WPoint, tol: f64) -> bool {
        let Some(f) = &scene.frame else { return false };
        let r = f.rect;
        let (x0, y0, x1, y1) = (r.origin.x, r.origin.y, r.origin.x + r.size.w, r.origin.y + r.size.h);
        let inside = p.x >= x0 - tol && p.x <= x1 + tol && p.y >= y0 - tol && p.y <= y1 + tol;
        let header = p.y <= y0 + graphing_scene::FRAME_HEAD;
        let border = (p.x - x0).abs() <= tol || (p.x - x1).abs() <= tol || (p.y - y0).abs() <= tol || (p.y - y1).abs() <= tol;
        inside && (header || border)
    }

    fn on_group_title(scene: &Scene, id: &str, p: WPoint) -> bool {
        scene.groups.iter().find(|g| g.id == id).is_some_and(|g| p.y - g.rect.origin.y <= g.look.head())
    }

    /// Corner of `r` under `p`, returned as the opposite (fixed) corner.
    fn corner_hit(&self, r: Rect, p: WPoint) -> Option<WPoint> {
        let (x0, y0, x1, y1) = (r.origin.x, r.origin.y, r.origin.x + r.size.w, r.origin.y + r.size.h);
        let t = self.tol(HANDLE_HIT);
        [(x0, y0, x1, y1), (x1, y0, x0, y1), (x1, y1, x0, y0), (x0, y1, x1, y0)]
            .into_iter()
            .find(|&(hx, hy, _, _)| (hx - p.x).abs() <= t && (hy - p.y).abs() <= t)
            .map(|(_, _, ax, ay)| WPoint::new(ax, ay))
    }

    /// Everything selected that can move: nodes, plus groups' members.
    fn drag_items(&self, scene: &Scene) -> Vec<Item> {
        let d = self.doc.diagram();
        let mut ids: Vec<String> = Vec::new();
        let mut stack: Vec<String> = Vec::new();
        for id in &self.selected {
            if d.node(id).is_some() {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            } else if d.group(id).is_some() {
                stack.push(id.clone());
            }
        }
        while let Some(g) = stack.pop() {
            // Fitted groups follow their members; sized ones move too.
            if d.layout.get(&g).is_some_and(|p| p.size.is_some()) {
                ids.push(g.clone());
            }
            for m in d.group(&g).map(|g| g.members.clone()).unwrap_or_default() {
                if d.group(&m).is_some() {
                    stack.push(m);
                } else if !ids.contains(&m) {
                    ids.push(m);
                }
            }
        }
        ids.into_iter()
            .filter_map(|id| {
                let r = scene.rect_of(&id)?;
                let size = d.layout.get(&id).and_then(|p| p.size);
                Some(Item { id, start: r.origin, size })
            })
            .collect()
    }

    fn mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let world = self.to_world(ev.position);
        self.mouse = world;
        // Below the drag threshold a press stays a click.
        if let Some(start) = self.press {
            let moved = f32::from(ev.position.x - start.x).hypot(f32::from(ev.position.y - start.y));
            if moved < DRAG_START && !matches!(self.drag, Some(Drag::Pan { .. }) | None) {
                return;
            }
            self.press = None;
        }
        let snap_free = ev.modifiers.alt || !self.snap_on;
        let step = self.grid;
        let snap = |v: f64| (v / step).round() * step;
        let prev_delta = match &self.drag {
            Some(Drag::Move { delta, .. }) => Some(*delta),
            _ => None,
        };
        let marquee_hits = match &self.drag {
            Some(Drag::Marquee { start, .. }) => Some(self.scene_static().marquee(scene::rect_from(*start, world))),
            _ => None,
        };
        match &mut self.drag {
            Some(Drag::Pan { last }) => {
                let (dx, dy) = (f32::from(ev.position.x - last.x), f32::from(ev.position.y - last.y));
                *last = ev.position;
                self.pan(dx, dy);
            }
            Some(Drag::Move { items, grab, delta, .. }) => {
                let raw = WPoint::new(world.x - grab.x, world.y - grab.y);
                // Snap the first item's origin to the grid, move the rest with it.
                *delta = match items.first() {
                    Some(first) if !snap_free => {
                        WPoint::new(snap(first.start.x + raw.x) - first.start.x, snap(first.start.y + raw.y) - first.start.y)
                    }
                    _ => raw,
                };
            }
            Some(Drag::Marquee { current, base, .. }) => {
                *current = world;
                let mut sel = base.clone();
                sel.extend(marquee_hits.unwrap_or_default().into_iter().filter(|id| !base.contains(id)));
                self.selected = sel;
            }
            Some(Drag::Link { current, .. }) => *current = world,
            Some(Drag::Resize { current, .. }) => {
                *current = if snap_free { world } else { WPoint::new(snap(world.x), snap(world.y)) };
            }
            None => {
                // Hover keeps ports visible while the mouse is near the node.
                let scene = self.scene();
                let pad = self.tol(HANDLE_HIT + 4.0);
                let near = |r: &Rect| {
                    world.x >= r.origin.x - pad
                        && world.y >= r.origin.y - pad
                        && world.x <= r.origin.x + r.size.w + pad
                        && world.y <= r.origin.y + r.size.h + pad
                };
                // Nodes win; otherwise the innermost group under the pointer,
                // so groups can be linked too.
                let inside = |r: &Rect| {
                    world.x >= r.origin.x && world.y >= r.origin.y && world.x <= r.origin.x + r.size.w && world.y <= r.origin.y + r.size.h
                };
                let hover = scene.nodes.iter().rev().find(|n| near(&n.rect)).map(|n| n.id.clone()).or_else(|| {
                    scene
                        .groups
                        .iter()
                        .filter(|g| near(&g.rect))
                        .min_by(|a, b| {
                            let area = |r: &Rect| if inside(r) { r.size.w * r.size.h } else { f64::MAX };
                            area(&a.rect).total_cmp(&area(&b.rect))
                        })
                        .map(|g| g.id.clone())
                });
                if hover == self.hover {
                    return;
                }
                self.hover = hover;
            }
        }
        if let Some(Drag::Move { items, delta, before, regroup, .. }) = &self.drag {
            // Snapping means most pointer moves land on the same spot:
            // nothing to redraw.
            if prev_delta == Some(*delta) {
                return;
            }
            let preview = (*regroup && *delta != WPoint::default()).then(|| Box::new(self.move_preview(before, items, *delta)));
            if let Some(Drag::Move { preview: p, .. }) = &mut self.drag {
                *p = preview;
            }
        }
        cx.notify();
    }

    /// Scene without drag previews (marquee hit testing).
    fn scene_static(&self) -> Scene {
        scene::build(self.doc.diagram(), &HashMap::new())
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let clicked = self.press.take().is_some();
        let Some(drag) = self.drag.take() else { return };
        if clicked {
            // A press that never moved: a click on a port or handle selects
            // its node, nothing else.
            if let Drag::Link { from, .. } = &drag {
                self.selected = vec![from.clone()];
            }
            cx.notify();
            return;
        }
        match drag {
            Drag::Pan { .. } | Drag::Marquee { .. } => {}
            // Dragging while previewing a step animates the move in that step.
            Drag::Move { items, delta, .. } if self.player.is_some() => {
                if (delta.x != 0.0 || delta.y != 0.0)
                    && let Some(i) = self.current_step()
                {
                    let dest: Vec<(String, WPoint)> = items.iter().map(|it| (it.id.clone(), WPoint::new((it.start.x + delta.x).round(), (it.start.y + delta.y).round()))).collect();
                    self.edit_step(
                        i,
                        |st| {
                            for (id, p) in dest {
                                st.moves.retain(|(m, _)| *m != id);
                                st.moves.push((id, p));
                            }
                        },
                        cx,
                    );
                }
            }
            Drag::Move { items, delta, before, regroup, .. } => {
                if delta.x != 0.0 || delta.y != 0.0 {
                    let mut centers = Vec::new();
                    let mut ops: Vec<Op> = items
                        .into_iter()
                        .map(|it| {
                            let pos = WPoint::new((it.start.x + delta.x).round(), (it.start.y + delta.y).round());
                            if let Some(r) = before.rect_of(&it.id) {
                                centers.push((it.id.clone(), WPoint::new(pos.x + r.size.w / 2.0, pos.y + r.size.h / 2.0)));
                            }
                            Op::SetPlacement { id: it.id, placement: Some(Placement { pos, size: it.size }) }
                        })
                        .collect();
                    // Dropping into a group joins it; dragging out leaves.
                    if regroup {
                        let mut probe = self.doc.diagram().clone();
                        probe.apply(&Op::Batch(ops.clone()));
                        ops.extend(ops::regroup_after_move(&probe, &before, &centers));
                    }
                    self.apply(Op::Batch(ops), cx);
                }
            }
            Drag::Link { from, from_port, current } => {
                let scene = self.scene();
                let tol = self.tol(HANDLE_HIT).max(graphing_scene::PORT / 2.0);
                let to_port = |to: &str| {
                    scene.nodes.iter().find(|n| n.id == to)?.ports.iter().find(|p| (p.at.x - current.x).abs() <= tol && (p.at.y - current.y).abs() <= tol).map(|p| p.name.clone())
                };
                let port_target = scene.nodes.iter().rev().find_map(|n| to_port(&n.id).map(|p| (n.id.clone(), p)));
                match (port_target, scene.hit(current)) {
                    (Some((to, tp)), _) if to != from => {
                        let op = ops::add_edge_ports(self.doc.diagram(), (&from, from_port.as_deref()), (&to, Some(&tp)));
                        self.apply(op, cx);
                    }
                    (_, Some(Hit::Node(to) | Hit::Group(to))) if to != from => {
                        let op = ops::add_edge_ports(self.doc.diagram(), (&from, from_port.as_deref()), (&to, None));
                        self.apply(op, cx);
                    }
                    (_, Some(_)) => {}
                    (_, None) => {
                        let at = WPoint::new(current.x - 60.0, current.y - 28.0);
                        let (id, op) = ops::add_connected(self.doc.diagram(), &from, at);
                        if self.apply(op, cx) {
                            self.selected = vec![id.clone()];
                            self.start_rename(&id, window, cx);
                        }
                    }
                }
            }
            Drag::Resize { id, anchor, current } => {
                let r = resize_rect(anchor, current);
                let placement = Placement {
                    pos: WPoint::new(r.origin.x.round(), r.origin.y.round()),
                    size: Some(Size::new(r.size.w.round(), r.size.h.round())),
                };
                let mut ops = vec![Op::SetPlacement { id: id.clone(), placement: Some(placement) }];
                // A group resized over a node takes it in; one shrunk off a
                // member lets it go.
                if self.doc.diagram().group(&id).is_some() {
                    let rect = Rect::new(placement.pos.x, placement.pos.y, r.size.w.round(), r.size.h.round());
                    ops.extend(ops::refit_members(self.doc.diagram(), &self.scene_static(), &id, rect));
                }
                self.apply(Op::Batch(ops), cx);
            }
        }
        cx.notify();
    }

    fn scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(graphing_ui::tokens::SCROLL_LINE);
        if ev.modifiers.control || ev.modifiers.platform {
            let factor = (1.0 + f32::from(d.y) * 0.005).clamp(0.5, 2.0);
            self.zoom_at(factor, ev.position);
        } else if ev.modifiers.shift {
            self.pan(f32::from(d.y), 0.0);
        } else {
            self.pan(f32::from(d.x), f32::from(d.y));
        }
        cx.notify();
    }

    // ---- actions ----

    /// Delete the selection, or ask the workspace to confirm first.
    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            return;
        }
        let d = self.doc.diagram();
        let ask = match self.confirm_delete {
            ConfirmDelete::Never => false,
            ConfirmDelete::Always => true,
            ConfirmDelete::Groups => self.selected.len() > 1 || self.selected.iter().any(|id| d.group(id).is_some()),
        };
        if ask {
            cx.emit(ViewEvent::ConfirmDelete(self.selected.clone()));
        } else {
            let ids = self.selected.clone();
            self.delete_ids(&ids, cx);
        }
    }

    /// Delete `ids` now (after any confirmation).
    pub fn delete_ids(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if let Some(op) = ops::delete(self.doc.diagram(), ids) {
            self.selected.retain(|s| !ids.contains(s));
            self.apply(op, cx);
        }
    }

    /// Images from a `.gphz` package.
    pub fn set_assets(&mut self, assets: crate::files::Assets, cx: &mut Context<Self>) {
        self.assets = assets;
        self.images.borrow_mut().clear();
        cx.notify();
    }

    pub fn assets(&self) -> &crate::files::Assets {
        &self.assets
    }

    /// The extension this diagram saves as: a package once it has pictures.
    pub fn save_ext(&self) -> &'static str {
        if self.assets.is_empty() { "gph" } else { "gphz" }
    }

    /// `path`, made a package if the diagram has pictures and it is not one.
    pub fn save_path(&self, path: PathBuf) -> PathBuf {
        if self.assets.is_empty() || crate::files::is_package_path(&path) { path } else { path.with_extension("gphz") }
    }

    /// Where a file dialog for this diagram opens, and the name it suggests
    /// with `ext`.
    pub fn dialog_start(&self, ext: &str) -> (PathBuf, String) {
        let dir = self.folder().filter(|d| !d.as_os_str().is_empty()).map(PathBuf::from).or_else(|| std::env::current_dir().ok()).unwrap_or_default();
        let stem = self.path.as_ref().and_then(|p| p.file_stem()).map_or("untitled".into(), |s| s.to_string_lossy());
        (dir, format!("{stem}.{ext}"))
    }

    /// Keep `bytes` in the package and return the `src` that shows them.
    pub fn add_asset(&mut self, original: &str, bytes: Vec<u8>) -> String {
        let name = graphing_package::asset_name(original, &bytes);
        self.assets.entry(name.clone()).or_insert(bytes);
        format!("{}{name}", graphing_package::ASSET_PREFIX)
    }

    /// The bytes behind a `src`: a packaged asset, or a file path (relative
    /// to the diagram's folder).
    /// The folder of this diagram's file, which `src` paths are relative to.
    pub fn folder(&self) -> Option<&std::path::Path> {
        self.path.as_ref().and_then(|p| p.parent())
    }

    fn image_bytes(&self, src: &str) -> Option<Vec<u8>> {
        graphing_export::picture(src, &self.assets, self.folder())
    }

    /// A link to the diagram at `path`, top-left at `at`.
    pub fn add_link(&mut self, path: &std::path::Path, at: WPoint, cx: &mut Context<Self>) {
        let src = graphing_export::src_for(path, self.folder());
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "link".into());
        let d = self.doc.diagram();
        let base: String = stem.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
        let base = format!("{}_link", if base.starts_with(|c: char| c.is_alphabetic() || c == '_') { base } else { format!("d{base}") });
        let id = graphing_model::unique_id(&base, "", |i| d.node(i).is_some() || d.group(i).is_some());
        let node = graphing_model::Node { id: id.clone(), stencil: Some("ref".into()), label: None, classes: Vec::new(), props: vec![("src".into(), graphing_model::Value::Str(src))] };
        let index = d.nodes.len();
        let op = Op::Batch(vec![Op::AddNode { node, index }, Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos: WPoint::new(at.x.round(), at.y.round()), size: None }) }]);
        if self.apply(op, cx) {
            self.selected = vec![id];
        }
    }

    pub(crate) fn link_target(&self, scene: &Scene, id: &str) -> Option<PathBuf> {
        let src = scene.nodes.iter().find(|n| n.id == id)?.reference.clone().filter(|s| !s.is_empty())?;
        Some(graphing_export::resolve_src(&src, self.folder()))
    }

    /// Load the diagrams links point at, again whenever their file changes.
    fn load_links(&self, scene: &Scene) {
        let mut linked = self.linked.borrow_mut();
        for src in scene.nodes.iter().filter_map(|n| n.reference.as_ref()).filter(|s| !s.is_empty()) {
            let path = graphing_export::resolve_src(src, self.folder());
            let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            if linked.get(src).is_some_and(|(t, _)| *t == mtime) {
                continue;
            }
            // The file itself, else the snapshot this package carries.
            let snapshot = || self.assets.get(&graphing_package::linked_name(src)).and_then(|b| graphing_export::ref_from_text(&String::from_utf8_lossy(b), src));
            let loaded = mtime.and_then(|_| graphing_export::load_ref(&path)).or_else(snapshot).map(Rc::new);
            linked.insert(src.clone(), (mtime, loaded));
        }
    }

    /// Decode pictures the scene shows that are not cached yet.
    fn decode_images(&self, scene: &Scene, cx: &gpui_kit::App) {
        let mut cache = self.images.borrow_mut();
        for n in &scene.nodes {
            let Some(src) = &n.image else { continue };
            if cache.contains_key(src) {
                continue;
            }
            let data = self.image_bytes(src).and_then(|bytes| crate::media::decode(&bytes, cx.svg_renderer()));
            cache.insert(src.clone(), data);
        }
    }

    /// Add image nodes at `at` (cascading), each picture either packaged
    /// (`embed`) or linked by its path relative to the diagram.
    pub fn insert_images(&mut self, images: Vec<IncomingImage>, at: WPoint, embed: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut probe = self.doc.diagram().clone();
        let mut ops_all = Vec::new();
        let mut ids = Vec::new();
        for (i, img) in images.into_iter().enumerate() {
            let src = match (&img.path, embed) {
                (Some(p), false) => graphing_export::src_for(p, self.folder()),
                _ => self.add_asset(&img.name, img.bytes.clone()),
            };
            // Natural size, scaled down to fit a sensible box.
            let (w, h) = crate::media::decode(&img.bytes, cx.svg_renderer()).map(|d| (d.size(0).width.0 as f64, d.size(0).height.0 as f64)).unwrap_or((240.0, 160.0));
            let k = (IMAGE_MAX_W / w).min(IMAGE_MAX_H / h).min(1.0);
            let (w, h) = ((w * k).round().max(24.0), (h * k).round().max(24.0));
            let off = i as f64 * 24.0;
            let pos = WPoint::new((at.x - w / 2.0 + off).round(), (at.y - h / 2.0 + off).round());
            let (id, add) = ops::add_node(&probe, Some("image"), pos);
            probe.apply(&add);
            let props = vec![
                Op::SetProp { id: id.clone(), key: "src".into(), value: Some(graphing_model::Value::Str(src)) },
                Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos, size: Some(Size::new(w, h)) }) },
            ];
            for op in &props {
                probe.apply(op);
            }
            ops_all.push(add);
            ops_all.extend(props);
            ids.push(id);
        }
        if !ids.is_empty() && self.apply(Op::Batch(ops_all), cx) {
            self.selected = ids;
            window.focus(&self.focus, cx);
        }
    }

    /// The canvas center in diagram units, where inserted things go.
    pub fn center(&self) -> WPoint {
        self.to_world(self.bounds.get().center())
    }


    /// When moving pictures play (set from settings by the workspace).
    pub fn set_play(&mut self, play: crate::settings::Play, cx: &mut Context<Self>) {
        self.play = play;
        cx.notify();
    }

    /// Ask for the next animation frame if anything on screen moves, at
    /// most 20 a second, and only while the window is in front.
    fn schedule_animation(&mut self, scene: &Scene, window: &Window, cx: &mut Context<Self>) {
        use crate::settings::Play;
        if self.anim_pending || self.play == Play::Never || !window.is_window_active() {
            return;
        }
        let cache = self.images.borrow();
        let moves = |n: &scene::NodeBox| n.image.as_ref().and_then(|s| cache.get(s)).and_then(Option::as_ref).is_some_and(|d| crate::media::animated(d));
        let any = match self.play {
            Play::Hover => scene.nodes.iter().any(|n| self.hover.as_deref() == Some(n.id.as_str()) && moves(n)),
            _ => scene.nodes.iter().any(moves),
        };
        drop(cache);
        if !any {
            return;
        }
        self.anim_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(crate::media::FRAME_GAP_MS)).await;
            this.update(cx, |v, cx| {
                v.anim_pending = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// When deleting asks first (set from settings by the workspace).
    pub fn set_confirm_delete(&mut self, mode: ConfirmDelete) {
        self.confirm_delete = mode;
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = ops::copy(self.doc.diagram(), &self.selected) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.set_status(format!("copied {} item(s)", self.selected.len()), cx);
        }
    }

    fn nudge(&mut self, dx: f64, dy: f64, cx: &mut Context<Self>) {
        let scene = self.scene();
        if let Some(op) = ops::move_by(&scene, self.doc.diagram(), &self.selected, dx, dy) {
            self.apply(op, cx);
        }
    }

    fn paste_text(&mut self, text: &str, offset: WPoint, cx: &mut Context<Self>) {
        match ops::paste(self.doc.diagram(), text, offset) {
            Some((ids, op)) => {
                if self.apply(op, cx) {
                    self.selected = ids;
                }
            }
            None => self.set_status("clipboard has no graphing nodes", cx),
        }
    }

    fn overlay(&self) -> Overlay {
        let mut o = Overlay { hover: self.hover.clone(), ..Default::default() };
        match &self.drag {
            Some(Drag::Marquee { start, current, .. }) => o.marquee = Some(scene::rect_from(*start, *current)),
            Some(Drag::Link { from, from_port, current }) => {
                o.link = Some((from.clone(), *current));
                o.link_start = from_port.as_ref().and_then(|p| self.scene().port(from, p)).map(|p| p.at);
                o.hover = Some(from.clone());
            }
            Some(Drag::Move { preview, .. }) => {
                o.hover = None;
                o.drop_group = preview.as_ref().and_then(|p| p.1.clone());
            }
            Some(Drag::Resize { .. }) => o.hover = None,
            _ => {}
        }
        o
    }

    /// Where `id`'s label paints and how, so the editor types it in place
    /// at the same size and face.
    fn label_spot(&self, id: &str) -> Option<LabelSpot> {
        use graphing_scene::GroupLook as L;
        let s = self.scene();
        let spot = |rect, pt, face, left, surface| Some(LabelSpot { rect, pt, face, left, surface });
        if id == FRAME_ID {
            let f = s.frame.as_ref()?;
            let inset = 12.0;
            return spot(Rect::new(f.rect.origin.x + inset, f.rect.origin.y, f.rect.size.w.min(360.0) - inset, graphing_scene::FRAME_HEAD), paint::FRAME_PT, Face::Mono, true, false);
        }
        if let Some(n) = s.nodes.iter().find(|n| n.id == id) {
            let structured = n.shape == scene::Shape::Block || !n.compartments.is_empty() || n.stereotype.is_some();
            if !structured {
                return spot(n.rect, paint::LABEL_PT, Face::Regular, false, false);
            }
            // The bold name under any stereotype, not the whole box.
            let y = n.rect.origin.y + if n.stereotype.is_some() { scene::notation::STEREO_H } else { 0.0 };
            return spot(Rect::new(n.rect.origin.x, y, n.rect.size.w, scene::notation::HEADER_H), paint::HEADER_PT, Face::Bold, false, false);
        }
        if let Some(g) = s.groups.iter().find(|g| g.id == id) {
            // The header strip only: the body holds members and links.
            let strip = if g.look == L::Package { f64::from(paint::GROUP_TAB) } else { g.look.head() };
            let inset = if g.look == L::Sysml { 0.0 } else { 12.0 + if g.icon.is_some() { 18.0 } else { 0.0 } };
            let face = if matches!(g.look, L::Dashed | L::Solid) { Face::Regular } else { Face::Bold };
            return spot(Rect::new(g.rect.origin.x + inset, g.rect.origin.y, (g.rect.size.w - inset - 8.0).max(60.0), strip), paint::GROUP_PT, face, g.look != L::Sysml, false);
        }
        // Edge labels sit on the line, so the field gets its own surface.
        let e = s.edges.iter().find(|e| e.id == id)?;
        let i = (e.points.len() - 1) / 2;
        let (a, b) = (e.points[i], e.points[i + 1]);
        spot(Rect::new((a.x + b.x) / 2.0 - 90.0, (a.y + b.y) / 2.0 - 12.0, 180.0, 24.0), paint::EDGE_PT, Face::Regular, false, true)
    }

    fn rename_field(&self, cx: &gpui_kit::App) -> Option<impl IntoElement> {
        use graphing_ui::tokens::*;
        let spot = self.label_spot(self.renaming.as_ref()?)?;
        let zoom = self.cam.get().zoom;
        let k = cx.ui();
        let o = self.bounds.get().origin;
        let tl = self.to_screen(spot.rect.origin);
        let (w, h) = (px((spot.rect.size.w as f32 * zoom).max(f32::from(HIT_LG) * 3.0)), px(spot.rect.size.h as f32 * zoom));
        let mono = gpui_kit::component::ActiveTheme::theme(cx).mono_font_family.clone();
        let input = Input::new(&self.rename)
            .appearance(false)
            .w_full()
            .h_full()
            .p_0()
            .text_size(px(spot.pt * zoom))
            .when(spot.face == Face::Bold, |i| i.font_weight(gpui_kit::FontWeight::SEMIBOLD))
            .when(spot.face == Face::Mono, |i| i.font_family(mono));
        Some(
            div()
                .absolute()
                .left(tl.x - o.x)
                .top(tl.y - o.y)
                .w(w)
                .h(h)
                .flex()
                .items_center()
                .when(!spot.left, |d| d.px(GAP_1 * zoom))
                .when(spot.surface, |d| d.bg(k.bg).border_1().border_color(k.accent).rounded(ROUND_XS).shadow_sm())
                .child(input),
        )
    }
}

/// The playing step's title at the top of the canvas: `2 / 5  API server`.
fn step_caption(text: String, a: f32, n: usize, of: usize, cx: &Context<DiagramView>) -> impl IntoElement {
    use graphing_ui::tokens::*;
    let k = cx.ui();
    div().absolute().top(GAP_4).left_0().right_0().flex().justify_center().opacity(a).child(
        graphing_ui::kit::raised(cx)
            .flex()
            .items_center()
            .gap(GAP_3)
            .px(GAP_4)
            .h(HIT_LG + GAP_1)
            .rounded(ROUND_PILL)
            .child(div().text_size(TEXT_XS).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.accent).child(format!("{n} / {of}")))
            .child(div().text_size(TEXT_MD).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.heading).child(text)),
    )
}

/// How a label paints, for the in-place editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Regular,
    Bold,
    Mono,
}

struct LabelSpot {
    /// Where the label sits, in diagram units.
    rect: Rect,
    /// Its size in diagram units (scaled by the zoom).
    pt: f32,
    face: Face,
    /// Starts at the left edge rather than centred.
    left: bool,
    /// Needs its own background (it sits over a line).
    surface: bool,
}

/// Rect between a fixed corner and the dragged one, never below the minimum.
fn resize_rect(anchor: WPoint, current: WPoint) -> Rect {
    let w = (current.x - anchor.x).abs().max(MIN_W);
    let h = (current.y - anchor.y).abs().max(MIN_H);
    let x = if current.x < anchor.x { anchor.x - w } else { anchor.x };
    let y = if current.y < anchor.y { anchor.y - h } else { anchor.y };
    Rect::new(x, y, w, h)
}

impl Render for DiagramView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let palette = Palette::from(cx.ui());
        let mono = gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone();
        let scene = self.scene();
        // An animation playing or previewed: its moment, and no editing chrome.
        let anim = self.anim_state();
        self.tick(cx);
        // Playing hides the editing chrome; a paused preview keeps it, so
        // shapes can be picked and dragged into moves.
        let playing = self.playing();
        let selected = if playing { Vec::new() } else { self.selected.clone() };
        let overlay = if playing { paint::Overlay::default() } else { self.overlay() };
        let caption = anim.as_ref().and_then(|(st, tl)| st.caption.clone().map(|(text, a)| (text, a, st.step.unwrap_or(0) + 1, tl.spans.len())));
        // Shapes a step moves, where they are now; lines re-route with them.
        let scene = anim.as_ref().and_then(|(st, tl)| tl.scene_for(st)).unwrap_or(scene);
        let minimap = self.minimap(&scene, cx);
        let anim = anim.map(|(st, _)| st);
        let editing = self.renaming.clone();
        let content = scene.bounds();
        let (cam, cam_paint) = (self.cam.clone(), self.cam.clone());
        let bounds_cell = self.bounds.clone();
        let grid = self.show_grid.then_some(self.grid as f32);
        self.decode_images(&scene, cx);
        self.load_links(&scene);
        let linked = self.linked.clone();
        self.schedule_animation(&scene, window, cx);
        let images = self.images.clone();
        let (now_ms, play) = (self.started.elapsed().as_millis() as u64, self.play);

        let surface = canvas(
            move |bounds, _, _| {
                let resized = bounds_cell.get().size != bounds.size;
                bounds_cell.set(bounds);
                let c = cam.get();
                if (c.fit || (c.auto && resized))
                    && let Some(c) = fit_camera(bounds, content)
                {
                    cam.set(c);
                }
            },
            move |bounds, (), window, cx| {
                // The animation's camera, when it moves one, without touching the user's.
                let c = anim.as_ref().and_then(|s| s.camera).and_then(|r| fit_camera(bounds, Some(r))).unwrap_or_else(|| cam_paint.get());
                let view = paint::View { origin: bounds.origin, offset: c.offset, zoom: c.zoom };
                let images = images.borrow();
                let linked = linked.borrow();
                let frame = Frame {
                    scene: &scene,
                    view,
                    bounds,
                    palette,
                    selected: &selected,
                    overlay: &overlay,
                    mono: mono.clone(),
                    editing: editing.as_deref(),
                    grid,
                    images: &images,
                    now_ms,
                    play,
                    anim: anim.as_ref(),
                    fade: 1.0,
                    linked: &linked,
                };
                let t = std::time::Instant::now();
                paint::paint(&frame, window, cx);
                crate::trace("canvas paint", t);
            },
        )
        .size_full();

        let pan = |v: &mut Self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
            v.drag = Some(Drag::Pan { last: ev.position });
            // Repaint so the grabbing hand shows at once.
            cx.notify();
        };
        let panning = matches!(self.drag, Some(Drag::Pan { .. }));
        // The rename field sits outside the "Diagram" key context so single
        // key canvas bindings never steal its typing.
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("diagram")
                    .key_context("Diagram")
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|v, _: &Undo, _, cx| v.undo_op(cx)))
                    .on_action(cx.listener(|v, _: &Redo, _, cx| v.redo_op(cx)))
                    .on_action(cx.listener(|v, _: &Delete, _, cx| v.delete_selected(cx)))
                    .on_action(cx.listener(|v, _: &SelectAll, _, cx| {
                        let ids = v.doc.diagram().nodes.iter().map(|n| n.id.clone()).collect();
                        v.select(ids, cx);
                    }))
                    .on_action(cx.listener(|v, _: &Escape, _, cx| {
                        v.stop_animation(cx);
                        v.menu = None;
                        v.drag = None;
                        v.select(Vec::new(), cx);
                    }))
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(|v, _: &Cut, w, cx| {
                        v.copy(&Copy, w, cx);
                        v.delete_selected(cx);
                    }))
                    .on_action(cx.listener(|v, _: &Paste, _, cx| {
                        let Some(item) = cx.read_from_clipboard() else { return };
                        // Pictures (copied images or image files) become image nodes.
                        let mut images = Vec::new();
                        for entry in item.entries() {
                            match entry {
                                gpui_kit::ClipboardEntry::Image(img) => images.push(IncomingImage { name: "pasted.png".into(), path: None, bytes: img.bytes.clone() }),
                                gpui_kit::ClipboardEntry::ExternalPaths(paths) => images.extend(paths.paths().iter().filter_map(|p| IncomingImage::from_path(p))),
                                _ => {}
                            }
                        }
                        if !images.is_empty() {
                            let at = v.center();
                            cx.emit(ViewEvent::AddImages(images, at));
                        } else if let Some(text) = item.text() {
                            v.paste_text(&text, WPoint::new(20.0, 20.0), cx);
                        }
                    }))
                    .on_action(cx.listener(|v, _: &Duplicate, _, cx| {
                        if let Some(text) = ops::copy(v.doc.diagram(), &v.selected) {
                            v.paste_text(&text, WPoint::new(20.0, 20.0), cx);
                        }
                    }))
                    // While an animation shows, left and right walk its steps.
                    .on_action(cx.listener(|v, _: &NudgeLeft, _, cx| if v.player.is_some() { v.step_by(-1, cx) } else { v.nudge(-1.0, 0.0, cx) }))
                    .on_action(cx.listener(|v, _: &NudgeRight, _, cx| if v.player.is_some() { v.step_by(1, cx) } else { v.nudge(1.0, 0.0, cx) }))
                    .on_action(cx.listener(|v, _: &crate::NewStep, _, cx| {
                        v.add_step(cx);
                    }))
                    .on_action(cx.listener(|v, _: &NudgeUp, _, cx| v.nudge(0.0, -1.0, cx)))
                    .on_action(cx.listener(|v, _: &NudgeDown, _, cx| v.nudge(0.0, 1.0, cx)))
                    .on_action(cx.listener(|v, _: &NudgeLeftBig, _, cx| v.nudge(-10.0, 0.0, cx)))
                    .on_action(cx.listener(|v, _: &NudgeRightBig, _, cx| v.nudge(10.0, 0.0, cx)))
                    .on_action(cx.listener(|v, _: &NudgeUpBig, _, cx| v.nudge(0.0, -10.0, cx)))
                    .on_action(cx.listener(|v, _: &NudgeDownBig, _, cx| v.nudge(0.0, 10.0, cx)))
                    .on_action(cx.listener(|v, _: &Rename, w, cx| {
                        if let [id] = v.selected.as_slice() {
                            let id = id.clone();
                            v.start_rename(&id, w, cx);
                        }
                    }))
                    .on_action(cx.listener(|v, _: &GroupSelection, _, cx| v.group_selection(cx)))
                    .on_action(cx.listener(|v, _: &Ungroup, _, cx| v.ungroup_selection(cx)))
                    .on_action(cx.listener(|v, _: &FitView, _, cx| v.request_fit(cx)))
                    .on_action(cx.listener(|v, _: &ZoomIn, _, cx| v.zoom_center(1.2, cx)))
                    .on_action(cx.listener(|v, _: &ZoomOut, _, cx| v.zoom_center(1.0 / 1.2, cx)))
                    .on_action(cx.listener(|v, _: &ZoomReset, _, cx| v.zoom_reset(cx)))
                    .size_full()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(pan))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::right_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Right, cx.listener(Self::right_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Right, cx.listener(Self::right_up))
                    .when(panning, |el| el.cursor(CursorStyle::ClosedHand))
                    .on_scroll_wheel(cx.listener(Self::scroll))
                    .on_drop(cx.listener(|v, d: &crate::library::StencilDrag, window, cx| {
                        let at = v.to_world(window.mouse_position());
                        v.add_shape_at(&d.stencil, at, window, cx);
                    }))
                    // Image files dragged in from the desktop.
                    .on_drop(cx.listener(|v, paths: &gpui_kit::ExternalPaths, window, cx| {
                        cx.stop_propagation();
                        let at = v.to_world(window.mouse_position());
                        // Diagrams: open, insert or link (the workspace asks).
                        let diagrams: Vec<PathBuf> = paths.paths().iter().filter(|p| crate::files::is_openable(p)).cloned().collect();
                        if !diagrams.is_empty() {
                            cx.emit(ViewEvent::DiagramsDropped(diagrams, at));
                        }
                        let images: Vec<IncomingImage> = paths.paths().iter().filter_map(|p| IncomingImage::from_path(p)).collect();
                        if !images.is_empty() {
                            cx.emit(ViewEvent::AddImages(images, at));
                        }
                    }))
                    .child(surface),
            )
            .when_some(self.rename_field(cx), |this, e| this.child(e))
            .when_some(self.context_menu(window, cx), |this, m| this.child(m))
            .when_some(caption, |this, (text, a, n, of)| this.child(step_caption(text, a, n, of, cx)))
            .when_some(minimap, |this, m| this.child(m))
            .map(|el| {
                crate::trace("canvas render", started);
                el
            })
    }
}

mod minimap;
#[cfg(test)]
mod tests;
