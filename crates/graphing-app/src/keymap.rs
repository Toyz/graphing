//! Keybindings: built-in defaults, then the user's `keybindings` from
//! settings.json on top. Later bindings win, so user entries override
//! defaults; `"action": null` unbinds. Actions are looked up by their gpui
//! name (`graphing::Save`), so anything a plugin registers is bindable too.

use std::rc::Rc;

use gpui_kit::{App, KeyBinding, KeyBindingContextPredicate, NoAction};

use crate::settings::UserBinding;

/// `(keys, action, context)`. Context is a gpui predicate. `secondary` is
/// cmd on macOS and ctrl elsewhere.
pub const DEFAULTS: &[(&str, &str, Option<&str>)] = &[
    // Workspace
    ("secondary-n", "graphing::NewFile", Some("Workspace")),
    ("secondary-alt-n", "graphing::NewFromTemplate", Some("Workspace")),
    ("secondary-o", "graphing::OpenFile", Some("Workspace")),
    ("secondary-s", "graphing::Save", Some("Workspace")),
    ("secondary-shift-s", "graphing::SaveAs", Some("Workspace")),
    ("secondary-w", "graphing::CloseTab", Some("Workspace")),
    ("ctrl-tab", "graphing::NextTab", Some("Workspace")),
    ("ctrl-pagedown", "graphing::NextTab", Some("Workspace")),
    ("ctrl-shift-tab", "graphing::PrevTab", Some("Workspace")),
    ("ctrl-pageup", "graphing::PrevTab", Some("Workspace")),
    ("secondary-shift-p", "graphing::CommandPalette", Some("Workspace")),
    ("secondary-k", "graphing::CommandPalette", Some("Workspace")),
    ("secondary-b", "graphing::ToggleLeft", Some("Workspace")),
    ("secondary-alt-b", "graphing::ToggleRight", Some("Workspace")),
    ("secondary-e", "graphing::ToggleSource", Some("Workspace")),
    ("secondary-shift-m", "graphing::ToggleProblems", Some("Workspace")),
    ("secondary-shift-e", "graphing::ExportSvg", Some("Workspace")),
    ("secondary-shift-l", "graphing::Relayout", Some("Workspace")),
    ("secondary-,", "graphing::OpenSettings", Some("Workspace")),
    ("f5", "graphing::PlayAnimation", Some("Workspace")),
    ("secondary-q", "graphing::Quit", None),
    // Canvas
    ("secondary-z", "graphing::Undo", None),
    ("secondary-shift-z", "graphing::Redo", None),
    ("secondary-y", "graphing::Redo", None),
    ("delete", "graphing::Delete", Some("Diagram")),
    ("backspace", "graphing::Delete", Some("Diagram")),
    ("secondary-c", "graphing::Copy", Some("Diagram")),
    ("secondary-x", "graphing::Cut", Some("Diagram")),
    ("secondary-v", "graphing::Paste", Some("Diagram")),
    ("secondary-d", "graphing::Duplicate", Some("Diagram")),
    ("secondary-a", "graphing::SelectAll", Some("Diagram")),
    ("escape", "graphing::Escape", Some("Diagram")),
    ("enter", "graphing::Rename", Some("Diagram")),
    ("f2", "graphing::Rename", Some("Diagram")),
    ("f", "graphing::FitView", Some("Diagram")),
    ("space", "graphing::PlayAnimation", Some("Diagram")),
    ("secondary-shift-n", "graphing::NewStep", Some("Diagram")),
    ("secondary-g", "graphing::GroupSelection", Some("Diagram")),
    ("secondary-shift-g", "graphing::Ungroup", Some("Diagram")),
    ("secondary-=", "graphing::ZoomIn", Some("Diagram")),
    ("secondary-+", "graphing::ZoomIn", Some("Diagram")),
    ("secondary--", "graphing::ZoomOut", Some("Diagram")),
    ("secondary-0", "graphing::ZoomReset", Some("Diagram")),
    ("left", "graphing::NudgeLeft", Some("Diagram")),
    ("right", "graphing::NudgeRight", Some("Diagram")),
    ("up", "graphing::NudgeUp", Some("Diagram")),
    ("down", "graphing::NudgeDown", Some("Diagram")),
    ("shift-left", "graphing::NudgeLeftBig", Some("Diagram")),
    ("shift-right", "graphing::NudgeRightBig", Some("Diagram")),
    ("shift-up", "graphing::NudgeUpBig", Some("Diagram")),
    ("shift-down", "graphing::NudgeDownBig", Some("Diagram")),
];

