//! Op -> text splices.
//!
//! Every patch step works on fresh spans: compound ops are broken into
//! smaller steps with a reparse after each one.

use std::ops::Range;

use crate::Document;
use crate::ast::{LayoutEntry, PropBlock, StmtKind};
use crate::print::{fmt_arrow, fmt_edge, fmt_node, fmt_num, fmt_props, fmt_step, fmt_str, fmt_value};
use graphing_model::{Node, Op, Placement, Point, Value};

struct Splice {
    range: Range<usize>,
    text: String,
}

fn put(range: Range<usize>, text: impl Into<String>) -> Splice {
    Splice { range, text: text.into() }
}

fn ins(at: usize, text: impl Into<String>) -> Splice {
    put(at..at, text)
}

impl Document {
    pub(crate) fn patch(&mut self, op: &Op) {
        match op {
            Op::Batch(ops) => ops.iter().for_each(|o| self.patch(o)),
            Op::RemoveNode { id } => self.remove_node(id),
            Op::RemoveEdge { id } => self.remove_edge(id),
            Op::RemoveGroup { id } => self.remove_group(id),
            _ => {
                let splices = self.splices(op);
                self.commit(splices);
            }
        }
    }

    fn commit(&mut self, mut splices: Vec<Splice>) {
        if splices.is_empty() {
            return;
        }
        splices.sort_by_key(|s| std::cmp::Reverse(s.range.start));
        for s in splices {
            self.src.replace_range(s.range, &s.text);
        }
        self.reparse();
    }

    fn splices(&self, op: &Op) -> Vec<Splice> {
        match op {
            Op::AddNode { node, index } => vec![self.add_node(node, *index)],
            Op::AddEdge { edge, index } => vec![self.add_edge(edge, *index)],
            Op::SetLabel { id, label } => self.set_label(id, label.as_deref()),
            Op::SetStencil { id, stencil } => self.set_stencil(id, stencil.as_deref()),
            Op::SetArrow { id, arrow } => self.set_arrow(id, *arrow),
            Op::SetProp { id, key, value } => self.set_prop(id, key, value.as_ref()),
            Op::SetPlacement { id, placement } => self.set_placement(id, placement.as_ref()),
            Op::SetWaypoints { id, points } => self.set_waypoints(id, points),
            Op::AddStep { index, step } => vec![self.add_step(*index, step)],
            Op::RemoveStep { index } => self.remove_step(*index),
            Op::SetStep { index, step } => self.set_step(*index, step),
            Op::SetMembers { group, members } => self.set_members(group, members),
            Op::SetTitle { title } => self.set_title(title.as_deref()),
            Op::SetDiagramProp { key, value } => self.set_diagram_prop(key, value.as_ref()),
            Op::SetEdgePorts { id, from_port, to_port } => self.set_edge_ports(id, from_port.as_deref(), to_port.as_deref()),
            Op::AddGroup { group, index } => vec![self.add_group(group, *index)],
            Op::RemoveNode { .. } | Op::RemoveEdge { .. } | Op::RemoveGroup { .. } | Op::Batch(_) => unreachable!("handled in patch"),
        }
    }

    // ---- source helpers ----

    /// The full line(s) `span` sits on, including the trailing newline.
    fn line_range(&self, span: &Range<usize>) -> Range<usize> {
        let start = self.src[..span.start].rfind('\n').map_or(0, |i| i + 1);
        let end = self.src[span.end..].find('\n').map_or(self.src.len(), |i| span.end + i + 1);
        start..end
    }

    fn line_end(&self, pos: usize) -> usize {
        self.src[pos..].find('\n').map_or(self.src.len(), |i| pos + i + 1)
    }

    fn indent_at(&self, pos: usize) -> &str {
        let start = self.src[..pos].rfind('\n').map_or(0, |i| i + 1);
        let line = &self.src[start..];
        &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
    }

    /// Extend a removal range left over spaces so `a "x"` -> `a`, not `a `.
    fn eat_space_before(&self, range: Range<usize>) -> Range<usize> {
        let trimmed = self.src[..range.start].trim_end_matches([' ', '\t']).len();
        trimmed..range.end
    }

