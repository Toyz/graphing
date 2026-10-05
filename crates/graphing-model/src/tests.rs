use super::*;

fn edge(id: &str, from: &str, to: &str) -> Edge {
    Edge {
        id: id.into(),
        from: from.into(),
        to: to.into(),
        ..Default::default()
    }
}

fn sample() -> Diagram {
    let mut d = Diagram { nodes: vec![Node::new("a"), Node::new("b"), Node::new("c")], ..Default::default() };
    d.edges = vec![edge("e1", "a", "b"), edge("e2", "b", "c"), edge("e3", "a", "c")];
    d.groups = vec![Group { id: "g".into(), label: None, members: vec!["a".into(), "b".into()], props: Vec::new() }];
    d.layout.insert("b".into(), Placement { pos: Point::new(10.0, 20.0), size: None });
    d.waypoints.insert("e1".into(), vec![Point::new(1.0, 2.0)]);
    d
}

#[test]
fn remove_node_undo_restores_everything() {
    let mut d = sample();
    let before = d.clone();
    let undo = d.apply(&Op::RemoveNode { id: "b".into() }).unwrap();
    assert!(d.node("b").is_none());
    assert_eq!(d.edges.len(), 1);
    assert_eq!(d.groups[0].members, vec!["a".to_string()]);
    assert!(d.waypoints.is_empty());
    d.apply(&undo).unwrap();
    assert_eq!(d, before);
}

#[test]
fn batch_is_atomic() {
    let mut d = sample();
    let before = d.clone();
    let op = Op::Batch(vec![
        Op::SetLabel { id: "a".into(), label: Some("A".into()) },
        Op::RemoveNode { id: "missing".into() },
    ]);
    assert!(d.apply(&op).is_none());
    assert_eq!(d, before);
}

#[test]
fn batch_undo_round_trips() {
    let mut d = sample();
    let before = d.clone();
    let op = Op::Batch(vec![
        Op::SetLabel { id: "a".into(), label: Some("A".into()) },
        Op::SetProp { id: "e2".into(), key: "line".into(), value: Some(Value::Ident("dashed".into())) },
        Op::SetPlacement { id: "a".into(), placement: Some(Placement { pos: Point::new(5.0, 5.0), size: None }) },
        Op::AddNode { node: Node::new("d"), index: 99 },
    ]);
    let undo = d.apply(&op).unwrap();
    assert_eq!(d.node("a").unwrap().text(), "A");
    assert_eq!(d.nodes.last().unwrap().id, "d");
    d.apply(&undo).unwrap();
    assert_eq!(d, before);
}

#[test]
fn duplicate_ids_rejected() {
    let mut d = sample();
    assert!(d.apply(&Op::AddNode { node: Node::new("a"), index: 0 }).is_none());
    assert!(d.apply(&Op::AddNode { node: Node::new("g"), index: 0 }).is_none());
}

#[test]
fn class_props_resolve_with_inline_priority() {
    let mut d = sample();
    d.styles.insert("s".into(), vec![("fill".into(), Value::Color("#111".into()))]);
    d.styles.insert("t".into(), vec![("fill".into(), Value::Color("#222".into()))]);
    let n = d.node_mut("a").unwrap();
    n.classes = vec!["s".into(), "t".into()];
    let n = d.node("a").unwrap().clone();
    assert_eq!(d.node_prop(&n, "fill"), Some(&Value::Color("#222".into())));
    let mut n2 = n.clone();
    n2.props.push(("fill".into(), Value::Color("#333".into())));
    assert_eq!(d.node_prop(&n2, "fill"), Some(&Value::Color("#333".into())));
}

#[test]
fn boundary_hits_edge_of_rect() {
    let r = Rect::new(0.0, 0.0, 100.0, 50.0);
    let p = r.boundary_toward(Point::new(200.0, 25.0));
    assert_eq!(p, Point::new(100.0, 25.0));
    let p = r.boundary_toward(Point::new(50.0, -100.0));
    assert_eq!(p, Point::new(50.0, 0.0));
}
