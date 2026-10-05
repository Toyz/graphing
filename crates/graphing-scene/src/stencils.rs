//! The stencil registry: every shape, edge kind and diagram kind graphing
//! knows, loaded from packs. Built-in packs (`core`, `sysml`) use the same
//! JSON format as user packs in `<config>/packs/<id>/pack.json`, and plugins
//! register through [`register`], so nothing about a shape is hard-coded.
//!
//! A stencil is referenced in `.gph` files as `<pack>.<name>`
//! (`sysml.block`); `core` stencils also answer to their bare name (`db`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock, RwLockReadGuard};

use graphing_model::{Rect, Value};
use serde::Deserialize;

use crate::Shape;
use crate::path::{Outline, parse};

/// How a node's header line reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Header {
    /// The label (or id).
    #[default]
    Label,
    /// `id : Label`, SysML part style.
    Role,
    /// No text (initial, final, fork).
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PropKind {
    #[default]
    Text,
    LongText,
    List,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PropDef {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub kind: PropKind,
}

/// A text line built from a prop: `format` with `{}` replaced by its value.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FieldDef {
    pub key: String,
    pub format: String,
    /// Wrap at this many characters.
    #[serde(default)]
    pub wrap: Option<usize>,
}

/// Where the outline comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum OutlineDef {
    Builtin(Shape),
    Path(Arc<Outline>),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum RawOutline {
    Name(String),
    Path {
        path: String,
        /// `[x, y, w, h]` of the path's coordinates; default the unit box.
        #[serde(default)]
        view: Option<[f64; 4]>,
    },
}

fn builtin_shape(name: &str) -> Option<Shape> {
    Some(match name {
        "rect" => Shape::Rect,
        "rounded" => Shape::Rounded,
        "ellipse" => Shape::Ellipse,
        "diamond" => Shape::Diamond,
        "cylinder" => Shape::Cylinder,
        "parallelogram" => Shape::Parallelogram,
        "hexagon" => Shape::Hexagon,
        "note" => Shape::Note,
        "actor" => Shape::Actor,
        "block" => Shape::Block,
        "initial" => Shape::Initial,
        "final" => Shape::Final,
        "bar" => Shape::Bar,
        "package" => Shape::Package,
        "lifeline" => Shape::Lifeline,
        _ => return None,
    })
}

/// Where an icon sits inside a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GlyphAt {
    /// Centred, scaled with the shape (devices, events).
    #[default]
    Center,
    /// A small mark in a corner (ArchiMate layers, task types).
    TopLeft,
    TopRight,
    /// Vertically centred at the left edge (avatars on cards).
    Left,
}

/// A Lucide icon drawn inside a shape.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Glyph {
    pub icon: String,
    #[serde(default)]
    pub at: GlyphAt,
    /// Share of the shape's smaller side, for centred icons.
    #[serde(default)]
    pub size: Option<f64>,
}

impl Glyph {
    /// Where the icon goes in `r`. Corner icons are 14 units square, 7 in,
    /// times `unit`. A centred icon whose box also holds the label keeps to
    /// the top 60%.
    pub fn rect(&self, r: Rect, shares_box: bool, unit: f64) -> Rect {
        let (corner, inset) = (14.0 * unit, 7.0 * unit);
        let share = self.size.unwrap_or(0.45);
        match self.at {
            GlyphAt::Center => {
                let area = if shares_box { r.size.h * 0.6 } else { r.size.h };
                let s = r.size.w.min(area) * share;
                Rect::new(r.origin.x + (r.size.w - s) / 2.0, r.origin.y + (area - s) / 2.0, s, s)
            }
            GlyphAt::TopLeft => Rect::new(r.origin.x + inset, r.origin.y + inset, corner, corner),
            GlyphAt::TopRight => Rect::new(r.origin.x + r.size.w - inset - corner, r.origin.y + inset, corner, corner),
            GlyphAt::Left => {
                let s = r.size.h * share;
                Rect::new(r.origin.x + (r.size.h - s) / 2.0, r.origin.y + (r.size.h - s) / 2.0, s, s)
            }
        }
    }
}

