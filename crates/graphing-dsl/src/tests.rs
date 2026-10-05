use super::*;
use graphing_model::{Action, Arrow, Edge, Node, Op, Placement, Point, Size, Step, Value, Verb};

const SAMPLE: &str = r##"# comments survive every edit
diagram "Auth flow"
use core, flow

style service { fill: #e8eefc, stroke: "#3b5bdb" }

user: actor "User"
api:  flow.process "API Gateway" .service
db:   db "Postgres" { fill: #fff4e6 }   # trailing note
cache: "Redis"

group backend "Backend" { api db cache }

user -> api "login"
api -> db "lookup" { line: dashed }
api <-> cache
refresh: user -> api "refresh"

layout {
  user    40 180
  api     320 160 180x64
  db      620 120
  user -> api via 180 120, 260 120
}
"##;

fn doc() -> Document {
    let d = Document::parse(SAMPLE);
    assert!(d.diags().is_empty(), "{:?}", d.diags());
    d
}

/// Apply through the document and through the model; both must agree, and
/// the inverse must restore the original text's model.
fn check(doc: &mut Document, op: Op) {
    let before = doc.diagram().clone();
    let mut expect = before.clone();
    expect.apply(&op).expect("op applies to model");
    let inv = doc.apply(&op).expect("op applies to doc");
    assert!(doc.diags().is_empty(), "diags after {op:?}: {:?}\n{}", doc.diags(), doc.source());
    assert_same(doc.diagram(), &expect, doc.source());
    doc.apply(&inv).expect("inverse applies");
    assert_same(doc.diagram(), &before, doc.source());
}

/// Model equality, ignoring node order (implied nodes may move when text
/// gives them a line).
fn assert_same(a: &graphing_model::Diagram, b: &graphing_model::Diagram, src: &str) {
    let mut a = a.clone();
    let mut b = b.clone();
    a.nodes.sort_by(|x, y| x.id.cmp(&y.id));
    b.nodes.sort_by(|x, y| x.id.cmp(&y.id));
    assert_eq!(a.nodes, b.nodes, "\n{src}");
    assert_eq!(a.edges, b.edges, "\n{src}");
    assert_eq!(a.groups, b.groups, "\n{src}");
    assert_eq!(a.layout, b.layout, "\n{src}");
    assert_eq!(a.waypoints, b.waypoints, "\n{src}");
    assert_eq!(a.steps, b.steps, "\n{src}");
}

#[test]
fn parses_sample() {
    let d = doc();
    let m = d.diagram();
    assert_eq!(m.title.as_deref(), Some("Auth flow"));
    assert_eq!(m.packs, ["core", "flow"]);
    assert_eq!(m.nodes.len(), 4);
    let api = m.node("api").unwrap();
    assert_eq!(api.stencil.as_deref(), Some("flow.process"));
    assert_eq!(api.classes, ["service"]);
    assert_eq!(m.node_prop(api, "fill"), Some(&Value::Color("#e8eefc".into())));
    assert_eq!(m.edges.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["user->api", "api->db", "api->cache", "refresh"]);
    assert_eq!(m.edge("api->cache").unwrap().arrow, Arrow::Both);
    assert_eq!(m.groups[0].members, ["api", "db", "cache"]);
    assert_eq!(m.layout["api"].size, Some(Size::new(180.0, 64.0)));
    assert_eq!(m.waypoints["user->api"].len(), 2);
}

#[test]
fn chains_and_implied_nodes() {
    let d = Document::parse("a -> b -> c \"x\"\nc -- a\na -> b\n");
    let m = d.diagram();
    assert_eq!(m.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    let ids: Vec<_> = m.edges.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["a->b", "b->c", "c->a", "a->b#2"]);
    assert_eq!(m.edge("b->c").unwrap().label.as_deref(), Some("x"));
}

#[test]
fn bad_lines_are_kept_and_reported() {
    let src = "a: rect\n%%% nope\nb -> \nlayout {\n  a 1 2\n  ??\n}\n";
    let d = Document::parse(src);
    assert_eq!(d.diags().len(), 3, "{:?}", d.diags());
    assert_eq!(d.diagram().layout["a"].pos, Point::new(1.0, 2.0));
    assert_eq!(d.source(), src);
}

#[test]
fn move_only_touches_layout_numbers() {
    let mut d = doc();
    let p = Placement { pos: Point::new(400.0, 200.5), size: Some(Size::new(180.0, 64.0)) };
    d.apply(&Op::SetPlacement { id: "api".into(), placement: Some(p) }).unwrap();
    let expected = SAMPLE.replace("api     320 160 180x64", "api     400 200.5 180x64");
    assert_eq!(d.source(), expected);
}

#[test]
fn place_new_entry_and_create_block() {
    let mut d = doc();
    let p = Placement { pos: Point::new(1.0, 2.0), size: None };
    d.apply(&Op::SetPlacement { id: "cache".into(), placement: Some(p) }).unwrap();
    assert!(d.source().contains("  user -> api via 180 120, 260 120\n  cache 1 2\n}"));

    let mut d = Document::parse("a\nb");
    d.apply(&Op::SetPlacement { id: "b".into(), placement: Some(p) }).unwrap();
    assert_eq!(d.source(), "a\nb\n\nlayout {\n  b 1 2\n}\n");
    d.apply(&Op::SetPlacement { id: "a".into(), placement: Some(p) }).unwrap();
    assert_eq!(d.source(), "a\nb\n\nlayout {\n  b 1 2\n  a 1 2\n}\n");
}

#[test]
fn ops_round_trip() {
    let mut d = doc();
    let place = |x, y| Some(Placement { pos: Point::new(x, y), size: None });
    let ops = vec![
        Op::SetPlacement { id: "db".into(), placement: place(1.0, 2.0) },
        Op::SetPlacement { id: "cache".into(), placement: place(5.0, 5.0) },
        Op::SetPlacement { id: "db".into(), placement: None },
        Op::SetPlacement { id: "backend".into(), placement: Some(Placement { pos: Point::new(0.0, 0.0), size: Some(Size::new(10.0, 10.0)) }) },
        Op::SetLabel { id: "api".into(), label: Some("Gate \"way\"".into()) },
        Op::SetLabel { id: "api".into(), label: None },
        Op::SetLabel { id: "api->cache".into(), label: Some("hit".into()) },
        Op::SetLabel { id: "backend".into(), label: None },
        Op::SetProp { id: "db".into(), key: "fill".into(), value: Some(Value::Color("#000".into())) },
        Op::SetProp { id: "db".into(), key: "fill".into(), value: None },
        Op::SetProp { id: "db".into(), key: "stroke".into(), value: Some(Value::Num(2.0)) },
        Op::SetProp { id: "user".into(), key: "w".into(), value: Some(Value::Num(3.0)) },
        Op::SetProp { id: "api->db".into(), key: "line".into(), value: None },
        Op::SetWaypoints { id: "user->api".into(), points: vec![Point::new(9.0, 9.0)] },
        Op::SetWaypoints { id: "user->api".into(), points: vec![] },
        Op::SetWaypoints { id: "refresh".into(), points: vec![Point::new(1.0, 1.0)] },
        Op::SetWaypoints { id: "api->db".into(), points: vec![Point::new(1.0, 1.0)] },
        Op::SetMembers { group: "backend".into(), members: vec!["db".into()] },
        Op::SetStencil { id: "api".into(), stencil: Some("core.diamond".into()) },
        Op::SetStencil { id: "api".into(), stencil: None },
        Op::SetStencil { id: "cache".into(), stencil: Some("db".into()) },
        Op::SetArrow { id: "api->cache".into(), arrow: Arrow::None },
        Op::SetArrow { id: "refresh".into(), arrow: Arrow::Back },
        Op::AddNode { node: Node { stencil: Some("core.rect".into()), label: Some("Q".into()), ..Node::new("queue") }, index: 2 },
        Op::AddNode { node: Node::new("first"), index: 0 },
        Op::AddEdge {
            edge: Edge {
                id: "db->user".into(),
                from: "db".into(),
                to: "user".into(),
                arrow: Arrow::Back,
                label: Some("x".into()),
                classes: vec!["service".into()],
                ..Default::default()
            },
            index: 99,
        },
        Op::RemoveEdge { id: "user->api".into() },
        Op::RemoveEdge { id: "refresh".into() },
        Op::RemoveNode { id: "api".into() },
        Op::RemoveNode { id: "user".into() },
        Op::Batch(vec![
            Op::SetLabel { id: "db".into(), label: Some("DB".into()) },
            Op::SetPlacement { id: "cache".into(), placement: place(3.0, 3.0) },
        ]),
    ];
    for op in ops {
        check(&mut d, op);
    }
    // Every round trip was undone through text alone; the file is unchanged
    // apart from whitespace the edits normalised.
    assert_same(d.diagram(), Document::parse(SAMPLE).diagram(), d.source());
    assert!(d.source().contains("# trailing note"));
}

#[test]
fn chain_edit_splits_and_keeps_implied_nodes() {
    let mut d = Document::parse("a -> b -> c -> d \"l\" { line: dashed }\n");
    check(&mut d, Op::RemoveEdge { id: "b->c".into() });
    d.apply(&Op::RemoveEdge { id: "b->c".into() }).unwrap();
    assert_eq!(d.source(), "b\nc\na -> b \"l\" { line: dashed }\nc -> d \"l\" { line: dashed }\n");
    check(&mut d, Op::RemoveNode { id: "c".into() });
}

#[test]
fn stencil_on_lenient_and_implied_nodes() {
    let mut d = Document::parse("a \"A\"\na -> b\n");
    check(&mut d, Op::SetStencil { id: "a".into(), stencil: Some("ellipse".into()) });
    check(&mut d, Op::SetStencil { id: "b".into(), stencil: Some("db".into()) });
    d.apply(&Op::SetStencil { id: "a".into(), stencil: Some("ellipse".into()) }).unwrap();
    assert_eq!(d.source(), "a: ellipse \"A\"\nb\na -> b\n");
}

#[test]
fn ports_lists_and_diagram_props() {
    let src = r#"diagram "HIL" { kind: ibd, context: block, view: Architecture }
use sysml

uut: sysml.part "FlightArticle" {
  parts: ["obc : OnboardComputer", "adcs : ADCSProcessor"]
  stereotype: block
}
fe: sysml.part "IOFrontEnd"
uut.busPort -- fe.busPort "SensorFrame, ActuatorCommand" { kind: flow, items: [SensorFrame, ActuatorCommand] }
uut.pwrPort -- phys.pwrPort
named: fe.dmaPort -> sim.dmaPort
layout {
  uut.busPort -- fe.busPort via 10 20
}
"#;
    let mut d = Document::parse(src);
    assert!(d.diags().is_empty(), "{:?}", d.diags());
    let m = d.diagram();
    assert_eq!(m.prop("kind"), Some(&Value::Ident("ibd".into())));
    let uut = m.node("uut").unwrap();
    assert_eq!(uut.stencil.as_deref(), Some("sysml.part"));
    let parts = m.node_prop(uut, "parts").unwrap().as_list().unwrap();
    assert_eq!(parts.len(), 2);
    let e = m.edge("uut->fe").unwrap();
    assert_eq!((e.from_port.as_deref(), e.to_port.as_deref()), (Some("busPort"), Some("busPort")));
    assert_eq!(m.edge_prop(e, "kind"), Some(&Value::Ident("flow".into())));
    assert!(m.node("phys").is_some() && m.node("sim").is_some());
    assert_eq!(m.edge("named").unwrap().to_port.as_deref(), Some("dmaPort"));
    assert_eq!(m.waypoints["uut->fe"].len(), 1);
    // Edits keep ports and lists intact.
    check(&mut d, Op::RemoveEdge { id: "uut->phys".into() });
    check(&mut d, Op::SetProp { id: "uut".into(), key: "parts".into(), value: Some(Value::List(vec![Value::Str("x : Y".into())])) });
    let edge = Edge { id: "fe->uut".into(), from: "fe".into(), to: "uut".into(), from_port: Some("a".into()), to_port: Some("b".into()), ..Default::default() };
    check(&mut d, Op::AddEdge { edge, index: 9 });
    check(&mut d, Op::RemoveNode { id: "fe".into() });
}

#[test]
fn diagram_title_and_props() {
    let mut d = Document::parse("# note\na -> b\n");
    check(&mut d, Op::SetTitle { title: Some("Flow".into()) });
    check(&mut d, Op::SetDiagramProp { key: "kind".into(), value: Some(Value::Ident("ibd".into())) });
    d.apply(&Op::SetDiagramProp { key: "kind".into(), value: Some(Value::Ident("ibd".into())) }).unwrap();
    assert_eq!(d.source(), "# note\ndiagram { kind: ibd }\na -> b\n");
    check(&mut d, Op::SetTitle { title: Some("Flow".into()) });
    d.apply(&Op::SetTitle { title: Some("Flow".into()) }).unwrap();
    check(&mut d, Op::SetDiagramProp { key: "view".into(), value: Some(Value::Ident("Main".into())) });
    check(&mut d, Op::SetDiagramProp { key: "kind".into(), value: None });
    assert!(d.source().starts_with("# note\ndiagram \"Flow\" { kind: ibd }\n"), "{}", d.source());
}

#[test]
fn ports_and_groups_round_trip() {
    let mut d = Document::parse("a\nb\nc\ngroup g \"G\" { a b }\na -> b\nlayout {\n  g 0 0 300x200\n}\n");
    check(&mut d, Op::SetEdgePorts { id: "a->b".into(), from_port: Some("out".into()), to_port: Some("in".into()) });
    d.apply(&Op::SetEdgePorts { id: "a->b".into(), from_port: Some("out".into()), to_port: None }).unwrap();
    assert!(d.source().contains("a.out -> b\n"), "{}", d.source());
    check(&mut d, Op::SetEdgePorts { id: "a->b".into(), from_port: None, to_port: Some("x".into()) });
    let g2 = graphing_model::Group { id: "h".into(), label: Some("H".into()), members: vec!["c".into(), "g".into()], props: vec![] };
    check(&mut d, Op::AddGroup { group: g2.clone(), index: 9 });
    d.apply(&Op::AddGroup { group: g2, index: 9 }).unwrap();
    assert!(d.source().contains("group h \"H\" { c g }"), "{}", d.source());
    check(&mut d, Op::RemoveGroup { id: "g".into() });
    d.apply(&Op::RemoveGroup { id: "g".into() }).unwrap();
    assert!(!d.source().contains("group g") && !d.source().contains("g 0 0"), "{}", d.source());
    assert_eq!(d.diagram().group("h").unwrap().members, ["c"]);
}

#[test]
fn colors_vs_comments() {
    let d = Document::parse("a { fill: #abc }  # add a note\nb # bad\n#abc\n");
    assert!(d.diags().is_empty(), "{:?}", d.diags());
    assert_eq!(d.diagram().nodes.len(), 2);
    assert_eq!(d.diagram().node("a").unwrap().props[0].1, Value::Color("#abc".into()));
}

#[test]
fn num_format() {
    assert_eq!(fmt_num(10.0), "10");
    assert_eq!(fmt_num(-3.5), "-3.5");
    assert_eq!(fmt_num(1.256), "1.26");
    assert_eq!(fmt_num(0.1 + 0.2), "0.3");
}

#[test]
fn group_style_props_round_trip() {
    let mut d = Document::parse("a\ngroup g \"G\" { a }\n");
    check(&mut d, Op::SetProp { id: "g".into(), key: "fill".into(), value: Some(Value::Color("#e7f5ff".into())) });
    d.apply(&Op::SetProp { id: "g".into(), key: "fill".into(), value: Some(Value::Color("#e7f5ff".into())) }).unwrap();
    assert!(d.source().contains("group g \"G\" { a } { fill: #e7f5ff }"), "{}", d.source());
    check(&mut d, Op::SetProp { id: "g".into(), key: "line".into(), value: Some(Value::Ident("solid".into())) });
    check(&mut d, Op::SetProp { id: "g".into(), key: "fill".into(), value: None });
}

mod lang_tests {
    use crate::lang::{Owner, Role, Want, complete_at, folds, roles};

    fn role_of(src: &str, text: &str) -> Role {
        let at = src.find(text).unwrap();
        roles(src).into_iter().find(|(s, _)| s.start == at).map(|(_, r)| r).unwrap()
    }

    #[test]
    fn roles_cover_statements() {
        let src = "# top\ndiagram \"T\" { kind: ibd }\nuse sysml\napi: sysml.part \"API\" .hot { fill: #ff0000 }\napi.out -> db \"q\" { kind: flow }\ngroup g \"G\" { api db } { fill: #eee }\nlayout {\n  api 0 0 120x40\n}\n";
        assert_eq!(role_of(src, "# top"), Role::Comment);
        assert_eq!(role_of(src, "diagram"), Role::Keyword);
        assert_eq!(role_of(src, "kind: ibd"), Role::Key(Owner::Diagram));
        assert_eq!(role_of(src, "ibd"), Role::Value(Owner::Diagram, "kind".into()));
        assert_eq!(role_of(src, "sysml\n"), Role::Pack);
        assert_eq!(role_of(src, "api:"), Role::Def);
        assert_eq!(role_of(src, "sysml.part"), Role::Stencil);
        assert_eq!(role_of(src, "hot"), Role::Class);
        assert_eq!(role_of(src, "fill"), Role::Key(Owner::Node(Some("sysml.part".into()))));
        assert_eq!(role_of(src, "out ->"), Role::Port("api".into()));
        assert_eq!(role_of(src, "db \"q\""), Role::Ref);
        assert_eq!(role_of(src, "kind: flow"), Role::Key(Owner::Edge));
        assert_eq!(role_of(src, "g \"G\""), Role::Def);
        assert_eq!(role_of(src, "api db }"), Role::Ref);
        assert_eq!(role_of(src, "fill: #eee"), Role::Key(Owner::Group));
        assert_eq!(role_of(src, "api 0 0"), Role::Ref);
        // Ordered and non-overlapping.
        let r = roles(src);
        assert!(r.windows(2).all(|w| w[0].0.end <= w[1].0.start), "{r:?}");
    }

    #[test]
    fn completion_contexts() {
        let want = |src: &str| complete_at(src, src.len()).map(|c| (c.want, c.prefix));
        assert_eq!(want("api: "), Some((Want::Stencil, String::new())));
        assert_eq!(want("api: sysml.pa"), Some((Want::Stencil, "sysml.pa".into())));
        assert_eq!(want("a -> "), Some((Want::Ref, String::new())));
        assert_eq!(want("a -> b"), Some((Want::Ref, "b".into())));
        assert_eq!(want("a -> uut."), Some((Want::Port("uut".into()), String::new())));
        assert_eq!(want("a -> uut.bu"), Some((Want::Port("uut".into()), "bu".into())));
        assert_eq!(want("a: rect { fi"), Some((Want::Key(Owner::Node(Some("rect".into()))), "fi".into())));
        assert_eq!(want("a -> b { kind: "), Some((Want::Value(Owner::Edge, "kind".into()), String::new())));
        assert_eq!(want("diagram { kind: "), Some((Want::Value(Owner::Diagram, "kind".into()), String::new())));
        assert_eq!(want("x\nla"), Some((Want::Statement, "la".into())));
        assert_eq!(want("use sy"), Some((Want::Pack, "sy".into())));
        assert_eq!(want("a: rect \"lab"), None);
        assert_eq!(want("# comment wo"), None);
        assert_eq!(want("a: rect \"A\" ."), Some((Want::Class, String::new())));
        assert_eq!(want("group g { a, "), Some((Want::Ref, String::new())));
    }

    #[test]
    fn folds_multiline_blocks() {
        assert_eq!(folds("a { x: 1 }\nlayout {\n  a 0 0\n}\n"), vec![(1, 3)]);
    }
}

const ANIMATED: &str = r#"diagram "Tour"
a
b
c
a -> b
b -> c

# the walkthrough
animate {
  step "Start" 1.5s {
    show a
    focus a
  }
  step "Then" 500ms {
    show b, c
    flow a -> b, b -> c   # both lines
  }
}
"#;

fn step(title: &str, actions: &[(Verb, &[&str])]) -> Step {
    Step {
        title: Some(title.into()),
        seconds: None,
        ease: None,
        moves: Vec::new(),
        actions: actions.iter().map(|(v, t)| Action { verb: *v, targets: t.iter().map(|s| s.to_string()).collect() }).collect(),
    }
}

#[test]
fn animate_block_parses() {
    let d = Document::parse(ANIMATED);
    assert!(d.diags().is_empty(), "{:?}", d.diags());
    let steps = &d.diagram().steps;
    assert_eq!(steps.len(), 2);
    assert_eq!((steps[0].title.as_deref(), steps[0].seconds), (Some("Start"), Some(1.5)));
    assert_eq!(steps[1].seconds, Some(0.5));
    assert_eq!(steps[1].actions[0], Action { verb: Verb::Show, targets: vec!["b".into(), "c".into()] });
    assert_eq!(steps[1].actions[1].targets, ["a->b", "b->c"]);
}

#[test]
fn animate_problems_are_reported() {
    let d = Document::parse("a\nanimate {\n  step {\n    wiggle a\n    show nope\n  }\n}\n");
    let msgs: Vec<&str> = d.diags().iter().map(|x| x.message.as_str()).collect();
    assert!(msgs.iter().any(|m| m.contains("unknown action `wiggle`")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("`nope` is not in the diagram")), "{msgs:?}");
}

#[test]
fn step_ops_round_trip() {
    // A first step makes the block.
    let mut d = doc();
    check(&mut d, Op::AddStep { index: 0, step: step("Login", &[(Verb::Show, &["user", "api"]), (Verb::Flow, &["user->api"])]) });
    d.apply(&Op::AddStep { index: 0, step: step("Login", &[(Verb::Show, &["user"])]) }).unwrap();
    assert!(d.source().contains("animate {\n  step \"Login\" {\n    show user\n  }\n}\n"), "{}", d.source());

    let mut d = Document::parse(ANIMATED);
    check(&mut d, Op::AddStep { index: 0, step: step("Before", &[(Verb::Highlight, &["c"])]) });
    check(&mut d, Op::AddStep { index: 1, step: step("Middle", &[]) });
    check(&mut d, Op::AddStep { index: 9, step: Step { seconds: Some(3.0), ..step("Last", &[(Verb::Hide, &["a"])]) } });
    check(&mut d, Op::SetStep { index: 1, step: step("Renamed", &[(Verb::Focus, &["b", "c"])]) });
    check(&mut d, Op::RemoveStep { index: 0 });
    check(&mut d, Op::RemoveStep { index: 1 });
    // Comments outside the touched step survive.
    let mut d = Document::parse(ANIMATED);
    d.apply(&Op::SetStep { index: 0, step: step("Start", &[]) }).unwrap();
    assert!(d.source().contains("# the walkthrough") && d.source().contains("# both lines"), "{}", d.source());
    assert!(d.source().contains("  step \"Start\" {}\n  step \"Then\" 500ms {"), "{}", d.source());
}

#[test]
fn steps_carry_ease_and_moves() {
    let src = "a\nb\nanimate {\n  step \"Slide\" 1s {\n    ease bounce\n    move a 300 120\n    show b\n  }\n}\n";
    let mut d = Document::parse(src);
    assert!(d.diags().is_empty(), "{:?}", d.diags());
    let st = &d.diagram().steps[0];
    assert_eq!(st.ease, Some(graphing_model::Ease::Bounce));
    assert_eq!(st.moves, [("a".to_string(), Point::new(300.0, 120.0))]);
    let mut changed = st.clone();
    changed.ease = Some(graphing_model::Ease::Linear);
    changed.moves.push(("b".into(), Point::new(-40.0, 10.5)));
    check(&mut d, Op::SetStep { index: 0, step: changed });
    let bad = Document::parse("a\nanimate {\n  step {\n    ease wobbly\n    move nope 1 2\n  }\n}\n");
    let msgs: Vec<&str> = bad.diags().iter().map(|x| x.message.as_str()).collect();
    assert!(msgs.iter().any(|m| m.contains("unknown ease")) && msgs.iter().any(|m| m.contains("not a shape or group to move")), "{msgs:?}");
}
