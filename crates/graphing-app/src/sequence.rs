//! Guided explainers: the diagram's `animate` steps played on the canvas,
//! and the sequence strip under it that builds and edits them. Steps live in
//! the `.gph` text like everything else (`Op::AddStep` and friends).

use std::time::Instant;

use gpui_kit::{
    AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};
use graphing_model::{Action, Ease, Op, Step, Verb};
use graphing_scene::anim::{AnimState, DEFAULT_STEP, Timeline};
use graphing_ui::UiExt;
use graphing_ui::kit::{self, IconButton, Lucide};
use graphing_ui::menu::{self, MenuRow};
use graphing_ui::tokens::*;

use crate::view::DiagramView;
use crate::workspace::Workspace;

/// Where playback is: `at` seconds when `since` was taken; running while
/// `since` is set, paused (previewing a moment) otherwise.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Player {
    pub at: f64,
    pub since: Option<Instant>,
}

impl Player {
    pub fn now(&self) -> f64 {
        self.at + self.since.map_or(0.0, |s| s.elapsed().as_secs_f64())
    }
}

/// What a step menu item does.
type StepAct = Box<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// Step lengths the strip offers.
const DURATIONS: [f64; 5] = [1.0, 2.0, 3.0, 5.0, 8.0];

impl DiagramView {
    pub fn steps(&self) -> &[Step] {
        &self.doc().diagram().steps
    }

    /// The steps in time, over the diagram as laid out (not as previewed).
    pub fn timeline(&self) -> Timeline {
        let d = self.doc().diagram();
        Timeline::new(d, &graphing_scene::build(d, &std::collections::HashMap::new()))
    }

    /// While a step is previewed, where its moves have put shapes.
    pub(crate) fn preview_moved(&self) -> std::collections::HashMap<String, graphing_model::Point> {
        match self.player {
            Some(p) if p.since.is_none() && self.doc().diagram().steps.iter().any(|s| !s.moves.is_empty()) => self.timeline().state(p.now()).moved,
            _ => std::collections::HashMap::new(),
        }
    }

    pub fn playing(&self) -> bool {
        self.player.is_some_and(|p| p.since.is_some())
    }

    /// The animation's moment while playing or previewing.
    pub(crate) fn anim_state(&self) -> Option<(AnimState, Timeline)> {
        let p = self.player?;
        let tl = self.timeline();
        Some((tl.state(p.now()), tl))
    }

    /// The step playing or previewed.
    pub fn current_step(&self) -> Option<usize> {
        let p = self.player?;
        self.timeline().step_at(p.now())
    }

