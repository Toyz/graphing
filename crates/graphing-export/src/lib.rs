//! Scene -> SVG text and PNG bytes. Mirrors the canvas painter so exports
//! look like the editor.

pub mod sysml2;
mod webm;
pub use sysml2::to_sysml2;

use std::collections::HashMap;
use std::fmt::Write as _;

use graphing_dsl::Document;
use graphing_model::{Op, Placement, Point, Rect};
use graphing_scene::anim::{AnimState, Timeline};
use graphing_scene::stencils::GlyphAt;
use graphing_scene::{EdgeLine, End, FrameBox, GroupBox, NodeBox, PORT, PortBox, Scene, Shape, Side, notation, pins};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("svg: {0}")]
    Svg(String),
    #[error("png: {0}")]
    Png(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Colors as 0xRRGGBB; group fill alpha separately.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub bg: u32,
    pub fill: u32,
    pub stroke: u32,
    pub text: u32,
    pub edge: u32,
    pub group_fill: u32,
    pub group_alpha: f64,
    pub group_stroke: u32,
    /// Secondary text: compartments, port names, stereotypes.
    pub muted: u32,
    /// Item flow stereotypes (`« flow »`).
    pub flow: u32,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            bg: 0xfafafa,
            fill: 0xffffff,
            stroke: 0x495057,
            text: 0x212529,
            edge: 0x495057,
            group_fill: 0x868e96,
            group_alpha: 0.06,
            group_stroke: 0xc4c4c4,
            muted: 0x767676,
            flow: 0xc24e00,
        }
    }

    pub fn dark() -> Self {
        Self {
            bg: 0x16181d,
            fill: 0x22252c,
            stroke: 0x9aa1ad,
            text: 0xe6e8eb,
            edge: 0x9aa1ad,
            group_fill: 0xffffff,
            group_alpha: 0.03,
            group_stroke: 0x3d3d3d,
            muted: 0x8c8c98,
            flow: 0xff6900,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SvgOptions {
    pub theme: Theme,
    pub padding: f64,
    pub background: bool,
    pub font_family: String,
    /// Picture bytes by the `src` nodes use, embedded as data URIs. Missing
    /// ones draw nothing.
    pub images: std::collections::BTreeMap<String, Vec<u8>>,
    /// Lucide icon SVG files by icon name, for shapes that carry an icon.
    /// This crate has no icon set of its own; the app and CLI fill it (see
    /// [`icons_used`]). Missing ones draw nothing.
    pub icons: std::collections::BTreeMap<String, String>,
    /// Diagrams that diagram links point at, by their `src` (see
    /// [`refs_for`]). Missing ones draw as missing.
    pub refs: std::collections::BTreeMap<String, RefDiagram>,
}

/// A linked diagram, loaded: its title and scene.
#[derive(Debug, Clone)]
pub struct RefDiagram {
    pub title: String,
    pub scene: Scene,
}

/// Load the diagram at `path` (`.gph` or `.gphz`) for a link to show.
pub fn load_ref(path: &std::path::Path) -> Option<RefDiagram> {
    let pkg = graphing_package::open(std::fs::read(path).ok()?).ok()?;
    ref_from_text(&pkg.doc, &path.to_string_lossy())
}

/// Every diagram `scene`'s links point at, `src` resolved against `base`
/// (the linking file's folder); a file that is not there falls back to the
/// snapshot a package carries of it (`snapshots`, by `src`).
pub fn refs_for(scene: &Scene, base: Option<&std::path::Path>, snapshots: &std::collections::BTreeMap<String, Vec<u8>>) -> std::collections::BTreeMap<String, RefDiagram> {
    scene
        .nodes
        .iter()
        .filter_map(|n| n.reference.clone())
        .filter(|src| !src.is_empty())
        .filter_map(|src| {
            let path = resolve_src(&src, base);
            let found = load_ref(&path).or_else(|| ref_from_text(&String::from_utf8_lossy(snapshots.get(&src)?), &src));
            found.map(|r| (src, r))
        })
        .collect()
}

/// The `src` to write for the file at `path`: relative to `base`, the
/// diagram file's folder, when it is inside it, and with forward slashes so
/// the text reads the same on every platform.
pub fn src_for(path: &std::path::Path, base: Option<&std::path::Path>) -> String {
    // The same folder can be spelled two ways (macOS's `/var` is
    // `/private/var`), so compare resolved paths when the spellings differ.
    let relative = base.and_then(|b| {
        path.strip_prefix(b).ok().map(std::path::Path::to_path_buf).or_else(|| {
            let (p, b) = (path.canonicalize().ok()?, b.canonicalize().ok()?);
            p.strip_prefix(b).ok().map(std::path::Path::to_path_buf)
        })
    });
    relative.as_deref().unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// A picture's or link's `src` against the diagram file's folder.
pub fn resolve_src(src: &str, base: Option<&std::path::Path>) -> std::path::PathBuf {
    let p = std::path::Path::new(src);
    match base {
        Some(b) if p.is_relative() => b.join(p),
        _ => p.to_path_buf(),
    }
}

/// A linked diagram from its `.gph` text (a package's snapshot of it).
pub fn ref_from_text(text: &str, src: &str) -> Option<RefDiagram> {
    let doc = Document::parse(text);
    let title = doc.diagram().title.clone().unwrap_or_else(|| std::path::Path::new(src).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    Some(RefDiagram { title, scene: scene_of(text) })
}

/// The `src` of every diagram link in `.gph` text.
pub fn link_sources(text: &str) -> Vec<String> {
    sources(Document::parse(text).diagram(), true)
}

/// The `src` of every picture in `d` (a link's `src` names a diagram).
pub fn picture_sources(d: &graphing_model::Diagram) -> Vec<String> {
    sources(d, false)
}

/// A picture's bytes: packaged (`asset:<name>` in `assets`), else the file
/// `src` names against `base`, the diagram file's folder.
pub fn picture(src: &str, assets: &std::collections::BTreeMap<String, Vec<u8>>, base: Option<&std::path::Path>) -> Option<Vec<u8>> {
    match src.strip_prefix(graphing_package::ASSET_PREFIX) {
        Some(name) => assets.get(name).cloned(),
        None => std::fs::read(resolve_src(src, base)).ok(),
    }
}

/// Bytes of every picture `d` shows, by `src`; see [`picture`].
pub fn pictures(d: &graphing_model::Diagram, assets: &std::collections::BTreeMap<String, Vec<u8>>, base: Option<&std::path::Path>) -> std::collections::BTreeMap<String, Vec<u8>> {
    picture_sources(d).into_iter().filter_map(|src| Some((src.clone(), picture(&src, assets, base)?))).collect()
}

fn sources(d: &graphing_model::Diagram, links: bool) -> Vec<String> {
    d.nodes
        .iter()
        .filter(|n| notation::is_link(n) == links)
        .filter_map(|n| d.node_prop(n, "src").map(graphing_model::Value::text))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Put a snapshot of each diagram `text` links to into `assets` (named
/// `linked/<src>`), read fresh from disk; a link whose file is not there
/// keeps the snapshot it had.
pub fn snapshot_links(text: &str, base: Option<&std::path::Path>, assets: &mut std::collections::BTreeMap<String, Vec<u8>>) {
    for src in link_sources(text) {
        let Some(pkg) = std::fs::read(resolve_src(&src, base)).ok().and_then(|b| graphing_package::open(b).ok()) else { continue };
        assets.insert(graphing_package::linked_name(&src), pkg.doc.into_bytes());
    }
}

/// The snapshots in a package's assets, by the `src` they stand for.
pub fn snapshots(assets: &std::collections::BTreeMap<String, Vec<u8>>) -> std::collections::BTreeMap<String, Vec<u8>> {
    assets.iter().filter_map(|(k, v)| k.strip_prefix(graphing_package::LINKED_PREFIX).map(|s| (s.to_string(), v.clone()))).collect()
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self { theme: Theme::light(), padding: 24.0, background: true, font_family: "Inter, system-ui, sans-serif".into(), images: Default::default(), icons: Default::default(), refs: Default::default() }
    }
}

fn n(v: f64) -> String {
    graphing_dsl::fmt_num(v)
}

fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Same rule as the canvas: dark text on light fills.
fn contrast(fill: u32) -> u32 {
    let ch = |shift: u32| ((fill >> shift) & 0xff) as f64 / 255.0;
    let luma = 0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0);
    if luma > 0.55 { 0x212529 } else { 0xf1f3f5 }
}

const MONO: &str = "ui-monospace, 'IBM Plex Mono', 'JetBrains Mono', monospace";

/// The icons `scene`'s shapes draw, by name, for [`SvgOptions::icons`].
pub fn icons_used(scene: &Scene) -> std::collections::BTreeSet<String> {
    scene.nodes.iter().filter_map(|n| n.glyph.as_ref().map(|g| g.icon.clone())).collect()
}

pub fn to_svg(scene: &Scene, opts: &SvgOptions) -> String {
    render(scene, opts, &Motion::Still, None)
}

/// The diagram at one moment of its animation (a GIF or APNG frame).
/// `viewport` keeps every frame the same size while shapes move.
pub fn to_svg_at(scene: &Scene, opts: &SvgOptions, state: &AnimState, viewport: Option<Rect>) -> String {
    render(scene, opts, &Motion::At(state), viewport)
}

/// Everything the animation ever shows, moved shapes included, sampled
/// every quarter second.
pub fn anim_viewport(scene: &Scene, timeline: &Timeline) -> Option<Rect> {
    let base = scene.bounds()?;
    if !timeline.moves_shapes() {
        return Some(base);
    }
    let n = (timeline.total * 4.0).ceil() as usize;
    Some((0..=n).filter_map(|i| timeline.scene_for(&timeline.state(i as f64 / 4.0)).and_then(|s| s.content_bounds())).fold(base, |a, b| a.union(b)))
}

/// The whole animation as one looping SVG, played by the browser (SMIL).
pub fn to_svg_animated(scene: &Scene, opts: &SvgOptions, timeline: &Timeline) -> String {
    if timeline.is_empty() {
        return to_svg(scene, opts);
    }
    let n = (timeline.total * SVG_FPS).ceil().max(1.0) as usize;
    let samples: Vec<(f64, AnimState)> = (0..=n).map(|i| {
        let t = timeline.total * i as f64 / n as f64;
        (t, timeline.state(t))
    }).collect();
    // Shapes that move need the scene at each sample for their lines.
    let scenes = if timeline.moves_shapes() { samples.iter().map(|(_, st)| timeline.scene_for(st)).collect() } else { Vec::new() };
    let viewport = anim_viewport(scene, timeline);
    render(scene, opts, &Motion::Loop { total: timeline.total, samples, scenes }, viewport)
}

/// Samples per second for animated SVG keyframes.
const SVG_FPS: f64 = 12.0;
/// Glow and flow dots: the app's violet accent.
const ACCENT: u32 = 0x7950f2;

/// How the diagram moves, for [`render`].
enum Motion<'a> {
    Still,
    At(&'a AnimState),
    Loop {
        total: f64,
        samples: Vec<(f64, AnimState)>,
        /// The scene at each sample while shapes move; empty otherwise.
        scenes: Vec<Option<Scene>>,
    },
}

impl Motion<'_> {
    /// SMIL keyframes for a value over the loop: ` <animate .../>`, or
    /// `None` when it never changes. `first` is the value to write as the
    /// attribute itself.
    fn keyframes(&self, attr: &str, f: impl Fn(&AnimState) -> String) -> Option<(String, String)> {
        let (first, values, times, total) = self.series(f)?;
        Some((first, format!(r#"<animate attributeName="{attr}" values="{values}" keyTimes="{times}" dur="{}s" repeatCount="indefinite"/>"#, n(total))))
    }

    /// A sliding node: `<animateTransform>` keyframes of its offset.
    fn slide(&self, f: impl Fn(&AnimState) -> String) -> Option<String> {
        let (_, values, times, total) = self.series(f)?;
        Some(format!(r#"<animateTransform attributeName="transform" type="translate" values="{values}" keyTimes="{times}" dur="{}s" repeatCount="indefinite"/>"#, n(total)))
    }

    /// A value over the loop as (first, values, keyTimes, seconds), or
    /// `None` when it never changes.
    fn series(&self, f: impl Fn(&AnimState) -> String) -> Option<(String, String, String, f64)> {
        let Motion::Loop { total, samples, .. } = self else { return None };
        let vals: Vec<String> = samples.iter().map(|(_, s)| f(s)).collect();
        if vals.iter().all(|v| v == &vals[0]) {
            return None;
        }
        // Keep the ends of every plateau; linear between them.
        let last = vals.len() - 1;
        let keep: Vec<usize> = (0..vals.len()).filter(|&i| i == 0 || i == last || vals[i] != vals[i - 1] || vals[i] != vals[i + 1]).collect();
        let values = keep.iter().map(|&i| vals[i].as_str()).collect::<Vec<_>>().join(";");
        let times = keep.iter().map(|&i| format!("{:.4}", samples[i].0 / total)).collect::<Vec<_>>().join(";");
        Some((vals[0].clone(), values, times, *total))
    }

    /// Runs of samples over which an element keeps one shape, by its
    /// `look` in each sample's scene; `None` when it never changes.
    fn runs(&self, look: impl Fn(&Scene) -> Option<String>, base: &Scene) -> Option<Vec<(usize, usize)>> {
        let Motion::Loop { scenes, .. } = self else { return None };
        if scenes.is_empty() {
            return None;
        }
        let looks: Vec<Option<String>> = scenes.iter().map(|sc| look(sc.as_ref().unwrap_or(base))).collect();
        if looks.iter().all(|l| l == &looks[0]) {
            return None;
        }
        let mut runs = Vec::new();
        let mut start = 0;
        for i in 1..=looks.len() {
            if i == looks.len() || looks[i] != looks[start] {
                runs.push((start, i));
                start = i;
            }
        }
        Some(runs)
    }

    /// `<g>` shown only from sample `a` up to sample `b` (exclusive).
    fn sprite_open(&self, s: &mut String, (a, b): (usize, usize)) {
        let Motion::Loop { total, samples, .. } = self else { return };
        let at = |i: usize| if i >= samples.len() { 1.0 } else { samples[i].0 / total };
        let mut keys: Vec<(f64, u8)> = vec![(0.0, u8::from(a == 0))];
        if a > 0 {
            keys.push((at(a), 1));
        }
        if b < samples.len() {
            keys.push((at(b), 0));
        }
        let values = keys.iter().map(|k| k.1.to_string()).collect::<Vec<_>>().join(";");
        let times = keys.iter().map(|k| format!("{:.4}", k.0)).collect::<Vec<_>>().join(";");
        let _ = write!(
            s,
            r#"<g opacity="{}"><animate attributeName="opacity" values="{values}" keyTimes="{times}" calcMode="discrete" dur="{}s" repeatCount="indefinite"/>"#,
            keys[0].1,
            n(*total)
        );
    }

    /// The scene at sample `i`, or `base` when shapes did not move.
    fn scene_at<'s>(&'s self, i: usize, base: &'s Scene) -> &'s Scene {
        match self {
            Motion::Loop { scenes, .. } => scenes.get(i).and_then(Option::as_ref).unwrap_or(base),
            _ => base,
        }
    }

    /// Wrap one element in its opacity; skip it while invisible.
    fn wrap(&self, s: &mut String, id: &str, draw: impl FnOnce(&mut String)) {
        match self {
            Motion::Still => draw(s),
            Motion::At(st) => {
                let a = st.alpha(id);
                if a <= 0.005 {
                    return;
                }
                if a < 0.999 {
                    let _ = writeln!(s, r#"<g opacity="{a:.3}">"#);
                    draw(s);
                    s.push_str("</g>\n");
                } else {
                    draw(s);
                }
            }
            Motion::Loop { .. } => match self.keyframes("opacity", |st| format!("{:.3}", st.alpha(id))) {
                Some((first, tag)) => {
                    let _ = writeln!(s, r#"<g opacity="{first}">{tag}"#);
                    draw(s);
                    s.push_str("</g>\n");
                }
                None => draw(s),
            },
        }
    }

    /// Something drawn only while `strength` is above zero, faded by it.
    fn layer(&self, s: &mut String, strength: impl Fn(&AnimState) -> f32, draw: impl FnOnce(&mut String)) {
        match self {
            Motion::Still => {}
            Motion::At(st) => {
                let g = strength(st);
                if g > 0.005 {
                    let _ = writeln!(s, r#"<g opacity="{g:.3}">"#);
                    draw(s);
                    s.push_str("</g>\n");
                }
            }
            Motion::Loop { .. } => {
                if let Some((first, tag)) = self.keyframes("opacity", |st| format!("{:.3}", strength(st))) {
                    let _ = writeln!(s, r#"<g opacity="{first}">{tag}"#);
                    draw(s);
                    s.push_str("</g>\n");
                }
            }
        }
    }
}

fn glow_rect(s: &mut String, r: Rect, radius: f64) {
    for (w, o) in [(12.0, 0.18), (6.0, 0.35), (2.5, 0.95)] {
        let _ = writeln!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="none" stroke="{}" stroke-width="{w}" stroke-opacity="{o}"/>"#,
            n(r.origin.x - 6.0),
            n(r.origin.y - 6.0),
            n(r.size.w + 12.0),
            n(r.size.h + 12.0),
            n(radius + 6.0),
            hex(ACCENT)
        );
    }
}

fn glow_path(s: &mut String, d: &str) {
    for (w, o) in [(22.0, 0.16), (12.0, 0.3), (5.0, 0.9)] {
        let _ = writeln!(s, r#"<path d="{d}" fill="none" stroke="{}" stroke-width="{w}" stroke-opacity="{o}" stroke-linejoin="round"/>"#, hex(ACCENT));
    }
}

fn polyline_d(points: &[Point]) -> String {
    points.iter().enumerate().map(|(i, p)| format!("{}{} {}", if i == 0 { "M" } else { "L" }, n(p.x), n(p.y))).collect::<Vec<_>>().join(" ")
}

/// The camera rect grown to the picture's aspect, so frames never stretch.
fn fit_aspect(cam: Rect, w: f64, h: f64) -> Rect {
    let want = w / h;
    let have = cam.size.w / cam.size.h.max(f64::EPSILON);
    let (cw, ch) = if have > want { (cam.size.w, cam.size.w / want) } else { (cam.size.h * want, cam.size.h) };
    let c = cam.center();
    Rect::new(c.x - cw / 2.0, c.y - ch / 2.0, cw, ch)
}

fn view_box(r: Rect) -> String {
    format!("{} {} {} {}", n(r.origin.x), n(r.origin.y), n(r.size.w), n(r.size.h))
}

fn render(scene: &Scene, opts: &SvgOptions, motion: &Motion, viewport: Option<Rect>) -> String {
    let t = &opts.theme;
    let b = viewport.or_else(|| scene.bounds()).unwrap_or(Rect::new(0.0, 0.0, 100.0, 100.0));
    // Port names and flow labels hang a little outside boxes.
    let p = if scene.frame.is_some() { opts.padding.min(8.0) } else { opts.padding };
    let whole = b.inflate(p);
    let (w, h) = (whole.size.w, whole.size.h);
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" width="{}" height="{}" font-family="{}">"#,
        n(w),
        n(h),
        n(w),
        n(h),
        esc(&opts.font_family)
    );
    if opts.background {
        let _ = writeln!(s, r#"<rect width="{}" height="{}" fill="{}"/>"#, n(w), n(h), hex(t.bg));
    }
    // The diagram in its own viewport, so the camera can move over it.
    let camera = |st: &AnimState| view_box(st.camera.map_or(whole, |c| fit_aspect(c, w, h)));
    let (vb, cam_anim) = match motion {
        Motion::Still => (view_box(whole), String::new()),
        Motion::At(st) => (camera(st), String::new()),
        Motion::Loop { .. } => match motion.keyframes("viewBox", camera) {
            Some((first, tag)) => (first, tag),
            None => (view_box(whole), String::new()),
        },
    };
    let _ = writeln!(s, r#"<svg x="0" y="0" width="{}" height="{}" viewBox="{vb}">{cam_anim}"#, n(w), n(h));
    let k = Kit { t, technical: scene.technical, images: &opts.images, icons: &opts.icons, refs: &opts.refs };
    if let Some(f) = &scene.frame {
        k.frame(&mut s, f);
    }
    let draw_group = |s: &mut String, g: &GroupBox| {
        let glow = |st: &AnimState| st.glow.get(&g.id).copied().unwrap_or(0.0);
        motion.layer(s, glow, |s| glow_rect(s, g.rect, 6.0));
        motion.wrap(s, &g.id, |s| k.group(s, g));
    };
    for g in &scene.groups {
        // A frame whose members move changes shape: one copy per run.
        let look = |sc: &Scene| sc.groups.iter().find(|x| x.id == g.id).map(|x| format!("{:?}", x.rect));
        match motion.runs(look, scene) {
            None => draw_group(&mut s, g),
            Some(runs) => {
                for run in runs {
                    let Some(gg) = motion.scene_at(run.0, scene).groups.iter().find(|x| x.id == g.id) else { continue };
                    motion.sprite_open(&mut s, run);
                    draw_group(&mut s, gg);
                    s.push_str("</g>\n");
                }
            }
        }
    }
    let draw_edge = |s: &mut String, e: &EdgeLine| {
        let glow = |st: &AnimState| st.glow.get(&e.id).copied().unwrap_or(0.0);
        let d = polyline_d(&e.points);
        motion.layer(s, glow, |s| {
            let _ = writeln!(s, r#"<path d="{d}" fill="none" stroke="{}" stroke-width="9" stroke-opacity="0.35" stroke-linecap="round"/>"#, hex(ACCENT));
        });
        // Flow: round dots (zero-length dashes) running along the line.
        let gap = graphing_scene::anim::FLOW_GAP;
        let flowing = |st: &AnimState| if st.flow.contains_key(&e.id) { st.alpha(&e.id) } else { 0.0 };
        match motion {
            Motion::At(st) => {
                if let Some(off) = st.flow.get(&e.id) {
                    motion.layer(s, flowing, |s| {
                        let _ = writeln!(
                            s,
                            r#"<path d="{d}" fill="none" stroke="{}" stroke-width="7" stroke-linecap="round" stroke-dasharray="0 {}" stroke-dashoffset="{}"/>"#,
                            hex(ACCENT),
                            n(gap),
                            n(-(off % gap))
                        );
                    });
                }
            }
            Motion::Loop { .. } => motion.layer(s, flowing, |s| {
                let _ = writeln!(
                    s,
                    r#"<path d="{d}" fill="none" stroke="{}" stroke-width="7" stroke-linecap="round" stroke-dasharray="0 {}"><animate attributeName="stroke-dashoffset" values="0;{}" dur="{}s" repeatCount="indefinite"/></path>"#,
                    hex(ACCENT),
                    n(gap),
                    n(-gap),
                    n(gap / graphing_scene::anim::FLOW_SPEED)
                );
            }),
            Motion::Still => {}
        }
        // The line and its label over the dots.
        motion.wrap(s, &e.id, |s| k.edge(s, e));
    };
    for e in &scene.edges {
        // A line whose ends move re-routes: one copy per run of samples.
        let look = |sc: &Scene| sc.edges.iter().find(|x| x.id == e.id).map(|x| format!("{:?}", x.points));
        match motion.runs(look, scene) {
            None => draw_edge(&mut s, e),
            Some(runs) => {
                for run in runs {
                    let Some(ee) = motion.scene_at(run.0, scene).edges.iter().find(|x| x.id == e.id) else { continue };
                    motion.sprite_open(&mut s, run);
                    draw_edge(&mut s, ee);
                    s.push_str("</g>\n");
                }
            }
        }
    }
    for nb in &scene.nodes {
        // Sliding shapes travel as a whole.
        let home = nb.rect.origin;
        let slide = motion.slide(|st| st.moved.get(&nb.id).map_or("0 0".into(), |p| format!("{} {}", n(p.x - home.x), n(p.y - home.y))));
        if let Some(tag) = &slide {
            let _ = writeln!(s, "<g>{tag}");
        }
        let glow = |st: &AnimState| st.glow.get(&nb.id).copied().unwrap_or(0.0);
        // Along the shape's own outline; the node then covers the inner half.
        let d = match &nb.path {
            Some(cmds) => path_data(cmds),
            None => outline(nb.shape, nb.rect, scene.technical),
        };
        motion.layer(&mut s, glow, |s| glow_path(s, &d));
        motion.wrap(&mut s, &nb.id, |s| {
            k.node(s, nb);
            k.picture(s, nb);
        });
        if slide.is_some() {
            s.push_str("</g>\n");
        }
    }
    s.push_str("</svg>\n");
    // Step titles over the bottom of the picture, outside the camera.
    match motion {
        Motion::At(st) => {
            if let Some((text, a)) = &st.caption {
                caption(&mut s, text, w, h, &format!(r#" opacity="{a:.3}""#));
            }
        }
        Motion::Loop { samples, .. } => {
            let mut titles: Vec<(usize, String)> = Vec::new();
            for (_, st) in samples {
                if let (Some(i), Some((text, _))) = (st.step, &st.caption)
                    && !titles.iter().any(|(j, _)| *j == i)
                {
                    titles.push((i, text.clone()));
                }
            }
            for (i, text) in titles {
                let fade = |st: &AnimState| match (&st.caption, st.step) {
                    (Some((_, a)), Some(j)) if j == i => *a,
                    _ => 0.0,
                };
                if let Some((first, tag)) = motion.keyframes("opacity", |st| format!("{:.3}", fade(st))) {
                    let mut g = String::new();
                    caption(&mut g, &text, w, h, "");
                    let _ = writeln!(s, r#"<g opacity="{first}">{tag}{g}</g>"#);
                }
            }
        }
        Motion::Still => {}
    }
    s.push_str("</svg>\n");
    s
}

/// A step title in a pill at the bottom centre, sized to the picture.
fn caption(s: &mut String, text: &str, w: f64, h: f64, attrs: &str) {
    let size = (h * 0.032).clamp(13.0, 30.0);
    let tw = text.chars().count() as f64 * size * 0.56 + size * 2.0;
    let (cx, cy) = (w / 2.0, h - size * 2.2);
    let _ = write!(
        s,
        r##"<g{attrs}><rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="#1b1d23" fill-opacity="0.88"/><text x="{}" y="{}" font-size="{}" font-weight="600" fill="#ffffff" text-anchor="middle" dominant-baseline="central">{}</text></g>"##,
        n(cx - tw / 2.0),
        n(cy - size * 0.95),
        n(tw),
        n(size * 1.9),
        n(size * 0.95),
        n(cx),
        n(cy),
        n(size),
        esc(text)
    );
    s.push('\n');
}

/// Drawing helpers bound to one theme and look.
struct Kit<'a> {
    t: &'a Theme,
    technical: bool,
    images: &'a std::collections::BTreeMap<String, Vec<u8>>,
    icons: &'a std::collections::BTreeMap<String, String>,
    refs: &'a std::collections::BTreeMap<String, RefDiagram>,
}

impl Kit<'_> {
    /// A node's picture as an embedded image, fitted like the canvas does.
    fn picture(&self, s: &mut String, nb: &NodeBox) {
        use base64::Engine as _;
        let Some(src) = &nb.image else { return };
        let Some(bytes) = self.images.get(src) else { return };
        let mime = match bytes {
            b if b.starts_with(b"\x89PNG") => "image/png",
            b if b.starts_with(b"\xff\xd8\xff") => "image/jpeg",
            b if b.starts_with(b"GIF8") => "image/gif",
            b if b.len() > 12 && &b[8..12] == b"WEBP" => "image/webp",
            b if b.len() > 12 && &b[4..8] == b"ftyp" && b[8..12].starts_with(b"avi") => "image/avif",
            b if b.windows(4).take(512).any(|w| w == b"<svg") => "image/svg+xml",
            _ => return,
        };
        let aspect = match nb.fit {
            graphing_scene::ImageFit::Contain => "xMidYMid meet",
            graphing_scene::ImageFit::Cover => "xMidYMid slice",
            graphing_scene::ImageFit::Fill => "none",
        };
        let r = nb.rect;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        // A nested viewport clips a covering picture to the node.
        let _ = writeln!(
            s,
            r#"<svg x="{}" y="{}" width="{}" height="{}"><image width="100%" height="100%" preserveAspectRatio="{aspect}" href="data:{mime};base64,{data}"/></svg>"#,
            n(r.origin.x + 2.0),
            n(r.origin.y + 2.0),
            n((r.size.w - 4.0).max(0.0)),
            n((r.size.h - 4.0).max(0.0)),
        );
    }

    fn frame(&self, s: &mut String, f: &FrameBox) {
        let r = f.rect;
        let (x, y) = (r.origin.x, r.origin.y);
        let tab_w = f.title.chars().count() as f64 * 6.9 + 28.0;
        let tab_h = 30.0;
        let _ = writeln!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}"/>"#,
            n(x),
            n(y),
            n(r.size.w),
            n(r.size.h),
            hex(self.t.group_stroke)
        );
        let _ = writeln!(
            s,
            r#"<path d="M{} {} H{} L{} {} V{} H{} Z" fill="none" stroke="{}"/>"#,
            n(x),
            n(y),
            n(x + tab_w),
            n(x + tab_w + 12.0),
            n(y + 12.0),
            n(y + tab_h),
            n(x),
            hex(self.t.group_stroke)
        );
        let _ = writeln!(
            s,
            r#"<text x="{}" y="{}" font-family="{MONO}" font-size="11" font-weight="500" letter-spacing="0.4" fill="{}">{}</text>"#,
            n(x + 12.0),
            n(y + 19.5),
            hex(self.t.text),
            esc(&f.title)
        );
    }

    fn group(&self, s: &mut String, g: &GroupBox) {
        use graphing_scene::GroupLook as L;
        let t = self.t;
        let r = g.rect;
        let (x, y, w, h) = (n(r.origin.x), n(r.origin.y), n(r.size.w), n(r.size.h));
        let (fill, alpha) = match g.fill {
            Some(c) => (c, 0.12),
            None => (t.group_fill, t.group_alpha),
        };
        let base = g.stroke.unwrap_or(t.group_stroke);
        let stroke = hex(base);
        let head = g.look.head();
        let label_w = g.label.as_ref().map_or(60.0, |l| l.chars().count() as f64 * 7.2 + 24.0);
        let rect = |s: &mut String, x: &str, y: &str, w: &str, h: &str, rx: u32, fill: &str, opacity: f64, stroke: &str, extra: &str| {
            let _ = writeln!(s, r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}" fill-opacity="{opacity}" stroke="{stroke}"{extra}/>"#);
        };
        match g.look {
            L::Dashed | L::Solid => {
                let rx = if self.technical { 2 } else { 10 };
                let dash = if g.look == L::Dashed { r#" stroke-dasharray="4 3""# } else { "" };
                rect(s, &x, &y, &w, &h, rx, &hex(fill), alpha, &stroke, dash);
            }
            L::Package => {
                rect(s, &x, &y, &w, &h, 0, &hex(fill), alpha, &stroke, "");
                let (tw, th, cut) = (label_w.min(r.size.w - 10.0), 22.0, 8.0);
                let (ox, oy) = (r.origin.x, r.origin.y);
                let _ = writeln!(s, r#"<path d="M{} {} L{} {} V{} H{}" fill="none" stroke="{stroke}"/>"#, n(ox + tw), n(oy), n(ox + tw + cut), n(oy + cut), n(oy + th), n(ox));
            }
            L::Sysml => {
                let body = g.fill.unwrap_or(t.fill);
                rect(s, &x, &y, &w, &h, 0, &hex(body), if g.fill.is_some() { 0.10 } else { 0.5 }, &stroke, "");
                let _ = writeln!(s, r#"<line x1="{x}" y1="{}" x2="{}" y2="{}" stroke="{stroke}"/>"#, n(r.origin.y + head), n(r.origin.x + r.size.w), n(r.origin.y + head));
            }
            L::Lane => {
                rect(s, &x, &y, &w, &h, 4, &hex(fill), alpha, &stroke, "");
                let band = g.fill.unwrap_or(base);
                let _ = writeln!(s, r#"<path d="M{x} {} V{y} H{} V{} Z" fill="{}" fill-opacity="0.22"/>"#, n(r.origin.y + head), n(r.origin.x + r.size.w), n(r.origin.y + head), hex(band));
                let _ = writeln!(s, r#"<line x1="{x}" y1="{}" x2="{}" y2="{}" stroke="{stroke}"/>"#, n(r.origin.y + head), n(r.origin.x + r.size.w), n(r.origin.y + head));
            }
            L::Zone => rect(s, &x, &y, &w, &h, 10, &hex(g.fill.unwrap_or(base)), 0.16, "none", ""),
            L::Card => {
                let body = g.fill.unwrap_or(t.fill);
                rect(s, &x, &y, &w, &h, 12, &hex(body), if g.fill.is_some() { 0.18 } else { 1.0 }, &stroke, "");
                let _ = writeln!(s, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{stroke}" stroke-opacity="0.6"/>"#, n(r.origin.x + 10.0), n(r.origin.y + head), n(r.origin.x + r.size.w - 10.0), n(r.origin.y + head));
            }
        }
        if let (Some(label), L::Sysml) = (&g.label, g.look) {
            let cx = n(r.origin.x + r.size.w / 2.0);
            let mut y = r.origin.y + 12.0;
            if let Some(st) = &g.stereotype {
                let _ = writeln!(s, r#"<text x="{cx}" y="{}" font-size="10" fill="{}" text-anchor="middle" dominant-baseline="central">{}</text>"#, n(y), hex(t.muted), esc(&format!("\u{ab}{st}\u{bb}")));
                y += 14.0;
            } else {
                y = r.origin.y + head / 2.0;
            }
            let _ = writeln!(s, r#"<text x="{cx}" y="{}" font-size="12" font-weight="bold" fill="{}" text-anchor="middle" dominant-baseline="central">{}</text>"#, n(y), hex(g.text.unwrap_or(t.text)), esc(label));
            return;
        }
        if let Some(label) = &g.label {
            let (text, weight, color, opacity) = match g.look {
                L::Dashed | L::Solid => (label.clone(), "normal", g.text.unwrap_or(t.text), if g.text.is_some() { 1.0 } else { 0.7 }),
                L::Zone => (label.to_uppercase(), "bold", g.text.unwrap_or(base), 1.0),
                _ => (label.clone(), "bold", g.text.unwrap_or(t.text), 1.0),
            };
            let strip = if g.look == L::Package { 22.0 } else { head };
            if let Some(details) = &g.details {
                // Field values after the name; width estimated from the label.
                let x = r.origin.x + 12.0 + text.chars().count() as f64 * 7.2 + 10.0;
                let _ = writeln!(
                    s,
                    r#"<text x="{}" y="{}" font-size="11" font-family="monospace" fill="{}" dominant-baseline="central">{}</text>"#,
                    n(x),
                    n(r.origin.y + strip / 2.0),
                    hex(t.muted),
                    esc(details)
                );
            }
            let _ = writeln!(
                s,
                r#"<text x="{}" y="{}" font-size="12" font-weight="{weight}" fill="{}" fill-opacity="{opacity}" dominant-baseline="central">{}</text>"#,
                n(r.origin.x + 12.0),
                n(r.origin.y + strip / 2.0),
                hex(color),
                esc(&text)
            );
        }
    }

    fn node(&self, s: &mut String, nb: &NodeBox) {
        if let Some(w) = &nb.wave {
            return self.wave(s, nb, w);
        }
        if let Some(src) = &nb.reference {
            return self.link(s, nb, src);
        }
        let t = self.t;
        let r = nb.rect;
        let fill = nb.fill.unwrap_or(t.fill);
        let stroke = nb.stroke.unwrap_or(if self.technical { t.edge } else { t.stroke });
        let ink = match (nb.text, nb.fill) {
            (Some(c), _) => c,
            (None, Some(f)) if nb.shape != Shape::Actor => contrast(f),
            _ => t.text,
        };
        match nb.shape {
            Shape::Initial => {
                let c = r.center();
                let _ = writeln!(s, r#"<circle cx="{}" cy="{}" r="{}" fill="{}"/>"#, n(c.x), n(c.y), n(r.size.w.min(r.size.h) / 2.0), hex(ink));
                return;
            }
            Shape::Final => {
                let c = r.center();
                let rad = r.size.w.min(r.size.h) / 2.0;
                let _ = writeln!(s, r#"<circle cx="{}" cy="{}" r="{}" fill="none" stroke="{}" stroke-width="1.5"/>"#, n(c.x), n(c.y), n(rad), hex(ink));
                let _ = writeln!(s, r#"<circle cx="{}" cy="{}" r="{}" fill="{}"/>"#, n(c.x), n(c.y), n(rad * 0.6), hex(ink));
                return;
            }
            Shape::Bar => {
                let _ = writeln!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1.5" fill="{}"/>"#, n(r.origin.x), n(r.origin.y), n(r.size.w), n(r.size.h), hex(ink));
                return;
            }
            _ => {}
        }
        let fill_attr = if nb.shape == Shape::Actor { "none".to_string() } else { hex(fill) };
        let d = match &nb.path {
            Some(cmds) => path_data(cmds),
            None => outline(nb.shape, r, self.technical),
        };
        let weight = nb.weight.unwrap_or(if self.technical { 1.0 } else { 1.5 });
        let _ = writeln!(s, r#"<path d="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#, d, fill_attr, hex(stroke), n(weight));
        if let Some(cmds) = &nb.detail {
            let _ = writeln!(s, r#"<path d="{}" fill="none" stroke="{}" stroke-width="1.25"/>"#, path_data(cmds), hex(stroke));
        }
        if let Some(cmds) = &nb.mark {
            let _ = writeln!(s, r#"<path d="{}" fill="{}"/>"#, path_data(cmds), hex(stroke));
        }
        self.glyph(s, nb, ink);
        if nb.shape == Shape::Cylinder {
            let ry = cyl_ry(r);
            let _ = writeln!(
                s,
                r#"<path d="M{} {} A{} {} 0 0 0 {} {}" fill="none" stroke="{}" stroke-width="1.5"/>"#,
                n(r.origin.x),
                n(r.origin.y + ry),
                n(r.size.w / 2.0),
                n(ry),
                n(r.origin.x + r.size.w),
                n(r.origin.y + ry),
                hex(stroke)
            );
        }
        if nb.shape == Shape::Lifeline {
            let c = r.center().x;
            let _ = writeln!(
                s,
                r#"<path d="M{} {} V{}" stroke="{}" stroke-dasharray="5 4"/>"#,
                n(c),
                n(r.origin.y + LIFELINE_HEAD),
                n(r.origin.y + r.size.h),
                hex(stroke)
            );
        }

        let structured = nb.shape == Shape::Block || !nb.compartments.is_empty() || nb.stereotype.is_some();
        let centered_glyph = nb.glyph.as_ref().is_some_and(|g| g.at == GlyphAt::Center);
        let pinned = nb.pin_band;
        if pinned {
            // A node-graph node: its title on a band in its color.
            let (x, y, w) = (r.origin.x, r.origin.y, r.size.w);
            let h = pins::PIN_TOP.min(r.size.h);
            let rad = if nb.shape == Shape::Rounded { 14.0 } else { 6.0 };
            let band = nb.stroke.unwrap_or(0x7950f2);
            let _ = writeln!(
                s,
                r#"<path d="M{} {} Q{} {} {} {} H{} Q{} {} {} {} V{} H{} Z" fill="{}" fill-opacity="{}"/>"#,
                n(x),
                n(y + rad),
                n(x),
                n(y),
                n(x + rad),
                n(y),
                n(x + w - rad),
                n(x + w),
                n(y),
                n(x + w),
                n(y + rad),
                n(y + h),
                n(x),
                hex(band),
                if nb.stroke.is_some() { "1" } else { "0.85" }
            );
            let ink = nb.stroke.map_or(t.text, contrast);
            let _ = writeln!(s, r#"<text x="{}" y="{}" font-size="12.5" font-weight="600" fill="{}" dominant-baseline="central">{}</text>"#, n(x + 12.0), n(y + h / 2.0), hex(ink), esc(&nb.label));
        } else if structured {
            self.structured(s, nb, ink, stroke);
        } else if nb.label_below {
            let lines = nb.label.split('\n').count().max(1) as f64;
            label(s, &nb.label, r.origin.x + r.size.w / 2.0, r.origin.y + r.size.h + 4.0 + 14.0 * 1.3 * lines / 2.0, 14.0, t.text);
        } else {
            let (top, height) = match nb.shape {
                _ if centered_glyph => (r.origin.y + r.size.h * 0.6, r.size.h * 0.4),
                Shape::Actor => (r.origin.y + r.size.h * 0.62, r.size.h * 0.38),
                Shape::Lifeline => (r.origin.y, LIFELINE_HEAD),
                Shape::Package => (r.origin.y + PACKAGE_TAB, r.size.h - PACKAGE_TAB),
                _ => (nb.label_area.origin.y, nb.label_area.size.h),
            };
            let cx = nb.label_area.origin.x + nb.label_area.size.w / 2.0;
            if nb.notes.is_empty() {
                label(s, &nb.label, cx, top + height / 2.0, 14.0, ink);
            } else {
                // Name over smaller notes, centred together.
                let labels: Vec<&str> = nb.label.split('\n').filter(|l| !l.is_empty()).collect();
                let (lh, nh) = (14.0 * 1.3, notation::NOTE_PT * 1.3);
                let total = lh * labels.len() as f64 + 4.0 + nh * nb.notes.len() as f64;
                let mut y = top + (height - total) / 2.0;
                for l in labels {
                    let _ = writeln!(s, r#"<text x="{}" y="{}" font-size="14" font-weight="600" fill="{}" text-anchor="middle" dominant-baseline="central">{}</text>"#, n(cx), n(y + lh / 2.0), hex(ink), esc(l));
                    y += lh;
                }
                y += 4.0;
                for l in &nb.notes {
                    let _ = writeln!(
                        s,
                        r#"<text x="{}" y="{}" font-size="{}" fill="{}" fill-opacity="0.8" text-anchor="middle" dominant-baseline="central">{}</text>"#,
                        n(cx),
                        n(y + nh / 2.0),
                        n(notation::NOTE_PT),
                        hex(ink),
                        esc(l.trim_start())
                    );
                    y += nh;
                }
            }
        }
        for p in &nb.ports {
            self.port(s, p, nb.pin_band, stroke, fill);
        }
    }

    /// A diagram link: a card with the linked diagram's title and a
    /// miniature of it.
    fn link(&self, s: &mut String, nb: &NodeBox, src: &str) {
        let t = self.t;
        let r = nb.rect;
        let (head, area) = graphing_scene::link_layout(r);
        let fill = nb.fill.unwrap_or(t.fill);
        let ink = nb.text.unwrap_or(t.text);
        let _ = writeln!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="10" fill="{}" stroke="{}" stroke-width="1.5"/>"#, n(r.origin.x), n(r.origin.y), n(r.size.w), n(r.size.h), hex(fill), hex(nb.stroke.unwrap_or(t.stroke)));
        let _ = writeln!(s, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}"/>"#, n(r.origin.x), n(head.origin.y + head.size.h), n(r.origin.x + r.size.w), n(head.origin.y + head.size.h), hex(t.group_stroke));
        let linked = self.refs.get(src);
        let title = if nb.label.is_empty() { linked.map_or_else(|| src.to_string(), |l| l.title.clone()) } else { nb.label.clone() };
        let _ = writeln!(s, r#"<text x="{}" y="{}" font-size="13" font-weight="600" fill="{}" dominant-baseline="central">{}</text>"#, n(head.origin.x + 12.0), n(head.origin.y + head.size.h / 2.0), hex(ink), esc(&title));
        match linked.and_then(|l| l.scene.miniature(area)) {
            Some(m) => {
                for g in &m.groups {
                    let _ = writeln!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="2" fill="none" stroke="{}" stroke-width="0.8"/>"#, n(g.origin.x), n(g.origin.y), n(g.size.w), n(g.size.h), hex(t.group_stroke));
                }
                for l in &m.lines {
                    let pts = l.iter().map(|p| format!("{},{}", n(p.x), n(p.y))).collect::<Vec<_>>().join(" ");
                    let _ = writeln!(s, r#"<polyline points="{pts}" fill="none" stroke="{}" stroke-width="0.8"/>"#, hex(t.muted));
                }
                for b in &m.nodes {
                    let _ = writeln!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1.5" fill="{}" fill-opacity="0.55"/>"#, n(b.origin.x), n(b.origin.y), n(b.size.w.max(1.5)), n(b.size.h.max(1.5)), hex(t.muted));
                }
            }
            None => {
                let what = if src.is_empty() { "No file set".to_string() } else { format!("Missing: {src}") };
                let c = area.center();
                let _ = writeln!(s, r#"<text x="{}" y="{}" font-size="11" fill="{}" text-anchor="middle" dominant-baseline="central">{}</text>"#, n(c.x), n(c.y), hex(t.muted), esc(&what));
            }
        }
        self.glyph(s, nb, ink);
    }

    /// A timing signal (see `graphing_scene::wave`).
    fn wave(&self, s: &mut String, nb: &NodeBox, w: &graphing_scene::wave::Wave) {
        use graphing_scene::wave::WavePrim;
        let t = self.t;
        let line = nb.stroke.unwrap_or(t.text);
        let pts = |p: &[Point]| p.iter().map(|p| format!("{},{}", n(p.x), n(p.y))).collect::<Vec<_>>().join(" ");
        for &x in &w.ticks {
            let _ = writeln!(s, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-opacity="0.35"/>"#, n(x), n(nb.rect.origin.y), n(x), n(nb.rect.origin.y + nb.rect.size.h), hex(t.group_stroke));
        }
        for prim in &w.prims {
            match prim {
                WavePrim::Line(p) => {
                    let _ = writeln!(s, r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="1.6" stroke-linejoin="round"/>"#, pts(p), hex(line));
                }
                WavePrim::Bus { points, label, unknown, center } => {
                    let (fill, alpha) = if *unknown { (t.muted, 0.22) } else { (nb.fill.unwrap_or(0x7950f2), if nb.fill.is_some() { 1.0 } else { 0.14 }) };
                    let _ = writeln!(s, r#"<polygon points="{}" fill="{}" fill-opacity="{alpha}" stroke="{}" stroke-width="1.3"/>"#, pts(points), hex(fill), hex(line));
                    if let Some(text) = label {
                        let _ = writeln!(
                            s,
                            r#"<text x="{}" y="{}" font-family="{MONO}" font-size="11" fill="{}" text-anchor="middle" dominant-baseline="central">{}</text>"#,
                            n(center.x),
                            n(center.y),
                            hex(t.text),
                            esc(text)
                        );
                    }
                }
            }
        }
        let _ = writeln!(
            s,
            r#"<text x="{}" y="{}" font-size="13" font-weight="600" fill="{}" dominant-baseline="central">{}</text>"#,
            n(w.label.origin.x + 4.0),
            n(w.label.origin.y + w.label.size.h / 2.0),
            hex(t.text),
            esc(&nb.label)
        );
    }

    /// A shape's Lucide icon, from [`SvgOptions::icons`].
    fn glyph(&self, s: &mut String, nb: &NodeBox, ink: u32) {
        let Some((g, at)) = nb.glyph_at() else { return };
        let Some(svg) = self.icons.get(&g.icon) else { return };
        let (x, y, size) = (at.origin.x, at.origin.y, at.size.w);
        // The file's own drawing, restyled: Lucide draws 24x24 strokes.
        let inner = svg.split_once('>').map(|(_, rest)| rest).unwrap_or("");
        let inner = inner.rsplit_once("</svg>").map_or(inner, |(body, _)| body);
        let _ = writeln!(
            s,
            r#"<g transform="translate({} {}) scale({})" fill="none" stroke="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{}</g>"#,
            n(x),
            n(y),
            n(size / 24.0),
            hex(ink),
            inner.trim()
        );
    }

    /// Header (stereotype + name), divider, compartments.
    fn structured(&self, s: &mut String, nb: &NodeBox, ink: u32, stroke: u32) {
        let t = self.t;
        let r = nb.rect;
        let x = r.origin.x + 14.0;
        let mut y = r.origin.y;
        let rounded = nb.shape != Shape::Block;
        // Classic UML centers the header; the technical look reads left to right.
        let (hx, anchor) = if self.technical && !rounded { (x, "start") } else { (r.origin.x + r.size.w / 2.0, "middle") };
        if let Some(st) = &nb.stereotype {
            y += notation::STEREO_H;
            let _ = writeln!(
                s,
                r#"<text x="{}" y="{}" text-anchor="{anchor}" font-family="{MONO}" font-size="10" fill="{}">{}</text>"#,
                n(hx),
                n(y + 4.0),
                hex(t.muted),
                esc(&format!("\u{ab}{st}\u{bb}"))
            );
        }
        let _ = writeln!(
            s,
            r#"<text x="{}" y="{}" text-anchor="{anchor}" font-size="12.5" font-weight="600" fill="{}">{}</text>"#,
            n(hx),
            n(y + 22.0),
            hex(ink),
            esc(&nb.label)
        );
        y += notation::HEADER_H;
        for c in &nb.compartments {
            let _ = writeln!(
                s,
                r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}"/>"#,
                n(r.origin.x),
                n(y),
                n(r.origin.x + r.size.w),
                n(y),
                hex(if self.technical { t.group_stroke } else { stroke })
            );
            y += notation::COMP_PAD;
            let mut lines: Vec<(String, bool)> = Vec::new();
            if let Some(title) = &c.title {
                lines.push((title.clone(), true));
            }
            lines.extend(c.lines.iter().map(|l| (format!("{}{l}", if c.title.is_some() { "  " } else { "" }), false)));
            for (text, _) in lines {
                y += notation::LINE_H;
                let _ = writeln!(
                    s,
                    r#"<text x="{}" y="{}" font-family="{MONO}" font-size="10.5" fill="{}" xml:space="preserve">{}</text>"#,
                    n(x),
                    n(y - 4.0),
                    hex(t.muted),
                    esc(&text)
                );
            }
            y += notation::COMP_PAD / 2.0;
        }
    }

    /// A node-graph pin: a dot (data) or an arrow (execution) in its type's
    /// color, its name inside the node.
    fn pin(&self, s: &mut String, p: &PortBox, pin: &pins::Pin, inside: bool) {
        let (c, r) = (p.at, pins::PIN_R);
        let color = if pin.exec() { hex(self.t.edge) } else { hex(pins::color(pin.shown_type())) };
        if pin.exec() {
            let pts = match p.side {
                Side::Left | Side::Right => [(c.x - r, c.y - r), (c.x + r, c.y), (c.x - r, c.y + r)],
                Side::Top | Side::Bottom => [(c.x - r, c.y - r), (c.x + r, c.y - r), (c.x, c.y + r)],
            };
            let pts = pts.iter().map(|(x, y)| format!("{},{}", n(*x), n(*y))).collect::<Vec<_>>().join(" ");
            let _ = writeln!(s, r#"<polygon points="{pts}" fill="{color}"/>"#);
        } else {
            let _ = writeln!(s, r#"<circle cx="{}" cy="{}" r="{}" fill="{color}" stroke="{}"/>"#, n(c.x), n(c.y), n(r), hex(self.t.bg));
        }
        let caption = pin.caption();
        if caption.is_empty() {
            return;
        }
        let gap = r + 6.0;
        let (x, y, anchor) = match (p.side, inside) {
            (Side::Left, true) => (c.x + gap, c.y, "start"),
            (Side::Right, true) => (c.x - gap, c.y, "end"),
            (Side::Bottom, true) => (c.x, c.y - gap - 6.0, "middle"),
            // Outside a shape that has content of its own, above the wire.
            (Side::Left, false) => (c.x - gap, c.y - 8.0, "end"),
            (Side::Right, false) => (c.x + gap, c.y - 8.0, "start"),
            (Side::Bottom, false) => (c.x + r + 3.0, c.y + r + 7.0, "start"),
            (Side::Top, _) => (c.x + r + 3.0, c.y - r - 7.0, "start"),
        };
        let _ = writeln!(
            s,
            r#"<text x="{}" y="{}" text-anchor="{anchor}" font-size="{}" fill="{}" dominant-baseline="central">{}</text>"#,
            n(x),
            n(y),
            n(pins::PIN_PT),
            hex(self.t.text),
            esc(&caption)
        );
    }

    fn port(&self, s: &mut String, p: &PortBox, inside: bool, stroke: u32, fill: u32) {
        if let Some(pin) = &p.pin {
            return self.pin(s, p, pin, inside);
        }
        let h = PORT / 2.0;
        let _ = writeln!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}" stroke="{}"/>"#,
            n(p.at.x - h),
            n(p.at.y - h),
            n(PORT),
            n(PORT),
            hex(fill),
            hex(stroke)
        );
        // Names sit clear of the connector: below a horizontal one, beside a vertical one.
        let (dx, dy, anchor) = match p.side {
            Side::Left => (-12.0, 17.0, "end"),
            Side::Right => (12.0, 17.0, "start"),
            Side::Top => (-12.0, -9.0, "end"),
            Side::Bottom => (-12.0, 17.0, "end"),
        };
        let _ = writeln!(
            s,
            r#"<text x="{}" y="{}" text-anchor="{anchor}" font-family="{MONO}" font-size="9.5" fill="{}">{}</text>"#,
            n(p.at.x + dx),
            n(p.at.y + dy),
            hex(self.t.muted),
            esc(&p.name)
        );
    }

    fn edge(&self, s: &mut String, e: &EdgeLine) {
        let t = self.t;
        if e.points.len() < 2 {
            return;
        }
        let color = hex(e.stroke.unwrap_or(t.edge));
        let mut d = String::new();
        for (i, p) in e.points.iter().enumerate() {
            let _ = write!(d, "{}{} {} ", if i == 0 { "M" } else { "L" }, n(p.x), n(p.y));
        }
        let dash = if e.dashed { r#" stroke-dasharray="6 4""# } else { "" };
        let width = if self.technical { "1.25" } else { "1.5" };
        let _ = writeln!(s, r#"<path d="{}" fill="none" stroke="{color}" stroke-width="{width}"{dash}/>"#, d.trim_end());
        let k = e.points.len();
        end(s, e.head, e.points[k - 2], e.points[k - 1], &color, &hex(t.bg));
        end(s, e.tail, e.points[1], e.points[0], &color, &hex(t.bg));

        let i = (k - 1) / 2;
        let (a, b) = (e.points[i], e.points[i + 1]);
        let mid = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let horizontal = (b.x - a.x).abs() >= (b.y - a.y).abs();
        let mut rows: Vec<(String, bool)> = Vec::new();
        if let Some(st) = &e.stereotype {
            rows.push((format!("\u{ab} {st} \u{bb}"), true));
        }
        if let Some(l) = &e.label {
            rows.extend(l.split('\n').map(|l| (l.to_string(), false)));
        }
        if rows.is_empty() {
            return;
        }
        if self.technical || e.stereotype.is_some() {
            // Above a horizontal connector, beside a vertical one.
            let (x0, y0, anchor) = if horizontal { (mid.x, mid.y - 8.0 - 13.0 * (rows.len() as f64 - 1.0), "middle") } else { (mid.x + 12.0, mid.y - 6.0, "start") };
            for (j, (text, stereo)) in rows.iter().enumerate() {
                let (font, size, fill) = if *stereo { (MONO, "10", t.flow) } else { ("inherit", "10.5", t.text) };
                let _ = writeln!(
                    s,
                    r#"<text x="{}" y="{}" text-anchor="{anchor}" font-family="{font}" font-size="{size}" fill="{}">{}</text>"#,
                    n(x0),
                    n(y0 + 13.0 * j as f64),
                    hex(fill),
                    esc(text)
                );
            }
        } else {
            let text = rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join("\n");
            // Rough width; renderers differ, so err wide.
            let w = text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * 7.0 + 8.0;
            let h = 12.0 * 1.4 * rows.len() as f64;
            let _ = writeln!(
                s,
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="3" fill="{}"/>"#,
                n(mid.x - w / 2.0),
                n(mid.y - h / 2.0),
                n(w),
                n(h),
                hex(t.bg)
            );
            label(s, &text, mid.x, mid.y, 12.0, t.text);
        }
    }
}

const LIFELINE_HEAD: f64 = 40.0;
const PACKAGE_TAB: f64 = 18.0;

/// SVG path data for a shape outline in `r`.
fn outline(shape: Shape, r: Rect, technical: bool) -> String {
    let (x, y, w, h) = (r.origin.x, r.origin.y, r.size.w, r.size.h);
    let poly = |pts: &[(f64, f64)]| {
        let mut d = String::new();
        for (i, (px, py)) in pts.iter().enumerate() {
            let _ = write!(d, "{}{} {} ", if i == 0 { "M" } else { "L" }, n(*px), n(*py));
        }
        d + "Z"
    };
    let rrect = |x: f64, y: f64, w: f64, h: f64, rr: f64| {
        format!(
            "M{} {} H{} A{rr} {rr} 0 0 1 {} {} V{} A{rr} {rr} 0 0 1 {} {} H{} A{rr} {rr} 0 0 1 {} {} V{} A{rr} {rr} 0 0 1 {} {} Z",
            n(x + rr),
            n(y),
            n(x + w - rr),
            n(x + w),
            n(y + rr),
            n(y + h - rr),
            n(x + w - rr),
            n(y + h),
            n(x + rr),
            n(x),
            n(y + h - rr),
            n(y + rr),
            n(x + rr),
            n(y),
        )
    };
    match shape {
        Shape::Rect | Shape::Block | Shape::Path => rrect(x, y, w, h, if technical { 2.0 } else { 6.0 }),
        Shape::Rounded => rrect(x, y, w, h, if technical { 10.0 } else { 14.0 }),
        Shape::Lifeline => rrect(x, y, w, LIFELINE_HEAD, 2.0),
        Shape::Package => {
            let tab = (w * 0.4).min(140.0);
            format!(
                "M{} {} H{} V{} H{} V{} H{} Z",
                n(x),
                n(y),
                n(x + tab),
                n(y + PACKAGE_TAB),
                n(x + w),
                n(y + h),
                n(x)
            )
        }
        Shape::Ellipse | Shape::Initial | Shape::Final => format!(
            "M{} {} A{} {} 0 0 1 {} {} A{} {} 0 0 1 {} {} Z",
            n(x),
            n(y + h / 2.0),
            n(w / 2.0),
            n(h / 2.0),
            n(x + w),
            n(y + h / 2.0),
            n(w / 2.0),
            n(h / 2.0),
            n(x),
            n(y + h / 2.0)
        ),
        Shape::Bar => rrect(x, y, w, h, 1.5),
        Shape::Diamond => poly(&[(x + w / 2.0, y), (x + w, y + h / 2.0), (x + w / 2.0, y + h), (x, y + h / 2.0)]),
        Shape::Cylinder => {
            let ry = cyl_ry(r);
            format!(
                "M{} {} A{} {} 0 0 1 {} {} V{} A{} {} 0 0 1 {} {} Z",
                n(x),
                n(y + ry),
                n(w / 2.0),
                n(ry),
                n(x + w),
                n(y + ry),
                n(y + h - ry),
                n(w / 2.0),
                n(ry),
                n(x),
                n(y + h - ry)
            )
        }
        Shape::Parallelogram => {
            let k = w * 0.15;
            poly(&[(x + k, y), (x + w, y), (x + w - k, y + h), (x, y + h)])
        }
        Shape::Hexagon => {
            let k = (w * 0.15).min(h / 2.0);
            poly(&[(x + k, y), (x + w - k, y), (x + w, y + h / 2.0), (x + w - k, y + h), (x + k, y + h), (x, y + h / 2.0)])
        }
        Shape::Note => {
            let f = w.min(h) * 0.25;
            poly(&[(x, y), (x + w - f, y), (x + w, y + f), (x + w, y + h), (x, y + h)])
        }
        Shape::Actor => {
            let cx = x + w / 2.0;
            let fig = h * 0.6;
            let head = fig * 0.18;
            let arm = fig * 0.3;
            format!(
                "M{} {} A{hd} {hd} 0 0 1 {} {} A{hd} {hd} 0 0 1 {} {} Z M{} {} L{} {} M{} {} L{} {} M{} {} L{} {} L{} {}",
                n(cx + head),
                n(y + head),
                n(cx - head),
                n(y + head),
                n(cx + head),
                n(y + head),
                n(cx),
                n(y + head * 2.0),
                n(cx),
                n(y + fig * 0.7),
                n(cx - arm),
                n(y + fig * 0.45),
                n(cx + arm),
                n(y + fig * 0.45),
                n(cx - arm * 0.8),
                n(y + fig),
                n(cx),
                n(y + fig * 0.7),
                n(cx + arm * 0.8),
                n(y + fig),
                hd = n(head),
            )
        }
    }
}

/// SVG path data for already-fitted custom outline commands.
fn path_data(cmds: &[graphing_scene::path::PathCmd]) -> String {
    use graphing_scene::path::PathCmd as C;
    let mut d = String::new();
    for c in cmds {
        let _ = match *c {
            C::Move(p) => write!(d, "M{} {} ", n(p.x), n(p.y)),
            C::Line(p) => write!(d, "L{} {} ", n(p.x), n(p.y)),
            C::Cubic(a, b, p) => write!(d, "C{} {} {} {} {} {} ", n(a.x), n(a.y), n(b.x), n(b.y), n(p.x), n(p.y)),
            C::Quad(a, p) => write!(d, "Q{} {} {} {} ", n(a.x), n(a.y), n(p.x), n(p.y)),
            C::Arc { rx, ry, rotation, large, sweep, to } => {
                write!(d, "A{} {} {} {} {} {} {} ", n(rx), n(ry), n(rotation), u8::from(large), u8::from(sweep), n(to.x), n(to.y))
            }
            C::Close => write!(d, "Z "),
        };
    }
    d.trim_end().to_string()
}

fn cyl_ry(r: Rect) -> f64 {
    (r.size.h * 0.12).min(r.size.w * 0.2)
}

/// Centered multi-line text block around (`cx`, `cy`).
fn label(s: &mut String, text: &str, cx: f64, cy: f64, size: f64, color: u32) {
    let lines: Vec<&str> = text.split('\n').collect();
    let lh = size * 1.3;
    let first = cy - lh * (lines.len() as f64 - 1.0) / 2.0;
    let _ = write!(s, r#"<text font-size="{}" fill="{}" text-anchor="middle" dominant-baseline="central">"#, n(size), hex(color));
    for (i, l) in lines.iter().enumerate() {
        let _ = write!(s, r#"<tspan x="{}" y="{}">{}</tspan>"#, n(cx), n(first + lh * i as f64), esc(l));
    }
    s.push_str("</text>\n");
}

/// End marker at `tip`, pointing away from `from`.
fn end(s: &mut String, kind: End, from: Point, tip: Point, color: &str, bg: &str) {
    let (dx, dy) = (tip.x - from.x, tip.y - from.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 || kind == End::None {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let at = |back: f64, side: f64| Point::new(tip.x - ux * back - uy * side, tip.y - uy * back + ux * side);
    let pts = |ps: &[Point]| ps.iter().map(|p| format!("{},{}", n(p.x), n(p.y))).collect::<Vec<_>>().join(" ");
    match kind {
        End::Arrow => {
            let _ = writeln!(s, r#"<polygon points="{}" fill="{color}"/>"#, pts(&[tip, at(11.0, 5.0), at(11.0, -5.0)]));
        }
        End::Open => {
            let _ = writeln!(s, r#"<polyline points="{}" fill="none" stroke="{color}" stroke-width="1.25"/>"#, pts(&[at(10.0, 5.5), tip, at(10.0, -5.5)]));
        }
        End::Triangle => {
            let _ = writeln!(s, r#"<polygon points="{}" fill="{bg}" stroke="{color}" stroke-width="1.25"/>"#, pts(&[tip, at(13.0, 7.0), at(13.0, -7.0)]));
        }
        End::Diamond | End::FilledDiamond => {
            let fill = if kind == End::FilledDiamond { color } else { bg };
            let _ = writeln!(
                s,
                r#"<polygon points="{}" fill="{fill}" stroke="{color}" stroke-width="1.25"/>"#,
                pts(&[tip, at(8.0, 5.0), at(16.0, 0.0), at(8.0, -5.0)])
            );
        }
        End::Circle => {
            let c = at(5.0, 0.0);
            let _ = writeln!(s, r#"<circle cx="{}" cy="{}" r="5" fill="{bg}" stroke="{color}" stroke-width="1.25"/>"#, n(c.x), n(c.y));
        }
        End::None => {}
        crow => {
            for st in crow.crow_strokes() {
                match st {
                    notation::EndStroke::Line(b1, s1, b2, s2) => {
                        let (a, b) = (at(b1, s1), at(b2, s2));
                        let _ = writeln!(s, r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{color}" stroke-width="1.25"/>"#, n(a.x), n(a.y), n(b.x), n(b.y));
                    }
                    notation::EndStroke::Ring { back, r } => {
                        let c = at(back, 0.0);
                        let _ = writeln!(s, r#"<circle cx="{}" cy="{}" r="{}" fill="{bg}" stroke="{color}" stroke-width="1.25"/>"#, n(c.x), n(c.y), n(r));
                    }
                }
            }
        }
    }
}

/// Render the SVG to PNG at `scale` (1.0 = one pixel per world unit).
pub fn to_png(scene: &Scene, opts: &SvgOptions, scale: f32) -> Result<Vec<u8>, ExportError> {
    let svg = to_svg(scene, opts);
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| ExportError::Svg(e.to_string()))?;
    let size = tree.size();
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or_else(|| ExportError::Png("empty image".into()))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| ExportError::Png(e.to_string()))
}

/// How an animation turns into frames.
#[derive(Debug, Clone, Copy)]
pub struct AnimOptions {
    pub fps: f64,
    /// Pixels per diagram unit, before `max_width` caps it.
    pub scale: f32,
    pub max_width: u32,
    /// Seconds the last frame holds before the loop starts over.
    pub hold: f64,
}

impl Default for AnimOptions {
    fn default() -> Self {
        Self { fps: 15.0, scale: 1.0, max_width: 1280, hold: 1.5 }
    }
}

/// One rendered frame: width, height, RGBA.
type Rendered = (u32, u32, Vec<u8>);

/// Width, height and every frame with how long it shows (ms).
pub type Frames = (u32, u32, Vec<(Vec<u8>, u32)>);

/// Every frame as straight RGBA with how long it shows (ms); identical
/// neighbours merge into one longer frame.
pub fn frames(scene: &Scene, opts: &SvgOptions, timeline: &Timeline, a: &AnimOptions) -> Result<Frames, ExportError> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let step_ms = (1000.0 / a.fps.max(1.0)).round() as u32;
    let n = ((timeline.total + a.hold) * a.fps).ceil().max(1.0) as usize;
    let viewport = anim_viewport(scene, timeline);
    let render_one = |i: usize| -> Result<Rendered, ExportError> {
        let t = (i as f64 / a.fps).min(timeline.total);
        let state = timeline.state(t);
        let moved = timeline.scene_for(&state);
        let svg = to_svg_at(moved.as_ref().unwrap_or(scene), opts, &state, viewport);
        let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| ExportError::Svg(e.to_string()))?;
        let size = tree.size();
        let scale = a.scale.min(a.max_width as f32 / size.width());
        let (w, h) = ((size.width() * scale).ceil().max(1.0) as u32, (size.height() * scale).ceil().max(1.0) as u32);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| ExportError::Png("empty image".into()))?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        let rgba = pixmap.pixels().iter().flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        }).collect();
        Ok((w, h, rgba))
    };
    // Frames render on every core, each thread a stretch of the timeline.
    let threads = std::thread::available_parallelism().map_or(4, |c| c.get()).min(n);
    let per = n.div_ceil(threads);
    let rendered: Vec<Result<Rendered, ExportError>> = std::thread::scope(|sc| {
        let handles: Vec<_> = (0..threads).map(|k| {
            let render_one = &render_one;
            sc.spawn(move || (k * per..((k + 1) * per).min(n)).map(render_one).collect::<Vec<_>>())
        }).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|_| vec![Err(ExportError::Png("a frame failed".into()))])).collect()
    });
    let mut out: Vec<(Vec<u8>, u32)> = Vec::new();
    let (mut w, mut h) = (1, 1);
    for frame in rendered {
        let (fw, fh, rgba) = frame?;
        (w, h) = (fw, fh);
        match out.last_mut() {
            Some((prev, ms)) if *prev == rgba => *ms += step_ms,
            _ => out.push((rgba, step_ms)),
        }
    }
    Ok((w, h, out))
}

/// The rectangle `(x, y, w, h)` where `cur` differs from `prev` (both
/// `w`-wide RGBA), or `None` when they match.
fn changed(prev: &[u8], cur: &[u8], w: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (i, (a, b)) in prev.as_chunks::<4>().0.iter().zip(cur.as_chunks::<4>().0).enumerate() {
        if a != b {
            let (x, y) = (i as u32 % w, i as u32 / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    (x0 != u32::MAX).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// The `(x, y, w, h)` part of a `full_w`-wide RGBA image.
fn crop(rgba: &[u8], full_w: u32, (x, y, w, h): (u32, u32, u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * full_w + x) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    out
}

/// Frames as the part that changed since the previous one, so still areas
/// are stored once: `(x, y, w, h, rgba, ms)`.
type Patch = (u32, u32, u32, u32, Vec<u8>, u32);

fn patches(w: u32, h: u32, frames: Vec<(Vec<u8>, u32)>) -> Vec<Patch> {
    let mut out: Vec<Patch> = Vec::with_capacity(frames.len());
    let mut prev: Option<Vec<u8>> = None;
    for (rgba, ms) in frames {
        let area = match &prev {
            None => (0, 0, w, h),
            Some(p) => changed(p, &rgba, w).unwrap_or((0, 0, 1, 1)),
        };
        out.push((area.0, area.1, area.2, area.3, crop(&rgba, w, area), ms));
        prev = Some(rgba);
    }
    out
}

/// The animation as a looping GIF.
pub fn to_gif(scene: &Scene, opts: &SvgOptions, timeline: &Timeline, a: &AnimOptions) -> Result<Vec<u8>, ExportError> {
    let (w, h, frames) = frames(scene, opts, timeline, a)?;
    let err = |e: gif::EncodingError| ExportError::Png(e.to_string());
    let (gw, gh) = (u16::try_from(w).unwrap_or(u16::MAX), u16::try_from(h).unwrap_or(u16::MAX));
    let mut bytes = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut bytes, gw, gh, &[]).map_err(err)?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(err)?;
        // Each frame draws over the last, so only what changed goes in.
        for (x, y, pw, ph, mut rgba, ms) in patches(w, h, frames) {
            let mut frame = gif::Frame::from_rgba_speed(pw as u16, ph as u16, &mut rgba, 10);
            frame.left = x as u16;
            frame.top = y as u16;
            frame.dispose = gif::DisposalMethod::Keep;
            // Hundredths of a second.
            frame.delay = (ms / 10).clamp(1, u16::MAX as u32) as u16;
            enc.write_frame(&frame).map_err(err)?;
        }
    }
    Ok(bytes)
}

/// The animation as a WebM video (AV1), for slides and sites that want
/// video rather than a GIF.
pub fn to_webm(scene: &Scene, opts: &SvgOptions, timeline: &Timeline, a: &AnimOptions) -> Result<Vec<u8>, ExportError> {
    let (w, h, frames) = frames(scene, opts, timeline, a)?;
    webm::encode(w, h, &frames, a.fps)
}

/// The animation as a looping APNG: full colour, sharper than a GIF.
pub fn to_apng(scene: &Scene, opts: &SvgOptions, timeline: &Timeline, a: &AnimOptions) -> Result<Vec<u8>, ExportError> {
    let (w, h, frames) = frames(scene, opts, timeline, a)?;
    let err = |e: png::EncodingError| ExportError::Png(e.to_string());
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(frames.len() as u32, 0).map_err(err)?;
        let mut writer = enc.write_header().map_err(err)?;
        for (x, y, pw, ph, rgba, ms) in patches(w, h, frames) {
            writer.set_frame_position(0, 0).map_err(err)?;
            writer.set_frame_dimension(pw, ph).map_err(err)?;
            writer.set_frame_position(x, y).map_err(err)?;
            // Blend over the last frame instead of replacing it.
            writer.set_blend_op(png::BlendOp::Over).map_err(err)?;
            writer.set_dispose_op(png::DisposeOp::None).map_err(err)?;
            writer.set_frame_delay(ms.min(u16::MAX as u32) as u16, 1000).map_err(err)?;
            writer.write_image_data(&rgba).map_err(err)?;
        }
        writer.finish().map_err(err)?;
    }
    Ok(bytes)
}

/// Parse `.gph` source, place unplaced nodes and build its scene.
pub fn scene_of(src: &str) -> Scene {
    animation_of(src).0
}

/// [`scene_of`] plus the diagram's `animate` steps laid out in time.
pub fn animation_of(src: &str) -> (Scene, Timeline) {
    let mut doc = Document::parse(src);
    let placed = graphing_scene::auto_place(doc.diagram());
    if !placed.is_empty() {
        let ops = placed.into_iter().map(|(id, pos)| Op::SetPlacement { id, placement: Some(Placement { pos, size: None }) }).collect();
        doc.apply(&Op::Batch(ops));
    }
    let scene = graphing_scene::build(doc.diagram(), &HashMap::new());
    let timeline = Timeline::new(doc.diagram(), &scene);
    (scene, timeline)
}

pub fn render_source(src: &str, opts: &SvgOptions) -> String {
    to_svg(&scene_of(src), opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn src_is_relative_with_forward_slashes() {
        let base = std::path::Path::new("/work/diagrams");
        assert_eq!(src_for(&base.join("sub").join("auth.gph"), Some(base)), "sub/auth.gph");
        assert_eq!(src_for(std::path::Path::new("/elsewhere/x.gph"), Some(base)), "/elsewhere/x.gph");
    }

    #[cfg(unix)]
    #[test]
    fn src_is_relative_through_a_symlinked_folder() {
        // macOS spells its temp folder both `/var/...` and `/private/var/...`.
        let real = std::env::temp_dir().join(format!("graphing-src-real-{}", std::process::id()));
        let link = std::env::temp_dir().join(format!("graphing-src-link-{}", std::process::id()));
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("auth.gph"), "a\n").unwrap();
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(src_for(&real.join("auth.gph"), Some(&link)), "auth.gph");
        assert_eq!(src_for(&link.join("auth.gph"), Some(&real)), "auth.gph");
        std::fs::remove_file(&link).ok();
        std::fs::remove_dir_all(&real).ok();
    }

    #[test]
    fn every_builtin_stencil_exports() {
        let reg = graphing_scene::stencils::registry();
        let ids: Vec<String> = reg.stencils().map(|s| s.id.clone()).collect();
        drop(reg);
        for id in ids {
            let src = format!("n: {id} \"Name\" {{ description: \"what it does\", technology: \"Rust\" }}\n");
            let scene = scene_of(&src);
            let n = &scene.nodes[0];
            assert!(n.rect.size.w > 0.0 && n.rect.size.h > 0.0, "{id}: empty box");
            let svg = to_svg(&scene, &SvgOptions::default());
            assert!(svg.contains("<path") || svg.contains("<circle") || svg.contains("<rect"), "{id}: draws nothing");
        }
    }

    #[test]
    fn crow_foot_ends_export() {
        let svg = render_source("a\nb\na -> b { kind: one-to-many }\nlayout {\n  a 0 0\n  b 300 0\n}\n", &SvgOptions::default());
        // Two bars for one-only, two prongs and a ring for zero-many.
        assert!(svg.matches("<line").count() >= 4, "{svg}");
        assert!(svg.contains("<circle"));
    }

    #[test]
    fn icons_draw_from_the_supplied_files() {
        let scene = scene_of("r: net.router \"Edge\"\n");
        assert_eq!(icons_used(&scene).into_iter().collect::<Vec<_>>(), ["Router"]);
        let mut opts = SvgOptions::default();
        opts.icons.insert("Router".into(), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="20" height="8" x="2" y="14" rx="2"/></svg>"#.into());
        let svg = to_svg(&scene, &opts);
        assert!(svg.contains(r#"<rect width="20" height="8" x="2" y="14" rx="2"/>"#), "{svg}");
        // Its label sits under the device, inside the picture.
        assert!(svg.contains(">Edge<"));
    }

    #[test]
    fn notation_examples_are_clean() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/notations");
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "gph") {
                continue;
            }
            seen += 1;
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&path).unwrap();
            let doc = Document::parse(src.clone());
            assert!(doc.diags().is_empty(), "{name}: {:?}", doc.diags());
            let wiring = pins::problems(doc.diagram());
            assert!(wiring.is_empty(), "{name}: {wiring:?}");
            let d = doc.diagram();
            let reg = graphing_scene::stencils::registry();
            for n in &d.nodes {
                let st = n.stencil.as_deref().unwrap_or("rect");
                assert!(reg.get(st).is_some(), "{name}: unknown stencil {st}");
            }
            let text = |v: Option<&graphing_model::Value>| v.map(graphing_model::Value::text);
            for e in &d.edges {
                if let Some(k) = text(d.edge_prop(e, "kind")) {
                    assert!(reg.edge_kind(&k).is_some(), "{name}: unknown edge kind {k}");
                }
            }
            for g in &d.groups {
                if let Some(k) = text(graphing_model::find_prop(&g.props, "kind")) {
                    assert!(reg.group_kind(&k).is_some(), "{name}: unknown group kind {k}");
                }
            }
            let kind = text(d.prop("kind")).expect("examples name their diagram kind");
            assert!(reg.diagram_kind(&kind).is_some(), "{name}: unknown diagram kind {kind}");
            drop(reg);
            let svg = render_source(&src, &SvgOptions::default());
            assert!(svg.len() > 1000, "{name}: suspiciously small SVG");
        }
        assert!(seen >= 12, "only {seen} examples");
    }

    const ANIMATED: &str = "a \"Client\"\nb \"Server\"\na -> b\nlayout {\n  a 0 0\n  b 300 0\n}\nanimate {\n  step \"One\" 1s {\n    show a\n    focus a\n  }\n  step \"Two\" 1s {\n    show b\n    flow a -> b\n    highlight b\n  }\n}\n";

    #[test]
    fn a_moment_hides_what_is_not_shown_yet() {
        let (scene, tl) = animation_of(ANIMATED);
        assert_eq!(tl.total, 2.0);
        let early = to_svg_at(&scene, &SvgOptions::default(), &tl.state(0.5), None);
        assert!(early.contains(">Client<") && !early.contains(">Server<"), "{early}");
        assert!(early.contains(">One<"), "the caption shows");
        let late = to_svg_at(&scene, &SvgOptions::default(), &tl.state(1.5), None);
        assert!(late.contains(">Server<") && late.contains("stroke-dasharray=\"0 46\""), "flow dots run");
    }

    #[test]
    fn animated_outputs_are_valid() {
        let (scene, tl) = animation_of(ANIMATED);
        let svg = to_svg_animated(&scene, &SvgOptions::default(), &tl);
        assert!(svg.contains("<animate attributeName=\"opacity\"") && svg.contains("attributeName=\"viewBox\""), "{svg}");
        resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).expect("parses");
        let small = AnimOptions { fps: 4.0, max_width: 200, hold: 0.25, ..Default::default() };
        let gif = to_gif(&scene, &SvgOptions::default(), &tl, &small).unwrap();
        assert!(gif.starts_with(b"GIF89a"));
        let png = to_apng(&scene, &SvgOptions::default(), &tl, &small).unwrap();
        assert!(png.starts_with(b"\x89PNG") && png.windows(4).any(|w| w == b"acTL"), "APNG carries its animation chunk");
        let (_, _, frames) = frames(&scene, &SvgOptions::default(), &tl, &small).unwrap();
        assert!(frames.len() > 2 && frames.iter().map(|f| f.1).sum::<u32>() >= 2000);
    }

    const AUTH: &str = include_str!("../../../examples/auth.gph");

    #[test]
    fn svg_has_every_node_and_arrow() {
        let svg = render_source(AUTH, &SvgOptions::default());
        let scene = scene_of(AUTH);
        // One outline path per node, plus cylinder lips.
        let cylinders = scene.nodes.iter().filter(|n| n.shape == Shape::Cylinder).count();
        let paths = svg.matches("<path d=").count();
        assert_eq!(paths, scene.nodes.len() + cylinders + scene.edges.len());
        let heads: usize = scene.edges.iter().map(|e| usize::from(e.head == End::Arrow) + usize::from(e.tail == End::Arrow)).sum();
        assert_eq!(svg.matches("<polygon").count(), heads);
        assert!(svg.contains("API Gateway"));
    }

    #[test]
    fn escapes_and_deterministic() {
        let src = "a: \"x < y & z > w\"\nb\na -> b \"<go>\"\n";
        let one = render_source(src, &SvgOptions::default());
        assert!(one.contains("x &lt; y &amp; z &gt; w") && one.contains("&lt;go&gt;"));
        assert_eq!(one, render_source(src, &SvgOptions::default()));
    }

    #[test]
    fn viewbox_and_theme() {
        let src = "a\nb\nlayout {\n  a 0 0\n  b 300 100\n}\n";
        let light = render_source(src, &SvgOptions::default());
        // Bounds 0..420 x 0..156 plus 24 padding.
        assert!(light.contains(r#"viewBox="-24 -24 468 204""#), "{light}");
        let dark = render_source(src, &SvgOptions { theme: Theme::dark(), ..Default::default() });
        assert!(dark.contains("#16181d") && !light.contains("#16181d"));
    }

    #[test]
    fn png_has_signature_and_size() {
        let scene = scene_of("a\nlayout {\n  a 0 0\n}\n");
        let png = to_png(&scene, &SvgOptions::default(), 2.0).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((w, h), ((120 + 48) * 2, (56 + 48) * 2));
    }
}
