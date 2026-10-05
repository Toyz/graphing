//! Diagram animation: how every element looks at a moment of the
//! `animate { }` steps. UI-free, so the canvas, the SVG/GIF exporters and
//! the CLI all play the same thing.
//!
//! - `show` fades targets in; anything a `show` names is hidden before its
//!   step. `hide` fades them out. A group carries its members with it.
//! - `flow` sends dots along edges for the length of the step.
//! - `highlight` makes targets glow for the length of the step.
//! - `focus` moves the camera onto the targets; `focus all` back out.
//! - `move a x y` slides a shape (a group: everything in it) to a new
//!   place during the step; lines re-route as their ends travel.
//! - `ease` picks how a step's fades, camera and moves speed up and settle.
//! - Edges fade with their ends, so none dangles.

use std::collections::{HashMap, HashSet};

use graphing_model::{Diagram, Ease, Point, Rect, Verb};

use crate::Scene;

/// Seconds a step lasts when it does not say.
pub const DEFAULT_STEP: f64 = 2.0;
/// Seconds a fade takes.
pub const FADE: f64 = 0.45;
/// Seconds the camera takes to reach a new focus nearby; farther moves
/// take longer, up to [`MOVE_MAX`].
pub const MOVE: f64 = 0.9;
pub const MOVE_MAX: f64 = 1.8;
/// Diagram units a flow dot travels per second.
pub const FLOW_SPEED: f64 = 90.0;
/// Distance between flow dots along an edge.
pub const FLOW_GAP: f64 = 46.0;
/// Room around focused elements.
const FOCUS_PAD: f64 = 70.0;
/// Seconds the glow and caption take to come and go.
const EASE_IN: f64 = 0.3;

/// One stretch of a node's travel: start second, seconds, from, to, curve.
type Leg = (f64, f64, Point, Point, Ease);

/// The `animate` steps laid out in time.
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    /// Each step: start second, length, title.
    pub spans: Vec<(f64, f64, Option<String>)>,
    pub total: f64,
    /// Visibility changes per id: (second, visible, curve), in time order.
    fades: HashMap<String, Vec<(f64, bool, Ease)>>,
    /// Node travel: (start, seconds, from, to, curve), in time order.
    moves: HashMap<String, Vec<Leg>>,
    /// Where moving nodes sit before any step.
    home: HashMap<String, Point>,
    /// The diagram, to rebuild the scene while shapes move.
    diagram: Diagram,
    /// Ids hidden until a `show`.
    hidden_at_start: HashSet<String>,
    /// Flowing edges and highlighted ids per step.
    flows: Vec<Vec<String>>,
    glows: Vec<Vec<String>>,
    /// Per step, what drops back: the paths a branch does not take.
    dims: Vec<Vec<String>>,
    /// Camera moves: start, target, curve, seconds.
    focus: Vec<(f64, Rect, Ease, f64)>,
    /// Everything, for `focus all` and the start.
    whole: Option<Rect>,
    /// Edge id -> (from, to).
    ends: HashMap<String, (String, String)>,
}

/// One moment of the animation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimState {
    /// Opacity of node, group and edge ids that are not fully shown.
    pub alpha: HashMap<String, f32>,
    /// Flowing edges and how far (diagram units) their dots have moved.
    pub flow: HashMap<String, f64>,
    /// Glow strength (0..1) per highlighted id.
    pub glow: HashMap<String, f32>,
    /// What the camera frames, when the animation moves it.
    pub camera: Option<Rect>,
    /// Nodes away from their place, by top-left corner (see
    /// [`Timeline::scene_for`]).
    pub moved: HashMap<String, Point>,
    /// The step playing, and its title with an opacity.
    pub step: Option<usize>,
    pub caption: Option<(String, f32)>,
}

impl AnimState {
    pub fn alpha(&self, id: &str) -> f32 {
        self.alpha.get(id).copied().unwrap_or(1.0)
    }
}

fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

fn lerp_rect(a: Rect, b: Rect, t: f64) -> Rect {
    let l = |x: f64, y: f64| x + (y - x) * t;
    Rect::new(l(a.origin.x, b.origin.x), l(a.origin.y, b.origin.y), l(a.size.w, b.size.w), l(a.size.h, b.size.h))
}

