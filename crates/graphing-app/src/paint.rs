//! Scene -> gpui draw calls.

use gpui_kit::{
    App, BorderStyle, Bounds, Hsla, PathBuilder, Pixels, Point, SharedString, TextAlign, TextRun, Window, point,
    px, quad, rgb, size,
};

use graphing_model::Rect;
use graphing_scene::notation::EndStroke;
use graphing_scene::{EdgeLine, End, FrameBox, GroupBox, NodeBox, PORT, PortBox, Scene, Shape, Side, notation, pins};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub grid: Hsla,
    pub fill: Hsla,
    pub stroke: Hsla,
    pub text: Hsla,
    pub edge: Hsla,
    pub group_fill: Hsla,
    pub group_stroke: Hsla,
    pub accent: Hsla,
    pub muted: Hsla,
    /// Item flow stereotypes.
    pub flow: Hsla,
    /// Wires that break the diagram's rules.
    pub danger: Hsla,
}

impl From<graphing_ui::Colors> for Palette {
    fn from(k: graphing_ui::Colors) -> Self {
        Self {
            bg: k.bg,
            grid: k.grid,
            fill: k.node_fill,
            stroke: k.node_stroke,
            text: k.text,
            edge: k.edge,
            group_fill: k.group_fill,
            group_stroke: k.group_stroke,
            accent: k.accent,
            muted: k.text_muted,
            flow: k.flow,
            danger: k.danger,
        }
    }
}

impl Palette {
    /// Every color at `a` of its opacity, for elements fading in or out.
    pub fn faded(self, a: f32) -> Self {
        let f = |c: Hsla| c.opacity(a);
        Self {
            bg: f(self.bg),
            grid: f(self.grid),
            fill: f(self.fill),
            stroke: f(self.stroke),
            text: f(self.text),
            edge: f(self.edge),
            group_fill: f(self.group_fill),
            group_stroke: f(self.group_stroke),
            accent: f(self.accent),
            muted: f(self.muted),
            flow: f(self.flow),
            danger: f(self.danger),
        }
    }
}

/// Diagrams that links point at, by `src`: when last read, and what
/// (`None`: missing).
pub type Linked = std::collections::HashMap<String, (Option<std::time::SystemTime>, Option<std::rc::Rc<graphing_export::RefDiagram>>)>;

/// World -> screen mapping for one frame.
#[derive(Clone, Copy)]
pub struct View {
    pub origin: Point<Pixels>,
    pub offset: Point<f32>,
    pub zoom: f32,
}

impl View {
    pub fn pt(&self, p: graphing_model::Point) -> Point<Pixels> {
        point(
            self.origin.x + px(self.offset.x + p.x as f32 * self.zoom),
            self.origin.y + px(self.offset.y + p.y as f32 * self.zoom),
        )
    }

    pub fn rect(&self, r: Rect) -> Bounds<Pixels> {
        Bounds {
            origin: self.pt(r.origin),
            size: size(px(r.size.w as f32 * self.zoom), px(r.size.h as f32 * self.zoom)),
        }
    }

    pub fn len(&self, v: f32) -> Pixels {
        px(v * self.zoom)
    }
}

/// Transient interaction visuals drawn over the scene.
#[derive(Default)]
pub struct Overlay {
    /// Node whose connection ports are shown.
    pub hover: Option<String>,
    pub marquee: Option<Rect>,
    /// Connection being dragged: source node and current mouse point.
    pub link: Option<(String, graphing_model::Point)>,
    /// Where that connection starts when it leaves from a port.
    pub link_start: Option<graphing_model::Point>,
    /// The side of the pin a wire is dragged from: the preview curves.
    pub link_side: Option<Side>,
    /// Pins a dragged wire could go to (`true`) or not.
    pub pin_targets: Vec<(graphing_model::Point, bool)>,
    /// The group dragged nodes will be in when dropped.
    pub drop_group: Option<String>,
}

pub struct Frame<'a> {
    pub scene: &'a Scene,
    pub view: View,
    pub bounds: Bounds<Pixels>,
    pub palette: Palette,
    pub selected: &'a [String],
    pub overlay: &'a Overlay,
    /// Monospace family for compartments, ports and frame titles.
    pub mono: SharedString,
    /// Item whose label is being edited in place: its label and handles hide.
    pub editing: Option<&'a str>,
    /// Grid step in diagram units, or `None` when the grid is hidden.
    pub grid: Option<f32>,
    /// Decoded pictures by `src`; `None` for ones that failed to load.
    pub images: &'a std::collections::HashMap<String, Option<std::sync::Arc<gpui_kit::RenderImage>>>,
    /// Milliseconds on the animation clock, and when moving pictures play.
    pub now_ms: u64,
    pub play: crate::settings::Play,
    /// The animation's current moment, while one plays or previews.
    pub anim: Option<&'a graphing_scene::anim::AnimState>,
    /// Opacity of the element being drawn (1 outside animations); the
    /// palette is already faded by it.
    pub fade: f32,
    /// Diagrams that links point at, by `src` (`None`: missing).
    pub linked: &'a Linked,
}

impl Frame<'_> {
    fn is_selected(&self, id: &str) -> bool {
        self.selected.iter().any(|s| s == id)
    }

    /// This frame for one element at opacity `a`.
    fn faded(&self, a: f32) -> Frame<'_> {
        Frame {
            scene: self.scene,
            view: self.view,
            bounds: self.bounds,
            palette: self.palette.faded(a),
            selected: self.selected,
            overlay: self.overlay,
            mono: self.mono.clone(),
            editing: self.editing,
            grid: self.grid,
            images: self.images,
            now_ms: self.now_ms,
            play: self.play,
            anim: self.anim,
            fade: a,
            linked: self.linked,
        }
    }

    /// A file's own color (faded with the element), else the palette's.
    fn hex(&self, c: Option<u32>, fallback: Hsla) -> Hsla {
        c.map_or(fallback, |c| self.rgb(c))
    }

    fn rgb(&self, c: u32) -> Hsla {
        Hsla::from(rgb(c)).opacity(self.fade)
    }

    fn alpha_of(&self, id: &str) -> f32 {
        self.anim.map_or(1.0, |s| s.alpha(id))
    }

    fn glow_of(&self, id: &str) -> f32 {
        self.anim.and_then(|s| s.glow.get(id).copied()).unwrap_or(0.0)
    }
}

pub fn paint(f: &Frame, window: &mut Window, cx: &mut App) {
    let p = f.palette;
    window.paint_quad(quad(f.bounds, px(0.), p.bg, px(0.), p.bg, BorderStyle::default()));
    grid(f, window);
    if let Some(fb) = &f.scene.frame {
        frame_box(f, fb, window, cx);
    }
    // While animating, each element draws at its own opacity, glowing or
    // flowing as its step says; invisible ones are skipped.
    for g in &f.scene.groups {
        let a = f.alpha_of(&g.id);
        if a > 0.005 {
            let ff = f.faded(a);
            glow(&ff, &g.id, ff.view.rect(g.rect), ff.view.len(8.0), window);
            group(&ff, g, window, cx);
        }
    }
    for e in &f.scene.edges {
        let a = f.alpha_of(&e.id);
        if a > 0.005 {
            let ff = f.faded(a);
            edge_glow(&ff, e, window);
            flow_dots(&ff, e, window);
            edge(&ff, e, window, cx);
        }
    }
    for n in &f.scene.nodes {
        let a = f.alpha_of(&n.id);
        if a > 0.005 {
            let ff = f.faded(a);
            match &n.path {
                // Along a pack outline; the node then covers its inner half.
                Some(cmds) => path_glow(&ff, &n.id, cmds, window),
                None => {
                    // The glow hugs the box's own corners.
                    let round = if matches!(n.shape, Shape::Ellipse | Shape::Initial | Shape::Final) { n.rect.size.w.min(n.rect.size.h) as f32 / 2.0 } else { corner(n.shape, ff.scene.technical) };
                    glow(&ff, &n.id, ff.view.rect(n.rect), ff.view.len(round), window);
                }
            }
            node(&ff, n, window, cx);
        }
    }
    overlay(f, window);
}

/// A soft accent ring around a highlighted element.
/// Corner radius of a box shape, in diagram units.
fn corner(shape: Shape, technical: bool) -> f32 {
    match (shape, technical) {
        (Shape::Rounded, true) => 10.0,
        (Shape::Rounded, false) => 14.0,
        (_, true) => 2.0,
        (_, false) => 6.0,
    }
}

