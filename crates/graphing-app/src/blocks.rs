//! Saved blocks: shapes set up the way you like them (a Select with its
//! pins, a sub-graph with the wires inside it) kept in `<config>/blocks/`
//! as small `.gph` files, and dropped into any diagram as a fresh copy.
//! A copy is the diagram's own: editing it leaves the saved block alone,
//! and saving over the block changes only what is dropped in later, so
//! diagrams never depend on someone's blocks folder.

use std::io;
use std::path::PathBuf;

use gpui_kit::{AnyElement, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Pixels, Point as PxPoint, SharedString, Window};
use graphing_dsl::Document;
use graphing_model::{Diagram, Op, Placement, Point};
use graphing_scene::Scene;
use graphing_ui::kit::Lucide;
use graphing_ui::menu::{self, MenuRow};

use crate::workspace::Workspace;

/// One saved block.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The file's name without `.gph`; how tiles and drags refer to it.
    pub name: String,
    pub title: String,
    pub text: String,
    /// The first shape's stencil, for the tile's picture.
    pub stencil: Option<String>,
    /// Shapes in it.
    pub count: usize,
}

/// Where blocks live.
pub fn dir() -> PathBuf {
    crate::settings::config_dir().join("blocks")
}

fn read(name: &str, text: String) -> Block {
    let doc = Document::parse(text.as_str());
    let d = doc.diagram();
    Block {
        name: name.to_string(),
        title: d.title.clone().unwrap_or_else(|| name.to_string()),
        stencil: d.nodes.first().and_then(|n| n.stencil.clone()),
        count: d.nodes.len(),
        text,
    }
}

/// Every saved block, by title.
pub fn list() -> Vec<Block> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut out: Vec<Block> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "gph"))
        .filter_map(|p| Some(read(&p.file_stem()?.to_string_lossy(), std::fs::read_to_string(&p).ok()?)))
        .collect();
    out.sort_by_key(|b| b.title.to_lowercase());
    out
}

pub fn load(name: &str) -> Option<Block> {
    Some(read(name, std::fs::read_to_string(dir().join(format!("{name}.gph"))).ok()?))
}

/// A file name for a title: `Fast select` -> `fast-select`.
fn slug(title: &str) -> String {
    let s: String = title.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "block".into() } else { s }
}

/// Whether a block already goes by `title`.
pub fn exists(title: &str) -> bool {
    dir().join(format!("{}.gph", slug(title))).exists()
}

/// Save `text` as the block `title` (replacing one of the same name).
pub fn save(title: &str, text: &str) -> io::Result<String> {
    std::fs::create_dir_all(dir())?;
    let name = slug(title);
    std::fs::write(dir().join(format!("{name}.gph")), text)?;
    Ok(name)
}

pub fn delete(name: &str) -> io::Result<()> {
    std::fs::remove_file(dir().join(format!("{name}.gph")))
}

/// Give block `name` a new title (and file name).
pub fn rename(name: &str, title: &str) -> io::Result<String> {
    let block = load(name).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such block"))?;
    let mut doc = Document::parse(block.text.as_str());
    doc.apply(&Op::SetTitle { title: Some(title.trim().to_string()) });
    let new = save(title, doc.source())?;
    if new != name {
        delete(name)?;
    }
    Ok(new)
}

/// The shapes `ids` of `d` (and the lines between them) as block text,
/// titled `title`. Shapes the layout never placed keep where `scene` drew
/// them, so the block comes back as it looked.
pub fn snippet(d: &Diagram, scene: &Scene, ids: &[String], title: &str) -> Option<String> {
    let mut probe = d.clone();
    for id in ids {
        if probe.node(id).is_some()
            && !probe.layout.contains_key(id)
            && let Some(r) = scene.rect_of(id)
        {
            probe.layout.insert(id.clone(), Placement { pos: r.origin, size: None });
        }
    }
    let body = crate::ops::copy(&probe, ids)?;
    Some(format!("diagram {}\n{body}", graphing_dsl::print::fmt_str(title)))
}

