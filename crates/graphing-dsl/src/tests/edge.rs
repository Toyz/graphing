//! Hand-picked formatting and error cases: braces on their own lines,
//! comments in every gap, odd encodings, and text broken mid-typing.

use super::{assert_same, check};
use crate::Document;
use graphing_model::{Arrow, Diagram, Node, Op, Placement, Point, Value};

/// Parses with no problems.
fn clean(src: &str) -> Document {
    let d = Document::parse(src);
    assert!(d.diags().is_empty(), "{:?}\n{src}", d.diags());
    d
}

/// Both texts give the same diagram.
fn same(a: &str, b: &str) {
    assert_same(clean(a).diagram(), clean(b).diagram(), a);
}

fn ids(d: &Diagram) -> Vec<&str> {
    d.nodes.iter().map(|n| n.id.as_str()).collect()
}

#[test]
fn group_braces_on_their_own_lines() {
    let compact = "api\ndb\ngroup backend \"Backend\" { api db } { kind: boundary }\n";
    same(
        "api\ndb\ngroup backend \"Backend\"\n{\n  api\n  db\n}\n{ kind: boundary }\n",
        compact,
    );
    same("api\ndb\ngroup backend \"Backend\" {\n  api,\n  db,\n} {\n  kind: boundary\n}\n", compact);
    same("api\ndb\ngroup backend \"Backend\"\n\n{ api, db }\n\n{\n  kind: boundary,\n}\n", compact);
    // Members split any way: commas, newlines, both, trailing commas.
    same("api\ndb\ngroup backend \"Backend\" { api,\n\n db, } { kind: boundary }\n", compact);
}

#[test]
fn other_blocks_open_on_the_next_line() {
    same("diagram \"T\"\n{\n  routing: orthogonal\n}\na\n", "diagram \"T\" { routing: orthogonal }\na\n");
    same("a \"A\"\n{\n  fill: #fff\n  stroke: red\n}\n", "a \"A\" { fill: #fff, stroke: red }\n");
    same("a -> b \"x\"\n{\n  line: dashed\n}\n", "a -> b \"x\" { line: dashed }\n");
    same("style hot\n{\n  fill: red\n}\na .hot\n", "style hot { fill: red }\na .hot\n");
    same("a\nlayout\n{\n  a 1 2\n}\n", "a\nlayout {\n  a 1 2\n}\n");
    same(
        "a\nanimate\n{\n  step \"One\" 2s\n  {\n    show a\n  }\n}\n",
        "a\nanimate {\n  step \"One\" 2s {\n    show a\n  }\n}\n",
    );
}

#[test]
fn comments_in_every_gap() {
    let src = "# top\ndiagram \"T\" # after title\n# between\na \"A\" # trailing\n{ # opening\n  # inside\n  fill: red # after value\n  // slashes too\n}\ngroup g # header\n{\n  # first\n  a # member\n  # last\n}\nlayout { # open\n  # in layout\n  a 1 2 # entry\n}\n";
    let d = clean(src);
    let m = d.diagram();
    assert_eq!(m.node("a").and_then(|n| m.node_prop(n, "fill")), Some(&Value::Ident("red".into())));
    assert_eq!(m.groups[0].members, ["a"]);
    assert_eq!(m.layout["a"].pos, Point::new(1.0, 2.0));
}

#[test]
fn encodings_and_whitespace() {
    // A byte order mark (Windows editors write one) is not part of the
    // first word.
    same("\u{feff}diagram \"T\"\na\n", "diagram \"T\"\na\n");
    same("diagram \"T\"\r\na -> b\r\n\r\nlayout {\r\n  a 1 2\r\n}\r\n", "diagram \"T\"\na -> b\nlayout {\n  a 1 2\n}\n");
    same("a\t->\tb\t\"x\"\n", "a -> b \"x\"\n");
    // A pasted non-breaking space separates words like a space does.
    same("a\u{a0}->\u{a0}b\n", "a -> b\n");
    same("a -> b", "a -> b\n");
    for blank in ["", "\n", "   \n\t\n", "# only a comment", "// note\n# note\n", "\u{feff}"] {
        let d = clean(blank);
        assert!(d.diagram().nodes.is_empty(), "{blank:?}");
    }
}

#[test]
fn strings_and_unicode() {
    let d = clean("a \"say \\\"hi\\\" \\\\ ok\"\nb \"line\\nbreak\"\nc \"caf\u{e9} \u{1f600}\"\n\u{e9}t\u{e9} -> na\u{ef}ve\n");
    let m = d.diagram();
    assert_eq!(m.node("a").unwrap().label.as_deref(), Some("say \"hi\" \\ ok"));
    assert_eq!(m.node("b").unwrap().label.as_deref(), Some("line\nbreak"));
    assert_eq!(m.node("c").unwrap().label.as_deref(), Some("caf\u{e9} \u{1f600}"));
    assert!(m.edge("\u{e9}t\u{e9}->na\u{ef}ve").is_some());
    // An escape before a multi-byte character keeps the character whole.
    let d = Document::parse("a \"x\\\u{e9}y\"\n");
    assert_eq!(d.diagram().node("a").unwrap().label.as_deref(), Some("x\u{e9}y"));
}

