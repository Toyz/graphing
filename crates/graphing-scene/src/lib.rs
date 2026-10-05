//! Model -> resolved geometry and style, in world units. Shared by the gpui
//! canvas and the exporters, so it must never depend on a UI toolkit.

pub mod notation;
pub mod anim;
pub mod path;
pub mod pins;
pub mod route;
pub mod wave;
pub mod stencils;

use std::collections::{HashMap, HashSet, VecDeque};

use graphing_model::{Diagram, Point, Rect, Value, find_prop};

pub use notation::{Compartment, End};

pub const GRID: f64 = 10.0;
const MIN_W: f64 = 120.0;
const NODE_H: f64 = 56.0;
const LINE_H: f64 = 18.0;
const CHAR_W: f64 = 7.6;
const GROUP_PAD: f64 = 24.0;
/// Height of a group's title strip above its members.
pub const GROUP_HEAD: f64 = 28.0;
/// Header of a SysML group: stereotype line over the name.
pub const GROUP_HEAD_TALL: f64 = 42.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Rounded,
    Ellipse,
    Diamond,
    Cylinder,
    Parallelogram,
    Hexagon,
    Note,
    Actor,
    /// Header, optional stereotype, compartments (SysML block, part,
    /// requirement; UML class).
    Block,
    /// Filled dot (activity / state initial node).
    Initial,
    /// Bullseye (final node).
    Final,
    /// Solid bar (fork / join).
    Bar,
    /// Folder with a name tab.
    Package,
    /// Head box with a dashed line down (sequence diagrams).
    Lifeline,
    /// Custom outline from a pack's SVG path (see `NodeBox::path`).
    Path,
}

impl Shape {
    /// Stencil path -> built-in shape. Only the last segment matters until
    /// stencil packs land.
    pub fn from_stencil(stencil: Option<&str>) -> Self {
        notation::shape_of(stencil)
    }
}

/// Which side of a node a port sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// A proxy port: a square straddling the node border.
#[derive(Debug, Clone, PartialEq)]
pub struct PortBox {
    pub name: String,
    /// Center, on the border.
    pub at: Point,
    pub side: Side,
    /// A node-graph pin (direction and type) rather than a plain port.
    pub pin: Option<pins::Pin>,
}

pub const PORT: f64 = 11.0;

/// The diagram frame with its header tab (`ibd [block] X [view]`).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameBox {
    pub rect: Rect,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct NodeBox {
    pub id: String,
    pub rect: Rect,
    pub shape: Shape,
    /// Header line (or the whole label for plain shapes).
    pub label: String,
    pub stereotype: Option<String>,
    pub compartments: Vec<Compartment>,
    pub ports: Vec<PortBox>,
    /// Custom outline, fitted to `rect`, when `shape` is `Path`.
    pub path: Option<Vec<path::PathCmd>>,
    pub fill: Option<u32>,
    pub stroke: Option<u32>,
    pub text: Option<u32>,
    /// A picture filling the node (`src: "asset:logo-3f2a.png"` or a file
    /// path relative to the diagram).
    pub image: Option<String>,
    pub fit: ImageFit,
    /// Pack strokes and filled marks over the outline, fitted to `rect`.
    pub detail: Option<Vec<path::PathCmd>>,
    pub mark: Option<Vec<path::PathCmd>>,
    /// A Lucide icon inside the shape.
    pub glyph: Option<stencils::Glyph>,
    /// The label sits under the shape instead of inside it.
    pub label_below: bool,
    /// Outline width, when the stencil sets one.
    pub weight: Option<f64>,
    /// Smaller centred lines under the label.
    pub notes: Vec<String>,
    /// Where the label (and notes) sit, in diagram units.
    pub label_area: Rect,
    /// A timing signal's waveform, drawn instead of an outline.
    pub wave: Option<wave::Wave>,
    /// A link to another diagram (its `src`), drawn as a card with a
    /// miniature of it.
    pub reference: Option<String>,
}

impl NodeBox {
    /// The shape's icon and where it goes.
    pub fn glyph_at(&self) -> Option<(&stencils::Glyph, Rect)> {
        let g = self.glyph.as_ref()?;
        Some((g, g.rect(self.rect, !self.label_below && !self.label.is_empty(), 1.0)))
    }

    /// The node's box plus a label drawn under it.
    pub fn footprint(&self) -> Rect {
        if !self.label_below || self.label.is_empty() {
            return self.rect;
        }
        let lines = self.label.split('\n').count() as f64;
        let w = self.label.split('\n').map(|l| l.chars().count()).max().unwrap_or(0) as f64 * 7.6;
        let r = self.rect;
        let extra = ((w - r.size.w) / 2.0).max(0.0);
        Rect::new(r.origin.x - extra, r.origin.y, r.size.w + extra * 2.0, r.size.h + 6.0 + lines * 18.2)
    }
}

/// Another diagram drawn small: its group frames, shape boxes and lines,
/// fitted (aspect kept, centred) into a box. For diagram links.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Miniature {
    pub groups: Vec<Rect>,
    pub nodes: Vec<Rect>,
    pub lines: Vec<Vec<Point>>,
}

impl Scene {
    /// This scene fitted into `into`.
    pub fn miniature(&self, into: Rect) -> Option<Miniature> {
        let all = self.content_bounds()?;
        let k = (into.size.w / all.size.w.max(1.0)).min(into.size.h / all.size.h.max(1.0));
        let ox = into.origin.x + (into.size.w - all.size.w * k) / 2.0;
        let oy = into.origin.y + (into.size.h - all.size.h * k) / 2.0;
        let p = |q: Point| Point::new(ox + (q.x - all.origin.x) * k, oy + (q.y - all.origin.y) * k);
        let r = |q: Rect| {
            let o = p(q.origin);
            Rect::new(o.x, o.y, q.size.w * k, q.size.h * k)
        };
        Some(Miniature {
            groups: self.groups.iter().map(|g| r(g.rect)).collect(),
            nodes: self.nodes.iter().map(|n| r(n.rect)).collect(),
            lines: self.edges.iter().map(|e| e.points.iter().map(|&q| p(q)).collect()).collect(),
        })
    }
}

/// Where a diagram link's parts go in its box: the title strip and the
/// miniature's area below it.
pub fn link_layout(r: Rect) -> (Rect, Rect) {
    let head = 34.0;
    let pad = 10.0;
    (Rect::new(r.origin.x, r.origin.y, r.size.w, head), Rect::new(r.origin.x + pad, r.origin.y + head, r.size.w - pad * 2.0, (r.size.h - head - pad).max(1.0)))
}

/// How a picture sits in its node (`fit:`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// Whole picture visible, letterboxed.
    #[default]
    Contain,
    /// Fills the node, cropped.
    Cover,
    /// Stretched to the node.
    Fill,
}

impl ImageFit {
    pub fn parse(s: &str) -> Self {
        match s {
            "cover" => ImageFit::Cover,
            "fill" | "stretch" => ImageFit::Fill,
            _ => ImageFit::Contain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ImageFit::Contain => "contain",
            ImageFit::Cover => "cover",
            ImageFit::Fill => "fill",
        }
    }

    /// Where a `(w, h)` picture lands in `r`; may spill out for `Cover`.
    pub fn place(self, r: Rect, w: f64, h: f64) -> Rect {
        if w <= 0.0 || h <= 0.0 || self == ImageFit::Fill {
            return r;
        }
        let k = match self {
            ImageFit::Contain => (r.size.w / w).min(r.size.h / h),
            _ => (r.size.w / w).max(r.size.h / h),
        };
        let (iw, ih) = (w * k, h * k);
        Rect::new(r.origin.x + (r.size.w - iw) / 2.0, r.origin.y + (r.size.h - ih) / 2.0, iw, ih)
    }
}