fn glow(f: &Frame, id: &str, b: Bounds<Pixels>, radius: Pixels, window: &mut Window) {
    let g = f.glow_of(id);
    if g <= 0.005 {
        return;
    }
    for (spread, alpha, width) in [(12.0, 0.14, 10.0), (7.0, 0.28, 6.0), (3.0, 0.9, 2.5)] {
        let d = f.view.len(spread);
        let r = Bounds { origin: point(b.origin.x - d, b.origin.y - d), size: size(b.size.width + d * 2.0, b.size.height + d * 2.0) };
        window.paint_quad(quad(r, radius + d, gpui_kit::transparent_black(), px(width), f.palette.accent.opacity(alpha * g), BorderStyle::default()));
    }
}

fn path_glow(f: &Frame, id: &str, cmds: &[graphing_scene::path::PathCmd], window: &mut Window) {
    let g = f.glow_of(id);
    if g <= 0.005 {
        return;
    }
    for (width, alpha) in [(22.0, 0.14), (12.0, 0.28), (5.0, 0.9)] {
        if let Ok(path) = custom_path(cmds, f.view, PathBuilder::stroke(f.view.len(width))).build() {
            window.paint_path(path, f.palette.accent.opacity(alpha * g));
        }
    }
}

fn edge_glow(f: &Frame, e: &EdgeLine, window: &mut Window) {
    let g = f.glow_of(&e.id);
    if g <= 0.005 || e.points.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::stroke(f.view.len(9.0));
    pb.move_to(f.view.pt(e.points[0]));
    for &p in &e.points[1..] {
        pb.line_to(f.view.pt(p));
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, f.palette.accent.opacity(0.35 * g));
    }
}

/// Dots running along a flowing edge.
fn flow_dots(f: &Frame, e: &EdgeLine, window: &mut Window) {
    use graphing_scene::anim::{FLOW_CLEAR, FLOW_GAP, trim};
    let Some(off) = f.anim.and_then(|s| s.flow.get(&e.id)) else { return };
    // A typed wire carries beads of its own color; other lines the accent.
    let color = e.stroke.map_or(f.palette.accent, |c| f.rgb(c));
    let (r, halo) = (f.view.len(3.5), f.view.len(6.5));
    for p in graphing_scene::anim::flow_dots(&trim(&e.points, FLOW_CLEAR), *off, FLOW_GAP) {
        let c = f.view.pt(p);
        let soft = Bounds { origin: point(c.x - halo, c.y - halo), size: size(halo * 2.0, halo * 2.0) };
        window.paint_quad(quad(soft, halo, color.opacity(0.25 * f.fade), px(0.), color, BorderStyle::default()));
        let b = Bounds { origin: point(c.x - r, c.y - r), size: size(r * 2.0, r * 2.0) };
        window.paint_quad(quad(b, r, color, px(0.), color, BorderStyle::default()));
    }
}

fn overlay(f: &Frame, window: &mut Window) {
    let accent = f.palette.accent;
    if f.editing.is_some() {
        return;
    }
    // Resize handles when exactly one node or group is selected.
    if let [id] = f.selected
        && let Some(r) = f.scene.rect_of(id)
    {
        let corners = [
            r.origin,
            graphing_model::Point::new(r.origin.x + r.size.w, r.origin.y),
            graphing_model::Point::new(r.origin.x + r.size.w, r.origin.y + r.size.h),
            graphing_model::Point::new(r.origin.x, r.origin.y + r.size.h),
        ];
        for c in corners {
            let p = f.view.pt(c);
            let b = Bounds { origin: point(p.x - px(4.0), p.y - px(4.0)), size: size(px(8.0), px(8.0)) };
            window.paint_quad(quad(b, px(2.0), f.palette.bg, px(1.5), accent, BorderStyle::default()));
        }
    }
    let link_target = f.overlay.link.as_ref().and_then(|(from, at)| match f.scene.hit(*at) {
        Some(graphing_scene::Hit::Node(id) | graphing_scene::Hit::Group(id)) if &id != from => Some(id),
        _ => None,
    });
    if let Some(id) = &link_target
        && let Some(r) = f.scene.rect_of(id)
    {
        let b = f.view.rect(r);
        let pad = px(3.0);
        let b = Bounds { origin: point(b.origin.x - pad, b.origin.y - pad), size: size(b.size.width + pad * 2.0, b.size.height + pad * 2.0) };
        window.paint_quad(quad(b, px(6.0), accent.opacity(0.08), px(2.0), accent, BorderStyle::default()));
    }
    if let Some(id) = &f.overlay.hover
        && let Some(r) = f.scene.rect_of(id)
    {
        for p in graphing_scene::ports(r) {
            let c = f.view.pt(p);
            let b = Bounds { origin: point(c.x - px(4.5), c.y - px(4.5)), size: size(px(9.0), px(9.0)) };
            window.paint_quad(quad(b, px(4.5), f.palette.bg, px(1.5), accent, BorderStyle::default()));
        }
    }
    if let Some((from, at)) = &f.overlay.link
        && let Some((r, shape)) = f.scene.outline_of(from)
    {
        let from = f.overlay.link_start.unwrap_or_else(|| graphing_scene::boundary(shape, r, *at));
        let (start, end) = (f.view.pt(from), f.view.pt(*at));
        let mut pb = PathBuilder::stroke(px(1.5)).dash_array(&[px(5.0), px(4.0)]);
        pb.move_to(start);
        match f.overlay.link_side {
            // A wire from a pin curves like the one it will become.
            Some(side) => {
                for p in pins::wire(from, Some(side), *at, None).into_iter().skip(1) {
                    pb.line_to(f.view.pt(p));
                }
            }
            None => pb.line_to(end),
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, accent);
        }
        if f.overlay.link_side.is_none() {
            arrowhead(start, end, f.view.zoom, accent, window);
        }
    }
    for (p, ok) in &f.overlay.pin_targets {
        let c = f.view.pt(*p);
        let r = f.view.len(pins::PIN_R as f32);
        if *ok {
            let ring = r + px(3.0);
            let b = Bounds { origin: point(c.x - ring, c.y - ring), size: size(ring * 2.0, ring * 2.0) };
            window.paint_quad(quad(b, ring, gpui_kit::transparent_black(), px(2.0), accent, BorderStyle::default()));
        } else {
            // Wash out what will not take this wire.
            let b = Bounds { origin: point(c.x - r - px(1.0), c.y - r - px(1.0)), size: size(r * 2.0 + px(2.0), r * 2.0 + px(2.0)) };
            window.paint_quad(quad(b, r + px(1.0), f.palette.bg.opacity(0.7), px(0.), f.palette.bg, BorderStyle::default()));
        }
    }
    if let Some(id) = &f.overlay.drop_group
        && let Some(r) = f.scene.rect_of(id)
    {
        window.paint_quad(quad(f.view.rect(r), f.view.len(6.0), accent.opacity(0.06), px(2.0), accent, BorderStyle::default()));
    }
    if let Some(m) = f.overlay.marquee {
        let b = f.view.rect(m);
        window.paint_quad(quad(b, px(2.0), accent.opacity(0.08), px(1.0), accent, BorderStyle::default()));
    }
}

fn grid(f: &Frame, window: &mut Window) {
    let Some(g) = f.grid else { return };
    // A dot every other grid step, thinning out as the view zooms away.
    let mut step = g * 2.0 * f.view.zoom;
    while step > 0.0 && step < 12.0 {
        step *= 2.0;
    }
    if step < 8.0 {
        return;
    }
    let b = f.bounds;
    let start_x = (f.view.offset.x.rem_euclid(step)) + f32::from(b.origin.x);
    let start_y = (f.view.offset.y.rem_euclid(step)) + f32::from(b.origin.y);
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let dot = px(1.5);
    let mut y = start_y;
    while y < f32::from(b.origin.y) + h {
        let mut x = start_x;
        while x < f32::from(b.origin.x) + w {
            let r = Bounds { origin: point(px(x), px(y)), size: size(dot, dot) };
            window.paint_quad(quad(r, px(0.), f.palette.grid, px(0.), f.palette.grid, BorderStyle::default()));
            x += step;
        }
        y += step;
    }
}

/// Text style for one run: proportional or mono, regular or semibold.
#[derive(Clone, Copy)]
enum Face {
    Sans,
    SansBold,
    Mono,
}

