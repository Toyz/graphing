use crate::{Arrow, Diagram, Edge, Group, Node, Placement, Point, Step, Value};

/// One edit. Addressed by id, never by position, so ops stay meaningful when
/// replayed on a diverged copy (undo after other edits, future sync).
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Insert a node at `index` in source order (clamped).
    AddNode { node: Node, index: usize },
    /// Remove a node, its edges, its placement and its group memberships.
    RemoveNode { id: String },
    AddEdge { edge: Edge, index: usize },
    RemoveEdge { id: String },
    SetLabel { id: String, label: Option<String> },
    /// Set or clear (default shape) a node's stencil.
    SetStencil { id: String, stencil: Option<String> },
    SetArrow { id: String, arrow: Arrow },
    /// Set or clear (`None`) an inline prop on a node or edge.
    SetProp { id: String, key: String, value: Option<Value> },
    /// Set or clear the placement of a node or group.
    SetPlacement { id: String, placement: Option<Placement> },
    SetWaypoints { id: String, points: Vec<Point> },
    SetMembers { group: String, members: Vec<String> },
    /// Point an edge's ends at ports (or at the bare node with `None`).
    SetEdgePorts { id: String, from_port: Option<String>, to_port: Option<String> },
    AddGroup { group: Group, index: usize },
    /// Remove a group, its placement and its membership in other groups.
    /// Its members stay, now ungrouped.
    RemoveGroup { id: String },
    /// Diagram title (`diagram "..."`).
    SetTitle { title: Option<String> },
    /// Set or clear a diagram-level prop (`kind`, `view`, `look` ...).
    SetDiagramProp { key: String, value: Option<Value> },
    /// Insert an animation step at `index` (clamped). Steps have no ids, so
    /// these three address them by position.
    AddStep { index: usize, step: Step },
    RemoveStep { index: usize },
    /// Replace the step at `index`.
    SetStep { index: usize, step: Step },
    /// Applied in order; inverse is applied in reverse.
    Batch(Vec<Op>),
}

impl Op {
    /// The id this op is about, if it targets one thing.
    pub fn target(&self) -> Option<&str> {
        Some(match self {
            Op::AddNode { node, .. } => &node.id,
            Op::AddEdge { edge, .. } => &edge.id,
            Op::RemoveNode { id }
            | Op::RemoveEdge { id }
            | Op::SetLabel { id, .. }
            | Op::SetStencil { id, .. }
            | Op::SetArrow { id, .. }
            | Op::SetProp { id, .. }
            | Op::SetPlacement { id, .. }
            | Op::SetWaypoints { id, .. } => id,
            Op::SetMembers { group, .. } => group,
            Op::SetEdgePorts { id, .. } | Op::RemoveGroup { id } => id,
            Op::AddGroup { group, .. } => &group.id,
            Op::Batch(_) | Op::SetTitle { .. } | Op::SetDiagramProp { .. } | Op::AddStep { .. } | Op::RemoveStep { .. } | Op::SetStep { .. } => return None,
        })
    }
}

/// Give unnamed edges the keys the text would: `a->b`, then `a->b#2` and
/// on in order, as lowering does. Bend points move with their edges.
fn renumber(d: &mut Diagram) {
    let mut taken = std::collections::HashSet::new();
    let mut moved = Vec::new();
    for e in &mut d.edges {
        if crate::is_auto_key(e) {
            let key = crate::edge_key(&e.from, &e.to, |k| taken.contains(k));
            if key != e.id {
                moved.push((std::mem::replace(&mut e.id, key.clone()), key));
            }
        }
        taken.insert(e.id.clone());
    }
    let points: Vec<_> = moved.iter().map(|(old, _)| d.waypoints.remove(old)).collect();
    for ((_, new), pts) in moved.into_iter().zip(points) {
        if let Some(p) = pts {
            d.waypoints.insert(new, p);
        }
    }
}

/// Take removed ids out of the animation: from action targets (an action
/// left with none goes) and moves. Undo restores each changed step.
fn forget_in_steps(d: &mut Diagram, gone: &[String], undo: &mut Vec<Op>) {
    for (index, step) in d.steps.iter_mut().enumerate() {
        let mut next = step.clone();
        for a in &mut next.actions {
            a.targets.retain(|t| !gone.contains(t));
        }
        next.actions.retain(|a| !a.targets.is_empty());
        next.moves.retain(|(t, _)| !gone.contains(t));
        if next != *step {
            undo.push(Op::SetStep { index, step: std::mem::replace(step, next) });
        }
    }
}

