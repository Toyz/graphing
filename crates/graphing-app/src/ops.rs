//! Editing commands as pure functions: model + scene in, `Op` out. No gpui,
//! so every command is unit tested without a window.

use std::collections::{HashMap, HashSet};

use graphing_dsl::Document;
use graphing_model::{Arrow, Diagram, Edge, Group, Node, Op, Placement, Point, Rect, Size, Value};
use graphing_scene::{Scene, snap};

/// Smallest `{prefix}{n}` id not used by any node, group or edge.
pub fn fresh_id(d: &Diagram, prefix: &str) -> String {
    let taken = |id: &str| d.node(id).is_some() || d.group(id).is_some() || d.edge(id).is_some();
    (1..).map(|n| format!("{prefix}{n}")).find(|id| !taken(id)).expect("unbounded")
}

/// Key lowering would give the next unnamed `from -> to` edge.
pub fn edge_key(d: &Diagram, from: &str, to: &str) -> String {
    let base = format!("{from}->{to}");
    if d.edge(&base).is_none() {
        return base;
    }
    (2..).map(|n| format!("{base}#{n}")).find(|k| d.edge(k).is_none()).expect("unbounded")
}

pub fn add_node(d: &Diagram, stencil: Option<&str>, at: Point) -> (String, Op) {
    let id = fresh_id(d, "n");
    let node = Node { stencil: stencil.map(str::to_string), ..Node::new(&id) };
    let pos = Point::new(snap(at.x), snap(at.y));
    let op = Op::Batch(vec![
        Op::AddNode { node, index: d.nodes.len() },
        Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos, size: None }) },
    ]);
    (id, op)
}

pub fn add_edge(d: &Diagram, from: &str, to: &str) -> Op {
    add_edge_ports(d, (from, None), (to, None))
}

/// Edge between two ends, each a node and an optional port. Port to port
/// connections are plain connectors (SysML ibd style), others get an arrow.
pub fn add_edge_ports(d: &Diagram, from: (&str, Option<&str>), to: (&str, Option<&str>)) -> Op {
    let ported = from.1.is_some() || to.1.is_some();
    let edge = Edge {
        id: edge_key(d, from.0, to.0),
        from: from.0.into(),
        to: to.0.into(),
        from_port: from.1.map(str::to_string),
        to_port: to.1.map(str::to_string),
        arrow: if ported { Arrow::None } else { Arrow::Forward },
        ..Default::default()
    };
    Op::AddEdge { edge, index: d.edges.len() }
}

/// Every port a node has: declared in `ports` (`name : Type`) or used by a
/// connection end. Name, type, number of connections.
pub fn ports_of(d: &Diagram, node: &str) -> Vec<(String, Option<String>, usize)> {
    let mut out: Vec<(String, Option<String>, usize)> = Vec::new();
    if let Some(items) = d.node(node).and_then(|n| d.node_prop(n, "ports")).and_then(Value::as_list) {
        for it in items {
            let text = it.text();
            let (name, ty) = match text.split_once(':') {
                Some((n, t)) => (n.trim().to_string(), Some(t.trim().to_string())),
                None => (text.trim().to_string(), None),
            };
            if !name.is_empty() && !out.iter().any(|p| p.0 == name) {
                out.push((name, ty, 0));
            }
        }
    }
    for e in &d.edges {
        for (n, p) in [(&e.from, &e.from_port), (&e.to, &e.to_port)] {
            if n != node {
                continue;
            }
            let Some(p) = p else { continue };
            match out.iter_mut().find(|x| &x.0 == p) {
                Some(x) => x.2 += 1,
                None => out.push((p.clone(), None, 1)),
            }
        }
    }
    out
}

/// Declared `ports` list with `name` replaced (or removed with `None`).
fn edit_port_list(d: &Diagram, node: &str, old: Option<&str>, new: Option<&str>) -> Option<Op> {
    let n = d.node(node)?;
    let mut items: Vec<Value> = d.node_prop(n, "ports").and_then(Value::as_list).map(<[Value]>::to_vec).unwrap_or_default();
    let name_of = |v: &Value| v.text().split(':').next().unwrap_or_default().trim().to_string();
    let at = old.and_then(|o| items.iter().position(|v| name_of(v) == o));
    match (at, new) {
        (Some(i), Some(new)) => {
            let ty = items[i].text().split_once(':').map(|(_, t)| t.trim().to_string());
            items[i] = Value::Str(ty.map_or(new.to_string(), |t| format!("{new} : {t}")));
        }
        (Some(i), None) => {
            items.remove(i);
        }
        (None, Some(new)) => items.push(Value::Str(new.to_string())),
        (None, None) => return None,
    }
    let value = (!items.is_empty()).then_some(Value::List(items));
    Some(Op::SetProp { id: node.into(), key: "ports".into(), value })
}