/// Ops placing a fresh copy of block `text` in `d`, centred on `at`.
pub fn insert(d: &Diagram, text: &str, at: Point) -> Option<(Vec<String>, Op)> {
    let src = Document::parse(text);
    let s = src.diagram();
    let boxes: Vec<(Point, (f64, f64))> = s
        .nodes
        .iter()
        .filter_map(|n| {
            let p = s.layout.get(&n.id)?;
            Some((p.pos, p.size.map_or_else(|| graphing_scene::default_size(n.text()), |z| (z.w, z.h))))
        })
        .collect();
    let (x0, y0) = boxes.iter().fold((f64::MAX, f64::MAX), |(x, y), (p, _)| (x.min(p.x), y.min(p.y)));
    let (x1, y1) = boxes.iter().fold((f64::MIN, f64::MIN), |(x, y), (p, (w, h))| (x.max(p.x + w), y.max(p.y + h)));
    let offset = if boxes.is_empty() { at } else { Point::new((at.x - (x0 + x1) / 2.0).round(), (at.y - (y0 + y1) / 2.0).round()) };
    crate::ops::paste(d, text, offset)
}

impl Workspace {
    pub(crate) fn refresh_blocks(&mut self) {
        self.blocks = list();
    }

    /// Ask for a name and save the selected shapes as a block.
    pub(crate) fn save_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.view().clone();
        let (ids, title) = {
            let v = view.read(cx);
            let d = v.doc().diagram();
            let ids: Vec<String> = v.selection().iter().filter(|id| d.node(id).is_some()).cloned().collect();
            let title = match ids.as_slice() {
                [one] => d.node(one).map(|n| n.text().to_string()).unwrap_or_default(),
                _ => format!("{} shapes", ids.len()),
            };
            (ids, title)
        };
        if ids.is_empty() {
            self.toast("select the shapes to keep as a block", window, cx);
            return;
        }
        let input = cx.new(|cx| {
            let mut s = gpui_kit::component::input::InputState::new(window, cx).placeholder("Block name");
            s.set_value(title, window, cx);
            s
        });
        let field = input.clone();
        let what = if ids.len() == 1 { "this shape, set up as it is" } else { "these shapes and the lines between them" };
        let c = crate::confirm::Confirm::prompt(
            "Save as block",
            format!("Keeps {what} in My blocks. Dropping it in places a copy that is the diagram's own; editing it leaves the block as saved."),
            Lucide::BookmarkPlus,
            input,
            "Save block",
            Box::new(move |ws, window, cx| {
                let title = field.read(cx).value().trim().to_string();
                let title = if title.is_empty() { "Block".to_string() } else { title };
                let text = {
                    let v = ws.view().read(cx);
                    snippet(v.doc().diagram(), &v.scene(), &ids, &title)
                };
                let Some(text) = text else { return };
                let replacing = exists(&title);
                let write = move |ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
                    match save(&title, &text) {
                        Ok(_) => ws.toast(format!("saved block \u{201c}{title}\u{201d}"), window, cx),
                        Err(e) => ws.toast(format!("could not save the block: {e}"), window, cx),
                    }
                    ws.refresh_blocks();
                    cx.notify();
                };
                if replacing {
                    let c = crate::confirm::Confirm::danger(
                        "Replace the saved block?",
                        "A block with this name exists. Copies already in diagrams stay as they are; only new ones change.",
                        "Replace block",
                        Box::new(write),
                    );
                    ws.ask(c, window, cx);
                } else {
                    write(ws, window, cx);
                }
            }),
        );
        self.ask(c, window, cx);
    }

    /// Drop a copy of block `name` into the open diagram at its centre.
    pub(crate) fn insert_block(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.view().read(cx).center();
        let name = name.to_string();
        self.with_view(cx, |v, cx| v.insert_block(&name, at, window, cx));
    }

    fn rename_block(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = load(name) else { return };
        let input = cx.new(|cx| {
            let mut s = gpui_kit::component::input::InputState::new(window, cx).placeholder("Block name");
            s.set_value(block.title.clone(), window, cx);
            s
        });
        let (field, name) = (input.clone(), name.to_string());
        let c = crate::confirm::Confirm::prompt("Rename block", "Copies already in diagrams keep their names.", Lucide::PencilLine, input, "Rename", Box::new(move |ws, window, cx| {
            let title = field.read(cx).value().trim().to_string();
            if title.is_empty() {
                return;
            }
            if let Err(e) = rename(&name, &title) {
                ws.toast(format!("could not rename the block: {e}"), window, cx);
            }
            ws.refresh_blocks();
            cx.notify();
        }));
        self.ask(c, window, cx);
    }

    fn delete_block(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = load(name) else { return };
        let name = name.to_string();
        let c = crate::confirm::Confirm::danger(
            format!("Delete the block \u{201c}{}\u{201d}?", block.title),
            "It leaves My blocks. Copies already in diagrams stay.",
            "Delete block",
            Box::new(move |ws, window, cx| {
                if let Err(e) = delete(&name) {
                    ws.toast(format!("could not delete the block: {e}"), window, cx);
                }
                ws.refresh_blocks();
                cx.notify();
            }),
        );
        self.ask(c, window, cx);
    }

    /// Right-click menu of a block tile.
    pub(crate) fn render_block_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (name, at): (String, PxPoint<Pixels>) = self.block_menu.clone()?;
        type Act = fn(&mut Workspace, &str, &mut Window, &mut Context<Workspace>);
        let item = |label: &str, icon: Lucide, f: Act, cx: &mut Context<Self>| {
            let name = name.clone();
            MenuRow::item(SharedString::from(label.to_string()), cx.listener(move |ws, _, window, cx| {
                ws.block_menu = None;
                f(ws, &name, window, cx);
                cx.notify();
            }))
            .icon(icon)
        };
        let rows = vec![
            item("Insert", Lucide::Plus, |ws, n, w, cx| ws.insert_block(n, w, cx), cx),
            item("Rename", Lucide::PencilLine, |ws, n, w, cx| ws.rename_block(n, w, cx), cx),
            MenuRow::Separator,
            item("Delete", Lucide::Trash, |ws, n, w, cx| ws.delete_block(n, w, cx), cx),
        ];
        let surface = menu::menu_surface("block-menu", rows, cx).on_mouse_down_out(cx.listener(|ws, _, _, cx| {
            ws.block_menu = None;
            cx.notify();
        }));
        let placed = gpui_kit::anchored().position(at).snap_to_window().child(menu::animate(surface, "block-menu-anim"));
        Some(gpui_kit::deferred(placed).with_priority(2).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_keeps_its_setup_and_lands_as_a_fresh_copy() {
        let src = "use graph\nsel: graph.select \"Fast select\" { in: [pick: bool, a: float, b: float], out: [result: float], defaults: [b: 1] }\nx: graph.variable { out: [value: float] }\nx.value -> sel.a\nlayout {\n  sel 300 100\n  x 0 100\n}\n";
        let doc = Document::parse(src);
        let d = doc.diagram();
        let scene = graphing_scene::build(d, &Default::default());
        // One shape: its props and pins, titled, and no outside wire.
        let text = snippet(d, &scene, &["sel".into()], "Fast select").unwrap();
        assert!(text.starts_with("diagram \"Fast select\"\n"), "{text}");
        assert!(text.contains("defaults: [b: 1]") && !text.contains("x.value"), "{text}");
        // Into another diagram that already has a `sel`: a new id, centred
        // where it was dropped, set up the same.
        let mut target = Document::parse("sel\n");
        let (ids, op) = insert(target.diagram(), &text, Point::new(1000.0, 1000.0)).unwrap();
        target.apply(&op).unwrap();
        assert_eq!(ids.len(), 1);
        assert_ne!(ids[0], "sel");
        let copy = target.diagram().node(&ids[0]).unwrap();
        assert_eq!(copy.stencil.as_deref(), Some("graph.select"));
        assert!(target.source().contains("defaults: [b: 1]"));
        let p = target.diagram().layout[&ids[0]].pos;
        assert!((p.x - 1000.0).abs() < 200.0 && (p.y - 1000.0).abs() < 200.0, "{p:?}");
        // Editing the copy changes nothing saved: the block text is separate.
        target.apply(&Op::SetLabel { id: ids[0].clone(), label: Some("Changed".into()) }).unwrap();
        assert!(text.contains("Fast select"));
        // Several shapes keep the lines between them.
        let both = snippet(d, &scene, &["sel".into(), "x".into()], "Pair").unwrap();
        assert!(both.contains("x.value -> sel.a"), "{both}");
    }

    #[test]
    fn names_become_file_names() {
        assert_eq!(slug("Fast select"), "fast-select");
        assert_eq!(slug("  A/B: test!  "), "a-b-test");
        assert_eq!(slug("***"), "block");
    }
}
