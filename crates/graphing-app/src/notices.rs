//! Notices in the corner: short messages that fade on their own, work in
//! progress with a bar (exports), and results with something to do next
//! (open the file, show it in its folder). Errors stay until dismissed.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, Window, div};
use graphing_ui::kit::{self, Lucide, NoticeTone, TextButton};
use graphing_ui::tokens::*;

use crate::workspace::Workspace;

/// What a notice button does.
pub(crate) type NoticeAct = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

pub(crate) struct Notice {
    pub id: usize,
    pub tone: NoticeTone,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    /// 0..1 while work goes on.
    pub progress: Option<f32>,
    pub actions: Vec<(SharedString, Lucide, NoticeAct)>,
}

/// How long a plain message stays.
const SHORT: Duration = Duration::from_millis(3200);
/// How long a result with buttons stays, so there is time to use them.
const LONG: Duration = Duration::from_secs(10);
/// Most notices shown at once; the oldest finished ones go first.
const MOST: usize = 4;

impl Workspace {
    /// Show a notice; returns its id for later updates. Plain messages and
    /// results fade on their own; work in progress and errors stay.
    pub(crate) fn notice(&mut self, tone: NoticeTone, title: impl Into<SharedString>, detail: Option<SharedString>, window: &mut Window, cx: &mut Context<Self>) -> usize {
        self.notice_seq += 1;
        let id = self.notice_seq;
        self.notices.push(Notice { id, tone, title: title.into(), detail, progress: None, actions: Vec::new() });
        while self.notices.len() > MOST {
            // Drop the oldest that is not still working.
            match self.notices.iter().position(|n| n.tone != NoticeTone::Working) {
                Some(i) => {
                    self.notices.remove(i);
                }
                None => break,
            }
        }
        match tone {
            NoticeTone::Info => self.expire(id, SHORT, window, cx),
            NoticeTone::Success => self.expire(id, LONG, window, cx),
            NoticeTone::Working | NoticeTone::Error => {}
        }
        cx.notify();
        id
    }

    /// Change a notice in place (progress, or work turning into a result).
    pub(crate) fn update_notice(&mut self, id: usize, f: impl FnOnce(&mut Notice), window: &mut Window, cx: &mut Context<Self>) {
        let Some(n) = self.notices.iter_mut().find(|n| n.id == id) else { return };
        let before = n.tone;
        f(n);
        // Work that finished fades like any result.
        if before == NoticeTone::Working && n.tone == NoticeTone::Success {
            self.expire(id, LONG, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn dismiss_notice(&mut self, id: usize, cx: &mut Context<Self>) {
        self.notices.retain(|n| n.id != id);
        cx.notify();
    }

    fn expire(&mut self, id: usize, after: Duration, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |ws, cx| {
            cx.background_executor().timer(after).await;
            ws.update(cx, |ws, cx| ws.dismiss_notice(id, cx)).ok();
        })
        .detach();
    }

    /// The corner stack of notices.
    pub(crate) fn render_notices(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.notices.is_empty() {
            return None;
        }
        let cards: Vec<AnyElement> = self
            .notices
            .iter()
            .map(|n| {
                let id = n.id;
                let actions: Vec<AnyElement> = n
                    .actions
                    .iter()
                    .enumerate()
                    .map(|(i, (label, icon, act))| {
                        let act = act.clone();
                        TextButton::new(SharedString::from(format!("notice-{id}-{i}")), label.clone())
                            .icon(*icon)
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                act(ws, window, cx);
                                ws.dismiss_notice(id, cx);
                            }))
                            .into_any_element()
                    })
                    .collect();
                let title = match (n.tone, n.progress) {
                    (NoticeTone::Working, Some(p)) => format!("{} \u{b7} {}%", n.title, (p * 100.0).round() as u32).into(),
                    _ => n.title.clone(),
                };
                kit::notice(SharedString::from(format!("notice-{id}")), n.tone, title, n.detail.clone(), n.progress, actions, cx.listener(move |ws, _, _, cx| ws.dismiss_notice(id, cx)), cx)
                    .debug_selector(move || format!("notice-{id}"))
                    .into_any_element()
            })
            .collect();
        Some(div().absolute().right(GAP_4).bottom(DOCK_STRIP_H + GAP_4).flex().flex_col().items_end().gap(GAP_2).children(cards).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn work_shows_progress_then_a_result_to_act_on(cx: &mut TestAppContext) {
        let (ws, cx) = crate::test_support::workspace(cx, Vec::new());
        let id = ws.update_in(cx, |ws, window, cx| {
            let id = ws.notice(NoticeTone::Working, "Exporting a.gif", None, window, cx);
            ws.update_notice(id, |n| n.progress = Some(0.42), window, cx);
            id
        });
        cx.run_until_parked();
        let selector: &'static str = format!("notice-{id}").leak();
        assert!(cx.debug_bounds(selector).is_some(), "shown");
        // Work stays up however long it takes.
        cx.executor().advance_clock(Duration::from_secs(30));
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.notices.iter().any(|n| n.id == id)));
        // Done: a result with buttons, which then fades.
        ws.update_in(cx, |ws, window, cx| {
            ws.update_notice(
                id,
                |n| {
                    n.tone = NoticeTone::Success;
                    n.progress = None;
                    n.actions = vec![("Open".into(), Lucide::ExternalLink, Rc::new(|_: &mut Workspace, _: &mut Window, _: &mut Context<Workspace>| {}))];
                },
                window,
                cx,
            )
        });
        assert!(ws.read_with(cx, |ws, _| ws.notices.iter().any(|n| n.id == id && n.actions.len() == 1)));
        cx.executor().advance_clock(LONG + Duration::from_secs(1));
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.notices.is_empty()));
        // Errors stay until dismissed.
        let err = ws.update_in(cx, |ws, window, cx| ws.notice(NoticeTone::Error, "Could not export", Some("disk full".into()), window, cx));
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.notices.iter().any(|n| n.id == err)));
        ws.update(cx, |ws, cx| ws.dismiss_notice(err, cx));
        assert!(ws.read_with(cx, |ws, _| ws.notices.is_empty()));
    }
}