/// Add a port (`name` or `name : Type`) to a node's declared ports.
pub fn add_port(d: &Diagram, node: &str, spec: &str) -> Option<Op> {
    let name = spec.split(':').next().unwrap_or_default().trim();
    if name.is_empty() || ports_of(d, node).iter().any(|p| p.0 == name) {
        return None;
    }
    edit_port_list(d, node, None, Some(spec.trim()))
}

/// Rename a port everywhere: the declaration and every connection end.
pub fn rename_port(d: &Diagram, node: &str, old: &str, new: &str) -> Option<Op> {
    if old == new || new.trim().is_empty() || new.contains(char::is_whitespace) {
        return None;
    }
    let mut ops: Vec<Op> = edit_port_list(d, node, Some(old), Some(new)).into_iter().collect();
    ops.extend(ends_with_port(d, node, old, Some(new)));
    (!ops.is_empty()).then_some(Op::Batch(ops))
}

/// Remove a port: the declaration goes, connections fall back to the node.
pub fn remove_port(d: &Diagram, node: &str, name: &str) -> Option<Op> {
    let mut ops: Vec<Op> = edit_port_list(d, node, Some(name), None).into_iter().collect();
    ops.extend(ends_with_port(d, node, name, None));
    (!ops.is_empty()).then_some(Op::Batch(ops))
}

fn ends_with_port(d: &Diagram, node: &str, port: &str, to: Option<&str>) -> Vec<Op> {
    d.edges
        .iter()
        .filter(|e| (e.from == node && e.from_port.as_deref() == Some(port)) || (e.to == node && e.to_port.as_deref() == Some(port)))
        .map(|e| {
            let swap = |n: &str, p: &Option<String>| if n == node && p.as_deref() == Some(port) { to.map(str::to_string) } else { p.clone() };
            Op::SetEdgePorts { id: e.id.clone(), from_port: swap(&e.from, &e.from_port), to_port: swap(&e.to, &e.to_port) }
        })
        .collect()
}

/// The group directly containing `id`, if any.
pub fn parent_group<'a>(d: &'a Diagram, id: &str) -> Option<&'a Group> {
    d.groups.iter().find(|g| g.members.iter().any(|m| m == id))
}

/// Wrap the selected nodes and groups in a new group. If they all share a
/// parent group, the new group nests inside it.
pub fn group_selection(d: &Diagram, ids: &[String]) -> Option<(String, Op)> {
    let picked: Vec<String> = ids.iter().filter(|id| d.node(id).is_some() || d.group(id).is_some()).cloned().collect();
    if picked.is_empty() {
        return None;
    }
    let parents: Vec<Option<String>> = picked.iter().map(|id| parent_group(d, id).map(|g| g.id.clone())).collect();
    let common = parents.first().cloned().flatten().filter(|p| parents.iter().all(|q| q.as_deref() == Some(p.as_str())));
    let id = fresh_id(d, "group");
    let mut ops = Vec::new();
    // Leave old groups first, so nothing ends up in two.
    for g in &d.groups {
        if g.members.iter().any(|m| picked.contains(m)) {
            let mut members: Vec<String> = g.members.iter().filter(|m| !picked.contains(m)).cloned().collect();
            if common.as_deref() == Some(g.id.as_str()) {
                members.push(id.clone());
            }
            ops.push(Op::SetMembers { group: g.id.clone(), members });
        }
    }
    let group = Group { id: id.clone(), label: Some("Group".into()), members: picked, props: Vec::new() };
    ops.insert(0, Op::AddGroup { group, index: d.groups.len() });
    Some((id, Op::Batch(ops)))
}

