//! Editing commands as pure functions: model + scene in, `Op` out. No gpui,
//! so every command is unit tested without a window.

use std::collections::{HashMap, HashSet};

use graphing_dsl::Document;
use graphing_model::{Arrow, Diagram, Edge, Group, Node, Op, Placement, Point, Rect, Size, Value};
use graphing_scene::pins::PinDir;
use graphing_scene::{Scene, snap};

/// Smallest `{prefix}{n}` id not used by any node, group or edge.
pub fn fresh_id(d: &Diagram, prefix: &str) -> String {
    let taken = |id: &str| d.node(id).is_some() || d.group(id).is_some() || d.edge(id).is_some();
    (1..).map(|n| format!("{prefix}{n}")).find(|id| !taken(id)).expect("unbounded")
}

/// Key lowering would give the next unnamed `from -> to` edge.
pub fn edge_key(d: &Diagram, from: &str, to: &str) -> String {
    graphing_model::edge_key(from, to, |k| d.edge(k).is_some())
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

/// One end of a wire: node, pin, and which way the pin faces. An empty pin
/// name is the node itself, wired as an item (any shape can feed a pin).
pub type PinEnd<'a> = (&'a str, &'a str, PinDir);

/// Wire two pins, in whichever order they were picked: from the output to
/// the input. A data input (unless it takes `many`) or an execution output
/// already wired lets go of the old wire, in the same undo step. `Err`
/// says why the wire cannot be made.
pub fn wire(d: &Diagram, a: PinEnd, b: PinEnd) -> Result<Op, String> {
    if a.2 == b.2 {
        return Err(if a.2 == PinDir::In { "both ends are inputs; wire an output to an input".into() } else { "both ends are outputs; wire an output to an input".into() });
    }
    let (src, dst) = if a.2 == PinDir::Out { (a, b) } else { (b, a) };
    if src.0 == dst.0 {
        return Err("a node cannot wire to itself".into());
    }
    let pin = |(node, name, dir): PinEnd| {
        let n = d.node(node)?;
        if name.is_empty() {
            return Some(graphing_scene::pins::Pin::new("", None, dir));
        }
        graphing_scene::pins::pins(d, n).into_iter().find(|p| p.name == name && p.dir == dir)
    };
    let port = |p: PinEnd| (!p.1.is_empty()).then(|| p.1.to_string());
    let (Some(sp), Some(dp)) = (pin(src), pin(dst)) else { return Err("no such pin".into()) };
    // Which pin an edge end means: the source end is the output.
    type End<'e> = (&'e str, Option<&'e str>);
    fn ends(e: &Edge) -> Option<(End<'_>, End<'_>)> {
        match e.arrow {
            Arrow::Back => Some(((&e.to, e.to_port.as_deref()), (&e.from, e.from_port.as_deref()))),
            Arrow::Forward => Some(((&e.from, e.from_port.as_deref()), (&e.to, e.to_port.as_deref()))),
            _ => None,
        }
    }
    let at = |end: (&str, Option<&str>), p: PinEnd| end.0 == p.0 && end.1 == (!p.1.is_empty()).then_some(p.1);
    if d.edges.iter().filter_map(ends).any(|(s, t)| at(s, src) && at(t, dst)) {
        return Err("already wired".into());
    }
    let mut ops: Vec<Op> = d
        .edges
        .iter()
        .filter(|e| ends(e).is_some_and(|(s, t)| (!dp.exec() && !dp.many && at(t, dst)) || (sp.exec() && at(s, src))))
        .map(|e| Op::RemoveEdge { id: e.id.clone() })
        .collect();
    let mut probe = d.clone();
    probe.apply(&Op::Batch(ops.clone()));
    let edge = Edge {
        id: edge_key(&probe, src.0, dst.0),
        from: src.0.into(),
        to: dst.0.into(),
        from_port: port(src),
        to_port: port(dst),
        arrow: Arrow::Forward,
        ..Default::default()
    };
    let id = edge.id.clone();
    ops.push(Op::AddEdge { edge, index: probe.edges.len() });
    probe.apply(ops.last().expect("pushed"));
    if let Some(p) = graphing_scene::pins::problems(&probe).into_iter().find(|p| p.id == id) {
        return Err(p.message);
    }
    Ok(Op::Batch(ops))
}

/// The pin of `node` a wire from `from` would go to when dropped on the
/// node's body: one that fits, unwired ones first, in the node's order.
pub fn best_pin(d: &Diagram, from: PinEnd, node: &str) -> Option<(String, PinDir)> {
    let n = d.node(node)?;
    let wiring = graphing_scene::pins::analyze(d);
    let want = if from.2 == PinDir::Out { PinDir::In } else { PinDir::Out };
    let mut fitting: Vec<(usize, String)> = graphing_scene::pins::pins(d, n)
        .into_iter()
        .filter(|p| p.dir == want && wire(d, from, (node, &p.name, want)).is_ok())
        .map(|p| (wiring.wires.get(&(node.to_string(), p.name.clone(), want)).copied().unwrap_or(0), p.name))
        .collect();
    fitting.sort_by_key(|(wired, _)| usize::from(*wired > 0));
    fitting.into_iter().next().map(|(_, name)| (name, want))
}

// ---- editing pins ----

fn pin_key(dir: PinDir) -> &'static str {
    if dir == PinDir::In { "in" } else { "out" }
}