/// Where a shape's label goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelAt {
    #[default]
    Inside,
    /// Under the shape (small symbols: events, gateways, devices).
    Below,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum RawPath {
    Data(String),
    Boxed {
        path: String,
        #[serde(default)]
        view: Option<[f64; 4]>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RawStencil {
    name: String,
    title: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    outline: Option<RawOutline>,
    #[serde(default)]
    header: Header,
    #[serde(default)]
    stereotype: Option<String>,
    #[serde(default)]
    compartments: Option<Vec<String>>,
    #[serde(default)]
    fields: Vec<FieldDef>,
    #[serde(default)]
    props: Vec<PropDef>,
    #[serde(default)]
    size: Option<[f64; 2]>,
    #[serde(default)]
    defaults: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    hidden: bool,
    /// Strokes drawn over the outline, in the outline's box.
    #[serde(default)]
    detail: Option<RawPath>,
    /// Filled marks drawn over the outline, in the text color.
    #[serde(default)]
    mark: Option<RawPath>,
    #[serde(default)]
    glyph: Option<Glyph>,
    #[serde(default)]
    label: LabelAt,
    /// Outline stroke width.
    #[serde(default)]
    weight: Option<f64>,
    /// Where the label goes inside the shape, as fractions `[x, y, w, h]`.
    #[serde(default)]
    label_area: Option<[f64; 4]>,
    /// Fields read as centred notes under the label (C4, cards) instead of
    /// a compartment.
    #[serde(default)]
    notes: bool,
    /// Drawn by a notation renderer instead of an outline: `wave`.
    #[serde(default)]
    render: Option<String>,
}

/// One shape, resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct StencilDef {
    /// Full id as written in files: `sysml.block`, or `rect` for core.
    pub id: String,
    pub title: String,
    pub category: String,
    pub pack: String,
    pub outline: OutlineDef,
    pub header: Header,
    pub stereotype: Option<String>,
    /// List props drawn as compartments, in order.
    pub compartments: Vec<String>,
    pub fields: Vec<FieldDef>,
    pub props: Vec<PropDef>,
    pub size: Option<(f64, f64)>,
    /// Props applied when the node does not set them (fill, stroke ...).
    pub defaults: Vec<(String, Value)>,
    /// Lucide icon name for UI lists.
    pub icon: Option<String>,
    pub hidden: bool,
    /// Other names that resolve here, fully qualified.
    pub aliases: Vec<String>,
    /// Strokes over the outline (gateway marks, gate curves).
    pub detail: Option<Arc<Outline>>,
    /// Filled marks over the outline.
    pub mark: Option<Arc<Outline>>,
    pub glyph: Option<Glyph>,
    pub label: LabelAt,
    pub weight: Option<f64>,
    /// Label region as fractions of the box.
    pub label_area: Option<[f64; 4]>,
    /// Fields as centred notes under the label.
    pub notes: bool,
    /// A notation renderer in place of the outline (`wave` for timing).
    pub render: Option<String>,
}

impl StencilDef {
    pub fn shape(&self) -> Shape {
        match self.outline {
            OutlineDef::Builtin(s) => s,
            OutlineDef::Path(_) => Shape::Path,
        }
    }

    pub fn path(&self) -> Option<Arc<Outline>> {
        match &self.outline {
            OutlineDef::Path(o) => Some(o.clone()),
            OutlineDef::Builtin(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EdgeKindDef {
    pub name: String,
    /// Id of the pack that defines it (set on load).
    #[serde(skip)]
    pub pack: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub tail: Option<String>,
    #[serde(default)]
    pub dashed: bool,
    #[serde(default)]
    pub stereotype: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DiagramKindDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Layout direction for new nodes: `right` (default) or `down`.
    #[serde(default)]
    pub flow: Option<String>,
    /// Lines may not form a loop (a DAG); breaking it shows in Problems.
    #[serde(default)]
    pub acyclic: bool,
    #[serde(skip)]
    pub pack: String,
}

/// A group preset: a design, icon and colors under one name
/// (`group g "Prod" { .. } { kind: vpc }`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GroupKindDef {
    pub name: String,
    pub title: String,
    /// Library section; defaults to the pack name.
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: String,
    /// A `GroupLook` name: dashed, solid, sysml, package, lane, zone, card.
    #[serde(default)]
    pub look: Option<String>,
    /// Lucide icon drawn in the header and shown in lists.
    #[serde(default)]
    pub icon: Option<String>,
    /// `«stereotype»` for SysML designs.
    #[serde(default)]
    pub stereotype: Option<String>,
    /// Starting values: style props (`fill`, `stroke`, `color`) apply when
    /// the group sets none; the rest are written into a new group's props.
    #[serde(default, deserialize_with = "props_map")]
    pub defaults: Vec<(String, Value)>,
    /// Fields the inspector offers (`cidr`, `region`...); set values show in
    /// the group's header after its name.
    #[serde(default)]
    pub props: Vec<PropDef>,
    #[serde(skip)]
    pub pack: String,
}

fn props_map<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<(String, Value)>, D::Error> {
    let map: serde_json::Map<String, serde_json::Value> = Deserialize::deserialize(d)?;
    Ok(map.into_iter().filter_map(|(k, v)| json_value(&v).map(|v| (k, v))).collect())
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RawPack {
    id: String,
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    stencils: Vec<RawStencil>,
    #[serde(default)]
    edge_kinds: Vec<EdgeKindDef>,
    #[serde(default)]
    diagram_kinds: Vec<DiagramKindDef>,
    #[serde(default)]
    group_kinds: Vec<GroupKindDef>,
}

/// A pack's identity, for listings.
#[derive(Debug, Clone, PartialEq)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    /// `builtin`, a file path, or a plugin id.
    pub source: String,
    /// Lucide icon for the library's notation picker.
    pub icon: Option<String>,
    pub description: String,
}

/// The packs graphing ships, in library order.
pub const BUILTIN: [(&str, &str); 16] = [
    (include_str!("../packs/core.json"), "core"),
    (include_str!("../packs/sysml.json"), "sysml"),
    (include_str!("../packs/uml.json"), "uml"),
    (include_str!("../packs/c4.json"), "c4"),
    (include_str!("../packs/er.json"), "er"),
    (include_str!("../packs/bpmn.json"), "bpmn"),
    (include_str!("../packs/dfd.json"), "dfd"),
    (include_str!("../packs/control.json"), "control"),
    (include_str!("../packs/fta.json"), "fta"),
    (include_str!("../packs/archimate.json"), "archimate"),
    (include_str!("../packs/es.json"), "es"),
    (include_str!("../packs/org.json"), "org"),
    (include_str!("../packs/timing.json"), "timing"),
    (include_str!("../packs/net.json"), "net"),
    (include_str!("../packs/infra.json"), "infra"),
    (include_str!("../packs/graph.json"), "graph"),
];

/// Compartments a stencil shows when its pack does not say.
const DEFAULT_COMPARTMENTS: &[&str] =
    &["parts", "references", "values", "properties", "attributes", "operations", "constraints", "ports", "flows", "allocations"];

#[derive(Debug, Default)]
pub struct Registry {
    stencils: Vec<StencilDef>,
    /// Full ids, aliases and core bare names -> index.
    names: HashMap<String, usize>,
    pub edge_kinds: Vec<EdgeKindDef>,
    pub diagram_kinds: Vec<DiagramKindDef>,
    pub group_kinds: Vec<GroupKindDef>,
    pub packs: Vec<PackInfo>,
}

impl Registry {
    fn builtin() -> Self {
        let mut r = Registry::default();
        for (text, name) in BUILTIN {
            if let Err(e) = r.add_json(text, "builtin") {
                panic!("built-in pack {name}: {e}");
            }
        }
        r
    }

    /// Parse and add a pack. Later packs override earlier ones by id.
    pub fn add_json(&mut self, text: &str, source: &str) -> Result<String, String> {
        let raw: RawPack = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if raw.id.is_empty() || !raw.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!("pack id `{}` must be letters, digits, - or _", raw.id));
        }
        let mut defs = Vec::new();
        for s in raw.stencils {
            let outline = match s.outline {
                None => OutlineDef::Builtin(Shape::Rect),
                Some(RawOutline::Name(n)) => OutlineDef::Builtin(builtin_shape(&n).ok_or_else(|| format!("{}: unknown outline `{n}`", s.name))?),
                Some(RawOutline::Path { path, view }) => {
                    let cmds = parse(&path).map_err(|e| format!("{}: {e}", s.name))?;
                    let [x, y, w, h] = view.unwrap_or([0.0, 0.0, 1.0, 1.0]);
                    OutlineDef::Path(Arc::new(Outline { cmds, view: Rect::new(x, y, w, h) }))
                }
            };
            // Details share the outline's box unless they name their own.
            let outline_view = match &outline {
                OutlineDef::Path(o) => o.view,
                OutlineDef::Builtin(_) => Rect::new(0.0, 0.0, 1.0, 1.0),
            };
            let overlay = |raw: Option<RawPath>, what: &str| -> Result<Option<Arc<Outline>>, String> {
                let Some(raw) = raw else { return Ok(None) };
                let (d, view) = match raw {
                    RawPath::Data(d) => (d, None),
                    RawPath::Boxed { path, view } => (path, view),
                };
                let cmds = parse(&d).map_err(|e| format!("{} {what}: {e}", s.name))?;
                let view = view.map_or(outline_view, |[x, y, w, h]| Rect::new(x, y, w, h));
                Ok(Some(Arc::new(Outline { cmds, view })))
            };
            let detail = overlay(s.detail, "detail")?;
            let mark = overlay(s.mark, "mark")?;
            let defaults = s.defaults.into_iter().filter_map(|(k, v)| json_value(&v).map(|v| (k, v))).collect();
            let core = raw.id == "core";
            let id = if core { s.name.clone() } else { format!("{}.{}", raw.id, s.name) };
            let aliases = s.aliases.iter().map(|a| if core { a.clone() } else { format!("{}.{a}", raw.id) }).collect();
            defs.push(
                StencilDef {
                    id,
                    title: s.title,
                    category: s.category.unwrap_or_else(|| raw.name.clone()),
                    pack: raw.id.clone(),
                    outline,
                    header: s.header,
                    stereotype: s.stereotype,
                    compartments: s.compartments.unwrap_or_else(|| DEFAULT_COMPARTMENTS.iter().map(|s| s.to_string()).collect()),
                    fields: s.fields,
                    props: s.props,
                    size: s.size.map(|[w, h]| (w, h)),
                    defaults,
                    icon: s.icon,
                    hidden: s.hidden,
                    aliases,
                    detail,
                    mark,
                    glyph: s.glyph,
                    label: s.label,
                    weight: s.weight,
                    label_area: s.label_area,
                    notes: s.notes,
                    render: s.render,
                },
            );
        }
        // Replace a pack loaded before under the same id.
        self.remove_pack(&raw.id);
        self.stencils.extend(defs);
        self.reindex();
        // Kinds of the same name from different packs live side by side; a
        // bare name finds the last loaded, `pack.name` a particular one.
        self.edge_kinds.retain(|k| k.pack != raw.id);
        self.edge_kinds.extend(raw.edge_kinds.into_iter().map(|mut k| {
            k.pack = raw.id.clone();
            k
        }));
        self.diagram_kinds.retain(|k| k.pack != raw.id);
        self.diagram_kinds.extend(raw.diagram_kinds.into_iter().map(|mut k| {
            k.pack = raw.id.clone();
            k
        }));
        self.group_kinds.retain(|k| k.pack != raw.id);
        self.group_kinds.extend(raw.group_kinds.into_iter().map(|mut k| {
            if k.category.is_empty() {
                k.category = raw.name.clone();
            }
            k.pack = raw.id.clone();
            k
        }));
        self.packs.push(PackInfo { id: raw.id.clone(), name: raw.name, version: raw.version, source: source.to_string(), icon: raw.icon, description: raw.description });
        Ok(raw.id)
    }

    fn remove_pack(&mut self, id: &str) {
        if !self.packs.iter().any(|p| p.id == id) {
            return;
        }
        self.packs.retain(|p| p.id != id);
        self.stencils.retain(|s| s.pack != id);
        self.reindex();
    }

    /// Ids win over aliases; earlier aliases win over later ones.
    fn reindex(&mut self) {
        self.names.clear();
        for (i, s) in self.stencils.iter().enumerate() {
            self.names.insert(s.id.clone(), i);
        }
        for (i, s) in self.stencils.iter().enumerate() {
            for a in &s.aliases {
                self.names.entry(a.clone()).or_insert(i);
            }
        }
    }

    /// The stencil for a node's `stencil` field. Unknown names fall back to
    /// their last segment among core stencils, then to `rect`.
    pub fn resolve(&self, stencil: Option<&str>) -> &StencilDef {
        let name = stencil.unwrap_or("rect");
        let i = self
            .names
            .get(name)
            .or_else(|| name.rsplit('.').next().and_then(|last| self.names.get(last)))
            .or_else(|| self.names.get("rect"))
            .copied()
            .unwrap_or(0);
        &self.stencils[i]
    }

    pub fn get(&self, id: &str) -> Option<&StencilDef> {
        self.names.get(id).map(|&i| &self.stencils[i])
    }

    pub fn stencils(&self) -> impl Iterator<Item = &StencilDef> {
        self.stencils.iter()
    }

    /// Library sections: category and its visible stencils, in pack order.
    pub fn catalog(&self) -> Vec<(String, Vec<&StencilDef>)> {
        let mut out: Vec<(String, Vec<&StencilDef>)> = Vec::new();
        for s in self.stencils.iter().filter(|s| !s.hidden) {
            match out.iter_mut().find(|(c, _)| *c == s.category) {
                Some((_, list)) => list.push(s),
                None => out.push((s.category.clone(), vec![s])),
            }
        }
        out
    }

    /// A line kind by `name` or `pack.name`.
    pub fn edge_kind(&self, name: &str) -> Option<&EdgeKindDef> {
        self.edge_kind_in(name, &[])
    }

    /// A line kind as a diagram using `packs` means it: a bare name prefers
    /// those packs, in the order the diagram lists them.
    pub fn edge_kind_in(&self, name: &str, packs: &[String]) -> Option<&EdgeKindDef> {
        find_kind(&self.edge_kinds, name, packs, |k| (&k.pack, &k.name))
    }

    /// A group kind by `name` or `pack.name`.
    pub fn group_kind(&self, name: &str) -> Option<&GroupKindDef> {
        self.group_kind_in(name, &[])
    }

    /// See [`Registry::edge_kind_in`].
    pub fn group_kind_in(&self, name: &str, packs: &[String]) -> Option<&GroupKindDef> {
        find_kind(&self.group_kinds, name, packs, |k| (&k.pack, &k.name))
    }

    /// A diagram kind by `id` or `pack.id`.
    pub fn diagram_kind(&self, id: &str) -> Option<&DiagramKindDef> {
        self.diagram_kind_in(id, &[])
    }

    /// See [`Registry::edge_kind_in`].
    pub fn diagram_kind_in(&self, id: &str, packs: &[String]) -> Option<&DiagramKindDef> {
        find_kind(&self.diagram_kinds, id, packs, |k| (&k.pack, &k.id))
    }

    /// How a file names kind `name` of `pack`: bare unless another pack
    /// defines the same name.
    pub fn kind_ref(&self, pack: &str, name: &str) -> String {
        let edges = self.edge_kinds.iter().map(|k| (&k.pack, &k.name));
        let groups = self.group_kinds.iter().map(|k| (&k.pack, &k.name));
        let diagrams = self.diagram_kinds.iter().map(|k| (&k.pack, &k.id));
        let clash = edges.chain(groups).chain(diagrams).any(|(p, n)| n == name && p != pack);
        if clash { format!("{pack}.{name}") } else { name.to_string() }
    }
}

fn json_value(v: &serde_json::Value) -> Option<Value> {
    Some(match v {
        serde_json::Value::String(s) if s.starts_with('#') => Value::Color(s.clone()),
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Number(n) => Value::Num(n.as_f64()?),
        serde_json::Value::Bool(b) => Value::Ident(b.to_string()),
        serde_json::Value::Array(items) => Value::List(items.iter().filter_map(json_value).collect()),
        _ => return None,
    })
}

/// `pack.name` finds that pack's. A bare name finds the first of `packs`
/// (the diagram's `use` list) that has it, else the last pack loaded.
fn find_kind<'a, K>(kinds: &'a [K], name: &str, packs: &[String], key: impl Fn(&K) -> (&String, &String)) -> Option<&'a K> {
    let of = |pack: &str, short: &str| {
        kinds.iter().rev().find(|k| {
            let (p, n) = key(k);
            p == pack && n == short
        })
    };
    if let Some((pack, short)) = name.rsplit_once('.')
        && let Some(k) = of(pack, short)
    {
        return Some(k);
    }
    packs.iter().find_map(|p| of(p, name)).or_else(|| kinds.iter().rev().find(|k| key(k).1 == name))
}