#[derive(Debug, Clone)]
pub struct GroupBox {
    pub id: String,
    pub rect: Rect,
    pub label: Option<String>,
    pub fill: Option<u32>,
    pub stroke: Option<u32>,
    /// Label color.
    pub text: Option<u32>,
    pub look: GroupLook,
    /// Lucide icon name from the group's kind, drawn in the header.
    pub icon: Option<String>,
    /// `«stereotype»` line of SysML groups.
    pub stereotype: Option<String>,
    /// The kind's fields that are set (`10.0.0.0/16 · us-east-1`), shown
    /// after the name.
    pub details: Option<String>,
}

/// How a group is drawn (`look:` on the group).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupLook {
    /// Dashed outline, the default.
    #[default]
    Dashed,
    Solid,
    /// SysML package: a name tab on a square body.
    Package,
    /// Swimlane: a filled header band.
    Lane,
    /// Tinted area without a border.
    Zone,
    /// Raised card with a header rule.
    Card,
    /// SysML block boundary: `«stereotype»` and name in a compartment.
    Sysml,
}

impl GroupLook {
    pub const ALL: [GroupLook; 7] = [GroupLook::Dashed, GroupLook::Solid, GroupLook::Sysml, GroupLook::Package, GroupLook::Lane, GroupLook::Zone, GroupLook::Card];

    /// As written in `.gph`.
    pub fn name(self) -> &'static str {
        match self {
            GroupLook::Dashed => "dashed",
            GroupLook::Solid => "solid",
            GroupLook::Package => "package",
            GroupLook::Lane => "lane",
            GroupLook::Zone => "zone",
            GroupLook::Card => "card",
            GroupLook::Sysml => "sysml",
        }
    }

    /// Header strip height: room for the label (two lines for SysML).
    pub fn head(self) -> f64 {
        match self {
            GroupLook::Sysml => GROUP_HEAD_TALL,
            _ => GROUP_HEAD,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            GroupLook::Dashed => "Group",
            GroupLook::Solid => "Solid group",
            GroupLook::Package => "Package",
            GroupLook::Lane => "Swimlane",
            GroupLook::Zone => "Zone",
            GroupLook::Card => "Card",
            GroupLook::Sysml => "SysML block",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            GroupLook::Dashed => "Dashed outline",
            GroupLook::Solid => "Solid outline",
            GroupLook::Package => "SysML / UML package with a name tab",
            GroupLook::Lane => "Header band, for lanes and stages",
            GroupLook::Zone => "Tinted area, no border",
            GroupLook::Card => "Raised panel with a header rule",
            GroupLook::Sysml => "Block boundary with a «stereotype» name compartment",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        GroupLook::ALL.into_iter().find(|l| l.name() == s)
    }

    /// A group's look: `look:` if set, then its `kind:`'s, `line: solid` for
    /// older files, else SysML blocks in technical diagrams and dashed
    /// elsewhere.
    pub fn of(props: &graphing_model::Props, technical: bool) -> Self {
        if let Some(l) = find_prop(props, "look").map(Value::text).as_deref().and_then(GroupLook::parse) {
            return l;
        }
        if let Some(kind) = find_prop(props, "kind").map(Value::text)
            && let Some(l) = stencils::registry().group_kind(&kind).and_then(|k| k.look.as_deref().and_then(GroupLook::parse))
        {
            return l;
        }
        match find_prop(props, "line").map(Value::text).as_deref() {
            Some("solid") => GroupLook::Solid,
            _ if technical => GroupLook::Sysml,
            _ => GroupLook::Dashed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EdgeLine {
    pub id: String,
    pub points: Vec<Point>,
    pub head: End,
    pub tail: End,
    pub dashed: bool,
    /// `«flow»`, `«satisfy»` ...
    pub stereotype: Option<String>,
    pub label: Option<String>,
    pub stroke: Option<u32>,
    /// Wired wrong (see `Scene::problems`); the canvas marks it.
    pub problem: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub groups: Vec<GroupBox>,
    pub edges: Vec<EdgeLine>,
    pub nodes: Vec<NodeBox>,
    pub frame: Option<FrameBox>,
    /// Mono compartments, sharp corners, thin strokes.
    pub technical: bool,
    /// Wiring problems: pin directions and types, doubled inputs, loops.
    pub problems: Vec<pins::Problem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Node(String),
    Group(String),
    Edge(String),
}

impl Hit {
    pub fn id(&self) -> &str {
        match self {
            Hit::Node(id) | Hit::Group(id) | Hit::Edge(id) => id,
        }
    }
}

impl Scene {
    /// Topmost thing under `p`: nodes over groups, later over earlier.
    pub fn hit(&self, p: Point) -> Option<Hit> {
        if let Some(n) = self.nodes.iter().rev().find(|n| n.rect.contains(p)) {
            return Some(Hit::Node(n.id.clone()));
        }
        self.groups.iter().rev().find(|g| g.rect.contains(p)).map(|g| Hit::Group(g.id.clone()))
    }

    /// Rect and outline shape of a node or group (groups are rects).
    pub fn outline_of(&self, id: &str) -> Option<(Rect, Shape)> {
        self.nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| (n.rect, n.shape))
            .or_else(|| self.groups.iter().find(|g| g.id == id).map(|g| (g.rect, Shape::Rect)))
    }

    /// Edge whose polyline passes within `tol` of `p`, topmost first.
    pub fn hit_edge(&self, p: Point, tol: f64) -> Option<String> {
        self.edges
            .iter()
            .rev()
            .find(|e| e.points.windows(2).any(|w| seg_dist(p, w[0], w[1]) <= tol))
            .map(|e| e.id.clone())
    }

    /// Nodes whose rect overlaps `r` at all (box select).
    pub fn nodes_touching(&self, r: Rect) -> Vec<String> {
        let hit = |n: &Rect| {
            n.origin.x <= r.origin.x + r.size.w && n.origin.x + n.size.w >= r.origin.x && n.origin.y <= r.origin.y + r.size.h && n.origin.y + n.size.h >= r.origin.y
        };
        self.nodes.iter().filter(|n| hit(&n.rect)).map(|n| n.id.clone()).collect()
    }

    /// What a marquee over `r` selects: nodes it touches, edges it encloses
    /// or cuts across, and groups it fully encloses (so a marquee inside a big container
    /// does not grab the container).
    pub fn marquee(&self, r: Rect) -> Vec<String> {
        let mut out = self.nodes_touching(r);
        // Edges: fully enclosed, or crossed by a band that holds neither end
        // (a marquee around one node leaves its connections alone).
        let inside = |p: &Point| r.contains(*p);
        out.extend(
            self.edges
                .iter()
                .filter(|e| {
                    let (first, last) = (e.points.first(), e.points.last());
                    let ends_in = first.is_some_and(inside) as u8 + last.is_some_and(inside) as u8;
                    ends_in == 2 || (ends_in == 0 && e.points.windows(2).any(|w| seg_hits_rect(w[0], w[1], r)))
                })
                .map(|e| e.id.clone()),
        );
        let within = |g: &Rect| g.origin.x >= r.origin.x && g.origin.y >= r.origin.y && g.origin.x + g.size.w <= r.origin.x + r.size.w && g.origin.y + g.size.h <= r.origin.y + r.size.h;
        out.extend(self.groups.iter().filter(|g| within(&g.rect)).map(|g| g.id.clone()));
        out
    }

    /// Nodes whose rect lies fully inside `r`.
    pub fn nodes_in(&self, r: Rect) -> Vec<String> {
        let inside = |n: &Rect| {
            n.origin.x >= r.origin.x
                && n.origin.y >= r.origin.y
                && n.origin.x + n.size.w <= r.origin.x + r.size.w
                && n.origin.y + n.size.h <= r.origin.y + r.size.h
        };
        self.nodes.iter().filter(|n| inside(&n.rect)).map(|n| n.id.clone()).collect()
    }

    /// Where a connector toward `toward` leaves `id`: the custom outline
    /// when it has one, else the built-in shape.
    fn clip(&self, id: &str, shape: Shape, r: Rect, toward: Point) -> Point {
        let custom = self.nodes.iter().find(|n| n.id == id).and_then(|n| n.path.as_ref());
        match custom.and_then(|cmds| ray_exit(&path::polyline(cmds), r.center(), toward)) {
            Some(p) => p,
            None => boundary(shape, r, toward),
        }
    }

    pub fn port(&self, node: &str, port: &str) -> Option<PortBox> {
        self.nodes.iter().find(|n| n.id == node)?.ports.iter().find(|p| p.name == port).cloned()
    }

    /// A wire's end at `node.port`: where an input and an output share the
    /// name (`exec`), the one facing `dir`.
    pub fn port_toward(&self, node: &str, port: &str, dir: pins::PinDir) -> Option<PortBox> {
        let ports = &self.nodes.iter().find(|n| n.id == node)?.ports;
        ports.iter().find(|p| p.name == port && p.pin.as_ref().is_some_and(|q| q.dir == dir)).or_else(|| ports.iter().find(|p| p.name == port)).cloned()
    }

    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        self.nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.rect)
            .or_else(|| self.groups.iter().find(|g| g.id == id).map(|g| g.rect))
    }

    pub fn bounds(&self) -> Option<Rect> {
        if let Some(f) = &self.frame {
            return Some(f.rect);
        }
        self.content_bounds()
    }

    /// Nodes and groups only, without the frame.
    pub fn content_bounds(&self) -> Option<Rect> {
        // A label under its shape counts as part of it.
        let rects = self.nodes.iter().map(|n| n.footprint()).chain(self.groups.iter().map(|g| g.rect));
        rects.reduce(|a, b| a.union(b))
    }
}