pub(crate) fn apply(d: &mut Diagram, op: &Op) -> Option<Op> {
    match op {
        Op::AddNode { node, index } => {
            if d.node(&node.id).is_some() || d.group(&node.id).is_some() {
                return None;
            }
            let i = (*index).min(d.nodes.len());
            d.nodes.insert(i, node.clone());
            Some(Op::RemoveNode { id: node.id.clone() })
        }
        Op::RemoveNode { id } => {
            let index = d.nodes.iter().position(|n| &n.id == id)?;
            let node = d.nodes.remove(index);
            let mut undo = vec![Op::AddNode { node, index }];
            if let Some(p) = d.layout.remove(id) {
                undo.push(Op::SetPlacement { id: id.clone(), placement: Some(p) });
            }
            // Undo re-adds in ascending original index, so each insert lands
            // where it was.
            let mut original = 0;
            let mut i = 0;
            while i < d.edges.len() {
                if &d.edges[i].from == id || &d.edges[i].to == id {
                    let edge = d.edges.remove(i);
                    let points = d.waypoints.remove(&edge.id);
                    let eid = edge.id.clone();
                    undo.push(Op::AddEdge { edge, index: original });
                    if let Some(points) = points {
                        undo.push(Op::SetWaypoints { id: eid, points });
                    }
                } else {
                    i += 1;
                }
                original += 1;
            }
            for g in &mut d.groups {
                if g.members.contains(id) {
                    undo.push(Op::SetMembers { group: g.id.clone(), members: g.members.clone() });
                    g.members.retain(|m| m != id);
                }
            }
            let mut gone = vec![id.clone()];
            gone.extend(undo.iter().filter_map(|o| match o {
                Op::AddEdge { edge, .. } => Some(edge.id.clone()),
                _ => None,
            }));
            forget_in_steps(d, &gone, &mut undo);
            renumber(d);
            Some(Op::Batch(undo))
        }
        Op::AddEdge { edge, index } => {
            // An unnamed edge's key comes from its place among its parallel
            // siblings, so it may take a key in use; the rest move up.
            let auto = crate::is_auto_key(edge);
            if !auto && d.edge(&edge.id).is_some() {
                return None;
            }
            let i = (*index).min(d.edges.len());
            d.edges.insert(i, edge.clone());
            if auto {
                renumber(d);
            }
            Some(Op::RemoveEdge { id: d.edges[i].id.clone() })
        }
        Op::RemoveEdge { id } => {
            let index = d.edges.iter().position(|e| &e.id == id)?;
            let edge = d.edges.remove(index);
            let mut undo = vec![Op::AddEdge { edge, index }];
            if let Some(points) = d.waypoints.remove(id) {
                undo.push(Op::SetWaypoints { id: id.clone(), points });
            }
            forget_in_steps(d, std::slice::from_ref(id), &mut undo);
            renumber(d);
            Some(if undo.len() == 1 { undo.remove(0) } else { Op::Batch(undo) })
        }
        Op::SetLabel { id, label } => {
            let slot = if let Some(n) = d.node_mut(id) {
                &mut n.label
            } else if let Some(e) = d.edges.iter_mut().find(|e| &e.id == id) {
                &mut e.label
            } else {
                &mut d.groups.iter_mut().find(|g| &g.id == id)?.label
            };
            let old = std::mem::replace(slot, label.clone());
            Some(Op::SetLabel { id: id.clone(), label: old })
        }
        Op::SetStencil { id, stencil } => {
            let n = d.node_mut(id)?;
            let old = std::mem::replace(&mut n.stencil, stencil.clone());
            Some(Op::SetStencil { id: id.clone(), stencil: old })
        }
        Op::SetArrow { id, arrow } => {
            let e = d.edges.iter_mut().find(|e| &e.id == id)?;
            let old = std::mem::replace(&mut e.arrow, *arrow);
            Some(Op::SetArrow { id: id.clone(), arrow: old })
        }
        Op::SetProp { id, key, value } => {
            let props = if let Some(n) = d.node_mut(id) {
                &mut n.props
            } else if let Some(e) = d.edges.iter_mut().find(|e| &e.id == id) {
                &mut e.props
            } else {
                &mut d.groups.iter_mut().find(|g| &g.id == id)?.props
            };
            // Replace in place so source order (and the text) is stable.
            let at = props.iter().position(|(k, _)| k == key);
            let old = match (at, value) {
                (Some(i), Some(v)) => Some(std::mem::replace(&mut props[i].1, v.clone())),
                (Some(i), None) => Some(props.remove(i).1),
                (None, Some(v)) => {
                    props.push((key.clone(), v.clone()));
                    None
                }
                (None, None) => None,
            };
            Some(Op::SetProp { id: id.clone(), key: key.clone(), value: old })
        }
        Op::SetPlacement { id, placement } => {
            if d.node(id).is_none() && d.group(id).is_none() {
                return None;
            }
            let old = match placement {
                Some(p) => d.layout.insert(id.clone(), *p),
                None => d.layout.remove(id),
            };
            Some(Op::SetPlacement { id: id.clone(), placement: old })
        }
        Op::SetWaypoints { id, points } => {
            d.edge(id)?;
            let old = if points.is_empty() {
                d.waypoints.remove(id)
            } else {
                d.waypoints.insert(id.clone(), points.clone())
            };
            Some(Op::SetWaypoints { id: id.clone(), points: old.unwrap_or_default() })
        }
        Op::SetMembers { group, members } => {
            let g = d.groups.iter_mut().find(|g| &g.id == group)?;
            let old = std::mem::replace(&mut g.members, members.clone());
            Some(Op::SetMembers { group: group.clone(), members: old })
        }
        Op::SetEdgePorts { id, from_port, to_port } => {
            let e = d.edges.iter_mut().find(|e| &e.id == id)?;
            let old = Op::SetEdgePorts { id: id.clone(), from_port: e.from_port.clone(), to_port: e.to_port.clone() };
            e.from_port = from_port.clone();
            e.to_port = to_port.clone();
            Some(old)
        }
        Op::AddGroup { group, index } => {
            if d.node(&group.id).is_some() || d.group(&group.id).is_some() {
                return None;
            }
            let i = (*index).min(d.groups.len());
            d.groups.insert(i, group.clone());
            Some(Op::RemoveGroup { id: group.id.clone() })
        }
        Op::RemoveGroup { id } => {
            let index = d.groups.iter().position(|g| &g.id == id)?;
            let group = d.groups.remove(index);
            let mut undo = vec![Op::AddGroup { group, index }];
            if let Some(p) = d.layout.remove(id) {
                undo.push(Op::SetPlacement { id: id.clone(), placement: Some(p) });
            }
            for g in &mut d.groups {
                if g.members.contains(id) {
                    undo.push(Op::SetMembers { group: g.id.clone(), members: g.members.clone() });
                    g.members.retain(|m| m != id);
                }
            }
            forget_in_steps(d, std::slice::from_ref(id), &mut undo);
            Some(Op::Batch(undo))
        }
        Op::SetTitle { title } => {
            let old = std::mem::replace(&mut d.title, title.clone());
            Some(Op::SetTitle { title: old })
        }
        Op::SetDiagramProp { key, value } => {
            let props = &mut d.props;
            let at = props.iter().position(|(k, _)| k == key);
            let old = match (at, value) {
                (Some(i), Some(v)) => Some(std::mem::replace(&mut props[i].1, v.clone())),
                (Some(i), None) => Some(props.remove(i).1),
                (None, Some(v)) => {
                    props.push((key.clone(), v.clone()));
                    None
                }
                (None, None) => None,
            };
            Some(Op::SetDiagramProp { key: key.clone(), value: old })
        }
        Op::AddStep { index, step } => {
            let i = (*index).min(d.steps.len());
            d.steps.insert(i, step.clone());
            Some(Op::RemoveStep { index: i })
        }
        Op::RemoveStep { index } => {
            if *index >= d.steps.len() {
                return None;
            }
            let step = d.steps.remove(*index);
            Some(Op::AddStep { index: *index, step })
        }
        Op::SetStep { index, step } => {
            let slot = d.steps.get_mut(*index)?;
            let old = std::mem::replace(slot, step.clone());
            Some(Op::SetStep { index: *index, step: old })
        }
        Op::Batch(ops) => {
            let mut undo = Vec::with_capacity(ops.len());
            for o in ops {
                match apply(d, o) {
                    Some(inv) => undo.push(inv),
                    None => {
                        // Roll back what already applied so a batch is atomic.
                        for inv in undo.iter().rev() {
                            apply(d, inv);
                        }
                        return None;
                    }
                }
            }
            undo.reverse();
            Some(Op::Batch(undo))
        }
    }
}