/// `id` and, for a group, everything inside it.
fn expand(d: &Diagram, id: &str, out: &mut Vec<String>) {
    if out.iter().any(|o| o == id) {
        return;
    }
    out.push(id.to_string());
    if let Some(g) = d.group(id) {
        for m in &g.members {
            expand(d, m, out);
        }
    }
}

impl Timeline {
    pub fn new(d: &Diagram, scene: &Scene) -> Self {
        let mut t = Timeline { whole: scene.content_bounds().map(|r| r.inflate(FOCUS_PAD)), diagram: d.clone(), ..Default::default() };
        t.ends = d.edges.iter().map(|e| (e.id.clone(), (e.from.clone(), e.to.clone()))).collect();
        let start: HashMap<String, Point> = scene.nodes.iter().map(|n| (n.id.clone(), n.rect.origin)).collect();
        // Where nodes are as steps move them, and how far groups have gone.
        let mut pos = start.clone();
        let mut shifted: HashMap<String, (f64, f64)> = HashMap::new();
        let mut at = 0.0;
        let mut seen: HashSet<String> = HashSet::new();
        // Where the camera is framed; `None` while it shows everything.
        let mut camera: Option<Rect> = None;
        for step in &d.steps {
            let len = step.seconds.filter(|s| *s > 0.0).unwrap_or(DEFAULT_STEP);
            let ease = step.ease.unwrap_or_default();
            let travel = (len * 0.7).min(1.4);
            for (id, to) in &step.moves {
                // A group moves by its frame's corner; its nodes go along.
                let (dx, dy) = match scene.groups.iter().find(|g| &g.id == id) {
                    Some(g) => {
                        let (sx, sy) = shifted.get(id).copied().unwrap_or_default();
                        (to.x - (g.rect.origin.x + sx), to.y - (g.rect.origin.y + sy))
                    }
                    None => match pos.get(id) {
                        Some(p) => (to.x - p.x, to.y - p.y),
                        None => continue,
                    },
                };
                let mut ids = Vec::new();
                expand(d, id, &mut ids);
                for g in ids.iter().filter(|i| d.group(i).is_some()) {
                    let e = shifted.entry(g.clone()).or_default();
                    (e.0, e.1) = (e.0 + dx, e.1 + dy);
                }
                for n in &ids {
                    let Some(&from) = pos.get(n) else { continue };
                    let dest = Point::new(from.x + dx, from.y + dy);
                    t.moves.entry(n.clone()).or_default().push((at, travel, from, dest, ease));
                    pos.insert(n.clone(), dest);
                }
            }
            // The box around `ids` once this step's moves land.
            let frame = |ids: &[String]| {
                let offset = |id: &str| match (pos.get(id), start.get(id)) {
                    (Some(p), Some(s)) => (p.x - s.x, p.y - s.y),
                    _ => shifted.get(id).copied().unwrap_or_default(),
                };
                ids.iter()
                    .filter_map(|id| {
                        let (dx, dy) = offset(id);
                        footprint(scene, id).map(|r| Rect::new(r.origin.x + dx, r.origin.y + dy, r.size.w, r.size.h))
                    })
                    .reduce(|a, b| a.union(b))
                    .map(|r| r.inflate(FOCUS_PAD))
            };
            let (mut flows, mut glows, mut introduced) = (Vec::new(), Vec::new(), Vec::new());
            let mut focused = false;
            for a in &step.actions {
                let mut ids = Vec::new();
                for target in &a.targets {
                    expand(d, target, &mut ids);
                }
                match a.verb {
                    Verb::Show | Verb::Hide => {
                        let show = a.verb == Verb::Show;
                        for id in ids {
                            // First mention a `show`: hidden until then.
                            if seen.insert(id.clone()) && show {
                                t.hidden_at_start.insert(id.clone());
                            }
                            if show {
                                introduced.push(id.clone());
                            }
                            t.fades.entry(id).or_default().push((at, show, ease));
                        }
                    }
                    Verb::Flow => {
                        introduced.extend(ids.iter().cloned());
                        flows.extend(ids.into_iter().filter(|id| t.ends.contains_key(id)));
                    }
                    Verb::Highlight => {
                        introduced.extend(ids.iter().cloned());
                        glows.extend(ids);
                    }
                    Verb::Focus => {
                        let rect = if a.targets.iter().any(|x| x == "all") { t.whole } else { frame(&ids) };
                        if let Some(r) = rect {
                            focused = true;
                            t.focus.push((at, r, ease, move_time(camera.or(t.whole), r)));
                            camera = Some(r);
                        }
                    }
                }
            }
            // A step that brings in something the camera does not frame,
            // without saying where to look, widens the view to take it in.
            if !focused
                && let Some(cam) = camera
                && let Some(need) = frame(&introduced)
                && !contains(cam, need)
            {
                let r = cam.union(need);
                camera = Some(r);
                t.focus.push((at, r, ease, move_time(Some(cam), r)));
            }
            t.dims.push(untaken(d, &flows, &glows));
            t.flows.push(flows);
            t.glows.push(glows);
            t.spans.push((at, len, step.title.clone()));
            at += len;
        }
        t.total = at;
        t.home = t.moves.keys().filter_map(|id| start.get(id).map(|p| (id.clone(), *p))).collect();
        t
    }

