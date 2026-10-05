use super::*;
use crate::view::IncomingImage;
use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};

const SRC: &str = "a: \"A\"\nb\na -> b\n\nlayout {\n  a 0 0\n  b 400 0\n}\n";

fn screen(v: &DiagramView, p: graphing_model::Point) -> Point<Pixels> {
    let o = v.bounds.get().origin;
    let c = v.cam.get();
    point(o.x + px(c.offset.x + p.x as f32 * c.zoom), o.y + px(c.offset.y + p.y as f32 * c.zoom))
}

fn open<'a>(cx: &'a mut TestAppContext, src: &str) -> (Entity<DiagramView>, &'a mut VisualTestContext) {
    let doc = Document::parse(src);
    crate::test_support::window(cx, |window, cx| cx.new(|cx| DiagramView::new(doc, None, window, cx)))
}

fn at(view: &Entity<DiagramView>, cx: &mut VisualTestContext, x: f64, y: f64) -> Point<Pixels> {
    view.read_with(cx, |v, _| screen(v, graphing_model::Point::new(x, y)))
}

fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>, button: MouseButton) {
    let none = Modifiers::default();
    cx.simulate_mouse_move(from, None, none);
    cx.simulate_mouse_down(from, button, none);
    cx.simulate_mouse_move(to, button, none);
    cx.simulate_mouse_up(to, button, none);
}

fn source(view: &Entity<DiagramView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| v.doc.source().to_string())
}

#[gpui_kit::test]
fn drag_writes_layout_and_undo_restores(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let (from, to) = (at(&view, cx, 60.0, 28.0), at(&view, cx, 63.0, 128.0));
    drag(cx, from, to, MouseButton::Left);
    let selected = view.read_with(cx, |v, _| v.selected.clone());
    assert_eq!(selected, ["a"]);
    assert!(source(&view, cx).contains("  a 0 100\n"), "{}", source(&view, cx));

    cx.simulate_keystrokes("secondary-z");
    assert_eq!(source(&view, cx), SRC);
    cx.simulate_keystrokes("secondary-shift-z");
    assert!(source(&view, cx).contains("  a 0 100\n"));
}

#[gpui_kit::test]
fn delete_removes_node_edges_and_layout(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let p = at(&view, cx, 450.0, 20.0);
    cx.simulate_click(p, Modifiers::default());
    cx.simulate_keystrokes("delete");
    assert_eq!(source(&view, cx), "a: \"A\"\n\nlayout {\n  a 0 0\n}\n");
}

#[gpui_kit::test]
fn middle_drag_pans(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let start = at(&view, cx, 200.0, 300.0);
    let before = view.read_with(cx, |v, _| v.cam.get().offset);
    let end = point(start.x + px(40.0), start.y + px(25.0));
    drag(cx, start, end, MouseButton::Middle);
    let after = view.read_with(cx, |v, _| v.cam.get().offset);
    assert_eq!((after.x - before.x, after.y - before.y), (40.0, 25.0));
}

#[gpui_kit::test]
fn marquee_selects_enclosed_nodes(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let (from, to) = (at(&view, cx, -20.0, -20.0), at(&view, cx, 300.0, 100.0));
    drag(cx, from, to, MouseButton::Left);
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["a"]);
    cx.simulate_keystrokes("secondary-a");
    assert_eq!(view.read_with(cx, |v, _| v.selected.len()), 2);
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |v, _| v.selected.is_empty()));
}

#[gpui_kit::test]
fn double_click_empty_creates_and_renames(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let p = at(&view, cx, 200.0, 300.0);
    cx.simulate_event(MouseDownEvent { position: p, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position: p, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2 });
    let renaming = view.read_with(cx, |v, _| v.renaming.clone());
    assert_eq!(renaming.as_deref(), Some("n1"));
    cx.simulate_input("Queue");
    cx.simulate_keystrokes("enter");
    let src = source(&view, cx);
    assert!(src.contains("n1: \"Queue\""), "{src}");
    assert!(src.contains("  n1 140 270\n"), "{src}");
}

