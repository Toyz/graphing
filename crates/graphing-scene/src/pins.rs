//! Node-graph pins, Blueprint style: a node lists typed inputs and outputs
//! (`{ in: [exec, a: float], out: [exec, sum: float] }`), inputs on the side
//! flow comes from and outputs opposite, wires curving between them. Here
//! too are the checks a node graph wants: wires run from outputs to inputs
//! of the same type, a data input takes one wire, an exec output leads one
//! way, and an acyclic diagram has no loops.

use std::collections::HashMap;

use graphing_model::{Diagram, Node, Point, Rect, Value};

use crate::notation::{Flow, TITLE_CHAR};

/// Which way a pin carries flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PinDir {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub name: String,
    /// `float`, `exec`, a struct name; `None` takes anything.
    pub ty: Option<String>,
    pub dir: PinDir,
    /// Where the node puts it (`in_side`, `out_side`, `sides: [a: top]`);
    /// `None` for the default, inputs facing the inflow.
    pub side: Option<crate::Side>,
}

impl Pin {
    /// Execution order rather than data: `exec`, or typed `exec`.
    pub fn exec(&self) -> bool {
        self.ty.as_deref() == Some("exec") || self.ty.is_none() && self.name == "exec"
    }

    /// Text beside the pin; a lone `exec` pin goes unnamed, as in Blueprint.
    pub fn label(&self) -> &str {
        if self.exec() && self.name == "exec" { "" } else { &self.name }
    }
}

/// Header band above the pin rows (the node's title).
pub const PIN_TOP: f64 = 34.0;
pub const PIN_ROW: f64 = 22.0;
/// Pin dot radius.
pub const PIN_R: f64 = 5.0;
/// Pin label size and width per character.
pub const PIN_PT: f64 = 11.0;
const PIN_CHAR: f64 = 6.4;

fn side_named(name: &str) -> Option<crate::Side> {
    Some(match name {
        "left" => crate::Side::Left,
        "right" => crate::Side::Right,
        "top" => crate::Side::Top,
        "bottom" => crate::Side::Bottom,
        _ => return None,
    })
}

/// The node's pins: `in` then `out`, each item `name` or `name: type`.
/// `in_side: top` (or `out_side`) moves a whole list; `sides: [a: bottom]`
/// one pin.
pub fn pins(d: &Diagram, n: &Node) -> Vec<Pin> {
    let mut out = Vec::new();
    let text = |key: &str| d.node_prop(n, key).map(Value::text);
    let one: HashMap<String, crate::Side> = d
        .node_prop(n, "sides")
        .and_then(Value::as_list)
        .unwrap_or_default()
        .iter()
        .filter_map(|it| {
            let t = it.text();
            let (name, side) = t.split_once(':')?;
            Some((name.trim().to_string(), side_named(side.trim())?))
        })
        .collect();
    for (key, dir) in [("in", PinDir::In), ("out", PinDir::Out)] {
        let list_side = text(&format!("{key}_side")).as_deref().and_then(side_named);
        // The node's own list, else its stencil's (a branch comes wired).
        let fallback = || graphing_model::find_prop(&crate::stencils::registry().resolve(n.stencil.as_deref()).defaults, key).cloned();
        let Some(list) = d.node_prop(n, key).cloned().or_else(fallback) else { continue };
        let Some(items) = list.as_list() else { continue };
        for it in items {
            let (name, ty) = match it {
                Value::Pair(name, ty) => (name.clone(), Some(ty.text())),
                other => {
                    let text = other.text();
                    match text.split_once(':') {
                        Some((n, t)) => (n.trim().to_string(), Some(t.trim().to_string())),
                        None => (text.trim().to_string(), None),
                    }
                }
            };
            if !name.is_empty() {
                let side = one.get(&name).copied().or(list_side);
                out.push(Pin { name, ty: ty.filter(|t| !t.is_empty()), dir, side });
            }
        }
    }
    out
}