fn ident_like(s: &str) -> bool {
    !s.is_empty() && s.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && s.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// A pin as a list item: `a`, `a: float`, or quoted when the text needs it.
fn pin_item(name: &str, ty: Option<&str>) -> Value {
    match ty.filter(|t| !t.is_empty()) {
        Some(t) if ident_like(name) && (ident_like(t) || t.ends_with(".*")) => Value::Pair(name.into(), Box::new(Value::Ident(t.into()))),
        Some(t) if ident_like(name) => Value::Pair(name.into(), Box::new(Value::Str(t.into()))),
        Some(t) => Value::Str(format!("{name}: {t}")),
        None if ident_like(name) => Value::Ident(name.into()),
        None => Value::Str(name.into()),
    }
}

/// `node`'s `dir` pins as (name, type), from its own list or, when it has
/// none, as its stencil gives them (an edit then writes them out).
fn pin_list(d: &Diagram, node: &str, dir: PinDir) -> Option<Vec<(String, Option<String>)>> {
    let n = d.node(node)?;
    Some(graphing_scene::pins::pins(d, n).into_iter().filter(|p| p.dir == dir).map(|p| (p.name, p.ty)).collect())
}

fn write_pins(node: &str, dir: PinDir, list: &[(String, Option<String>)]) -> Op {
    let items = list.iter().map(|(n, t)| pin_item(n, t.as_deref())).collect();
    Op::SetProp { id: node.into(), key: pin_key(dir).into(), value: Some(Value::List(items)) }
}

/// Add a pin from `spec` (`name` or `name: type`), keeping names unique.
pub fn add_pin(d: &Diagram, node: &str, dir: PinDir, spec: &str) -> Option<Op> {
    let (name, ty) = match spec.split_once(':') {
        Some((n, t)) => (n.trim().to_string(), Some(t.trim().to_string()).filter(|t| !t.is_empty())),
        None => (spec.trim().to_string(), None),
    };
    let mut list = pin_list(d, node, dir)?;
    if name.is_empty() || list.iter().any(|(n, _)| *n == name) {
        return None;
    }
    list.push((name, ty));
    Some(write_pins(node, dir, &list))
}

/// Change a pin's type (`None`: untyped, takes anything).
pub fn set_pin_type(d: &Diagram, node: &str, dir: PinDir, name: &str, ty: Option<&str>) -> Option<Op> {
    let mut list = pin_list(d, node, dir)?;
    let slot = list.iter_mut().find(|(n, _)| n == name)?;
    let ty = ty.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if slot.1 == ty {
        return None;
    }
    slot.1 = ty;
    Some(write_pins(node, dir, &list))
}

/// Edges ending at `node`'s pin `name` facing `dir`, and which end.
fn pin_wires<'a>(d: &'a Diagram, node: &str, name: &str, dir: PinDir) -> Vec<(&'a Edge, bool)> {
    d.edges
        .iter()
        .filter_map(|e| {
            // The source end of a wire is an output, its target an input.
            let from_dir = if e.arrow == Arrow::Back { PinDir::In } else { PinDir::Out };
            let to_dir = if from_dir == PinDir::Out { PinDir::In } else { PinDir::Out };
            if e.from == node && e.from_port.as_deref() == Some(name) && from_dir == dir {
                Some((e, true))
            } else if e.to == node && e.to_port.as_deref() == Some(name) && to_dir == dir {
                Some((e, false))
            } else {
                None
            }
        })
        .collect()
}

