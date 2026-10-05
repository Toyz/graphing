//! What a stencil means beyond its outline: stereotype, header line,
//! compartments, size; and what an edge `kind` means: end markers, dashes,
//! stereotype. SysML lives here, but nothing is SysML-only: any node can
//! carry compartments and any edge can carry a kind.

use graphing_model::{Diagram, Edge, Node, Value};

use crate::Shape;
use crate::stencils::{Header, registry};
pub use crate::stencils::{PropDef, PropKind};

/// A titled list inside a node, under its header.
#[derive(Debug, Clone, PartialEq)]
pub struct Compartment {
    pub title: Option<String>,
    pub lines: Vec<String>,
}

/// Resolved presentation of one node.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    pub shape: Shape,
    pub stereotype: Option<String>,
    /// Bold header line (`role : Type`, block name) or the plain label.
    pub title: String,
    pub compartments: Vec<Compartment>,
    /// Smaller centred lines under the label (stencils with `notes`).
    pub notes: Vec<String>,
    pub min: (f64, f64),
}

/// End decorations, drawn at `from` (tail) and `to` (head).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum End {
    #[default]
    None,
    /// Filled triangle.
    Arrow,
    /// Two-stroke open V.
    Open,
    /// Hollow triangle (generalization, realization).
    Triangle,
    /// Hollow diamond (aggregation).
    Diamond,
    /// Filled diamond (composition).
    FilledDiamond,
    /// Hollow circle (containment, lollipop).
    Circle,
    /// Crow's foot, ER cardinality: exactly one (one bar).
    One,
    /// Exactly one, drawn with two bars.
    OneOnly,
    /// Many (crow's foot).
    Many,
    /// Zero or one (circle and bar).
    ZeroOne,
    /// One or many (bar and crow's foot).
    OneMany,
    /// Zero or many (circle and crow's foot).
    ZeroMany,
}

/// A stroke of a cardinality end, in units back from the tip along the
/// line (`back`) and to its side (`side`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EndStroke {
    /// From (back, side) to (back, side).
    Line(f64, f64, f64, f64),
    /// A hollow ring centred `back` units from the tip.
    Ring { back: f64, r: f64 },
}

impl End {
    /// The strokes of a crow's foot end; empty for the other ends.
    pub fn crow_strokes(self) -> Vec<EndStroke> {
        use EndStroke::{Line, Ring};
        let bar = |b: f64| Line(b, 6.0, b, -6.0);
        let crow = [Line(0.0, 7.0, 12.0, 0.0), Line(0.0, -7.0, 12.0, 0.0)];
        match self {
            End::One => vec![bar(9.0)],
            End::OneOnly => vec![bar(8.0), bar(13.0)],
            End::Many => crow.to_vec(),
            End::ZeroOne => vec![bar(8.0), Ring { back: 17.5, r: 4.5 }],
            End::OneMany => vec![crow[0], crow[1], bar(16.0)],
            End::ZeroMany => vec![crow[0], crow[1], Ring { back: 18.5, r: 4.5 }],
            _ => Vec::new(),
        }
    }

    /// Every end, by the name files use.
    pub const NAMES: [&'static str; 13] =
        ["none", "arrow", "open", "triangle", "diamond", "filled-diamond", "circle", "one", "one-only", "many", "zero-one", "one-many", "zero-many"];
}


#[derive(Debug, Clone, PartialEq, Default)]
pub struct EdgeSpec {
    pub head: End,
    pub tail: End,
    pub dashed: bool,
    pub stereotype: Option<String>,
}

pub const HEADER_H: f64 = 34.0;
pub const STEREO_H: f64 = 14.0;
pub const LINE_H: f64 = 15.0;
pub const COMP_PAD: f64 = 10.0;
/// Width estimates; the canvas measures real text, these only size boxes.
pub const TITLE_CHAR: f64 = 7.4;
pub const MONO_CHAR: f64 = 6.4;
const MIN_W: f64 = 120.0;
const NODE_H: f64 = 56.0;
const PLAIN_LINE_H: f64 = 18.0;
const PLAIN_CHAR: f64 = 7.6;
/// Notes under a label: 11 units, line height and width per character.
pub const NOTE_PT: f64 = 11.0;
pub const NOTE_LINE_H: f64 = 14.5;
const NOTE_CHAR: f64 = 6.2;

fn text_prop(d: &Diagram, n: &Node, key: &str) -> Option<String> {
    d.node_prop(n, key).map(Value::text).filter(|s| !s.is_empty())
}

/// Greedy word wrap; continuation lines indent by two spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
            line.push_str("  ");
        } else if !line.is_empty() && !line.ends_with("  ") {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.trim().is_empty() {
        out.push(line);
    }
    out
}

