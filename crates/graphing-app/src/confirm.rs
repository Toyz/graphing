//! Modal dialogs for decisions: confirmations of destructive actions
//! (deleting, closing unsaved work, resetting the layout) and choices (how
//! pictures are stored). One at a time, above everything: Enter takes the
//! highlighted choice or the primary button, arrows move between choices,
//! Escape cancels.

use gpui_kit::{
    AnyElement, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, deferred, div, prelude::FluentBuilder,
};
use gpui_kit::component::input::InputState;
use graphing_ui::UiExt;
use graphing_ui::kit::{self, ButtonKind, Lucide};
use graphing_ui::tokens::*;

use crate::workspace::Workspace;

pub(crate) type DialogAction = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Danger,
    Warning,
    /// A question with no wrong answer.
    Info,
}

pub(crate) struct DialogButton {
    pub label: SharedString,
    /// The highlighted button Enter presses (when there are no choices).
    pub primary: bool,
    pub action: Option<DialogAction>,
}

/// One option of a choice dialog, picked by clicking or arrows + Enter.
pub(crate) struct Choice {
    pub icon: Lucide,
    pub title: SharedString,
    pub description: SharedString,
    pub badge: Option<SharedString>,
    pub action: Option<DialogAction>,
}

pub(crate) struct Confirm {
    pub title: SharedString,
    pub message: SharedString,
    /// Counts of what is affected, as pills ("3 shapes").
    pub stats: Vec<(Lucide, SharedString)>,
    /// What is affected by name, as a short list.
    pub items: Vec<(Lucide, SharedString)>,
    pub tone: Tone,
    /// Right of Cancel, in order.
    pub buttons: Vec<DialogButton>,
    /// Options shown as cards; when present, Enter picks the highlighted one.
    pub choices: Vec<Choice>,
    pub highlight: usize,
    /// What the Cancel button says ("Keep my edits"); Cancel by default.
    pub cancel: Option<SharedString>,
    /// The badge icon, when the tone's default does not fit.
    pub icon: Option<Lucide>,
    /// A text field the buttons read (a name to save under); it has the
    /// keyboard while the dialog is up.
    pub input: Option<gpui_kit::Entity<InputState>>,
    pub(crate) focus: Option<FocusHandle>,
}

impl Confirm {
    /// One destructive action with a Cancel beside it.
    pub(crate) fn danger(title: impl Into<SharedString>, message: impl Into<SharedString>, label: impl Into<SharedString>, action: DialogAction) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            stats: Vec::new(),
            items: Vec::new(),
            tone: Tone::Danger,
            buttons: vec![DialogButton { label: label.into(), primary: true, action: Some(action) }],
            choices: Vec::new(),
            highlight: 0,
            cancel: None,
            icon: None,
            input: None,
            focus: None,
        }
    }

    /// A question answered by picking one of `choices` (or Cancel).
    pub(crate) fn choose(title: impl Into<SharedString>, message: impl Into<SharedString>, choices: Vec<Choice>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            stats: Vec::new(),
            items: Vec::new(),
            tone: Tone::Info,
            buttons: Vec::new(),
            choices,
            highlight: 0,
            cancel: None,
            icon: None,
            input: None,
            focus: None,
        }
    }

    /// Ask for a line of text in `input`; `label` runs `action`, which
    /// reads the field.
    pub(crate) fn prompt(title: impl Into<SharedString>, message: impl Into<SharedString>, icon: Lucide, input: gpui_kit::Entity<InputState>, label: impl Into<SharedString>, action: DialogAction) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            stats: Vec::new(),
            items: Vec::new(),
            tone: Tone::Info,
            buttons: vec![DialogButton { label: label.into(), primary: true, action: Some(action) }],
            choices: Vec::new(),
            highlight: 0,
            cancel: None,
            icon: Some(icon),
            input: Some(input),
            focus: None,
        }
    }

    pub(crate) fn summary(mut self, stats: Vec<(Lucide, SharedString)>, items: Vec<(Lucide, SharedString)>) -> Self {
        self.stats = stats;
        self.items = items;
        self
    }
}

/// What answered the dialog.
#[derive(Debug, Clone, Copy)]
enum Answer {
    Cancel,
    Button(usize),
    Choice(usize),
}

impl Workspace {
    /// Show `c`, taking keyboard focus until it is answered.
    pub(crate) fn ask(&mut self, mut c: Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        c.focus = Some(focus);
        // A text field takes the keyboard; Enter and Esc still reach the dialog.
        if let Some(input) = &c.input {
            input.update(cx, |s, cx| s.focus(window, cx));
        }
        self.confirm = Some(c);
        cx.notify();
    }