    /// Insert a full statement line after statement `after`, or at the end.
    fn insert_stmt_line(&self, after: Option<usize>, line: &str) -> Splice {
        match after {
            Some(si) => {
                let span = &self.file.stmts[si].span;
                let end = self.line_end(span.end);
                let indent = self.indent_at(span.start).to_string();
                let nl = if self.src[..end].ends_with('\n') { "" } else { "\n" };
                ins(end, format!("{nl}{indent}{line}\n"))
            }
            None => self.insert_before_layout(line),
        }
    }

    /// New statements go before the layout block so geometry stays last.
    fn insert_before_layout(&self, line: &str) -> Splice {
        if let Some(si) = self.index.layout {
            let start = self.line_range(&self.file.stmts[si].span).start;
            return ins(start, format!("{line}\n\n"));
        }
        let nl = if self.src.is_empty() || self.src.ends_with('\n') { "" } else { "\n" };
        ins(self.src.len(), format!("{nl}{line}\n"))
    }

    fn last_stmt(&self, f: impl Fn(&StmtKind) -> bool) -> Option<usize> {
        self.file.stmts.iter().rposition(|s| f(&s.kind))
    }

    fn edge_anchor(&self) -> Option<usize> {
        self.last_stmt(|k| matches!(k, StmtKind::Edge(_)))
            .or_else(|| self.last_stmt(|k| matches!(k, StmtKind::Node(_) | StmtKind::Group(_))))
    }

    fn add_edge(&self, edge: &graphing_model::Edge, index: usize) -> Splice {
        let line = fmt_edge(edge);
        let edges = &self.diagram.edges;
        if index == 0
            && let Some(first) = edges.first().and_then(|e| self.index.edges.get(&e.id))
        {
            let start = self.line_range(&self.file.stmts[first.0].span).start;
            let indent = self.indent_at(self.file.stmts[first.0].span.start).to_string();
            return ins(start, format!("{indent}{line}\n"));
        }
        let before = edges.iter().take(index).next_back().and_then(|e| self.index.edges.get(&e.id)).map(|e| e.0);
        self.insert_stmt_line(before.or_else(|| self.edge_anchor()), &line)
    }

    fn add_node(&self, node: &Node, index: usize) -> Splice {
        let line = fmt_node(node);
        // After the closest declared node at or before `index`.
        let before = self.diagram.nodes.iter().take(index).rev().find_map(|n| self.index.nodes.get(&n.id).copied());
        let anchor = before
            .or_else(|| self.last_stmt(|k| matches!(k, StmtKind::Node(_))))
            .or_else(|| self.last_stmt(|k| matches!(k, StmtKind::Diagram { .. } | StmtKind::Use { .. } | StmtKind::Style { .. })));
        match anchor {
            Some(si) => self.insert_stmt_line(Some(si), &line),
            None => {
                // Above the first edge or group if there is one.
                let first = self.file.stmts.iter().position(|s| matches!(s.kind, StmtKind::Edge(_) | StmtKind::Group(_)));
                match first {
                    Some(si) => ins(self.line_range(&self.file.stmts[si].span).start, format!("{line}\n")),
                    None => self.insert_before_layout(&line),
                }
            }
        }
    }

    /// Give an edge-implied node its own line so it survives text edits that
    /// would drop its last reference. Returns true if the text changed.
    fn materialize(&mut self, id: &str) -> bool {
        if self.index.nodes.contains_key(id) || self.diagram.node(id).is_none() {
            return false;
        }
        let index = self.diagram.nodes.len();
        let s = self.add_node(&Node::new(id), index);
        self.commit(vec![s]);
        true
    }

    // ---- labels and props ----

