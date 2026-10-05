//! Diagram -> SysML v2 textual notation.
//!
//! The output is a practical subset of SysML v2 that `graphing_import`
//! reads back: every element keeps its graphing id as a v2 short name
//! (`part def <vehicle> Vehicle`), so references resolve and a round trip
//! keeps ids. Relationships with a v2 form use it (specialization,
//! `satisfy`, `connect`, `flow`, `transition`, successions, `message`);
//! the rest become dependencies tagged with metadata
//! (`#derive dependency from a to b;`). Geometry, which v2 has no place
//! for, rides along in `// @layout` comments.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use graphing_model::{Arrow, Diagram, Edge, Node, Value};
use graphing_scene::notation;
use graphing_scene::stencils::registry;

/// SysML v2 text for `d`.
pub fn to_sysml2(d: &Diagram) -> String {
    let mut w = Writer { out: String::new(), depth: 0 };
    let title = d.title.clone().unwrap_or_else(|| "Model".into());
    let kind = |k: &str| d.prop(k).map(Value::text);
    let _ = writeln!(w.out, "// Exported by graphing. Layout and diagram settings ride in comments.");
    let mut header: Vec<String> = Vec::new();
    for key in ["kind", "context", "view", "look"] {
        if let Some(v) = kind(key) {
            header.push(format!("{key}: {v}"));
        }
    }
    if !header.is_empty() {
        let _ = writeln!(w.out, "// @diagram {}", header.join(", "));
    }
    w.open(&format!("package {}", name(&title)));
    if !d.packs.is_empty() {
        w.line(&format!("// @use {}", d.packs.join(", ")));
    }
    let ctx = Ctx::new(d);

    // Nodes, nested in their group's package.
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    let roots: Vec<&str> = d.groups.iter().filter(|g| !d.groups.iter().any(|p| p.members.contains(&g.id))).map(|g| g.id.as_str()).collect();
    for g in roots {
        ctx.group(&mut w, g, &mut emitted);
    }
    for n in &d.nodes {
        if !emitted.contains(&n.id) {
            ctx.node(&mut w, n);
        }
    }

    // Relationships not already written as specialization.
    let mut first = true;
    for e in &d.edges {
        if ctx.is_specialization(e) {
            continue;
        }
        if first {
            w.blank();
            first = false;
        }
        w.line(&ctx.edge(e));
    }
    w.close();

    if !d.layout.is_empty() {
        w.out.push('\n');
        for (id, p) in &d.layout {
            let size = p.size.map(|s| format!(" {}x{}", num(s.w), num(s.h))).unwrap_or_default();
            let _ = writeln!(w.out, "// @layout {id} {} {}{size}", num(p.pos.x), num(p.pos.y));
        }
    }
    w.out
}

struct Writer {
    out: String,
    depth: usize,
}

impl Writer {
    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn blank(&mut self) {
        self.out.push('\n');
    }

    fn open(&mut self, head: &str) {
        self.open_note(head, "");
    }

    /// `head {` with an optional trailing comment.
    fn open_note(&mut self, head: &str, note: &str) {
        self.line(&format!("{head} {{{note}"));
        self.depth += 1;
    }

    fn close(&mut self) {
        self.depth -= 1;
        self.line("}");
    }
}

struct Ctx<'a> {
    d: &'a Diagram,
    /// Node id -> canonical stencil name inside the sysml pack (`block`).
    kinds: HashMap<&'a str, String>,
    ibd: bool,
}

impl<'a> Ctx<'a> {
    fn new(d: &'a Diagram) -> Self {
        let reg = registry();
        let kinds = d
            .nodes
            .iter()
            .map(|n| {
                let def = reg.resolve(n.stencil.as_deref());
                let k = def.id.strip_prefix("sysml.").map(str::to_string).unwrap_or_else(|| format!("@{}", def.id));
                (n.id.as_str(), k)
            })
            .collect();
        let ibd = d.prop("kind").map(Value::text).as_deref().map(notation::short_kind) == Some("ibd");
        Self { d, kinds, ibd }
    }