/// `keys` as gpui writes a recorded keystroke on this platform: `secondary`
/// resolved and modifiers in gpui's order, so bindings compare as text.
pub fn canonical(keys: &str) -> String {
    let platform = if cfg!(target_os = "macos") {
        "cmd"
    } else if cfg!(target_os = "windows") {
        "win"
    } else {
        "super"
    };
    let chord = |c: &str| {
        // `ctrl--` is ctrl plus the minus key.
        let (mods, key) = match c.strip_suffix("--") {
            Some(m) => (m, "-"),
            None => c.rsplit_once('-').unwrap_or(("", c)),
        };
        let has = |m: &[&str]| mods.split('-').any(|p| m.contains(&p));
        let (f, ctrl, alt, plat, shift) = (
            has(&["fn"]),
            has(&["ctrl", "control"]) || (!cfg!(target_os = "macos") && has(&["secondary"])),
            has(&["alt", "option"]),
            has(&["cmd", "super", "win", "platform"]) || (cfg!(target_os = "macos") && has(&["secondary"])),
            has(&["shift"]),
        );
        let mut out = String::new();
        for (on, name) in [(f, "fn"), (ctrl, "ctrl"), (alt, "alt"), (plat, platform), (shift, "shift")] {
            if on {
                out.push_str(name);
                out.push('-');
            }
        }
        out + key
    };
    keys.split_whitespace().map(chord).collect::<Vec<_>>().join(" ")
}

/// The defaults with their keys in [`canonical`] form.
pub fn defaults() -> impl Iterator<Item = (String, &'static str, Option<&'static str>)> {
    DEFAULTS.iter().map(|(k, a, c)| (canonical(k), *a, *c))
}

fn bind(cx: &mut App, keys: &str, action: Option<&str>, args: Option<serde_json::Value>, context: Option<&str>) -> Result<(), String> {
    let action = match action {
        Some(name) => cx.build_action(name, args).map_err(|e| format!("{name}: {e}"))?,
        None => Box::new(NoAction {}),
    };
    let predicate = match context {
        Some(c) => Some(Rc::new(KeyBindingContextPredicate::parse(c).map_err(|e| format!("context `{c}`: {e}"))?)),
        None => None,
    };
    let binding = KeyBinding::load(keys, action, predicate, false, None, &gpui_kit::DummyKeyboardMapper)
        .map_err(|e| format!("keys `{keys}`: {e}"))?;
    cx.bind_keys([binding]);
    Ok(())
}

/// Bind defaults then user bindings. Returns one message per bad entry.
pub fn apply(cx: &mut App, user: &[UserBinding]) -> Vec<String> {
    let mut errors = Vec::new();
    for (keys, action, context) in DEFAULTS {
        if let Err(e) = bind(cx, keys, Some(action), None, *context) {
            errors.push(format!("default {e}"));
        }
    }
    errors.extend(apply_user(cx, user));
    errors
}

/// User bindings only (reload adds these again on top).
pub fn apply_user(cx: &mut App, user: &[UserBinding]) -> Vec<String> {
    user.iter()
        .filter_map(|b| bind(cx, &b.keys, b.action.as_deref(), b.args.clone(), b.context.as_deref()).err())
        .map(|e| format!("keybinding {e}"))
        .collect()
}

/// A binding in force once user entries are applied over the defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub keys: String,
    pub action: String,
    pub context: Option<String>,
    /// Comes from settings.json rather than the defaults.
    pub user: bool,
}

