//! Mermaid flowcharts (`graph`/`flowchart`) and basic state diagrams.

use crate::{ImportError, OEdge, OGroup, Out, ident, value};

pub(crate) fn parse(src: &str) -> Result<Out, ImportError> {
    let mut out = Out::default();
    let mut lines = statements(src);
    let header = lines.next().ok_or_else(|| ImportError::Parse("empty mermaid source".into()))?;
    let kw = header.split_whitespace().next().unwrap_or("");
    match kw {
        "graph" | "flowchart" => flowchart(&mut out, lines),
        "stateDiagram" | "stateDiagram-v2" => state(&mut out, lines),
        other => return Err(ImportError::Unsupported(other.to_string())),
    }
    Ok(out)
}

/// Non-empty statements: comments and code fences dropped, `;` splits.
fn statements(src: &str) -> impl Iterator<Item = String> + '_ {
    src.lines()
        .map(|l| l.split("%%").next().unwrap_or("").trim())
        .filter(|l| !l.starts_with("```"))
        .flat_map(|l| l.split(';').map(str::trim).map(str::to_string).collect::<Vec<_>>())
        .filter(|l| !l.is_empty())
}

/// Shape brackets, longest opener first: (open, close, stencil).
const SHAPES: &[(&str, &str, &str)] = &[
    ("([", "])", "rounded"),
    ("[[", "]]", "rect"),
    ("[(", ")]", "db"),
    ("((", "))", "ellipse"),
    ("{{", "}}", "hexagon"),
    ("[/", "/]", "io"),
    ("[\\", "\\]", "io"),
    ("[/", "\\]", "io"),
    ("[\\", "/]", "io"),
    ("[", "]", "rect"),
    ("(", ")", "rounded"),
    ("{", "}", "decision"),
    (">", "]", "rect"),
];

fn clean_label(raw: &str) -> String {
    let t = raw.trim();
    let t = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t);
    t.replace("<br/>", "\n").replace("<br>", "\n").replace("<br />", "\n")
}

struct Cursor<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.at..]
    }

    fn skip_ws(&mut self) {
        let r = self.rest();
        self.at += r.len() - r.trim_start().len();
    }

    fn eat(&mut self, p: &str) -> bool {
        if self.rest().starts_with(p) {
            self.at += p.len();
            true
        } else {
            false
        }
    }

    fn done(&self) -> bool {
        self.rest().trim().is_empty()
    }
}

/// `id`, `id[label]`, `id:::class` ...; declares/updates the node.
fn node_ref(out: &mut Out, c: &mut Cursor) -> Option<String> {
    c.skip_ws();
    let r = c.rest();
    // Dashes start edges, so an id stops before `--`, `-.` or `->`.
    let mut len = 0;
    for ch in r.chars() {
        let tail = &r[len..];
        let dash_ok = ch == '-' && !(tail.starts_with("--") || tail.starts_with("-.") || tail.starts_with("->"));
        if !(ch.is_alphanumeric() || ch == '_' || dash_ok) {
            break;
        }
        len += ch.len_utf8();
    }
    let raw = r[..len].trim_end_matches('-');
    if raw.is_empty() {
        return None;
    }
    c.at += raw.len();
    let id = ident(raw);
    out.node(&id);
    for (open, close, stencil) in SHAPES {
        if c.rest().starts_with(open) {
            let body_start = c.at + open.len();
            // Quoted labels may contain the closer.
            let body = &c.s[body_start..];
            let end = if body.trim_start().starts_with('"') {
                let q = body.find('"').unwrap_or(0);
                body[q + 1..].find('"').map(|e| q + 1 + e + 1).and_then(|after| body[after..].find(close).map(|k| after + k))
            } else {
                body.find(close)
            };
            if let Some(end) = end {
                let n = out.node(&id);
                n.label = Some(clean_label(&body[..end]));
                n.stencil = Some(stencil);
                c.at = body_start + end + close.len();
                break;
            }
        }
    }
    if c.eat(":::") {
        let r = c.rest();
        let len = r.find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-')).unwrap_or(r.len());
        let class = ident(&r[..len]);
        c.at += len;
        out.node(&id).classes.push(class);
    }
    Some(id)
}

/// Group of `&`-joined node refs.
fn node_list(out: &mut Out, c: &mut Cursor) -> Option<Vec<String>> {
    let mut ids = vec![node_ref(out, c)?];
    loop {
        c.skip_ws();
        if !c.eat("&") {
            return Some(ids);
        }
        ids.push(node_ref(out, c)?);
    }
}