/// What deleting `ids` removes, for the confirmation dialog: a title, a
/// count line and the names involved. Counts include group contents and the
/// connections that go with removed shapes.
pub fn delete_summary(d: &Diagram, ids: &[String]) -> Option<DeleteSummary> {
    let Some(Op::Batch(ops)) = delete(d, ids) else { return None };
    let mut nodes: Vec<&str> = Vec::new();
    let mut groups: Vec<&str> = Vec::new();
    let mut edges: HashSet<&str> = HashSet::new();
    for op in &ops {
        match op {
            Op::RemoveNode { id } => nodes.extend(d.node(id).map(|n| n.id.as_str())),
            Op::RemoveGroup { id } => groups.extend(d.group(id).map(|g| g.id.as_str())),
            Op::RemoveEdge { id } => edges.extend(d.edge(id).map(|e| e.id.as_str())),
            _ => {}
        }
    }
    for e in &d.edges {
        if nodes.contains(&e.from.as_str()) || nodes.contains(&e.to.as_str()) {
            edges.insert(e.id.as_str());
        }
    }
    let name = |id: &str| {
        d.node(id).map(|n| n.text().replace('\n', " ")).or_else(|| d.group(id).map(|g| g.label.clone().unwrap_or_else(|| g.id.clone()))).unwrap_or_else(|| id.to_string())
    };
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let single_group = matches!(ids, [one] if d.group(one).is_some());
    let title = match ids {
        [one] if single_group => format!("Delete group \"{}\"?", name(one)),
        [one] if d.edge(one).is_some() => "Delete this connection?".to_string(),
        [one] => format!("Delete \"{}\"?", name(one)),
        _ => format!("Delete {} items?", ids.len()),
    };
    let message = if single_group {
        "Everything inside it goes too. You can undo this.".to_string()
    } else if ids.len() == 1 && !edges.is_empty() && d.node(&ids[0]).is_some() {
        format!("Its {} go too. You can undo this.", plural(edges.len(), "connection", "connections"))
    } else {
        "You can undo this.".to_string()
    };
    let mut names: Vec<(bool, String)> = groups.iter().map(|g| (true, name(g))).collect();
    names.extend(nodes.iter().map(|n| (false, name(n))));
    Some(DeleteSummary { title, message, nodes: nodes.len(), groups: groups.len(), edges: edges.len(), names })
}

/// See [`delete_summary`].
pub struct DeleteSummary {
    pub title: String,
    pub message: String,
    pub nodes: usize,
    pub groups: usize,
    pub edges: usize,
    /// (is a group, name), groups first.
    pub names: Vec<(bool, String)>,
}

/// A new sized group of kind `kind` (a pack group kind, or a design name)
/// filling `rect`. Ungrouped nodes and groups
/// lying wholly inside it join it, so a group drawn around things holds them.
pub fn add_group(d: &Diagram, scene: &Scene, kind: &str, rect: graphing_model::Rect) -> (String, Op) {
    let id = fresh_id(d, "group");
    let inside = |r: graphing_model::Rect| {
        r.origin.x >= rect.origin.x && r.origin.y >= rect.origin.y && r.origin.x + r.size.w <= rect.origin.x + rect.size.w && r.origin.y + r.size.h <= rect.origin.y + rect.size.h
    };
    let members: Vec<String> = scene
        .nodes
        .iter()
        .map(|n| (&n.id, n.rect))
        .chain(scene.groups.iter().map(|g| (&g.id, g.rect)))
        .filter(|(id, r)| inside(*r) && parent_group(d, id).is_none())
        .map(|(id, _)| id.clone())
        .collect();
    let (title, props) = match graphing_scene::stencils::registry().group_kind(kind) {
        // Plain groups stay plain in the text.
        Some(k) if k.name == "group" => (k.title.clone(), Vec::new()),
        Some(k) => {
            // The kind's field values start filled in, ready to edit.
            let mut props = vec![("kind".to_string(), Value::Ident(k.name.clone()))];
            props.extend(k.defaults.iter().filter(|(key, _)| k.props.iter().any(|p| &p.key == key)).cloned());
            (k.title.clone(), props)
        }
        None => {
            let look = graphing_scene::GroupLook::parse(kind).unwrap_or_default();
            (look.title().to_string(), vec![("look".to_string(), Value::Ident(look.name().into()))])
        }
    };
    let group = Group { id: id.clone(), label: Some(title), members, props };
    let place = Op::SetPlacement {
        id: id.clone(),
        placement: Some(Placement { pos: rect.origin, size: Some(graphing_model::Size::new(rect.size.w, rect.size.h)) }),
    };
    (id, Op::Batch(vec![Op::AddGroup { group, index: d.groups.len() }, place]))
}