#[gpui_kit::test]
fn port_drag_connects_and_drop_on_empty_creates(cx: &mut TestAppContext) {
    let src = "a\nb\nlayout {\n  a 0 0\n  b 400 0\n}\n";
    let (view, cx) = open(cx, src);
    // Right-side port of `a` (120 x 56 box).
    let port = at(&view, cx, 120.0, 28.0);
    let onto_b = at(&view, cx, 460.0, 28.0);
    drag(cx, port, onto_b, MouseButton::Left);
    assert!(source(&view, cx).contains("a -> b\n"), "{}", source(&view, cx));

    let empty = at(&view, cx, 200.0, 300.0);
    drag(cx, port, empty, MouseButton::Left);
    let src = source(&view, cx);
    assert!(src.contains("a -> n1\n") && src.contains("  n1 140 270\n"), "{src}");
    assert_eq!(view.read_with(cx, |v, _| v.renaming.clone()).as_deref(), Some("n1"));
}

#[gpui_kit::test]
fn corner_drag_resizes(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let body = at(&view, cx, 60.0, 28.0);
    cx.simulate_click(body, Modifiers::default());
    let (corner, to) = (at(&view, cx, 120.0, 56.0), at(&view, cx, 203.0, 98.0));
    drag(cx, corner, to, MouseButton::Left);
    assert!(source(&view, cx).contains("  a 0 0 200x100\n"), "{}", source(&view, cx));
}

#[gpui_kit::test]
fn copy_paste_and_duplicate(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let p = at(&view, cx, 60.0, 28.0);
    cx.simulate_click(p, Modifiers::default());
    cx.simulate_keystrokes("secondary-c");
    cx.simulate_keystrokes("secondary-v");
    let src = source(&view, cx);
    assert!(src.contains("a_1: \"A\"") && src.contains("  a_1 20 20\n"), "{src}");
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["a_1"]);
    cx.simulate_keystrokes("secondary-d");
    assert!(source(&view, cx).contains("  a_2 40 40\n"));
    cx.simulate_keystrokes("shift-right right");
    assert!(source(&view, cx).contains("  a_2 51 40\n"), "{}", source(&view, cx));
}

#[gpui_kit::test]
fn click_never_nudges_off_grid_node(cx: &mut TestAppContext) {
    let src = "a\nlayout {\n  a 3 7\n}\n";
    let (view, cx) = open(cx, src);
    let p = at(&view, cx, 60.0, 30.0);
    // A press with a hair of jitter is still a click.
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(point(p.x + px(1.0), p.y), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point(p.x + px(1.0), p.y), MouseButton::Left, Modifiers::default());
    assert_eq!(source(&view, cx), src);
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["a"]);
}

#[gpui_kit::test]
fn drag_port_to_port_makes_ported_connector(cx: &mut TestAppContext) {
    // Unconnected declared ports sit on the left side, centered.
    let src = "a { ports: [out] }\nb { ports: [in] }\nlayout {\n  a 0 0\n  b 400 0\n}\n";
    let (view, cx) = open(cx, src);
    let (pa, pb) = view.read_with(cx, |v, _| {
        let s = v.scene();
        (s.port("a", "out").unwrap().at, s.port("b", "in").unwrap().at)
    });
    let (from, to) = (at(&view, cx, pa.x, pa.y), at(&view, cx, pb.x, pb.y));
    drag(cx, from, to, MouseButton::Left);
    let out = source(&view, cx);
    assert!(out.contains("a.out -- b.in\n"), "{out}");
}