/// Connection handles: top, right, bottom, left side midpoints.
pub fn ports(r: Rect) -> [Point; 4] {
    let c = r.center();
    [
        Point::new(c.x, r.origin.y),
        Point::new(r.origin.x + r.size.w, c.y),
        Point::new(c.x, r.origin.y + r.size.h),
        Point::new(r.origin.x, c.y),
    ]
}

/// Rect spanning two corner points in any order.
pub fn rect_from(a: Point, b: Point) -> Rect {
    Rect::new(a.x.min(b.x), a.y.min(b.y), (a.x - b.x).abs(), (a.y - b.y).abs())
}

/// Segment `a`-`b` touches rect `r` (an end inside, or it crosses a side).
fn seg_hits_rect(a: Point, b: Point, r: Rect) -> bool {
    let (x0, y0, x1, y1) = (r.origin.x, r.origin.y, r.origin.x + r.size.w, r.origin.y + r.size.h);
    let inside = |p: Point| p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1;
    if inside(a) || inside(b) {
        return true;
    }
    let cross = |p: Point, q: Point, s: Point, t: Point| {
        let d = |a: Point, b: Point, c: Point| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        let (d1, d2, d3, d4) = (d(s, t, p), d(s, t, q), d(p, q, s), d(p, q, t));
        (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
    };
    let corners = [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)];
    (0..4).any(|i| cross(a, b, corners[i], corners[(i + 1) % 4]))
}

/// Distance from `p` to segment `a`-`b`.
pub fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.x + t * dx, a.y + t * dy);
    ((p.x - qx).powi(2) + (p.y - qy).powi(2)).sqrt()
}

/// Default box size for a label when the file gives none.
pub fn default_size(label: &str) -> (f64, f64) {
    let lines = label.split('\n');
    let widest = lines.clone().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
    let n = lines.count().max(1) as f64;
    let w = (widest * CHAR_W + 32.0).max(MIN_W);
    ((w / GRID).ceil() * GRID, NODE_H + (n - 1.0) * LINE_H)
}