    fn set_label(&self, id: &str, label: Option<&str>) -> Vec<Splice> {
        let Some((current, anchor)) = self.label_slot(id) else {
            return match label {
                Some(l) => vec![self.add_node(&Node { label: Some(l.into()), ..Node::new(id) }, usize::MAX)],
                None => Vec::new(),
            };
        };
        match (current, label) {
            (Some(span), Some(l)) => vec![put(span, fmt_str(l))],
            (Some(span), None) => vec![put(self.eat_space_before(span), "")],
            (None, Some(l)) => {
                // `id` alone becomes `id: "label"`, the canonical form.
                let bare = self.index.nodes.get(id).is_some_and(|&si| {
                    matches!(&self.file.stmts[si].kind, StmtKind::Node(n) if n.stencil.is_none() && n.id.span.end == anchor)
                });
                let colon = if bare { ":" } else { "" };
                vec![ins(anchor, format!("{colon} {}", fmt_str(l)))]
            }
            (None, None) => Vec::new(),
        }
    }

    fn set_stencil(&self, id: &str, stencil: Option<&str>) -> Vec<Splice> {
        let Some(&si) = self.index.nodes.get(id) else {
            return match stencil {
                Some(st) => vec![self.add_node(&Node { stencil: Some(st.into()), ..Node::new(id) }, usize::MAX)],
                None => Vec::new(),
            };
        };
        let StmtKind::Node(n) = &self.file.stmts[si].kind else { return Vec::new() };
        match (&n.stencil, stencil) {
            (Some(cur), Some(st)) => vec![put(cur.span.clone(), st)],
            (Some(cur), None) if n.label.is_none() && n.classes.is_empty() && n.props.is_none() => {
                // `id: shape` -> `id`, no dangling colon.
                vec![put(n.id.span.end..cur.span.end, "")]
            }
            (Some(cur), None) => vec![put(self.eat_space_before(cur.span.clone()), "")],
            (None, None) => Vec::new(),
            (None, Some(st)) => {
                let after = &self.src[n.id.span.end..];
                let gap = after.len() - after.trim_start_matches([' ', '\t']).len();
                if after[gap..].starts_with(':') {
                    vec![ins(n.id.span.end + gap + 1, format!(" {st}"))]
                } else {
                    vec![ins(n.id.span.end, format!(": {st}"))]
                }
            }
        }
    }

    fn set_arrow(&self, id: &str, arrow: graphing_model::Arrow) -> Vec<Splice> {
        let Some(&(si, hop)) = self.index.edges.get(id) else { return Vec::new() };
        let StmtKind::Edge(e) = &self.file.stmts[si].kind else { return Vec::new() };
        vec![put(e.arrow_spans[hop].clone(), fmt_arrow(arrow))]
    }

    /// Current label span and where to insert one. `None` if `id` has no
    /// statement (implied node or unknown).
    fn label_slot(&self, id: &str) -> Option<(Option<Range<usize>>, usize)> {
        if let Some(&si) = self.index.nodes.get(id) {
            let StmtKind::Node(n) = &self.file.stmts[si].kind else { return None };
            let anchor = match &n.stencil {
                Some(s) => s.span.end,
                None => {
                    let after = &self.src[n.id.span.end..];
                    let gap = after.len() - after.trim_start_matches([' ', '\t']).len();
                    if after[gap..].starts_with(':') { n.id.span.end + gap + 1 } else { n.id.span.end }
                }
            };
            return Some((n.label.as_ref().map(|l| l.span.clone()), anchor));
        }
        if let Some(&(si, _)) = self.index.edges.get(id) {
            let StmtKind::Edge(e) = &self.file.stmts[si].kind else { return None };
            let anchor = e.chain.last()?.span.end;
            return Some((e.label.as_ref().map(|l| l.span.clone()), anchor));
        }
        if let Some(&si) = self.index.groups.get(id) {
            let StmtKind::Group(g) = &self.file.stmts[si].kind else { return None };
            return Some((g.label.as_ref().map(|l| l.span.clone()), g.id.span.end));
        }
        None
    }

