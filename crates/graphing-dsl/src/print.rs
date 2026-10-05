use graphing_model::{Arrow, Edge, Node, Step, Value};

/// Whole numbers print without a fraction; others keep up to two decimals.
pub fn fmt_num(n: f64) -> String {
    let r = (n * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').to_string()
    }
}

pub fn fmt_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn fmt_value(v: &Value) -> String {
    match v {
        Value::Str(s) => fmt_str(s),
        Value::Num(n) => fmt_num(*n),
        Value::Color(c) | Value::Ident(c) => c.clone(),
        Value::List(items) => format!("[{}]", items.iter().map(fmt_value).collect::<Vec<_>>().join(", ")),
        Value::Pair(name, v) => format!("{name}: {}", fmt_value(v)),
    }
}

pub fn fmt_arrow(a: Arrow) -> &'static str {
    match a {
        Arrow::Forward => "->",
        Arrow::Back => "<-",
        Arrow::Both => "<->",
        Arrow::None => "--",
    }
}

pub fn fmt_props(props: &[(String, Value)]) -> String {
    let inner: Vec<String> = props.iter().map(|(k, v)| format!("{k}: {}", fmt_value(v))).collect();
    format!("{{ {} }}", inner.join(", "))
}

/// Tail shared by nodes and edges: ` "label" .a .b { props }`.
fn tail(label: &Option<String>, classes: &[String], props: &[(String, Value)]) -> String {
    let mut s = String::new();
    if let Some(l) = label {
        s.push(' ');
        s.push_str(&fmt_str(l));
    }
    for c in classes {
        s.push_str(" .");
        s.push_str(c);
    }
    if !props.is_empty() {
        s.push(' ');
        s.push_str(&fmt_props(props));
    }
    s
}

pub fn fmt_node(n: &Node) -> String {
    let tail = tail(&n.label, &n.classes, &n.props);
    match &n.stencil {
        Some(st) => format!("{}: {st}{tail}", n.id),
        None if tail.is_empty() => n.id.clone(),
        None => format!("{}:{tail}", n.id),
    }
}

pub fn fmt_edge(e: &Edge) -> String {
    // Generated keys (`a->b`, `a->b#2`) are not valid idents; lowering makes
    // them again.
    let auto = e.id.starts_with(&format!("{}->{}", e.from, e.to));
    let head = if auto { String::new() } else { format!("{}: ", e.id) };
    let end = |n: &str, p: &Option<String>| match p {
        Some(p) => format!("{n}.{p}"),
        None => n.to_string(),
    };
    let (from, to) = (end(&e.from, &e.from_port), end(&e.to, &e.to_port));
    format!("{head}{from} {} {to}{}", fmt_arrow(e.arrow), tail(&e.label, &e.classes, &e.props))
}

/// A step and its action lines; inner lines get `indent` plus two spaces.
/// The first line carries no indent (the caller places it).
pub fn fmt_step(step: &Step, indent: &str) -> String {
    let mut head = String::from("step");
    if let Some(t) = &step.title {
        head.push(' ');
        head.push_str(&fmt_str(t));
    }
    if let Some(s) = step.seconds {
        head.push_str(&format!(" {}s", fmt_num(s)));
    }
    if step.actions.is_empty() && step.ease.is_none() && step.moves.is_empty() {
        return format!("{head} {{}}");
    }
    let mut out = format!("{head} {{\n");
    if let Some(e) = step.ease {
        out.push_str(&format!("{indent}  ease {}\n", e.name()));
    }
    for (id, p) in &step.moves {
        out.push_str(&format!("{indent}  move {id} {} {}\n", fmt_num(p.x), fmt_num(p.y)));
    }
    for a in &step.actions {
        // Edge keys `a->b` read back as `a -> b`.
        let targets: Vec<String> = a.targets.iter().map(|t| t.replacen("->", " -> ", 1)).collect();
        out.push_str(&format!("{indent}  {} {}\n", a.verb.name(), targets.join(", ")));
    }
    out.push_str(&format!("{indent}}}"));
    out
}
