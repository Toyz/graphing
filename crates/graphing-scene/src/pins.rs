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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PinDir {
    #[default]
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pin {
    pub name: String,
    /// `float`, `exec`, a struct name, or a type variable (`T`) the wires
    /// decide; `None` takes anything.
    pub ty: Option<String>,
    pub dir: PinDir,
    /// Where the node puts it (`in_side`, `out_side`, `sides: [a: top]`);
    /// `None` for the default, inputs facing the inflow.
    pub side: Option<crate::Side>,
    /// The value an unwired input uses (`defaults: [b: 0.5]`).
    pub default: Option<String>,
    /// Must be wired or have a default (`required: [a]`).
    pub required: bool,
    /// An input that takes any number of wires (`many: [items]`).
    pub many: bool,
    /// What it is for (`docs: [a: "Health before damage"]`).
    pub doc: Option<String>,
    /// The type the wires settled on for a type variable; set by the scene.
    pub resolved: Option<String>,
    /// Wires meeting it; set by the scene.
    pub wired: usize,
}

impl Pin {
    pub fn new(name: &str, ty: Option<&str>, dir: PinDir) -> Self {
        Pin { name: name.to_string(), ty: ty.map(str::to_string), dir, ..Default::default() }
    }

    /// The type to show and check: what the wires settled on, else as
    /// declared.
    pub fn shown_type(&self) -> Option<&str> {
        self.resolved.as_deref().or(self.ty.as_deref()).filter(|t| !is_var(t))
    }

    /// Execution order rather than data: `exec`, or typed `exec`.
    pub fn exec(&self) -> bool {
        self.ty.as_deref() == Some("exec") || self.ty.is_none() && self.name == "exec"
    }

    /// Text beside the pin; a lone `exec` pin goes unnamed, as in Blueprint.
    pub fn label(&self) -> &str {
        if self.exec() && self.name == "exec" { "" } else { &self.name }
    }

    /// The label with an unwired input's own value: `b = 0.5`.
    pub fn caption(&self) -> String {
        match &self.default {
            Some(v) if self.wired == 0 && self.dir == PinDir::In => format!("{} = {v}", self.label()),
            _ => self.label().to_string(),
        }
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

/// A type variable: one capital letter, maybe numbered (`T`, `K`, `T2`).
/// Each node has its own; the wires decide what it is.
pub fn is_var(ty: &str) -> bool {
    let mut c = ty.chars();
    c.next().is_some_and(|f| f.is_ascii_uppercase()) && c.all(|d| d.is_ascii_digit())
}

/// `name` or `name: type` from a list item.
fn split_item(it: &Value) -> (String, Option<String>) {
    match it {
        Value::Pair(name, v) => (name.clone(), Some(v.text())),
        other => {
            let text = other.text();
            match text.split_once(':') {
                Some((n, t)) => (n.trim().to_string(), Some(t.trim().to_string())),
                None => (text.trim().to_string(), None),
            }
        }
    }
}

/// The node's pins: `in` then `out`, each item `name` or `name: type`. A
/// node without its own lists takes its stencil's (`pins`, which may
/// repeat by a count or over a list, or the older `in`/`out` defaults).
/// `in_side: top` (or `out_side`) moves a whole list, `sides: [a: bottom]`
/// one pin; `defaults`, `required`, `many` and `docs` describe pins by name.
pub fn pins(d: &Diagram, n: &Node) -> Vec<Pin> {
    let reg = crate::stencils::registry();
    let def = reg.resolve(n.stencil.as_deref());
    // The node's prop, else its stencil's default.
    let prop = |key: &str| d.node_prop(n, key).cloned().or_else(|| graphing_model::find_prop(&def.defaults, key).cloned());
    let pairs = |key: &str| -> HashMap<String, String> {
        prop(key).as_ref().and_then(Value::as_list).unwrap_or_default().iter().map(split_item).filter_map(|(k, v)| Some((k, v?))).collect()
    };
    let names = |key: &str| -> Vec<String> { prop(key).as_ref().and_then(Value::as_list).unwrap_or_default().iter().map(|v| split_item(v).0).collect() };
    let (sides, defaults, docs) = (pairs("sides"), pairs("defaults"), pairs("docs"));
    let (required, many) = (names("required"), names("many"));
    let mut out = Vec::new();
    for (key, dir) in [("in", PinDir::In), ("out", PinDir::Out)] {
        let list_side = prop(&format!("{key}_side")).map(|v| v.text()).as_deref().and_then(side_named);
        let declared: Vec<(String, Option<String>)> = match d.node_prop(n, key).and_then(Value::as_list) {
            Some(items) => items.iter().map(split_item).collect(),
            None => match &def.pins {
                Some(t) => t.expand(if dir == PinDir::In { &t.ins } else { &t.outs }, &|k: &str| d.node_prop(n, k).cloned()),
                None => prop(key).as_ref().and_then(Value::as_list).unwrap_or_default().iter().map(split_item).collect(),
            },
        };
        for (name, ty) in declared {
            if name.is_empty() {
                continue;
            }
            out.push(Pin {
                side: sides.get(&name).and_then(|s| side_named(s)).or(list_side),
                default: defaults.get(&name).cloned(),
                required: required.contains(&name),
                many: many.contains(&name),
                doc: docs.get(&name).cloned(),
                ty: ty.filter(|t| !t.is_empty()),
                dir,
                name,
                resolved: None,
                wired: 0,
            });
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
    let widest = |side| on(side).map(|p| p.caption().chars().count()).max().unwrap_or(0) as f64 * PIN_CHAR;
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

/// Whether a wire may run from output `src` into input `dst`: both carry
/// execution or both data, and data types agree (`any` or no type takes
/// anything).
/// `src` and `dst` carry their settled types (`shown_type`); an unsettled
/// type variable takes anything, as does `any`.
pub fn fits(d: &Diagram, src: &Pin, dst: &Pin) -> bool {
    if src.dir != PinDir::Out || dst.dir != PinDir::In || src.exec() != dst.exec() {
        return false;
    }
    match (src.shown_type(), dst.shown_type()) {
        (Some(a), Some(b)) => src.exec() || assignable(a, b, &supertypes(d)),
        _ => true,
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

/// A pin by node, name and direction.
pub type PinKey = (String, String, PinDir);

/// A wire end during analysis: the node and its pin.
type PinAt<'a> = (String, &'a Pin);

/// What the wiring of a diagram works out to.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub problems: Vec<Problem>,
    /// The concrete type each type-variable pin settled on.
    pub resolved: HashMap<PinKey, String>,
    /// Wires at each pin.
    pub wires: HashMap<PinKey, usize>,
}

/// Every problem with how `d` is wired.
pub fn problems(d: &Diagram) -> Vec<Problem> {
    analyze(d).problems
}

/// Supertypes: the diagram's `types: [Pawn: Actor]`, then every pack's.
fn supertypes(d: &Diagram) -> HashMap<String, String> {
    let mut up = crate::stencils::registry().types.clone();
    if let Some(items) = d.prop("types").and_then(Value::as_list) {
        up.extend(items.iter().map(split_item).filter_map(|(k, v)| Some((k, v?))));
    }
    up
}

/// Whether a value of type `from` may go where `to` is wanted: the same
/// type, a subtype, or either side loose (`any`, untyped).
fn assignable(from: &str, to: &str, up: &HashMap<String, String>) -> bool {
    if matches!(from, "any" | "wildcard") || matches!(to, "any" | "wildcard") || from == to {
        return true;
    }
    let mut at = from;
    for _ in 0..64 {
        match up.get(at) {
            Some(next) if next == to => return true,
            Some(next) => at = next,
            None => return false,
        }
    }
    false
}

/// Type variables, one set per node, joined by wires (union-find); each
/// set holds the first concrete type that reached it.
struct Vars {
    parent: Vec<usize>,
    bound: Vec<Option<String>>,
    ids: HashMap<(String, String), usize>,
}

impl Vars {
    fn var(&mut self, node: &str, name: &str) -> usize {
        let next = self.parent.len();
        let id = *self.ids.entry((node.to_string(), name.to_string())).or_insert(next);
        if id == next {
            self.parent.push(id);
            self.bound.push(None);
        }
        id
    }

    fn root(&mut self, mut v: usize) -> usize {
        while self.parent[v] != v {
            self.parent[v] = self.parent[self.parent[v]];
            v = self.parent[v];
        }
        v
    }

    /// Join two sets; `Err` with both types when they hold different ones.
    fn join(&mut self, a: usize, b: usize) -> Result<(), (String, String)> {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra == rb {
            return Ok(());
        }
        match (self.bound[ra].clone(), self.bound[rb].clone()) {
            (Some(x), Some(y)) if x != y => return Err((x, y)),
            (None, Some(y)) => self.bound[ra] = Some(y),
            _ => {}
        }
        self.parent[rb] = ra;
        Ok(())
    }

    /// Give a set a concrete type; `Err` with the one it already has.
    fn bind(&mut self, v: usize, ty: &str) -> Result<(), String> {
        let r = self.root(v);
        match &self.bound[r] {
            Some(t) if t != ty => Err(t.clone()),
            Some(_) => Ok(()),
            None => {
                self.bound[r] = Some(ty.to_string());
                Ok(())
            }
        }
    }

    fn get(&mut self, node: &str, name: &str) -> Option<String> {
        let v = *self.ids.get(&(node.to_string(), name.to_string()))?;
        let r = self.root(v);
        self.bound[r].clone()
    }
}

/// `node.pin` for messages; a shape wired as an item is just `node`.
fn pin_at(node: &str, p: &Pin) -> String {
    if p.name.is_empty() { node.to_string() } else { format!("{node}.{}", p.name) }
}

/// Work out the wiring of `d`: problems, the types type-variable pins
/// settle on, and how many wires meet each pin.
pub fn analyze(d: &Diagram) -> Analysis {
    let mut a = Analysis::default();
    let pins_of: HashMap<&str, Vec<Pin>> = d.nodes.iter().map(|n| (n.id.as_str(), pins(d, n))).filter(|(_, p)| !p.is_empty()).collect();
    let up = supertypes(d);
    // A wire end without a pin is the shape itself, as an item: it gives
    // itself, typed by its `type:` or its shape (`c4.database`), and takes
    // anything. A C4 container can feed a DAG step this way.
    let items: HashMap<&str, [Pin; 2]> = d
        .nodes
        .iter()
        .map(|n| {
            let ty = d.node_prop(n, "type").map(Value::text).or_else(|| n.stencil.clone());
            (n.id.as_str(), [Pin { ty, ..Pin::new("", None, PinDir::Out) }, Pin::new("", None, PinDir::In)])
        })
        .collect();
    // Where an input and an output share a name (`exec`), a wire's source
    // end means the output and its target end the input.
    let find = |node: &str, port: &Option<String>, dir: PinDir| -> Option<Result<&Pin, String>> {
        if port.is_none()
            && let Some(item) = items.get(node)
        {
            return Some(Ok(&item[usize::from(dir == PinDir::In)]));
        }
        let pins = pins_of.get(node)?;
        let port = port.as_deref()?;
        let named = |p: &&Pin| p.name == port;
        Some(pins.iter().filter(named).find(|p| p.dir == dir).or_else(|| pins.iter().find(named)).ok_or_else(|| format!("`{node}` has no pin `{port}`")))
    };
    let mut vars = Vars { parent: Vec::new(), bound: Vec::new(), ids: HashMap::new() };
    // Wires that passed the shape checks, for the type pass.
    let mut typed: Vec<(String, PinAt, PinAt)> = Vec::new();
    let mut into: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut from: HashMap<(String, String), Vec<String>> = HashMap::new();
    for e in &d.edges {
        let (from_dir, to_dir) = if e.arrow == graphing_model::Arrow::Back { (PinDir::In, PinDir::Out) } else { (PinDir::Out, PinDir::In) };
        let (pa, pb) = (find(&e.from, &e.from_port, from_dir), find(&e.to, &e.to_port, to_dir));
        for r in [&pa, &pb] {
            if let Some(Err(m)) = r {
                a.problems.push(Problem { id: e.id.clone(), message: m.clone() });
            }
        }
        for (node, r) in [(&e.from, &pa), (&e.to, &pb)] {
            if let Some(Ok(p)) = r
                && !p.name.is_empty()
            {
                *a.wires.entry((node.clone(), p.name.clone(), p.dir)).or_default() += 1;
            }
        }
        let (Some(Ok(pa)), Some(Ok(pb))) = (pa, pb) else { continue };
        // Two plain shapes: an ordinary line, nothing to check.
        if pa.name.is_empty() && pb.name.is_empty() {
            continue;
        }
        // `<-` runs the other way; `--` and `<->` say nothing about direction.
        let (src, dst) = match e.arrow {
            graphing_model::Arrow::Back => ((e.to.clone(), pb), (e.from.clone(), pa)),
            graphing_model::Arrow::Forward => ((e.from.clone(), pa), (e.to.clone(), pb)),
            _ => continue,
        };
        let end = |(n, p): &(String, &Pin)| pin_at(n, p);
        let problem = |message: String| Problem { id: e.id.clone(), message };
        if src.1.dir != PinDir::Out {
            a.problems.push(problem(format!("`{}` is an input; wires leave from outputs", end(&src))));
            continue;
        }
        if dst.1.dir != PinDir::In {
            a.problems.push(problem(format!("`{}` is an output; wires arrive at inputs", end(&dst))));
            continue;
        }
        if src.1.exec() != dst.1.exec() {
            a.problems.push(problem(format!("`{}` and `{}` mix execution and data", end(&src), end(&dst))));
            continue;
        }
        if src.1.exec() {
            from.entry((src.0.clone(), src.1.name.clone())).or_default().push(e.id.clone());
        } else {
            if !dst.1.many {
                into.entry((dst.0.clone(), dst.1.name.clone())).or_default().push(e.id.clone());
            }
            typed.push((e.id.clone(), src, dst));
        }
    }
    // Type variables: a wire joins the sets at both ends, or gives one a type.
    for (id, (sn, sp), (dn, dp)) in &typed {
        let term = |vars: &mut Vars, n: &str, p: &Pin| p.ty.as_deref().filter(|t| is_var(t)).map(|t| vars.var(n, t));
        let (sv, dv) = (term(&mut vars, sn, sp), term(&mut vars, dn, dp));
        let clash = match (sv, dv, &sp.ty, &dp.ty) {
            (Some(x), Some(y), _, _) => vars.join(x, y).err().map(|(x, y)| format!("this wire would make one type both {x} and {y}")),
            (Some(x), None, _, Some(t)) if !matches!(t.as_str(), "any" | "wildcard") => {
                vars.bind(x, t).err().map(|had| format!("`{}` is {had} here, but `{}` takes {t}", pin_at(sn, sp), pin_at(dn, dp)))
            }
            (None, Some(y), Some(t), _) if !matches!(t.as_str(), "any" | "wildcard") => {
                vars.bind(y, t).err().map(|had| format!("`{}` is {had} here, but `{}` gives {t}", pin_at(dn, dp), pin_at(sn, sp)))
            }
            _ => None,
        };
        if let Some(message) = clash {
            a.problems.push(Problem { id: id.clone(), message });
        }
    }
    // Then every wire's settled types must fit, subtypes allowed.
    let mut settled = |n: &str, p: &Pin| match p.ty.as_deref() {
        Some(t) if is_var(t) => vars.get(n, t),
        other => other.map(str::to_string),
    };
    for (id, (sn, sp), (dn, dp)) in &typed {
        let (st, dt) = (settled(sn, sp), settled(dn, dp));
        if let (Some(st), Some(dt)) = (&st, &dt)
            && !assignable(st, dt, &up)
            && !a.problems.iter().any(|p| &p.id == id)
        {
            a.problems.push(Problem { id: id.clone(), message: format!("`{}` gives {st} but `{}` takes {dt}", pin_at(sn, sp), pin_at(dn, dp)) });
        }
    }
    for (node, list) in &pins_of {
        for p in list {
            if let Some(t) = p.ty.as_deref().filter(|t| is_var(t))
                && let Some(ty) = settled(node, p).filter(|r| r != t)
            {
                a.resolved.insert((node.to_string(), p.name.clone(), p.dir), ty);
            }
            // A required input needs a wire or a value of its own.
            let wired = a.wires.get(&(node.to_string(), p.name.clone(), p.dir)).copied().unwrap_or(0);
            if p.required && p.dir == PinDir::In && wired == 0 && p.default.is_none() {
                a.problems.push(Problem { id: node.to_string(), message: format!("`{node}.{}` needs a wire or a default", p.name) });
            }
        }
    }
    for ((node, pin), wires) in into.iter().filter(|(_, w)| w.len() > 1) {
        for id in &wires[1..] {
            a.problems.push(Problem { id: id.clone(), message: format!("`{node}.{pin}` takes one wire (list it in `many` to take more)") });
        }
    }
    for ((node, pin), wires) in from.iter().filter(|(_, w)| w.len() > 1) {
        for id in &wires[1..] {
            a.problems.push(Problem { id: id.clone(), message: format!("`{node}.{pin}` leads one way; add a sequence to branch") });
        }
    }
    if acyclic(d) {
        a.problems.extend(loops(d, &pins_of));
    }
    a.problems.sort_by(|x, y| x.id.cmp(&y.id).then(x.message.cmp(&y.message)));
    a.problems.dedup();
    a
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
            Pin::new("a", None, PinDir::In),
            Pin::new("b", None, PinDir::In),
            Pin::new("out", None, PinDir::Out),
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

    fn analysis(src: &str) -> (Analysis, Vec<String>) {
        let doc = Document::parse(src);
        assert!(doc.diags().is_empty(), "{:?}", doc.diags());
        let a = analyze(doc.diagram());
        let msgs = a.problems.iter().map(|p| format!("{}: {}", p.id, p.message)).collect();
        (a, msgs)
    }

    #[test]
    fn type_variables_settle_from_the_wires() {
        let src = "use graph\nx: graph.variable { out: [value: float] }\ny: graph.variable { out: [value: float] }\npick: graph.variable { out: [value: bool] }\nsel: graph.select\nshow: graph.pure { in: [v: float] }\nx.value -> sel.a\ny.value -> sel.b\npick.value -> sel.pick\nsel.result -> show.v\n";
        let (a, msgs) = analysis(src);
        assert!(msgs.is_empty(), "{msgs:?}");
        // Every `T` on the select node is float now, the output included.
        assert_eq!(a.resolved.get(&("sel".into(), "result".into(), PinDir::Out)).map(String::as_str), Some("float"));
        // Feeding it a bool on the other side clashes, on the wire that does it.
        let (_, msgs) = analysis(&src.replace("y: graph.variable { out: [value: float] }", "y: graph.variable { out: [value: bool] }"));
        assert!(msgs.iter().any(|m| m.starts_with("y->sel") && m.contains("float")), "{msgs:?}");
        // Variables belong to their node: two selects can settle differently.
        let two = "use graph\nf: graph.variable { out: [value: float] }\nb: graph.variable { out: [value: bool] }\ns1: graph.select\ns2: graph.select\nf.value -> s1.a\nb.value -> s2.a\n";
        let (a, msgs) = analysis(two);
        assert!(msgs.is_empty(), "{msgs:?}");
        assert_eq!(a.resolved.get(&("s2".into(), "b".into(), PinDir::In)).map(String::as_str), Some("bool"));
        // A chain of generic nodes carries the type through.
        let chain = "use graph\nf: graph.variable { out: [value: int] }\ns1: graph.select\ns2: graph.select\nf.value -> s1.a\ns1.result -> s2.a\n";
        let (a, _) = analysis(chain);
        assert_eq!(a.resolved.get(&("s2".into(), "result".into(), PinDir::Out)).map(String::as_str), Some("int"));
    }

    #[test]
    fn subtypes_fit_where_their_supertype_is_wanted() {
        let src = "diagram { types: [Pawn: Actor, Actor: Object] }\np: { out: [it: Pawn] }\nuse_actor: { in: [who: Actor] }\nuse_object: { in: [what: Object] }\nuse_pawn: { in: [who: Pawn] }\nobj: { out: [it: Object] }\np.it -> use_actor.who\np.it -> use_object.what\nobj.it -> use_pawn.who\n";
        let (_, msgs) = analysis(src);
        assert_eq!(msgs, ["obj->use_pawn: `obj.it` gives Object but `use_pawn.who` takes Pawn"]);
    }

    #[test]
    fn pin_details_defaults_required_and_many() {
        let src = "join: { in: [parts: text, sep: text, size: int], out: [all: text], many: [parts], defaults: [sep: \", \"], required: [sep, size], docs: [size: \"Most items\"] }\na: { out: [t: text] }\nb: { out: [t: text] }\na.t -> join.parts\nb.t -> join.parts\n";
        let (a, msgs) = analysis(src);
        // Many wires into `parts` are fine; `size` has neither wire nor default.
        assert_eq!(msgs, ["join: `join.size` needs a wire or a default"]);
        assert_eq!(a.wires.get(&("join".into(), "parts".into(), PinDir::In)), Some(&2));
        let doc = Document::parse(src);
        let list = pins(doc.diagram(), doc.diagram().node("join").unwrap());
        let sep = list.iter().find(|p| p.name == "sep").unwrap();
        assert_eq!(sep.caption(), "sep = , ");
        assert_eq!(list.iter().find(|p| p.name == "size").unwrap().doc.as_deref(), Some("Most items"));
    }

    #[test]
    fn stencil_pins_grow_with_their_settings() {
        let doc = Document::parse("use graph\nseq: graph.sequence { outputs: 4 }\nsw: graph.switch { cases: [red, green, blue] }\narr: graph.make-array\n");
        let d = doc.diagram();
        let names = |id: &str, dir: PinDir| pins(d, d.node(id).unwrap()).into_iter().filter(|p| p.dir == dir).map(|p| p.name).collect::<Vec<_>>();
        assert_eq!(names("seq", PinDir::Out), ["then 0", "then 1", "then 2", "then 3"]);
        assert_eq!(names("sw", PinDir::Out), ["red", "green", "blue", "default"]);
        assert_eq!(names("arr", PinDir::In), ["[0]", "[1]"]);
        // The node's own list replaces the stencil's.
        let doc = Document::parse("use graph\nseq: graph.sequence { out: [first: exec] }\n");
        assert_eq!(pins(doc.diagram(), doc.diagram().node("seq").unwrap()).iter().filter(|p| p.dir == PinDir::Out).count(), 1);
    }

    #[test]
    fn any_shape_feeds_a_pin_as_itself() {
        let src = "diagram { types: [c4.database: c4.container] }\nuse c4, graph\ndb: c4.database\nweb: c4.container { out: [events: event] }\nnote: { type: memo }\ndump: graph.task { in: [source: c4.container, who: c4.database, text: memo] }\ndb -> dump.source\nnote -> dump.text\nweb -> dump.who\nweb -- db\n";
        let (a, msgs) = analysis(src);
        // A database fits a container input (subtype); a container is not a
        // database; a shape's own `type:` counts; plain lines are not wires.
        assert_eq!(msgs, ["web->dump: `web` gives c4.container but `dump.who` takes c4.database"]);
        assert_eq!(a.wires.get(&("dump".into(), "source".into(), PinDir::In)), Some(&1));
    }
}

