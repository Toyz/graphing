//! Title bar menus that open on hover. Moving between titles switches
//! menus; leaving the bar and the open menu closes it after a short delay,
//! so crossing the gap between title and menu never flickers.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    Action, AnyElement, App, Bounds, Context, DispatchPhase, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent,
    ParentElement, Pixels, Point, SharedString, StatefulInteractiveElement, Styled, Window, canvas, deferred, div,
    prelude::FluentBuilder,
};
use graphing_ui::UiExt;
use graphing_ui::kit::Lucide;
use graphing_ui::menu::{self, MenuRow};
use graphing_ui::tokens::*;

use crate::workspace::Workspace;
use crate::*;

/// How long the pointer may be outside a menu before it closes.
const CLOSE_DELAY: Duration = Duration::from_millis(220);

/// A menu entry: label, icon, action; `checked` for toggles.
pub enum Item {
    Action { label: SharedString, icon: Lucide, action: Box<dyn Action>, checked: bool },
    Caption(SharedString),
    Separator,
    /// More entries in a menu beside this one.
    Sub { label: SharedString, icon: Lucide, items: Vec<Item> },
}

fn sub(label: impl Into<SharedString>, icon: Lucide, items: Vec<Item>) -> Item {
    Item::Sub { label: label.into(), icon, items }
}

fn act(label: impl Into<SharedString>, icon: Lucide, action: impl Action) -> Item {
    Item::Action { label: label.into(), icon, action: Box::new(action), checked: false }
}

fn check(label: impl Into<SharedString>, icon: Lucide, on: bool, action: impl Action) -> Item {
    Item::Action { label: label.into(), icon, action: Box::new(action), checked: on }
}