/// Defaults with user entries applied: an entry replaces any binding of the
/// same keys in the same context; `action: null` just removes it.
pub fn effective(user: &[UserBinding]) -> Vec<Effective> {
    let mut out: Vec<Effective> =
        defaults().map(|(k, a, c)| Effective { keys: k, action: a.to_string(), context: c.map(str::to_string), user: false }).collect();
    for b in user {
        let keys = canonical(&b.keys);
        out.retain(|e| !(e.keys == keys && e.context == b.context));
        if let Some(action) = &b.action {
            out.push(Effective { keys, action: action.clone(), context: b.context.clone(), user: true });
        }
    }
    out
}

/// The context an action's bindings live in: its default's, else the
/// workspace.
pub fn context_of(action: &str) -> Option<String> {
    match DEFAULTS.iter().find(|(_, a, _)| *a == action) {
        Some((_, _, c)) => c.map(str::to_string),
        None => Some("Workspace".into()),
    }
}

/// Another action already on `keys` where `action` would be bound.
pub fn conflict(user: &[UserBinding], action: &str, keys: &str) -> Option<String> {
    let ctx = context_of(action);
    let keys = canonical(keys);
    effective(user).into_iter().find(|e| e.keys == keys && e.action != action && (e.context == ctx || e.context.is_none() || ctx.is_none())).map(|e| e.action)
}

/// Bind `action` to `keys` only, recording the change in `user`.
pub fn rebind(user: &mut Vec<UserBinding>, action: &str, keys: &str) {
    user.retain(|b| b.action.as_deref() != Some(action));
    for e in effective(user).into_iter().filter(|e| e.action == action) {
        user.push(UserBinding { keys: e.keys, action: None, context: e.context, args: None });
    }
    user.push(UserBinding { keys: keys.to_string(), action: Some(action.to_string()), context: context_of(action), args: None });
}

/// Drop the user's changes to `action`, restoring its defaults.
pub fn reset(user: &mut Vec<UserBinding>, action: &str) {
    let defaults: Vec<(String, Option<&str>)> = defaults().filter(|(_, a, _)| *a == action).map(|(k, _, c)| (k, c)).collect();
    user.retain(|b| b.action.as_deref() != Some(action) && !(b.action.is_none() && defaults.iter().any(|(k, c)| *k == canonical(&b.keys) && *c == b.context.as_deref())));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(user: &[UserBinding], action: &str) -> Vec<String> {
        effective(user).into_iter().filter(|e| e.action == action).map(|e| e.keys).collect()
    }

    #[test]
    fn rebind_replaces_defaults_and_reset_restores_them() {
        let mut user = Vec::new();
        assert_eq!(keys(&user, "graphing::NextTab"), ["ctrl-tab", "ctrl-pagedown"]);
        rebind(&mut user, "graphing::NextTab", "alt-right");
        assert_eq!(keys(&user, "graphing::NextTab"), ["alt-right"]);
        // Rebinding again keeps a single entry for it.
        rebind(&mut user, "graphing::NextTab", "alt-l");
        assert_eq!(keys(&user, "graphing::NextTab"), ["alt-l"]);
        reset(&mut user, "graphing::NextTab");
        assert!(user.is_empty(), "{user:?}");
        assert_eq!(keys(&user, "graphing::NextTab"), ["ctrl-tab", "ctrl-pagedown"]);
    }

    #[test]
    fn conflicts_are_reported_within_a_context() {
        let user = Vec::new();
        assert_eq!(conflict(&user, "graphing::NewFile", "secondary-s").as_deref(), Some("graphing::Save"));
        // `f` fits the canvas, but only on the canvas.
        assert_eq!(conflict(&user, "graphing::Save", "f"), None);
        assert_eq!(conflict(&user, "graphing::Save", "secondary-s"), None);
    }

    #[test]
    fn keys_compare_in_the_form_gpui_records_them() {
        let primary = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
        assert_eq!(canonical("secondary-shift-z"), format!("{primary}-shift-z"));
        assert_eq!(canonical("secondary--"), format!("{primary}--"));
        assert_eq!(canonical("shift-alt-ctrl-k ctrl-c"), "ctrl-alt-shift-k ctrl-c");
        // A recorded shortcut finds the default it collides with.
        let user = Vec::new();
        assert_eq!(conflict(&user, "graphing::NewFile", &format!("{primary}-s")).as_deref(), Some("graphing::Save"));
    }
}