fn frame_box(f: &Frame, fb: &FrameBox, window: &mut Window, cx: &mut App) {
    let p = f.palette;
    let b = f.view.rect(fb.rect);
    let stroke = if f.is_selected(crate::view::FRAME_ID) { p.accent } else { p.group_stroke };
    window.paint_quad(quad(b, px(0.), gpui_kit::transparent_black(), px(1.0), stroke, BorderStyle::default()));
    let z = f.view.zoom;
    let size_px = FRAME_PT * z;
    let title = shape_text(&fb.title, size_px, p.text, Face::Mono, f, window);
    let tab_w = title.width + f.view.len(24.0);
    let tab_h = f.view.len(30.0);
    let cut = f.view.len(12.0);
    let (x, y) = (b.origin.x, b.origin.y);
    let mut pb = PathBuilder::stroke(px(1.0));
    pb.move_to(point(x + tab_w, y));
    pb.line_to(point(x + tab_w + cut, y + cut));
    pb.line_to(point(x + tab_w + cut, y + tab_h));
    pb.line_to(point(x, y + tab_h));
    if let Ok(path) = pb.build() {
        window.paint_path(path, stroke);
    }
    if f.editing != Some(crate::view::FRAME_ID) {
        title.paint(point(x + f.view.len(12.0), y + (tab_h - px(size_px * 1.3)) / 2.0), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
    }
}

fn group(f: &Frame, g: &GroupBox, window: &mut Window, cx: &mut App) {
    use graphing_scene::GroupLook as L;
    let b = f.view.rect(g.rect);
    let selected = f.is_selected(&g.id);
    let base = f.hex(g.stroke, f.palette.group_stroke);
    let stroke = if selected { f.palette.accent } else { base };
    let tint = g.fill.map(|c| f.rgb(c));
    let label_ink = g.text.map(|c| f.rgb(c));
    let head = f.view.len(g.look.head() as f32);
    let size_px = GROUP_PT * f.view.zoom;
    let icon_w = if g.icon.is_some() { f.view.len(18.0) } else { px(0.) };
    let tab_w = g.label.as_ref().map_or(f.view.len(60.0), |l| shape_text(l, size_px, f.palette.text, Face::SansBold, f, window).width + f.view.len(24.0) + icon_w);
    group_chrome(b, g.look, GroupInk { stroke, base, tint, panel: f.palette.fill, bg: f.palette.group_fill, selected }, f.view.zoom, head, tab_w, f.scene.technical, window);
    let Some(label) = &g.label else { return };
    if f.editing == Some(g.id.as_str()) {
        // The in-place editor draws the label.
        return;
    }
    if g.look == L::Sysml {
        // «stereotype» over the bold name, centred, as on SysML blocks.
        let ink = label_ink.unwrap_or(f.palette.text);
        let line_h = px(size_px * 1.3);
        let mut y = b.origin.y + f.view.len(5.0);
        if let Some(st) = &g.stereotype {
            let s = shape_text(&format!("\u{ab}{st}\u{bb}"), 10.0 * f.view.zoom, f.palette.muted, Face::Sans, f, window);
            s.paint(point(b.origin.x + (b.size.width - s.width) / 2.0, y), px(10.0 * f.view.zoom * 1.3), TextAlign::Left, None, window, cx).ok();
            y += px(10.0 * f.view.zoom * 1.3);
        } else {
            y += (head - line_h) / 2.0 - f.view.len(5.0);
        }
        let name = shape_text(label, size_px, ink, Face::SansBold, f, window);
        name.paint(point(b.origin.x + (b.size.width - name.width) / 2.0, y), line_h, TextAlign::Left, None, window, cx).ok();
        return;
    }
    let (text, face, ink) = match g.look {
        L::Dashed | L::Solid => (label.clone(), Face::Sans, label_ink.unwrap_or(f.palette.text.opacity(0.7))),
        L::Zone => (label.to_uppercase(), Face::SansBold, label_ink.or(tint).unwrap_or(f.palette.text)),
        _ => (label.clone(), Face::SansBold, label_ink.unwrap_or(f.palette.text)),
    };
    let line = shape_text(&text, size_px, ink, face, f, window);
    // Vertically centred in the header strip (the package tab is shorter).
    let strip = if g.look == L::Package { f.view.len(GROUP_TAB) } else { head };
    let mut x = b.origin.x + f.view.len(12.0);
    if let Some(icon) = &g.icon {
        let s = f.view.len(13.0);
        let at = Bounds { origin: point(x, b.origin.y + (strip - s) / 2.0), size: size(s, s) };
        window.paint_svg(at, graphing_ui::kit::icon_path(icon), None, gpui_kit::TransformationMatrix::unit(), ink, cx).ok();
        x += icon_w;
    }
    let y = b.origin.y + (strip - px(size_px * 1.3)) / 2.0;
    line.paint(point(x, y), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
    // Field values after the name, quieter.
    if let Some(details) = &g.details {
        let small = 11.0 * f.view.zoom;
        let d = shape_text(details, small, f.palette.muted, Face::Mono, f, window);
        let dy = b.origin.y + (strip - px(small * 1.3)) / 2.0;
        d.paint(point(x + line.width + f.view.len(10.0), dy), px(small * 1.3), TextAlign::Left, None, window, cx).ok();
    }
}

/// Height of a package group's name tab, in diagram units.
pub const GROUP_TAB: f32 = 22.0;
/// Label sizes in diagram units, shared with the in-place label editor so
/// typing shows text exactly as it will paint.
pub const LABEL_PT: f32 = 14.0;
/// A structured node's (block, stereotyped) bold name.
pub const HEADER_PT: f32 = 12.5;
pub const GROUP_PT: f32 = 12.0;
pub const EDGE_PT: f32 = 12.0;
pub const FRAME_PT: f32 = 11.0;

/// Colors a group's chrome is drawn with.
#[derive(Clone, Copy)]
pub struct GroupInk {
    /// Outline (accent when selected).
    pub stroke: Hsla,
    /// The group's own color, for bands and tints.
    pub base: Hsla,
    /// User fill, if any.
    pub tint: Option<Hsla>,
    /// Card body.
    pub panel: Hsla,
    /// Default faint fill.
    pub bg: Hsla,
    pub selected: bool,
}

/// A group's body and border in one of its looks. `zoom` scales corner radii;
/// `head` is the header strip height and `tab_w` the package tab width, both
/// in pixels. Shared by the canvas and the library tiles.
#[allow(clippy::too_many_arguments)]
pub fn group_chrome(b: Bounds<Pixels>, look: graphing_scene::GroupLook, ink: GroupInk, zoom: f32, head: Pixels, tab_w: Pixels, technical: bool, window: &mut Window) {
    use graphing_scene::GroupLook as L;
    let z = |v: f32| px(v * zoom);
    let fill = ink.tint.map_or(ink.bg, |c| c.alpha(0.12));
    match look {
        L::Dashed | L::Solid => {
            let r = if technical { z(2.0) } else { z(10.0) };
            let border = if look == L::Dashed { BorderStyle::Dashed } else { BorderStyle::Solid };
            window.paint_quad(quad(b, r, fill, px(1.0), ink.stroke, border));
        }
        L::Package => {
            // SysML frame: a name tab with a cut corner on a square body.
            window.paint_quad(quad(b, px(0.), fill, px(1.0), ink.stroke, BorderStyle::Solid));
            let (x, y) = (b.origin.x, b.origin.y);
            let (tw, th, cut) = (tab_w.min(b.size.width - z(10.0)), z(GROUP_TAB), z(8.0));
            let mut pb = PathBuilder::stroke(px(1.0));
            pb.move_to(point(x + tw, y));
            pb.line_to(point(x + tw + cut, y + cut));
            pb.line_to(point(x + tw + cut, y + th));
            pb.line_to(point(x, y + th));
            if let Ok(path) = pb.build() {
                window.paint_path(path, ink.stroke);
            }
        }
        L::Sysml => {
            window.paint_quad(quad(b, px(0.), ink.tint.map_or(ink.panel.opacity(0.5), |c| c.alpha(0.10)), px(1.0), ink.stroke, BorderStyle::Solid));
            let rule = Bounds { origin: point(b.origin.x, b.origin.y + head), size: size(b.size.width, px(1.0)) };
            window.paint_quad(quad(rule, px(0.), ink.stroke, px(0.), gpui_kit::transparent_black(), BorderStyle::Solid));
        }
        L::Lane => {
            let r = z(4.0);
            window.paint_quad(quad(b, r, fill, px(1.0), ink.stroke, BorderStyle::Solid));
            let band = Bounds { origin: b.origin, size: size(b.size.width, head.min(b.size.height)) };
            let band_fill = ink.tint.unwrap_or(ink.base).alpha(0.22);
            window.paint_quad(gpui_kit::PaintQuad { corner_radii: gpui_kit::Corners { top_left: r, top_right: r, bottom_left: px(0.), bottom_right: px(0.) }, ..quad(band, px(0.), band_fill, px(0.), gpui_kit::transparent_black(), BorderStyle::Solid) });
            let rule = Bounds { origin: point(b.origin.x, b.origin.y + head), size: size(b.size.width, px(1.0)) };
            window.paint_quad(quad(rule, px(0.), ink.stroke, px(0.), gpui_kit::transparent_black(), BorderStyle::Solid));
        }
        L::Zone => {
            let zone = ink.tint.unwrap_or(ink.base).alpha(0.16);
            let border = if ink.selected { ink.stroke } else { gpui_kit::transparent_black() };
            window.paint_quad(quad(b, z(10.0), zone, px(if ink.selected { 1.5 } else { 0.0 }), border, BorderStyle::Solid));
        }
        L::Card => {
            let body = ink.tint.map_or(ink.panel, |c| c.alpha(0.18));
            window.paint_quad(quad(b, z(12.0), body, px(1.0), ink.stroke, BorderStyle::Solid));
            let rule = Bounds { origin: point(b.origin.x + z(10.0), b.origin.y + head), size: size((b.size.width - z(20.0)).max(px(0.)), px(1.0)) };
            window.paint_quad(quad(rule, px(0.), ink.stroke.opacity(0.6), px(0.), gpui_kit::transparent_black(), BorderStyle::Solid));
        }
    }
}

/// A node's picture, fitted inside it; a placeholder when it cannot load.
fn picture(f: &Frame, n: &NodeBox, src: &str, b: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let inset = f.view.len(2.0);
    let clip = Bounds { origin: point(b.origin.x + inset, b.origin.y + inset), size: size(b.size.width - inset * 2.0, b.size.height - inset * 2.0) };
    match f.images.get(src) {
        Some(Some(data)) => {
            use crate::settings::Play;
            let plays = match f.play {
                Play::Always => true,
                Play::Hover => f.overlay.hover.as_deref() == Some(n.id.as_str()),
                Play::Never => false,
            };
            let frame = if plays { crate::media::frame_at(data, f.now_ms) } else { 0 };
            let px_size = data.size(frame);
            let r = n.fit.place(n.rect, px_size.width.0 as f64, px_size.height.0 as f64);
            let image = f.view.rect(r);
            let radius = f.view.len(4.0);
            window
                .paint_image(clip, image, gpui_kit::Corners { top_left: radius, top_right: radius, bottom_left: radius, bottom_right: radius }, data.clone(), frame, false)
                .ok();
        }
        // Loading failed, or not decoded yet: say so where the picture goes.
        _ => {
            let s = f.view.len(22.0).min(b.size.height * 0.5);
            let at = Bounds { origin: point(b.origin.x + (b.size.width - s) / 2.0, b.origin.y + (b.size.height - s) / 2.0 - f.view.len(6.0)), size: size(s, s) };
            window.paint_svg(at, graphing_ui::kit::icon_path("ImageOff"), None, gpui_kit::TransformationMatrix::unit(), f.palette.muted, cx).ok();
            let size_px = 10.0 * f.view.zoom;
            let name = src.rsplit(['/', ':']).next().unwrap_or(src);
            let line = shape_text(name, size_px, f.palette.muted, Face::Sans, f, window);
            let y = at.origin.y + s + f.view.len(4.0);
            line.paint(point(b.origin.x + (b.size.width - line.width) / 2.0, y), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
        }
    }
}

fn node(f: &Frame, n: &NodeBox, window: &mut Window, cx: &mut App) {
    if let Some(w) = &n.wave {
        return wave(f, n, w, window, cx);
    }
    if let Some(src) = &n.reference {
        return link(f, n, src, window, cx);
    }
    let b = f.view.rect(n.rect);
    let tech = f.scene.technical;
    let fill = f.hex(n.fill, f.palette.fill);
    let stroke = f.hex(n.stroke, if tech { f.palette.edge } else { f.palette.stroke });
    let width = px(n.weight.map_or(if tech { 1.0 } else { 1.5 }, |w| w as f32 * f.view.zoom.clamp(0.6, 1.5)));
    // Explicit text color wins; otherwise pick whichever reads on the fill.
    let ink = match (n.text, n.fill) {
        (Some(c), _) => f.rgb(c),
        (None, Some(fill)) if n.shape != Shape::Actor => contrast(fill).opacity(f.fade),
        _ => f.palette.text,
    };
    match n.shape {
        Shape::Initial | Shape::Final => {
            let r = b.size.width.min(b.size.height) / 2.0;
            let c = point(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height / 2.0);
            let disc = |rad: Pixels| Bounds { origin: point(c.x - rad, c.y - rad), size: size(rad * 2.0, rad * 2.0) };
            if n.shape == Shape::Initial {
                window.paint_quad(quad(disc(r), r, ink, px(0.), ink, BorderStyle::default()));
            } else {
                window.paint_quad(quad(disc(r), r, gpui_kit::transparent_black(), px(1.5), ink, BorderStyle::default()));
                window.paint_quad(quad(disc(r * 0.6), r, ink, px(0.), ink, BorderStyle::default()));
            }
        }
        Shape::Bar => window.paint_quad(quad(b, f.view.len(1.5), ink, px(0.), ink, BorderStyle::default())),
        Shape::Rect | Shape::Rounded | Shape::Block => {
            let r = f.view.len(corner(n.shape, tech));
            window.paint_quad(quad(b, r, fill, width, stroke, BorderStyle::default()));
        }
        Shape::Lifeline => {
            let head = Bounds { origin: b.origin, size: size(b.size.width, f.view.len(LIFELINE_HEAD)) };
            window.paint_quad(quad(head, f.view.len(2.0), fill, width, stroke, BorderStyle::default()));
            let cx_ = b.origin.x + b.size.width / 2.0;
            let mut pb = PathBuilder::stroke(px(1.0)).dash_array(&[px(5.0), px(4.0)]);
            pb.move_to(point(cx_, head.origin.y + head.size.height));
            pb.line_to(point(cx_, b.origin.y + b.size.height));
            if let Ok(path) = pb.build() {
                window.paint_path(path, stroke);
            }
        }
        Shape::Path if n.path.is_some() => {
            let cmds = n.path.as_deref().unwrap_or_default();
            if let Ok(path) = custom_path(cmds, f.view, PathBuilder::fill()).build() {
                window.paint_path(path, fill);
            }
            if let Ok(path) = custom_path(cmds, f.view, PathBuilder::stroke(width)).build() {
                window.paint_path(path, stroke);
            }
        }
        shape => {
            if let Ok(path) = outline(shape, b, PathBuilder::fill()).build() {
                window.paint_path(path, fill);
            }
            if let Ok(path) = outline(shape, b, PathBuilder::stroke(width)).build() {
                window.paint_path(path, stroke);
            }
            if shape == Shape::Cylinder {
                // Front lip of the top ellipse.
                let ry = cyl_ry(b);
                let mut lip = PathBuilder::stroke(width);
                let y = b.origin.y + ry;
                lip.move_to(point(b.origin.x, y));
                lip.arc_to(point(b.size.width / 2.0, ry), px(0.), false, false, point(b.origin.x + b.size.width, y));
                if let Ok(path) = lip.build() {
                    window.paint_path(path, stroke);
                }
            }
        }
    }
    if n.pin_band {
        node_header(f, n, b, stroke, window, cx);
    }
    // Pictures cannot fade; they appear halfway through.
    if let Some(src) = &n.image
        && f.fade > 0.5
    {
        picture(f, n, src, b, window, cx);
    }
    overlays(f, n, ink, stroke, window, cx);
    if f.is_selected(&n.id) && f.editing != Some(n.id.as_str()) {
        let pad = px(4.0);
        let sel = Bounds {
            origin: point(b.origin.x - pad, b.origin.y - pad),
            size: size(b.size.width + pad * 2.0, b.size.height + pad * 2.0),
        };
        window.paint_quad(quad(sel, f.view.len(6.0), gpui_kit::transparent_black(), px(1.5), f.palette.accent, BorderStyle::default()));
    }

    let structured = n.shape == Shape::Block || !n.compartments.is_empty() || n.stereotype.is_some();
    if f.editing == Some(n.id.as_str()) || n.pin_band {
        // The in-place editor draws the label; a node graph node, its header.
    } else if structured {
        structured_text(f, n, b, ink, stroke, window, cx);
    } else if n.label_below {
        // Small symbols: the name reads under them, on the canvas.
        let lines = n.label.split('\n').count().max(1) as f32;
        let h = px(LABEL_PT * f.view.zoom * 1.3) * lines;
        let wide = b.size.width.max(f.view.len(160.0));
        let x = b.origin.x + (b.size.width - wide) / 2.0;
        label_block(&n.label, x, b.origin.y + b.size.height + f.view.len(4.0), wide, h, LABEL_PT * f.view.zoom, f.palette.text, f, window, cx);
    } else if !matches!(n.shape, Shape::Initial | Shape::Final | Shape::Bar) {
        let centered_glyph = n.glyph.as_ref().is_some_and(|g| g.at == graphing_scene::stencils::GlyphAt::Center);
        let a = f.view.rect(n.label_area);
        let (top, height) = match n.shape {
            // A centred icon keeps the top; the name goes under it.
            _ if centered_glyph => (b.origin.y + b.size.height * 0.6, b.size.height * 0.4),
            // Actors carry their label under the figure.
            Shape::Actor => (b.origin.y + b.size.height * 0.62, b.size.height * 0.38),
            Shape::Lifeline => (b.origin.y, f.view.len(LIFELINE_HEAD)),
            Shape::Package => (b.origin.y + f.view.len(PACKAGE_TAB), b.size.height - f.view.len(PACKAGE_TAB)),
            _ => (a.origin.y, a.size.height),
        };
        if n.notes.is_empty() {
            label_block(&n.label, a.origin.x, top, a.size.width, height, LABEL_PT * f.view.zoom, ink, f, window, cx);
        } else {
            label_with_notes(n, a.origin.x, top, a.size.width, height, ink, f, window, cx);
        }
    }
    for p in &n.ports {
        port(f, p, n.pin_band, stroke, fill, window, cx);
    }
}

/// Stereotype, header, divider and compartments, laid out like the SVG export.
fn structured_text(f: &Frame, n: &NodeBox, b: Bounds<Pixels>, ink: Hsla, stroke: Hsla, window: &mut Window, cx: &mut App) {
    let z = f.view.zoom;
    let tech = f.scene.technical;
    let centered = !(tech && n.shape == Shape::Block);
    let left = b.origin.x + f.view.len(14.0);
    let mut y = n.rect.origin.y;
    let at_y = |wy: f64| f.view.pt(graphing_model::Point::new(0.0, wy)).y;
    let place = |line: &gpui_kit::ShapedLine| if centered { b.origin.x + (b.size.width - line.width) / 2.0 } else { left };
    if let Some(st) = &n.stereotype {
        let size_px = 10.0 * z;
        let line = shape_text(&format!("\u{ab}{st}\u{bb}"), size_px, f.palette.muted, Face::Mono, f, window);
        let x = place(&line);
        line.paint(point(x, at_y(y + 6.0)), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
        y += notation::STEREO_H;
    }
    let size_px = HEADER_PT * z;
    let line = shape_text(&n.label, size_px, ink, Face::SansBold, f, window);
    let x = place(&line);
    line.paint(point(x, at_y(y + 9.0)), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
    y += notation::HEADER_H;
    let divider = if tech { f.palette.group_stroke } else { stroke };
    for c in &n.compartments {
        let yy = at_y(y);
        window.paint_quad(quad(
            Bounds { origin: point(b.origin.x, yy), size: size(b.size.width, px(1.0)) },
            px(0.),
            divider,
            px(0.),
            divider,
            BorderStyle::default(),
        ));
        y += notation::COMP_PAD;
        let mut rows: Vec<String> = c.title.iter().cloned().collect();
        let indent = if c.title.is_some() { "  " } else { "" };
        rows.extend(c.lines.iter().map(|l| format!("{indent}{l}")));
        for text in rows {
            let size_px = 10.5 * z;
            if size_px >= 4.0 {
                let line = shape_text(&text, size_px, f.palette.muted, Face::Mono, f, window);
                line.paint(point(left, at_y(y + 2.0)), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
            }
            y += notation::LINE_H;
        }
        y += notation::COMP_PAD / 2.0;
    }
}

/// A node-graph node's title band: the node's color (its stroke), the
/// title on it.
fn node_header(f: &Frame, n: &NodeBox, b: Bounds<Pixels>, stroke: Hsla, window: &mut Window, cx: &mut App) {
    let band = Bounds { origin: b.origin, size: size(b.size.width, f.view.len(pins::PIN_TOP as f32).min(b.size.height)) };
    let color = n.stroke.map_or(f.palette.accent.opacity(0.85 * f.fade), |c| f.rgb(c));
    let r = f.view.len(corner(n.shape, f.scene.technical));
    window.paint_quad(quad(band, gpui_kit::Corners { top_left: r, top_right: r, bottom_left: px(0.), bottom_right: px(0.) }, color, px(0.), stroke, BorderStyle::default()));
    let size_px = HEADER_PT * f.view.zoom;
    if size_px < 4.0 {
        return;
    }
    let ink = n.stroke.map_or(f.palette.text, |c| contrast(c).opacity(f.fade));
    let line = shape_text(&n.label, size_px, ink, Face::SansBold, f, window);
    let lh = px(size_px * 1.3);
    line.paint(point(b.origin.x + f.view.len(12.0), band.origin.y + (band.size.height - lh) / 2.0), lh, TextAlign::Left, None, window, cx).ok();
}

/// A node-graph pin: a dot (data) or an arrow (execution) in its type's
/// color, its name inside the node.
fn pin(f: &Frame, p: &PortBox, pin: &pins::Pin, inside: bool, window: &mut Window, cx: &mut App) {
    let c = f.view.pt(p.at);
    let r = f.view.len(pins::PIN_R as f32);
    let color = if pin.exec() { f.palette.edge } else { f.rgb(pins::color(pin.shown_type())) };
    if pin.exec() {
        // Points along the flow: right on the sides, down on top and bottom.
        let tri = match p.side {
            Side::Left | Side::Right => [point(c.x - r, c.y - r), point(c.x + r, c.y), point(c.x - r, c.y + r)],
            Side::Top | Side::Bottom => [point(c.x - r, c.y - r), point(c.x + r, c.y - r), point(c.x, c.y + r)],
        };
        let mut pb = PathBuilder::fill();
        pb.add_polygon(&tri, true);
        if let Ok(path) = pb.build() {
            window.paint_path(path, color);
        }
    } else {
        let dot = Bounds { origin: point(c.x - r, c.y - r), size: size(r * 2.0, r * 2.0) };
        window.paint_quad(quad(dot, r, color, px(1.0), f.palette.bg, BorderStyle::default()));
    }
    let size_px = pins::PIN_PT as f32 * f.view.zoom;
    let caption = pin.caption();
    if size_px < 4.0 || caption.is_empty() {
        return;
    }
    let line = shape_text(&caption, size_px, f.palette.text, Face::Sans, f, window);
    let lh = px(size_px * 1.3);
    let gap = r + f.view.len(6.0);
    let origin = match (p.side, inside) {
        (Side::Left, true) => point(c.x + gap, c.y - lh / 2.0),
        (Side::Right, true) => point(c.x - gap - line.width, c.y - lh / 2.0),
        (Side::Bottom, true) => point(c.x - line.width / 2.0, c.y - gap - lh),
        // Outside a shape that has content of its own, above the wire.
        (Side::Left, false) => point(c.x - gap - line.width, c.y - lh),
        (Side::Right, false) => point(c.x + gap, c.y - lh),
        (Side::Bottom, false) => point(c.x + r + f.view.len(3.0), c.y + r),
        // Above the node, clear of its title, beside the wire coming in.
        (Side::Top, _) => point(c.x + r + f.view.len(3.0), c.y - r - lh),
    };
    line.paint(origin, lh, TextAlign::Left, None, window, cx).ok();
}

fn port(f: &Frame, p: &PortBox, inside: bool, stroke: Hsla, fill: Hsla, window: &mut Window, cx: &mut App) {
    if let Some(info) = &p.pin {
        return pin(f, p, info, inside, window, cx);
    }
    let c = f.view.pt(p.at);
    let h = f.view.len(PORT as f32 / 2.0);
    let sq = Bounds { origin: point(c.x - h, c.y - h), size: size(h * 2.0, h * 2.0) };
    window.paint_quad(quad(sq, px(0.), fill, px(1.0), stroke, BorderStyle::default()));
    let size_px = 9.5 * f.view.zoom;
    if size_px < 4.0 {
        return;
    }
    let line = shape_text(&p.name, size_px, f.palette.muted, Face::Mono, f, window);
    let lh = px(size_px * 1.3);
    let gap = f.view.len(12.0);
    // Names sit clear of the connector: below a horizontal one, beside a vertical one.
    let origin = match p.side {
        Side::Left => point(c.x - gap - line.width, c.y + f.view.len(6.0)),
        Side::Right => point(c.x + gap, c.y + f.view.len(6.0)),
        Side::Top => point(c.x - gap - line.width, c.y - f.view.len(6.0) - lh),
        Side::Bottom => point(c.x - gap - line.width, c.y + f.view.len(6.0)),
    };
    line.paint(origin, lh, TextAlign::Left, None, window, cx).ok();
}

/// Dark text on light fills, light text on dark ones.
fn contrast(fill: u32) -> Hsla {
    let ch = |shift: u32| ((fill >> shift) & 0xff) as f32 / 255.0;
    let luma = 0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0);
    if luma > 0.55 { rgb(0x212529).into() } else { rgb(0xf1f3f5).into() }
}

fn cyl_ry(b: Bounds<Pixels>) -> Pixels {
    (b.size.height * 0.12).min(b.size.width * 0.2)
}

const LIFELINE_HEAD: f32 = 40.0;
const PACKAGE_TAB: f32 = 18.0;

/// A pack's custom outline (already fitted, world units) as a gpui path.
fn custom_path(cmds: &[graphing_scene::path::PathCmd], view: View, mut pb: PathBuilder) -> PathBuilder {
    use graphing_scene::path::PathCmd as C;
    for c in cmds {
        match *c {
            C::Move(p) => pb.move_to(view.pt(p)),
            C::Line(p) => pb.line_to(view.pt(p)),
            C::Cubic(a, b, p) => pb.cubic_bezier_to(view.pt(p), view.pt(a), view.pt(b)),
            C::Quad(a, p) => pb.curve_to(view.pt(p), view.pt(a)),
            C::Arc { rx, ry, rotation, large, sweep, to } => {
                pb.arc_to(point(view.len(rx as f32), view.len(ry as f32)), px(rotation as f32), large, sweep, view.pt(to))
            }
            C::Close => pb.close(),
        }
    }
    pb
}

/// Add the outline of `shape` in `b` to a path builder.
fn outline(shape: Shape, b: Bounds<Pixels>, mut pb: PathBuilder) -> PathBuilder {
    let (x, y, w, h) = (b.origin.x, b.origin.y, b.size.width, b.size.height);
    let p = |px_: Pixels, py_: Pixels| point(px_, py_);
    match shape {
        Shape::Ellipse | Shape::Initial | Shape::Final => {
            let r = point(w / 2.0, h / 2.0);
            pb.move_to(p(x, y + h / 2.0));
            pb.arc_to(r, px(0.), false, true, p(x + w, y + h / 2.0));
            pb.arc_to(r, px(0.), false, true, p(x, y + h / 2.0));
            pb.close();
        }
        Shape::Diamond => {
            pb.add_polygon(&[p(x + w / 2.0, y), p(x + w, y + h / 2.0), p(x + w / 2.0, y + h), p(x, y + h / 2.0)], true);
        }
        Shape::Cylinder => {
            let ry = cyl_ry(b);
            let r = point(w / 2.0, ry);
            pb.move_to(p(x, y + ry));
            pb.arc_to(r, px(0.), false, true, p(x + w, y + ry));
            pb.line_to(p(x + w, y + h - ry));
            pb.arc_to(r, px(0.), false, true, p(x, y + h - ry));
            pb.close();
        }
        Shape::Parallelogram => {
            let s = w * 0.15;
            pb.add_polygon(&[p(x + s, y), p(x + w, y), p(x + w - s, y + h), p(x, y + h)], true);
        }
        Shape::Hexagon => {
            let s = (w * 0.15).min(h / 2.0);
            pb.add_polygon(
                &[p(x + s, y), p(x + w - s, y), p(x + w, y + h / 2.0), p(x + w - s, y + h), p(x + s, y + h), p(x, y + h / 2.0)],
                true,
            );
        }
        Shape::Note => {
            let fold = (w.min(h)) * 0.25;
            pb.add_polygon(&[p(x, y), p(x + w - fold, y), p(x + w, y + fold), p(x + w, y + h), p(x, y + h)], true);
        }
        Shape::Package => {
            let tab = (w * 0.4).min(px(140.0));
            let th = h.min(px(PACKAGE_TAB)) * (w / w.max(px(1.0)));
            pb.add_polygon(&[p(x, y), p(x + tab, y), p(x + tab, y + th), p(x + w, y + th), p(x + w, y + h), p(x, y + h)], true);
        }
        Shape::Actor => {
            // Head and body inside the top 60% of the box.
            let cx_ = x + w / 2.0;
            let fig = h * 0.6;
            let head = fig * 0.18;
            pb.move_to(p(cx_ + head, y + head));
            pb.arc_to(point(head, head), px(0.), false, true, p(cx_ - head, y + head));
            pb.arc_to(point(head, head), px(0.), false, true, p(cx_ + head, y + head));
            pb.close();
            let arm = fig * 0.3;
            pb.move_to(p(cx_, y + head * 2.0));
            pb.line_to(p(cx_, y + fig * 0.7));
            pb.move_to(p(cx_ - arm, y + fig * 0.45));
            pb.line_to(p(cx_ + arm, y + fig * 0.45));
            pb.move_to(p(cx_ - arm * 0.8, y + fig));
            pb.line_to(p(cx_, y + fig * 0.7));
            pb.line_to(p(cx_ + arm * 0.8, y + fig));
        }
        Shape::Rect | Shape::Rounded | Shape::Block | Shape::Bar | Shape::Lifeline | Shape::Path => {
            pb.add_polygon(&[p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h)], true);
        }
    }
    pb
}

fn edge(f: &Frame, e: &EdgeLine, window: &mut Window, cx: &mut App) {
    if e.points.len() < 2 {
        return;
    }
    let tech = f.scene.technical;
    let pts: Vec<Point<Pixels>> = e.points.iter().map(|&p| f.view.pt(p)).collect();
    let selected = f.is_selected(&e.id);
    let color = match (selected, e.problem) {
        (true, _) => f.palette.accent,
        (false, true) => f.palette.danger,
        (false, false) => f.hex(e.stroke, f.palette.edge),
    };
    let base = if tech { 1.25 } else { 1.5 };
    let mut pb = PathBuilder::stroke(px(if selected { base + 1.0 } else { base }));
    if e.dashed {
        pb = pb.dash_array(&[px(6.0), px(4.0)]);
    }
    pb.move_to(pts[0]);
    for &p in &pts[1..] {
        pb.line_to(p);
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
    let n = pts.len();
    end_marker(e.head, pts[n - 2], pts[n - 1], f.view.zoom, color, f.palette.bg, window);
    end_marker(e.tail, pts[1], pts[0], f.view.zoom, color, f.palette.bg, window);

    let mut rows: Vec<(String, bool)> = Vec::new();
    if let Some(st) = &e.stereotype {
        rows.push((format!("\u{ab} {st} \u{bb}"), true));
    }
    if let Some(l) = &e.label {
        rows.extend(l.split('\n').map(|l| (l.to_string(), false)));
    }
    if rows.is_empty() || f.editing == Some(e.id.as_str()) {
        return;
    }
    // Middle of the middle segment.
    let i = (n - 1) / 2;
    let (a, b) = (pts[i], pts[i + 1]);
    let mid = point((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let horizontal = (b.x - a.x).abs() >= (b.y - a.y).abs();
    let z = f.view.zoom;
    if tech || e.stereotype.is_some() {
        // Above a horizontal connector, beside a vertical one.
        let step = f.view.len(13.0);
        let count = rows.len() as f32;
        for (j, (text, stereo)) in rows.iter().enumerate() {
            let (size_px, fill, face) = if *stereo { (10.0 * z, f.palette.flow, Face::Mono) } else { (10.5 * z, f.palette.text, Face::Sans) };
            let line = shape_text(text, size_px, fill, face, f, window);
            let lh = px(size_px * 1.3);
            let origin = if horizontal {
                point(mid.x - line.width / 2.0, mid.y - f.view.len(10.0) - step * (count - 1.0 - j as f32) - lh)
            } else {
                point(mid.x + f.view.len(12.0), mid.y - f.view.len(16.0) + step * j as f32)
            };
            line.paint(origin, lh, TextAlign::Left, None, window, cx).ok();
        }
    } else {
        let font = EDGE_PT * z;
        let text = rows.iter().map(|r| r.0.clone()).collect::<Vec<_>>().join(" ");
        let line = shape_text(&text, font, f.palette.text, Face::Sans, f, window);
        let lh = px(font * 1.4);
        let pad = px(4.0);
        let bg = Bounds {
            origin: point(mid.x - line.width / 2.0 - pad, mid.y - lh / 2.0),
            size: size(line.width + pad * 2.0, lh),
        };
        window.paint_quad(quad(bg, px(3.0), f.palette.bg, px(0.), f.palette.bg, BorderStyle::default()));
        line.paint(point(mid.x - line.width / 2.0, mid.y - lh / 2.0), lh, TextAlign::Left, None, window, cx).ok();
    }
}

/// End decoration at `tip`, pointing away from `from`.
fn end_marker(kind: End, from: Point<Pixels>, tip: Point<Pixels>, zoom: f32, color: Hsla, bg: Hsla, window: &mut Window) {
    let (dx, dy) = (f32::from(tip.x - from.x), f32::from(tip.y - from.y));
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 || kind == End::None {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let k = zoom.max(0.5);
    let at = |back: f32, side: f32| point(tip.x - px((ux * back + uy * side) * k), tip.y - px((uy * back - ux * side) * k));
    let fill_poly = |pts: &[Point<Pixels>], fill: Hsla, window: &mut Window| {
        let mut pb = PathBuilder::fill();
        pb.add_polygon(pts, true);
        if let Ok(path) = pb.build() {
            window.paint_path(path, fill);
        }
    };
    let stroke_poly = |pts: &[Point<Pixels>], close: bool, window: &mut Window| {
        let mut pb = PathBuilder::stroke(px(1.25));
        pb.add_polygon(pts, close);
        if let Ok(path) = pb.build() {
            window.paint_path(path, color);
        }
    };
    match kind {
        End::Arrow => fill_poly(&[tip, at(11.0, 5.0), at(11.0, -5.0)], color, window),
        End::Open => stroke_poly(&[at(10.0, 5.5), tip, at(10.0, -5.5)], false, window),
        End::Triangle => {
            let pts = [tip, at(13.0, 7.0), at(13.0, -7.0)];
            fill_poly(&pts, bg, window);
            stroke_poly(&pts, true, window);
        }
        End::Diamond | End::FilledDiamond => {
            let pts = [tip, at(8.0, 5.0), at(16.0, 0.0), at(8.0, -5.0)];
            fill_poly(&pts, if kind == End::FilledDiamond { color } else { bg }, window);
            stroke_poly(&pts, true, window);
        }
        End::Circle => {
            let c = at(5.0, 0.0);
            let r = px(5.0 * k);
            let b = Bounds { origin: point(c.x - r, c.y - r), size: size(r * 2.0, r * 2.0) };
            window.paint_quad(quad(b, r, bg, px(1.25), color, BorderStyle::default()));
        }
        End::None => {}
        crow => {
            for st in crow.crow_strokes() {
                match st {
                    EndStroke::Line(b1, s1, b2, s2) => stroke_poly(&[at(b1 as f32, s1 as f32), at(b2 as f32, s2 as f32)], false, window),
                    EndStroke::Ring { back, r } => {
                        let c = at(back as f32, 0.0);
                        let r = px(r as f32 * k);
                        let b = Bounds { origin: point(c.x - r, c.y - r), size: size(r * 2.0, r * 2.0) };
                        window.paint_quad(quad(b, r, bg, px(1.25), color, BorderStyle::default()));
                    }
                }
            }
        }
    }
}

/// Filled arrowhead used by the link preview.
fn arrowhead(from: Point<Pixels>, tip: Point<Pixels>, zoom: f32, color: Hsla, window: &mut Window) {
    end_marker(End::Arrow, from, tip, zoom, color, color, window);
}

fn shape_text(text: &str, font_size: f32, color: Hsla, face: Face, f: &Frame, window: &mut Window) -> gpui_kit::ShapedLine {
    let mut font = window.text_style().font();
    match face {
        Face::Sans => {}
        Face::SansBold => font.weight = gpui_kit::FontWeight::SEMIBOLD,
        Face::Mono => font = gpui_kit::font(f.mono.clone()),
    }
    let run = TextRun { len: text.len(), font, color, background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line(SharedString::from(text.to_string()), px(font_size.max(1.0)), &[run], None)
}

/// Centered multi-line label inside a box.
#[allow(clippy::too_many_arguments)]
fn label_block(
    text: &str,
    x: Pixels,
    y: Pixels,
    w: Pixels,
    h: Pixels,
    font_size: f32,
    color: Hsla,
    f: &Frame,
    window: &mut Window,
    cx: &mut App,
) {
    if font_size < 4.0 {
        return;
    }
    let lh = px(font_size * 1.3);
    let lines: Vec<&str> = text.split('\n').collect();
    let total = lh * lines.len() as f32;
    let mut top = y + (h - total) / 2.0;
    for l in lines {
        let line = shape_text(l, font_size, color, Face::Sans, f, window);
        let left = x + (w - line.width) / 2.0;
        line.paint(point(left, top), lh, TextAlign::Left, None, window, cx).ok();
        top += lh;
    }
}

/// Bold-free label over smaller notes, centred together in the area (C4
/// style: name, `[Container: tech]`, description).
#[allow(clippy::too_many_arguments)]
fn label_with_notes(n: &NodeBox, x: Pixels, y: Pixels, w: Pixels, h: Pixels, ink: Hsla, f: &Frame, window: &mut Window, cx: &mut App) {
    let z = f.view.zoom;
    let (big, small) = (LABEL_PT * z, graphing_scene::notation::NOTE_PT as f32 * z);
    if small < 4.0 {
        return;
    }
    let (lh, nh) = (px(big * 1.3), px(small * 1.3));
    let labels: Vec<&str> = n.label.split('\n').filter(|l| !l.is_empty()).collect();
    let gap = f.view.len(4.0);
    let total = lh * labels.len() as f32 + gap + nh * n.notes.len() as f32;
    let mut top = y + (h - total) / 2.0;
    for l in labels {
        let line = shape_text(l, big, ink, Face::SansBold, f, window);
        line.paint(point(x + (w - line.width) / 2.0, top), lh, TextAlign::Left, None, window, cx).ok();
        top += lh;
    }
    top += gap;
    for l in &n.notes {
        let line = shape_text(l.trim_start(), small, ink.opacity(0.8), Face::Sans, f, window);
        line.paint(point(x + (w - line.width) / 2.0, top), nh, TextAlign::Left, None, window, cx).ok();
        top += nh;
    }
}

/// A diagram link: a card titled after the linked diagram, with a
/// miniature of it (groups, lines, boxes), or a note when it is missing.
fn link(f: &Frame, n: &NodeBox, src: &str, window: &mut Window, cx: &mut App) {
    let b = f.view.rect(n.rect);
    let fill = f.hex(n.fill, f.palette.fill);
    let stroke = f.hex(n.stroke, f.palette.stroke);
    let ink = n.text.map_or(f.palette.text, |c| f.rgb(c));
    window.paint_quad(quad(b, f.view.len(10.0), fill, px(1.5), stroke, BorderStyle::default()));
    let (head, area) = graphing_scene::link_layout(n.rect);
    let rule = f.view.rect(Rect::new(n.rect.origin.x, head.origin.y + head.size.h, n.rect.size.w, 0.0));
    window.paint_quad(quad(Bounds { origin: rule.origin, size: size(rule.size.width, px(1.0)) }, px(0.), f.palette.group_stroke, px(0.), f.palette.group_stroke, BorderStyle::default()));
    let linked = f.linked.get(src).and_then(|(_, l)| l.clone());
    let title = if n.label.is_empty() { linked.as_ref().map_or_else(|| src.to_string(), |l| l.title.clone()) } else { n.label.clone() };
    let size_px = 13.0 * f.view.zoom;
    let line = shape_text(&title, size_px, ink, Face::SansBold, f, window);
    let hb = f.view.rect(head);
    line.paint(point(hb.origin.x + f.view.len(12.0), hb.origin.y + (hb.size.height - px(size_px * 1.3)) / 2.0), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
    match linked.as_ref().and_then(|l| l.scene.miniature(area)) {
        Some(m) => {
            for g in &m.groups {
                window.paint_quad(quad(f.view.rect(*g), f.view.len(2.0), gpui_kit::transparent_black(), px(1.0), f.palette.group_stroke, BorderStyle::default()));
            }
            for l in &m.lines {
                if l.len() < 2 {
                    continue;
                }
                let mut pb = PathBuilder::stroke(px(1.0));
                pb.move_to(f.view.pt(l[0]));
                for &p in &l[1..] {
                    pb.line_to(f.view.pt(p));
                }
                if let Ok(path) = pb.build() {
                    window.paint_path(path, f.palette.muted);
                }
            }
            for r in &m.nodes {
                let mut rb = f.view.rect(*r);
                rb.size = size(rb.size.width.max(px(1.5)), rb.size.height.max(px(1.5)));
                window.paint_quad(quad(rb, f.view.len(1.5), f.palette.muted.opacity(0.55), px(0.), f.palette.muted, BorderStyle::default()));
            }
        }
        None => {
            let what = if src.is_empty() { "No file set".to_string() } else { format!("Missing: {src}") };
            let size_px = 11.0 * f.view.zoom;
            let t = shape_text(&what, size_px, f.palette.muted, Face::Sans, f, window);
            let ab = f.view.rect(area);
            t.paint(point(ab.origin.x + (ab.size.width - t.width) / 2.0, ab.origin.y + (ab.size.height - px(size_px * 1.3)) / 2.0), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
        }
    }
    overlays(f, n, ink, stroke, window, cx);
    if f.is_selected(&n.id) {
        let pad = px(4.0);
        let sel = Bounds { origin: point(b.origin.x - pad, b.origin.y - pad), size: size(b.size.width + pad * 2.0, b.size.height + pad * 2.0) };
        window.paint_quad(quad(sel, f.view.len(12.0), gpui_kit::transparent_black(), px(1.5), f.palette.accent, BorderStyle::default()));
    }
}

/// A timing signal: its name on the left, a faint slot grid, the levels,
/// clocks and buses of its wave.
fn wave(f: &Frame, n: &NodeBox, w: &graphing_scene::wave::Wave, window: &mut Window, cx: &mut App) {
    use graphing_scene::wave::WavePrim;
    let b = f.view.rect(n.rect);
    let line = f.hex(n.stroke, f.palette.text);
    let grid = f.palette.grid;
    for &x in &w.ticks {
        let sx = f.view.pt(graphing_model::Point::new(x, 0.0)).x;
        window.paint_quad(quad(Bounds { origin: point(sx, b.origin.y), size: size(px(1.0), b.size.height) }, px(0.), grid, px(0.), grid, BorderStyle::default()));
    }
    let poly = |pts: &[graphing_model::Point], mut pb: PathBuilder, close: bool| {
        pb.add_polygon(&pts.iter().map(|&p| f.view.pt(p)).collect::<Vec<_>>(), close);
        pb.build()
    };
    for prim in &w.prims {
        match prim {
            WavePrim::Line(pts) => {
                if let Ok(path) = poly(pts, PathBuilder::stroke(px(1.6)), false) {
                    window.paint_path(path, line);
                }
            }
            WavePrim::Bus { points, label, unknown, center } => {
                let fill = if *unknown { f.palette.muted.opacity(0.22) } else { f.hex(n.fill, f.palette.accent.opacity(0.14)) };
                if let Ok(path) = poly(points, PathBuilder::fill(), true) {
                    window.paint_path(path, fill);
                }
                if let Ok(path) = poly(points, PathBuilder::stroke(px(1.3)), true) {
                    window.paint_path(path, line);
                }
                if let Some(text) = label {
                    let size_px = 11.0 * f.view.zoom;
                    let t = shape_text(text, size_px, f.palette.text, Face::Mono, f, window);
                    let c = f.view.pt(*center);
                    t.paint(point(c.x - t.width / 2.0, c.y - px(size_px * 0.65)), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
                }
            }
        }
    }
    // The name, left, in the label column.
    let size_px = 13.0 * f.view.zoom;
    let name = shape_text(&n.label, size_px, f.palette.text, Face::SansBold, f, window);
    let lb = f.view.rect(w.label);
    name.paint(point(lb.origin.x + f.view.len(4.0), lb.origin.y + (lb.size.height - px(size_px * 1.3)) / 2.0), px(size_px * 1.3), TextAlign::Left, None, window, cx).ok();
    if f.is_selected(&n.id) {
        let pad = px(4.0);
        let sel = Bounds { origin: point(b.origin.x - pad, b.origin.y - pad), size: size(b.size.width + pad * 2.0, b.size.height + pad * 2.0) };
        window.paint_quad(quad(sel, f.view.len(6.0), gpui_kit::transparent_black(), px(1.5), f.palette.accent, BorderStyle::default()));
    }
}

/// A pack's strokes and marks over the outline, and its icon.
fn overlays(f: &Frame, n: &NodeBox, ink: Hsla, stroke: Hsla, window: &mut Window, cx: &mut App) {
    let line = px(if f.scene.technical { 1.0 } else { 1.25 });
    if let Some(cmds) = &n.detail
        && let Ok(path) = custom_path(cmds, f.view, PathBuilder::stroke(line)).build()
    {
        window.paint_path(path, stroke);
    }
    if let Some(cmds) = &n.mark
        && let Ok(path) = custom_path(cmds, f.view, PathBuilder::fill()).build()
    {
        window.paint_path(path, stroke);
    }
    if let Some((g, at)) = n.glyph_at() {
        window.paint_svg(f.view.rect(at), graphing_ui::kit::icon_path(&g.icon), None, gpui_kit::TransformationMatrix::unit(), ink, cx).ok();
    }
}

/// Small shape glyph for palettes and pickers.
pub fn preview(shape: Shape, b: Bounds<Pixels>, fill: Hsla, stroke: Hsla, window: &mut Window) {
    // On the canvas an actor keeps the bottom of its box for the label; a
    // glyph has no label, so let the figure use the full height.
    let b = if shape == Shape::Actor { Bounds { origin: b.origin, size: size(b.size.width, b.size.height / 0.6) } } else { b };
    match shape {
        Shape::Rect | Shape::Rounded => {
            let r = if shape == Shape::Rounded { px(5.0) } else { px(1.5) };
            window.paint_quad(quad(b, r, fill, px(1.2), stroke, BorderStyle::default()));
        }
        shape => {
            if let Ok(path) = outline(shape, b, PathBuilder::fill()).build() {
                window.paint_path(path, fill);
            }
            if let Ok(path) = outline(shape, b, PathBuilder::stroke(px(1.2))).build() {
                window.paint_path(path, stroke);
            }
        }
    }
}

/// `b` as a diagram rect, and the view that maps it back to `b`, so tile
/// previews draw with the canvas code.
fn tile_space(b: Bounds<Pixels>) -> (Rect, View) {
    let r = Rect::new(f32::from(b.origin.x) as f64, f32::from(b.origin.y) as f64, f32::from(b.size.width) as f64, f32::from(b.size.height) as f64);
    (r, View { origin: point(px(0.), px(0.)), offset: point(0.0, 0.0), zoom: 1.0 })
}

/// A stencil's details, marks and icon over its preview in `b`.
pub fn preview_overlays(def: &graphing_scene::stencils::StencilDef, b: Bounds<Pixels>, stroke: Hsla, window: &mut Window, cx: &mut App) {
    let (r, view) = tile_space(b);
    if let Some(o) = &def.detail
        && let Ok(path) = custom_path(&o.fit(r), view, PathBuilder::stroke(px(1.0))).build()
    {
        window.paint_path(path, stroke);
    }
    if let Some(o) = &def.mark
        && let Ok(path) = custom_path(&o.fit(r), view, PathBuilder::fill()).build()
    {
        window.paint_path(path, stroke);
    }
    if let Some(g) = &def.glyph {
        // Tiles are small: corner icons shrink, centred ones fill more.
        let zoom = f32::from(b.size.height) as f64 / 56.0;
        let g = graphing_scene::stencils::Glyph { size: Some(g.size.unwrap_or(0.45).max(0.6)), ..g.clone() };
        let at = view.rect(g.rect(r, false, zoom));
        window.paint_svg(at, graphing_ui::kit::icon_path(&g.icon), None, gpui_kit::TransformationMatrix::unit(), stroke, cx).ok();
    }
}

/// A pack's custom outline, fitted into `b`.
pub fn preview_path(o: &graphing_scene::path::Outline, b: Bounds<Pixels>, fill: Hsla, stroke: Hsla, window: &mut Window) {
    let (r, view) = tile_space(b);
    let cmds = o.fit(r);
    if let Ok(path) = custom_path(&cmds, view, PathBuilder::fill()).build() {
        window.paint_path(path, fill);
    }
    if let Ok(path) = custom_path(&cmds, view, PathBuilder::stroke(px(1.2))).build() {
        window.paint_path(path, stroke);
    }
}
