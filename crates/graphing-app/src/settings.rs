//! User settings (`settings.json`) and app state (`state.json`).
//!
//! Settings are the user's: theme, panels, canvas options, keybindings and
//! anything added later. The app only rewrites keys it changes (theme and
//! panel toggles) and keeps every other key, known or not, as written.
//! State is the app's: open files, active tab, recent files.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("graphing")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn state_path() -> PathBuf {
    config_dir().join("state.json")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

/// When moving pictures (GIF, WebP, AVIF) play, as in notesy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Play {
    /// Whenever the window is in front.
    #[default]
    Always,
    /// Only the picture under the pointer.
    Hover,
    Never,
}

/// When deleting asks first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfirmDelete {
    /// Every delete.
    #[default]
    Always,
    /// Groups (with what is inside) and more than one item at once.
    Groups,
    Never,
}

/// One user keybinding. `action: null` (or omitted) unbinds `keys` in
/// `context`, silencing the default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserBinding {
    pub keys: String,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Arguments for actions that take them, e.g. `{ "stencil": "db" }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The file as read, so unknown keys survive a write.
    raw: Map<String, Value>,
    pub theme: ThemeChoice,
    pub shapes_panel: bool,
    pub inspector: bool,
    pub source_panel: bool,
    pub grid: f64,
    pub snap: bool,
    pub show_grid: bool,
    pub confirm_delete: ConfirmDelete,
    pub play: Play,
    pub keybindings: Vec<UserBinding>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_raw(Map::new()).0
    }
}

impl Settings {
    /// Read settings, falling back to defaults per key. Problems are
    /// returned as messages, never fatal.
    pub fn load() -> (Self, Vec<String>) {
        match std::fs::read_to_string(settings_path()) {
            Ok(text) => Self::parse(&text),
            Err(_) => (Self::default(), Vec::new()),
        }
    }

    pub fn parse(text: &str) -> (Self, Vec<String>) {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => Self::from_raw(map),
            Ok(_) => (Self::default(), vec!["settings.json must be an object".into()]),
            Err(e) => (Self::default(), vec![format!("settings.json: {e}")]),
        }
    }

    fn from_raw(raw: Map<String, Value>) -> (Self, Vec<String>) {
        let mut errors = Vec::new();
        let get_bool = |path: &[&str], default: bool| lookup(&raw, path).and_then(Value::as_bool).unwrap_or(default);
        let theme = match lookup(&raw, &["theme"]) {
            None => ThemeChoice::System,
            Some(v) => serde_json::from_value(v.clone()).unwrap_or_else(|_| {
                errors.push(format!("theme: expected \"system\", \"light\" or \"dark\", got {v}"));
                ThemeChoice::System
            }),
        };
        let keybindings = match lookup(&raw, &["keybindings"]) {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .filter_map(|(i, v)| match serde_json::from_value::<UserBinding>(v.clone()) {
                    Ok(b) => Some(b),
                    Err(e) => {
                        errors.push(format!("keybindings[{i}]: {e}"));
                        None
                    }
                })
                .collect(),
            Some(_) => {
                errors.push("keybindings must be an array".into());
                Vec::new()
            }
        };
        let s = Self {
            theme,
            shapes_panel: get_bool(&["panels", "shapes"], true),
            inspector: get_bool(&["panels", "inspector"], true),
            source_panel: get_bool(&["panels", "source"], false),
            grid: lookup(&raw, &["canvas", "grid"]).and_then(Value::as_f64).filter(|g| *g > 0.0).unwrap_or(10.0),
            snap: get_bool(&["canvas", "snap"], true),
            show_grid: get_bool(&["canvas", "show_grid"], true),
            play: lookup(&raw, &["canvas", "animations"]).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
            confirm_delete: lookup(&raw, &["editing", "confirm_delete"]).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
            keybindings,
            raw,
        };
        (s, errors)
    }

    /// The JSON written back: the original file with the app-owned keys
    /// updated.
    pub fn to_json(&self) -> String {
        let mut raw = self.raw.clone();
        raw.insert("theme".into(), serde_json::to_value(self.theme).unwrap_or(Value::Null));
        let panels = raw.entry("panels").or_insert_with(|| json!({}));
        if let Value::Object(p) = panels {
            p.insert("shapes".into(), self.shapes_panel.into());
            p.insert("inspector".into(), self.inspector.into());
            p.insert("source".into(), self.source_panel.into());
        }
        let canvas = raw.entry("canvas").or_insert_with(|| json!({}));
        if let Value::Object(c) = canvas {
            c.insert("grid".into(), json!(self.grid));
            c.insert("snap".into(), self.snap.into());
            c.insert("show_grid".into(), self.show_grid.into());
            c.insert("animations".into(), serde_json::to_value(self.play).unwrap_or(Value::Null));
        }
        let editing = raw.entry("editing").or_insert_with(|| json!({}));
        if let Value::Object(e) = editing {
            e.insert("confirm_delete".into(), serde_json::to_value(self.confirm_delete).unwrap_or(Value::Null));
        }
        raw.insert("keybindings".into(), serde_json::to_value(&self.keybindings).unwrap_or_else(|_| json!([])));
        serde_json::to_string_pretty(&Value::Object(raw)).unwrap_or_default() + "\n"
    }

    pub fn save(&self) {
        write_file(&settings_path(), &self.to_json());
    }

    /// Create the file with every default spelled out, so it is easy to edit.
    pub fn ensure_file() -> PathBuf {
        let path = settings_path();
        if !path.exists() {
            Self::default().save();
        }
        path
    }
}

fn lookup<'a>(raw: &'a Map<String, Value>, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    rest.iter().try_fold(raw.get(*first)?, |v, k| v.get(*k))
}

fn write_file(path: &PathBuf, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub files: Vec<PathBuf>,
    pub active: usize,
    pub recent: Vec<PathBuf>,
    /// The dock layout (panes, splits, dock sizes), as the dock writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<serde_json::Value>,
}

impl State {
    pub fn load() -> Self {
        std::fs::read_to_string(state_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            write_file(&state_path(), &text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let (s, errors) = Settings::parse("{}");
        assert!(errors.is_empty());
        assert_eq!(s.theme, ThemeChoice::System);
        assert!(s.shapes_panel && s.inspector && !s.source_panel);
        assert_eq!(s.grid, 10.0);
    }

    #[test]
    fn reads_values_and_reports_bad_ones() {
        let text = r#"{
            "theme": "dark",
            "panels": { "source": true },
            "canvas": { "grid": 20, "snap": false },
            "keybindings": [
                { "keys": "ctrl-k", "action": "graphing::CommandPalette" },
                { "keys": "f", "context": "Diagram" },
                { "keys": "ctrl-1", "action": "graphing::AddShape", "args": { "stencil": "db" } },
                { "action": "graphing::Save" }
            ],
            "future_thing": { "x": 1 }
        }"#;
        let (s, errors) = Settings::parse(text);
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert!(s.source_panel && !s.snap);
        assert_eq!(s.grid, 20.0);
        assert_eq!(s.keybindings.len(), 3);
        assert_eq!(s.keybindings[1].action, None);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("keybindings[3]"));
        // Unknown keys survive a write; app-owned keys update.
        let mut s = s;
        s.theme = ThemeChoice::Light;
        let out = s.to_json();
        assert!(out.contains("future_thing") && out.contains("\"light\"") && out.contains("ctrl-k"));
        let (back, _) = Settings::parse(&out);
        assert_eq!(back.theme, ThemeChoice::Light);
        assert_eq!(back.keybindings.len(), 3);
    }

    #[test]
    fn bad_json_falls_back() {
        let (s, errors) = Settings::parse("{ nope");
        assert_eq!(s, Settings::default());
        assert_eq!(errors.len(), 1);
    }
}