    fn prop_slot(&self, id: &str) -> Option<(Option<&PropBlock>, usize)> {
        let si = self
            .index
            .nodes
            .get(id)
            .or_else(|| self.index.groups.get(id))
            .copied()
            .or_else(|| self.index.edges.get(id).map(|e| e.0))?;
        let stmt = &self.file.stmts[si];
        let block = match &stmt.kind {
            StmtKind::Node(n) => n.props.as_ref(),
            StmtKind::Edge(e) => e.props.as_ref(),
            StmtKind::Group(g) => g.props.as_ref(),
            _ => return None,
        };
        Some((block, stmt.span.end))
    }

    fn set_prop(&self, id: &str, key: &str, value: Option<&Value>) -> Vec<Splice> {
        let Some((block, end)) = self.prop_slot(id) else {
            return match value {
                Some(v) => {
                    let node = Node { props: vec![(key.into(), v.clone())], ..Node::new(id) };
                    vec![self.add_node(&node, usize::MAX)]
                }
                None => Vec::new(),
            };
        };
        let Some(block) = block else {
            return match value {
                Some(v) => vec![ins(end, format!(" {}", fmt_props(&[(key.into(), v.clone())])))],
                None => Vec::new(),
            };
        };
        self.edit_block(block, key, value)
    }

    /// Set, replace or remove `key` inside an existing `{ ... }` block.
    fn edit_block(&self, block: &PropBlock, key: &str, value: Option<&Value>) -> Vec<Splice> {
        let found = block.entries.iter().rposition(|e| e.key.value == key);
        match (found, value) {
            (Some(i), Some(v)) => vec![put(block.entries[i].value.span.clone(), fmt_value(v))],
            (Some(i), None) => vec![self.remove_prop(block, i)],
            (None, Some(v)) => {
                let entry = format!("{key}: {}", fmt_value(v));
                match block.entries.last() {
                    None => vec![put(block.span.clone(), format!("{{ {entry} }}"))],
                    Some(last) => {
                        let multiline = self.src[block.span.clone()].contains('\n');
                        let sep = if multiline {
                            format!("\n{}", self.indent_at(last.key.span.start))
                        } else {
                            ", ".to_string()
                        };
                        vec![ins(last.value.span.end, format!("{sep}{entry}"))]
                    }
                }
            }
            (None, None) => Vec::new(),
        }
    }

    fn remove_prop(&self, block: &PropBlock, i: usize) -> Splice {
        let e = &block.entries[i];
        if block.entries.len() == 1 {
            return put(self.eat_space_before(block.span.clone()), "");
        }
        if i > 0 {
            // From the end of the previous value, so its separator goes too.
            return put(block.entries[i - 1].value.span.end..e.value.span.end, "");
        }
        put(e.key.span.start..block.entries[1].key.span.start, "")
    }

    // ---- layout ----

    fn animate_block(&self) -> Option<&crate::ast::AnimateBlock> {
        match &self.file.stmts[self.index.animate?].kind {
            StmtKind::Animate(b) => Some(b),
            _ => None,
        }
    }

    /// A new step before step `index`, or last; a new block at the end of
    /// the file when there is none.
    fn add_step(&self, index: usize, step: &graphing_model::Step) -> Splice {
        let Some(block) = self.animate_block() else {
            let nl = match self.src.as_str() {
                "" => "",
                s if s.ends_with("\n\n") => "",
                s if s.ends_with('\n') => "\n",
                _ => "\n\n",
            };
            return ins(self.src.len(), format!("{nl}animate {{\n  {}\n}}\n", fmt_step(step, "  ")));
        };
        if let Some(next) = block.steps.get(index) {
            let indent = self.indent_at(next.span.start).to_string();
            let start = self.line_range(&next.span).start;
            return ins(start, format!("{indent}{}\n", fmt_step(step, &indent)));
        }
        let indent = match block.steps.last() {
            Some(st) => self.indent_at(st.span.start).to_string(),
            None => format!("{}  ", self.indent_at(block.open.start)),
        };
        let close_line = self.line_range(&block.close).start;
        if self.src[close_line..block.close.start].trim().is_empty() && close_line > block.open.end {
            ins(close_line, format!("{indent}{}\n", fmt_step(step, &indent)))
        } else {
            let outer = self.indent_at(block.open.start).to_string();
            ins(block.close.start, format!("\n{indent}{}\n{outer}", fmt_step(step, &indent)))
        }
    }