/// Build the scene. `moved` overrides node and placed-group positions (live
/// drag preview).
pub fn build(d: &Diagram, moved: &HashMap<String, Point>) -> Scene {
    let mut scene = Scene { technical: notation::technical(d), ..Default::default() };
    for n in &d.nodes {
        let spec = notation::node_spec(d, n);
        let (outline, defaults, detail, mark, glyph, label_below, weight, area, waves) = {
            let reg = stencils::registry();
            let def = reg.resolve(n.stencil.as_deref());
            (
                def.path(),
                def.defaults.clone(),
                def.detail.clone(),
                def.mark.clone(),
                def.glyph.clone(),
                def.label == stencils::LabelAt::Below,
                def.weight,
                def.label_area,
                def.render.as_deref() == Some("wave"),
            )
        };
        let reference = {
            let reg = stencils::registry();
            (reg.resolve(n.stencil.as_deref()).render.as_deref() == Some("ref")).then(|| d.node_prop(n, "src").map(Value::text).unwrap_or_default())
        };
        let place = d.layout.get(&n.id);
        let size = place.and_then(|p| p.size).map_or(spec.min, |s| (s.w, s.h));
        let pos = moved.get(&n.id).copied().or(place.map(|p| p.pos)).unwrap_or_default();
        scene.nodes.push(NodeBox {
            id: n.id.clone(),
            rect: Rect::new(pos.x, pos.y, size.0, size.1),
            shape: spec.shape,
            // A link without its own label takes the linked diagram's title.
            label: if reference.is_some() && n.label.is_none() { String::new() } else { spec.title },
            stereotype: spec.stereotype,
            compartments: spec.compartments,
            ports: Vec::new(),
            path: outline.map(|o| o.fit(Rect::new(pos.x, pos.y, size.0, size.1))),
            // Pack defaults apply where the node sets nothing.
            fill: d.node_prop(n, "fill").or_else(|| find_prop(&defaults, "fill")).and_then(color),
            stroke: d.node_prop(n, "stroke").or_else(|| find_prop(&defaults, "stroke")).and_then(color),
            text: d.node_prop(n, "color").or_else(|| find_prop(&defaults, "color")).and_then(color),
            // A link's `src` is a diagram, not a picture.
            image: d.node_prop(n, "src").map(Value::text).filter(|s| !s.is_empty() && reference.is_none()),
            fit: d.node_prop(n, "fit").map(Value::text).map_or_else(ImageFit::default, |s| ImageFit::parse(&s)),
            detail: detail.map(|o| o.fit(Rect::new(pos.x, pos.y, size.0, size.1))),
            mark: mark.map(|o| o.fit(Rect::new(pos.x, pos.y, size.0, size.1))),
            glyph,
            label_below,
            weight,
            notes: spec.notes,
            reference,
            wave: waves.then(|| {
                // `wave` and `data` from the node, else the stencil's defaults.
                let prop = |k: &str| d.node_prop(n, k).or_else(|| find_prop(&defaults, k));
                let text = prop("wave").map(Value::text).unwrap_or_else(|| "x".into());
                let data: Vec<String> = prop("data").map(|v| v.as_list().map_or_else(|| vec![v.text()], |l| l.iter().map(Value::text).collect())).unwrap_or_default();
                let label_w = area.map_or(0.2, |a| a[2]) * size.0;
                wave::layout(&text, &data, Rect::new(pos.x, pos.y, size.0, size.1), label_w)
            }),
            label_area: area.map_or(Rect::new(pos.x, pos.y, size.0, size.1), |[x, y, w, h]| Rect::new(pos.x + x * size.0, pos.y + y * size.1, w * size.0, h * size.1)),
        });
    }

    // Groups fit their members unless placed; inner groups resolve first.
    let technical = scene.technical;
    let mut done: HashMap<String, Rect> = HashMap::new();
    let mut pending: Vec<_> = d.groups.iter().collect();
    for _ in 0..=d.groups.len() {
        pending.retain(|g| {
            if let Some(p) = d.layout.get(&g.id)
                && let Some(s) = p.size
            {
                // A sized group being dragged moves with the drag.
                let pos = moved.get(&g.id).copied().unwrap_or(p.pos);
                done.insert(g.id.clone(), Rect::new(pos.x, pos.y, s.w, s.h));
                return false;
            }
            if g.members.iter().any(|m| d.group(m).is_some() && !done.contains_key(m)) {
                return true;
            }
            let head = GroupLook::of(&g.props, technical).head();
            // Members' labels drawn under them stay inside the frame.
            let footprint = |m: &String| scene.nodes.iter().find(|n| &n.id == m).map(NodeBox::footprint);
            let rects = g.members.iter().filter_map(|m| done.get(m).copied().or_else(|| footprint(m)).or_else(|| scene.rect_of(m)));
            let r = rects.reduce(|a, b| a.union(b)).map_or(Rect::new(0.0, 0.0, 200.0, 120.0), |r| {
                Rect::new(
                    r.origin.x - GROUP_PAD,
                    r.origin.y - GROUP_PAD - head,
                    r.size.w + 2.0 * GROUP_PAD,
                    r.size.h + 2.0 * GROUP_PAD + head,
                )
            });
            done.insert(g.id.clone(), r);
            false
        });
    }
    for g in &d.groups {
        let Some(rect) = done.get(&g.id).copied() else { continue };
        let kind = find_prop(&g.props, "kind").map(Value::text).and_then(|k| stencils::registry().group_kind_in(&k, &d.packs).cloned());
        let prop = |key: &str| find_prop(&g.props, key).or_else(|| kind.as_ref().and_then(|k| find_prop(&k.defaults, key)));
        let look = GroupLook::of(&g.props, scene.technical);
        let stereotype = find_prop(&g.props, "stereotype")
            .map(Value::text)
            .or_else(|| kind.as_ref().and_then(|k| k.stereotype.clone()))
            .or_else(|| (look == GroupLook::Sysml).then(|| "block".to_string()));
        scene.groups.push(GroupBox {
            id: g.id.clone(),
            rect,
            label: g.label.clone(),
            fill: prop("fill").and_then(color),
            stroke: prop("stroke").and_then(color),
            text: prop("color").and_then(color),
            look,
            icon: kind.as_ref().and_then(|k| k.icon.clone()),
            details: kind.as_ref().map(|k| k.props.iter().filter_map(|p| find_prop(&g.props, &p.key).map(Value::text)).filter(|v| !v.is_empty()).collect::<Vec<_>>().join(" \u{b7} ")).filter(|s| !s.is_empty()),
            stereotype,
        });
    }
    // Outer groups paint first.
    scene.groups.sort_by(|a, b| (b.rect.size.w * b.rect.size.h).total_cmp(&(a.rect.size.w * a.rect.size.h)));

    place_ports(d, &mut scene);
    // Sequence diagrams: messages between lifelines run straight across, one
    // row each, in source order.
    let mut message_row = 0usize;
    let parallel = parallel_offsets(d);
    // Elbow lines share one routing grid, made when the first needs it.
    let orthogonal_default = matches!(d.prop("routing").map(Value::text).as_deref(), Some("orthogonal" | "elbow" | "manhattan"));
    let mut grid: Option<route::Grid> = None;
    for e in &d.edges {
        let lifelines = [&e.from, &e.to].map(|id| scene.nodes.iter().find(|n| &n.id == id).filter(|n| n.shape == Shape::Lifeline).map(|n| n.rect));
        if let [Some(a), Some(b)] = lifelines {
            let top = a.origin.y.max(b.origin.y) + MESSAGE_TOP + MESSAGE_STEP * message_row as f64;
            message_row += 1;
            let (x0, x1) = (a.center().x, b.center().x);
            let points = if e.from == e.to {
                // Self message: a small loop to the right.
                vec![Point::new(x0, top), Point::new(x0 + 40.0, top), Point::new(x0 + 40.0, top + 20.0), Point::new(x0, top + 20.0)]
            } else {
                vec![Point::new(x0, top), Point::new(x1, top)]
            };
            let mut spec = notation::edge_spec(d, e);
            match d.edge_prop(e, "kind").map(Value::text).as_deref().map(notation::short_kind) {
                Some("reply" | "return") => {
                    spec.head = End::Open;
                    spec.dashed = true;
                }
                Some("async") => spec.head = End::Open,
                _ => {}
            }
            scene.edges.push(EdgeLine {
                id: e.id.clone(),
                points,
                head: spec.head,
                tail: spec.tail,
                dashed: spec.dashed,
                stereotype: spec.stereotype,
                label: e.label.clone(),
                stroke: d.edge_prop(e, "stroke").and_then(color),
                problem: false,
            });
            continue;
        }
        let (Some((a, sa)), Some((b, sb))) = (scene.outline_of(&e.from), scene.outline_of(&e.to)) else { continue };
        let via = d.waypoints.get(&e.id).cloned().unwrap_or_default();
        // A wire leaves an output and arrives at an input (`<-` the other way).
        let (from_dir, to_dir) = if e.arrow == graphing_model::Arrow::Back { (pins::PinDir::In, pins::PinDir::Out) } else { (pins::PinDir::Out, pins::PinDir::In) };
        let from_tip = e.from_port.as_deref().and_then(|p| scene.port_toward(&e.from, p, from_dir)).map(|p| port_tip(&p));
        let to_tip = e.to_port.as_deref().and_then(|p| scene.port_toward(&e.to, p, to_dir)).map(|p| port_tip(&p));
        let first = via.first().copied().or(to_tip).unwrap_or(b.center());
        let last = via.last().copied().or(from_tip).unwrap_or(a.center());
        let start = from_tip.unwrap_or_else(|| scene.clip(&e.from, sa, a, first));
        let end = to_tip.unwrap_or_else(|| scene.clip(&e.to, sb, b, last));
        let plain = via.is_empty() && from_tip.is_none() && to_tip.is_none();
        let elbow = match d.edge_prop(e, "route").map(Value::text).as_deref() {
            Some("orthogonal" | "elbow") => true,
            Some(_) => false,
            None => orthogonal_default,
        };
        if plain && elbow {
            let g = grid.get_or_insert_with(|| route::Grid::new(&scene.nodes.iter().filter(|n| n.shape != Shape::Lifeline).map(|n| (n.id.clone(), n.rect)).collect::<Vec<_>>()));
            if let Some(points) = g.route(&e.from, &e.to) {
                let spec = notation::edge_spec(d, e);
                scene.edges.push(EdgeLine {
                    id: e.id.clone(),
                    points,
                    head: spec.head,
                    tail: spec.tail,
                    dashed: spec.dashed,
                    stereotype: spec.stereotype,
                    label: e.label.clone(),
                    stroke: d.edge_prop(e, "stroke").and_then(color),
                    problem: false,
                });
                continue;
            }
        }
        // Wires between pins curve out of one and into the other.
        let pin_side = |node: &str, port: &Option<String>, dir| port.as_deref().and_then(|p| scene.port_toward(node, p, dir)).filter(|p| p.pin.is_some());
        let (pa, pb) = (pin_side(&e.from, &e.from_port, from_dir), pin_side(&e.to, &e.to_port, to_dir));
        if via.is_empty() && (pa.is_some() || pb.is_some()) {
            let spec = notation::edge_spec(d, e);
            let pin = pa.as_ref().or(pb.as_ref()).and_then(|p| p.pin.clone());
            // Pins already show which way a wire runs: no arrowheads. Data
            // wires take their type's color, execution wires the line color.
            let typed = pin.filter(|p| !p.exec()).map(|p| pins::color(p.ty.as_deref()));
            scene.edges.push(EdgeLine {
                id: e.id.clone(),
                points: pins::wire(start, pa.map(|p| p.side), end, pb.map(|p| p.side)),
                head: End::None,
                tail: End::None,
                dashed: spec.dashed,
                stereotype: spec.stereotype,
                label: e.label.clone(),
                stroke: d.edge_prop(e, "stroke").and_then(color).or(typed),
                problem: false,
            });
            continue;
        }
        let mut points = vec![start];
        points.extend(via);
        points.push(end);
        // Edges sharing two nodes fan out side by side instead of overlapping.
        if plain && let Some(shift) = parallel.get(&e.id).copied() {
            let (dx, dy) = (end.x - start.x, end.y - start.y);
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let (nx, ny) = (-dy / len * shift, dx / len * shift);
            for p in &mut points {
                p.x += nx;
                p.y += ny;
            }
        }
        let spec = notation::edge_spec(d, e);
        scene.edges.push(EdgeLine {
            id: e.id.clone(),
            points,
            head: spec.head,
            tail: spec.tail,
            dashed: spec.dashed,
            stereotype: spec.stereotype,
            label: e.label.clone(),
            stroke: d.edge_prop(e, "stroke").and_then(color),
            problem: false,
        });
    }
    scene.problems = pins::problems(d);
    for e in &mut scene.edges {
        e.problem = scene.problems.iter().any(|p| p.id == e.id);
    }
    if let Some(title) = notation::frame_title(d) {
        let r = scene.content_bounds().unwrap_or(Rect::new(0.0, 0.0, 400.0, 240.0));
        let (pad, head) = (FRAME_PAD, FRAME_HEAD);
        scene.frame = Some(FrameBox {
            rect: Rect::new(r.origin.x - pad, r.origin.y - pad - head, r.size.w + 2.0 * pad, r.size.h + 2.0 * pad + head),
            title,
        });
    }
    scene
}