/// Whether the node is a diagram link (its `src` names a diagram, not a
/// picture).
pub fn is_link(n: &Node) -> bool {
    registry().resolve(n.stencil.as_deref()).render.as_deref() == Some("ref")
}

/// Whether the node's stencil puts its label under the shape.
pub fn label_below(n: &Node) -> bool {
    registry().resolve(n.stencil.as_deref()).label == crate::stencils::LabelAt::Below
}

pub fn shape_of(stencil: Option<&str>) -> Shape {
    registry().resolve(stencil).shape()
}

pub fn node_spec(d: &Diagram, n: &Node) -> NodeSpec {
    let reg = registry();
    let def = reg.resolve(n.stencil.as_deref());
    let shape = def.shape();
    let label = n.text().to_string();
    let stereotype = text_prop(d, n, "stereotype").or_else(|| def.stereotype.clone());
    let title = match def.header {
        Header::None => String::new(),
        // `role : Type`: the id is the role, the label the type.
        Header::Role => match &n.label {
            Some(t) if t != &n.id => format!("{} : {t}", n.id),
            _ => n.id.clone(),
        },
        // Symbols named underneath (gateways, events) show only a real label.
        Header::Label if def.label == crate::stencils::LabelAt::Below => n.label.clone().unwrap_or_default(),
        Header::Label => label,
    };

    let mut compartments = Vec::new();
    let mut lines = Vec::new();
    for f in &def.fields {
        if let Some(v) = text_prop(d, n, &f.key) {
            let line = f.format.replace("{}", &v);
            match f.wrap {
                Some(w) => lines.extend(wrap(&line, w)),
                None => lines.push(line),
            }
        }
    }
    let mut notes = Vec::new();
    if def.notes {
        notes = lines;
    } else if !lines.is_empty() {
        compartments.push(Compartment { title: None, lines });
    }
    for key in &def.compartments {
        if let Some(items) = d.node_prop(n, key).and_then(Value::as_list) {
            let lines = items.iter().map(Value::text).collect();
            compartments.push(Compartment { title: Some(key.clone()), lines });
        }
    }

    let mut min = def.size.unwrap_or_else(|| size_for(shape, stereotype.as_deref(), &title, &compartments));
    if !notes.is_empty() {
        // Grow to fit the notes; a label area shares the box with a figure.
        let share = def.label_area.map_or(1.0, |a| a[3]).max(0.2);
        let widest = notes.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * NOTE_CHAR + 28.0;
        let need = (NODE_H + notes.len() as f64 * NOTE_LINE_H) / share;
        min = ((min.0.max(widest) / 10.0).ceil() * 10.0, (min.1.max(need) / 10.0).ceil() * 10.0);
    }
    // Node-graph pins need their rows.
    let pins = crate::pins::pins(d, n);
    if !pins.is_empty() {
        let need = crate::pins::size(&title, &pins, flow(d));
        min = (min.0.max(need.0), min.1.max(need.1));
    }
    NodeSpec { shape, stereotype, title, compartments, notes, min }
}

fn size_for(shape: Shape, stereotype: Option<&str>, title: &str, comps: &[Compartment]) -> (f64, f64) {
    let snap = |v: f64| (v / 10.0).ceil() * 10.0;
    match shape {
        Shape::Initial | Shape::Final => return (28.0, 28.0),
        Shape::Bar => return (120.0, 8.0),
        Shape::Lifeline => {
            let w = (title.chars().count() as f64 * TITLE_CHAR + 32.0).max(MIN_W);
            return (snap(w), 320.0);
        }
        _ => {}
    }
    if comps.is_empty() && stereotype.is_none() && shape != Shape::Block {
        let lines = title.split('\n');
        let widest = lines.clone().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
        let n = lines.count().max(1) as f64;
        let w = (widest * PLAIN_CHAR + 32.0).max(MIN_W);
        return (snap(w), NODE_H + (n - 1.0) * PLAIN_LINE_H);
    }
    let title_w = title.chars().count() as f64 * TITLE_CHAR;
    let stereo_w = stereotype.map_or(0.0, |s| (s.chars().count() + 4) as f64 * MONO_CHAR);
    let comp_w = comps
        .iter()
        .flat_map(|c| c.lines.iter().chain(c.title.iter()))
        .map(|l| (l.chars().count() + 2) as f64 * MONO_CHAR)
        .fold(0.0, f64::max);
    let w = title_w.max(stereo_w).max(comp_w) + 28.0;
    let mut h = HEADER_H + if stereotype.is_some() { STEREO_H } else { 0.0 };
    for c in comps {
        h += COMP_PAD + LINE_H * (c.lines.len() + usize::from(c.title.is_some())) as f64 + COMP_PAD / 2.0;
    }
    if comps.is_empty() {
        h += COMP_PAD;
    }
    (snap(w.max(160.0)), snap(h))
}