#[test]
fn keywords_work_as_ids_where_unambiguous() {
    let d = clean("group: rect \"G\"\nlayout: rect\nstep -> group\ndiagram -- use\n");
    assert_eq!(ids(d.diagram()), ["group", "layout", "step", "diagram", "use"]);
}

#[test]
fn unfinished_text_never_panics_and_keeps_what_came_before() {
    let cases = [
        "a\ngroup g {",
        "a\ngroup g {\n  a\n",
        "a\ngroup g",
        "a\na { fill: red",
        "a\na { fill:",
        "a\na { fill",
        "a\na {",
        "a\na \"open string",
        "a\na -> ",
        "a\na ->",
        "a\nlayout {",
        "a\nlayout {\n  a 1",
        "a\nlayout {\n  a 1 2 300x",
        "a\nlayout {\n  a -> ",
        "a\nanimate {",
        "a\nanimate {\n  step \"x\" 2",
        "a\nanimate {\n  step \"x\" 2s {\n    show",
        "a\nanimate {\n  step {\n    move a 1",
        "a\ndiagram \"T\" {",
        "a\nuse",
        "a\nstyle s {",
        "a\na: ",
        "a\na.",
        "a\n{",
        "a\n}",
        "a\n]",
        "a\nx [ 1, 2",
        "a\nb { list: [1, 2",
        "a\nb { list: [",
    ];
    for src in cases {
        let d = Document::parse(src);
        assert!(d.diagram().node("a").is_some(), "lost `a` in {src:?}");
        assert_eq!(d.source(), src);
    }
}

#[test]
fn a_bad_line_inside_a_block_does_not_spill_its_members() {
    // The broken group is skipped whole; its members do not become shapes.
    let d = Document::parse("a\ngroup g {\n  b\n  \"oops\"\n  c\n}\nd\n");
    assert_eq!(ids(d.diagram()), ["a", "d"]);
    assert!(!d.diags().is_empty());
    let d = Document::parse("a { fill: red\n  ??? \n  stroke: blue\n}\nd\n");
    assert_eq!(ids(d.diagram()), ["d"]);
}

#[test]
fn an_unclosed_block_does_not_swallow_the_file() {
    // While typing `group g {` the rest of the file still shows.
    let d = Document::parse("group g {\na\nb -> c\n");
    assert!(d.diagram().node("a").is_some());
    assert!(d.diagram().edge("b->c").is_some());
    assert!(!d.diags().is_empty());
}

/// A document in the brace-on-its-own-line style, with comments inside
/// every block, for edits to work on.
const ALLMAN: &str = "# keep: top\ndiagram \"T\"\n{\n  routing: orthogonal # keep: diagram prop\n}\n\napi \"API\"\n{\n  # keep: in node\n  fill: #eef\n}\ndb\ncache\n\ngroup backend \"Backend\"\n{\n  # keep: before members\n  api\n  db # keep: after db\n  # keep: after members\n}\n{\n  kind: boundary\n}\n\napi -> db \"q\"\n{\n  line: dashed\n}\n\nlayout\n{\n  # keep: in layout\n  api 10 20\n}\n";

fn comments(src: &str) -> Vec<&str> {
    src.lines().filter_map(|l| l.find("# keep:").map(|i| &l[i..])).collect()
}