struct Link {
    arrow: &'static str,
    dashed: bool,
    thick: bool,
    label: Option<String>,
    odd_head: bool,
}

fn link(c: &mut Cursor) -> Option<Link> {
    c.skip_ws();
    let r = c.rest();
    let op_len = r.find(|ch: char| !matches!(ch, '<' | '>' | '-' | '=' | '.')).unwrap_or(r.len());
    if op_len < 2 {
        return None;
    }
    let mut op = r[..op_len].to_string();
    c.at += op_len;
    let mut label = None;
    // `-- text -->`, `== text ==>`, `-. text .->`
    if matches!(op.as_str(), "--" | "==" | "-.") && c.rest().starts_with(char::is_whitespace) {
        let closers: &[&str] = match op.as_str() {
            "--" => &["-->", "---", "--o", "--x"],
            "==" => &["==>", "==="],
            _ => &[".->", ".-"],
        };
        let rest = c.rest();
        let (pos, closer) = closers.iter().filter_map(|cl| rest.find(cl).map(|p| (p, *cl))).min()?;
        label = Some(clean_label(&rest[..pos]));
        c.at += pos + closer.len();
        op.push_str(closer);
    }
    let mut odd_head = false;
    if op.ends_with('-') && c.rest().starts_with(['o', 'x']) && c.rest()[1..].starts_with(|ch: char| ch.is_whitespace() || ch.is_alphanumeric()) {
        // `--o` / `--x`: circle/cross heads become plain arrows.
        let after = &c.rest()[1..];
        if after.starts_with(char::is_whitespace) || op.len() >= 2 {
            c.at += 1;
            op.push('>');
            odd_head = true;
        }
    }
    c.skip_ws();
    if c.eat("|") {
        let r = c.rest();
        let end = r.find('|')?;
        label = Some(clean_label(&r[..end]));
        c.at += end + 1;
    }
    let back = op.starts_with('<');
    let fwd = op.ends_with('>');
    let arrow = match (back, fwd) {
        (true, true) => "<->",
        (false, true) => "->",
        (true, false) => "<-",
        (false, false) => "--",
    };
    Some(Link { arrow, dashed: op.contains('.'), thick: op.contains('='), label, odd_head })
}

fn props_from_css(out: &mut Out, css: &str) -> Vec<(String, String)> {
    let mut props = Vec::new();
    for part in css.split(',') {
        let Some((k, v)) = part.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let key = match k {
            "fill" => "fill",
            "stroke" => "stroke",
            "color" => "color",
            "stroke-width" => "width",
            "stroke-dasharray" => {
                props.push(("line".to_string(), "dashed".to_string()));
                continue;
            }
            other => {
                out.warnings.push(format!("style `{other}` ignored"));
                continue;
            }
        };
        props.push((key.to_string(), value(v)));
    }
    props
}

