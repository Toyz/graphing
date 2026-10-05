//! Putting one diagram inside another: a dropped `.gph` inserted where it
//! lands, as a group of its own, through ordinary ops (one undo step).

use std::collections::HashSet;

use graphing_model::{Diagram, Edge, Group, Node, Op, Placement, Point, Value};

/// A prefix for `stem` that no id in `taken` starts with.
fn prefix_for(stem: &str, d: &Diagram) -> String {
    let base: String = stem.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let base = if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit() || c == '-') { format!("d_{base}") } else { base };
    let taken: HashSet<&str> = d.nodes.iter().map(|n| n.id.as_str()).chain(d.groups.iter().map(|g| g.id.as_str())).chain(d.edges.iter().map(|e| e.id.as_str())).collect();
    let clash = |p: &str| taken.iter().any(|t| *t == p || t.starts_with(&format!("{p}_")));
    if !clash(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}{n}")).find(|p| !clash(p)).expect("unbounded")
}

/// Ops adding everything in `other` to `target`: ids prefixed by the
/// file's name, inside one group titled after it, its top-left at `at`.
/// Styles become inline props, since classes name styles of the other file.
pub fn insert(target: &Diagram, other: &Diagram, stem: &str, at: Point) -> Op {
    let p = prefix_for(stem, target);
    let id = |raw: &str| format!("{p}_{raw}");
    // Lay out what the other file never placed, so it lands as it would show.
    let mut other = other.clone();
    for (nid, pos) in graphing_scene::auto_place(&other) {
        other.layout.insert(nid, Placement { pos, size: None });
    }
    let scene = graphing_scene::build(&other, &Default::default());
    let origin = scene.content_bounds().map_or(Point::default(), |r| r.origin);
    let shift = |q: Point| Point::new((q.x - origin.x + at.x).round(), (q.y - origin.y + at.y).round());

    let mut ops = Vec::new();
    let base = target.nodes.len();
    for (k, n) in other.nodes.iter().enumerate() {
        let mut props = n.props.clone();
        for class in &n.classes {
            for (key, v) in other.styles.get(class).into_iter().flatten() {
                if !props.iter().any(|(k2, _)| k2 == key) {
                    props.push((key.clone(), v.clone()));
                }
            }
        }
        ops.push(Op::AddNode { node: Node { id: id(&n.id), stencil: n.stencil.clone(), label: Some(n.text().to_string()), classes: Vec::new(), props }, index: base + k });
    }
    let base = target.edges.len();
    for (k, e) in other.edges.iter().enumerate() {
        // Unnamed edges get the key they will have again (`a->b`).
        let auto = e.id.starts_with(&format!("{}->{}", e.from, e.to));
        let eid = if auto { format!("{}->{}", id(&e.from), id(&e.to)) } else { id(&e.id) };
        let mut props = e.props.clone();
        for class in &e.classes {
            for (key, v) in other.styles.get(class).into_iter().flatten() {
                if !props.iter().any(|(k2, _)| k2 == key) {
                    props.push((key.clone(), v.clone()));
                }
            }
        }
        ops.push(Op::AddEdge {
            edge: Edge { id: eid, from: id(&e.from), to: id(&e.to), from_port: e.from_port.clone(), to_port: e.to_port.clone(), arrow: e.arrow, label: e.label.clone(), classes: Vec::new(), props },
            index: base + k,
        });
    }
    // Its own groups, then one around everything not already in a group.
    let mut base = target.groups.len();
    for g in &other.groups {
        ops.push(Op::AddGroup { group: Group { id: id(&g.id), label: g.label.clone(), members: g.members.iter().map(|m| id(m)).collect(), props: g.props.clone() }, index: base });
        base += 1;
    }
    let inner: HashSet<&str> = other.groups.iter().flat_map(|g| g.members.iter().map(String::as_str)).collect();
    let top: Vec<String> = other.nodes.iter().map(|n| n.id.as_str()).chain(other.groups.iter().map(|g| g.id.as_str())).filter(|m| !inner.contains(m)).map(id).collect();
    let title = other.title.clone().unwrap_or_else(|| stem.to_string());
    ops.push(Op::AddGroup { group: Group { id: p.clone(), label: Some(title), members: top, props: vec![("look".into(), Value::Ident("solid".into()))] }, index: base });
    for (k, pl) in &other.layout {
        if other.node(k).is_some() || (other.group(k).is_some() && pl.size.is_some()) {
            ops.push(Op::SetPlacement { id: id(k), placement: Some(Placement { pos: shift(pl.pos), size: pl.size }) });
        }
    }
    Op::Batch(ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphing_dsl::Document;

    #[test]
    fn a_diagram_goes_in_as_a_group_with_its_own_ids() {
        let mut host = Document::parse("api\napi -> db\n");
        let other = Document::parse("diagram \"Auth\"\nstyle hot { fill: #ffe3e3 }\napi: \"Login API\" .hot\nstore: db\napi -> store\ngroup inner { store }\nlayout {\n  api 0 0\n  store 300 0\n}\n");
        let op = insert(host.diagram(), other.diagram(), "auth", Point::new(500.0, 400.0));
        host.apply(&op).expect("applies");
        assert!(host.diags().is_empty(), "{:?}\n{}", host.diags(), host.source());
        let d = host.diagram();
        // Nothing of the host was touched; the visitor's ids are prefixed.
        assert!(d.node("api").is_some() && d.node("auth_api").is_some() && d.node("auth_store").is_some());
        assert!(d.edge("auth_api->auth_store").is_some());
        let outer = d.group("auth").expect("wrapped in a group");
        assert_eq!(outer.label.as_deref(), Some("Auth"));
        assert_eq!(outer.members, ["auth_api", "auth_inner"]);
        // Styles came along inline; positions moved to the drop point.
        let api = d.node("auth_api").unwrap();
        assert!(api.props.iter().any(|(k, v)| k == "fill" && v.text() == "#ffe3e3"));
        // The whole drawing's top-left (the inner group's frame) lands at the
        // drop point; shapes keep their places relative to each other.
        let (a, s) = (d.layout["auth_api"].pos, d.layout["auth_store"].pos);
        assert!(a.x >= 500.0 && a.y >= 400.0, "{a:?}");
        assert_eq!((s.x - a.x, s.y - a.y), (300.0, 0.0));
        // Dropping it again picks a fresh prefix.
        let again = insert(host.diagram(), other.diagram(), "auth", Point::new(0.0, 0.0));
        host.apply(&again).expect("applies again");
        assert!(host.diagram().node("auth2_api").is_some());
    }
}