/// The side `p` sits on: its own, else inputs facing the inflow and
/// outputs opposite.
pub fn side_of(p: &Pin, flow: Flow) -> crate::Side {
    p.side.unwrap_or(match (p.dir, flow) {
        (PinDir::In, Flow::Down) => crate::Side::Top,
        (PinDir::Out, Flow::Down) => crate::Side::Bottom,
        (PinDir::In, _) => crate::Side::Left,
        (PinDir::Out, _) => crate::Side::Right,
    })
}

/// Room for the title and the pins: rows down the left and right under
/// the header, and pins spread along the top and bottom.
pub fn size(title: &str, pins: &[Pin], flow: Flow) -> (f64, f64) {
    let snap = |v: f64| (v / 10.0).ceil() * 10.0;
    let on = |side: crate::Side| pins.iter().filter(move |p| side_of(p, flow) == side);
    let widest = |side| on(side).map(|p| p.label().chars().count()).max().unwrap_or(0) as f64 * PIN_CHAR;
    let count = |side| on(side).count() as f64;
    let (l, r, t, b) = (crate::Side::Left, crate::Side::Right, crate::Side::Top, crate::Side::Bottom);
    let per = (widest(t).max(widest(b)) + 24.0).max(36.0);
    let w = (title.chars().count() as f64 * TITLE_CHAR + 40.0).max(widest(l) + widest(r) + 64.0).max(count(t).max(count(b)) * per + 24.0).max(140.0);
    let rows = count(l).max(count(r));
    // Pins along the bottom need room for their names inside; names of pins
    // along the top sit above the node, clear of the title.
    let ends = if count(b) > 0.0 { PIN_ROW } else { 0.0 };
    let h = PIN_TOP + rows.max(if ends > 0.0 { 0.0 } else { 1.0 }) * PIN_ROW + ends + 8.0;
    (snap(w), snap(h.max(PIN_TOP + PIN_ROW + 8.0)))
}

/// Where each pin sits on `r`'s border, in `pins` order: rows under the
/// header on the left and right, spread evenly along the top and bottom.
pub fn place(r: Rect, pins: &[Pin], flow: Flow) -> Vec<(Point, crate::Side)> {
    let mut index: HashMap<crate::Side, usize> = HashMap::new();
    pins.iter()
        .map(|p| {
            let side = side_of(p, flow);
            let i = index.entry(side).or_insert(0);
            let k = *i as f64;
            *i += 1;
            let n = pins.iter().filter(|q| side_of(q, flow) == side).count() as f64;
            let along = |len: f64| len * (k + 1.0) / (n + 1.0);
            let row = r.origin.y + PIN_TOP + (k + 0.5) * PIN_ROW;
            let at = match side {
                crate::Side::Left => Point::new(r.origin.x, row),
                crate::Side::Right => Point::new(r.origin.x + r.size.w, row),
                crate::Side::Top => Point::new(r.origin.x + along(r.size.w), r.origin.y),
                crate::Side::Bottom => Point::new(r.origin.x + along(r.size.w), r.origin.y + r.size.h),
            };
            (at, side)
        })
        .collect()
}

/// A wire from `a` leaving toward `sa` to `b` arriving from `sb`, as a
/// smooth curve flattened into short segments.
pub fn wire(a: Point, sa: Option<crate::Side>, b: Point, sb: Option<crate::Side>) -> Vec<Point> {
    let reach = ((b.x - a.x).abs().max((b.y - a.y).abs()) * 0.5).clamp(40.0, 160.0);
    let out = |s: Option<crate::Side>, toward: Point, from: Point| match s {
        Some(crate::Side::Right) => Point::new(from.x + reach, from.y),
        Some(crate::Side::Left) => Point::new(from.x - reach, from.y),
        Some(crate::Side::Bottom) => Point::new(from.x, from.y + reach),
        Some(crate::Side::Top) => Point::new(from.x, from.y - reach),
        // No pin: aim straight at the other end.
        None => Point::new(from.x + (toward.x - from.x) * 0.3, from.y + (toward.y - from.y) * 0.3),
    };
    let (c1, c2) = (out(sa, b, a), out(sb, a, b));
    const STEPS: usize = 24;
    (0..=STEPS)
        .map(|i| {
            let t = i as f64 / STEPS as f64;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            Point::new(w0 * a.x + w1 * c1.x + w2 * c2.x + w3 * b.x, w0 * a.y + w1 * c1.y + w2 * c2.y + w3 * b.y)
        })
        .collect()
}