/// Dissolve a group; its members move up into its parent group.
pub fn ungroup(d: &Diagram, id: &str) -> Option<Op> {
    let g = d.group(id)?;
    let mut ops = Vec::new();
    if let Some(parent) = parent_group(d, id) {
        let mut members: Vec<String> = Vec::new();
        for m in &parent.members {
            if m == id {
                members.extend(g.members.iter().cloned());
            } else {
                members.push(m.clone());
            }
        }
        ops.push(Op::SetMembers { group: parent.id.clone(), members });
    }
    ops.push(Op::RemoveGroup { id: id.into() });
    Some(Op::Batch(ops))
}

/// Move `node` into `target` (or out of every group with `None`).
pub fn set_group(d: &Diagram, node: &str, target: Option<&str>) -> Option<Op> {
    let current = parent_group(d, node).map(|g| g.id.clone());
    if current.as_deref() == target {
        return None;
    }
    let mut ops = Vec::new();
    if let Some(cur) = &current
        && let Some(g) = d.group(cur)
    {
        ops.push(Op::SetMembers { group: cur.clone(), members: g.members.iter().filter(|m| *m != node).cloned().collect() });
    }
    if let Some(t) = target {
        let g = d.group(t)?;
        let mut members = g.members.clone();
        members.push(node.into());
        ops.push(Op::SetMembers { group: t.into(), members });
    }
    Some(Op::Batch(ops))
}

/// After a group is resized to `rect`: nodes at the group's level whose
/// center is now inside join it; members whose center is now outside leave
/// for the group's parent.
pub fn refit_members(d: &Diagram, scene: &Scene, group: &str, rect: Rect) -> Vec<Op> {
    let Some(g) = d.group(group) else { return Vec::new() };
    let parent = parent_group(d, group).map(|p| p.id.clone());
    let inside = |id: &str| scene.rect_of(id).is_some_and(|r| rect.contains(r.center()));
    let mut probe = d.clone();
    let mut ops = Vec::new();
    let mut step = |probe: &mut Diagram, id: &str, target: Option<&str>| {
        if let Some(op) = set_group(probe, id, target) {
            probe.apply(&op);
            ops.push(op);
        }
    };
    for m in g.members.clone() {
        if d.node(&m).is_some() && !inside(&m) {
            step(&mut probe, &m, parent.as_deref());
        }
    }
    for n in &d.nodes {
        let level = parent_group(&probe, &n.id).map(|p| p.id.clone());
        if level == parent && inside(&n.id) {
            step(&mut probe, &n.id, Some(group));
        }
    }
    ops
}

/// After nodes moved: each lands in the innermost group whose box (as it was
/// before the move) holds its new center, or leaves its group when dropped
/// outside it. `before` is the scene before the drag.
pub fn regroup_after_move(d: &Diagram, before: &Scene, moved: &[(String, Point)]) -> Vec<Op> {
    let moving: Vec<&str> = moved.iter().map(|(id, _)| id.as_str()).collect();
    let mut probe = d.clone();
    let mut ops = Vec::new();
    for (id, center) in moved {
        if probe.node(id).is_none() {
            continue;
        }
        let target = before
            .groups
            .iter()
            .filter(|g| !moving.contains(&g.id.as_str()) && g.rect.contains(*center))
            .min_by(|a, b| (a.rect.size.w * a.rect.size.h).total_cmp(&(b.rect.size.w * b.rect.size.h)))
            .map(|g| g.id.clone());
        let current = parent_group(&probe, id).map(|g| g.id.clone());
        // Still inside its own group's old box: leave membership alone.
        let stays = current.as_ref().is_some_and(|c| before.groups.iter().any(|g| &g.id == c && g.rect.contains(*center)));
        if stays && target.as_ref().is_none_or(|t| Some(t) == current.as_ref()) {
            continue;
        }
        if let Some(op) = set_group(&probe, id, target.as_deref()) {
            probe.apply(&op);
            ops.push(op);
        }
    }
    ops
}