/// The pin detail lists (`defaults`, `docs`, `sides`, `required`, `many`)
/// with the entry for `name` changed by `f`.
fn detail_ops(d: &Diagram, node: &str, f: impl Fn(&str, Vec<Value>) -> Vec<Value>) -> Vec<Op> {
    let Some(n) = d.node(node) else { return Vec::new() };
    ["defaults", "docs", "sides", "required", "many"]
        .into_iter()
        .filter_map(|key| {
            let before = n.props.iter().rev().find(|(k, _)| k == key).and_then(|(_, v)| v.as_list().map(<[Value]>::to_vec))?;
            let after = f(key, before.clone());
            (after != before).then(|| Op::SetProp { id: node.into(), key: key.into(), value: (!after.is_empty()).then_some(Value::List(after)) })
        })
        .collect()
}

fn item_name(v: &Value) -> String {
    match v {
        Value::Pair(n, _) => n.clone(),
        other => other.text().split(':').next().unwrap_or_default().trim().to_string(),
    }
}

/// Rename a pin; its wires and details follow.
pub fn rename_pin(d: &Diagram, node: &str, dir: PinDir, old: &str, new: &str) -> Option<Op> {
    let new = new.trim();
    let mut list = pin_list(d, node, dir)?;
    if new.is_empty() || new == old || list.iter().any(|(n, _)| n == new) {
        return None;
    }
    list.iter_mut().find(|(n, _)| n == old)?.0 = new.to_string();
    let mut ops = vec![write_pins(node, dir, &list)];
    for (e, from_end) in pin_wires(d, node, old, dir) {
        let (fp, tp) = if from_end { (Some(new.to_string()), e.to_port.clone()) } else { (e.from_port.clone(), Some(new.to_string())) };
        ops.push(Op::SetEdgePorts { id: e.id.clone(), from_port: fp, to_port: tp });
    }
    ops.extend(detail_ops(d, node, |_, items| {
        items
            .into_iter()
            .map(|it| match it {
                Value::Pair(n, v) if n == old => Value::Pair(new.to_string(), v),
                other if item_name(&other) == old => match &other {
                    Value::Ident(_) | Value::Str(_) if !other.text().contains(':') => pin_item(new, None),
                    _ => other,
                },
                other => other,
            })
            .collect()
    }));
    Some(Op::Batch(ops))
}

/// Remove a pin, the wires on it and its details.
pub fn remove_pin(d: &Diagram, node: &str, dir: PinDir, name: &str) -> Option<Op> {
    let mut list = pin_list(d, node, dir)?;
    let before = list.len();
    list.retain(|(n, _)| n != name);
    if list.len() == before {
        return None;
    }
    let mut ops: Vec<Op> = pin_wires(d, node, name, dir).into_iter().map(|(e, _)| Op::RemoveEdge { id: e.id.clone() }).collect();
    ops.push(write_pins(node, dir, &list));
    ops.extend(detail_ops(d, node, |_, items| items.into_iter().filter(|it| item_name(it) != name).collect()));
    Some(Op::Batch(ops))
}

/// Set or clear a pin's `defaults`, `docs` or `sides` entry.
pub fn set_pin_detail(d: &Diagram, node: &str, key: &str, name: &str, value: Option<&str>) -> Option<Op> {
    let n = d.node(node)?;
    let mut items = n.props.iter().rev().find(|(k, _)| k == key).and_then(|(_, v)| v.as_list().map(<[Value]>::to_vec)).unwrap_or_default();
    items.retain(|it| item_name(it) != name);
    if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
        let v = match v.parse::<f64>() {
            Ok(n) => Value::Num(n),
            Err(_) if ident_like(v) && key == "sides" => Value::Ident(v.into()),
            Err(_) => Value::Str(v.into()),
        };
        if !ident_like(name) {
            return None;
        }
        items.push(Value::Pair(name.into(), Box::new(v)));
    }
    let value = (!items.is_empty()).then_some(Value::List(items));
    (d.node_prop(n, key) != value.as_ref()).then(|| Op::SetProp { id: node.into(), key: key.into(), value })
}