#[test]
fn edits_keep_brace_style_and_comments() {
    let ops = vec![
        Op::SetLabel { id: "api".into(), label: Some("Gateway".into()) },
        Op::SetLabel { id: "backend".into(), label: None },
        Op::SetLabel { id: "backend".into(), label: Some("Servers".into()) },
        Op::SetProp { id: "api".into(), key: "stroke".into(), value: Some(Value::Ident("red".into())) },
        Op::SetProp { id: "api".into(), key: "fill".into(), value: None },
        Op::SetProp { id: "backend".into(), key: "look".into(), value: Some(Value::Ident("solid".into())) },
        Op::SetProp { id: "api->db".into(), key: "line".into(), value: None },
        Op::SetProp { id: "db".into(), key: "fill".into(), value: Some(Value::Color("#abc".into())) },
        Op::SetDiagramProp { key: "flow".into(), value: Some(Value::Ident("down".into())) },
        Op::SetTitle { title: Some("New".into()) },
        Op::SetMembers { group: "backend".into(), members: vec!["api".into(), "db".into(), "cache".into()] },
        Op::SetMembers { group: "backend".into(), members: vec!["db".into()] },
        Op::SetMembers { group: "backend".into(), members: vec![] },
        Op::SetPlacement { id: "db".into(), placement: Some(Placement { pos: Point::new(5.0, 6.0), size: None }) },
        Op::SetPlacement { id: "api".into(), placement: None },
        Op::SetWaypoints { id: "api->db".into(), points: vec![Point::new(1.0, 1.0)] },
        Op::SetStencil { id: "db".into(), stencil: Some("core.cylinder".into()) },
        Op::SetArrow { id: "api->db".into(), arrow: Arrow::Both },
        Op::AddNode { node: Node::new("queue"), index: 1 },
        Op::AddEdge { edge: graphing_model::Edge { id: "db->cache".into(), from: "db".into(), to: "cache".into(), ..Default::default() }, index: 0 },
    ];
    for op in ops {
        let mut d = clean(ALLMAN);
        check(&mut d, op.clone());
        let mut d = clean(ALLMAN);
        d.apply(&op).expect("applies");
        assert!(d.diags().is_empty(), "{op:?}\n{}", d.source());
        assert_eq!(comments(d.source()), comments(ALLMAN), "{op:?} lost a comment:\n{}", d.source());
    }
}

#[test]
fn removals_keep_the_comments_around_them() {
    for op in [Op::RemoveNode { id: "cache".into() }, Op::RemoveNode { id: "db".into() }, Op::RemoveEdge { id: "api->db".into() }] {
        let mut d = clean(ALLMAN);
        check(&mut d, op.clone());
        let mut d = clean(ALLMAN);
        d.apply(&op).expect("applies");
        assert_eq!(comments(d.source()), comments(ALLMAN), "{op:?} lost a comment:\n{}", d.source());
    }
    // A removed group takes its own lines, and only those.
    let mut d = clean(ALLMAN);
    d.apply(&Op::RemoveGroup { id: "backend".into() }).unwrap();
    let kept: Vec<&str> = comments(ALLMAN).into_iter().filter(|c| !c.contains("members") && !c.contains("after db")).collect();
    assert_eq!(comments(d.source()), kept, "\n{}", d.source());
}

#[test]
fn member_edits_follow_the_bodys_style() {
    let mut d = clean("a\nb\nc\ngroup g {\n  a\n  b\n}\n");
    d.apply(&Op::SetMembers { group: "g".into(), members: vec!["a".into(), "b".into(), "c".into()] }).unwrap();
    assert_eq!(d.source(), "a\nb\nc\ngroup g {\n  a\n  b\n  c\n}\n");
    d.apply(&Op::SetMembers { group: "g".into(), members: vec!["a".into(), "c".into()] }).unwrap();
    assert_eq!(d.source(), "a\nb\nc\ngroup g {\n  a\n  c\n}\n");
    let mut d = clean("a\nb\nc\ngroup g { a, b, c }\n");
    d.apply(&Op::SetMembers { group: "g".into(), members: vec!["b".into(), "c".into()] }).unwrap();
    assert_eq!(d.source(), "a\nb\nc\ngroup g { b, c }\n");
    d.apply(&Op::SetMembers { group: "g".into(), members: vec!["b".into(), "c".into(), "a".into()] }).unwrap();
    assert_eq!(d.source(), "a\nb\nc\ngroup g { b, c, a }\n");
    d.apply(&Op::SetMembers { group: "g".into(), members: vec!["a".into(), "b".into()] }).unwrap();
    assert_eq!(d.source(), "a\nb\nc\ngroup g { a, b }\n");
    d.apply(&Op::SetMembers { group: "g".into(), members: vec![] }).unwrap();
    d.apply(&Op::SetMembers { group: "g".into(), members: vec!["c".into()] }).unwrap();
    assert_eq!(d.source(), "a\nb\nc\ngroup g { c }\n");
}