    fn kind(&self, id: &str) -> &str {
        self.kinds.get(id).map_or("", String::as_str)
    }

    fn group(&self, w: &mut Writer, id: &str, emitted: &mut BTreeSet<String>) {
        let Some(g) = self.d.group(id) else { return };
        let label = g.label.as_deref().unwrap_or(&g.id);
        let mut head = format!("package <{}> {}", short(&g.id), name(label));
        let props: Vec<String> = g.props.iter().map(|(k, v)| format!("{k}: {}", graphing_dsl::fmt_value(v))).collect();
        if !props.is_empty() {
            head = format!("// @group {}\n{}{head}", props.join(", "), "    ".repeat(w.depth));
        }
        w.open(&head);
        for m in &g.members {
            if self.d.group(m).is_some() {
                self.group(w, m, emitted);
            } else if let Some(n) = self.d.node(m) {
                self.node(w, n);
                emitted.insert(n.id.clone());
            }
        }
        w.close();
    }

    fn node(&self, w: &mut Writer, n: &Node) {
        let d = self.d;
        let list = |key: &str| d.node_prop(n, key).and_then(Value::as_list).map(|l| l.iter().map(Value::text).collect::<Vec<_>>()).unwrap_or_default();
        let text = |key: &str| d.node_prop(n, key).map(Value::text);
        let label = n.label.clone().unwrap_or_else(|| n.id.clone());
        let id = short(&n.id);
        let meta = text("stereotype").map(|s| format!("#{} ", name(&s))).unwrap_or_default();
        let kind = self.kind(&n.id).to_string();
        // Specializations go in the header: `part def <a> A :> b`.
        let supers: Vec<String> = d.edges.iter().filter(|e| e.from == n.id && self.is_specialization(e)).map(|e| short(&e.to)).collect();
        let spec = if supers.is_empty() { String::new() } else { format!(" :> {}", supers.join(", ")) };
        let mut body: Vec<String> = Vec::new();
        let extra = props_note(&n.props, HANDLED_NODE_KEYS);
        let members = |kw: &str, key: &str, body: &mut Vec<String>| {
            for item in list(key) {
                body.push(member(kw, &item));
            }
        };
        let head = match kind.as_str() {
            "block" => {
                members("attribute", "values", &mut body);
                members("part", "parts", &mut body);
                members("ref part", "references", &mut body);
                members("port", "ports", &mut body);
                members("action", "operations", &mut body);
                members("constraint", "constraints", &mut body);
                format!("{meta}part def <{id}> {}{spec}", name(&label))
            }
            "part" => {
                members("attribute", "values", &mut body);
                members("part", "parts", &mut body);
                members("port", "ports", &mut body);
                // An IBD part is a usage: `part uut : FlightArticle`.
                if self.ibd { format!("{meta}part {} : {}", name(&n.id), name(&label)) } else { format!("{meta}part <{id}> {}", name(&label)) }
            }
            "interface" => {
                members("port", "ports", &mut body);
                members("flow", "flows", &mut body);
                members("action", "operations", &mut body);
                format!("{meta}interface def <{id}> {}{spec}", name(&label))
            }
            "constraint" => {
                members("constraint", "constraints", &mut body);
                members("in", "parameters", &mut body);
                format!("{meta}constraint def <{id}> {}", name(&label))
            }
            "valuetype" => {
                members("attribute", "values", &mut body);
                format!("{meta}attribute def <{id}> {}{spec}", name(&label))
            }
            "requirement" => {
                if let Some(t) = text("text") {
                    body.push(format!("doc /* {} */", t.replace("*/", "* /")));
                }
                if let Some(r) = text("rid") {
                    body.push(format!("attribute :>> reqId = {};", quote(&r)));
                }
                format!("{meta}requirement def <{id}> {}{spec}", name(&label))
            }
            "testcase" => {
                members("action", "operations", &mut body);
                format!("{meta}verification def <{id}> {}", name(&label))
            }
            "rationale" => return w.line(&format!("comment <{id}> /* {} */", label.replace("*/", "* /"))),
            "package" => format!("package <{id}> {}", name(&label)),
            "action" => format!("{meta}action <{id}> {}", name(&label)),
            "initial" => format!("#initial action <{id}>"),
            "final" => format!("#final action <{id}>"),
            "decision" => format!("decide <{id}>"),
            "fork" => format!("fork <{id}>"),
            "join" => format!("join <{id}>"),
            "state" => {
                for (kw, key) in [("entry", "entry"), ("do", "do"), ("exit", "exit")] {
                    if let Some(t) = text(key) {
                        body.push(format!("{kw} action {};", name(&t)));
                    }
                }
                format!("{meta}state <{id}> {}", name(&label))
            }
            "actor" => format!("#actor part <{id}> {}", name(&label)),
            "usecase" => format!("{meta}use case <{id}> {}", name(&label)),
            "lifeline" => format!("#lifeline part <{id}> {}", name(&label)),
            other => {
                // Not a SysML shape: a plain part that remembers its stencil.
                let stencil = other.trim_start_matches('@');
                format!("#{} part <{id}> {}", name(stencil), name(&label))
            }
        };
        if body.is_empty() {
            w.line(&format!("{head};{extra}"));
        } else {
            w.open_note(&head, &extra);
            for b in body {
                w.line(&b);
            }
            w.close();
        }
    }

