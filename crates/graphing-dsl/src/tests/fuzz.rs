//! Randomised tests, no dependencies: a small seeded generator writes
//! diagrams in every brace and comment style, then random edits must round
//! trip through the text exactly as through the model, and comments must
//! survive. A second pass throws token soup at the parser.
//!
//! `GRAPHING_FUZZ=<cases>` runs more cases (the default keeps
//! `cargo test` quick); a failure prints the seed to replay.

use super::check;
use crate::Document;
use graphing_model::{Arrow, Diagram, Edge, Group, Node, Op, Placement, Point, Size, Step, Value, Verb};

/// xorshift64*: tiny, seeded, good enough to explore.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn cases() -> u64 {
    std::env::var("GRAPHING_FUZZ").ok().and_then(|v| v.parse().ok()).unwrap_or(150)
}

/// How one document is written.
struct Style {
    indent: &'static str,
    /// `{` on its own line after the header.
    allman: bool,
    /// Prop blocks spread over lines.
    multiline: bool,
    trailing_commas: bool,
    comments: bool,
}

impl Style {
    fn random(r: &mut Rng) -> Self {
        Self {
            indent: *r.pick(&["  ", "    ", "\t"]),
            allman: r.chance(40),
            multiline: r.chance(50),
            trailing_commas: r.chance(30),
            comments: r.chance(70),
        }
    }
}

/// Writes a document and counts the comments it must keep.
struct Writer<'a> {
    r: &'a mut Rng,
    st: Style,
    out: String,
    keep: usize,
    inner: usize,
}

const LABELS: &[&str] = &["API", "Data base", "say \"hi\"", "back\\slash", "two\nlines", "caf\u{e9}", "\u{1f680} launch", "", "a, b: {c}", "# not a comment"];
const STENCILS: &[&str] = &["rect", "core.cylinder", "flow.process", "ellipse", "diamond"];

impl Writer<'_> {
    /// A comment line that must survive every edit.
    fn keep(&mut self, indent: &str) {
        if self.st.comments && self.r.chance(35) {
            self.keep += 1;
            let n = self.keep;
            self.out.push_str(&format!("{indent}# keep: {n}\n"));
        }
    }

    /// A comment line inside a block; it goes only with its statement.
    fn inner(&mut self, indent: &str) {
        if self.st.comments && self.r.chance(30) {
            self.inner += 1;
            let n = self.inner;
            self.out.push_str(&format!("{indent}# inner: {n}\n"));
        }
    }

    fn trailing(&mut self) -> String {
        if self.st.comments && self.r.chance(20) { "  # note".to_string() } else { String::new() }
    }

    fn blank(&mut self) {
        if self.r.chance(25) {
            self.out.push('\n');
        }
    }

    fn value(&mut self) -> String {
        match self.r.below(6) {
            0 => format!("{}", self.r.below(400)),
            1 => format!("{}.5", self.r.below(9)),
            2 => (*self.r.pick(&["#fff", "#3b5bdb", "#11223344"])).to_string(),
            3 => (*self.r.pick(&["dashed", "solid", "sysml.flow"])).to_string(),
            4 => crate::print::fmt_str(self.r.pick(LABELS)),
            _ => "[1, \"two\", three]".to_string(),
        }
    }

    /// ` { k: v, ... }` in this document's style, or nothing.
    fn props(&mut self, keys: &[&str]) {
        let n = self.r.below(keys.len() + 1);
        if n == 0 {
            return;
        }
        let entries: Vec<String> = keys[..n].iter().map(|k| format!("{k}: {}", self.value())).collect();
        let ind = self.st.indent;
        if self.st.allman {
            self.out.push('\n');
            self.out.push('{');
        } else {
            self.out.push_str(" {");
        }
        if self.st.multiline {
            let t = self.trailing();
            self.out.push_str(&format!("{t}\n"));
            for (i, e) in entries.iter().enumerate() {
                self.inner(ind);
                let comma = if self.st.trailing_commas || (i + 1 < entries.len() && self.r.chance(30)) { "," } else { "" };
                self.out.push_str(&format!("{ind}{e}{comma}\n"));
            }
            self.inner(ind);
            self.out.push('}');
        } else {
            let comma = if self.st.trailing_commas { "," } else { "" };
            self.out.push_str(&format!(" {}{comma} }}", entries.join(", ")));
        }
    }

    fn end_stmt(&mut self) {
        let t = self.trailing();
        self.out.push_str(&t);
        self.out.push('\n');
    }
}