fn flowchart(out: &mut Out, lines: impl Iterator<Item = String>) {
    let mut stack: Vec<OGroup> = Vec::new();
    let mut subgraph_n = 0;
    for line in lines {
        let kw = line.split_whitespace().next().unwrap_or("");
        match kw {
            "subgraph" => {
                let rest = line["subgraph".len()..].trim();
                subgraph_n += 1;
                let (id, label) = match rest.find('[') {
                    Some(i) if rest.ends_with(']') => (ident(rest[..i].trim()), Some(clean_label(&rest[i + 1..rest.len() - 1]))),
                    _ if rest.is_empty() => (format!("group{subgraph_n}"), None),
                    _ if rest.contains(char::is_whitespace) || rest.starts_with('"') => (format!("group{subgraph_n}"), Some(clean_label(rest))),
                    _ => (ident(rest), Some(rest.to_string())),
                };
                stack.push(OGroup { id, label, ..Default::default() });
            }
            "end" => {
                if let Some(g) = stack.pop() {
                    if let Some(parent) = stack.last_mut() {
                        parent.members.push(g.id.clone());
                    }
                    out.groups.push(g);
                }
            }
            "direction" => {}
            "classDef" => {
                let rest = line["classDef".len()..].trim();
                let Some((names, css)) = rest.split_once(char::is_whitespace) else { continue };
                let props = props_from_css(out, css.trim());
                for name in names.split(',') {
                    out.styles.push((ident(name.trim()), props.clone()));
                }
            }
            "class" => {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let [_, ids, class] = parts.as_slice() {
                    for id in ids.split(',') {
                        out.node(&ident(id.trim())).classes.push(ident(class));
                    }
                }
            }
            "style" => {
                let rest = line["style".len()..].trim();
                let Some((id, css)) = rest.split_once(char::is_whitespace) else { continue };
                let props = props_from_css(out, css.trim());
                out.node(&ident(id)).props.extend(props);
            }
            "linkStyle" | "click" | "accTitle" | "accDescr" => out.warnings.push(format!("`{kw}` ignored")),
            _ => {
                let before: Vec<String> = out.nodes.iter().map(|n| n.id.clone()).collect();
                if !chain(out, &line) {
                    out.warnings.push(format!("could not read `{line}`"));
                }
                // New nodes inside a subgraph belong to it.
                if let Some(g) = stack.last_mut() {
                    let mentioned = mentioned_ids(&line);
                    for n in &out.nodes {
                        let new = !before.contains(&n.id);
                        if (new || mentioned.contains(&n.id)) && !g.members.contains(&n.id) && (new || !line.contains("--")) {
                            g.members.push(n.id.clone());
                        }
                    }
                }
            }
        }
    }
    while let Some(g) = stack.pop() {
        out.warnings.push(format!("subgraph `{}` not closed", g.id));
        out.groups.push(g);
    }
}

/// Bare ids on a line, for subgraph membership of existing nodes.
fn mentioned_ids(line: &str) -> Vec<String> {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).map(ident).collect()
}

/// `A --> B -- x --> C & D`. Returns false if the line is not a statement.
fn chain(out: &mut Out, line: &str) -> bool {
    let mut c = Cursor { s: line, at: 0 };
    let Some(mut left) = node_list(out, &mut c) else { return false };
    while !c.done() {
        let Some(l) = link(&mut c) else { return false };
        let Some(right) = node_list(out, &mut c) else { return false };
        if l.odd_head {
            out.warnings.push("circle/cross arrow heads imported as plain arrows".into());
        }
        for a in &left {
            for b in &right {
                let mut props = Vec::new();
                if l.dashed {
                    props.push(("line".into(), "dashed".into()));
                }
                if l.thick {
                    props.push(("width".into(), "3".into()));
                }
                out.edges.push(OEdge { from: a.clone(), to: b.clone(), arrow: l.arrow, label: l.label.clone(), props, via: Vec::new() });
            }
        }
        left = right;
    }
    true
}