    /// Generalization between two definitions, written as `:>`.
    fn is_specialization(&self, e: &Edge) -> bool {
        let defs = ["block", "interface", "valuetype", "requirement"];
        self.d.edge_prop(e, "kind").map(Value::text).as_deref().map(notation::short_kind) == Some("generalization")
            && e.label.is_none()
            && defs.contains(&self.kind(&e.from))
            && self.d.node(&e.to).is_some()
    }

    fn edge(&self, e: &Edge) -> String {
        let d = self.d;
        let kind = d.edge_prop(e, "kind").map(|v| notation::short_kind(&v.text()).to_string()).unwrap_or_default();
        let from = endpoint(&e.from, e.from_port.as_deref());
        let to = endpoint(&e.to, e.to_port.as_deref());
        let label = e.label.as_deref();
        let named = |kw: &str| match label {
            Some(l) => format!("{kw} {} ", name(l)),
            None => format!("{kw} "),
        };
        let diagram = d.prop("kind").map(|v| notation::short_kind(&v.text()).to_string()).unwrap_or_default();
        let extra = props_note(&e.props, &["kind"]);
        let line = match kind.as_str() {
            "satisfy" if label.is_none() => format!("satisfy {to} by {from};"),
            "flow" => format!("{}from {from} to {to};", named("flow")),
            "transition" => transition(&from, &to, label),
            "" if e.arrow == Arrow::None => match label {
                Some(l) => format!("connection {} connect {from} to {to};", name(l)),
                None => format!("connect {from} to {to};"),
            },
            "" if diagram == "act" || diagram == "stm" => match label {
                Some(l) => format!("succession {} first {from} then {to};", name(l)),
                None => format!("first {from} then {to};"),
            },
            "" if diagram == "sd" => format!("{}from {from} to {to};", named("message")),
            "" => format!("{}from {from} to {to};", named("dependency")),
            k => format!("#{} {}from {from} to {to};", name(k), named("dependency")),
        };
        format!("{line}{extra}")
    }
}

