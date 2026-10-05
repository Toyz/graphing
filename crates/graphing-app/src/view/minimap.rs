//! The minimap: the whole diagram small in the canvas corner, with the part
//! in view outlined. Shown only while something is out of view; click or
//! drag in it to look there.

use super::*;
use gpui_kit::AnyElement;
use graphing_ui::tokens::{FOCUS_RING, GAP_0, GAP_3, HAIRLINE, MINIMAP_H, MINIMAP_W, ROUND_MD};

/// How the minimap maps the world: what it shows, units to pixels, and
/// the top-left inset that centres it.
#[derive(Clone, Copy, Default)]
pub(super) struct MiniMap {
    pub all: Rect,
    pub scale: f32,
    pub inset: Point<Pixels>,
    /// Where it sits in the window, from its last paint.
    pub bounds: Bounds<Pixels>,
}

impl DiagramView {
    /// The world rect in view.
    fn visible(&self) -> Option<Rect> {
        let b = self.bounds.get();
        let c = self.cam.get();
        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
        if w <= 0.0 || h <= 0.0 || c.zoom <= 0.0 {
            return None;
        }
        Some(Rect::new((-c.offset.x / c.zoom) as f64, (-c.offset.y / c.zoom) as f64, (w / c.zoom) as f64, (h / c.zoom) as f64))
    }

    pub(super) fn minimap(&self, scene: &Scene, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content = scene.content_bounds()?;
        let seen = self.visible()?;
        let fits = content.origin.x >= seen.origin.x
            && content.origin.y >= seen.origin.y
            && content.origin.x + content.size.w <= seen.origin.x + seen.size.w
            && content.origin.y + content.size.h <= seen.origin.y + seen.size.h;
        if fits || self.player.is_some() {
            return None;
        }
        let all = scene::union(content, seen);
        let pad = 8.0;
        let (mw, mh) = (f32::from(MINIMAP_W) - pad * 2.0, f32::from(MINIMAP_H) - pad * 2.0);
        let scale = (mw / all.size.w as f32).min(mh / all.size.h as f32);
        let inset = point(px(pad + (mw - all.size.w as f32 * scale) / 2.0), px(pad + (mh - all.size.h as f32 * scale) / 2.0));
        let boxes: Vec<(Rect, bool)> = scene.groups.iter().map(|g| (g.rect, true)).chain(scene.nodes.iter().map(|n| (n.rect, false))).collect();
        let k = cx.ui();
        let cell = self.minimap.clone();
        cell.set(MiniMap { all, scale, inset, bounds: cell.get().bounds });
        let paint_cell = cell.clone();
        let surface = canvas(
            move |b, _, _| {
                let mut m = paint_cell.get();
                m.bounds = b;
                paint_cell.set(m);
            },
            move |b, (), window, _| {
                use gpui_kit::{BorderStyle, quad, size};
                let map = |r: Rect| Bounds {
                    origin: point(b.origin.x + inset.x + px((r.origin.x - all.origin.x) as f32 * scale), b.origin.y + inset.y + px((r.origin.y - all.origin.y) as f32 * scale)),
                    size: size(px((r.size.w as f32 * scale).max(1.5)), px((r.size.h as f32 * scale).max(1.5))),
                };
                for (r, group) in &boxes {
                    let (fill, stroke) = if *group { (k.group_fill, k.group_stroke) } else { (k.text_muted.opacity(0.55), k.text_muted.opacity(0.55)) };
                    window.paint_quad(quad(map(*r), HAIRLINE, fill, if *group { HAIRLINE } else { Pixels::ZERO }, stroke, BorderStyle::default()));
                }
                window.paint_quad(quad(map(seen), GAP_0, k.accent.opacity(0.1), FOCUS_RING, k.accent, BorderStyle::default()));
            },
        )
        .size_full();
        let jump = move |v: &mut Self, at: Point<Pixels>, cx: &mut Context<Self>| {
            let m = v.minimap.get();
            if m.scale <= 0.0 {
                return;
            }
            let local = (at.x - m.bounds.origin.x - m.inset.x, at.y - m.bounds.origin.y - m.inset.y);
            let world = WPoint::new(m.all.origin.x + (f32::from(local.0) / m.scale) as f64, m.all.origin.y + (f32::from(local.1) / m.scale) as f64);
            let b = v.bounds.get();
            let mut c = v.cam.get();
            c.offset = point(f32::from(b.size.width) / 2.0 - world.x as f32 * c.zoom, f32::from(b.size.height) / 2.0 - world.y as f32 * c.zoom);
            c.fit = false;
            c.auto = false;
            v.cam.set(c);
            cx.notify();
        };
        Some(
            div()
                .id("minimap")
                .debug_selector(|| "minimap".into())
                .absolute()
                .right(GAP_3)
                .bottom(GAP_3)
                .w(MINIMAP_W)
                .h(MINIMAP_H)
                .rounded(ROUND_MD)
                .bg(k.raised.opacity(0.92))
                .border_1()
                .border_color(k.border)
                .overflow_hidden()
                .cursor_pointer()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(move |v, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    v.minimap_drag = true;
                    jump(v, ev.position, cx);
                }))
                .on_mouse_move(cx.listener(move |v, ev: &MouseMoveEvent, _, cx| {
                    if v.minimap_drag && ev.pressed_button == Some(MouseButton::Left) {
                        jump(v, ev.position, cx);
                    }
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.minimap_drag = false))
                .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _, _, _| v.minimap_drag = false))
                .child(surface)
                .into_any_element(),
        )
    }
}