    /// Whether any step moves shapes (the scene then changes over time).
    pub fn moves_shapes(&self) -> bool {
        !self.moves.is_empty()
    }

    /// The scene with shapes where `state` has moved them; `None` when
    /// nothing has moved and the still scene serves.
    pub fn scene_for(&self, state: &AnimState) -> Option<Scene> {
        (!state.moved.is_empty()).then(|| crate::build(&self.diagram, &state.moved))
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// The step playing at `secs`; the last one once it is over.
    pub fn step_at(&self, secs: f64) -> Option<usize> {
        if self.spans.is_empty() {
            return None;
        }
        Some(self.spans.iter().rposition(|(start, ..)| *start <= secs).unwrap_or(0))
    }

    /// When step `i` starts.
    pub fn start_of(&self, i: usize) -> f64 {
        self.spans.get(i).map_or(self.total, |s| s.0)
    }

    /// How many steps there are.
    pub fn steps(&self) -> usize {
        self.spans.len()
    }

    /// Step `i`'s start and length, in seconds.
    pub fn span(&self, i: usize) -> Option<(f64, f64)> {
        self.spans.get(i).map(|s| (s.0, s.1))
    }

    fn own_alpha(&self, id: &str, secs: f64) -> f32 {
        let mut a = if self.hidden_at_start.contains(id) { 0.0 } else { 1.0 };
        let Some(changes) = self.fades.get(id) else { return a as f32 };
        for &(at, show, curve) in changes {
            if at > secs {
                break;
            }
            let target = if show { 1.0 } else { 0.0 };
            // Bounce overshoots positions, not opacity.
            let k = curve.at((secs - at) / FADE).clamp(0.0, 1.0);
            a += (target - a) * k;
        }
        a as f32
    }

    /// Everything at `secs` seconds in (clamped to the animation).
    pub fn state(&self, secs: f64) -> AnimState {
        let secs = secs.clamp(0.0, self.total);
        let mut s = AnimState::default();
        for id in self.hidden_at_start.iter().chain(self.fades.keys()) {
            let a = self.own_alpha(id, secs);
            if a < 0.999 {
                s.alpha.insert(id.clone(), a);
            }
        }
        // Edges fade with their ends.
        for (id, (from, to)) in &self.ends {
            let a = s.alpha(id).min(s.alpha(from)).min(s.alpha(to));
            if a < 0.999 {
                s.alpha.insert(id.clone(), a);
            }
        }
        for (id, legs) in &self.moves {
            let mut p = self.home[id];
            for &(at, len, from, to, curve) in legs {
                if at > secs {
                    break;
                }
                let k = curve.at((secs - at) / len.max(0.01));
                p = Point::new(from.x + (to.x - from.x) * k, from.y + (to.y - from.y) * k);
            }
            if p != self.home[id] {
                s.moved.insert(id.clone(), p);
            }
        }
        let Some(i) = self.step_at(secs) else { return s };
        let (start, len, title) = &self.spans[i];
        let into = secs - start;
        // Fades in at the step's start and out at its end.
        let envelope = (ease(into / EASE_IN) * ease((len - into) / EASE_IN)) as f32;
        for id in &self.flows[i] {
            s.flow.insert(id.clone(), into * FLOW_SPEED);
        }
        // Branches not taken step back while this one plays.
        for id in &self.dims[i] {
            let a = s.alpha(id) * (1.0 - (1.0 - UNTAKEN) * envelope);
            s.alpha.insert(id.clone(), a);
        }
        for id in &self.glows[i] {
            // A gentle pulse on top of the envelope.
            let pulse = 0.8 + 0.2 * (into * std::f64::consts::TAU / 1.2).cos() as f32;
            s.glow.insert(id.clone(), envelope * pulse);
        }
        if let Some(t) = title {
            s.caption = Some((t.clone(), envelope.max(if secs >= self.total { 1.0 } else { 0.0 })));
        }
        s.step = Some(i);
        if let Some(whole) = self.whole
            && !self.focus.is_empty()
        {
            let mut cam = whole;
            for &(at, r, curve, len) in &self.focus {
                if at > secs {
                    break;
                }
                cam = lerp_rect(cam, r, curve.at((secs - at) / len));
            }
            s.camera = Some(cam);
        }
        s
    }
}

/// Seconds to move the camera from `from` to `to`: [`MOVE`] for a short
/// hop, longer the farther it pans or the more it zooms, so wide jumps
/// still ease in and out instead of whipping across.
fn move_time(from: Option<Rect>, to: Rect) -> f64 {
    let Some(from) = from else { return MOVE };
    let (a, b) = (from.center(), to.center());
    let pan = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt() / to.size.w.max(from.size.w).max(1.0);
    let zoom = (to.size.w.max(1.0) / from.size.w.max(1.0)).ln().abs();
    (MOVE * (1.0 + 0.6 * pan + 0.4 * zoom)).min(MOVE_MAX)
}

/// How visible a path the branch does not take stays.
const UNTAKEN: f32 = 0.3;

/// The paths a step's flow does not take: where it runs along a wire out
/// of one execution output of a node that has several (a branch, a switch,
/// a sequence), the wires out of the others and whatever only they lead
/// to. What the taken path also reaches, or the step itself shows, stays.
fn untaken(d: &Diagram, flows: &[String], active: &[String]) -> Vec<String> {
    let exec_outs = |node: &str| -> Vec<String> {
        d.node(node).map(|n| crate::pins::pins(d, n).into_iter().filter(|p| p.dir == crate::pins::PinDir::Out && p.exec()).map(|p| p.name).collect()).unwrap_or_default()
    };
    // Where edges lead, following their arrows.
    let next = |node: &str| -> Vec<(&str, &str)> {
        d.edges
            .iter()
            .filter_map(|e| match e.arrow {
                graphing_model::Arrow::Back if e.to == node => Some((e.id.as_str(), e.from.as_str())),
                graphing_model::Arrow::Back => None,
                _ if e.from == node => Some((e.id.as_str(), e.to.as_str())),
                _ => None,
            })
            .collect()
    };
    // Nodes reachable from `starts`, and the edges walked with where each
    // leaves from.
    let reach = |starts: Vec<&str>| -> (HashSet<String>, Vec<(String, String)>) {
        let (mut nodes, mut edges): (HashSet<String>, Vec<(String, String)>) = Default::default();
        let mut stack: Vec<&str> = starts;
        while let Some(n) = stack.pop() {
            if !nodes.insert(n.to_string()) {
                continue;
            }
            for (e, to) in next(n) {
                edges.push((e.to_string(), n.to_string()));
                stack.push(to);
            }
        }
        (nodes, edges)
    };
    let mut out: HashSet<String> = HashSet::new();
    for id in flows {
        let Some(e) = d.edge(id) else { continue };
        let (Some(port), graphing_model::Arrow::Forward) = (&e.from_port, e.arrow) else { continue };
        let outs = exec_outs(&e.from);
        if outs.len() < 2 || !outs.contains(port) {
            continue;
        }
        let others: Vec<&graphing_model::Edge> = d
            .edges
            .iter()
            .filter(|o| o.from == e.from && o.from_port.as_ref().is_some_and(|p| p != port && outs.contains(p)) && !flows.contains(&o.id))
            .collect();
        let (taken, _) = reach(vec![e.to.as_str()]);
        let (lost_nodes, lost_edges) = reach(others.iter().map(|o| o.to.as_str()).collect());
        let lost: HashSet<String> = lost_nodes.into_iter().filter(|n| !taken.contains(n) && !active.contains(n)).collect();
        out.extend(others.iter().map(|o| o.id.clone()));
        // Wires leaving what is lost go with it; ones from the taken side stay.
        out.extend(lost_edges.into_iter().filter(|(x, from)| lost.contains(from) && !flows.contains(x)).map(|(x, _)| x));
        out.extend(lost);
    }
    let mut out: Vec<String> = out.into_iter().collect();
    out.sort();
    out
}

/// Whether `outer` holds all of `inner`.
fn contains(outer: Rect, inner: Rect) -> bool {
    inner.origin.x >= outer.origin.x - 0.5
        && inner.origin.y >= outer.origin.y - 0.5
        && inner.origin.x + inner.size.w <= outer.origin.x + outer.size.w + 0.5
        && inner.origin.y + inner.size.h <= outer.origin.y + outer.size.h + 0.5
}

/// A node's box with any label under it, or a group's box.
fn footprint(scene: &Scene, id: &str) -> Option<Rect> {
    scene
        .nodes
        .iter()
        .find(|n| n.id == id)
        .map(crate::NodeBox::footprint)
        .or_else(|| scene.rect_of(id))
        .or_else(|| Rect::around(scene.edges.iter().find(|e| e.id == id)?.points.iter().copied()))
}

/// Points along a polyline every `gap` units, shifted by `offset`: where
/// the flow dots of an edge are.
/// How far flow dots keep from a line's ends, so none sits on a pin or an
/// arrowhead.
pub const FLOW_CLEAR: f64 = 10.0;

/// `points` with `by` cut off each end (the whole line when it is shorter
/// than that).
pub fn trim(points: &[graphing_model::Point], by: f64) -> Vec<graphing_model::Point> {
    let len: f64 = points.windows(2).map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt()).sum();
    if len <= by * 2.0 || points.len() < 2 {
        return points.to_vec();
    }
    let at = |dist: f64| {
        let mut walked = 0.0;
        for (i, w) in points.windows(2).enumerate() {
            let l = ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt();
            if walked + l >= dist {
                let t = if l == 0.0 { 0.0 } else { (dist - walked) / l };
                return (i, graphing_model::Point::new(w[0].x + (w[1].x - w[0].x) * t, w[0].y + (w[1].y - w[0].y) * t));
            }
            walked += l;
        }
        (points.len() - 2, points[points.len() - 1])
    };
    let ((i, a), (j, b)) = (at(by), at(len - by));
    let mut out = vec![a];
    out.extend_from_slice(&points[i + 1..=j]);
    out.push(b);
    out
}

