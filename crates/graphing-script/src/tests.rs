use std::path::PathBuf;
use std::time::Duration;

use super::*;

fn plugin_dir(name: &str, manifest: &str, script: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("graphing-plugin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.json"), manifest).unwrap();
    std::fs::write(dir.join("main.rn"), script).unwrap();
    dir
}

/// Messages until `pred` matches one (or a timeout).
fn until(p: &Plugin, pred: impl Fn(&FromScript) -> bool) -> Vec<FromScript> {
    let mut got = Vec::new();
    for _ in 0..200 {
        if let Ok(m) = p.rx.recv_timeout(Duration::from_millis(25)) {
            let done = pred(&m);
            got.push(m);
            if done {
                return got;
            }
        }
    }
    panic!("timed out; got {got:?}");
}

const FULL: &str = r#"{ "id": "demo", "name": "Demo", "main": "main.rn",
  "permissions": ["doc.read", "doc.write", "commands", "stencils", "notify"] }"#;

#[test]
fn registers_pack_and_command_then_edits() {
    let script = r##"
use graphing::{log, commands, doc, stencils};

pub fn main() {
    log::info("hello");
    stencils::register(#{ id: "demo", name: "Demo", stencils: [#{ name: "chip", title: "Chip", outline: "rounded" }] })?;
    commands::register("pair", "Add a pair", add_pair);
}

pub fn add_pair() {
    let n = doc::nodes().len();
    let a = doc::add_node(#{ label: "A", x: 0.0, y: 0.0 })?;
    let b = doc::add_node(#{ label: "B", x: 200.0, y: 0.0, stencil: "demo.chip", props: #{ fill: "#ffeeaa" } })?;
    doc::add_edge(#{ from: a, to: b, label: "link", kind: "flow" })?;
    doc::set_label(a, `A of ${n}`);
    doc::set_selection([a, b]);
}
"##;
    let dir = plugin_dir("full", FULL, script);
    let p = start(&dir, Limits::default()).unwrap();
    let msgs = until(&p, |m| matches!(m, FromScript::Ready { .. }));
    assert!(msgs.iter().any(|m| matches!(m, FromScript::Log { level: Level::Info, text } if text == "hello")));
    assert!(msgs.iter().any(|m| matches!(m, FromScript::Pack(t) if t.contains("chip"))));
    let Some(FromScript::Ready { commands }) = msgs.last() else { unreachable!() };
    assert_eq!(commands, &[CommandInfo { id: "pair".into(), title: "Add a pair".into() }]);

    // The snapshot has n1 taken, so new ids skip it.
    let mut d = Diagram { nodes: vec![Node::new("n1")], ..Default::default() };
    p.run("pair", Snapshot::of(&d, &[]));
    let msgs = until(&p, |m| matches!(m, FromScript::Edits { .. } | FromScript::Failed(_)));
    let Some(FromScript::Edits { edits, .. }) = msgs.last() else { panic!("{msgs:?}") };
    let (op, select) = edits_to_ops(&d, edits);
    d.apply(&op.unwrap()).unwrap();
    assert_eq!(d.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["n1", "n2", "n3"]);
    assert_eq!(d.node("n2").unwrap().label.as_deref(), Some("A of 1"));
    assert_eq!(d.node("n3").unwrap().stencil.as_deref(), Some("demo.chip"));
    assert_eq!(d.node("n3").unwrap().props, vec![("fill".to_string(), Value::Color("#ffeeaa".into()))]);
    let e = d.edge("n2->n3").unwrap();
    assert_eq!(e.label.as_deref(), Some("link"));
    assert_eq!(d.layout["n3"].pos, Point::new(200.0, 0.0));
    assert_eq!(select, Some(vec!["n2".to_string(), "n3".to_string()]));
}

#[test]
fn ungranted_modules_do_not_exist() {
    let manifest = r#"{ "id": "ro", "name": "Read only", "permissions": ["doc.read"] }"#;
    let script = "use graphing::doc;\npub fn main() { doc::add_node(#{ label: \"x\" }); }\n";
    let dir = plugin_dir("ro", manifest, script);
    let p = start(&dir, Limits::default()).unwrap();
    let msgs = until(&p, |m| matches!(m, FromScript::Stopped(_) | FromScript::Ready { .. }));
    assert!(matches!(msgs.last(), Some(FromScript::Stopped(why)) if why.contains("add_node")), "{msgs:?}");
}

#[test]
fn runaway_scripts_hit_the_budget() {
    let script = "use graphing::commands;\npub fn main() { commands::register(\"spin\", \"Spin\", spin); }\npub fn spin() { loop {} }\n";
    let dir = plugin_dir("spin", FULL, script);
    let p = start(&dir, Limits { instructions: 10_000, ..Limits::default() }).unwrap();
    until(&p, |m| matches!(m, FromScript::Ready { .. }));
    p.run("spin", Snapshot::default());
    let msgs = until(&p, |m| matches!(m, FromScript::Failed(_) | FromScript::Edits { .. }));
    assert!(matches!(msgs.last(), Some(FromScript::Failed(_))), "{msgs:?}");
}

#[test]
fn bad_manifests_and_escapes_are_refused() {
    let dir = plugin_dir("bad", r#"{ "id": "bad id", "name": "x" }"#, "");
    assert!(start(&dir, Limits::default()).is_err());
    let dir = plugin_dir("escape", r#"{ "id": "esc", "name": "x", "main": "../../../../../../etc/hostname" }"#, "");
    match start(&dir, Limits::default()) {
        Err(e) => assert!(e.contains("outside") || e.contains("No such file"), "{e}"),
        Ok(_) => panic!("a script outside its folder must not load"),
    }
}

#[test]
fn example_plugin_loads_and_numbers_requirements() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/sysml-tools");
    let p = start(&dir, Limits::default()).unwrap();
    let msgs = until(&p, |m| matches!(m, FromScript::Ready { .. } | FromScript::Stopped(_)));
    let Some(FromScript::Ready { commands }) = msgs.last() else { panic!("{msgs:?}") };
    assert_eq!(commands.len(), 2);
    assert!(msgs.iter().any(|m| matches!(m, FromScript::Pack(t) if t.contains("callout"))));
    let mut d = Diagram::default();
    for (id, y) in [("b", 200.0), ("a", 0.0)] {
        let node = Node { stencil: Some("sysml.requirement".into()), ..Node::new(id) };
        d.apply(&Op::AddNode { node, index: 9 }).unwrap();
        d.apply(&Op::SetPlacement { id: id.into(), placement: Some(Placement { pos: Point::new(0.0, y), size: None }) }).unwrap();
    }
    p.run("number-requirements", Snapshot::of(&d, &[]));
    let msgs = until(&p, |m| matches!(m, FromScript::Edits { .. } | FromScript::Failed(_)));
    let Some(FromScript::Edits { edits, .. }) = msgs.last() else { panic!("{msgs:?}") };
    let (op, _) = edits_to_ops(&d, edits);
    d.apply(&op.unwrap()).unwrap();
    assert_eq!(d.node("a").unwrap().props, vec![("rid".to_string(), Value::Str("R1".into()))]);
    assert_eq!(d.node("b").unwrap().props, vec![("rid".to_string(), Value::Str("R2".into()))]);
    p.run("grid", Snapshot::of(&d, &["b".to_string(), "a".to_string()]));
    let msgs = until(&p, |m| matches!(m, FromScript::Edits { .. } | FromScript::Failed(_)));
    let Some(FromScript::Edits { edits, .. }) = msgs.last() else { panic!("{msgs:?}") };
    let (op, _) = edits_to_ops(&d, edits);
    d.apply(&op.unwrap()).unwrap();
    assert_eq!(d.layout["a"].pos, Point::new(260.0, 0.0));
}