/// Turn a pin's `required` or `many` mark on or off.
pub fn set_pin_flag(d: &Diagram, node: &str, key: &str, name: &str, on: bool) -> Option<Op> {
    let n = d.node(node)?;
    let mut items = n.props.iter().rev().find(|(k, _)| k == key).and_then(|(_, v)| v.as_list().map(<[Value]>::to_vec)).unwrap_or_default();
    let had = items.iter().any(|it| item_name(it) == name);
    if had == on {
        return None;
    }
    if on {
        items.push(pin_item(name, None));
    } else {
        items.retain(|it| item_name(it) != name);
    }
    Some(Op::SetProp { id: node.into(), key: key.into(), value: (!items.is_empty()).then_some(Value::List(items)) })
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

    const GRAPH: &str = "use graph\nbegin: graph.event\nx: graph.variable { out: [value: float] }\nflag: graph.variable { out: [value: bool] }\nadd: graph.pure { in: [a: float, b: float], out: [sum: float] }\nlog: graph.function\nlog2: graph.function\n";

    #[test]
    fn wires_run_output_to_input_whichever_end_is_picked() {
        let mut doc = Document::parse(GRAPH);
        // Picked input first: still written output -> input.
        let op = wire(doc.diagram(), ("add", "a", PinDir::In), ("x", "value", PinDir::Out)).unwrap();
        doc.apply(&op).unwrap();
        let e = doc.diagram().edge("x->add").unwrap();
        assert_eq!((e.from_port.as_deref(), e.to_port.as_deref(), e.arrow), (Some("value"), Some("a"), Arrow::Forward));
        assert!(doc.source().contains("x.value -> add.a"), "{}", doc.source());
        assert!(graphing_scene::pins::problems(doc.diagram()).is_empty());
    }

    #[test]
    fn wires_that_cannot_be_are_refused_with_a_reason() {
        let d = Document::parse(GRAPH);
        let d = d.diagram();
        let why = |a, b| wire(d, a, b).unwrap_err();
        assert!(why(("x", "value", PinDir::Out), ("flag", "value", PinDir::Out)).contains("both ends are outputs"));
        assert!(why(("flag", "value", PinDir::Out), ("add", "a", PinDir::In)).contains("gives bool"));
        assert!(why(("begin", "exec", PinDir::Out), ("add", "a", PinDir::In)).contains("mix execution and data"));
        assert!(why(("add", "sum", PinDir::Out), ("add", "a", PinDir::In)).contains("itself"));
    }

    #[test]
    fn a_new_wire_takes_over_a_one_wire_pin_in_one_step() {
        let mut doc = Document::parse(GRAPH);
        doc.apply(&wire(doc.diagram(), ("x", "value", PinDir::Out), ("add", "a", PinDir::In)).unwrap()).unwrap();
        let y = Document::parse("y: { out: [v: float] }\n");
        doc.apply(&Op::AddNode { node: y.diagram().nodes[0].clone(), index: 9 }).unwrap();
        // A second wire into `add.a` replaces the first.
        let op = wire(doc.diagram(), ("y", "v", PinDir::Out), ("add", "a", PinDir::In)).unwrap();
        doc.apply(&op).unwrap();
        let into: Vec<&str> = doc.diagram().edges.iter().filter(|e| e.to == "add").map(|e| e.from.as_str()).collect();
        assert_eq!(into, ["y"]);
        // An execution output leads one way: rewiring moves it.
        doc.apply(&wire(doc.diagram(), ("begin", "exec", PinDir::Out), ("log", "exec", PinDir::In)).unwrap()).unwrap();
        doc.apply(&wire(doc.diagram(), ("begin", "exec", PinDir::Out), ("log2", "exec", PinDir::In)).unwrap()).unwrap();
        let out: Vec<&str> = doc.diagram().edges.iter().filter(|e| e.from == "begin").map(|e| e.to.as_str()).collect();
        assert_eq!(out, ["log2"]);
        assert!(graphing_scene::pins::problems(doc.diagram()).is_empty(), "{}", doc.source());
        // Wiring the same pins twice says so.
        assert_eq!(wire(doc.diagram(), ("begin", "exec", PinDir::Out), ("log2", "exec", PinDir::In)).unwrap_err(), "already wired");
    }

    #[test]
    fn dropping_on_a_node_finds_the_pin_that_fits() {
        let mut doc = Document::parse(GRAPH);
        let d = doc.diagram();
        assert_eq!(best_pin(d, ("x", "value", PinDir::Out), "add"), Some(("a".into(), PinDir::In)));
        assert_eq!(best_pin(d, ("flag", "value", PinDir::Out), "add"), None);
        assert_eq!(best_pin(d, ("begin", "exec", PinDir::Out), "log"), Some(("exec".into(), PinDir::In)));
        // Unwired pins come first.
        doc.apply(&wire(doc.diagram(), ("x", "value", PinDir::Out), ("add", "a", PinDir::In)).unwrap()).unwrap();
        assert_eq!(best_pin(doc.diagram(), ("x", "value", PinDir::Out), "add"), Some(("b".into(), PinDir::In)));
        // From an input, the node's fitting output.
        assert_eq!(best_pin(doc.diagram(), ("add", "b", PinDir::In), "x"), Some(("value".into(), PinDir::Out)));
    }

    #[test]
    fn a_plain_shape_wires_in_as_an_item() {
        let mut doc = Document::parse("use c4, graph\ndb: c4.database\ndump: graph.task { in: [source: c4.database, n: int] }\n");
        let op = wire(doc.diagram(), ("dump", "source", PinDir::In), ("db", "", PinDir::Out)).unwrap();
        doc.apply(&op).unwrap();
        assert!(doc.source().contains("db -> dump.source"), "{}", doc.source());
        assert!(wire(doc.diagram(), ("db", "", PinDir::Out), ("dump", "n", PinDir::In)).unwrap_err().contains("takes int"));
        assert_eq!(best_pin(doc.diagram(), ("db", "", PinDir::Out), "dump"), None, "source is taken, n does not fit");
    }

    #[test]
    fn pins_are_edited_with_their_wires_and_details() {
        let src = "use graph\nx: graph.variable { out: [value: float] }\nadd: graph.pure { in: [a: float, b: float], out: [sum: float], defaults: [b: 1], required: [a] }\nx.value -> add.a\n";
        let mut doc = Document::parse(src);
        let ok = |doc: &mut Document, make: &dyn Fn(&Diagram) -> Option<Op>| {
            let op = make(doc.diagram());
            doc.apply(&op.expect("an op")).expect("applies");
            assert!(doc.diags().is_empty(), "{:?}\n{}", doc.diags(), doc.source());
        };
        // Add one, typed; names stay unique.
        ok(&mut doc, &|d| add_pin(d, "add", PinDir::In, "c: int"));
        assert!(add_pin(doc.diagram(), "add", PinDir::In, "c").is_none());
        assert!(doc.source().contains("in: [a: float, b: float, c: int]"), "{}", doc.source());
        // Rename: the wire and the details follow.
        ok(&mut doc, &|d| rename_pin(d, "add", PinDir::In, "a", "left"));
        let s = doc.source().to_string();
        assert!(s.contains("x.value -> add.left") && s.contains("required: [left]") && s.contains("in: [left: float"), "{s}");
        ok(&mut doc, &|d| rename_pin(d, "add", PinDir::In, "b", "right"));
        assert!(doc.source().contains("defaults: [right: 1]"), "{}", doc.source());
        // Retype, details, flags.
        ok(&mut doc, &|d| set_pin_type(d, "add", PinDir::In, "c", Some("T")));
        ok(&mut doc, &|d| set_pin_detail(d, "add", "docs", "c", Some("How many")));
        ok(&mut doc, &|d| set_pin_flag(d, "add", "many", "c", true));
        let s = doc.source().to_string();
        assert!(s.contains("c: T") && s.contains("docs: [c: \"How many\"]") && s.contains("many: [c]"), "{s}");
        // Remove: its wire and its details go with it.
        ok(&mut doc, &|d| remove_pin(d, "add", PinDir::In, "left"));
        let s = doc.source().to_string();
        assert!(!s.contains("left") && !s.contains("x.value ->"), "{s}");
        assert!(graphing_scene::pins::problems(doc.diagram()).is_empty());
    }

    #[test]
    fn editing_stencil_pins_writes_them_out() {
        let mut doc = Document::parse("use graph\ns: graph.sequence { outputs: 2 }\n");
        doc.apply(&add_pin(doc.diagram(), "s", PinDir::Out, "done: exec").unwrap()).unwrap();
        assert!(doc.source().contains("out: [\"then 0: exec\", \"then 1: exec\", done: exec]"), "{}", doc.source());
    }
}