/// `transition first a accept t if g do e then b;` from `t [g] / e`.
fn transition(from: &str, to: &str, label: Option<&str>) -> String {
    let mut s = format!("transition first {from}");
    if let Some(l) = label {
        let (rest, effect) = match l.split_once('/') {
            Some((a, b)) => (a, Some(b.trim())),
            None => (l, None),
        };
        let (trigger, guard) = match (rest.find('['), rest.rfind(']')) {
            (Some(a), Some(b)) if b > a => (rest[..a].trim(), Some(rest[a + 1..b].trim())),
            _ => (rest.trim(), None),
        };
        if !trigger.is_empty() {
            let _ = write!(s, " accept {}", name(trigger));
        }
        if let Some(g) = guard.filter(|g| !g.is_empty()) {
            let _ = write!(s, " if {}", name(g));
        }
        if let Some(e) = effect.filter(|e| !e.is_empty()) {
            let _ = write!(s, " do {}", name(e));
        }
    }
    let _ = write!(s, " then {to};");
    s
}

/// A compartment line as a member: `mass : kg = 1500` -> `attribute mass :
/// kg = 1500;`. Lines that are not `name [: Type]...` become quoted names, and
/// a trailing `{ note }` becomes the member's doc.
fn member(kw: &str, item: &str) -> String {
    let (main, note) = match (item.find('{'), item.rfind('}')) {
        (Some(a), Some(b)) if b > a => (item[..a].trim(), Some(item[a + 1..b].trim())),
        _ => (item.trim(), None),
    };
    let plain = is_typed_member(main);
    let text = if plain { tidy_mult(main) } else { name(main) };
    match note {
        Some(n) => format!("{kw} {text} {{ doc /* {n} */ }}"),
        None => format!("{kw} {text};"),
    }
}

/// `name`, `name : Type`, `name : Type [4]`, each optionally `= value`.
fn is_typed_member(s: &str) -> bool {
    let (decl, value) = match s.split_once('=') {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (s.trim(), None),
    };
    if value.is_some_and(|v| v.is_empty() || v.contains([';', '{', '}', '\'', '"'])) {
        return false;
    }
    let (n, ty) = match decl.split_once(':') {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (decl, None),
    };
    let ty_ok = ty.is_none_or(|t| {
        let t = t.trim_end_matches(|c: char| c == ']' || c.is_ascii_digit() || c == '.' || c == '*' || c == ' ').trim_end_matches('[').trim();
        let t = t.split('[').next().unwrap_or(t).trim();
        t.split('.').all(is_name)
    });
    is_name(n) && ty_ok
}

/// `Wheel [4]` -> `Wheel[4]`.
fn tidy_mult(s: &str) -> String {
    s.replace(" [", "[")
}

/// Node props written as members or headers; the rest go in `// @props`.
const HANDLED_NODE_KEYS: &[&str] =
    &["values", "parts", "references", "ports", "operations", "constraints", "flows", "parameters", "text", "rid", "entry", "do", "exit", "stereotype"];

/// ` // @props k: v, ...` for props v2 has no place for, or empty.
fn props_note(props: &[(String, Value)], handled: &[&str]) -> String {
    let rest: Vec<String> = props.iter().filter(|(k, _)| !handled.contains(&k.as_str())).map(|(k, v)| format!("{k}: {}", graphing_dsl::fmt_value(v))).collect();
    if rest.is_empty() { String::new() } else { format!(" // @props {}", rest.join(", ")) }
}