/// Perpendicular offset for each edge that shares its node pair with
/// others, centered around the straight line.
fn parallel_offsets(d: &Diagram) -> HashMap<String, f64> {
    let mut pairs: HashMap<(String, String), Vec<(String, bool)>> = HashMap::new();
    for e in d.edges.iter().filter(|e| e.from != e.to) {
        let forward = e.from < e.to;
        let key = if forward { (e.from.clone(), e.to.clone()) } else { (e.to.clone(), e.from.clone()) };
        pairs.entry(key).or_default().push((e.id.clone(), forward));
    }
    let mut out = HashMap::new();
    for list in pairs.values().filter(|l| l.len() > 1) {
        let mid = (list.len() as f64 - 1.0) / 2.0;
        for (i, (id, forward)) in list.iter().enumerate() {
            // The perpendicular flips with direction; undo that so offsets stay distinct.
            let shift = (i as f64 - mid) * PARALLEL_GAP;
            out.insert(id.clone(), if *forward { shift } else { -shift });
        }
    }
    out
}

pub const PARALLEL_GAP: f64 = 18.0;
/// Space between auto-layout columns, wide enough for edge labels.
pub const RANK_GAP: f64 = 140.0;
pub const FRAME_PAD: f64 = 40.0;
/// First message row below a lifeline's top, and the gap between rows.
pub const MESSAGE_TOP: f64 = 70.0;
pub const MESSAGE_STEP: f64 = 40.0;
pub const FRAME_HEAD: f64 = 30.0;

/// Where a connector meets a port: the outer face of its square.
fn port_tip(p: &PortBox) -> Point {
    let h = PORT / 2.0;
    match p.side {
        Side::Top => Point::new(p.at.x, p.at.y - h),
        Side::Bottom => Point::new(p.at.x, p.at.y + h),
        Side::Left => Point::new(p.at.x - h, p.at.y),
        Side::Right => Point::new(p.at.x + h, p.at.y),
    }
}

/// Ports used by edges (and declared in a `ports` list) go on the side that
/// faces what they connect to, spread evenly along it.
fn place_ports(d: &Diagram, scene: &mut Scene) {
    // Node-graph pins have fixed places: inputs on the inflow side.
    let flow = notation::flow(d);
    let mut pinned = std::collections::HashSet::new();
    for n in &d.nodes {
        let list = pins::pins(d, n);
        if list.is_empty() {
            continue;
        }
        let Some(nb) = scene.nodes.iter_mut().find(|b| b.id == n.id) else { continue };
        for (p, (at, side)) in list.iter().zip(pins::place(nb.rect, &list, flow)) {
            nb.ports.push(PortBox { name: p.name.clone(), at, side, pin: Some(p.clone()) });
        }
        pinned.insert(n.id.clone());
    }
    // node -> side -> [(port, coordinate of the far end along that side)]
    let mut want: HashMap<String, HashMap<String, (Side, f64)>> = HashMap::new();
    let center = |s: &Scene, id: &str| s.rect_of(id).map(|r| r.center());
    for e in &d.edges {
        for (node, port, other) in [(&e.from, &e.from_port, &e.to), (&e.to, &e.to_port, &e.from)] {
            if pinned.contains(node) {
                continue;
            }
            let (Some(port), Some(r), Some(o)) = (port, scene.rect_of(node), center(scene, other)) else { continue };
            let c = r.center();
            let (dx, dy) = (o.x - c.x, o.y - c.y);
            let side = if dx.abs() * r.size.h > dy.abs() * r.size.w {
                if dx > 0.0 { Side::Right } else { Side::Left }
            } else if dy > 0.0 {
                Side::Bottom
            } else {
                Side::Top
            };
            let along = if matches!(side, Side::Left | Side::Right) { o.y } else { o.x };
            want.entry(node.clone()).or_default().entry(port.clone()).or_insert((side, along));
        }
    }
    for n in &d.nodes {
        if let Some(items) = d.node_prop(n, "ports").and_then(Value::as_list) {
            for it in items {
                let name = it.text().split(':').next().unwrap_or_default().trim().to_string();
                if !name.is_empty() {
                    want.entry(n.id.clone()).or_default().entry(name).or_insert((Side::Left, f64::MAX));
                }
            }
        }
    }
    for nb in &mut scene.nodes {
        let Some(ports) = want.remove(&nb.id) else { continue };
        let r = nb.rect;
        for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
            let mut on: Vec<(&String, f64)> = ports.iter().filter(|(_, (s, _))| *s == side).map(|(n, (_, a))| (n, *a)).collect();
            on.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(b.0)));
            let k = on.len() as f64;
            for (i, (name, _)) in on.into_iter().enumerate() {
                let f = (i as f64 + 1.0) / (k + 1.0);
                let at = match side {
                    Side::Top => Point::new(r.origin.x + r.size.w * f, r.origin.y),
                    Side::Bottom => Point::new(r.origin.x + r.size.w * f, r.origin.y + r.size.h),
                    Side::Left => Point::new(r.origin.x, r.origin.y + r.size.h * f),
                    Side::Right => Point::new(r.origin.x + r.size.w, r.origin.y + r.size.h * f),
                };
                nb.ports.push(PortBox { name: name.clone(), at, side, pin: None });
            }
        }
    }
}

/// Farthest crossing of the ray from `from` toward `to` with a closed
/// polyline (the outline's exit point).
fn ray_exit(poly: &[Point], from: Point, to: Point) -> Option<Point> {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if poly.len() < 2 || (dx == 0.0 && dy == 0.0) {
        return None;
    }
    let mut best: Option<f64> = None;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ex, ey) = (b.x - a.x, b.y - a.y);
        let den = dx * ey - dy * ex;
        if den.abs() < 1e-12 {
            continue;
        }
        let t = ((a.x - from.x) * ey - (a.y - from.y) * ex) / den;
        let u = ((a.x - from.x) * dy - (a.y - from.y) * dx) / den;
        if t > 0.0 && (0.0..=1.0).contains(&u) {
            best = Some(best.map_or(t, |b: f64| b.max(t)));
        }
    }
    best.map(|t| Point::new(from.x + dx * t, from.y + dy * t))
}

