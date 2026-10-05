//! Command palette. Empty query: recent commands, then every command by
//! group. Typing: one ranked list, matched letters highlighted. Arrows move,
//! enter runs, escape closes.

use std::ops::Range;

use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    Action, AnyElement, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, StyledText, Styled, Subscription, Window, actions, div, prelude::FluentBuilder,
};
use graphing_ui::UiExt;
use graphing_ui::kit::{self, Lucide};
use graphing_ui::tokens::*;

actions!(palette, [SelectNext, SelectPrev, Confirm, Dismiss]);

pub fn bind_keys(cx: &mut gpui_kit::App) {
    let ctx = Some("Palette");
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("ctrl-n", SelectNext, ctx),
        KeyBinding::new("tab", SelectNext, ctx),
        KeyBinding::new("up", SelectPrev, ctx),
        KeyBinding::new("ctrl-p", SelectPrev, ctx),
        KeyBinding::new("shift-tab", SelectPrev, ctx),
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
    ]);
}

pub struct Command {
    pub name: SharedString,
    pub action: Box<dyn Action>,
    pub keys: SharedString,
    pub icon: Icon,
    pub group: &'static str,
}

pub enum PaletteEvent {
    /// Run this command (by name, so the caller can remember it as recent).
    Run(SharedString, Box<dyn Action>),
    Dismiss,
}