impl End {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => End::None,
            "arrow" | "filled" => End::Arrow,
            "open" | "vee" => End::Open,
            "triangle" | "hollow" => End::Triangle,
            "diamond" => End::Diamond,
            "filled-diamond" | "filled_diamond" | "composite" => End::FilledDiamond,
            "circle" => End::Circle,
            "one" => End::One,
            "one-only" | "only-one" => End::OneOnly,
            "many" | "crow" => End::Many,
            "zero-one" | "zero-or-one" => End::ZeroOne,
            "one-many" | "one-or-many" => End::OneMany,
            "zero-many" | "zero-or-many" => End::ZeroMany,
            _ => return None,
        })
    }
}

pub fn edge_spec(d: &Diagram, e: &Edge) -> EdgeSpec {
    use graphing_model::Arrow;
    let (mut head, mut tail) = match e.arrow {
        Arrow::Forward => (End::Arrow, End::None),
        Arrow::Back => (End::None, End::Arrow),
        Arrow::Both => (End::Arrow, End::Arrow),
        Arrow::None => (End::None, End::None),
    };
    let ident = |k: &str| d.edge_prop(e, k).map(Value::text).filter(|s| !s.is_empty());
    let mut dashed = matches!(ident("line").as_deref(), Some("dashed" | "dotted"));
    let mut stereotype = ident("stereotype");
    if let Some(kind) = ident("kind")
        && let Some(def) = registry().edge_kind_in(&kind, &d.packs)
    {
        if let Some(h) = def.head.as_deref().and_then(End::parse) {
            head = h;
        }
        if let Some(t) = def.tail.as_deref().and_then(End::parse) {
            tail = t;
        }
        dashed |= def.dashed;
        if stereotype.is_none() {
            stereotype = def.stereotype.clone();
        }
    }
    if let Some(h) = ident("head").as_deref().and_then(End::parse) {
        head = h;
    }
    if let Some(t) = ident("tail").as_deref().and_then(End::parse) {
        tail = t;
    }
    EdgeSpec { head, tail, dashed, stereotype }
}

/// A kind's own name without the pack in front (`c4.uses` -> `uses`).
pub fn short_kind(kind: &str) -> &str {
    kind.rsplit('.').next().unwrap_or(kind)
}

/// `ibd [block] HIL Test Bench [Architecture]` when the diagram has a kind.
pub fn frame_title(d: &Diagram) -> Option<String> {
    let kind = d.prop("kind").map(Value::text).filter(|s| !s.is_empty())?;
    // SysML frames use the short kind (`ibd`); other notations their name.
    let named = registry().diagram_kind_in(&kind, &d.packs).filter(|k| k.pack != "sysml").map(|k| k.name.clone());
    let mut t = named.unwrap_or_else(|| short_kind(&kind).to_string());
    if let Some(c) = d.prop("context").map(Value::text).filter(|s| !s.is_empty()) {
        t.push_str(&format!(" [{c}]"));
    }
    if let Some(title) = &d.title {
        t.push(' ');
        t.push_str(title);
    }
    if let Some(v) = d.prop("view").map(Value::text).filter(|s| !s.is_empty()) {
        t.push_str(&format!(" [{v}]"));
    }
    Some(t)
}

/// Whether new nodes are laid out top to bottom: `flow: down` on the
/// diagram, or the diagram kind's default (fault trees, org charts).
pub fn flows_down(d: &Diagram) -> bool {
    flow(d) == Flow::Down
}

/// How new nodes are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Right,
    Down,
    /// Around the root, rings outward (mind maps).
    Radial,
}

/// `flow:` on the diagram, else its kind's default, else left to right.
pub fn flow(d: &Diagram) -> Flow {
    let name = d.prop("flow").map(Value::text).or_else(|| d.prop("kind").map(Value::text).and_then(|k| registry().diagram_kind_in(&k, &d.packs).and_then(|k| k.flow.clone())));
    match name.as_deref() {
        Some("down" | "vertical" | "tb") => Flow::Down,
        Some("radial" | "around") => Flow::Radial,
        _ => Flow::Right,
    }
}

/// Technical look: asked for, or implied by the SysML pack.
pub fn technical(d: &Diagram) -> bool {
    match d.prop("look").map(Value::text).as_deref() {
        Some("technical") => true,
        Some(_) => false,
        None => d.packs.iter().any(|p| p == "sysml"),
    }
}

/// Props the inspector offers for a stencil, in display order.
pub fn props_for(stencil: Option<&str>) -> Vec<PropDef> {
    let reg = registry();
    let def = reg.resolve(stencil);
    if def.props.is_empty() {
        vec![PropDef { key: "stereotype".into(), label: "Stereotype".into(), kind: PropKind::Text }]
    } else {
        def.props.clone()
    }
}