    fn answer(&mut self, a: Answer, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut c) = self.confirm.take() else { return };
        let action = match a {
            Answer::Cancel => None,
            Answer::Button(i) => c.buttons.get_mut(i).and_then(|b| b.action.take()),
            Answer::Choice(i) => c.choices.get_mut(i).and_then(|b| b.action.take()),
        };
        self.focus_canvas(window, cx);
        if let Some(action) = action {
            action(self, window, cx);
        }
        cx.notify();
    }

    fn dialog_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &mut self.confirm else { return };
        let n = c.choices.len();
        match ev.keystroke.key.as_str() {
            "escape" => self.answer(Answer::Cancel, window, cx),
            "up" | "left" if n > 0 => {
                c.highlight = (c.highlight + n - 1) % n;
                cx.notify();
            }
            "down" | "right" | "tab" if n > 0 => {
                c.highlight = (c.highlight + 1) % n;
                cx.notify();
            }
            "enter" => {
                let a = if n > 0 { Answer::Choice(c.highlight) } else { c.buttons.iter().position(|b| b.primary).map_or(Answer::Cancel, Answer::Button) };
                self.answer(a, window, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_confirm(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.confirm.as_ref()?;
        let k = cx.ui();
        let (icon, tone) = match c.tone {
            Tone::Danger => (Lucide::Trash, k.danger),
            Tone::Warning => (Lucide::CircleAlert, k.warning),
            Tone::Info => (Lucide::Image, k.accent),
        };
        let icon = c.icon.unwrap_or(icon);
        let mut buttons: Vec<AnyElement> =
            vec![kit::button("dialog-cancel", c.cancel.clone().unwrap_or_else(|| "Cancel".into()), ButtonKind::Secondary, None, cx).on_click(cx.listener(|ws, _, window, cx| ws.answer(Answer::Cancel, window, cx))).into_any_element()];
        for (i, b) in c.buttons.iter().enumerate() {
            let kind = match (b.primary, c.tone) {
                (true, Tone::Danger) => ButtonKind::Danger,
                (true, _) => ButtonKind::Primary,
                _ => ButtonKind::Secondary,
            };
            buttons.push(
                kit::button(SharedString::from(format!("dialog-{i}")), b.label.clone(), kind, None, cx)
                    .on_click(cx.listener(move |ws, _, window, cx| ws.answer(Answer::Button(i), window, cx)))
                    .into_any_element(),
            );
        }
        let hint = if c.choices.is_empty() { "Enter to confirm, Esc to cancel" } else { "\u{2191}\u{2193} to choose, Enter to pick, Esc to cancel" };

        let mut extra: Vec<AnyElement> = Vec::new();
        if let Some(input) = &c.input {
            extra.push(kit::text_input(input).into_any_element());
        }
        if !c.choices.is_empty() {
            extra.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(GAP_2)
                    .children(c.choices.iter().enumerate().map(|(i, ch)| {
                        kit::choice_card(SharedString::from(format!("dialog-choice-{i}")), ch.icon, ch.title.clone(), ch.description.clone(), ch.badge.clone(), i == c.highlight, cx)
                            .on_click(cx.listener(move |ws, _, window, cx| ws.answer(Answer::Choice(i), window, cx)))
                    }))
                    .into_any_element(),
            );
        }
        // What it touches: counts as pills, then a few names.
        if !c.stats.is_empty() {
            extra.push(div().flex().flex_wrap().gap(GAP_2).children(c.stats.iter().map(|(i, t)| kit::stat_pill(*i, t.clone(), cx))).into_any_element());
        }
        if !c.items.is_empty() {
            let shown = 4;
            let more = c.items.len().saturating_sub(shown);
            extra.push(
                div()
                    .flex()
                    .flex_col()
                    .rounded(ROUND_MD)
                    .bg(k.bg)
                    .border_1()
                    .border_color(k.border)
                    .py(GAP_1)
                    .children(c.items.iter().take(shown).map(|(i, t)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(GAP_2)
                            .px(GAP_3)
                            .h(ROW_H)
                            .child(gpui_kit::component::Icon::new(*i).size(ICON_SM).text_color(k.text_faint))
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(TEXT_SM).text_color(k.text).child(t.clone()))
                    }))
                    .when(more > 0, |el| el.child(div().px(GAP_3).h(ROW_H).flex().items_center().text_size(TEXT_XS).text_color(k.text_faint).child(format!("and {more} more"))))
                    .into_any_element(),
            );
        }
        let extra = (!extra.is_empty()).then(|| div().flex().flex_col().gap(GAP_3).children(extra).into_any_element());
        let card = kit::dialog_card(icon, tone, c.title.clone(), c.message.clone(), extra, hint, buttons, cx);
        let focus = c.focus.clone();
        let overlay = div()
            .id("dialog-backdrop")
            .absolute()
            .inset_0()
            .bg(k.modal_scrim)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Left, cx.listener(|ws, _, window, cx| ws.answer(Answer::Cancel, window, cx)))
            .when_some(focus, |el, f| el.track_focus(&f))
            .on_key_down(cx.listener(|ws, ev: &KeyDownEvent, window, cx| ws.dialog_key(ev, window, cx)))
            .child(div().id("dialog-card").occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(graphing_ui::menu::animate(card, "dialog-anim")));
        // Above everything, including the canvas and popovers.
        Some(deferred(overlay).with_priority(100).into_any_element())
    }
}