/// New node at `at` connected from `from`, same stencil as the source.
pub fn add_connected(d: &Diagram, from: &str, at: Point) -> (String, Op) {
    let stencil = d.node(from).and_then(|n| n.stencil.clone());
    let (id, add) = add_node(d, stencil.as_deref(), at);
    let mut probe = d.clone();
    probe.apply(&add);
    let edge = add_edge(&probe, from, &id);
    (id, Op::Batch(vec![add, edge]))
}

/// Remove nodes, groups' memberships and edges for every selected id.
pub fn delete(d: &Diagram, ids: &[String]) -> Option<Op> {
    let mut ops = Vec::new();
    // A group goes with everything inside it, nested groups included.
    let mut groups: Vec<&str> = Vec::new();
    let mut stack: Vec<&str> = ids.iter().map(String::as_str).filter(|id| d.group(id).is_some()).collect();
    while let Some(g) = stack.pop() {
        if groups.contains(&g) {
            continue;
        }
        groups.push(g);
        stack.extend(d.group(g).into_iter().flat_map(|g| g.members.iter().map(String::as_str)).filter(|m| d.group(m).is_some()));
    }
    let mut nodes: HashSet<&str> = ids.iter().map(String::as_str).filter(|id| d.node(id).is_some()).collect();
    for g in &groups {
        nodes.extend(d.group(g).into_iter().flat_map(|g| g.members.iter().map(String::as_str)).filter(|m| d.node(m).is_some()));
    }
    for e in &d.edges {
        // Edges touching a removed node go with it; those touching a removed
        // group, or picked themselves, are removed here.
        let picked = ids.contains(&e.id);
        let on_group = groups.contains(&e.from.as_str()) || groups.contains(&e.to.as_str());
        if (picked || on_group) && !nodes.contains(e.from.as_str()) && !nodes.contains(e.to.as_str()) {
            ops.push(Op::RemoveEdge { id: e.id.clone() });
        }
    }
    let mut nodes: Vec<&str> = nodes.into_iter().collect();
    nodes.sort_unstable();
    ops.extend(nodes.iter().map(|id| Op::RemoveNode { id: id.to_string() }));
    // Inner groups first.
    ops.extend(groups.iter().rev().map(|g| Op::RemoveGroup { id: g.to_string() }));
    (!ops.is_empty()).then_some(Op::Batch(ops))
}

/// Selected nodes, the edges between them and their geometry as `.gph`
/// text. Pasting it into any diagram (or a text editor) round-trips.
pub fn copy(d: &Diagram, ids: &[String]) -> Option<String> {
    let picked: HashSet<&str> = ids.iter().map(String::as_str).filter(|id| d.node(id).is_some()).collect();
    if picked.is_empty() {
        return None;
    }
    let mut doc = Document::parse("");
    let mut ops = Vec::new();
    for (i, n) in d.nodes.iter().filter(|n| picked.contains(n.id.as_str())).enumerate() {
        let mut n = n.clone();
        // Classes may not exist in the target diagram; inline what they set.
        for c in std::mem::take(&mut n.classes) {
            for (k, v) in d.styles.get(&c).into_iter().flatten() {
                if graphing_model::find_prop(&n.props, k).is_none() {
                    n.props.push((k.clone(), v.clone()));
                }
            }
        }
        ops.push(Op::AddNode { node: n, index: i });
    }
    for (i, e) in d.edges.iter().filter(|e| picked.contains(e.from.as_str()) && picked.contains(e.to.as_str())).enumerate() {
        ops.push(Op::AddEdge { edge: Edge { classes: Vec::new(), ..e.clone() }, index: i });
    }
    for id in &picked {
        if let Some(p) = d.layout.get(*id) {
            ops.push(Op::SetPlacement { id: id.to_string(), placement: Some(*p) });
        }
    }
    doc.apply(&Op::Batch(ops))?;
    Some(doc.source().to_string())
}

