//! Rune plugins.
//!
//! A plugin is a folder with `plugin.json` and a Rune script:
//!
//! ```json
//! { "id": "hello", "name": "Hello", "version": "0.1.0",
//!   "main": "main.rn", "permissions": ["doc.read", "doc.write", "commands"] }
//! ```
//!
//! Each runs on its own thread in its own VM, under an instruction budget
//! and a memory limit, and talks to the app only through messages: the app
//! sends commands to run with a snapshot of the diagram, the script sends
//! back logs, packs, commands it offers and edits to apply. Host modules a
//! plugin was not granted are not installed, so using one fails to compile.
//!
//! Script API (`graphing::...`):
//! - `log::info/warn/error(text)` always
//! - `notify(text)` with `notify`
//! - `stencils::register(pack)` with `stencils` (same shape as pack.json)
//! - `commands::register(id, title, fn)` with `commands`
//! - `doc::title() nodes() edges() selection()` with `doc.read`
//! - `doc::add_node(spec) add_edge(spec) set_label(id, text)
//!   set_prop(id, key, value) move_to(id, x, y) remove(id) set_selection(ids)`
//!   with `doc.write`
//!
//! A script's `main()` runs once when it loads.

mod host;
mod values;

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use graphing_model::{Arrow, Diagram, Edge, Node, Op, Placement, Point, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
pub enum Permission {
    #[serde(rename = "doc.read")]
    DocRead,
    #[serde(rename = "doc.write")]
    DocWrite,
    #[serde(rename = "stencils")]
    Stencils,
    #[serde(rename = "commands")]
    Commands,
    #[serde(rename = "notify")]
    Notify,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_main")]
    pub main: String,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    /// A static pack shipped with the plugin (`pack.json`).
    #[serde(default)]
    pub pack: Option<String>,
}

fn default_main() -> String {
    "main.rn".into()
}

/// Limits every script runs under.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Rune instructions per call (load, each command).
    pub instructions: usize,
    /// Bytes a script may hold.
    pub memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { instructions: 2_000_000, memory: 64 * 1024 * 1024 }
    }
}

/// The diagram as a script sees it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub title: String,
    pub nodes: Vec<NodeSnap>,
    pub edges: Vec<EdgeSnap>,
    pub groups: Vec<GroupSnap>,
    pub selection: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
/// Plain values throughout (empty text, 0.0) so scripts compare without
/// unwrapping options.
pub struct NodeSnap {
    pub id: String,
    pub label: String,
    pub stencil: String,
    pub x: f64,
    pub y: f64,
    pub props: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeSnap {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
    pub props: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupSnap {
    pub id: String,
    pub label: String,
    pub members: Vec<String>,
}

fn props_json(props: &[(String, Value)]) -> serde_json::Map<String, serde_json::Value> {
    props.iter().map(|(k, v)| (k.clone(), value_json(v))).collect()
}

fn value_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Str(s) | Value::Color(s) | Value::Ident(s) => serde_json::Value::String(s.clone()),
        Value::Num(n) => serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_json).collect()),
    }
}

/// JSON from a script back into a prop value.
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

impl Snapshot {
    pub fn of(d: &Diagram, selection: &[String]) -> Self {
        Self {
            title: d.title.clone().unwrap_or_default(),
            nodes: d
                .nodes
                .iter()
                .map(|n| {
                    let p = d.layout.get(&n.id);
                    NodeSnap {
                        id: n.id.clone(),
                        label: n.label.clone().unwrap_or_default(),
                        stencil: n.stencil.clone().unwrap_or_default(),
                        x: p.map_or(0.0, |p| p.pos.x),
                        y: p.map_or(0.0, |p| p.pos.y),
                        props: props_json(&n.props),
                    }
                })
                .collect(),
            edges: d
                .edges
                .iter()
                .map(|e| EdgeSnap { id: e.id.clone(), from: e.from.clone(), to: e.to.clone(), label: e.label.clone().unwrap_or_default(), props: props_json(&e.props) })
                .collect(),
            groups: d.groups.iter().map(|g| GroupSnap { id: g.id.clone(), label: g.label.clone().unwrap_or_default(), members: g.members.clone() }).collect(),
            selection: selection.to_vec(),
        }
    }
}

/// An edit a script asked for. Ids are already unique against the snapshot
/// the script ran on.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    AddNode { id: String, stencil: Option<String>, label: Option<String>, at: Option<(f64, f64)>, props: Vec<(String, serde_json::Value)> },
    AddEdge { from: String, to: String, label: Option<String>, kind: Option<String> },
    SetLabel { id: String, label: Option<String> },
    SetProp { id: String, key: String, value: Option<serde_json::Value> },
    Remove { id: String },
    Move { id: String, x: f64, y: f64 },
    Select(Vec<String>),
}

/// A command a plugin offers.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandInfo {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

pub enum ToScript {
    Run { command: String, snapshot: Snapshot },
    Stop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FromScript {
    /// Loaded; these are its commands.
    Ready { commands: Vec<CommandInfo> },
    Log { level: Level, text: String },
    Notify(String),
    /// A pack to register, as JSON.
    Pack(String),
    /// A command finished; apply these as one undo step.
    Edits { command: String, edits: Vec<Edit> },
    /// Something went wrong; the plugin keeps running.
    Failed(String),
    /// The plugin stopped and will not run again.
    Stopped(String),
}

/// A running plugin.
pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    tx: Sender<ToScript>,
    pub rx: Receiver<FromScript>,
    thread: Option<JoinHandle<()>>,
}