#[gpui_kit::test]
fn group_and_ungroup_keys(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let empty = at(&view, cx, 200.0, 300.0);
    cx.simulate_click(empty, Modifiers::default());
    cx.simulate_keystrokes("secondary-a secondary-g");
    let src = source(&view, cx);
    assert!(src.contains("group group1 \"Group\" { a b }"), "{src}");
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["group1"]);
    cx.simulate_keystrokes("secondary-shift-g");
    assert!(!source(&view, cx).contains("group group1"));
}

#[gpui_kit::test]
fn new_nodes_outside_groups_stay_outside(cx: &mut TestAppContext) {
    let src = "a\nb\ngroup g \"G\" { a b }\nlayout {\n  a 0 0\n  b 200 0\n}\n";
    let (view, cx) = open(cx, src);
    // Well outside the group's box.
    let p = at(&view, cx, 100.0, 400.0);
    cx.simulate_event(MouseDownEvent { position: p, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position: p, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2 });
    cx.simulate_keystrokes("escape");
    let members = view.read_with(cx, |v, _| v.doc.diagram().group("g").unwrap().members.clone());
    assert_eq!(members, ["a", "b"], "{}", source(&view, cx));
    let src = source(&view, cx);
    assert!(src.contains("group g \"G\" { a b }"), "{src}");
}

#[gpui_kit::test]
fn grouping_then_adding_inside_and_outside(cx: &mut TestAppContext) {
    let src = "a\nb\nlayout {\n  a 0 0\n  b 300 0\n  far 1200 800\n}\nfar\n";
    let (view, cx) = open(cx, src);
    let dbl = |cx: &mut VisualTestContext, p: Point<Pixels>| {
        cx.simulate_event(MouseDownEvent { position: p, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2, first_mouse: false });
        cx.simulate_event(MouseUpEvent { position: p, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2 });
        cx.simulate_keystrokes("escape");
    };
    let a = at(&view, cx, 60.0, 28.0);
    cx.simulate_click(a, Modifiers::default());
    let b = at(&view, cx, 360.0, 28.0);
    cx.simulate_event(MouseDownEvent { position: b, modifiers: Modifiers::shift(), button: MouseButton::Left, click_count: 1, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position: b, modifiers: Modifiers::shift(), button: MouseButton::Left, click_count: 1 });
    cx.simulate_keystrokes("secondary-g");
    // Outside, between the group and `far`.
    let out = at(&view, cx, 700.0, 500.0);
    dbl(cx, out);
    // Inside the group's open area, below its title strip.
    let group = view.read_with(cx, |v, _| v.scene().rect_of("group1").unwrap());
    let inside = at(&view, cx, group.center().x, group.origin.y + group.size.h - 10.0);
    dbl(cx, inside);
    let d = view.read_with(cx, |v, _| v.doc.diagram().clone());
    assert_eq!(d.nodes.len(), 5, "{}", source(&view, cx));
    let members = &d.group("group1").unwrap().members;
    assert!(members.contains(&"a".into()) && members.contains(&"b".into()));
    assert_eq!(members.len(), 3, "{}", source(&view, cx));
}

const GROUPS: &str = "a\nb\ngroup g1 \"A\" { a }\ngroup g2 \"B\" { b }\nlayout {\n  a 40 60\n  b 640 60\n  g1 0 0 300x200\n  g2 600 0 300x200\n}\n";

#[gpui_kit::test]
fn groups_connect_from_their_handles(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, GROUPS);
    // Right-edge handle of g1, dropped in g2's open area.
    let handle = at(&view, cx, 300.0, 100.0);
    let into_g2 = at(&view, cx, 620.0, 180.0);
    drag(cx, handle, into_g2, MouseButton::Left);
    assert!(source(&view, cx).contains("g1 -> g2\n"), "{}", source(&view, cx));
    // A node can link to a group too.
    let a_port = at(&view, cx, 160.0, 88.0);
    drag(cx, a_port, into_g2, MouseButton::Left);
    assert!(source(&view, cx).contains("a -> g2\n"), "{}", source(&view, cx));
    // No implied node appeared for the group ids.
    let d = view.read_with(cx, |v, _| v.doc().diagram().clone());
    assert_eq!(d.nodes.len(), 2);
}