/// Where the ray from the center of `r` towards `toward` leaves the shape.
pub fn boundary(shape: Shape, r: Rect, toward: Point) -> Point {
    let c = r.center();
    let (dx, dy) = (toward.x - c.x, toward.y - c.y);
    let (a, b) = (r.size.w / 2.0, r.size.h / 2.0);
    let t = match shape {
        Shape::Ellipse => 1.0 / ((dx / a).powi(2) + (dy / b).powi(2)).sqrt(),
        Shape::Diamond => 1.0 / (dx.abs() / a + dy.abs() / b),
        _ => return r.boundary_toward(toward),
    };
    if !t.is_finite() {
        return c;
    }
    Point::new(c.x + dx * t, c.y + dy * t)
}

/// Mind-map placement: each root in the middle, its branches around it in
/// rings, every subtree a wedge as wide as its leaves need. `None` when the
/// unplaced nodes do not form a forest.
fn radial(d: &Diagram, unplaced: &HashSet<&str>) -> Option<Vec<(String, Point)>> {
    use std::f64::consts::TAU;
    let size: HashMap<&str, (f64, f64)> = d.nodes.iter().filter(|n| unplaced.contains(n.id.as_str())).map(|n| (n.id.as_str(), notation::node_spec(d, n).min)).collect();
    let mut parent: HashMap<&str, &str> = HashMap::new();
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &d.edges {
        let (f, t) = (e.from.as_str(), e.to.as_str());
        if size.contains_key(f) && size.contains_key(t) && f != t && !parent.contains_key(t) {
            parent.insert(t, f);
            children.entry(f).or_default().push(t);
        }
    }
    let roots: Vec<&str> = d.nodes.iter().map(|n| n.id.as_str()).filter(|id| size.contains_key(id) && !parent.contains_key(id)).collect();
    // A cycle leaves nodes no root reaches: not a forest.
    fn count<'a>(id: &'a str, children: &HashMap<&'a str, Vec<&'a str>>, seen: &mut HashSet<&'a str>) -> usize {
        if !seen.insert(id) {
            return 0;
        }
        let kids = children.get(id).map(Vec::as_slice).unwrap_or_default();
        if kids.is_empty() { 1 } else { kids.iter().map(|k| count(k, children, seen)).sum() }
    }
    let mut seen = HashSet::new();
    let leaves: HashMap<&str, usize> = roots.iter().map(|r| (*r, count(r, &children, &mut seen))).collect();
    if seen.len() != size.len() || roots.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut cx = 0.0;
    for root in roots {
        let total = leaves[root].max(1) as f64;
        // Rings far enough apart for the widest node, and wide enough for
        // every leaf to get about 150 units of arc.
        let (w, h) = size[root];
        let branch = size.iter().filter(|(id, _)| **id != root).map(|(_, s)| s.0).fold(0.0, f64::max);
        let ring = (branch * 0.8 + 70.0).max(190.0);
        let first = (w / 2.0 + branch / 2.0 + 90.0).max(total * 150.0 / TAU);
        let center = Point::new(cx + first + branch, first + branch);
        out.push((root.to_string(), Point::new(center.x - w / 2.0, center.y - h / 2.0)));
        struct Ctx<'a, 'b> {
            size: &'b HashMap<&'a str, (f64, f64)>,
            children: &'b HashMap<&'a str, Vec<&'a str>>,
            center: Point,
            first: f64,
            ring: f64,
            out: &'b mut Vec<(String, Point)>,
        }
        fn leaf_count(id: &str, children: &HashMap<&str, Vec<&str>>) -> usize {
            match children.get(id) {
                Some(k) if !k.is_empty() => k.iter().map(|c| leaf_count(c, children)).sum(),
                _ => 1,
            }
        }
        fn place(c: &mut Ctx, id: &str, from: f64, to: f64, depth: usize) {
            let Some(kids) = c.children.get(id).cloned() else { return };
            let all: usize = kids.iter().map(|k| leaf_count(k, c.children)).sum();
            let mut a = from;
            for k in kids {
                let span = (to - from) * leaf_count(k, c.children) as f64 / all.max(1) as f64;
                let mid = a + span / 2.0;
                let r = c.first + c.ring * (depth as f64 - 1.0);
                let (w, h) = c.size[k];
                let p = Point::new(c.center.x + r * mid.cos() - w / 2.0, c.center.y + r * mid.sin() - h / 2.0);
                c.out.push((k.to_string(), Point::new(p.x.round(), p.y.round())));
                place(c, k, a, a + span, depth + 1);
                a += span;
            }
        }
        let mut c = Ctx { size: &size, children: &children, center, first, ring, out: &mut out };
        // Start at the top and go clockwise.
        place(&mut c, root, -TAU / 4.0, TAU * 3.0 / 4.0, 1);
        cx = center.x + first + ring * 3.0;
    }
    Some(out)
}

/// Tidy tree placement across the flow: leaves in order, each parent
/// centred over its children.
struct TreeLayout<'a, 'b> {
    children: &'b HashMap<&'a str, Vec<&'a str>>,
    breadth: &'b dyn Fn(&str) -> f64,
    between: &'b dyn Fn(&str, &str) -> f64,
    /// Start of each node across the flow.
    out: HashMap<&'a str, f64>,
}

impl<'a> TreeLayout<'a, '_> {
    /// Lay out `id`'s subtree from `start`; returns how much room it takes.
    fn span(&mut self, id: &'a str, start: f64) -> f64 {
        let own = (self.breadth)(id);
        let kids = self.children.get(id).cloned().unwrap_or_default();
        if kids.is_empty() {
            self.out.insert(id, start);
            return own;
        }
        let mut at = start;
        for (i, k) in kids.iter().enumerate() {
            if i > 0 {
                at += (self.between)(kids[i - 1], k);
            }
            at += self.span(k, at);
        }
        let mid = |t: &Self, k: &str| t.out[k] + (t.breadth)(k) / 2.0;
        let centre = (mid(self, kids[0]) + mid(self, kids[kids.len() - 1])) / 2.0;
        // A parent wider than its children starts the span instead.
        let lo = (centre - own / 2.0).max(start);
        self.out.insert(id, lo);
        (at - start).max(lo + own - start)
    }
}