/// One line of the list: a group caption or a command.
enum Line {
    Caption(&'static str),
    Command { index: usize, hits: Vec<usize> },
}

pub struct Palette {
    commands: Vec<Command>,
    recent: Vec<SharedString>,
    input: Entity<InputState>,
    lines: Vec<Line>,
    /// Indices into `lines` that are commands, in order.
    rows: Vec<usize>,
    selected: usize,
    scroll: ScrollHandle,
    focus: FocusHandle,
    _sub: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Fuzzy match: every query char in order. Lower score is better; word
/// starts and runs of consecutive letters score best. Returns matched byte
/// offsets for highlighting.
pub fn fuzzy(name: &str, query: &str) -> Option<(i64, Vec<usize>)> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let lower: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    if lower.len() != chars.len() {
        return None;
    }
    let mut hits = Vec::with_capacity(q.len());
    let mut score: i64 = 0;
    let mut qi = 0;
    let mut last: Option<usize> = None;
    for (i, c) in lower.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            let word_start = i == 0 || !chars[i - 1].1.is_alphanumeric() || (chars[i].1.is_uppercase() && chars[i - 1].1.is_lowercase());
            // A run of consecutive letters beats scattered word starts.
            score += match (last.is_some_and(|l| l + 1 == i), word_start) {
                (true, _) => -10,
                (false, true) => -8,
                (false, false) => 2 + i as i64 / 4,
            };
            hits.push(chars[i].0);
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some((score + name.len() as i64 / 8, hits))
}

/// `ctrl-shift-p` -> ["Ctrl", "Shift", "P"]; chords separated by spaces.
pub fn key_chips(keys: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chord in keys.split_whitespace() {
        let mut parts: Vec<&str> = chord.split('-').collect();
        // `ctrl--` is ctrl plus the minus key.
        if chord.ends_with("--") {
            parts.retain(|p| !p.is_empty());
            parts.push("-");
        }
        for p in parts.into_iter().filter(|p| !p.is_empty()) {
            out.push(match p {
                "ctrl" => "Ctrl".into(),
                "shift" => "Shift".into(),
                "alt" => "Alt".into(),
                "cmd" | "super" | "platform" => "Super".into(),
                "enter" => "Enter".into(),
                "escape" => "Esc".into(),
                "delete" => "Del".into(),
                "backspace" => "Backspace".into(),
                "pageup" => "PgUp".into(),
                "pagedown" => "PgDn".into(),
                "tab" => "Tab".into(),
                "space" => "Space".into(),
                "left" => "\u{2190}".into(),
                "right" => "\u{2192}".into(),
                "up" => "\u{2191}".into(),
                "down" => "\u{2193}".into(),
                other if other.chars().count() == 1 => other.to_uppercase(),
                other => {
                    let mut c = other.chars();
                    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
                }
            });
        }
    }
    out
}

impl Palette {
    pub fn new(commands: Vec<Command>, recent: Vec<SharedString>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search commands"));
        let sub = cx.subscribe_in(&input, window, |p: &mut Self, _, ev: &InputEvent, _, cx| {
            if let InputEvent::Change = ev {
                p.refilter(cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let mut p = Self {
            commands,
            recent,
            input,
            lines: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            _sub: sub,
        };
        p.refilter(cx);
        p
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let q = self.input.read(cx).value().to_string();
        self.lines.clear();
        if q.trim().is_empty() {
            let recent: Vec<usize> =
                self.recent.iter().filter_map(|r| self.commands.iter().position(|c| &c.name == r)).take(5).collect();
            if !recent.is_empty() {
                self.lines.push(Line::Caption("Recent"));
                self.lines.extend(recent.iter().map(|&index| Line::Command { index, hits: Vec::new() }));
            }
            let mut group = "";
            for (index, c) in self.commands.iter().enumerate() {
                if recent.contains(&index) {
                    continue;
                }
                if c.group != group {
                    group = c.group;
                    self.lines.push(Line::Caption(group));
                }
                self.lines.push(Line::Command { index, hits: Vec::new() });
            }
        } else {
            let mut scored: Vec<(i64, usize, Vec<usize>)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let (s, hits) = fuzzy(&c.name, &q)?;
                    // Recently used commands win ties.
                    let bonus = self.recent.iter().position(|r| r == &c.name).map_or(0, |p| 6 - p.min(5) as i64);
                    Some((s - bonus, i, hits))
                })
                .collect();
            scored.sort_by_key(|(s, i, _)| (*s, *i));
            self.lines.extend(scored.into_iter().map(|(_, index, hits)| Line::Command { index, hits }));
        }
        self.rows = self.lines.iter().enumerate().filter(|(_, l)| matches!(l, Line::Command { .. })).map(|(i, _)| i).collect();
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn select(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = row.min(self.rows.len() - 1);
        // The first row brings its caption into view too.
        let line = self.rows[self.selected];
        self.scroll.scroll_to_item(if self.selected == 0 { 0 } else { line });
        cx.notify();
    }

    fn run(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(&line) = self.rows.get(row) else { return };
        if let Line::Command { index, .. } = &self.lines[line] {
            let c = &self.commands[*index];
            cx.emit(PaletteEvent::Run(c.name.clone(), c.action.boxed_clone()));
        }
    }

    fn render_line(&self, line_ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let k = cx.ui();
        match &self.lines[line_ix] {
            Line::Caption(title) => div().px(GAP_4).pt(GAP_3).pb(GAP_1).child(kit::caption(*title, cx)).into_any_element(),
            Line::Command { index, hits } => {
                let row = self.rows.iter().position(|&l| l == line_ix).unwrap_or(0);
                let selected = row == self.selected;
                let c = &self.commands[*index];
                let ranked = !self.input.read(cx).value().is_empty();
                let highlights: Vec<(Range<usize>, HighlightStyle)> = hits
                    .iter()
                    .map(|&at| {
                        let len = c.name[at..].chars().next().map_or(1, char::len_utf8);
                        (at..at + len, HighlightStyle { color: Some(k.accent), font_weight: Some(FontWeight::BOLD), ..Default::default() })
                    })
                    .collect();
                let chips = key_chips(&c.keys);
                div()
                    .id(("cmd", line_ix))
                    .h(PALETTE_ROW_H)
                    .mx(GAP_2)
                    .pl(GAP_2)
                    .pr(GAP_3)
                    .flex()
                    .items_center()
                    .gap(GAP_3)
                    .rounded(ROUND_MD)
                    .relative()
                    .cursor_pointer()
                    .when(selected, |d| {
                        d.bg(k.accent_soft).child(div().absolute().left_0().top(GAP_2).bottom(GAP_2).w(FOCUS_RING).rounded(ROUND_PILL).bg(k.accent))
                    })
                    .on_mouse_move(cx.listener(move |p, _, _, cx| {
                        if p.selected != row {
                            p.selected = row;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |p, _, _, cx| p.run(row, cx)))
                    .child(
                        div()
                            .flex_none()
                            .size(HIT_MD)
                            .rounded(ROUND_SM)
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if selected { k.raised } else { k.hover })
                            .child(c.icon.clone().size(ICON_MD).text_color(if selected { k.accent } else { k.text_muted })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(TEXT_MD)
                            .text_color(if selected { k.heading } else { k.text })
                            .child(StyledText::new(c.name.clone()).with_highlights(highlights)),
                    )
                    .when(ranked, |d| d.child(div().flex_none().text_size(TEXT_XS).text_color(k.text_faint).child(c.group)))
                    .child(div().flex_none().flex().gap(GAP_1).children(chips.into_iter().map(|key| kit::kbd(key, cx))))
                    .into_any_element()
            }
        }
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = cx.ui();
        let lines: Vec<AnyElement> = (0..self.lines.len()).map(|i| self.render_line(i, cx)).collect();
        let count = self.rows.len();
        let hint = |keys: &'static str, what: &'static str| div().flex().items_center().gap(GAP_1).child(kit::kbd(keys, cx)).child(what);
        kit::raised(cx)
            .key_context("Palette")
            .track_focus(&self.focus)
            .on_action(cx.listener(|p, _: &SelectNext, _, cx| {
                let next = if p.selected + 1 >= p.rows.len() { 0 } else { p.selected + 1 };
                p.select(next, cx);
            }))
            .on_action(cx.listener(|p, _: &SelectPrev, _, cx| {
                let prev = if p.selected == 0 { p.rows.len().saturating_sub(1) } else { p.selected - 1 };
                p.select(prev, cx);
            }))
            .on_action(cx.listener(|p, _: &Confirm, _, cx| p.run(p.selected, cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .w(PALETTE_W)
            .flex()
            .flex_col()
            .overflow_hidden()
            .shadow_2xl()
            .child(
                div()
                    .h(PALETTE_INPUT_H)
                    .px(GAP_4)
                    .flex()
                    .items_center()
                    .gap(GAP_3)
                    .child(Icon::new(Lucide::Search).size(ICON_LG).text_color(k.text_faint))
                    .child(div().flex_1().text_size(TEXT_LG).child(Input::new(&self.input).appearance(false)))
                    .child(kit::kbd("Esc", cx)),
            )
            .child(kit::divider_h(cx))
            .child(
                div()
                    .id("palette-list")
                    .track_scroll(&self.scroll)
                    .max_h(PALETTE_LIST_H)
                    .overflow_y_scroll()
                    .py(GAP_1)
                    .children(lines)
                    .when(count == 0, |d| d.child(kit::empty_state(Lucide::SearchX, "No matching command", "Try fewer letters", cx))),
            )
            .child(
                div()
                    .h(STATUS_H + GAP_1)
                    .px(GAP_4)
                    .flex()
                    .items_center()
                    .gap(GAP_4)
                    .border_t_1()
                    .border_color(k.border)
                    .bg(k.chrome)
                    .text_size(TEXT_XS)
                    .text_color(k.text_faint)
                    .child(hint("\u{2191}\u{2193}", "navigate"))
                    .child(hint("Enter", "run"))
                    .child(hint("Esc", "close"))
                    .child(div().flex_1())
                    .child(format!("{count} commands")),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{fuzzy, key_chips};

    #[test]
    fn fuzzy_prefers_word_starts() {
        let (a, hits) = fuzzy("Toggle Source Panel", "tsp").unwrap();
        assert_eq!(hits, [0, 7, 14]);
        let (b, _) = fuzzy("Distribute Horizontally", "tsp").unwrap_or((i64::MAX, vec![]));
        assert!(a < b);
        assert!(fuzzy("Save", "xyz").is_none());
        assert!(fuzzy("Save", "sa").unwrap().0 < fuzzy("Select All", "sa").unwrap().0);
    }

    #[test]
    fn keys_split_into_chips() {
        assert_eq!(key_chips("ctrl-shift-p"), ["Ctrl", "Shift", "P"]);
        assert_eq!(key_chips("ctrl--"), ["Ctrl", "-"]);
        assert_eq!(key_chips("f"), ["F"]);
        assert_eq!(key_chips("ctrl-pagedown"), ["Ctrl", "PgDn"]);
    }
}