/// A random valid document.
fn document(r: &mut Rng) -> (String, usize) {
    let st = Style::random(r);
    let mut w = Writer { r, st, out: String::new(), keep: 0, inner: 0 };
    w.keep("");
    // Short pack names: shapes may be written either way.
    let aliased = w.r.chance(40);
    if aliased {
        w.out.push_str("use flow as f, core as c\n");
    }
    if w.r.chance(60) {
        w.out.push_str(&format!("diagram {}", crate::print::fmt_str(w.r.pick(LABELS))));
        w.props(&["routing", "flow"]);
        w.end_stmt();
    }
    if w.r.chance(30) {
        w.out.push_str("style hot { fill: red }");
        w.end_stmt();
    }
    let n = 2 + w.r.below(6);
    let nodes: Vec<String> = (0..n).map(|i| format!("n{i}")).collect();
    for id in &nodes {
        w.keep("");
        w.blank();
        let mut line = id.clone();
        if w.r.chance(50) {
            let stencil = if aliased && w.r.chance(50) { *w.r.pick(&["f.process", "c.rect"]) } else { *w.r.pick(STENCILS) };
            line = format!("{id}: {stencil}");
        }
        if w.r.chance(60) {
            line.push(' ');
            line.push_str(&crate::print::fmt_str(w.r.pick(LABELS)));
        }
        if w.r.chance(15) {
            line.push_str(" .hot");
        }
        w.out.push_str(&line);
        w.props(&["fill", "stroke", "w"]);
        w.end_stmt();
    }
    // Groups over disjoint runs of nodes, sometimes one inside another.
    let mut groups: Vec<String> = Vec::new();
    let mut at = 0;
    while at < n && w.r.chance(55) {
        let take = 1 + w.r.below((n - at).min(3));
        let mut members: Vec<String> = nodes[at..at + take].to_vec();
        if let Some(prev) = groups.last()
            && w.r.chance(25)
        {
            members.push(prev.clone());
        }
        at += take;
        let gid = format!("g{}", groups.len());
        w.keep("");
        w.out.push_str(&format!("group {gid}"));
        if w.r.chance(60) {
            w.out.push_str(&format!(" {}", crate::print::fmt_str(w.r.pick(LABELS))));
        }
        let ind = w.st.indent;
        if w.st.allman {
            w.out.push_str("\n{\n");
        } else if w.st.multiline {
            w.out.push_str(" {\n");
        } else {
            w.out.push_str(" { ");
        }
        let spread = w.st.allman || w.st.multiline;
        for (i, m) in members.iter().enumerate() {
            if spread {
                w.inner(ind);
                let comma = if w.st.trailing_commas || (i + 1 < members.len() && w.r.chance(30)) { "," } else { "" };
                w.out.push_str(&format!("{ind}{m}{comma}\n"));
            } else {
                let sep = if i + 1 < members.len() { if w.r.chance(50) { ", " } else { " " } } else { " " };
                w.out.push_str(&format!("{m}{sep}"));
            }
        }
        if spread {
            w.inner(ind);
        }
        w.out.push('}');
        w.props(&["kind", "look"]);
        w.end_stmt();
        groups.push(gid);
    }
    let arrows = ["->", "<-", "<->", "--"];
    for e in 0..w.r.below(n + 2) {
        w.keep("");
        let (a, b) = (w.r.below(n), w.r.below(n));
        let arrow = *w.r.pick(&arrows);
        let mut line = if w.r.chance(20) {
            format!("e{e}: n{a} {arrow} n{b}")
        } else if w.r.chance(20) {
            let c = w.r.below(n);
            format!("n{a} {arrow} n{b} -> n{c}")
        } else {
            format!("n{a} {arrow} n{b}")
        };
        if w.r.chance(40) {
            line.push(' ');
            line.push_str(&crate::print::fmt_str(w.r.pick(LABELS)));
        }
        w.out.push_str(&line);
        w.props(&["line", "kind"]);
        w.end_stmt();
    }
    if w.r.chance(70) {
        w.keep("");
        let ind = w.st.indent;
        w.out.push_str(if w.st.allman { "layout\n{\n" } else { "layout {\n" });
        for id in nodes.iter().chain(&groups) {
            if w.r.chance(60) {
                w.inner(ind);
                let size = if w.r.chance(30) { format!(" {}x{}", 100 + w.r.below(100), 40 + w.r.below(40)) } else { String::new() };
                let t = w.trailing();
                w.out.push_str(&format!("{ind}{id} {} {}{size}{t}\n", w.r.below(800), w.r.below(600)));
            }
        }
        w.inner(ind);
        w.out.push_str("}\n");
    }
    if w.r.chance(50) {
        w.keep("");
        let ind = w.st.indent;
        w.out.push_str(if w.st.allman { "animate\n{\n" } else { "animate {\n" });
        for s in 0..1 + w.r.below(3) {
            let dur = if w.r.chance(50) { " 2s" } else { "" };
            let open = if w.st.allman { format!("\n{ind}{{") } else { " {".to_string() };
            w.out.push_str(&format!("{ind}step \"S{s}\"{dur}{open}\n"));
            let target = format!("n{}", w.r.below(n));
            w.out.push_str(&format!("{ind}{ind}show {target}\n"));
            if w.r.chance(40) {
                w.out.push_str(&format!("{ind}{ind}ease snappy\n"));
            }
            w.out.push_str(&format!("{ind}}}\n"));
        }
        w.out.push_str("}\n");
    }
    w.keep("");
    (w.out, w.keep)
}