#[gpui_kit::test]
fn selected_group_resizes_from_corner(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, GROUPS);
    let title = at(&view, cx, 150.0, 8.0);
    cx.simulate_click(title, Modifiers::default());
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["g1"]);
    let (corner, to) = (at(&view, cx, 300.0, 200.0), at(&view, cx, 401.0, 251.0));
    drag(cx, corner, to, MouseButton::Left);
    assert!(source(&view, cx).contains("  g1 0 0 400x250\n"), "{}", source(&view, cx));
}

#[gpui_kit::test]
fn source_typing_and_canvas_edits_share_undo(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let before = source(&view, cx);
    let typed = format!("{before}c: rect \"C\"\n");
    view.update(cx, |v, cx| v.set_source(typed.clone(), cx));
    let after_typing = source(&view, cx);
    // A canvas edit after typing, then undo both in order.
    view.update(cx, |v, cx| {
        v.apply(Op::SetLabel { id: "a".into(), label: Some("Renamed".into()) }, cx);
    });
    view.update(cx, |v, cx| v.undo_op(cx));
    assert_eq!(source(&view, cx), after_typing);
    view.update(cx, |v, cx| v.undo_op(cx));
    assert_eq!(source(&view, cx), before);
    view.update(cx, |v, cx| v.redo_op(cx));
    assert_eq!(source(&view, cx), after_typing);
}

#[gpui_kit::test]
fn clicking_inside_a_group_selects_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, GROUPS);
    // Open area of g1, away from its member and its handles.
    let p = at(&view, cx, 220.0, 170.0);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    cx.simulate_click(p, Modifiers::default());
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["g1"]);
}

#[gpui_kit::test]
fn clicking_a_fitted_group_selects_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "a\nb\ngroup g \"G\" { a b }\nlayout {\n  a 0 0\n  b 300 0\n}\n");
    let r = view.read_with(cx, |v, _| v.scene().rect_of("g").unwrap());
    // Below the members, inside the group's padding.
    let p = at(&view, cx, r.origin.x + r.size.w / 2.0, r.origin.y + r.size.h - 6.0);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    cx.simulate_click(p, Modifiers::default());
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["g"]);
    let title = at(&view, cx, r.origin.x + 30.0, r.origin.y + 6.0);
    cx.simulate_click(title, Modifiers::default());
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["g"]);
}

#[gpui_kit::test]
fn sysml_frame_selects_and_renames_the_diagram(cx: &mut TestAppContext) {
    let src = "diagram \"Bench\" { kind: ibd }\nuse sysml\na: sysml.part \"A\"\nlayout {\n  a 0 0\n}\n";
    let (view, cx) = open(cx, src);
    let f = view.read_with(cx, |v, _| v.scene().frame.clone().expect("ibd draws a frame").rect);
    let head = at(&view, cx, f.origin.x + 40.0, f.origin.y + 10.0);
    cx.simulate_click(head, Modifiers::default());
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), [FRAME_ID]);
    // Delete with the frame selected changes nothing.
    cx.simulate_keystrokes("delete");
    assert_eq!(source(&view, cx), src);
    cx.simulate_event(MouseDownEvent { position: head, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position: head, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: 2 });
    assert_eq!(view.read_with(cx, |v, _| v.renaming.clone()).as_deref(), Some(FRAME_ID));
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("Test Bench");
    cx.simulate_keystrokes("enter");
    assert!(source(&view, cx).starts_with("diagram \"Test Bench\" { kind: ibd }"), "{}", source(&view, cx));
}

