//! Debug builds: `GRAPHING_DEBUG_OPEN` opens a piece of UI at start, so it
//! can be screenshotted without synthetic input. The values are listed in
//! `docs/debugging.md`.

use std::time::Duration;

use gpui_kit::{Context, Entity, Window};

use super::{RightTab, Workspace};
use crate::dock::Tool;
use crate::view::DiagramView;

/// How long hooks that need a placed canvas wait for the first paint.
const FIRST_PAINT: Duration = Duration::from_millis(800);

/// Selects that `open-select:<id>` can open.
const SELECTS: [&str; 8] = ["diagram-kind", "shape", "edge-kind", "node-group", "group-add", "group-look", "group-kind", "library-notation"];

impl Workspace {
    pub(super) fn debug_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cfg!(debug_assertions) {
            return;
        }
        let Ok(hook) = std::env::var("GRAPHING_DEBUG_OPEN") else { return };
        let (name, arg) = hook.split_once(':').unwrap_or((hook.as_str(), ""));
        let arg = arg.to_string();
        let view = self.view().clone();
        match name {
            "palette" => self.open_palette(window, cx),
            "templates" => self.open_templates(window, cx),
            // `settings`, `settings:page` (appearance, canvas, panels,
            // shortcuts, extensions, oss), `settings:query`, and
            // `settings:oss=name` with the Open Source entry `name` open.
            "settings" => {
                use crate::settings_pane::Section;
                self.show_tool(Tool::Settings, window, cx);
                let (query, open) = arg.split_once('=').map_or((arg.as_str(), None), |(q, o)| (q, Some(o)));
                let page = match query {
                    "appearance" => Some(Section::Appearance),
                    "canvas" => Some(Section::Canvas),
                    "panels" => Some(Section::Panels),
                    "shortcuts" => Some(Section::Shortcuts),
                    "extensions" => Some(Section::Extensions),
                    "oss" => Some(Section::OpenSource),
                    _ => None,
                };
                match page {
                    Some(p) => self.settings_section = p,
                    None if !query.is_empty() => self.settings_query.update(cx, |s, cx| s.set_value(query.to_string(), window, cx)),
                    None => {}
                }
                self.oss_open = open.and_then(|name| crate::open_source::CRATES.iter().position(|c| c.name == name));
                if let Some(name) = open.filter(|_| self.oss_open.is_some()) {
                    // Find it, so it is on screen.
                    self.settings_query.update(cx, |s, cx| s.set_value(name.to_string(), window, cx));
                }
            }
            // `library:containers` or `library:shapes` filters the Shapes pane.
            "library" => {
                self.library_filter = match arg.as_str() {
                    "containers" => crate::library::Kinds::Containers,
                    "shapes" => crate::library::Kinds::Shapes,
                    _ => self.library_filter,
                }
            }
            // `notation:*` lists every notation in the Shapes pane, `notation:bpmn` one.
            "notation" => self.library_scope = arg,
            // `menu:0` opens the first menu; `menu:0/7` also its submenu at row 7.
            "menu" => {
                let (menu, sub) = arg.split_once('/').map_or((arg.as_str(), None), |(a, b)| (a, b.parse().ok()));
                self.menu.open = menu.parse().ok();
                self.menu.sub = sub;
            }
            "open-select" => {
                if let Some(id) = SELECTS.into_iter().find(|id| *id == arg) {
                    self.toggle_select(id, window, cx);
                }
            }
            "select" => self.with_view(cx, |v, cx| v.select(arg.split(',').map(str::to_string).collect(), cx)),
            // `notices` shows one of each: working, done (with actions), failed.
            "notices" => {
                use graphing_ui::kit::{Lucide, NoticeTone};
                let w = self.notice(NoticeTone::Working, "Exporting node-graph.webm", Some("~/Videos".into()), window, cx);
                self.update_notice(w, |n| n.progress = Some(0.42), window, cx);
                let ok = self.notice(NoticeTone::Working, "Exported node-graph.gif", Some("~/Pictures".into()), window, cx);
                self.update_notice(
                    ok,
                    |n| {
                        n.tone = NoticeTone::Success;
                        let noop: crate::notices::NoticeAct = std::rc::Rc::new(|_, _, _| {});
                        n.actions = vec![("Show in folder".into(), Lucide::FolderOpen, noop.clone()), ("Open".into(), Lucide::ExternalLink, noop)];
                    },
                    window,
                    cx,
                );
                self.notice(NoticeTone::Error, "Could not export node-graph.apng", Some("No space left on device".into()), window, cx);
            }
            // `export-menu` opens the sequence strip's export menu.
            "export-menu" => {
                self.sequence_open = true;
                // About where the strip's export button sits.
                let v = window.viewport_size();
                self.export_menu = Some(gpui_kit::point(v.width * 0.75, v.height * 0.92));
            }
            // `save-block:sel` selects `sel` and asks to save it as a block.
            "save-block" => {
                self.with_view(cx, |v, cx| v.select(arg.split(',').map(str::to_string).collect(), cx));
                self.save_block(window, cx);
            }
            // `pin:clamp/in/hi` selects `clamp` and opens pin `hi` in the inspector.
            "pin" => {
                let parts: Vec<&str> = arg.splitn(3, '/').collect();
                if let [node, dir, name] = parts[..] {
                    self.with_view(cx, |v, cx| v.select(vec![node.to_string()], cx));
                    let dir = if dir == "out" { graphing_scene::pins::PinDir::Out } else { graphing_scene::pins::PinDir::In };
                    self.pin_open = Some((node.to_string(), dir, name.to_string()));
                }
            }
            // `ask-images` shows the picture dialog with a sample picture.
            "ask-images" => {
                let img = crate::view::IncomingImage { name: "photo.png".into(), path: Some("photo.png".into()), bytes: Vec::new() };
                let at = view.read(cx).center();
                self.add_images(view, vec![img], at, window, cx);
            }
            "confirm-delete" => self.confirm_delete(view, vec![arg], window, cx),
            // `open-color:fill@api` selects `api` and opens its fill picker.
            "open-color" => {
                let (key, id) = arg.split_once('@').unwrap_or(("fill", ""));
                let key = match key {
                    "stroke" => "stroke",
                    "color" => "color",
                    _ => "fill",
                };
                let id = id.to_string();
                self.with_view(cx, |v, cx| v.select(vec![id.clone()], cx));
                self.right_tab = RightTab::Style;
                self.toggle_color(&id, key, window, cx);
            }
            // `zoom:3` zooms the canvas in (the minimap shows).
            "zoom" => {
                let z: f32 = arg.parse().unwrap_or(2.0);
                after_paint(view, window, cx, move |v, _, cx| v.zoom_center(z, cx));
            }
            // `step:N` previews step N (from 1); `play` starts the animation.
            "step" | "play" => {
                let n: usize = arg.parse().unwrap_or(0);
                after_paint(view, window, cx, move |v, _, cx| if n == 0 { v.toggle_play(cx) } else { v.preview_step(n - 1, cx) });
            }
            // `context:id` right-clicks `id` (`context:` alone, the empty
            // canvas); `rename:id` starts editing its label.
            "context" => after_paint(view, window, cx, move |v, _, cx| v.debug_context(&arg, cx)),
            "rename" => after_paint(view, window, cx, move |v, window, cx| v.start_rename(&arg, window, cx)),
            // `drag-bench:id` drags `id` for 60 frames, prints the time, quits.
            "drag-bench" => {
                self.with_view(cx, |v, cx| v.select(vec![arg], cx));
                cx.spawn_in(window, async move |_, cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    let t = std::time::Instant::now();
                    for i in 1..=60 {
                        view.update(cx, |v, cx| v.bench_drag(i, cx));
                        cx.background_executor().timer(Duration::from_millis(16)).await;
                    }
                    eprintln!("[bench] 60 steps in {:?}", t.elapsed());
                    cx.update(|_, cx| cx.quit()).ok();
                })
                .detach();
            }
            _ => {}
        }
    }
}

/// Run `f` on the canvas once its first paint has placed it.
fn after_paint(view: Entity<DiagramView>, window: &mut Window, cx: &mut Context<Workspace>, f: impl FnOnce(&mut DiagramView, &mut Window, &mut Context<DiagramView>) + 'static) {
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(FIRST_PAINT).await;
        view.update_in(cx, f).ok();
    })
    .detach();
}
