//! Running Rune plugins inside the workspace: start them, poll their
//! messages, route their commands and apply their edits.

use std::time::Duration;

use gpui_kit::{Context, SharedString, Window};
use graphing_script::{CommandInfo, FromScript, Level, Limits, Plugin, Snapshot};

use crate::workspace::Workspace;

/// How often plugin messages are picked up.
const POLL: Duration = Duration::from_millis(40);

pub struct Running {
    pub plugin: Plugin,
    pub commands: Vec<CommandInfo>,
    /// Why it stopped, once it has.
    pub stopped: Option<String>,
}

impl Workspace {
    /// (Re)start every plugin in `<config>/plugins`.
    pub(crate) fn start_plugins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plugins.clear();
        let dir = crate::settings::config_dir().join("plugins");
        let (plugins, errors) = graphing_script::start_all(&dir, Limits::default());
        self.plugins = plugins.into_iter().map(|plugin| Running { plugin, commands: Vec::new(), stopped: None }).collect();
        if !errors.is_empty() {
            self.toast(format!("plugins: {}", errors.join("; ")), window, cx);
        }
        if !self.polling {
            self.polling = true;
            cx.spawn_in(window, async move |ws, cx| {
                loop {
                    cx.background_executor().timer(POLL).await;
                    if ws.update_in(cx, |ws, window, cx| ws.poll_plugins(window, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    fn poll_plugins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut inbox: Vec<(String, FromScript)> = Vec::new();
        for r in &self.plugins {
            for m in r.plugin.drain() {
                inbox.push((r.plugin.manifest.id.clone(), m));
            }
        }
        for (id, m) in inbox {
            self.handle_plugin(&id, m, window, cx);
        }
    }

    fn handle_plugin(&mut self, id: &str, m: FromScript, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.plugins.iter().find(|r| r.plugin.manifest.id == id).map_or(id.to_string(), |r| r.plugin.manifest.name.clone());
        match m {
            FromScript::Ready { commands } => {
                if let Some(r) = self.plugins.iter_mut().find(|r| r.plugin.manifest.id == id) {
                    r.commands = commands;
                }
            }
            FromScript::Pack(json) => match graphing_scene::stencils::register(&json, &format!("plugin:{id}")) {
                Ok(_) => {
                    for t in self.views() {
                        t.update(cx, |_, cx| cx.notify());
                    }
                    cx.notify();
                }
                Err(e) => self.toast(format!("{name}: pack: {e}"), window, cx),
            },
            FromScript::Log { level, text } => {
                match level {
                    Level::Info => tracing::info!(plugin = id, "{text}"),
                    Level::Warn => tracing::warn!(plugin = id, "{text}"),
                    Level::Error => tracing::error!(plugin = id, "{text}"),
                }
                if level != Level::Info {
                    self.toast(format!("{name}: {text}"), window, cx);
                }
            }
            FromScript::Notify(text) => self.toast(format!("{name}: {text}"), window, cx),
            FromScript::Edits { edits, .. } => {
                let view = self.view().clone();
                view.update(cx, |v, cx| {
                    let (op, select) = graphing_script::edits_to_ops(v.doc().diagram(), &edits);
                    if let Some(op) = op {
                        v.apply(op, cx);
                    }
                    if let Some(ids) = select {
                        v.select(ids, cx);
                    }
                });
            }
            FromScript::Failed(why) => self.toast(format!("{name} failed: {why}"), window, cx),
            FromScript::Stopped(why) => {
                if let Some(r) = self.plugins.iter_mut().find(|r| r.plugin.manifest.id == id) {
                    r.stopped = Some(why.clone());
                }
                self.toast(format!("{name} stopped: {why}"), window, cx);
            }
        }
    }

    pub(crate) fn run_plugin_command(&mut self, plugin: &str, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.plugins.iter().find(|r| r.plugin.manifest.id == plugin) else {
            self.toast(format!("no plugin `{plugin}`"), window, cx);
            return;
        };
        let v = self.view().read(cx);
        r.plugin.run(command, Snapshot::of(v.doc().diagram(), v.selection()));
    }

    /// Every plugin command, for the palette.
    pub(crate) fn plugin_commands(&self) -> Vec<(String, String, SharedString)> {
        self.plugins
            .iter()
            .filter(|r| r.stopped.is_none())
            .flat_map(|r| {
                let (id, name) = (r.plugin.manifest.id.clone(), r.plugin.manifest.name.clone());
                r.commands.iter().map(move |c| (id.clone(), c.id.clone(), format!("{name}: {}", c.title).into()))
            })
            .collect()
    }
}
