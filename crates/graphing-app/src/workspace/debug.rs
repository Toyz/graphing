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
            // `settings`, `settings:query`, and `settings:oss=name` with the
            // Open Source entry `name` expanded.
            "settings" => {
                self.show_tool(Tool::Settings, window, cx);
                let (query, open) = arg.split_once('=').map_or((arg.as_str(), None), |(q, o)| (q, Some(o)));
                let query = if query == "oss" { "open source" } else { query };
                if !query.is_empty() {
                    self.settings_query.update(cx, |s, cx| s.set_value(query.to_string(), window, cx));
                }
                self.oss_open = open.and_then(|name| crate::open_source::CRATES.iter().position(|c| c.name == name));
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