fn state(out: &mut Out, lines: impl Iterator<Item = String>) {
    let mut stack: Vec<OGroup> = Vec::new();
    for line in lines {
        let l = line.trim();
        if l == "}" {
            if let Some(g) = stack.pop() {
                if let Some(p) = stack.last_mut() {
                    p.members.push(g.id.clone());
                }
                out.groups.push(g);
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("state ") {
            let rest = rest.trim();
            if let Some(body) = rest.strip_suffix('{') {
                let id = ident(body.trim());
                stack.push(OGroup { id, label: Some(body.trim().to_string()), ..Default::default() });
            } else if let Some((label, id)) = rest.split_once(" as ") {
                let id = ident(id.trim());
                out.node(&id).label = Some(clean_label(label));
                out.node(&id).stencil = Some("rounded");
                if let Some(g) = stack.last_mut() {
                    g.members.push(id);
                }
            }
            continue;
        }
        if l.starts_with("note") || l.starts_with("direction") {
            out.warnings.push(format!("`{l}` ignored"));
            continue;
        }
        let (body, label) = match l.split_once(" : ") {
            Some((b, lab)) => (b.trim(), Some(lab.trim().to_string())),
            None => (l, None),
        };
        if let Some((a, b)) = body.split_once("-->") {
            let mut end = |raw: &str, start: bool| -> String {
                let raw = raw.trim();
                if raw == "[*]" {
                    let id = if start { "start" } else { "end" };
                    let n = out.node(id);
                    n.stencil = Some("ellipse");
                    n.label.get_or_insert_with(|| if start { "start".into() } else { "end".into() });
                    id.to_string()
                } else {
                    let id = ident(raw);
                    let n = out.node(&id);
                    n.stencil.get_or_insert("rounded");
                    id
                }
            };
            let (from, to) = (end(a, true), end(b, false));
            if let Some(g) = stack.last_mut() {
                for id in [&from, &to] {
                    if !g.members.contains(id) {
                        g.members.push(id.clone());
                    }
                }
            }
            out.edges.push(OEdge { from, to, arrow: "->", label, props: Vec::new(), via: Vec::new() });
        } else if let Some(lab) = label {
            // `A : description`
            let id = ident(body);
            let n = out.node(&id);
            n.stencil.get_or_insert("rounded");
            n.label = Some(lab);
        } else {
            let id = ident(body);
            out.node(&id).stencil.get_or_insert("rounded");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{assert_clean, from_mermaid};
    use graphing_model::Arrow;

    #[test]
    fn flowchart_shapes_edges_and_styles() {
        let src = r#"
%% sample
graph TD
    A[Start] --> B{Is it?}
    B -->|Yes| C([Done])
    B -- No --> D[(DB)]
    D -.-> E((Circle)) ==> F{{Hex}}
    F --- G[/Data/]
    E <--> A
    C --o G
    A["quoted [label]"]
    classDef hot fill:#f96,stroke:#333,stroke-width:4px
    class C,D hot
    style G fill:#bbf
    H:::hot --> I
    J & K --> L
    linkStyle 0 stroke:red
"#;
        let out = from_mermaid(src).unwrap();
        let d = assert_clean(&out.source);
        assert_eq!(d.node("B").unwrap().stencil.as_deref(), Some("decision"));
        assert_eq!(d.node("C").unwrap().stencil.as_deref(), Some("rounded"));
        assert_eq!(d.node("D").unwrap().stencil.as_deref(), Some("db"));
        assert_eq!(d.node("E").unwrap().stencil.as_deref(), Some("ellipse"));
        assert_eq!(d.node("F").unwrap().stencil.as_deref(), Some("hexagon"));
        assert_eq!(d.node("G").unwrap().stencil.as_deref(), Some("io"));
        assert_eq!(d.node("A").unwrap().label.as_deref(), Some("quoted [label]"));
        assert_eq!(d.edge("B->C").unwrap().label.as_deref(), Some("Yes"));
        assert_eq!(d.edge("B->D").unwrap().label.as_deref(), Some("No"));
        assert!(d.edge_prop(d.edge("D->E").unwrap(), "line").is_some());
        assert_eq!(d.edge("E->A").unwrap().arrow, Arrow::Both);
        assert_eq!(d.edge("F->G").unwrap().arrow, Arrow::None);
        assert!(d.edge("J->L").is_some() && d.edge("K->L").is_some());
        assert!(d.styles.contains_key("hot"));
        assert_eq!(d.node("C").unwrap().classes, ["hot"]);
        assert_eq!(d.node("H").unwrap().classes, ["hot"]);
        assert!(out.warnings.iter().any(|w| w.contains("linkStyle")));
        assert!(out.warnings.iter().any(|w| w.contains("circle/cross")));
    }

    #[test]
    fn subgraphs_nest() {
        let src = "flowchart LR\n  subgraph outer [Outer box]\n    a --> b\n    subgraph inner\n      c\n    end\n  end\n  b --> c\n";
        let out = from_mermaid(src).unwrap();
        let d = assert_clean(&out.source);
        let outer = d.group("outer").unwrap();
        assert_eq!(outer.label.as_deref(), Some("Outer box"));
        assert!(outer.members.contains(&"a".into()) && outer.members.contains(&"inner".into()));
        assert_eq!(d.group("inner").unwrap().members, ["c"]);
        assert!(d.edge("b->c").is_some());
    }

    #[test]
    fn state_diagram() {
        let src = "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Busy : start\n  state \"Working hard\" as Busy\n  Busy --> [*]\n";
        let out = from_mermaid(src).unwrap();
        let d = assert_clean(&out.source);
        assert_eq!(d.node("start").unwrap().stencil.as_deref(), Some("ellipse"));
        assert_eq!(d.node("Busy").unwrap().label.as_deref(), Some("Working hard"));
        assert_eq!(d.edge("Idle->Busy").unwrap().label.as_deref(), Some("start"));
        assert!(d.edge("Busy->end").is_some());
    }

    #[test]
    fn unsupported_types_error() {
        assert!(from_mermaid("sequenceDiagram\n A->>B: hi").is_err());
    }
}
