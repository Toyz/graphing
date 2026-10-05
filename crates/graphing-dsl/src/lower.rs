use std::collections::HashMap;

use crate::ast::*;
use crate::{Diag, Severity};
use graphing_model::{Action, Diagram, Ease, Edge, Group, Node, Placement, Point, Props, Size, Step, Verb};

/// Where each model id came from in the source, for patching.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// Node id -> statement index. Missing for nodes only implied by edges.
    pub nodes: HashMap<String, usize>,
    /// Edge id -> (statement index, hop index in the chain).
    pub edges: HashMap<String, (usize, usize)>,
    pub groups: HashMap<String, usize>,
    /// The first `layout { }` block.
    pub layout: Option<usize>,
    /// Layout target (node, group or edge id) -> entry index in that block.
    pub entries: HashMap<String, usize>,
    /// The first `animate { }` block.
    pub animate: Option<usize>,
}

pub fn lower(file: &File, diags: &mut Vec<Diag>) -> (Diagram, Index) {
    let mut d = Diagram::default();
    let mut ix = Index::default();
    let mut warn = |span: &std::ops::Range<usize>, msg: String| {
        diags.push(Diag { span: span.clone(), message: msg, severity: Severity::Warning });
    };

    for (si, stmt) in file.stmts.iter().enumerate() {
        match &stmt.kind {
            StmtKind::Diagram { title, props } => {
                d.title = title.as_ref().map(|t| t.value.clone());
                d.props = props_of(props.as_ref());
            }
            StmtKind::Use { packs } => d.packs.extend(packs.iter().map(|p| p.value.clone())),
            StmtKind::Style { name, props } => {
                d.styles.insert(name.value.clone(), props_of(Some(props)));
            }
            StmtKind::Node(n) => {
                if ix.nodes.contains_key(&n.id.value) || ix.groups.contains_key(&n.id.value) {
                    warn(&n.id.span, format!("duplicate id `{}`, ignored", n.id.value));
                    continue;
                }
                ix.nodes.insert(n.id.value.clone(), si);
                d.nodes.push(Node {
                    id: n.id.value.clone(),
                    stencil: n.stencil.as_ref().map(|s| s.value.clone()),
                    label: n.label.as_ref().map(|s| s.value.clone()),
                    classes: n.classes.iter().map(|c| c.value.clone()).collect(),
                    props: props_of(n.props.as_ref()),
                });
            }
            StmtKind::Group(g) => {
                if ix.nodes.contains_key(&g.id.value) || ix.groups.contains_key(&g.id.value) {
                    warn(&g.id.span, format!("duplicate id `{}`, ignored", g.id.value));
                    continue;
                }
                ix.groups.insert(g.id.value.clone(), si);
                d.groups.push(Group {
                    id: g.id.value.clone(),
                    label: g.label.as_ref().map(|s| s.value.clone()),
                    members: g.members.iter().map(|m| m.value.clone()).collect(),
                    props: props_of(g.props.as_ref()),
                });
            }
            StmtKind::Edge(_) | StmtKind::Layout(_) | StmtKind::Animate(_) | StmtKind::Unknown => {}
        }
    }

    // Edges after nodes so implied endpoints land after declared nodes.
    let mut implied = Vec::new();
    for (si, stmt) in file.stmts.iter().enumerate() {
        let StmtKind::Edge(e) = &stmt.kind else { continue };
        for hop in 0..e.arrows.len() {
            let (from, from_port) = split_port(&e.chain[hop].value);
            let (to, to_port) = split_port(&e.chain[hop + 1].value);
            for end in [&from, &to] {
                if !ix.nodes.contains_key(end) && !implied.contains(end) {
                    if ix.groups.contains_key(end) {
                        continue;
                    }
                    implied.push(end.clone());
                }
            }
            let id = match (&e.id, e.arrows.len()) {
                (Some(id), 1) => id.value.clone(),
                _ => edge_key(&ix.edges, &from, &to),
            };
            if ix.edges.contains_key(&id) {
                warn(&stmt.span, format!("duplicate edge id `{id}`, ignored"));
                continue;
            }
            ix.edges.insert(id.clone(), (si, hop));
            d.edges.push(Edge {
                id,
                from,
                to,
                from_port,
                to_port,
                arrow: e.arrows[hop],
                label: e.label.as_ref().map(|s| s.value.clone()),
                classes: e.classes.iter().map(|c| c.value.clone()).collect(),
                props: props_of(e.props.as_ref()),
            });
        }
    }
    d.nodes.extend(implied.into_iter().map(Node::new));

    for (si, stmt) in file.stmts.iter().enumerate() {
        let StmtKind::Layout(block) = &stmt.kind else { continue };
        if ix.layout.is_some() {
            warn(&stmt.span, "only the first layout block is used".into());
            continue;
        }
        ix.layout = Some(si);
        for (ei, entry) in block.entries.iter().enumerate() {
            let key = match &entry.target {
                LayoutTarget::Id(id) => id.clone(),
                LayoutTarget::Edge(a, b) => format!("{a}->{b}"),
            };
            let is_edge = d.edge(&key).is_some();
            let is_box = d.node(&key).is_some() || d.group(&key).is_some();
            if !is_edge && !is_box {
                warn(&entry.span, format!("layout entry for unknown `{key}`"));
                continue;
            }
            ix.entries.insert(key.clone(), ei);
            if is_box && let Some((x, y)) = entry.pos {
                let size = entry.size.map(|(w, h)| Size::new(w, h));
                d.layout.insert(key.clone(), Placement { pos: Point::new(x, y), size });
            }
            if !entry.via.is_empty() {
                d.waypoints.insert(key, entry.via.iter().map(|&(x, y)| Point::new(x, y)).collect());
            }
        }
    }
    for (si, stmt) in file.stmts.iter().enumerate() {
        let StmtKind::Animate(block) = &stmt.kind else { continue };
        if ix.animate.is_some() {
            warn(&stmt.span, "only the first animate block is used".into());
            continue;
        }
        ix.animate = Some(si);
        for st in &block.steps {
            let mut actions = Vec::new();
            for a in &st.actions {
                let Some(verb) = Verb::parse(&a.verb.value) else {
                    warn(&a.verb.span, format!("unknown action `{}` (show, hide, flow, focus, highlight)", a.verb.value));
                    continue;
                };
                for t in &a.targets {
                    let known = d.node(&t.value).is_some()
                        || d.group(&t.value).is_some()
                        || d.edge(&t.value).is_some()
                        || verb == Verb::Focus && t.value == "all";
                    if !known {
                        warn(&t.span, format!("`{}` is not in the diagram", t.value));
                    }
                }
                actions.push(Action { verb, targets: a.targets.iter().map(|t| t.value.clone()).collect() });
            }
            let ease = st.ease.as_ref().and_then(|e| {
                let parsed = Ease::parse(&e.value);
                if parsed.is_none() {
                    warn(&e.span, format!("unknown ease `{}` (smooth, linear, snappy, bounce)", e.value));
                }
                parsed
            });
            let mut moves = Vec::new();
            for MoveDecl { target: t, to: (x, y), .. } in &st.moves {
                if d.node(&t.value).is_none() && d.group(&t.value).is_none() {
                    warn(&t.span, format!("`{}` is not a shape or group to move", t.value));
                }
                moves.push((t.value.clone(), Point::new(*x, *y)));
            }
            d.steps.push(Step { title: st.title.as_ref().map(|t| t.value.clone()), seconds: st.seconds, ease, actions, moves });
        }
    }
    (d, ix)
}

/// `node.port` -> (`node`, Some(`port`)).
fn split_port(end: &str) -> (String, Option<String>) {
    match end.split_once('.') {
        Some((n, p)) => (n.to_string(), Some(p.to_string())),
        None => (end.to_string(), None),
    }
}

fn edge_key(existing: &HashMap<String, (usize, usize)>, from: &str, to: &str) -> String {
    graphing_model::edge_key(from, to, |k| existing.contains_key(k))
}

fn props_of(block: Option<&PropBlock>) -> Props {
    block
        .map(|b| b.entries.iter().map(|e| (e.key.value.clone(), e.value.value.clone())).collect())
        .unwrap_or_default()
}