    fn remove_step(&self, index: usize) -> Vec<Splice> {
        let Some(st) = self.animate_block().and_then(|b| b.steps.get(index)) else { return Vec::new() };
        vec![put(self.line_range(&st.span), "")]
    }

    fn set_step(&self, index: usize, step: &graphing_model::Step) -> Vec<Splice> {
        let Some(st) = self.animate_block().and_then(|b| b.steps.get(index)) else { return Vec::new() };
        let indent = self.indent_at(st.span.start).to_string();
        vec![put(st.span.clone(), fmt_step(step, &indent))]
    }

    fn layout_block(&self) -> Option<&crate::ast::LayoutBlock> {
        match &self.file.stmts[self.index.layout?].kind {
            StmtKind::Layout(b) => Some(b),
            _ => None,
        }
    }

    fn entry(&self, key: &str) -> Option<&LayoutEntry> {
        Some(&self.layout_block()?.entries[*self.index.entries.get(key)?])
    }

    fn insert_layout_line(&self, line: &str) -> Splice {
        let Some(block) = self.layout_block() else {
            let nl = match self.src.as_str() {
                "" => "",
                s if s.ends_with("\n\n") => "",
                s if s.ends_with('\n') => "\n",
                _ => "\n\n",
            };
            return ins(self.src.len(), format!("{nl}layout {{\n  {line}\n}}\n"));
        };
        let indent = match block.entries.last() {
            Some(e) => self.indent_at(e.span.start).to_string(),
            None => format!("{}  ", self.indent_at(block.open.start)),
        };
        let close_line = self.line_range(&block.close).start;
        if self.src[close_line..block.close.start].trim().is_empty() && close_line > block.open.end {
            ins(close_line, format!("{indent}{line}\n"))
        } else {
            // `layout {}` or `... }` on one line.
            let outer = self.indent_at(block.open.start).to_string();
            ins(block.close.start, format!("\n{indent}{line}\n{outer}"))
        }
    }

    fn set_placement(&self, id: &str, placement: Option<&Placement>) -> Vec<Splice> {
        let entry = self.entry(id);
        let Some(p) = placement else {
            return match entry {
                Some(e) if e.via.is_empty() => vec![put(self.line_range(&e.span), "")],
                _ => Vec::new(),
            };
        };
        let mut geom = format!("{} {}", fmt_num(p.pos.x), fmt_num(p.pos.y));
        if let Some(s) = p.size {
            geom.push_str(&format!(" {}x{}", fmt_num(s.w), fmt_num(s.h)));
        }
        match entry {
            Some(LayoutEntry { geom: Some(span), .. }) => vec![put(span.clone(), geom)],
            Some(e) => vec![ins(e.span.end, format!(" {geom}"))],
            None => vec![self.insert_layout_line(&format!("{id} {geom}"))],
        }
    }

    fn set_waypoints(&self, id: &str, points: &[Point]) -> Vec<Splice> {
        let via = || {
            let pts: Vec<String> = points.iter().map(|p| format!("{} {}", fmt_num(p.x), fmt_num(p.y))).collect();
            format!("via {}", pts.join(", "))
        };
        match self.entry(id) {
            Some(e) => match (&e.via_span, points.is_empty()) {
                (Some(_), true) if e.geom.is_none() => vec![put(self.line_range(&e.span), "")],
                (Some(span), true) => vec![put(self.eat_space_before(span.clone()), "")],
                (Some(span), false) => vec![put(span.clone(), via())],
                (None, true) => Vec::new(),
                (None, false) => vec![ins(e.span.end, format!(" {}", via()))],
            },
            None if points.is_empty() => Vec::new(),
            None => {
                let Some(edge) = self.diagram.edge(id) else { return Vec::new() };
                let target = if id == format!("{}->{}", edge.from, edge.to) {
                    format!("{} -> {}", edge.from, edge.to)
                } else if id.contains("->") {
                    // Parallel unnamed edge: no way to address it yet.
                    return Vec::new();
                } else {
                    id.to_string()
                };
                vec![self.insert_layout_line(&format!("{target} {}", via()))]
            }
        }
    }