/// Ops that add `text`'s nodes and edges to `d`, renaming clashing ids and
/// shifting geometry by `offset`. Returns the new node ids.
pub fn paste(d: &Diagram, text: &str, offset: Point) -> Option<(Vec<String>, Op)> {
    let src = Document::parse(text);
    let s = src.diagram();
    if s.nodes.is_empty() {
        return None;
    }
    let mut probe = d.clone();
    let mut rename: HashMap<String, String> = HashMap::new();
    let mut ops = Vec::new();
    let mut new_ids = Vec::new();
    for n in &s.nodes {
        let taken = probe.node(&n.id).is_some() || probe.group(&n.id).is_some();
        // `a_1` copies to `a_2`, not `a_1_1`.
        let base = n.id.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches('_');
        let base = if base.is_empty() { n.id.as_str() } else { base };
        let id = if taken { fresh_id(&probe, &format!("{base}_")) } else { n.id.clone() };
        rename.insert(n.id.clone(), id.clone());
        let mut add = vec![Op::AddNode { node: Node { id: id.clone(), ..n.clone() }, index: probe.nodes.len() }];
        if let Some(p) = s.layout.get(&n.id) {
            let pos = Point::new(p.pos.x + offset.x, p.pos.y + offset.y);
            add.push(Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos, size: p.size }) });
        }
        let op = Op::Batch(add);
        probe.apply(&op)?;
        ops.push(op);
        new_ids.push(id);
    }
    for e in &s.edges {
        let (from, to) = (rename[&e.from].clone(), rename[&e.to].clone());
        let id = edge_key(&probe, &from, &to);
        let op = Op::AddEdge { edge: Edge { id, from, to, ..e.clone() }, index: probe.edges.len() };
        probe.apply(&op)?;
        ops.push(op);
    }
    Some((new_ids, Op::Batch(ops)))
}

pub fn move_by(scene: &Scene, d: &Diagram, ids: &[String], dx: f64, dy: f64) -> Option<Op> {
    let ops: Vec<Op> = ids
        .iter()
        .filter(|id| d.node(id).is_some())
        .filter_map(|id| {
            let r = scene.rect_of(id)?;
            let size = d.layout.get(id).and_then(|p| p.size);
            let pos = Point::new(r.origin.x + dx, r.origin.y + dy);
            Some(Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos, size }) })
        })
        .collect();
    (!ops.is_empty()).then_some(Op::Batch(ops))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
    /// Equal gaps between neighbours, left to right.
    SpreadX,
    /// Equal gaps between neighbours, top to bottom.
    SpreadY,
}

pub fn align(scene: &Scene, d: &Diagram, ids: &[String], how: Align) -> Option<Op> {
    let mut items: Vec<(String, Rect)> =
        ids.iter().filter(|id| d.node(id).is_some()).filter_map(|id| Some((id.clone(), scene.rect_of(id)?))).collect();
    if items.len() < 2 {
        return None;
    }
    let min_x = items.iter().map(|(_, r)| r.origin.x).fold(f64::INFINITY, f64::min);
    let max_x = items.iter().map(|(_, r)| r.origin.x + r.size.w).fold(f64::NEG_INFINITY, f64::max);
    let min_y = items.iter().map(|(_, r)| r.origin.y).fold(f64::INFINITY, f64::min);
    let max_y = items.iter().map(|(_, r)| r.origin.y + r.size.h).fold(f64::NEG_INFINITY, f64::max);
    let mut out: Vec<(String, Point)> = Vec::new();
    match how {
        Align::SpreadX | Align::SpreadY => {
            if items.len() < 3 {
                return None;
            }
            let horizontal = how == Align::SpreadX;
            let key = |r: &Rect| if horizontal { r.origin.x } else { r.origin.y };
            let extent = |r: &Rect| if horizontal { r.size.w } else { r.size.h };
            items.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
            let total: f64 = items.iter().map(|(_, r)| extent(r)).sum();
            let span = if horizontal { max_x - min_x } else { max_y - min_y };
            let gap = (span - total) / (items.len() - 1) as f64;
            let mut at = if horizontal { min_x } else { min_y };
            for (id, r) in &items {
                let p = if horizontal { Point::new(at, r.origin.y) } else { Point::new(r.origin.x, at) };
                out.push((id.clone(), p));
                at += extent(r) + gap;
            }
        }
        _ => {
            for (id, r) in &items {
                let (x, y) = (r.origin.x, r.origin.y);
                let p = match how {
                    Align::Left => Point::new(min_x, y),
                    Align::Right => Point::new(max_x - r.size.w, y),
                    Align::CenterX => Point::new((min_x + max_x) / 2.0 - r.size.w / 2.0, y),
                    Align::Top => Point::new(x, min_y),
                    Align::Bottom => Point::new(x, max_y - r.size.h),
                    Align::CenterY => Point::new(x, (min_y + max_y) / 2.0 - r.size.h / 2.0),
                    Align::SpreadX | Align::SpreadY => unreachable!(),
                };
                out.push((id.clone(), p));
            }
        }
    }
    let ops = out
        .into_iter()
        .map(|(id, p)| {
            let size = d.layout.get(&id).and_then(|pl| pl.size);
            let pos = Point::new(p.x.round(), p.y.round());
            Op::SetPlacement { id, placement: Some(Placement { pos, size }) }
        })
        .collect();
    Some(Op::Batch(ops))
}