#[gpui_kit::test]
fn library_group_tile_creates_a_sized_group_around_what_it_covers(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    // Centered on `a` (0,0 120x56); 360x220 covers it but not `b` at x=400.
    view.update_in(cx, |v, window, cx| v.add_shape_at("group:lane", graphing_model::Point::new(60.0, 28.0), window, cx));
    cx.simulate_keystrokes("escape");
    let src = source(&view, cx);
    assert!(src.contains("group group1 \"Swimlane\" { a } { kind: lane }"), "{src}");
    assert!(src.contains("  group1 -120 -82 360x220\n"), "{src}");
}

#[gpui_kit::test]
fn dragging_a_member_out_of_a_fitted_group_leaves_it(cx: &mut TestAppContext) {
    let src = "a\nb\ngroup g \"G\" { a b }\nlayout {\n  a 0 0\n  b 200 0\n}\n";
    let (view, cx) = open(cx, src);
    let (from, to) = (at(&view, cx, 260.0, 28.0), at(&view, cx, 260.0, 160.0));
    drag(cx, from, to, MouseButton::Left);
    let out = source(&view, cx);
    assert!(out.contains("group g \"G\" { a }"), "{out:?}");
}

#[gpui_kit::test]
fn delete_removes_a_group_with_its_contents(cx: &mut TestAppContext) {
    let src = "a\nb\nc\ngroup inner \"I\" { b }\ngroup g \"G\" { a inner }\na -> c\ng -> c\nlayout {\n  a 0 0\n  b 0 100\n  c 400 0\n}\n";
    let (view, cx) = open(cx, src);
    let empty = at(&view, cx, 300.0, 300.0);
    cx.simulate_click(empty, Modifiers::default());
    view.update(cx, |v, cx| v.select(vec!["g".into()], cx));
    cx.simulate_keystrokes("delete");
    let out = source(&view, cx);
    let d = view.read_with(cx, |v, _| v.doc().diagram().clone());
    assert!(d.groups.is_empty() && d.edges.is_empty(), "{out}");
    assert_eq!(d.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["c"], "{out}");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(source(&view, cx), src);
}

#[gpui_kit::test]
fn resizing_a_group_over_a_node_takes_it_in(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, GROUPS);
    let title = at(&view, cx, 150.0, 8.0);
    cx.simulate_click(title, Modifiers::default());
    // g1 is 0,0 300x200; b sits at 640,60 inside g2. Add a free node `c` at
    // 320,220 and grow g1 to cover it.
    view.update_in(cx, |v, window, cx| v.add_shape_at("rect", graphing_model::Point::new(380.0, 250.0), window, cx));
    cx.simulate_keystrokes("escape");
    cx.simulate_click(title, Modifiers::default());
    let (corner, to) = (at(&view, cx, 300.0, 200.0), at(&view, cx, 501.0, 321.0));
    drag(cx, corner, to, MouseButton::Left);
    let src = source(&view, cx);
    let d = view.read_with(cx, |v, _| v.doc().diagram().clone());
    let new_node = d.nodes.iter().find(|n| n.id != "a" && n.id != "b").unwrap().id.clone();
    assert!(d.group("g1").unwrap().members.contains(&new_node), "{src}");
    // Shrinking it back lets the node go.
    cx.simulate_click(title, Modifiers::default());
    let (corner, to) = (at(&view, cx, 500.0, 320.0), at(&view, cx, 301.0, 201.0));
    drag(cx, corner, to, MouseButton::Left);
    let d = view.read_with(cx, |v, _| v.doc().diagram().clone());
    assert!(!d.group("g1").unwrap().members.contains(&new_node), "{}", source(&view, cx));
}

#[gpui_kit::test]
fn a_sized_group_follows_the_pointer_while_dragged(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, GROUPS);
    let title = at(&view, cx, 150.0, 8.0);
    cx.simulate_click(title, Modifiers::default());
    let none = Modifiers::default();
    cx.simulate_mouse_down(title, MouseButton::Left, none);
    let to = at(&view, cx, 250.0, 108.0);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    // Mid-drag, before release: the box has moved with its member.
    let (g, a) = view.read_with(cx, |v, _| {
        let s = v.scene();
        (s.rect_of("g1").unwrap().origin, s.rect_of("a").unwrap().origin)
    });
    assert_eq!((g.x, g.y), (100.0, 100.0));
    assert_eq!((a.x, a.y), (140.0, 160.0));
    cx.simulate_mouse_up(to, MouseButton::Left, none);
}