/// A pin type's color, after Blueprint's where it has one. Execution pins
/// draw in the theme's line color instead (Blueprint's white is for dark
/// backgrounds).
pub fn color(ty: Option<&str>) -> u32 {
    match ty.map(str::to_ascii_lowercase).as_deref() {
        None | Some("any" | "wildcard") => 0x9aa0a6,
        Some("exec") => 0x868e96,
        Some("bool" | "boolean") => 0xb3261e,
        Some("int" | "integer" | "byte") => 0x1dc9a8,
        Some("float" | "double" | "number" | "real") => 0x8bc34a,
        Some("string" | "text" | "name") => 0xe040fb,
        Some("vector" | "vec3" | "vec2") => 0xfbc02d,
        Some("rotator" | "transform") => 0x9fa8da,
        Some("object" | "actor" | "ref") => 0x1e88e5,
        Some(other) => {
            // Anything else gets a steady color of its own.
            const SET: [u32; 6] = [0x26a69a, 0x7e57c2, 0xef6c00, 0x5c6bc0, 0xd81b60, 0x43a047];
            let h = other.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
            SET[h as usize % SET.len()]
        }
    }
}

/// Something wrong with a wire or a node, for the Problems list.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The edge or node it is about.
    pub id: String,
    pub message: String,
}

/// Whether the diagram must have no loops: `acyclic: true`, else its kind.
pub fn acyclic(d: &Diagram) -> bool {
    match d.prop("acyclic").map(Value::text).as_deref() {
        Some("true" | "yes") => true,
        Some(_) => false,
        None => d.prop("kind").map(Value::text).is_some_and(|k| crate::stencils::registry().diagram_kind_in(&k, &d.packs).is_some_and(|k| k.acyclic)),
    }
}

/// Every problem with how `d` is wired.
pub fn problems(d: &Diagram) -> Vec<Problem> {
    let mut out = Vec::new();
    let pins_of: HashMap<&str, Vec<Pin>> = d.nodes.iter().map(|n| (n.id.as_str(), pins(d, n))).filter(|(_, p)| !p.is_empty()).collect();
    // Where an input and an output share a name (`exec`), a wire's source
    // end means the output and its target end the input.
    let find = |node: &str, port: &Option<String>, dir: PinDir| -> Option<Result<&Pin, String>> {
        let pins = pins_of.get(node)?;
        let port = port.as_deref()?;
        let named = |p: &&Pin| p.name == port;
        Some(pins.iter().filter(named).find(|p| p.dir == dir).or_else(|| pins.iter().find(named)).ok_or_else(|| format!("`{node}` has no pin `{port}`")))
    };
    // How many wires reach each (node, pin), to catch doubled inputs.
    let mut into: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut from: HashMap<(String, String), Vec<String>> = HashMap::new();
    for e in &d.edges {
        let (from_dir, to_dir) = if e.arrow == graphing_model::Arrow::Back { (PinDir::In, PinDir::Out) } else { (PinDir::Out, PinDir::In) };
        let (a, b) = (find(&e.from, &e.from_port, from_dir), find(&e.to, &e.to_port, to_dir));
        for r in [&a, &b] {
            if let Some(Err(m)) = r {
                out.push(Problem { id: e.id.clone(), message: m.clone() });
            }
        }
        let (Some(Ok(pa)), Some(Ok(pb))) = (a, b) else { continue };
        // `<-` runs the other way; `--` and `<->` say nothing about direction.
        let (src, dst, src_end, dst_end) = match e.arrow {
            graphing_model::Arrow::Back => (pb, pa, (&e.to, &e.to_port), (&e.from, &e.from_port)),
            graphing_model::Arrow::Forward => (pa, pb, (&e.from, &e.from_port), (&e.to, &e.to_port)),
            _ => continue,
        };
        let end = |(n, p): (&String, &Option<String>)| format!("{n}.{}", p.as_deref().unwrap_or_default());
        if src.dir != PinDir::Out {
            out.push(Problem { id: e.id.clone(), message: format!("`{}` is an input; wires leave from outputs", end(src_end)) });
            continue;
        }
        if dst.dir != PinDir::In {
            out.push(Problem { id: e.id.clone(), message: format!("`{}` is an output; wires arrive at inputs", end(dst_end)) });
            continue;
        }
        if src.exec() != dst.exec() {
            out.push(Problem { id: e.id.clone(), message: format!("`{}` and `{}` mix execution and data", end(src_end), end(dst_end)) });
            continue;
        }
        let loose = |t: &Option<String>| t.as_deref().is_none_or(|t| matches!(t, "any" | "wildcard"));
        if !src.exec() && !loose(&src.ty) && !loose(&dst.ty) && src.ty != dst.ty {
            out.push(Problem {
                id: e.id.clone(),
                message: format!("`{}` gives {} but `{}` takes {}", end(src_end), src.ty.as_deref().unwrap_or_default(), end(dst_end), dst.ty.as_deref().unwrap_or_default()),
            });
            continue;
        }
        let key = |(n, p): (&String, &Option<String>)| (n.clone(), p.clone().unwrap_or_default());
        if src.exec() {
            from.entry(key(src_end)).or_default().push(e.id.clone());
        } else {
            into.entry(key(dst_end)).or_default().push(e.id.clone());
        }
    }
    for ((node, pin), wires) in into.iter().filter(|(_, w)| w.len() > 1) {
        for id in &wires[1..] {
            out.push(Problem { id: id.clone(), message: format!("`{node}.{pin}` takes one wire") });
        }
    }
    for ((node, pin), wires) in from.iter().filter(|(_, w)| w.len() > 1) {
        for id in &wires[1..] {
            out.push(Problem { id: id.clone(), message: format!("`{node}.{pin}` leads one way; add a sequence to branch") });
        }
    }
    if acyclic(d) {
        out.extend(loops(d, &pins_of));
    }
    out.sort_by(|a, b| a.id.cmp(&b.id).then(a.message.cmp(&b.message)));
    out.dedup();
    out
}