    fn diagram_stmt(&self) -> Option<usize> {
        self.file.stmts.iter().position(|s| matches!(s.kind, StmtKind::Diagram { .. }))
    }

    /// New `diagram` line at the top, above everything but leading comments.
    fn insert_diagram_line(&self, line: &str) -> Splice {
        let first = self.file.stmts.first().map_or(self.src.len(), |s| self.line_range(&s.span).start);
        ins(first, format!("{line}\n"))
    }

    fn set_title(&self, title: Option<&str>) -> Vec<Splice> {
        let Some(si) = self.diagram_stmt() else {
            return title.map(|t| vec![self.insert_diagram_line(&format!("diagram {}", fmt_str(t)))]).unwrap_or_default();
        };
        let StmtKind::Diagram { title: cur, .. } = &self.file.stmts[si].kind else { return Vec::new() };
        let kw_end = self.file.stmts[si].span.start + "diagram".len();
        match (cur, title) {
            (Some(c), Some(t)) => vec![put(c.span.clone(), fmt_str(t))],
            (Some(c), None) => vec![put(self.eat_space_before(c.span.clone()), "")],
            (None, Some(t)) => vec![ins(kw_end, format!(" {}", fmt_str(t)))],
            (None, None) => Vec::new(),
        }
    }

    fn set_diagram_prop(&self, key: &str, value: Option<&Value>) -> Vec<Splice> {
        let Some(si) = self.diagram_stmt() else {
            return value.map(|v| vec![self.insert_diagram_line(&format!("diagram {}", fmt_props(&[(key.into(), v.clone())])))]).unwrap_or_default();
        };
        let stmt = &self.file.stmts[si];
        let StmtKind::Diagram { props, .. } = &stmt.kind else { return Vec::new() };
        let Some(block) = props else {
            return value.map(|v| vec![ins(stmt.span.end, format!(" {}", fmt_props(&[(key.into(), v.clone())])))]).unwrap_or_default();
        };
        self.edit_block(block, key, value)
    }

    fn set_edge_ports(&self, id: &str, from_port: Option<&str>, to_port: Option<&str>) -> Vec<Splice> {
        let (Some(&(si, hop)), Some(edge)) = (self.index.edges.get(id), self.diagram.edge(id)) else { return Vec::new() };
        let StmtKind::Edge(e) = &self.file.stmts[si].kind else { return Vec::new() };
        let end = |node: &str, port: Option<&str>| port.map_or(node.to_string(), |p| format!("{node}.{p}"));
        // A chain shares middle endpoints between hops; only single-hop
        // statements can change ports without touching another edge.
        if e.arrows.len() != 1 {
            return Vec::new();
        }
        vec![put(e.chain[hop].span.clone(), end(&edge.from, from_port)), put(e.chain[hop + 1].span.clone(), end(&edge.to, to_port))]
    }

    fn add_group(&self, g: &graphing_model::Group, index: usize) -> Splice {
        let label = g.label.as_ref().map(|l| format!(" {}", fmt_str(l))).unwrap_or_default();
        let props = if g.props.is_empty() { String::new() } else { format!(" {}", fmt_props(&g.props)) };
        let line = format!("group {}{label} {{ {} }}{props}", g.id, g.members.join(" "));
        let groups = &self.diagram.groups;
        if index == 0
            && let Some(&si) = groups.first().and_then(|g| self.index.groups.get(&g.id))
        {
            let start = self.line_range(&self.file.stmts[si].span).start;
            return ins(start, format!("{line}\n"));
        }
        if let Some(&si) = groups.iter().take(index).next_back().and_then(|g| self.index.groups.get(&g.id)) {
            return self.insert_stmt_line(Some(si), &line);
        }
        let anchor = self
            .last_stmt(|k| matches!(k, StmtKind::Group(_)))
            .or_else(|| self.last_stmt(|k| matches!(k, StmtKind::Node(_))));
        self.insert_stmt_line(anchor, &line)
    }