#[gpui_kit::test]
fn pictures_go_into_the_package_and_survive_a_save(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let img = IncomingImage { name: "Logo.png".into(), path: None, bytes: b"\x89PNG not really".to_vec() };
    view.update_in(cx, |v, window, cx| v.insert_images(vec![img.clone(), img], graphing_model::Point::new(100.0, 100.0), true, window, cx));
    let (src, assets) = view.read_with(cx, |v, _| (v.doc().source().to_string(), v.assets().clone()));
    // Two nodes, one stored copy.
    assert_eq!(src.matches(": image").count(), 2, "{src}");
    assert_eq!(assets.len(), 1);
    let name = assets.keys().next().unwrap().clone();
    assert!(src.contains(&format!("asset:{name}")), "{src}");

    let file = std::env::temp_dir().join(format!("graphing-pics-{}.gphz", std::process::id()));
    view.update(cx, |v, cx| v.save_to(file.clone(), cx)).unwrap();
    let (text, back) = crate::files::load(&file).unwrap();
    assert_eq!(text, src);
    assert_eq!(back, assets);
    let _ = std::fs::remove_file(&file);
}

#[gpui_kit::test]
fn right_click_opens_a_menu_for_what_is_under_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let p = at(&view, cx, 450.0, 20.0);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    cx.simulate_mouse_down(p, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(p, MouseButton::Right, Modifiers::default());
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), ["b"], "right-click selects what it is on");
    assert!(view.read_with(cx, |v, _| v.menu.is_some()));
    assert!(cx.debug_bounds("canvas-menu").is_some(), "the menu renders");
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |v, _| v.menu.is_none()));

    // On empty canvas: nothing selected, still a menu.
    let empty = at(&view, cx, 200.0, 300.0);
    cx.simulate_mouse_down(empty, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(empty, MouseButton::Right, Modifiers::default());
    assert!(view.read_with(cx, |v, _| v.selected.is_empty() && v.menu.is_some()));
}

#[gpui_kit::test]
fn right_drag_pans_without_a_menu(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    let start = at(&view, cx, 200.0, 300.0);
    let before = view.read_with(cx, |v, _| v.cam.get().offset);
    drag(cx, start, point(start.x + px(30.0), start.y), MouseButton::Right);
    let after = view.read_with(cx, |v, _| v.cam.get().offset);
    assert_eq!(after.x - before.x, 30.0);
    assert!(view.read_with(cx, |v, _| v.menu.is_none()));
}

#[gpui_kit::test]
fn label_editor_matches_painted_labels(cx: &mut TestAppContext) {
    let src = "a: \"A\"\nb\ngroup g \"Team\" { a }\n\nlayout {\n  a 0 0\n  b 400 0\n  g -20 -60 300 200\n}\n";
    let (view, cx) = open(cx, src);
    view.read_with(cx, |v, _| {
        let node = v.label_spot("a").expect("node");
        assert_eq!((node.pt, node.face, node.left), (paint::LABEL_PT, Face::Regular, false));
        // A group edits in its header strip, not over its members and links.
        let g = v.label_spot("g").expect("group");
        let scene = v.scene();
        let gb = scene.groups.iter().find(|x| x.id == "g").unwrap();
        assert_eq!(g.rect.size.h, gb.look.head());
        assert!(g.left && g.rect.origin.x > gb.rect.origin.x);
    });
}