#[derive(Default)]
pub struct MenuState {
    pub open: Option<usize>,
    /// Bumped whenever the pointer is back inside; a pending close only
    /// fires if it is unchanged.
    generation: usize,
    closing: bool,
    /// Bounds of the title bar menus and of the open panel, from last frame.
    bar: Rc<Cell<Option<Bounds<Pixels>>>>,
    panel: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The open submenu (its row in the open menu), its row and its panel.
    pub sub: Option<usize>,
    /// Every submenu row of the open menu, by its index there.
    sub_rows: std::collections::HashMap<usize, Rc<Cell<Option<Bounds<Pixels>>>>>,
    sub_panel: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Workspace {
    fn menus(&self, cx: &App) -> Vec<(&'static str, Vec<Item>)> {
        use crate::dock::Tool;
        use Lucide as L;
        let mut file = vec![
            act("New Diagram", L::FilePlus, NewFile),
            act("New from Template...", L::LayoutTemplate, NewFromTemplate),
            act("Open...", L::FolderOpen, OpenFile),
            Item::Separator,
            act("Save", L::Save, Save),
            act("Save As...", L::SaveAll, SaveAs),
            Item::Separator,
            sub(
                "Import",
                L::FileInput,
                vec![
                    act("Mermaid...", L::GitFork, ImportAs { format: "mermaid".into() }),
                    act("draw.io...", L::PenTool, ImportAs { format: "drawio".into() }),
                    act("SysML v2...", L::Box, ImportAs { format: "sysml".into() }),
                    act("Visio...", L::LayoutTemplate, ImportAs { format: "visio".into() }),
                    Item::Separator,
                    act("Any Supported File...", L::FileInput, ImportFile),
                ],
            ),
            sub(
                "Export",
                L::FileOutput,
                vec![
                    act("SVG...", L::FileImage, ExportSvg),
                    act("PNG...", L::Image, ExportPng),
                    act("SysML v2...", L::FileCode, ExportSysml),
                    Item::Separator,
                    act("Animation (GIF, WebM)...", L::Clapperboard, ExportAnimation),
                ],
            ),
        ];
        if !self.recent.is_empty() {
            file.push(Item::Separator);
            file.push(Item::Caption("Recent".into()));
            for (i, p) in self.recent.iter().take(6).enumerate() {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                file.push(act(name, L::FileText, OpenRecent { index: i }));
            }
        }
        file.extend([Item::Separator, act("Settings", L::Settings, OpenSettings), act("Close Tab", L::X, CloseTab), act("Quit", L::LogOut, Quit)]);
        vec![
            ("File", file),
            (
                "Edit",
                vec![
                    act("Undo", L::Undo2, Undo),
                    act("Redo", L::Redo2, Redo),
                    Item::Separator,
                    act("Cut", L::Scissors, Cut),
                    act("Copy", L::Copy, Copy),
                    act("Paste", L::ClipboardPaste, Paste),
                    act("Duplicate", L::CopyPlus, Duplicate),
                    act("Delete", L::Trash, Delete),
                    Item::Separator,
                    act("Select All", L::SquareDashedMousePointer, SelectAll),
                    act("Rename", L::PencilLine, Rename),
                    Item::Separator,
                    act("Insert Image...", L::ImagePlus, InsertImage),
                ],
            ),
            (
                "Arrange",
                vec![
                    act("Auto Layout", L::Workflow, Relayout),
                    Item::Separator,
                    act("Align Left", L::AlignStartVertical, AlignLeft),
                    act("Align Center", L::AlignCenterVertical, AlignCenter),
                    act("Align Right", L::AlignEndVertical, AlignRight),
                    act("Align Top", L::AlignStartHorizontal, AlignTop),
                    act("Align Middle", L::AlignCenterHorizontal, AlignMiddle),
                    act("Align Bottom", L::AlignEndHorizontal, AlignBottom),
                    Item::Separator,
                    act("Distribute Horizontally", L::AlignHorizontalSpaceAround, SpreadH),
                    act("Distribute Vertically", L::AlignVerticalSpaceAround, SpreadV),
                    act("Make Same Size", L::Scaling, SameSize),
                    Item::Separator,
                    act("Group Selection", L::Group, GroupSelection),
                    act("Ungroup", L::Ungroup, Ungroup),
                ],
            ),
            (
                "View",
                vec![
                    check("Shapes", L::Shapes, self.shown(Tool::Shapes, cx), ToggleLeft),
                    check("Outline", L::ListTree, self.shown(Tool::Outline, cx), ToggleOutline),
                    check("Inspector", L::SlidersHorizontal, self.shown(Tool::Inspector, cx), ToggleRight),
                    check("Source", L::Code, self.shown(Tool::Source, cx), ToggleSource),
                    check("Problems", L::TriangleAlert, self.shown(Tool::Problems, cx), ToggleProblems),
                    act("Reset Layout", L::LayoutDashboard, ResetLayout),
                    Item::Separator,
                    act("Command Palette", L::Command, CommandPalette),
                    act("Cycle Theme", L::SunMoon, CycleTheme),
                    Item::Separator,
                    act("Fit to Window", L::Scan, FitView),
                    act("Zoom In", L::ZoomIn, ZoomIn),
                    act("Zoom Out", L::ZoomOut, ZoomOut),
                    act("Actual Size", L::Search, ZoomReset),
                    Item::Separator,
                    act("Play Animation", L::Play, PlayAnimation),
                    act("New Step from Selection", L::Plus, NewStep),
                    act("Sequence", L::Clapperboard, ToggleSequence),
                ],
            ),
        ]
    }

    /// Pointer moved somewhere in the window while a menu is open.
    fn menu_pointer(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        // Over the menu but off the submenu's row and panel: the submenu closes.
        // The row's whole band counts, so the gap between it and the
        // submenu is crossed without closing anything.
        let row = self.menu.sub.and_then(|j| self.menu.sub_rows.get(&j)).and_then(|z| z.get());
        let in_row = row.is_some_and(|r| at.y >= r.top() && at.y <= r.bottom());
        let in_sub = in_row || self.menu.sub_panel.get().is_some_and(|b| b.contains(&at));
        if self.menu.sub.is_some() && !in_sub && self.menu.panel.get().is_some_and(|b| b.contains(&at)) {
            self.menu.sub = None;
            self.menu.sub_panel.set(None);
            cx.notify();
        }
        let sub_panel = if self.menu.sub.is_some() { self.menu.sub_panel.get() } else { None };
        let inside = [self.menu.bar.get(), self.menu.panel.get(), sub_panel].into_iter().flatten().any(|b| b.contains(&at));
        if inside {
            self.menu.generation += 1;
            self.menu.closing = false;
            return;
        }
        if self.menu.closing {
            return;
        }
        self.menu.closing = true;
        let generation = self.menu.generation;
        cx.spawn(async move |ws, cx| {
            cx.background_executor().timer(CLOSE_DELAY).await;
            ws.update(cx, |ws, cx| {
                if ws.menu.generation == generation && ws.menu.closing {
                    ws.menu.open = None;
                    ws.menu.closing = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        self.menu.generation += 1;
        self.menu.closing = false;
        if self.menu.open != Some(index) {
            self.menu.open = Some(index);
            self.menu.sub = None;
            self.menu.sub_rows.clear();
            self.menu.panel.set(None);
            self.menu.sub_panel.set(None);
            cx.notify();
        }
    }

    fn run_menu_item(&mut self, action: Box<dyn Action>, window: &mut Window, cx: &mut Context<Self>) {
        self.menu.open = None;
        self.menu.sub = None;
        self.focus_canvas(window, cx);
        window.dispatch_action(action, cx);
        cx.notify();
    }

    pub fn render_menubar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        let menus = self.menus(cx);
        let open = self.menu.open;
        let titles: Vec<AnyElement> = menus
            .into_iter()
            .enumerate()
            .map(|(i, (title, items))| {
                let is_open = open == Some(i);
                let panel = is_open.then(|| self.menu_panel(i, items, window, cx));
                div()
                    .relative()
                    .child(
                        div()
                            .id(("menu", i))
                            .debug_selector(move || format!("menu-title-{i}"))
                            .h(HIT_MD)
                            .px(GAP_2)
                            .flex()
                            .items_center()
                            .rounded(ROUND_SM)
                            .text_size(TEXT_MD)
                            .cursor_pointer()
                            .when(is_open, |d| d.bg(k.accent_soft).text_color(k.heading))
                            .when(!is_open, |d| d.text_color(k.text_muted).hover(|d| d.bg(k.hover).text_color(k.text)))
                            .on_mouse_down(MouseButton::Left, cx.listener(move |ws, _, _, cx| {
                                cx.stop_propagation();
                                if ws.menu.open == Some(i) {
                                    ws.menu.open = None;
                                    cx.notify();
                                } else {
                                    ws.open_menu(i, cx);
                                }
                            }))
                            .on_hover(cx.listener(move |ws, h: &bool, _, cx| {
                                if *h {
                                    ws.open_menu(i, cx);
                                }
                            }))
                            .child(title),
                    )
                    .when_some(panel, |d, p| d.child(div().absolute().top(HIT_MD + GAP_1).left_0().child(deferred(p).with_priority(2))))
                    .into_any_element()
            })
            .collect();
        let bar = self.menu.bar.clone();
        let ws = cx.entity().downgrade();
        let watching = open.is_some();
        div()
            .relative()
            .flex()
            .items_center()
            .gap(GAP_0)
            .child(menu::zone(bar))
            .children(titles)
            // While a menu is open, watch every pointer move in the window:
            // hover events alone are not reliable across the deferred panel.
            .when(watching, |d| {
                d.child(canvas(|_, _, _| {}, move |_, (), window, _| {
                    let ws = ws.clone();
                    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            ws.update(cx, |ws, cx| ws.menu_pointer(ev.position, cx)).ok();
                        }
                    });
                }))
            })
            .into_any_element()
    }

    /// Menu rows for `items`; submenus open beside their row on hover.
    fn menu_rows(&mut self, items: Vec<Item>, window: &mut Window, cx: &mut Context<Self>) -> Vec<MenuRow> {
        items
            .into_iter()
            .enumerate()
            .map(|(j, item)| match item {
                Item::Separator => MenuRow::Separator,
                Item::Caption(t) => MenuRow::Caption(t),
                Item::Action { label, icon, action, checked } => {
                    let keys = self.keys_for(&*action, window, cx);
                    MenuRow::item(label, cx.listener(move |ws, _, window, cx| ws.run_menu_item(action.boxed_clone(), window, cx)))
                        .icon(icon)
                        .keys(keys)
                        .checked(checked)
                }
                Item::Sub { label, icon, items } => {
                    let open = self.menu.sub == Some(j);
                    let rows = if open { self.menu_rows(items, window, cx) } else { Vec::new() };
                    let ws = cx.entity().downgrade();
                    let on_open = move |_: &mut Window, cx: &mut App| {
                        ws.update(cx, |ws, cx| {
                            if ws.menu.sub != Some(j) {
                                ws.menu.sub = Some(j);
                                ws.menu.sub_panel.set(None);
                                cx.notify();
                            }
                        })
                        .ok();
                    };
                    let row_zone = self.menu.sub_rows.entry(j).or_default().clone();
                    MenuRow::sub(label, rows, open, on_open, row_zone, self.menu.sub_panel.clone()).icon(icon)
                }
            })
            .collect()
    }

    fn menu_panel(&mut self, index: usize, items: Vec<Item>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.menu_rows(items, window, cx);
        let slot = self.menu.panel.clone();
        let surface = menu::menu_surface(("menu-panel", index), rows, cx)
            .debug_selector(|| "menu-panel".into())
            .on_mouse_down_out(cx.listener(|ws, ev: &gpui_kit::MouseDownEvent, _, cx| {
                // A press in the open submenu is still in the menu.
                if ws.menu.sub.is_some() && ws.menu.sub_panel.get().is_some_and(|b| b.contains(&ev.position)) {
                    return;
                }
                ws.menu.open = None;
                ws.menu.sub = None;
                cx.notify();
            }))
            .child(menu::zone(slot));
        menu::animate(surface, ("menu-anim", index))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui_kit::{Modifiers, TestAppContext, point, px};


    #[gpui_kit::test]
    fn menu_stays_open_while_pointer_moves_onto_it(cx: &mut TestAppContext) {
        let (ws, cx) = crate::test_support::workspace(cx, Vec::new());

        let title = cx.debug_bounds("menu-title-0").expect("File title rendered");
        cx.simulate_mouse_move(title.center(), None, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |w, _| w.menu.open), Some(0), "hover opens");

        let panel = cx.debug_bounds("menu-panel").expect("panel rendered");
        // Walk down from the title into the panel like a real pointer.
        let mut y = title.center().y;
        while y < panel.center().y {
            y += px(4.0);
            cx.simulate_mouse_move(point(title.center().x, y), None, Modifiers::default());
        }
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |w, _| w.menu.open), Some(0), "stays open over the panel");

        // Leaving both closes it after the delay.
        cx.simulate_mouse_move(point(panel.right() + px(300.0), panel.bottom() + px(300.0)), None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |w, _| w.menu.open), None, "closes after leaving");
    }

    #[gpui_kit::test]
    fn import_opens_a_submenu_that_stays_while_the_pointer_crosses(cx: &mut TestAppContext) {
        let (ws, cx) = crate::test_support::workspace(cx, Vec::new());
        let title = cx.debug_bounds("menu-title-0").expect("File title");
        cx.simulate_mouse_move(title.center(), None, Modifiers::default());
        cx.run_until_parked();

        let row = cx.debug_bounds("menu-sub-row-Import").expect("Import row");
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        cx.run_until_parked();
        assert!(ws.read_with(cx, |w, _| w.menu.sub.is_some()), "hovering Import opens it");
        let sub = cx.debug_bounds("menu-sub").expect("submenu renders");
        assert!(sub.left() >= row.right() - px(8.0), "beside the row");

        // Straight across onto the submenu: everything stays open.
        let mut x = row.center().x;
        while x < sub.center().x {
            x += px(6.0);
            cx.simulate_mouse_move(point(x, sub.top() + px(14.0)), None, Modifiers::default());
        }
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |w, _| (w.menu.open, w.menu.sub.is_some())), (Some(0), true));

        // Back over another row of File: the submenu closes, File stays.
        let panel = cx.debug_bounds("menu-panel").unwrap();
        cx.simulate_mouse_move(point(panel.center().x, panel.top() + px(20.0)), None, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |w, _| (w.menu.open, w.menu.sub)), (Some(0), None));
    }
}