pub fn flow_dots(points: &[graphing_model::Point], offset: f64, gap: f64) -> Vec<graphing_model::Point> {
    let mut out = Vec::new();
    let mut next = offset.rem_euclid(gap);
    let mut walked = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
        while next <= walked + len {
            let t = if len == 0.0 { 0.0 } else { (next - walked) / len };
            out.push(graphing_model::Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
            next += gap;
        }
        walked += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a -> b -> c in a row, with b and c in group g.
    fn base() -> Diagram {
        let mut d = Diagram::default();
        for (id, x) in [("a", 0.0), ("b", 300.0), ("c", 600.0)] {
            d.nodes.push(graphing_model::Node::new(id));
            d.layout.insert(id.into(), graphing_model::Placement { pos: graphing_model::Point::new(x, 0.0), size: None });
        }
        for (f, t) in [("a", "b"), ("b", "c")] {
            d.edges.push(graphing_model::Edge { id: format!("{f}->{t}"), from: f.into(), to: t.into(), ..Default::default() });
        }
        d.groups.push(graphing_model::Group { id: "g".into(), label: None, members: vec!["b".into(), "c".into()], props: Vec::new() });
        d
    }

    fn with_steps(steps: Vec<graphing_model::Step>) -> (Timeline, Scene) {
        let mut d = base();
        d.steps = steps;
        let scene = crate::build(&d, &HashMap::new());
        (Timeline::new(&d, &scene), scene)
    }

    fn step(secs: Option<f64>, actions: &[(Verb, &[&str])]) -> graphing_model::Step {
        graphing_model::Step {
            title: Some("t".into()),
            seconds: secs,
            ease: None,
            moves: Vec::new(),
            actions: actions.iter().map(|(v, t)| graphing_model::Action { verb: *v, targets: t.iter().map(|s| s.to_string()).collect() }).collect(),
        }
    }

    #[test]
    fn shown_things_wait_for_their_step() {
        let (t, _) = with_steps(vec![step(None, &[(Verb::Show, &["a"])]), step(Some(1.0), &[(Verb::Show, &["g"]), (Verb::Flow, &["a->b"])])]);
        assert_eq!(t.total, 3.0);
        let s = t.state(0.0);
        assert_eq!((s.alpha("a"), s.alpha("b"), s.alpha("c")), (0.0, 0.0, 0.0));
        // Edges wait for both ends.
        assert_eq!(s.alpha("a->b"), 0.0);
        let s = t.state(1.0);
        assert_eq!(s.alpha("a"), 1.0);
        assert_eq!(s.alpha("b"), 0.0, "the group's members come with it");
        let s = t.state(2.9);
        assert_eq!((s.alpha("b"), s.alpha("c"), s.alpha("a->b")), (1.0, 1.0, 1.0));
        assert!(s.flow.contains_key("a->b") && !t.state(1.5).flow.contains_key("a->b"));
        assert_eq!(t.step_at(2.5), Some(1));
    }

    #[test]
    fn focus_moves_the_camera() {
        let (t, scene) = with_steps(vec![step(None, &[(Verb::Focus, &["a"])]), step(None, &[(Verb::Focus, &["all"])])]);
        let whole = scene.content_bounds().unwrap().inflate(FOCUS_PAD);
        assert_eq!(t.state(0.0).camera, Some(whole));
        let on_a = t.state(1.9).camera.unwrap();
        assert!(on_a.size.w < whole.size.w / 2.0, "{on_a:?}");
        assert_eq!(t.state(4.0).camera, Some(whole));
    }

    #[test]
    fn highlights_glow_only_in_their_step() {
        let (t, _) = with_steps(vec![step(None, &[(Verb::Highlight, &["b"])]), step(None, &[])]);
        assert!(t.state(1.0).glow["b"] > 0.5);
        assert!(!t.state(3.0).glow.contains_key("b"));
    }

    #[test]
    fn dots_spread_along_a_line() {
        let pts = [graphing_model::Point::new(0.0, 0.0), graphing_model::Point::new(100.0, 0.0)];
        let dots = flow_dots(&pts, 10.0, 40.0);
        assert_eq!(dots.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 50.0, 90.0]);
    }

    #[test]
    fn moves_slide_shapes_and_their_lines() {
        let mut slide = step(Some(2.0), &[(Verb::Focus, &["a"])]);
        slide.moves = vec![("a".into(), graphing_model::Point::new(0.0, 300.0))];
        let (t, scene) = with_steps(vec![slide]);
        assert!(t.moves_shapes());
        assert!(t.state(0.0).moved.is_empty() && t.scene_for(&t.state(0.0)).is_none());
        let mid = t.state(0.5).moved["a"];
        assert!(mid.y > 0.0 && mid.y < 300.0, "{mid:?}");
        let end = t.state(2.0);
        assert_eq!(end.moved["a"], graphing_model::Point::new(0.0, 300.0));
        // The line to b leaves from a's new place.
        let moved = t.scene_for(&end).unwrap();
        let line = moved.edges.iter().find(|e| e.id == "a->b").unwrap();
        let before = scene.edges.iter().find(|e| e.id == "a->b").unwrap();
        assert!(line.points[0].y > before.points[0].y + 200.0);
        // Focus in the same step frames where a lands.
        let cam = end.camera.unwrap();
        assert!(cam.origin.y > 150.0, "{cam:?}");
    }

    #[test]
    fn a_group_moves_with_everything_in_it() {
        let mut slide = step(Some(1.0), &[]);
        slide.moves = vec![("g".into(), graphing_model::Point::new(0.0, 0.0))];
        let (t, scene) = with_steps(vec![slide]);
        let g = scene.groups.iter().find(|g| g.id == "g").unwrap().rect;
        let end = t.state(1.0);
        let (b, c) = (end.moved["b"], end.moved["c"]);
        assert!((b.x - (300.0 - g.origin.x)).abs() < 1e-6 && (c.x - (600.0 - g.origin.x)).abs() < 1e-6, "{b:?} {c:?}");
        assert!(!end.moved.contains_key("a"));
    }

    #[test]
    fn bounce_overshoots_then_settles() {
        let mut slide = step(Some(2.0), &[]);
        slide.ease = Some(graphing_model::Ease::Bounce);
        slide.moves = vec![("a".into(), graphing_model::Point::new(100.0, 0.0))];
        let (t, _) = with_steps(vec![slide]);
        let peak = (1..140).map(|i| t.state(i as f64 / 100.0).moved.get("a").map_or(0.0, |p| p.x)).fold(0.0, f64::max);
        assert!(peak > 100.0, "{peak}");
        assert_eq!(t.state(2.0).moved["a"].x, 100.0);
    }

    #[test]
    fn a_step_that_brings_something_in_keeps_it_in_view() {
        // Step two frames `a`; step three shows `c`, far off, without a focus.
        let doc = graphing_dsl::Document::parse("a\nb\nc\nlayout {\n  a 0 0\n  b 200 0\n  c 1400 900\n}\nanimate {\n  step { show a\n focus a }\n  step { show b }\n  step { show c\n highlight c }\n}\n");
        let (d, scene) = (doc.diagram().clone(), crate::build(doc.diagram(), &Default::default()));
        let t = Timeline::new(&d, &scene);
        let c = scene.nodes.iter().find(|n| n.id == "c").unwrap().rect;
        let cam = t.state(t.total).camera.expect("framed");
        assert!(contains(cam, c), "{cam:?} misses {c:?}");
        // The camera framed `a`, then widened for `b`, then for `c`.
        assert_eq!(t.focus.len(), 3, "{:?}", t.focus);
        // A step that says where to look is left alone.
        let doc = graphing_dsl::Document::parse("a\nc\nlayout {\n  a 0 0\n  c 1400 900\n}\nanimate {\n  step { focus a }\n  step { show c\n focus a }\n}\n");
        let t = Timeline::new(doc.diagram(), &crate::build(doc.diagram(), &Default::default()));
        assert_eq!(t.focus.len(), 2);
    }

    #[test]
    fn far_camera_moves_take_longer() {
        let here = Rect::new(0.0, 0.0, 400.0, 300.0);
        assert_eq!(move_time(Some(here), here), MOVE);
        let near = move_time(Some(here), Rect::new(100.0, 0.0, 400.0, 300.0));
        let far = move_time(Some(here), Rect::new(3000.0, 2000.0, 400.0, 300.0));
        let zoom = move_time(Some(here), Rect::new(0.0, 0.0, 1600.0, 1200.0));
        assert!(MOVE < near && near < far && far <= MOVE_MAX, "{near} {far}");
        assert!(zoom > MOVE);
    }

    #[test]
    fn the_branch_not_taken_steps_back() {
        // check: true -> a -> join, false -> b -> join, false also -> c.
        let src = "use graph\ncheck: graph.branch\na: graph.function\nb: graph.function\nc: graph.function\njoin: graph.function\n\
            check.true -> a.exec\ncheck.false -> b.exec\na.exec -> join.exec\nb.exec -> join.exec\nb.exec -> c.exec\n\
            animate {\n  step \"Take true\" 2s {\n    flow check -> a\n  }\n  step \"All\" 1s {\n    focus all\n  }\n}\n";
        let doc = graphing_dsl::Document::parse(src);
        assert!(doc.diags().is_empty(), "{:?}", doc.diags());
        let t = Timeline::new(doc.diagram(), &crate::build(doc.diagram(), &Default::default()));
        let mid = t.state(1.0);
        // The false wire, `b` and what only `b` leads to fade back...
        for id in ["check->b", "b", "c", "b->c"] {
            assert!(mid.alpha(id) < 0.5, "{id}: {}", mid.alpha(id));
        }
        // ...the taken path and where both meet stay bright.
        for id in ["check->a", "a", "join", "a->join", "check"] {
            assert!(mid.alpha(id) > 0.99, "{id}: {}", mid.alpha(id));
        }
        // The next step brings everything back.
        assert!(t.state(2.7).alpha("b") > 0.99);
    }
}