#[gpui_kit::test]
fn steps_build_from_the_selection_and_play(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    view.update(cx, |v, cx| {
        v.select(vec!["a".into()], cx);
        assert_eq!(v.add_step(cx), 0);
        v.select(vec!["b".into()], cx);
        assert_eq!(v.add_step(cx), 1);
    });
    let src = source(&view, cx);
    assert!(src.contains("animate {\n  step \"A\" {\n    show a\n    highlight a\n  }"), "{src}");
    // b's step runs dots along the line that reaches it from a.
    assert!(src.contains("step \"b\" {\n    show b\n    flow a -> b\n    highlight b\n  }"), "{src}");
    view.update(cx, |v, cx| {
        // Previewing step 1: a shows, b waits.
        v.preview_step(0, cx);
        let (st, _) = v.anim_state().unwrap();
        assert_eq!((st.alpha("a"), st.alpha("b")), (1.0, 0.0));
        v.step_by(1, cx);
        assert_eq!(v.current_step(), Some(1));
        v.toggle_play(cx);
        assert!(v.playing());
        v.toggle_play(cx);
        assert!(!v.playing() && v.player.is_some());
        v.edit_step(0, |s| s.seconds = Some(3.0), cx);
        v.move_step(1, -1, cx);
        assert_eq!(v.steps()[0].title.as_deref(), Some("b"));
        v.delete_step(0, cx);
        v.delete_step(0, cx);
        assert!(v.steps().is_empty() && v.player.is_none());
    });
    // Undo brings the steps back.
    view.update(cx, |v, cx| {
        v.undo_op(cx);
        v.undo_op(cx);
    });
    assert_eq!(view.read_with(cx, |v, _| v.steps().len()), 2);
}

#[gpui_kit::test]
fn clicking_the_canvas_ends_a_preview(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    view.update(cx, |v, cx| {
        v.select(vec!["a".into()], cx);
        v.add_step(cx);
    });
    assert!(view.read_with(cx, |v, _| v.player.is_some()));
    let p = at(&view, cx, 200.0, 300.0);
    cx.simulate_click(p, Modifiers::default());
    assert!(view.read_with(cx, |v, _| v.player.is_none()));
}

#[gpui_kit::test]
fn dragging_while_previewing_records_a_move(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    view.update(cx, |v, cx| {
        v.select(vec!["b".into()], cx);
        v.add_step(cx);
    });
    let layout_before = source(&view, cx);
    assert!(layout_before.contains("  b 400 0\n"));
    let (from, to) = (at(&view, cx, 450.0, 20.0), at(&view, cx, 450.0, 220.0));
    drag(cx, from, to, MouseButton::Left);
    let src = source(&view, cx);
    // The layout keeps b where it was; the step moves it.
    assert!(src.contains("  b 400 0\n"), "{src}");
    assert!(src.contains("    move b 400 200\n"), "{src}");
    assert!(view.read_with(cx, |v, _| v.player.is_some()), "still previewing");
    // The preview shows b at its new place.
    let b = view.read_with(cx, |v, _| v.scene().rect_of("b").unwrap());
    assert_eq!(b.origin.y, 200.0);
}

#[gpui_kit::test]
fn the_minimap_shows_when_things_are_out_of_view(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SRC);
    cx.run_until_parked();
    assert!(cx.debug_bounds("minimap").is_none(), "everything fits at first");
    view.update(cx, |v, cx| {
        let mut c = v.cam.get();
        c.zoom = 3.0;
        c.fit = false;
        c.auto = false;
        v.cam.set(c);
        cx.notify();
    });
    cx.run_until_parked();
    let map = cx.debug_bounds("minimap").expect("minimap appears once zoomed in");
    let before = view.read_with(cx, |v, _| v.cam.get().offset);
    // Clicking its far corner looks over there.
    let corner = point(map.origin.x + map.size.width - px(10.0), map.origin.y + map.size.height - px(10.0));
    cx.simulate_click(corner, Modifiers::default());
    let after = view.read_with(cx, |v, _| v.cam.get().offset);
    assert_ne!(before, after);
}
