//! The script thread and the host modules a script calls into.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use rune::alloc::limit;
use rune::runtime::{Function, budget};
use rune::{Context, ContextError, Diagnostics, Module, Source, Sources, Value as RuneValue, Vm};

use crate::values::{from_json, to_json};
use crate::{CommandInfo, Edit, FromScript, Level, Limits, Permission, Snapshot, ToScript};

/// Failures in a row before a plugin is stopped.
const STRIKES: usize = 5;

struct Host {
    tx: Sender<FromScript>,
    snapshot: Snapshot,
    /// Node ids in the snapshot plus those added this run.
    taken: HashSet<String>,
    edits: Vec<Edit>,
    commands: Vec<(String, String, Function)>,
    /// Shape -> the function that works out its pins.
    pins: Vec<(String, Function)>,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R {
    HOST.with(|h| f(h.borrow_mut().as_mut().expect("host installed before the script runs")))
}

fn send(m: FromScript) {
    with(|h| {
        let _ = h.tx.send(m);
    });
}

pub(crate) fn worker(name: &str, source: &str, permissions: BTreeSet<Permission>, limits: Limits, rx: Receiver<ToScript>, tx: Sender<FromScript>) {
    HOST.with(|h| {
        *h.borrow_mut() = Some(Host { tx: tx.clone(), snapshot: Snapshot::default(), taken: HashSet::new(), edits: Vec::new(), commands: Vec::new(), pins: Vec::new() })
    });
    let vm = match compile(name, source, &permissions) {
        Ok(vm) => vm,
        Err(e) => {
            let _ = tx.send(FromScript::Stopped(e));
            return;
        }
    };
    if let Ok(main) = vm.lookup_function(["main"])
        && let Err(e) = call(&main, limits)
    {
        let _ = tx.send(FromScript::Stopped(format!("main: {e}")));
        return;
    }
    let commands = with(|h| h.commands.iter().map(|(id, title, _)| CommandInfo { id: id.clone(), title: title.clone() }).collect());
    let pin_providers = with(|h| h.pins.iter().map(|(shape, _)| shape.clone()).collect());
    let _ = tx.send(FromScript::Ready { commands, pin_providers });

    let mut strikes = 0;
    while let Ok(msg) = rx.recv() {
        match msg {
            ToScript::Stop => break,
            ToScript::Pins { shape, key, props } => {
                // Out of the host while it runs, as with commands.
                let providers = with(|h| std::mem::take(&mut h.pins));
                let pins = match providers.iter().find(|(s, _)| *s == shape) {
                    Some((_, f)) => call_with(f, props, limits).and_then(|v| pin_lists(&v)),
                    None => Err(format!("no pins for `{shape}`")),
                };
                with(|h| {
                    let added = std::mem::take(&mut h.pins);
                    h.pins = providers;
                    h.pins.extend(added);
                });
                let _ = tx.send(FromScript::Pins { shape, key, pins });
            }
            ToScript::Run { command, snapshot } => {
                // Out of the host while it runs: the call itself uses the host.
                let commands = with(|h| {
                    h.taken = snapshot.nodes.iter().map(|n| n.id.clone()).chain(snapshot.groups.iter().map(|g| g.id.clone())).collect();
                    h.snapshot = snapshot;
                    h.edits.clear();
                    std::mem::take(&mut h.commands)
                });
                let Some(f) = commands.iter().find(|c| c.0 == command).map(|c| &c.2) else {
                    with(|h| h.commands = commands);
                    let _ = tx.send(FromScript::Failed(format!("no command `{command}`")));
                    continue;
                };
                let result = call(f, limits);
                with(|h| {
                    let added = std::mem::take(&mut h.commands);
                    h.commands = commands;
                    h.commands.extend(added);
                });
                match result {
                    Ok(()) => {
                        strikes = 0;
                        let edits = with(|h| std::mem::take(&mut h.edits));
                        let _ = tx.send(FromScript::Edits { command, edits });
                    }
                    Err(e) => {
                        strikes += 1;
                        let _ = tx.send(FromScript::Failed(format!("{command}: {e}")));
                        if strikes >= STRIKES {
                            let _ = tx.send(FromScript::Stopped(format!("it failed {STRIKES} times in a row")));
                            break;
                        }
                    }
                }
            }
        }
    }
}

fn compile(name: &str, source: &str, permissions: &BTreeSet<Permission>) -> Result<Vm, String> {
    let mut context = Context::with_config(false).map_err(|e| e.to_string())?;
    for m in modules(permissions).map_err(|e| e.to_string())? {
        context.install(m).map_err(|e| e.to_string())?;
    }
    let runtime = Arc::new(context.runtime().map_err(|e| e.to_string())?);
    let mut sources = Sources::new();
    sources.insert(Source::new(name, source).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut diagnostics = Diagnostics::new();
    let unit = rune::prepare(&mut sources).with_context(&context).with_diagnostics(&mut diagnostics).build();
    let mut buffer = rune::termcolor::Buffer::no_color();
    let _ = diagnostics.emit(&mut buffer, &sources);
    let text = String::from_utf8_lossy(buffer.as_slice()).trim_end().to_string();
    match unit {
        Ok(unit) => Ok(Vm::new(runtime, Arc::new(unit))),
        Err(_) => Err(if text.is_empty() { "it doesn't compile".into() } else { text }),
    }
}

/// Call `f(arg)` under the limits and return what it gives back as JSON.
fn call_with(f: &Function, arg: serde_json::Value, limits: Limits) -> Result<serde_json::Value, String> {
    budget::with(
        limits.instructions,
        limit::with(limits.memory, || {
            let arg = from_json(&arg)?;
            let value = f.call::<RuneValue>((arg,)).into_result().map_err(|e| e.to_string())?;
            if let Ok(Err(why)) = rune::from_value::<Result<RuneValue, RuneValue>>(value.clone()) {
                return Err(to_json(&why).map(|j| j.as_str().map_or(j.to_string(), str::to_string)).unwrap_or_else(|e| e));
            }
            let value = rune::from_value::<Result<RuneValue, RuneValue>>(value.clone()).ok().and_then(Result::ok).unwrap_or(value);
            to_json(&value)
        }),
    )
    .call()
}

/// `#{ inputs: [..], outputs: [..] }` (or `"in"`, `"out"`), items
/// `"name: type"` or `#{ name, type }`.
fn pin_lists(v: &serde_json::Value) -> Result<(Vec<String>, Vec<String>), String> {
    // `in` is a keyword in Rune, so `inputs`/`outputs` work as keys too.
    let list = |key: &str| -> Result<Vec<String>, String> {
        let long = if key == "in" { "inputs" } else { "outputs" };
        let Some(items) = v.get(key).or_else(|| v.get(long)) else { return Ok(Vec::new()) };
        let items = items.as_array().ok_or_else(|| format!("`{key}` should be a list"))?;
        items
            .iter()
            .map(|it| match it {
                serde_json::Value::String(s) => Ok(s.clone()),
                serde_json::Value::Object(o) => {
                    let name = o.get("name").and_then(|n| n.as_str()).ok_or("a pin needs a `name`")?;
                    Ok(match o.get("type").and_then(|t| t.as_str()) {
                        Some(t) => format!("{name}: {t}"),
                        None => name.to_string(),
                    })
                }
                _ => Err(format!("`{key}` items are \"name: type\" or #{{ name, type }}")),
            })
            .collect()
    };
    if !v.is_object() {
        return Err("pins should be #{ inputs: [..], outputs: [..] }".into());
    }
    Ok((list("in")?, list("out")?))
}

/// Run a script function under the instruction budget and memory limit.
fn call(f: &Function, limits: Limits) -> Result<(), String> {
    budget::with(
        limits.instructions,
        limit::with(limits.memory, || {
            let value = f.call::<RuneValue>(()).into_result().map_err(|e| e.to_string())?;
            // `Err(why)` from the script is a failure that says why.
            if let Ok(Err(why)) = rune::from_value::<Result<RuneValue, RuneValue>>(value) {
                return Err(to_json(&why).map(|j| j.as_str().map_or(j.to_string(), str::to_string)).unwrap_or_else(|e| e));
            }
            Ok(())
        }),
    )
    .call()
}

fn modules(permissions: &BTreeSet<Permission>) -> Result<Vec<Module>, ContextError> {
    let mut out = Vec::new();
    let mut log = Module::with_crate_item("graphing", ["log"])?;
    log.function("info", |t: &str| send(FromScript::Log { level: Level::Info, text: t.to_string() })).build()?;
    log.function("warn", |t: &str| send(FromScript::Log { level: Level::Warn, text: t.to_string() })).build()?;
    log.function("error", |t: &str| send(FromScript::Log { level: Level::Error, text: t.to_string() })).build()?;
    out.push(log);

    if permissions.contains(&Permission::Notify) {
        let mut root = Module::with_crate("graphing")?;
        root.function("notify", |t: &str| send(FromScript::Notify(t.to_string()))).build()?;
        out.push(root);
    }
    if permissions.contains(&Permission::Stencils) {
        let mut m = Module::with_crate_item("graphing", ["stencils"])?;
        m.function("register", stencils_register).build()?;
        out.push(m);
    }
    if permissions.contains(&Permission::Pins) {
        let mut m = Module::with_crate_item("graphing", ["pins"])?;
        m.function("provide", |shape: &str, f: Function| with(|h| h.pins.push((shape.to_string(), f)))).build()?;
        out.push(m);
    }
    if permissions.contains(&Permission::Commands) {
        let mut m = Module::with_crate_item("graphing", ["commands"])?;
        m.function("register", |id: &str, title: &str, f: Function| with(|h| h.commands.push((id.to_string(), title.to_string(), f)))).build()?;
        out.push(m);
    }
    let read = permissions.contains(&Permission::DocRead);
    let write = permissions.contains(&Permission::DocWrite);
    if read || write {
        let mut m = Module::with_crate_item("graphing", ["doc"])?;
        if read {
            m.function("title", || with(|h| h.snapshot.title.clone())).build()?;
            m.function("nodes", || snapshot_part(|s| serde_json::to_value(&s.nodes))).build()?;
            m.function("edges", || snapshot_part(|s| serde_json::to_value(&s.edges))).build()?;
            m.function("groups", || snapshot_part(|s| serde_json::to_value(&s.groups))).build()?;
            m.function("selection", || snapshot_part(|s| serde_json::to_value(&s.selection))).build()?;
        }
        if write {
            m.function("add_node", doc_add_node).build()?;
            m.function("add_edge", doc_add_edge).build()?;
            m.function("set_label", |id: &str, label: RuneValue| {
                let label = to_json(&label).ok().and_then(|j| j.as_str().map(str::to_string)).filter(|s| !s.is_empty());
                with(|h| h.edits.push(Edit::SetLabel { id: id.to_string(), label }));
            })
            .build()?;
            m.function("set_prop", |id: &str, key: &str, value: RuneValue| -> Result<(), String> {
                let value = Some(to_json(&value)?).filter(|v| !v.is_null());
                with(|h| h.edits.push(Edit::SetProp { id: id.to_string(), key: key.to_string(), value }));
                Ok(())
            })
            .build()?;
            m.function("remove", |id: &str| with(|h| h.edits.push(Edit::Remove { id: id.to_string() }))).build()?;
            m.function("move_to", |id: &str, x: f64, y: f64| with(|h| h.edits.push(Edit::Move { id: id.to_string(), x, y }))).build()?;
            m.function("set_selection", |ids: RuneValue| -> Result<(), String> {
                let ids = to_json(&ids)?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                with(|h| h.edits.push(Edit::Select(ids)));
                Ok(())
            })
            .build()?;
        }
        out.push(m);
    }
    Ok(out)
}

/// Part of the snapshot as a plain value; reads never fail (an empty list
/// at worst), so scripts need no `?` on them.
fn snapshot_part(f: impl FnOnce(&Snapshot) -> serde_json::Result<serde_json::Value>) -> RuneValue {
    let json = with(|h| f(&h.snapshot)).unwrap_or(serde_json::Value::Array(Vec::new()));
    from_json(&json).or_else(|_| rune::to_value(rune::runtime::Vec::new())).expect("empty list converts")
}

fn stencils_register(pack: RuneValue) -> Result<(), String> {
    let json = to_json(&pack)?;
    let text = json.to_string();
    // Check it now, so the script hears what is wrong.
    graphing_scene::stencils::Registry::default().add_json(&text, "check")?;
    send(FromScript::Pack(text));
    Ok(())
}

fn field_str(obj: &serde_json::Value, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.is_empty())
}

/// `doc::add_node(#{ stencil, label, x, y, id, props })` -> the new id.
fn doc_add_node(spec: RuneValue) -> Result<String, String> {
    let spec = to_json(&spec)?;
    let wanted = field_str(&spec, "id");
    let at = match (spec.get("x").and_then(|v| v.as_f64()), spec.get("y").and_then(|v| v.as_f64())) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    let props: Vec<(String, serde_json::Value)> = spec.get("props").and_then(|p| p.as_object()).map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
    with(|h| {
        let id = match wanted.filter(|w| !h.taken.contains(w)) {
            Some(w) => w,
            None => (1..).map(|n| format!("n{n}")).find(|c| !h.taken.contains(c)).expect("unbounded"),
        };
        h.taken.insert(id.clone());
        h.edits.push(Edit::AddNode { id: id.clone(), stencil: field_str(&spec, "stencil"), label: field_str(&spec, "label"), at, props });
        Ok(id)
    })
}

/// `doc::add_edge(#{ from, to, label, kind })`.
fn doc_add_edge(spec: RuneValue) -> Result<(), String> {
    let spec = to_json(&spec)?;
    let from = field_str(&spec, "from").ok_or("add_edge needs `from`")?;
    let to = field_str(&spec, "to").ok_or("add_edge needs `to`")?;
    with(|h| h.edits.push(Edit::AddEdge { from, to, label: field_str(&spec, "label"), kind: field_str(&spec, "kind") }));
    Ok(())
}