/// Same size for every selected node: the largest of them.
pub fn same_size(scene: &Scene, d: &Diagram, ids: &[String]) -> Option<Op> {
    let rects: Vec<(String, Rect)> =
        ids.iter().filter(|id| d.node(id).is_some()).filter_map(|id| Some((id.clone(), scene.rect_of(id)?))).collect();
    if rects.len() < 2 {
        return None;
    }
    let w = rects.iter().map(|(_, r)| r.size.w).fold(0.0, f64::max);
    let h = rects.iter().map(|(_, r)| r.size.h).fold(0.0, f64::max);
    let ops = rects
        .into_iter()
        .map(|(id, r)| Op::SetPlacement { id, placement: Some(Placement { pos: r.origin, size: Some(Size::new(w, h)) }) })
        .collect();
    Some(Op::Batch(ops))
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphing_scene::build;

    #[test]
    fn ports_list_rename_remove() {
        let mut doc = Document::parse("a { ports: [\"out : Data\"] }\nb\na.out -- b.in\n");
        let d = doc.diagram();
        assert_eq!(ports_of(d, "a"), vec![("out".to_string(), Some("Data".to_string()), 1)]);
        assert_eq!(ports_of(d, "b"), vec![("in".to_string(), None, 1)]);
        doc.apply(&rename_port(doc.diagram(), "a", "out", "tx").unwrap()).unwrap();
        assert!(doc.source().contains("\"tx : Data\"") && doc.source().contains("a.tx -- b.in"), "{}", doc.source());
        doc.apply(&add_port(doc.diagram(), "b", "pwr : Power").unwrap()).unwrap();
        assert_eq!(ports_of(doc.diagram(), "b").len(), 2);
        doc.apply(&remove_port(doc.diagram(), "b", "in").unwrap()).unwrap();
        assert!(doc.source().contains("a.tx -- b\n"), "{}", doc.source());
        assert!(doc.diags().is_empty());
    }

    #[test]
    fn group_ungroup_and_move_into() {
        let mut doc = Document::parse("a\nb\nc\nlayout {\n  a 0 0\n  b 200 0\n  c 600 0\n}\n");
        let (g, op) = group_selection(doc.diagram(), &ids(&["a", "b"])).unwrap();
        doc.apply(&op).unwrap();
        assert_eq!(doc.diagram().group(&g).unwrap().members, ["a", "b"]);
        // Nested: grouping one member of g nests the new group inside g.
        let (inner, op) = group_selection(doc.diagram(), &ids(&["a"])).unwrap();
        doc.apply(&op).unwrap();
        assert_eq!(doc.diagram().group(&g).unwrap().members, ["b", inner.as_str()]);
        doc.apply(&ungroup(doc.diagram(), &inner).unwrap()).unwrap();
        assert_eq!(doc.diagram().group(&g).unwrap().members, ["b", "a"]);
        // Drop c into g's box: it joins.
        let before = build(doc.diagram(), &HashMap::new());
        let center = before.rect_of(&g).unwrap().center();
        for op in regroup_after_move(doc.diagram(), &before, &[("c".into(), center)]) {
            doc.apply(&op).unwrap();
        }
        assert!(doc.diagram().group(&g).unwrap().members.contains(&"c".to_string()));
        // Drag a far away: it leaves.
        let before = build(doc.diagram(), &HashMap::new());
        for op in regroup_after_move(doc.diagram(), &before, &[("a".into(), Point::new(5000.0, 5000.0))]) {
            doc.apply(&op).unwrap();
        }
        assert!(!doc.diagram().group(&g).unwrap().members.contains(&"a".to_string()));
        assert!(doc.diags().is_empty(), "{:?}", doc.diags());
    }

    const SRC: &str = "style s { fill: #abc }\na: rect \"A\" .s\nb\nc\na -> b \"x\"\nb -> c\nlayout {\n  a 0 0\n  b 200 10\n  c 500 40\n}\n";

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn copy_paste_renames_and_offsets() {
        let mut doc = Document::parse(SRC);
        let text = copy(doc.diagram(), &ids(&["a", "b"])).unwrap();
        assert!(text.contains("fill: #abc"), "{text}");
        assert!(!text.contains("b -> c"), "{text}");
        let (new, op) = paste(doc.diagram(), &text, Point::new(20.0, 20.0)).unwrap();
        assert_eq!(new, ["a_1", "b_1"]);
        doc.apply(&op).unwrap();
        let d = doc.diagram();
        assert!(doc.diags().is_empty(), "{:?}", doc.diags());
        assert_eq!(d.layout["a_1"].pos, Point::new(20.0, 20.0));
        assert_eq!(d.edge("a_1->b_1").unwrap().label.as_deref(), Some("x"));
    }

    #[test]
    fn connect_to_empty_space_makes_node() {
        let mut doc = Document::parse(SRC);
        let (id, op) = add_connected(doc.diagram(), "a", Point::new(103.0, 297.0));
        doc.apply(&op).unwrap();
        let d = doc.diagram();
        assert_eq!(id, "n1");
        assert_eq!(d.node("n1").unwrap().stencil.as_deref(), Some("rect"));
        assert_eq!(d.layout["n1"].pos, Point::new(100.0, 300.0));
        assert!(d.edge("a->n1").is_some());
        // Second parallel edge gets the next generated key.
        let op = add_edge(d, "a", "b");
        doc.apply(&op).unwrap();
        assert!(doc.diagram().edge("a->b#2").is_some(), "{}", doc.source());
    }

    #[test]
    fn delete_mixed_selection() {
        let mut doc = Document::parse(SRC);
        let op = delete(doc.diagram(), &ids(&["b->c", "a"])).unwrap();
        doc.apply(&op).unwrap();
        let d = doc.diagram();
        assert!(d.node("a").is_none() && d.edges.is_empty());
    }

    #[test]
    fn align_and_spread() {
        let mut doc = Document::parse(SRC);
        let scene = build(doc.diagram(), &HashMap::new());
        let op = align(&scene, doc.diagram(), &ids(&["a", "b", "c"]), Align::Top).unwrap();
        doc.apply(&op).unwrap();
        assert!(doc.diagram().layout.values().all(|p| p.pos.y == 0.0));
        let scene = build(doc.diagram(), &HashMap::new());
        let op = align(&scene, doc.diagram(), &ids(&["a", "b", "c"]), Align::SpreadX).unwrap();
        doc.apply(&op).unwrap();
        let l = &doc.diagram().layout;
        // 120 wide boxes across 0..620: gaps of 130.
        assert_eq!((l["a"].pos.x, l["b"].pos.x, l["c"].pos.x), (0.0, 250.0, 500.0));
    }

    #[test]
    fn nudge_and_same_size() {
        let mut doc = Document::parse(SRC);
        let scene = build(doc.diagram(), &HashMap::new());
        doc.apply(&move_by(&scene, doc.diagram(), &ids(&["a"]), 1.0, -10.0).unwrap()).unwrap();
        assert_eq!(doc.diagram().layout["a"].pos, Point::new(1.0, -10.0));
        doc.apply(&Op::SetPlacement { id: "c".into(), placement: Some(Placement { pos: Point::new(500.0, 40.0), size: Some(Size::new(300.0, 90.0)) }) }).unwrap();
        let scene = build(doc.diagram(), &HashMap::new());
        doc.apply(&same_size(&scene, doc.diagram(), &ids(&["a", "c"])).unwrap()).unwrap();
        assert_eq!(doc.diagram().layout["a"].size, Some(Size::new(300.0, 90.0)));
    }
}