/// A random op that the model accepts, or `None` if this pick does not fit.
fn op(r: &mut Rng, d: &Diagram) -> Option<Op> {
    let node = |r: &mut Rng| (!d.nodes.is_empty()).then(|| d.nodes[r.below(d.nodes.len())].id.clone());
    let edge = |r: &mut Rng| (!d.edges.is_empty()).then(|| d.edges[r.below(d.edges.len())].id.clone());
    let group = |r: &mut Rng| (!d.groups.is_empty()).then(|| d.groups[r.below(d.groups.len())].id.clone());
    let any = |r: &mut Rng| match r.below(3) {
        0 => node(r),
        1 => edge(r),
        _ => group(r),
    };
    let label = |r: &mut Rng| (!r.chance(25)).then(|| r.pick(LABELS).to_string());
    let value = |r: &mut Rng| match r.below(5) {
        0 => Value::Num(r.below(300) as f64),
        1 => Value::Color("#abcdef".into()),
        2 => Value::Ident("dashed".into()),
        3 => Value::Str(r.pick(LABELS).to_string()),
        _ => Value::List(vec![Value::Num(1.0), Value::Str("x".into())]),
    };
    let key = |r: &mut Rng| r.pick(&["fill", "stroke", "line", "kind", "w", "note"]).to_string();
    let fresh = |r: &mut Rng| format!("x{}", r.below(1000));
    Some(match r.below(19) {
        0 => Op::SetLabel { id: any(r)?, label: label(r) },
        1 => Op::SetStencil { id: node(r)?, stencil: (!r.chance(30)).then(|| r.pick(STENCILS).to_string()) },
        2 => Op::SetArrow { id: edge(r)?, arrow: *r.pick(&[Arrow::Forward, Arrow::Back, Arrow::Both, Arrow::None]) },
        3 => Op::SetProp { id: any(r)?, key: key(r), value: (!r.chance(30)).then(|| value(r)) },
        4 => {
            let id = if r.chance(70) { node(r)? } else { group(r)? };
            let size = r.chance(30).then(|| Size::new(120.0, 60.0));
            Op::SetPlacement { id, placement: (!r.chance(20)).then(|| Placement { pos: Point::new(r.below(900) as f64, r.below(700) as f64), size }) }
        }
        5 => {
            let pts = (0..r.below(3)).map(|i| Point::new(i as f64 * 10.0, r.below(99) as f64)).collect();
            Op::SetWaypoints { id: edge(r)?, points: pts }
        }
        6 => {
            let node = Node { stencil: r.chance(50).then(|| r.pick(STENCILS).to_string()), label: label(r), ..Node::new(fresh(r)) };
            Op::AddNode { node, index: r.below(d.nodes.len() + 1) }
        }
        7 => {
            let (from, to) = (node(r)?, node(r)?);
            let id = graphing_model::edge_key(&from, &to, |k| d.edge(k).is_some());
            Op::AddEdge { edge: Edge { id, from, to, label: label(r), ..Default::default() }, index: r.below(d.edges.len() + 1) }
        }
        8 => Op::RemoveNode { id: node(r)? },
        9 => Op::RemoveEdge { id: edge(r)? },
        10 => Op::RemoveGroup { id: group(r)? },
        11 => {
            let g = group(r)?;
            let free: Vec<String> = d.nodes.iter().map(|n| n.id.clone()).filter(|n| !d.groups.iter().any(|g| g.members.contains(n))).collect();
            let mut members = d.group(&g)?.members.clone();
            match r.below(3) {
                0 if !members.is_empty() => {
                    members.remove(r.below(members.len()));
                }
                1 if !free.is_empty() => members.push(free[r.below(free.len())].clone()),
                _ => members.reverse(),
            }
            Op::SetMembers { group: g, members }
        }
        12 => {
            let free: Vec<String> = d.nodes.iter().map(|n| n.id.clone()).filter(|n| !d.groups.iter().any(|g| g.members.contains(n))).collect();
            let members = free.into_iter().take(1 + r.below(2)).collect();
            Op::AddGroup { group: Group { id: format!("grp{}", r.below(99)), label: label(r), members, props: Vec::new() }, index: r.below(d.groups.len() + 1) }
        }
        13 => Op::SetTitle { title: label(r) },
        14 => Op::SetDiagramProp { key: r.pick(&["routing", "flow", "note"]).to_string(), value: (!r.chance(30)).then(|| value(r)) },
        15 => {
            let id = edge(r)?;
            Op::SetEdgePorts { id, from_port: r.chance(50).then(|| "out".to_string()), to_port: r.chance(50).then(|| "in".to_string()) }
        }
        16 => {
            let step = Step { title: label(r), seconds: r.chance(50).then_some(1.5), actions: vec![graphing_model::Action { verb: Verb::Show, targets: vec![node(r)?] }], ..Default::default() };
            Op::AddStep { index: r.below(d.steps.len() + 1), step }
        }
        17 if !d.steps.is_empty() => Op::RemoveStep { index: r.below(d.steps.len()) },
        18 if !d.steps.is_empty() => {
            let mut step = d.steps[r.below(d.steps.len())].clone();
            step.title = label(r);
            Op::SetStep { index: r.below(d.steps.len()), step }
        }
        _ => return None,
    })
}