#[test]
fn parallel_unnamed_edges_keep_their_bends_through_edits() {
    let src = "a -> b \"one\"\na -> b \"two\"\na -> b \"three\"\nlayout {\n  a -> b via 1 1\n  \"a->b#2\" via 2 2\n  \"a->b#3\" via 3 3\n}\n";
    let bend = |d: &Document, key: &str| d.diagram().waypoints.get(key).map(|p| p[0].x);
    let label = |d: &Document, key: &str| d.diagram().edge(key).and_then(|e| e.label.clone());
    let mut d = clean(src);
    check(&mut d, Op::RemoveEdge { id: "a->b".into() });
    // Removing the first: the others move down a key, bends with them.
    let undo = d.apply(&Op::RemoveEdge { id: "a->b".into() }).unwrap();
    assert!(d.diags().is_empty(), "{}", d.source());
    assert_eq!((label(&d, "a->b").as_deref(), bend(&d, "a->b")), (Some("two"), Some(2.0)));
    assert_eq!((label(&d, "a->b#2").as_deref(), bend(&d, "a->b#2")), (Some("three"), Some(3.0)));
    // Undo puts the first back in front, and every bend where it was.
    d.apply(&undo).unwrap();
    assert_same(d.diagram(), clean(src).diagram(), d.source());
    // A new one added in front takes the first key.
    let edge = graphing_model::Edge { id: "a->b".into(), from: "a".into(), to: "b".into(), label: Some("zero".into()), ..Default::default() };
    check(&mut d, Op::AddEdge { edge: edge.clone(), index: 0 });
    d.apply(&Op::AddEdge { edge, index: 0 }).unwrap();
    assert_eq!(label(&d, "a->b").as_deref(), Some("zero"));
    assert_eq!((label(&d, "a->b#2").as_deref(), bend(&d, "a->b#2")), (Some("one"), Some(1.0)));
}

#[test]
fn removing_a_shape_takes_it_out_of_the_animation() {
    let src = "a\nb\na -> b\nanimate {\n  step \"One\" {\n    show a, b\n    flow a -> b\n    move a 10 20\n  }\n  step \"Two\" { show a }\n}\n";
    check(&mut clean(src), Op::RemoveNode { id: "a".into() });
    let mut d = clean(src);
    d.apply(&Op::RemoveNode { id: "a".into() }).unwrap();
    assert!(d.diags().is_empty(), "{:?}\n{}", d.diags(), d.source());
    assert_eq!(d.source(), "b\nanimate {\n  step \"One\" {\n    show b\n  }\n  step \"Two\" { }\n}\n");
}

#[test]
fn hops_of_a_chain_change_on_their_own() {
    let src = "a -> b -> c \"x\" { line: dashed }\n";
    for op in [
        Op::SetLabel { id: "b->c".into(), label: Some("y".into()) },
        Op::SetProp { id: "a->b".into(), key: "line".into(), value: None },
        Op::SetEdgePorts { id: "b->c".into(), from_port: Some("out".into()), to_port: None },
    ] {
        check(&mut clean(src), op);
    }
    let mut d = clean(src);
    d.apply(&Op::SetLabel { id: "b->c".into(), label: Some("y".into()) }).unwrap();
    assert_eq!(d.source(), "a -> b \"x\" { line: dashed }\nb -> c \"y\" { line: dashed }\n");
}


#[test]
fn pack_short_names_read_as_the_pack_and_edits_write_them_back() {
    let src = "use c4 as arch, sysml as s, core\narch.person \"User\"\napi: arch.container { kind: arch.service }\nblk: s.block\napi -> blk { kind: arch.uses }\n";
    let d = clean(src);
    let m = d.diagram();
    assert_eq!(m.packs, ["c4", "sysml", "core"]);
    assert_eq!(m.aliases.get("arch").map(String::as_str), Some("c4"));
    // The model holds pack ids, whatever the text calls them.
    assert_eq!(m.node("api").unwrap().stencil.as_deref(), Some("c4.container"));
    assert_eq!(m.node("blk").unwrap().stencil.as_deref(), Some("sysml.block"));
    assert_eq!(m.node("api").and_then(|n| m.node_prop(n, "kind")), Some(&Value::Ident("c4.service".into())));
    assert_eq!(m.edge("api->blk").and_then(|e| m.edge_prop(e, "kind")), Some(&Value::Ident("c4.uses".into())));
    // Names an edit writes come out in the file's own short form.
    let edits = [
        (Op::SetStencil { id: "blk".into(), stencil: Some("c4.system".into()) }, "blk: arch.system"),
        (Op::SetStencil { id: "blk".into(), stencil: Some("uml.class".into()) }, "blk: uml.class"),
        (Op::SetProp { id: "api->blk".into(), key: "kind".into(), value: Some(Value::Ident("sysml.flow".into())) }, "kind: s.flow"),
        (Op::AddNode { node: Node { stencil: Some("c4.database".into()), ..Node::new("db") }, index: 9 }, "db: arch.database"),
    ];
    for (op, written) in edits {
        check(&mut clean(src), op.clone());
        let mut d = clean(src);
        d.apply(&op).unwrap();
        assert!(d.source().contains(written), "{op:?}\n{}", d.source());
    }
}

#[test]
fn a_short_name_used_twice_is_reported() {
    let d = Document::parse("use c4 as x, sysml as x\nn: x.block\n");
    assert_eq!(d.diags().len(), 1, "{:?}", d.diags());
    // The first stays.
    assert_eq!(d.diagram().node("n").unwrap().stencil.as_deref(), Some("c4.block"));
    // Naming the same pack twice the same way is fine.
    clean("use c4 as x\nuse c4 as x\n");
}