    /// Play, pause, or start over once finished.
    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        let tl = self.timeline();
        if tl.is_empty() {
            return;
        }
        match self.player {
            Some(p) if p.since.is_some() => self.player = Some(Player { at: p.now(), since: None }),
            Some(p) if p.at < tl.total - 0.05 => self.player = Some(Player { at: p.at, since: Some(Instant::now()) }),
            _ => self.player = Some(Player { at: 0.0, since: Some(Instant::now()) }),
        }
        cx.notify();
    }

    pub fn play_from(&mut self, i: usize, cx: &mut Context<Self>) {
        let at = self.timeline().start_of(i);
        self.player = Some(Player { at, since: Some(Instant::now()) });
        cx.notify();
    }

    /// Pause on step `i` with its fades and moves done and its highlights
    /// lit (moves take 70% of a step, at most 1.4 s).
    pub fn preview_step(&mut self, i: usize, cx: &mut Context<Self>) {
        let tl = self.timeline();
        let Some(&(start, len, _)) = tl.spans.get(i) else { return };
        self.player = Some(Player { at: start + (len * 0.7).min(1.4).max(len * 0.6), since: None });
        cx.notify();
    }

    /// Back to editing: everything visible, camera where it was.
    pub fn stop_animation(&mut self, cx: &mut Context<Self>) {
        if self.player.take().is_some() {
            cx.notify();
        }
    }

    /// The step before or after the current one.
    pub fn step_by(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.steps().len();
        if n == 0 {
            return;
        }
        let at = self.current_step().map_or(0, |i| (i as isize + delta).clamp(0, n as isize - 1) as usize);
        self.preview_step(at, cx);
    }

    /// Keep frames coming while playing; stop at the end.
    pub(crate) fn tick(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.player else { return };
        if p.since.is_none() || self.tick_pending {
            return;
        }
        let total = self.timeline().total;
        if p.now() >= total {
            self.player = Some(Player { at: total, since: None });
            return;
        }
        self.tick_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
            this.update(cx, |v, cx| {
                v.tick_pending = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Selected boxes (nodes and groups) and edges.
    fn picked(&self) -> (Vec<String>, Vec<String>) {
        let d = self.doc().diagram();
        let boxes = self.selection().iter().filter(|id| d.node(id).is_some() || d.group(id).is_some()).cloned().collect();
        let edges = self.selection().iter().filter(|id| d.edge(id).is_some()).cloned().collect();
        (boxes, edges)
    }

    /// A step for the selection: show and highlight its boxes, and run
    /// dots along the edges that reach them from what is already there.
    fn step_from_selection(&self) -> Step {
        let d = self.doc().diagram();
        let (boxes, mut edges) = self.picked();
        for e in &d.edges {
            if boxes.contains(&e.to) && !boxes.contains(&e.from) && !edges.contains(&e.id) {
                edges.push(e.id.clone());
            }
        }
        let names: Vec<String> = boxes
            .iter()
            .take(2)
            .map(|id| d.node(id).map(|n| n.text().to_string()).or_else(|| d.group(id).map(|g| g.label.clone().unwrap_or_else(|| g.id.clone()))).unwrap_or_default())
            .collect();
        let title = if names.is_empty() { format!("Step {}", d.steps.len() + 1) } else { names.join(" and ").replace('\n', " ") };
        let mut actions = Vec::new();
        if !boxes.is_empty() {
            actions.push(Action { verb: Verb::Show, targets: boxes.clone() });
        }
        if !edges.is_empty() {
            actions.push(Action { verb: Verb::Flow, targets: edges });
        }
        if !boxes.is_empty() {
            actions.push(Action { verb: Verb::Highlight, targets: boxes });
        }
        Step { title: Some(title), seconds: None, ease: None, actions, moves: Vec::new() }
    }

    /// A new step from the selection, after the current one; returns its index.
    pub fn add_step(&mut self, cx: &mut Context<Self>) -> usize {
        let index = self.current_step().map_or(self.steps().len(), |i| i + 1);
        let step = self.step_from_selection();
        self.apply(Op::AddStep { index, step }, cx);
        self.preview_step(index, cx);
        index
    }

    pub fn edit_step(&mut self, i: usize, f: impl FnOnce(&mut Step), cx: &mut Context<Self>) {
        let Some(mut step) = self.steps().get(i).cloned() else { return };
        f(&mut step);
        if self.steps().get(i) != Some(&step) {
            self.apply(Op::SetStep { index: i, step }, cx);
        }
    }

    /// Add the selection to step `i` under `verb` (edges for `flow`).
    pub fn add_to_step(&mut self, i: usize, verb: Verb, cx: &mut Context<Self>) {
        let (boxes, edges) = self.picked();
        let ids = if verb == Verb::Flow { edges } else { boxes };
        if ids.is_empty() {
            return;
        }
        self.edit_step(
            i,
            |s| match s.actions.iter_mut().find(|a| a.verb == verb) {
                Some(a) => a.targets.extend(ids.into_iter().filter(|id| !a.targets.contains(id)).collect::<Vec<_>>()),
                None => s.actions.push(Action { verb, targets: ids }),
            },
            cx,
        );
    }

    pub fn move_step(&mut self, i: usize, delta: isize, cx: &mut Context<Self>) {
        let n = self.steps().len();
        let to = i as isize + delta;
        if to < 0 || to >= n as isize {
            return;
        }
        let step = self.steps()[i].clone();
        self.apply(Op::Batch(vec![Op::RemoveStep { index: i }, Op::AddStep { index: to as usize, step }]), cx);
        self.preview_step(to as usize, cx);
    }

    pub fn delete_step(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.steps().len() {
            self.apply(Op::RemoveStep { index: i }, cx);
            if self.steps().is_empty() {
                self.stop_animation(cx);
            }
        }
    }
}

/// A step's title being edited in its chip.
pub(crate) struct StepEdit {
    pub index: usize,
    pub input: Entity<gpui_kit::component::input::InputState>,
    pub _sub: gpui_kit::Subscription,
}

impl Workspace {
    pub(crate) fn edit_step_title(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::input::{InputEvent, InputState};
        let title = self.view().read(cx).steps().get(i).and_then(|s| s.title.clone()).unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |s, cx| {
            s.set_value(title, window, cx);
            s.select_all(window, cx);
            s.focus(window, cx);
        });
        let sub = cx.subscribe_in(&input, window, move |ws: &mut Self, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                ws.commit_step_title(window, cx);
            }
        });
        self.step_edit = Some(StepEdit { index: i, input, _sub: sub });
        cx.notify();
    }

    fn commit_step_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.step_edit.take() else { return };
        let text = edit.input.read(cx).value().trim().to_string();
        let title = (!text.is_empty()).then_some(text);
        self.with_view(cx, |v, cx| v.edit_step(edit.index, |s| s.title = title, cx));
        self.focus_canvas(window, cx);
        cx.notify();
    }

    /// The sequence strip under the canvas: numbered step chips, a new-step
    /// button, playback and export. Shown once a diagram has steps, or when
    /// opened from View > Sequence.
    pub(crate) fn render_sequence(&mut self, view: &Entity<DiagramView>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let k = cx.ui();
        let v = view.read(cx);
        let steps: Vec<Step> = v.steps().to_vec();
        if steps.is_empty() && !self.sequence_open {
            return None;
        }
        let playing = v.playing();
        let current = v.current_step();
        let tl = v.timeline();
        let progress = v.player.map(|p| p.now());
        let any_selected = !v.selection().is_empty();

        let chips: Vec<AnyElement> = steps
            .iter()
            .enumerate()
            .map(|(i, st)| {
                let active = current == Some(i);
                let title: SharedString = st.title.clone().unwrap_or_else(|| format!("Step {}", i + 1)).into();
                let secs = st.seconds.unwrap_or(DEFAULT_STEP);
                // How far the playing step has got, as a bar under its chip.
                let done = match (playing && active, progress, tl.spans.get(i)) {
                    (true, Some(t), Some(&(start, len, _))) => ((t - start) / len).clamp(0.0, 1.0) as f32,
                    _ => 0.0,
                };
                let editing = self.step_edit.as_ref().filter(|e| e.index == i).map(|e| e.input.clone());
                let body: AnyElement = match editing {
                    Some(input) => div().w(SIDEBAR_W * 0.6).child(kit::text_input(&input)).into_any_element(),
                    None => div()
                        .flex()
                        .items_center()
                        .gap(GAP_2)
                        .child(div().max_w(SIDEBAR_W * 0.7).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(TEXT_SM).text_color(if active { k.heading } else { k.text }).child(title.clone()))
                        .child(div().text_size(TEXT_XS).text_color(k.text_faint).child(format!("{}s", graphing_dsl::print::fmt_num(secs))))
                        .into_any_element(),
                };
                let view = view.clone();
                div()
                    .id(("seq-step", i))
                    .relative()
                    .flex_none()
                    .h(HIT_MD)
                    .pl(GAP_1)
                    .pr(GAP_2)
                    .flex()
                    .items_center()
                    .gap(GAP_2)
                    .rounded(ROUND_MD)
                    .border_1()
                    .border_color(if active { k.accent } else { k.border })
                    .bg(if active { k.accent_soft } else { k.bg })
                    .cursor_pointer()
                    .hover(|d| d.border_color(k.accent))
                    .overflow_hidden()
                    .tooltip(kit::tip(title.clone(), Some(step_summary(st).into())))
                    .on_click(cx.listener({
                        let view = view.clone();
                        move |ws, ev: &gpui_kit::ClickEvent, window, cx| {
                            if ev.click_count() >= 2 {
                                ws.edit_step_title(i, window, cx);
                            } else {
                                view.update(cx, |v, cx| v.preview_step(i, cx));
                            }
                        }
                    }))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |ws, ev: &gpui_kit::MouseDownEvent, _, cx| {
                        ws.step_menu = Some((i, ev.position));
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_none()
                            .size(HIT_SM - GAP_1)
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(TEXT_XS)
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .bg(if active { k.accent } else { k.hover })
                            .text_color(if active { k.bg } else { k.text_muted })
                            .child((i + 1).to_string()),
                    )
                    .child(body)
                    .when(done > 0.0, |d| d.child(div().absolute().left_0().bottom_0().h(FOCUS_RING).w(gpui_kit::relative(done)).bg(k.accent)))
                    .into_any_element()
            })
            .collect();

        let status: AnyElement = if playing {
            div()
                .flex()
                .items_center()
                .gap(GAP_1)
                .child(div().size(DOT).rounded_full().bg(k.accent))
                .child(div().text_size(TEXT_XS).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(k.accent).child("PLAYING"))
                .into_any_element()
        } else if let Some(i) = current {
            div().text_size(TEXT_XS).text_color(k.text_faint).child(format!("Step {} of {}", i + 1, steps.len())).into_any_element()
        } else {
            div().text_size(TEXT_XS).text_color(k.text_faint).child(if steps.is_empty() { "Select shapes, then add a step".to_string() } else { format!("{} steps, {}s", steps.len(), graphing_dsl::print::fmt_num(tl.total)) }).into_any_element()
        };

        let add_tip = if any_selected { "New step: show and highlight the selection" } else { "New empty step (select shapes first to fill it)" };
        let strip = div()
            .id("sequence")
            .flex_none()
            .h(SEQUENCE_H)
            .px(GAP_3)
            .flex()
            .items_center()
            .gap(GAP_2)
            .bg(k.chrome)
            .border_t_1()
            .border_color(k.border)
            .child(div().flex_none().child(kit::caption("Sequence", cx)))
            .child(
                div()
                    .id("sequence-steps")
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .flex()
                    .items_center()
                    .gap(GAP_2)
                    .children(chips)
                    .child(
                        IconButton::new("seq-add", Lucide::Plus).tooltip(add_tip).on_click(cx.listener({
                            let view = view.clone();
                            move |_, _, _, cx| {
                                view.update(cx, |v, cx| {
                                    v.add_step(cx);
                                });
                            }
                        })),
                    ),
            )
            .child(div().flex_none().pl(GAP_2).child(status))
            .child(kit::divider_v(cx))
            .child(
                IconButton::new("seq-prev", Lucide::StepBack).tooltip("Previous step").on_click(cx.listener({
                    let view = view.clone();
                    move |_, _, _, cx| view.update(cx, |v, cx| v.step_by(-1, cx))
                })),
            )
            .child(
                IconButton::new("seq-play", if playing { Lucide::Pause } else { Lucide::Play })
                    .active(playing)
                    .tooltip(if playing { "Pause" } else { "Play" })
                    .action(Box::new(crate::PlayAnimation)),
            )
            .child(
                IconButton::new("seq-next", Lucide::StepForward).tooltip("Next step").on_click(cx.listener({
                    let view = view.clone();
                    move |_, _, _, cx| view.update(cx, |v, cx| v.step_by(1, cx))
                })),
            )
            .when(current.is_some() || playing, |d| {
                d.child(IconButton::new("seq-stop", Lucide::Square).tooltip("Stop: back to editing").on_click(cx.listener({
                    let view = view.clone();
                    move |_, _, _, cx| view.update(cx, |v, cx| v.stop_animation(cx))
                })))
            })
            .child(IconButton::new("seq-export", Lucide::Download).tooltip("Export animation: GIF, WebM, PNG or SVG").on_click(cx.listener(|ws, ev: &gpui_kit::ClickEvent, _, cx| {
                ws.export_menu = Some(ev.position());
                cx.notify();
            })))
            .when(steps.is_empty(), |d| {
                d.child(IconButton::new("seq-close", Lucide::X).tooltip("Hide sequence").on_click(cx.listener(|ws, _, _, cx| {
                    ws.sequence_open = false;
                    cx.notify();
                })))
            });
        let menu = self.render_step_menu(view, cx);
        let export = self.render_export_menu(cx);
        Some(div().relative().child(strip).children(menu).children(export).into_any_element())
    }

    /// The export button's menu: one row per animation format.
    fn render_export_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let at = self.export_menu?;
        let mut rows = vec![MenuRow::Caption("Export the animation as".into())];
        for (id, label, icon, ext) in crate::ANIMATION_FORMATS {
            let hint = match id {
                "gif" => "Plays anywhere",
                "webm" => "Small, for video players",
                "apng" => "Full color, for browsers",
                _ => "Sharp at any size, for the web",
            };
            rows.push(
                MenuRow::item(label, cx.listener(move |ws, _, window, cx| {
                    ws.export_menu = None;
                    window.dispatch_action(Box::new(crate::ExportAnimationAs { format: id.to_string() }), cx);
                    cx.notify();
                }))
                .icon(icon)
                .detail(format!(".{ext}"))
                .description(hint),
            );
        }
        let surface = menu::menu_surface("export-menu", rows, cx).on_mouse_down_out(cx.listener(|ws, _, _, cx| {
            ws.export_menu = None;
            cx.notify();
        }));
        // Opens upward from the button, over the canvas.
        let placed = gpui_kit::anchored().position(at).anchor(gpui_kit::Anchor::BottomLeft).snap_to_window().child(menu::animate(surface, "export-menu-anim"));
        Some(gpui_kit::deferred(placed).with_priority(2).into_any_element())
    }

    /// Right-click menu of a step chip.
    fn render_step_menu(&mut self, view: &Entity<DiagramView>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (i, at) = self.step_menu?;
        let v = view.read(cx);
        let step = v.steps().get(i)?.clone();
        let n = v.steps().len();
        let has_sel = !v.selection().is_empty();
        let item = |label: String, icon: Lucide, f: StepAct, cx: &mut Context<Self>| {
            MenuRow::item(label, cx.listener(move |ws, _, window, cx| {
                ws.step_menu = None;
                f(ws, window, cx);
                cx.notify();
            }))
            .icon(icon)
        };
        let with = |f: fn(&mut DiagramView, usize, &mut Context<DiagramView>)| -> StepAct {
            Box::new(move |ws, _, cx| ws.with_view(cx, |v, cx| f(v, i, cx)))
        };
        let mut rows = vec![
            item("Play from here".into(), Lucide::Play, with(|v, i, cx| v.play_from(i, cx)), cx),
            item("Rename".into(), Lucide::PencilLine, Box::new(move |ws, window, cx| ws.edit_step_title(i, window, cx)), cx),
            MenuRow::Separator,
            MenuRow::Caption(if has_sel { "Add the selection".into() } else { "Select shapes to add them".into() }),
        ];
        if has_sel {
            for (verb, label, icon) in [
                (Verb::Show, "Show it", Lucide::Eye),
                (Verb::Flow, "Flow along its lines", Lucide::Waypoints),
                (Verb::Highlight, "Highlight it", Lucide::Sparkles),
                (Verb::Focus, "Focus the camera on it", Lucide::Scan),
            ] {
                rows.push(item(label.into(), icon, Box::new(move |ws, _, cx| ws.with_view(cx, |v, cx| v.add_to_step(i, verb, cx))), cx));
            }
        }
        rows.push(MenuRow::Separator);
        rows.push(MenuRow::Caption("Length".into()));
        let current = step.seconds.unwrap_or(DEFAULT_STEP);
        for secs in DURATIONS {
            rows.push(
                item(format!("{}s", graphing_dsl::print::fmt_num(secs)), Lucide::Timer, Box::new(move |ws, _, cx| ws.with_view(cx, |v, cx| v.edit_step(i, |s| s.seconds = Some(secs), cx))), cx)
                    .checked((current - secs).abs() < 1e-6),
            );
        }
        rows.push(MenuRow::Caption("Motion".into()));
        let ease = step.ease.unwrap_or_default();
        for e in Ease::ALL {
            let label = match e {
                Ease::Smooth => "Smooth",
                Ease::Linear => "Steady",
                Ease::Snappy => "Snappy",
                Ease::Bounce => "Bouncy",
            };
            rows.push(
                item(label.into(), Lucide::Spline, Box::new(move |ws, _, cx| ws.with_view(cx, |v, cx| v.edit_step(i, |s| s.ease = (e != Ease::Smooth).then_some(e), cx))), cx)
                    .checked(ease == e),
            );
        }
        if !step.moves.is_empty() {
            rows.push(item(format!("Clear {} move{}", step.moves.len(), if step.moves.len() == 1 { "" } else { "s" }), Lucide::Undo2, with(|v, i, cx| v.edit_step(i, |s| s.moves.clear(), cx)), cx));
        }
        rows.push(MenuRow::Separator);
        if i > 0 {
            rows.push(item("Move earlier".into(), Lucide::ArrowLeft, with(|v, i, cx| v.move_step(i, -1, cx)), cx));
        }
        if i + 1 < n {
            rows.push(item("Move later".into(), Lucide::ArrowRight, with(|v, i, cx| v.move_step(i, 1, cx)), cx));
        }
        rows.push(item("Delete step".into(), Lucide::Trash, with(|v, i, cx| v.delete_step(i, cx)), cx));
        let surface = menu::menu_surface("step-menu", rows, cx).on_mouse_down_out(cx.listener(|ws, _, _, cx| {
            ws.step_menu = None;
            cx.notify();
        }));
        // Opens upward from the pointer, over the canvas.
        let placed = gpui_kit::anchored().position(at).anchor(gpui_kit::Anchor::BottomLeft).snap_to_window().child(menu::animate(surface, "step-menu-anim"));
        Some(gpui_kit::deferred(placed).with_priority(2).into_any_element())
    }
}

/// What a step does, in words, for its chip's tooltip.
fn step_summary(st: &Step) -> String {
    if st.actions.is_empty() && st.moves.is_empty() {
        return "Does nothing yet: select shapes, then right-click to add them".into();
    }
    let mut parts: Vec<String> = st.actions.iter().map(|a| format!("{} {}", a.verb.name(), a.targets.join(", "))).collect();
    parts.extend(st.moves.iter().map(|(id, _)| format!("move {id}")));
    parts.join("; ")
}