fn comments<'a>(src: &'a str, tag: &str) -> Vec<&'a str> {
    src.lines().filter_map(|l| l.find(tag).map(|i| l[i..].trim_end())).collect()
}

#[test]
fn random_documents_parse_clean_and_edits_round_trip() {
    for seed in 0..cases() {
        let mut r = Rng::new(seed);
        let (src, keeps) = document(&mut r);
        let mut d = Document::parse(src.as_str());
        assert!(d.diags().is_empty(), "seed {seed}: generated text has problems {:?}\n{src}", d.diags());
        assert_eq!(comments(&src, "# keep:").len(), keeps);
        for _ in 0..20 {
            let Some(op) = op(&mut r, d.diagram()) else { continue };
            if d.diagram().clone().apply(&op).is_none() {
                continue;
            }
            let before = d.source().to_string();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut probe = Document::parse(before.as_str());
                check(&mut probe, op.clone());
            }));
            if let Err(e) = outcome {
                let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                panic!("seed {seed}: {op:?}\n--- text before ---\n{before}\n--- failure ---\n{msg}");
            }
            d.apply(&op).expect("applies");
            let removes = matches!(op, Op::RemoveNode { .. } | Op::RemoveEdge { .. } | Op::RemoveGroup { .. });
            assert_eq!(comments(d.source(), "# keep:"), comments(&before, "# keep:"), "seed {seed}: {op:?} lost a comment\n{before}\n---\n{}", d.source());
            if !removes {
                // Splitting a chain copies its shared props, comments too.
                let now = comments(d.source(), "# inner:");
                let lost: Vec<&str> = comments(&before, "# inner:").into_iter().filter(|c| !now.contains(c)).collect();
                assert!(lost.is_empty(), "seed {seed}: {op:?} lost {lost:?}\n{before}\n---\n{}", d.source());
            }
        }
    }
}

const SOUP: &[&str] = &[
    "a", "b", "n1", "group", "layout", "animate", "step", "diagram", "use", "style", "via", "show", "move", "ease", "{", "}", "[", "]", ":", ",", ".", "->",
    "<-", "<->", "--", "\"", "\"x\"", "\"\\", "\\", "#", "# c", "//", "#fff", "12", "-3.5", "100x40", "2s", "500ms", "\n", "\n", "\n", " ", "\t", "\r\n", "\u{feff}",
    "\u{a0}", "\u{e9}", "\u{1f600}", "%", "?", "\0", "x.", ".y",
];

#[test]
fn token_soup_never_panics() {
    for seed in 0..cases() * 4 {
        let mut r = Rng::new(seed ^ 0xABCD);
        let src: String = (0..r.below(60)).map(|_| *r.pick(SOUP)).collect::<Vec<_>>().join(if r.chance(50) { " " } else { "" });
        let outcome = std::panic::catch_unwind(|| {
            let mut d = Document::parse(src.as_str());
            // Edits on broken text must not panic either.
            for _ in 0..4 {
                let mut r = Rng::new(seed);
                let Some(op) = op(&mut r, d.diagram()) else { continue };
                if d.diagram().clone().apply(&op).is_some() {
                    d.apply(&op);
                }
            }
        });
        assert!(outcome.is_ok(), "seed {seed} panicked on {src:?}");
    }
}