static REGISTRY: LazyLock<RwLock<Registry>> = LazyLock::new(|| RwLock::new(Registry::builtin()));

/// The live registry (built-in packs plus anything registered since).
pub fn registry() -> RwLockReadGuard<'static, Registry> {
    REGISTRY.read().unwrap_or_else(|e| e.into_inner())
}

/// Add a pack from JSON text. Returns its id.
pub fn register(text: &str, source: &str) -> Result<String, String> {
    REGISTRY.write().unwrap_or_else(|e| e.into_inner()).add_json(text, source)
}

/// Load every `<dir>/<id>/pack.json` (and `<dir>/*.json`). One message per
/// pack that failed; the rest still load.
pub fn load_dir(dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return errors };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| {
            if p.is_dir() {
                Some(p.join("pack.json")).filter(|f| f.exists())
            } else {
                p.extension().is_some_and(|x| x == "json").then_some(p)
            }
        })
        .collect();
    files.sort();
    for f in files {
        let result = std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| register(&t, &f.display().to_string()));
        if let Err(e) = result {
            errors.push(format!("{}: {e}", f.display()));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_resolve() {
        let r = Registry::builtin();
        assert_eq!(r.resolve(Some("sysml.block")).shape(), Shape::Block);
        assert_eq!(r.resolve(Some("db")).id, "db");
        assert_eq!(r.resolve(Some("database")).id, "db");
        assert_eq!(r.resolve(Some("flow.process")).id, "process");
        assert_eq!(r.resolve(Some("nope.nothing")).id, "rect");
        assert_eq!(r.resolve(None).id, "rect");
        assert!(r.edge_kind("satisfy").is_some_and(|k| k.dashed));
        assert!(r.diagram_kind("ibd").is_some());
        assert!(r.catalog().iter().any(|(c, _)| c == "SysML structure"));
    }

    #[test]
    fn builtin_packs_never_clash() {
        let r = Registry::builtin();
        assert_eq!(r.packs.len(), BUILTIN.len());
        let unique = |names: Vec<String>, what: &str| {
            let mut seen = std::collections::HashSet::new();
            for n in names {
                assert!(seen.insert(n.clone()), "{what} `{n}` is defined twice");
            }
        };
        // Kinds are unique within a pack; across packs a diagram's `use`
        // picks (C4 and SysML both have `async`).
        unique(r.edge_kinds.iter().map(|k| format!("{}.{}", k.pack, k.name)).collect(), "edge kind");
        unique(r.group_kinds.iter().map(|k| format!("{}.{}", k.pack, k.name)).collect(), "group kind");
        unique(r.diagram_kinds.iter().map(|k| format!("{}.{}", k.pack, k.id)).collect(), "diagram kind");
        unique(r.stencils().map(|s| s.id.clone()).collect(), "stencil");
        // Edge kinds name only ends that exist.
        for k in &r.edge_kinds {
            for end in k.head.iter().chain(k.tail.iter()) {
                assert!(crate::End::parse(end).is_some(), "{}: unknown end `{end}`", k.name);
            }
        }
    }

    #[test]
    fn notation_packs_resolve_their_stencils() {
        let r = Registry::builtin();
        for id in ["c4.person", "uml.class", "er.table", "er.entity", "bpmn.exclusive", "dfd.store", "control.gain", "fta.and", "archimate.goal", "es.event", "org.person", "net.router"] {
            assert_eq!(r.resolve(Some(id)).id, id.replace("er.entity", "er.table"), "{id}");
        }
        let gw = r.resolve(Some("bpmn.exclusive"));
        assert!(gw.detail.is_some() && gw.label == LabelAt::Below);
        assert!(r.resolve(Some("net.router")).glyph.as_ref().is_some_and(|g| g.icon == "Router"));
        assert!(r.resolve(Some("c4.container")).notes);
        assert!(r.edge_kind("one-to-many").is_some_and(|k| k.head.as_deref() == Some("zero-many")));
    }

    #[test]
    fn custom_pack_with_path_outline_overrides_by_id() {
        let mut r = Registry::builtin();
        let pack = r##"{
            "id": "aws", "name": "AWS",
            "stencils": [
                { "name": "lambda", "title": "Lambda", "category": "AWS Compute",
                  "outline": { "path": "M0 0 H10 L12 5 L10 10 H0 Z", "view": [0, 0, 12, 10] },
                  "size": [140, 60], "defaults": { "fill": "#ff9900" }, "props": [{ "key": "runtime", "label": "Runtime" }] }
            ],
            "edge_kinds": [{ "name": "invokes", "group": "AWS", "head": "open", "dashed": true }]
        }"##;
        assert_eq!(r.add_json(pack, "test").unwrap(), "aws");
        let s = r.resolve(Some("aws.lambda"));
        assert_eq!(s.shape(), Shape::Path);
        assert_eq!(s.size, Some((140.0, 60.0)));
        assert_eq!(s.defaults, vec![("fill".to_string(), Value::Color("#ff9900".into()))]);
        assert!(r.edge_kind("invokes").is_some());
        // Reloading the pack replaces it instead of duplicating.
        r.add_json(&pack.replace("Lambda", "Fn"), "test").unwrap();
        assert_eq!(r.resolve(Some("aws.lambda")).title, "Fn");
        assert_eq!(r.packs.iter().filter(|p| p.id == "aws").count(), 1);
        assert!(r.add_json(r#"{"id": "bad id", "name": "x"}"#, "t").is_err());
        assert!(r.add_json(r#"{"id": "x", "name": "x", "stencils": [{"name": "a", "title": "A", "outline": "blob"}]}"#, "t").is_err());
    }

    #[test]
    fn kinds_of_the_same_name_live_side_by_side() {
        let mut r = Registry::builtin();
        let pack = r#"{"id": "acme", "name": "Acme", "edge_kinds": [{"name": "uses", "head": "diamond"}], "group_kinds": [{"name": "system-boundary", "title": "Acme boundary"}]}"#;
        r.add_json(pack, "t").unwrap();
        // A bare name finds the pack loaded last; `pack.name` a particular one.
        assert_eq!(r.edge_kind("uses").map(|k| k.pack.as_str()), Some("acme"));
        assert_eq!(r.edge_kind("c4.uses").map(|k| k.head.as_deref()), Some(Some("arrow")));
        assert_eq!(r.edge_kind("acme.uses").map(|k| k.head.as_deref()), Some(Some("diamond")));
        assert_eq!(r.group_kind("c4.system-boundary").map(|k| k.pack.as_str()), Some("c4"));
        assert_eq!(r.diagram_kind("sysml.ibd").map(|k| k.id.as_str()), Some("ibd"));
        // Pickers write the qualified name only where it is needed.
        assert_eq!(r.kind_ref("c4", "uses"), "c4.uses");
        assert_eq!(r.kind_ref("sysml", "flow"), "flow");
        // C4 and SysML both have `async`.
        assert_eq!(r.kind_ref("sysml", "async"), "sysml.async");
        assert_eq!(r.edge_kind("sysml.async").map(|k| k.pack.as_str()), Some("sysml"));
        // A diagram that uses SysML means SysML's.
        assert_eq!(r.edge_kind_in("async", &["sysml".into()]).map(|k| k.pack.as_str()), Some("sysml"));
        assert_eq!(r.edge_kind_in("async", &["c4".into()]).map(|k| k.pack.as_str()), Some("c4"));
        // Reloading a pack replaces only its own kinds.
        r.add_json(pack, "t").unwrap();
        assert_eq!(r.edge_kinds.iter().filter(|k| k.name == "uses").count(), 2);
    }
}