    fn remove_group(&mut self, id: &str) {
        let parents: Vec<(String, Vec<String>)> = self
            .diagram
            .groups
            .iter()
            .filter(|g| g.members.iter().any(|m| m == id))
            .map(|g| (g.id.clone(), g.members.iter().filter(|m| *m != id).cloned().collect()))
            .collect();
        for (g, members) in parents {
            let s = self.set_members(&g, &members);
            self.commit(s);
        }
        if let Some(e) = self.entry(id) {
            let r = self.line_range(&e.span);
            self.commit(vec![put(r, "")]);
        }
        if let Some(&si) = self.index.groups.get(id) {
            let r = self.line_range(&self.file.stmts[si].span);
            self.commit(vec![put(r, "")]);
        }
    }

    fn set_members(&self, group: &str, members: &[String]) -> Vec<Splice> {
        let Some(&si) = self.index.groups.get(group) else { return Vec::new() };
        let StmtKind::Group(g) = &self.file.stmts[si].kind else { return Vec::new() };
        let body = if members.is_empty() { "{}".to_string() } else { format!("{{ {} }}", members.join(" ")) };
        vec![put(g.body.clone(), body)]
    }

    // ---- removal ----

    fn remove_edge(&mut self, id: &str) {
        let Some(edge) = self.diagram.edge(id).cloned() else { return };
        // Keep implied endpoints alive once this reference is gone.
        self.materialize(&edge.from);
        self.materialize(&edge.to);
        // Edge entries only carry waypoints, so the whole line goes.
        if let Some(e) = self.entry(id) {
            let r = self.line_range(&e.span);
            self.commit(vec![put(r, "")]);
        }
        let Some(&(si, hop)) = self.index.edges.get(id) else { return };
        let stmt = &self.file.stmts[si];
        let StmtKind::Edge(e) = &stmt.kind else { return };
        if e.arrows.len() == 1 {
            let r = self.line_range(&stmt.span);
            self.commit(vec![put(r, "")]);
            return;
        }
        // Split the chain around the removed hop; each piece keeps the
        // original label, classes and props text.
        let suffix = &self.src[e.chain.last().expect("chain").span.end..stmt.span.end];
        let indent = self.indent_at(stmt.span.start);
        let mut lines = Vec::new();
        for run in [0..hop, hop + 1..e.arrows.len()] {
            if run.is_empty() {
                continue;
            }
            let mut s = e.chain[run.start].value.clone();
            for h in run {
                s.push_str(&format!(" {} {}", fmt_arrow(e.arrows[h]), e.chain[h + 1].value));
            }
            s.push_str(suffix);
            lines.push(s);
        }
        let text = lines.join(&format!("\n{indent}"));
        let span = stmt.span.clone();
        self.commit(vec![put(span, text)]);
    }

    fn remove_node(&mut self, id: &str) {
        self.materialize(id);
        let edges: Vec<String> =
            self.diagram.edges.iter().filter(|e| e.from == id || e.to == id).map(|e| e.id.clone()).collect();
        // Remove from the back so generated `a->b#n` keys of the rest stay put.
        for e in edges.iter().rev() {
            self.remove_edge(e);
        }
        let groups: Vec<(String, Vec<String>)> = self
            .diagram
            .groups
            .iter()
            .filter(|g| g.members.iter().any(|m| m == id))
            .map(|g| (g.id.clone(), g.members.iter().filter(|m| *m != id).cloned().collect()))
            .collect();
        for (g, members) in groups {
            let s = self.set_members(&g, &members);
            self.commit(s);
        }
        if let Some(e) = self.entry(id) {
            let r = self.line_range(&e.span);
            self.commit(vec![put(r, "")]);
        }
        if let Some(&si) = self.index.nodes.get(id) {
            let r = self.line_range(&self.file.stmts[si].span);
            self.commit(vec![put(r, "")]);
        }
    }
}