fn endpoint(node: &str, port: Option<&str>) -> String {
    match port {
        Some(p) => format!("{}.{}", short(node), name(p)),
        None => short(node),
    }
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A v2 name: bare when it can be, `'quoted'` otherwise.
pub fn name(s: &str) -> String {
    const RESERVED: &[&str] = &[
        "part", "def", "port", "action", "state", "package", "attribute", "ref", "flow", "connect", "to", "from", "first", "then", "if", "do", "accept",
        "transition", "requirement", "satisfy", "by", "doc", "comment", "in", "out", "use", "case", "message", "entry", "exit", "decide", "fork",
        "join", "dependency", "connection", "interface", "constraint", "verification", "about", "end", "item", "abstract", "succession",
    ];
    if is_name(s) && !RESERVED.contains(&s) { s.to_string() } else { format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")) }
}

/// Short names are graphing ids, which may hold dots and dashes.
fn short(id: &str) -> String {
    name(id)
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn num(v: f64) -> String {
    graphing_dsl::fmt_num(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphing_dsl::Document;
    use std::collections::BTreeMap;

    type Facts = (BTreeMap<String, (String, Option<String>, Vec<String>)>, Vec<(String, String, String, Option<String>)>, Vec<(String, Vec<String>)>, Vec<String>);

    /// What a round trip must keep, normalised.
    fn facts(d: &Diagram) -> Facts {
        let reg = registry();
        let nodes = d
            .nodes
            .iter()
            .map(|n| {
                let stencil = reg.resolve(n.stencil.as_deref()).id.clone();
                let mut props: Vec<String> = n.props.iter().map(|(k, v)| format!("{k}={}", match v { Value::List(l) => l.iter().map(Value::text).collect::<Vec<_>>().join("|"), v => v.text() })).collect();
                props.sort();
                (n.id.clone(), (stencil, n.label.clone(), props))
            })
            .collect();
        let mut edges: Vec<_> = d
            .edges
            .iter()
            .map(|e| {
                let from = match &e.from_port { Some(p) => format!("{}.{p}", e.from), None => e.from.clone() };
                let to = match &e.to_port { Some(p) => format!("{}.{p}", e.to), None => e.to.clone() };
                (from, to, d.edge_prop(e, "kind").map(Value::text).unwrap_or_default(), e.label.clone())
            })
            .collect();
        edges.sort();
        let mut groups: Vec<_> = d.groups.iter().map(|g| (g.id.clone(), g.members.clone())).collect();
        groups.sort();
        let layout = d.layout.iter().map(|(id, p)| format!("{id} {} {} {:?}", p.pos.x, p.pos.y, p.size.map(|s| (s.w, s.h)))).collect();
        (nodes, edges, groups, layout)
    }

    #[test]
    fn examples_round_trip_through_sysml_v2() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
        let mut files: Vec<_> = std::fs::read_dir(format!("{root}/sysml")).unwrap().map(|e| e.unwrap().path()).collect();
        files.push(format!("{root}/hil-ibd.gph").into());
        files.push(format!("{root}/auth.gph").into());
        files.sort();
        for f in files {
            let src = std::fs::read_to_string(&f).unwrap();
            let before = Document::parse(src.as_str());
            let v2 = to_sysml2(before.diagram());
            let back = graphing_import::from_sysml2(&v2).unwrap_or_else(|e| panic!("{}: {e}\n{v2}", f.display()));
            let after = Document::parse(back.source.as_str());
            assert!(after.diags().is_empty(), "{}: {:?}\n{}", f.display(), after.diags(), back.source);
            assert_eq!(facts(before.diagram()), facts(after.diagram()), "{}\n--- v2\n{v2}\n--- back\n{}", f.display(), back.source);
            assert_eq!(before.diagram().title, after.diagram().title, "{}", f.display());
        }
    }

    #[test]
    fn hand_written_v2_imports() {
        let src = r#"
package Drone {
    part def Frame;
    part def Motor :> Component { attribute kv : RPM_per_V; }
    part def Component;
    requirement def <'R7'> Lift { doc /* Lift at least 2 kg. */ }
    satisfy Lift by Motor;
    dependency from Frame to Motor;
}
"#;
        let out = graphing_import::from_sysml2(src).unwrap();
        let d = Document::parse(out.source.as_str());
        assert!(d.diags().is_empty(), "{:?}\n{}", d.diags(), out.source);
        let dg = d.diagram();
        assert_eq!(dg.title.as_deref(), Some("Drone"));
        assert_eq!(dg.nodes.len(), 4);
        let kinds: Vec<_> = dg.edges.iter().map(|e| dg.edge_prop(e, "kind").map(Value::text).unwrap_or_default()).collect();
        assert_eq!(kinds, ["generalization", "satisfy", ""]);
        assert!(dg.node("R7").is_some(), "{}", out.source);
    }
}