impl Plugin {
    pub fn run(&self, command: &str, snapshot: Snapshot) {
        let _ = self.tx.send(ToScript::Run { command: command.to_string(), snapshot });
    }

    /// Messages the script sent since the last call.
    pub fn drain(&self) -> Vec<FromScript> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.tx.send(ToScript::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(dir.join("plugin.json")).map_err(|e| format!("plugin.json: {e}"))?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("plugin.json: {e}"))?;
    if m.id.is_empty() || !m.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("plugin id `{}` must be letters, digits, - or _", m.id));
    }
    Ok(m)
}

/// Start a plugin's script thread. Its static pack, if any, comes back as
/// the first `Pack` message.
pub fn start(dir: &Path, limits: Limits) -> Result<Plugin, String> {
    let manifest = read_manifest(dir)?;
    let (to_tx, to_rx) = channel();
    let (from_tx, from_rx) = channel();
    if let Some(pack) = &manifest.pack {
        let path = inside(dir, pack)?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{pack}: {e}"))?;
        let _ = from_tx.send(FromScript::Pack(text));
    }
    let main = inside(dir, &manifest.main)?;
    let source = std::fs::read_to_string(&main).map_err(|e| format!("{}: {e}", manifest.main))?;
    let permissions = manifest.permissions.clone();
    let name = manifest.main.clone();
    let thread = std::thread::Builder::new()
        .name(format!("plugin-{}", manifest.id))
        .spawn(move || host::worker(&name, &source, permissions, limits, to_rx, from_tx))
        .map_err(|e| e.to_string())?;
    Ok(Plugin { manifest, dir: dir.to_path_buf(), tx: to_tx, rx: from_rx, thread: Some(thread) })
}

/// A path named in the manifest, kept inside the plugin folder.
fn inside(dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let path = dir.join(rel);
    let real = path.canonicalize().map_err(|e| format!("{rel}: {e}"))?;
    let root = dir.canonicalize().map_err(|e| e.to_string())?;
    if !real.starts_with(&root) {
        return Err(format!("{rel} is outside the plugin folder"));
    }
    Ok(real)
}

/// Start every plugin in `<dir>/<id>/plugin.json`. Failures come back as
/// messages; the others still start.
pub fn start_all(dir: &Path, limits: Limits) -> (Vec<Plugin>, Vec<String>) {
    let mut plugins = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return (plugins, errors) };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join("plugin.json").exists()).collect();
    dirs.sort();
    for d in dirs {
        match start(&d, limits) {
            Ok(p) => plugins.push(p),
            Err(e) => errors.push(format!("{}: {e}", d.display())),
        }
    }
    (plugins, errors)
}

/// Turn a script's edits into ops on `d`, as one batch.
pub fn edits_to_ops(d: &Diagram, edits: &[Edit]) -> (Option<Op>, Option<Vec<String>>) {
    let mut probe = d.clone();
    let mut ops = Vec::new();
    let mut select = None;
    let mut taken: HashSet<String> = d.edges.iter().map(|e| e.id.clone()).collect();
    for edit in edits {
        let op = match edit {
            Edit::AddNode { id, stencil, label, at, props } => {
                let node = Node {
                    id: id.clone(),
                    stencil: stencil.clone(),
                    label: label.clone(),
                    classes: Vec::new(),
                    props: props.iter().filter_map(|(k, v)| Some((k.clone(), json_value(v)?))).collect(),
                };
                let mut batch = vec![Op::AddNode { node, index: probe.nodes.len() }];
                if let Some((x, y)) = at {
                    batch.push(Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos: Point::new(*x, *y), size: None }) });
                }
                Op::Batch(batch)
            }
            Edit::AddEdge { from, to, label, kind } => {
                let id = graphing_model::edge_key(from, to, |k| taken.contains(k));
                taken.insert(id.clone());
                let props = kind.iter().map(|k| ("kind".to_string(), Value::Ident(k.clone()))).collect();
                Op::AddEdge {
                    edge: Edge { id, from: from.clone(), to: to.clone(), arrow: Arrow::Forward, label: label.clone(), props, ..Default::default() },
                    index: probe.edges.len(),
                }
            }
            Edit::SetLabel { id, label } => Op::SetLabel { id: id.clone(), label: label.clone() },
            Edit::SetProp { id, key, value } => Op::SetProp { id: id.clone(), key: key.clone(), value: value.as_ref().and_then(json_value) },
            Edit::Move { id, x, y } => {
                let size = probe.layout.get(id).and_then(|p| p.size);
                Op::SetPlacement { id: id.clone(), placement: Some(Placement { pos: Point::new(*x, *y), size }) }
            }
            Edit::Remove { id } if probe.node(id).is_some() => Op::RemoveNode { id: id.clone() },
            Edit::Remove { id } => Op::RemoveEdge { id: id.clone() },
            Edit::Select(ids) => {
                select = Some(ids.clone());
                continue;
            }
        };
        // Skip what no longer applies instead of failing the whole batch.
        if probe.apply(&op).is_some() {
            ops.push(op);
        }
    }
    ((!ops.is_empty()).then_some(Op::Batch(ops)), select)
}

#[cfg(test)]
mod tests;