/// Positions for nodes without a placement: layered left to right by
/// longest path from sources, then stacked. Placed nodes are left alone.
pub fn auto_place(d: &Diagram) -> Vec<(String, Point)> {
    let unplaced: HashSet<&str> =
        d.nodes.iter().filter(|n| !d.layout.contains_key(&n.id)).map(|n| n.id.as_str()).collect();
    if unplaced.is_empty() {
        return Vec::new();
    }
    let mut indeg: HashMap<&str, usize> = d.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    for e in &d.edges {
        if let Some(v) = indeg.get_mut(e.to.as_str()) {
            *v += 1;
        }
    }
    let mut rank: HashMap<&str, usize> = HashMap::new();
    let mut queue: VecDeque<&str> = d.nodes.iter().map(|n| n.id.as_str()).filter(|id| indeg[id] == 0).collect();
    while let Some(id) = queue.pop_front() {
        let r = *rank.entry(id).or_insert(0);
        for e in d.edges.iter().filter(|e| e.from == id) {
            let Some(v) = indeg.get_mut(e.to.as_str()) else { continue };
            let next = rank.entry(e.to.as_str()).or_insert(0);
            *next = (*next).max(r + 1);
            *v -= 1;
            if *v == 0 {
                queue.push_back(e.to.as_str());
            }
        }
    }
    if notation::flow(d) == notation::Flow::Radial
        && let Some(out) = radial(d, &unplaced)
    {
        return out;
    }
    let down = notation::flows_down(d);
    // Start past anything already placed so we never overlap it.
    let far = |p: &graphing_model::Placement| if down { p.pos.y + p.size.map_or(NODE_H, |s| s.h) } else { p.pos.x + p.size.map_or(MIN_W, |s| s.w) };
    let d0 = d.layout.values().map(far).fold(0.0f64, f64::max);
    let d0 = if d.layout.is_empty() { 40.0 } else { d0 + 80.0 };
    // Cycles leave some nodes unranked; put them after the deepest rank.
    let after = rank.values().max().map_or(0, |m| m + 1);
    let placed: Vec<(&graphing_model::Node, usize, (f64, f64))> = d
        .nodes
        .iter()
        .filter(|n| unplaced.contains(n.id.as_str()))
        .map(|n| (n, rank.get(n.id.as_str()).copied().unwrap_or(after), notation::node_spec(d, n).min))
        .collect();
    // Along the flow (depth) and across it (breadth), per node.
    let depth_of = |s: (f64, f64)| if down { s.1 } else { s.0 };
    let breadth_of = |s: (f64, f64)| if down { s.0 } else { s.1 };
    let gap_depth = if down { RANK_GAP * 0.6 } else { RANK_GAP };
    let gap_breadth = if down { 40.0 } else { 50.0 };
    // Each rank is as deep as its deepest node, so big boxes never overlap.
    let ranks = placed.iter().map(|p| p.1).max().map_or(0, |m| m + 1);
    let mut at_depth = vec![d0; ranks];
    for r in 1..ranks {
        let deepest = placed.iter().filter(|p| p.1 == r - 1).map(|p| depth_of(p.2)).fold(0.0, f64::max);
        at_depth[r] = at_depth[r - 1] + deepest + gap_depth;
    }
    let point = |depth: f64, breadth: f64| if down { Point::new(breadth, depth) } else { Point::new(depth, breadth) };

    // A label under a node takes room across the flow when the flow runs right.
    let below: HashSet<&str> = placed.iter().filter(|p| notation::label_below(p.0)).map(|p| p.0.id.as_str()).collect();
    let label_room = |id: &str| if !down && below.contains(id) { 24.0 } else { 0.0 };
    // Neighbours in different groups need room for both frames.
    let group_of = |id: &str| d.groups.iter().position(|g| g.members.iter().any(|m| m == id));
    let frame_gap = 2.0 * GROUP_PAD + GROUP_HEAD_TALL;
    let between = |a: &str, b: &str| if group_of(a) != group_of(b) { frame_gap } else { gap_breadth };

    // A tree (one parent each, parents ranked before children) lays out with
    // each parent centred over its children; anything else stacks per rank.
    let size: HashMap<&str, (f64, f64)> = placed.iter().map(|p| (p.0.id.as_str(), p.2)).collect();
    let mut parent: HashMap<&str, &str> = HashMap::new();
    let mut tree = true;
    for e in &d.edges {
        if size.contains_key(e.from.as_str()) && size.contains_key(e.to.as_str()) && rank.get(e.to.as_str()) > rank.get(e.from.as_str()) {
            tree &= parent.insert(e.to.as_str(), e.from.as_str()).is_none();
        }
    }
    if tree && !parent.is_empty() {
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        for (n, ..) in &placed {
            if let Some(p) = parent.get(n.id.as_str()) {
                children.entry(p).or_default().push(n.id.as_str());
            }
        }
        // Siblings of one group side by side.
        for kids in children.values_mut() {
            kids.sort_by_key(|k| group_of(k).map_or(0, |g| g + 1));
        }
        let breadth = |id: &str| breadth_of(size[id]) + label_room(id);
        let mut tree = TreeLayout { children: &children, breadth: &breadth, between: &between, out: HashMap::new() };
        let roots: Vec<&str> = placed.iter().map(|p| p.0.id.as_str()).filter(|id| !parent.contains_key(id)).collect();
        let mut cursor = 40.0;
        for (i, r) in roots.iter().enumerate() {
            if i > 0 {
                cursor += between(roots[i - 1], r) + gap_breadth;
            }
            cursor += tree.span(r, cursor);
        }
        return placed.iter().map(|(n, r, _)| (n.id.clone(), point(at_depth[*r], tree.out[n.id.as_str()]))).collect();
    }

    // Members of one group stay together in a rank.
    let mut placed = placed;
    placed.sort_by_key(|p| (p.1, group_of(&p.0.id).map_or(0, |g| g + 1)));
    let mut per_rank: HashMap<usize, (f64, Option<Option<usize>>)> = HashMap::new();
    let mut out = Vec::new();
    for (n, r, s) in placed {
        let g = group_of(&n.id);
        let (b, last) = per_rank.entry(r).or_insert((40.0, None));
        if last.is_some_and(|l| l != g) {
            *b += frame_gap;
        }
        *last = Some(g);
        out.push((n.id.clone(), point(at_depth[r], *b)));
        *b += breadth_of(s) + gap_breadth + label_room(&n.id);
    }
    out
}

/// `#rgb`, `#rrggbb` or a small set of names -> 0xRRGGBB.
pub fn color(v: &Value) -> Option<u32> {
    let s = v.as_str();
    if let Some(hex) = s.strip_prefix('#') {
        return match hex.len() {
            3 | 4 => {
                let n = u32::from_str_radix(&hex[..3], 16).ok()?;
                let (r, g, b) = ((n >> 8) & 0xf, (n >> 4) & 0xf, n & 0xf);
                Some(((r * 17) << 16) | ((g * 17) << 8) | (b * 17))
            }
            6 | 8 => u32::from_str_radix(&hex[..6], 16).ok(),
            _ => None,
        };
    }
    Some(match s {
        "red" => 0xe03131,
        "orange" => 0xf08c00,
        "yellow" => 0xf5c518,
        "green" => 0x2f9e44,
        "teal" => 0x0c8599,
        "blue" => 0x1c7ed6,
        "indigo" => 0x4263eb,
        "purple" => 0x7048e8,
        "pink" => 0xd6336c,
        "gray" | "grey" => 0x868e96,
        "black" => 0x000000,
        "white" => 0xffffff,
        _ => return None,
    })
}

