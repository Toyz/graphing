//! Diagram model. Renderer and format agnostic.
//!
//! Everything is addressed by stable string ids taken from the source file.
//! Mutation goes through [`Op`] so undo, text patching and (later) sync all
//! share one vocabulary.

mod geom;
mod op;

pub use geom::{Point, Rect, Size};
pub use op::Op;

use std::collections::BTreeMap;

/// Ordered map so iteration follows source order.
pub type Props = Vec<(String, Value)>;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Color(String),
    Ident(String),
    /// `[a, "b", 3]`; compartments, item lists, ports.
    List(Vec<Value>),
}

impl Value {
    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) | Value::Color(s) | Value::Ident(s) => s,
            Value::Num(_) | Value::List(_) => "",
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(v) => Some(v),
            _ => None,
        }
    }

    /// Display text: strings as written, numbers trimmed, lists joined.
    pub fn text(&self) -> String {
        match self {
            Value::Num(n) => {
                if n.fract() == 0.0 { format!("{}", *n as i64) } else { n.to_string() }
            }
            Value::List(v) => v.iter().map(Value::text).collect::<Vec<_>>().join(", "),
            other => other.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Arrow {
    #[default]
    /// `->`
    Forward,
    /// `<-`
    Back,
    /// `<->`
    Both,
    /// `--`
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    /// Stencil path like `core.rect` or `flow.process`. `None` means default.
    pub stencil: Option<String>,
    pub label: Option<String>,
    pub classes: Vec<String>,
    pub props: Props,
}

impl Node {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), stencil: None, label: None, classes: Vec::new(), props: Vec::new() }
    }

    /// Text shown on the canvas: label, else id.
    pub fn text(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edge {
    /// Explicit id from the file, or a generated `from->to#n` key.
    pub id: String,
    pub from: String,
    pub to: String,
    /// Port on `from` (`node.port` endpoint), if any.
    pub from_port: Option<String>,
    pub to_port: Option<String>,
    pub arrow: Arrow,
    pub label: Option<String>,
    pub classes: Vec<String>,
    pub props: Props,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    pub label: Option<String>,
    pub members: Vec<String>,
    pub props: Props,
}

/// What a step of an animation does to its targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Fade in (and stay). Anything a `show` names is hidden before it.
    Show,
    /// Fade out.
    Hide,
    /// Dashes run along the edges, during the step.
    Flow,
    /// The camera frames the targets.
    Focus,
    /// The targets glow, during the step.
    Highlight,
}

impl Verb {
    pub const ALL: [Verb; 5] = [Verb::Show, Verb::Hide, Verb::Flow, Verb::Focus, Verb::Highlight];

    pub fn name(self) -> &'static str {
        match self {
            Verb::Show => "show",
            Verb::Hide => "hide",
            Verb::Flow => "flow",
            Verb::Focus => "focus",
            Verb::Highlight => "highlight",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Verb::ALL.into_iter().find(|v| v.name() == s)
    }
}

/// One line of a step: `flow user -> web, web -> api`.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub verb: Verb,
    /// Node, group or edge ids (`a->b` for unnamed edges).
    pub targets: Vec<String>,
}

/// How a step's fades and moves speed up and slow down (`ease snappy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ease {
    /// Slow, fast, slow.
    #[default]
    Smooth,
    /// Constant speed.
    Linear,
    /// Fast start, long settle.
    Snappy,
    /// Overshoots a little, then settles.
    Bounce,
}

impl Ease {
    pub const ALL: [Ease; 4] = [Ease::Smooth, Ease::Linear, Ease::Snappy, Ease::Bounce];

    pub fn name(self) -> &'static str {
        match self {
            Ease::Smooth => "smooth",
            Ease::Linear => "linear",
            Ease::Snappy => "snappy",
            Ease::Bounce => "bounce",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Ease::ALL.into_iter().find(|e| e.name() == s)
    }

    /// Progress at `t` (0..1) of the way through a transition.
    pub fn at(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::Smooth => {
                if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
            }
            Ease::Snappy => 1.0 - (1.0 - t).powi(4),
            Ease::Bounce => {
                // Back-out: past the target by about 10%, then home.
                let c = 1.70158;
                1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2)
            }
        }
    }
}

/// One step of the diagram's animation (`animate { step "..." 2s { .. } }`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Step {
    pub title: Option<String>,
    /// How long it holds; the player's default when `None`.
    pub seconds: Option<f64>,
    /// `None` for the default (smooth).
    pub ease: Option<Ease>,
    pub actions: Vec<Action>,
    /// `move a 300 120`: where shapes travel to during the step.
    pub moves: Vec<(String, Point)>,
}

/// Geometry for one node or group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub pos: Point,
    pub size: Option<Size>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Diagram {
    pub title: Option<String>,
    /// `diagram "t" { kind: ibd, look: technical }`.
    pub props: Props,
    pub packs: Vec<String>,
    pub styles: BTreeMap<String, Props>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub groups: Vec<Group>,
    /// Node and group placements keyed by id.
    pub layout: BTreeMap<String, Placement>,
    /// Edge waypoints keyed by edge id.
    pub waypoints: BTreeMap<String, Vec<Point>>,
    /// The `animate { }` steps, in order.
    pub steps: Vec<Step>,
}

impl Diagram {
    pub fn prop(&self, key: &str) -> Option<&Value> {
        find_prop(&self.props, key)
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn edge(&self, id: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.id == id)
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Resolve a prop for a node: inline props win over classes, later classes
    /// win over earlier ones.
    pub fn node_prop<'a>(&'a self, node: &'a Node, key: &str) -> Option<&'a Value> {
        if let Some(v) = find_prop(&node.props, key) {
            return Some(v);
        }
        node.classes.iter().rev().find_map(|c| self.styles.get(c).and_then(|p| find_prop(p, key)))
    }

    /// Edge counterpart of [`Diagram::node_prop`].
    pub fn edge_prop<'a>(&'a self, edge: &'a Edge, key: &str) -> Option<&'a Value> {
        if let Some(v) = find_prop(&edge.props, key) {
            return Some(v);
        }
        edge.classes.iter().rev().find_map(|c| self.styles.get(c).and_then(|p| find_prop(p, key)))
    }

    /// Apply an op to the model. Returns the inverse op for undo, or `None`
    /// when the op did not apply (unknown id, duplicate id).
    pub fn apply(&mut self, op: &Op) -> Option<Op> {
        op::apply(self, op)
    }
}

pub fn find_prop<'a>(props: &'a Props, key: &str) -> Option<&'a Value> {
    props.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

#[cfg(test)]
mod tests;