/// Edges on a loop, ignoring execution wires (Blueprint lets those loop).
fn loops(d: &Diagram, pins_of: &HashMap<&str, Vec<Pin>>) -> Vec<Problem> {
    let exec = |node: &str, port: &Option<String>| port.as_deref().is_some_and(|p| pins_of.get(node).is_some_and(|ps| ps.iter().any(|q| q.name == p && q.exec())));
    let arcs: Vec<(&str, &str, &str)> = d
        .edges
        .iter()
        .filter(|e| !exec(&e.from, &e.from_port) && !exec(&e.to, &e.to_port))
        .filter_map(|e| match e.arrow {
            graphing_model::Arrow::Forward => Some((e.from.as_str(), e.to.as_str(), e.id.as_str())),
            graphing_model::Arrow::Back => Some((e.to.as_str(), e.from.as_str(), e.id.as_str())),
            _ => None,
        })
        .collect();
    // Strongly connected components (Tarjan); an arc inside one is on a loop.
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for &(a, b, _) in &arcs {
        next.entry(a).or_default().push(b);
    }
    struct Walk<'a> {
        next: &'a HashMap<&'a str, Vec<&'a str>>,
        index: HashMap<&'a str, usize>,
        low: HashMap<&'a str, usize>,
        stack: Vec<&'a str>,
        on: std::collections::HashSet<&'a str>,
        comp: HashMap<&'a str, usize>,
        count: usize,
    }
    impl<'a> Walk<'a> {
        fn visit(&mut self, v: &'a str) {
            let i = self.index.len();
            self.index.insert(v, i);
            self.low.insert(v, i);
            self.stack.push(v);
            self.on.insert(v);
            for &w in self.next.get(v).map(Vec::as_slice).unwrap_or_default() {
                if !self.index.contains_key(w) {
                    self.visit(w);
                    let lw = self.low[w];
                    let lv = self.low.get_mut(v).expect("visited");
                    *lv = (*lv).min(lw);
                } else if self.on.contains(w) {
                    let iw = self.index[w];
                    let lv = self.low.get_mut(v).expect("visited");
                    *lv = (*lv).min(iw);
                }
            }
            if self.low[v] == self.index[v] {
                while let Some(w) = self.stack.pop() {
                    self.on.remove(w);
                    self.comp.insert(w, self.count);
                    if w == v {
                        break;
                    }
                }
                self.count += 1;
            }
        }
    }
    let mut walk = Walk { next: &next, index: HashMap::new(), low: HashMap::new(), stack: Vec::new(), on: Default::default(), comp: HashMap::new(), count: 0 };
    let starts: Vec<&str> = arcs.iter().map(|a| a.0).collect();
    for v in starts {
        if !walk.index.contains_key(v) {
            walk.visit(v);
        }
    }
    arcs.iter()
        .filter(|(a, b, _)| a == b || walk.comp.get(a) == walk.comp.get(b))
        .map(|(a, b, id)| Problem { id: id.to_string(), message: format!("`{a} -> {b}` closes a loop in an acyclic diagram") })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphing_dsl::Document;

    fn problems_of(src: &str) -> Vec<String> {
        let doc = Document::parse(src);
        assert!(doc.diags().is_empty(), "{:?}", doc.diags());
        problems(doc.diagram()).into_iter().map(|p| format!("{}: {}", p.id, p.message)).collect()
    }

    #[test]
    fn pins_come_from_the_node_or_its_stencil() {
        let doc = Document::parse("use graph\nadd { in: [exec, a: float, \"b: float\"], out: [exec, sum: float] }\nif: graph.branch\n");
        let d = doc.diagram();
        let add = pins(d, d.node("add").unwrap());
        assert_eq!(add.iter().map(|p| (p.name.as_str(), p.ty.as_deref(), p.dir)).collect::<Vec<_>>(), [
            ("exec", None, PinDir::In),
            ("a", Some("float"), PinDir::In),
            ("b", Some("float"), PinDir::In),
            ("exec", None, PinDir::Out),
            ("sum", Some("float"), PinDir::Out),
        ]);
        assert!(add[0].exec() && add[0].label().is_empty());
        let branch = pins(d, d.node("if").unwrap());
        assert_eq!(branch.iter().filter(|p| p.exec()).count(), 3);
        assert!(branch.iter().any(|p| p.name == "condition" && p.ty.as_deref() == Some("bool")));
    }

    #[test]
    fn inputs_face_the_inflow() {
        let list = vec![
            Pin { name: "a".into(), ty: None, dir: PinDir::In, side: None },
            Pin { name: "b".into(), ty: None, dir: PinDir::In, side: None },
            Pin { name: "out".into(), ty: None, dir: PinDir::Out, side: None },
        ];
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        let right = place(r, &list, Flow::Right);
        assert_eq!(right[0], (Point::new(0.0, PIN_TOP + PIN_ROW / 2.0), crate::Side::Left));
        assert_eq!(right[1].0.y, PIN_TOP + PIN_ROW * 1.5);
        assert_eq!(right[2], (Point::new(200.0, PIN_TOP + PIN_ROW / 2.0), crate::Side::Right));
        let down = place(r, &list, Flow::Down);
        assert_eq!(down[0].1, crate::Side::Top);
        assert_eq!(down[2], (Point::new(100.0, 100.0), crate::Side::Bottom));
    }

    const GRAPH: &str = "diagram { kind: graph }\nuse graph\nbegin: graph.event \"Begin\"\nx: graph.variable { out: [value: float] }\nflag: graph.variable { out: [value: bool] }\nadd: graph.pure { in: [a: float, b: float], out: [sum: float] }\nlog: graph.function { in: [exec, text: string] }\n";

    #[test]
    fn a_well_wired_graph_has_no_problems() {
        let src = format!("{GRAPH}begin.exec -> log.exec\nx.value -> add.a\nx.value -> add.b\n");
        assert_eq!(problems_of(&src), Vec::<String>::new());
    }

    #[test]
    fn wiring_mistakes_are_named() {
        let cases = [
            ("add.sum -> x.value", "is an output; wires arrive at inputs"),
            ("add.a -> add.b", "is an input; wires leave from outputs"),
            ("flag.value -> add.a", "gives bool but `add.a` takes float"),
            ("begin.exec -> add.a", "mix execution and data"),
            ("x.value -> add.c", "`add` has no pin `c`"),
        ];
        for (wire, want) in cases {
            let found = problems_of(&format!("{GRAPH}{wire}\n"));
            assert!(found.iter().any(|p| p.contains(want)), "{wire}: {found:?}");
        }
        // `<-` is read the other way round.
        assert_eq!(problems_of(&format!("{GRAPH}add.a <- x.value\n")), Vec::<String>::new());
        // One wire per data input; one way out of an exec pin.
        let found = problems_of(&format!("{GRAPH}x.value -> add.a\nflag.value -> add.a {{ }}\nx.value -> add.b\n"));
        assert!(found.iter().any(|p| p.contains("bool")), "{found:?}");
        let found = problems_of(&format!("{GRAPH}x.value -> add.a\nx2: graph.variable {{ out: [value: float] }}\nx2.value -> add.a\n"));
        assert!(found.iter().any(|p| p.contains("`add.a` takes one wire")), "{found:?}");
        let found = problems_of(&format!("{GRAPH}l2: graph.function\nbegin.exec -> log.exec\nbegin.exec -> l2.exec\n"));
        assert!(found.iter().any(|p| p.contains("leads one way")), "{found:?}");
    }

    #[test]
    fn loops_break_an_acyclic_diagram_but_execution_may_loop() {
        let dag = "diagram { kind: dag }\nuse graph\na -> b\nb -> c\nc -> a\nc -> d\n";
        let found = problems_of(dag);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().all(|p| p.contains("closes a loop")));
        // Not every diagram minds.
        assert!(problems_of("a -> b\nb -> a\n").is_empty());
        assert!(problems_of("diagram { acyclic: true }\na -> b\nb -> a\n").len() == 2);
        // Execution wires may loop back; data may not.
        let src = format!("{GRAPH}f: graph.function\nbegin.exec -> log.exec\nlog2: graph.function\nf.exec -> log2.exec\nlog2.exec -> f.exec\n");
        assert!(problems_of(&src).is_empty(), "{:?}", problems_of(&src));
    }

    #[test]
    fn wires_between_pins_curve_and_take_their_type_color() {
        let doc = Document::parse(format!("{GRAPH}x.value -> add.a\nflag.value -> add.b\n").as_str());
        let scene = crate::build(doc.diagram(), &Default::default());
        let wire = scene.edges.iter().find(|e| e.id == "x->add").unwrap();
        assert!(wire.points.len() > 10, "a smooth curve");
        assert_eq!(wire.stroke, Some(color(Some("float"))));
        // The miswired one is marked for the canvas.
        assert!(scene.edges.iter().find(|e| e.id == "flag->add").unwrap().problem);
        assert!(!wire.problem);
        // The node grew rows for its pins.
        let add = scene.nodes.iter().find(|n| n.id == "add").unwrap();
        assert!(add.rect.size.h >= PIN_TOP + 2.0 * PIN_ROW);
        assert_eq!(add.ports.iter().filter(|p| p.pin.is_some()).count(), 3);
    }

    #[test]
    fn any_count_on_any_side() {
        let doc = Document::parse("mix { in: [a, b, c], out: [x, y], in_side: top, sides: [y: bottom] }\n");
        let d = doc.diagram();
        let list = pins(d, d.node("mix").unwrap());
        let sides: Vec<_> = list.iter().map(|p| side_of(p, Flow::Right)).collect();
        use crate::Side::*;
        assert_eq!(sides, [Top, Top, Top, Right, Bottom]);
        let scene = crate::build(d, &Default::default());
        let n = scene.nodes.iter().find(|n| n.id == "mix").unwrap();
        // Three across the top, evenly; the rest where they were asked.
        let tops: Vec<f64> = n.ports.iter().filter(|p| p.side == Top).map(|p| p.at.x - n.rect.origin.x).collect();
        assert_eq!(tops.len(), 3);
        assert!((tops[1] - n.rect.size.w / 2.0).abs() < 1e-9);
        assert!(n.ports.iter().any(|p| p.name == "y" && p.side == Bottom && p.at.y == n.rect.origin.y + n.rect.size.h));
    }
}