pub fn snap(v: f64) -> f64 {
    (v / GRID).round() * GRID
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphing_dsl::Document;

    #[test]
    fn mind_maps_branch_all_around_their_centre() {
        let mut d = Diagram::default();
        d.props.push(("flow".into(), Value::Ident("radial".into())));
        for id in ["root", "a", "b", "c", "d", "a1", "a2"] {
            d.nodes.push(graphing_model::Node::new(id));
        }
        for (f, t) in [("root", "a"), ("root", "b"), ("root", "c"), ("root", "d"), ("a", "a1"), ("a", "a2")] {
            d.edges.push(graphing_model::Edge { id: format!("{f}->{t}"), from: f.into(), to: t.into(), ..Default::default() });
        }
        let placed: HashMap<_, _> = auto_place(&d).into_iter().collect();
        let size = |id: &str| notation::node_spec(&d, d.node(id).unwrap()).min;
        let centre = |id: &str| {
            let (p, (w, h)) = (placed[id], size(id));
            Point::new(p.x + w / 2.0, p.y + h / 2.0)
        };
        let c = centre("root");
        let dist = |id: &str| ((centre(id).x - c.x).powi(2) + (centre(id).y - c.y).powi(2)).sqrt();
        // Branches on every side, grandchildren further out than children.
        assert!(["a", "b", "c", "d"].iter().any(|id| centre(id).x < c.x) && ["a", "b", "c", "d"].iter().any(|id| centre(id).x > c.x));
        assert!(["a", "b", "c", "d"].iter().any(|id| centre(id).y < c.y) && ["a", "b", "c", "d"].iter().any(|id| centre(id).y > c.y));
        assert!(dist("a1") > dist("a") + 100.0);
    }

    #[test]
    fn auto_place_ranks_by_edges() {
        let d = Document::parse("a -> b\nb -> c\na -> c\nlone\n");
        let placed: HashMap<_, _> = auto_place(d.diagram()).into_iter().collect();
        assert_eq!(placed.len(), 4);
        assert!(placed["a"].x < placed["b"].x && placed["b"].x < placed["c"].x);
        assert_eq!(placed["lone"].x, placed["a"].x);
    }

    #[test]
    fn group_kinds_bring_design_icon_and_colors() {
        let d = Document::parse("a\nb\ngroup v \"Prod\" { a } { kind: vpc }\ngroup s { b } { kind: subnet, look: card }\n");
        let s = build(d.diagram(), &HashMap::new());
        let v = s.groups.iter().find(|g| g.id == "v").unwrap();
        assert_eq!((v.look, v.icon.as_deref(), v.fill), (GroupLook::Zone, Some("Cloud"), Some(0x4dabf7)));
        // An explicit look wins over the kind's.
        assert_eq!(s.groups.iter().find(|g| g.id == "s").unwrap().look, GroupLook::Card);
        // SysML diagrams default to block boundaries with a stereotype.
        let d = Document::parse("diagram { kind: ibd }\nuse sysml\na\ngroup g \"Bench\" { a }\n");
        let s = build(d.diagram(), &HashMap::new());
        assert_eq!((s.groups[0].look, s.groups[0].stereotype.as_deref()), (GroupLook::Sysml, Some("block")));
    }

    #[test]
    fn marquee_takes_crossed_edges_and_enclosed_groups() {
        let d = Document::parse("a\nb\ngroup g { a }\na -> b\nlayout {\n  a 0 0\n  b 400 0\n}\n");
        let s = build(d.diagram(), &HashMap::new());
        // A thin band across the middle of the edge only.
        let mut hits = s.marquee(Rect::new(200.0, 0.0, 40.0, 60.0));
        hits.sort();
        assert_eq!(hits, ["a->b"]);
        // Around the group: the group and its node; the edge leaves it.
        let g = s.groups[0].rect;
        let mut hits = s.marquee(g.inflate(5.0));
        hits.sort();
        assert_eq!(hits, ["a", "g"]);
        // Around everything: the edge too.
        let mut hits = s.marquee(Rect::new(-100.0, -100.0, 700.0, 300.0));
        hits.sort();
        assert_eq!(hits, ["a", "a->b", "b", "g"]);
    }

    #[test]
    fn groups_wrap_members() {
        let d = Document::parse("a\nb\ngroup g { a b }\nlayout {\n  a 0 0\n  b 300 100\n}\n");
        let s = build(d.diagram(), &HashMap::new());
        let g = s.groups[0].rect;
        assert!(g.contains(Point::new(1.0, 1.0)) && g.contains(Point::new(310.0, 150.0)));
        assert_eq!(s.hit(Point::new(5.0, 5.0)), Some(Hit::Node("a".into())));
        assert_eq!(s.hit(Point::new(200.0, -20.0)), Some(Hit::Group("g".into())));
    }

    #[test]
    fn boundary_follows_shape() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let p = boundary(Shape::Ellipse, r, Point::new(100.0, 50.0));
        // On the ellipse: (x-50)^2/50^2 + (y-25)^2/25^2 == 1
        let v = ((p.x - 50.0) / 50.0).powi(2) + ((p.y - 25.0) / 25.0).powi(2);
        assert!((v - 1.0).abs() < 1e-9);
        let p = boundary(Shape::Diamond, r, Point::new(200.0, 25.0));
        assert_eq!(p, Point::new(100.0, 25.0));
    }

    #[test]
    fn edge_hit_and_marquee() {
        let d = Document::parse("a\nb\na -> b\nlayout {\n  a 0 0\n  b 300 0\n}\n");
        let s = build(d.diagram(), &HashMap::new());
        assert_eq!(s.hit_edge(Point::new(200.0, 30.0), 4.0).as_deref(), Some("a->b"));
        assert_eq!(s.hit_edge(Point::new(200.0, 60.0), 4.0), None);
        assert_eq!(s.nodes_in(rect_from(Point::new(-5.0, -5.0), Point::new(200.0, 100.0))), ["a"]);
        assert_eq!(seg_dist(Point::new(5.0, 5.0), Point::new(0.0, 0.0), Point::new(10.0, 0.0)), 5.0);
    }

    #[test]
    fn sysml_ibd_ports_frame_and_flows() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hil-ibd.gph")).unwrap();
        let d = Document::parse(src);
        assert!(d.diags().is_empty(), "{:?}", d.diags());
        let s = build(d.diagram(), &HashMap::new());
        assert!(s.technical);
        assert_eq!(s.frame.as_ref().unwrap().title, "ibd [block] HIL Test Bench [Architecture]");
        let uut = s.nodes.iter().find(|n| n.id == "uut").unwrap();
        assert_eq!(uut.label, "uut : FlightArticle");
        assert_eq!(uut.shape, Shape::Block);
        assert_eq!(uut.compartments[0].title.as_deref(), Some("parts"));
        assert_eq!(uut.compartments[0].lines.len(), 4);
        let side = |n: &str, p: &str| s.port(n, p).unwrap().side;
        assert_eq!(side("uut", "busPort"), Side::Bottom);
        assert_eq!(side("fe", "busPort"), Side::Top);
        assert_eq!(side("uut", "pwrPort"), Side::Right);
        assert_eq!(side("phys", "rfPort"), Side::Left);
        // Two ports on one side are spread, not stacked.
        assert_ne!(s.port("uut", "pwrPort").unwrap().at, s.port("uut", "rfPort").unwrap().at);
        let flow = s.edges.iter().find(|e| e.id == "uut->fe").unwrap();
        assert_eq!(flow.stereotype.as_deref(), Some("flow"));
        assert_eq!((flow.head, flow.tail), (End::None, End::None));
        // Connector starts on the port face, not the box border.
        assert_eq!(flow.points[0].y, uut.rect.origin.y + uut.rect.size.h + PORT / 2.0);
    }

    #[test]
    fn edge_kinds_and_stencils() {
        let src = "use sysml\nv: sysml.block \"Vehicle\" { values: [\"mass : kg\"] }\nw: sysml.block \"Wheel\"\nr: sysml.requirement \"Range\" { rid: R1, text: \"go far\" }\nv -> w { kind: composition }\nw -> v { kind: generalization }\nv -> r { kind: satisfy }\ni: sysml.initial\n";
        let d = Document::parse(src);
        assert!(d.diags().is_empty(), "{:?}", d.diags());
        let s = build(d.diagram(), &HashMap::new());
        let n = |id: &str| s.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(n("v").stereotype.as_deref(), Some("block"));
        assert_eq!(n("r").compartments[0].lines, ["id = \"R1\"", "text = \"go far\""]);
        assert_eq!(n("i").shape, Shape::Initial);
        let e = |id: &str| s.edges.iter().find(|e| e.id == id).unwrap();
        assert_eq!((e("v->w").head, e("v->w").tail), (End::None, End::FilledDiamond));
        assert_eq!(e("w->v").head, End::Triangle);
        assert!(e("v->r").dashed && e("v->r").stereotype.as_deref() == Some("satisfy"));
    }

    #[test]
    fn colors() {
        assert_eq!(color(&Value::Color("#abc".into())), Some(0xaabbcc));
        assert_eq!(color(&Value::Color("#123456".into())), Some(0x123456));
        assert_eq!(color(&Value::Ident("blue".into())), Some(0x1c7ed6));
        assert_eq!(color(&Value::Ident("nope".into())), None);
    }
}
