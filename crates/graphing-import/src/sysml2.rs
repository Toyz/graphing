//! SysML v2 textual notation -> `.gph`, for the subset graphing writes
//! (see `graphing_export::sysml2`) and the common hand-written forms of it:
//! packages, part / attribute / interface / constraint / requirement
//! definitions and their members, part usages, actions and control nodes,
//! states, use cases, and the relationships `:>`, `satisfy`, `dependency`,
//! `connect`, `flow`, `transition`, successions and `message`.
//!
//! Short names (`part def <vehicle> Vehicle`) become graphing ids. Comments
//! starting `// @` carry what v2 has no place for: diagram settings, group
//! props, extra element props and layout.

use std::collections::HashMap;
use std::fmt::Write as _;

use graphing_dsl::fmt_str;

use crate::{ImportError, Imported, ident};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    /// `'unrestricted name'`
    Name(String),
    Str(String),
    /// `<short name>`
    Short(String),
    /// `/* ... */`
    Block(String),
    Sym(&'static str),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: usize,
}

/// Tokens plus `// @...` directives by line.
fn lex(src: &str) -> (Vec<Token>, Vec<(usize, String)>) {
    let b = src.as_bytes();
    let (mut i, mut line) = (0, 1);
    let mut toks = Vec::new();
    let mut notes = Vec::new();
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(b.len(), |n| i + n);
                let text = src[i + 2..end].trim();
                if let Some(rest) = text.strip_prefix('@') {
                    notes.push((line, rest.to_string()));
                }
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n);
                let text = src[i + 2..end].trim().to_string();
                let start_line = line;
                line += src[i..end].matches('\n').count();
                toks.push(Token { tok: Tok::Block(text), line: start_line });
                i = (end + 2).min(b.len());
            }
            b'\'' | b'"' => {
                let quote = c;
                let mut s = String::new();
                i += 1;
                while i < b.len() && b[i] != quote {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        i += 1;
                    }
                    let ch = src[i..].chars().next().unwrap_or(' ');
                    s.push(ch);
                    i += ch.len_utf8();
                }
                i += 1;
                toks.push(Token { tok: if quote == b'"' { Tok::Str(s) } else { Tok::Name(s) }, line });
            }
            b'<' => {
                let end = src[i..].find('>').map_or(b.len(), |n| i + n);
                let inner = src[i + 1..end].trim();
                let inner = inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).unwrap_or(inner);
                toks.push(Token { tok: Tok::Short(inner.replace("\\'", "'")), line });
                i = end + 1;
            }
            b':' if src[i..].starts_with(":>>") => {
                toks.push(Token { tok: Tok::Sym(":>>"), line });
                i += 3;
            }
            b':' if src[i..].starts_with(":>") => {
                toks.push(Token { tok: Tok::Sym(":>"), line });
                i += 2;
            }
            c if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
                    i += 1;
                }
                toks.push(Token { tok: Tok::Word(src[start..i].to_string()), line });
            }
            _ => {
                let sym = match c {
                    b'{' => "{",
                    b'}' => "}",
                    b';' => ";",
                    b':' => ":",
                    b',' => ",",
                    b'.' => ".",
                    b'[' => "[",
                    b']' => "]",
                    b'=' => "=",
                    b'#' => "#",
                    b'*' => "*",
                    b'-' => "-",
                    b'/' => "/",
                    _ => "?",
                };
                toks.push(Token { tok: Tok::Sym(sym), line });
                i += 1;
            }
        }
    }
    (toks, notes)
}

#[derive(Debug, Default)]
struct Node {
    id: String,
    stencil: String,
    label: Option<String>,
    /// Printed props, in order.
    props: Vec<(String, String)>,
    lists: Vec<(String, Vec<String>)>,
}

#[derive(Debug)]
struct Edge {
    from: String,
    to: String,
    arrow: &'static str,
    label: Option<String>,
    props: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct Group {
    id: String,
    label: Option<String>,
    members: Vec<String>,
    props: String,
}

#[derive(Default)]
struct Model {
    title: Option<String>,
    diagram_props: Option<String>,
    uses: Option<String>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    groups: Vec<Group>,
    layout: Vec<String>,
    warnings: Vec<String>,
    /// Declared name -> id, for references by name instead of short name.
    names: HashMap<String, String>,
}

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
    notes: &'a [(usize, String)],
    m: Model,
}

/// Core stencil names that may arrive as `#metadata` on a part.
const CORE_STENCILS: &[&str] = &[
    "rect", "rounded", "ellipse", "circle", "diamond", "decision", "cylinder", "db", "database", "parallelogram", "data", "hexagon", "note", "actor",
    "terminal", "process", "package", "block",
];

pub fn parse(src: &str) -> Result<Imported, ImportError> {
    let (toks, notes) = lex(src);
    let mut p = Parser { toks: &toks, pos: 0, notes: &notes, m: Model::default() };
    for (_, note) in &notes {
        if let Some(rest) = note.strip_prefix("diagram ") {
            p.m.diagram_props = Some(rest.trim().to_string());
        } else if let Some(rest) = note.strip_prefix("use ") {
            p.m.uses = Some(rest.trim().to_string());
        } else if let Some(rest) = note.strip_prefix("layout ") {
            p.m.layout.push(rest.trim().to_string());
        }
    }
    while p.pos < toks.len() {
        p.stmt(None)?;
    }
    Ok(p.m.finish())
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn peek_word(&self, n: usize) -> Option<&str> {
        match self.toks.get(self.pos + n).map(|t| &t.tok) {
            Some(Tok::Word(w)) => Some(w),
            _ => None,
        }
    }

    fn line(&self) -> usize {
        self.toks.get(self.pos).map_or(0, |t| t.line)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.tok.clone());
        self.pos += 1;
        t
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        if self.peek() == Some(&Tok::Sym(sym_static(s))) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.peek_word(0) == Some(w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// `// @props ...` on the line a statement starts.
    fn props_note(&self, line: usize) -> Option<String> {
        self.notes.iter().find(|(l, n)| *l == line && n.starts_with("props ")).map(|(_, n)| n["props ".len()..].trim().to_string())
    }

    fn group_note(&self, line: usize) -> Option<String> {
        self.notes.iter().find(|(l, n)| *l + 1 == line && n.starts_with("group ")).map(|(_, n)| n["group ".len()..].trim().to_string())
    }

    /// A name: bare word or quoted.
    fn name(&mut self) -> Option<String> {
        match self.peek()? {
            Tok::Word(w) => {
                let w = w.clone();
                self.pos += 1;
                Some(w)
            }
            Tok::Name(n) => {
                let n = n.clone();
                self.pos += 1;
                Some(n)
            }
            _ => None,
        }
    }

    /// `a`, `a.b`, `'x'.y`, `<a>`.
    fn reference(&mut self) -> Option<String> {
        let mut parts = vec![match self.peek()? {
            Tok::Short(s) => {
                let s = s.clone();
                self.pos += 1;
                s
            }
            _ => self.name()?,
        }];
        while self.peek() == Some(&Tok::Sym(".")) {
            self.pos += 1;
            parts.push(self.name()?);
        }
        Some(parts.join("."))
    }

    /// Resolve a reference to `id` or `id.port`.
    fn resolve(&self, r: &str) -> String {
        let (head, port) = match r.split_once('.') {
            Some((h, p)) => (h, Some(p)),
            None => (r, None),
        };
        let id = self.m.names.get(head).cloned().unwrap_or_else(|| ident(head));
        match port {
            Some(p) => format!("{id}.{}", ident(p)),
            None => id,
        }
    }

    fn skip_to_end(&mut self) {
        let mut depth = 0;
        while let Some(t) = self.bump() {
            match t {
                Tok::Sym("{") => depth += 1,
                Tok::Sym("}") if depth <= 1 => return,
                Tok::Sym("}") => depth -= 1,
                Tok::Sym(";") if depth == 0 => return,
                _ => {}
            }
        }
    }

    /// One statement at package level. `group` collects member ids.
    fn stmt(&mut self, group: Option<usize>) -> Result<(), ImportError> {
        let line = self.line();
        let mut meta = Vec::new();
        while self.eat_sym("#") {
            if let Some(n) = self.name() {
                meta.push(n);
            }
        }
        let Some(word) = self.peek_word(0).map(str::to_string) else {
            if self.eat_sym("}") || self.eat_sym(";") {
                return Ok(());
            }
            self.pos += 1;
            return Ok(());
        };
        let next = self.peek_word(1).map(str::to_string);
        match (word.as_str(), next.as_deref()) {
            ("package", _) => {
                self.pos += 1;
                let short = self.short();
                let name = self.name();
                if self.eat_sym(";") {
                    let id = self.node_id(short, name.as_deref());
                    self.add_node(Node { id, stencil: "sysml.package".into(), label: name, ..Default::default() }, group, line);
                    return Ok(());
                }
                if !self.eat_sym("{") {
                    return Err(ImportError::Parse(format!("line {line}: expected `{{` after package")));
                }
                // The outermost package is the diagram.
                if self.m.title.is_none() && group.is_none() && short.is_none() && self.m.nodes.is_empty() && self.m.groups.is_empty() {
                    self.m.title = name;
                    while self.pos < self.toks.len() && !self.eat_sym("}") {
                        self.stmt(None)?;
                    }
                    return Ok(());
                }
                let id = short.unwrap_or_else(|| ident(name.as_deref().unwrap_or("group")));
                let props = self.group_note(line).unwrap_or_default();
                self.m.groups.push(Group { id: id.clone(), label: name, members: Vec::new(), props });
                let gi = self.m.groups.len() - 1;
                if let Some(parent) = group {
                    self.m.groups[parent].members.push(id);
                }
                while self.pos < self.toks.len() && !self.eat_sym("}") {
                    self.stmt(Some(gi))?;
                }
            }
            ("part" | "attribute" | "interface" | "constraint" | "requirement" | "verification", Some("def")) => {
                self.pos += 2;
                let stencil = match word.as_str() {
                    "part" => "sysml.block",
                    "attribute" => "sysml.valuetype",
                    "interface" => "sysml.interface",
                    "constraint" => "sysml.constraint",
                    "requirement" => "sysml.requirement",
                    _ => "sysml.testcase",
                };
                let short = self.short();
                let name = self.name();
                let id = self.node_id(short, name.as_deref());
                let mut node = Node { id: id.clone(), stencil: stencil.into(), label: name, ..Default::default() };
                if let Some(st) = meta.first() {
                    node.props.push(("stereotype".into(), stereo(st)));
                }
                if self.eat_sym(":>") {
                    while let Some(r) = self.reference() {
                        let to = self.resolve(&r);
                        self.m.edges.push(Edge { from: id.clone(), to, arrow: "->", label: None, props: vec![("kind".into(), "generalization".into())] });
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                }
                self.body(&mut node);
                self.add_node(node, group, line);
            }
            ("part", _) => {
                self.pos += 1;
                let short = self.short();
                let name = self.name();
                let ty = if self.eat_sym(":") { self.reference() } else { None };
                let (id, label) = match (short, name, ty) {
                    (Some(s), n, _) => (s, n),
                    (None, Some(n), Some(t)) => (ident(&n), Some(t)),
                    (None, n, _) => (self.node_id(None, n.as_deref()), n),
                };
                let mut stencil = "sysml.part".to_string();
                let mut node_meta = None;
                for m in &meta {
                    match m.as_str() {
                        "actor" => stencil = "sysml.actor".into(),
                        "lifeline" => stencil = "sysml.lifeline".into(),
                        s if CORE_STENCILS.contains(&s) => stencil = s.into(),
                        s if s.contains('.') => stencil = s.into(),
                        s => node_meta = Some(s.to_string()),
                    }
                }
                let mut node = Node { id, stencil, label, ..Default::default() };
                if let Some(st) = node_meta {
                    node.props.push(("stereotype".into(), stereo(&st)));
                }
                self.body(&mut node);
                self.add_node(node, group, line);
            }
            ("action" | "state" | "decide" | "fork" | "join" | "merge", _) | ("use", Some("case")) => {
                self.pos += if word == "use" { 2 } else { 1 };
                let short = self.short();
                let name = self.name();
                let stencil = match (word.as_str(), meta.first().map(String::as_str)) {
                    ("action", Some("initial")) => "sysml.initial",
                    ("action", Some("final")) => "sysml.final",
                    ("action", _) => "sysml.action",
                    ("state", _) => "sysml.state",
                    ("decide" | "merge", _) => "sysml.decision",
                    ("fork", _) => "sysml.fork",
                    ("join", _) => "sysml.join",
                    _ => "sysml.usecase",
                };
                let id = self.node_id(short, name.as_deref());
                let label = if matches!(stencil, "sysml.initial" | "sysml.final" | "sysml.decision" | "sysml.fork" | "sysml.join") { None } else { name };
                let mut node = Node { id, stencil: stencil.into(), label, ..Default::default() };
                if let (Some(st), "sysml.action" | "sysml.state" | "sysml.usecase") = (meta.first(), stencil) {
                    node.props.push(("stereotype".into(), stereo(st)));
                }
                self.body(&mut node);
                self.add_node(node, group, line);
            }
            ("comment", _) => {
                self.pos += 1;
                let short = self.short();
                let text = match self.peek() {
                    Some(Tok::Block(t)) => {
                        let t = t.clone();
                        self.pos += 1;
                        Some(t)
                    }
                    _ => None,
                };
                let id = self.node_id(short, None);
                self.add_node(Node { id, stencil: "sysml.rationale".into(), label: text, ..Default::default() }, group, line);
            }
            ("satisfy", _) => {
                self.pos += 1;
                let to = self.reference();
                self.eat_word("by");
                let from = self.reference();
                self.eat_sym(";");
                if let (Some(f), Some(t)) = (from, to) {
                    self.edge(&f, &t, "->", None, Some("satisfy"), line);
                }
            }
            ("dependency" | "flow" | "message", _) => {
                self.pos += 1;
                let label = if self.peek_word(0) == Some("from") { None } else { self.name() };
                self.eat_word("of");
                self.eat_word("from");
                let from = self.reference();
                self.eat_word("to");
                let to = self.reference();
                self.eat_sym(";");
                let kind = match word.as_str() {
                    "flow" => Some("flow".to_string()),
                    "dependency" => meta.first().cloned(),
                    _ => None,
                };
                let arrow = if word == "flow" && meta.is_empty() { "--" } else { "->" };
                if let (Some(f), Some(t)) = (from, to) {
                    self.edge(&f, &t, arrow, label, kind.as_deref(), line);
                }
            }
            ("connect", _) | ("connection", _) => {
                self.pos += 1;
                let label = if word == "connection" {
                    let l = self.name();
                    self.eat_word("connect");
                    l
                } else {
                    None
                };
                let from = self.reference();
                self.eat_word("to");
                let to = self.reference();
                self.eat_sym(";");
                if let (Some(f), Some(t)) = (from, to) {
                    self.edge(&f, &t, "--", label, None, line);
                }
            }
            ("transition", _) => {
                self.pos += 1;
                if self.peek_word(0) != Some("first") {
                    self.name();
                }
                self.eat_word("first");
                let from = self.reference();
                let mut trigger = None;
                let mut guard = None;
                let mut effect = None;
                loop {
                    if self.eat_word("accept") {
                        trigger = self.name();
                    } else if self.eat_word("if") {
                        guard = self.name();
                    } else if self.eat_word("do") {
                        effect = self.name();
                    } else {
                        break;
                    }
                }
                self.eat_word("then");
                let to = self.reference();
                self.eat_sym(";");
                let mut label = trigger.unwrap_or_default();
                if let Some(g) = guard {
                    let _ = write!(label, "{}[{g}]", if label.is_empty() { "" } else { " " });
                }
                if let Some(e) = effect {
                    let _ = write!(label, "{}/ {e}", if label.is_empty() { "" } else { " " });
                }
                if let (Some(f), Some(t)) = (from, to) {
                    self.edge(&f, &t, "->", (!label.is_empty()).then_some(label), Some("transition"), line);
                }
            }
            ("first", _) | ("succession", _) => {
                self.pos += 1;
                let label = if word == "succession" {
                    let l = if self.peek_word(0) == Some("first") { None } else { self.name() };
                    self.eat_word("first");
                    l
                } else {
                    None
                };
                let from = self.reference();
                self.eat_word("then");
                let to = self.reference();
                self.eat_sym(";");
                if let (Some(f), Some(t)) = (from, to) {
                    self.edge(&f, &t, "->", label, None, line);
                }
            }
            (other, _) => {
                self.m.warnings.push(format!("line {line}: `{other}` is not imported"));
                self.skip_to_end();
            }
        }
        Ok(())
    }

    fn short(&mut self) -> Option<String> {
        match self.peek() {
            Some(Tok::Short(s)) => {
                let s = s.clone();
                self.pos += 1;
                Some(s)
            }
            _ => None,
        }
    }

    /// The id for a new element: its short name, else its name made an id,
    /// else a fresh `n<k>`.
    fn node_id(&mut self, short: Option<String>, name: Option<&str>) -> String {
        let id = short.unwrap_or_else(|| match name {
            Some(n) => ident(n),
            None => format!("n{}", self.m.nodes.len() + 1),
        });
        if let Some(n) = name {
            self.m.names.entry(n.to_string()).or_insert_with(|| id.clone());
        }
        id
    }

    fn add_node(&mut self, mut node: Node, group: Option<usize>, line: usize) {
        if let Some(p) = self.props_note(line) {
            node.props.push(("@raw".into(), p));
        }
        if let Some(g) = group {
            self.m.groups[g].members.push(node.id.clone());
        }
        self.m.nodes.push(node);
    }

    fn edge(&mut self, from: &str, to: &str, arrow: &'static str, label: Option<String>, kind: Option<&str>, line: usize) {
        let mut props = Vec::new();
        if let Some(k) = kind {
            props.push(("kind".to_string(), ident(k)));
        }
        if let Some(p) = self.props_note(line) {
            props.push(("@raw".into(), p));
        }
        let (from, to) = (self.resolve(from), self.resolve(to));
        self.m.edges.push(Edge { from, to, arrow, label, props });
    }

    /// `;` or `{ members }`, filling compartments and fields.
    fn body(&mut self, node: &mut Node) {
        if self.eat_sym(";") || !self.eat_sym("{") {
            return;
        }
        while let Some(t) = self.peek().cloned() {
            match t {
                Tok::Sym("}") => {
                    self.pos += 1;
                    return;
                }
                Tok::Word(w) if w == "doc" => {
                    self.pos += 1;
                    if let Some(Tok::Block(text)) = self.peek().cloned() {
                        self.pos += 1;
                        node.props.push(("text".into(), fmt_str(&text)));
                    }
                }
                Tok::Word(w) if w == "attribute" && self.toks.get(self.pos + 1).map(|t| &t.tok) == Some(&Tok::Sym(":>>")) => {
                    self.pos += 2;
                    let key = self.name().unwrap_or_default();
                    self.eat_sym("=");
                    let value = match self.bump() {
                        Some(Tok::Str(s) | Tok::Name(s) | Tok::Word(s)) => s,
                        _ => String::new(),
                    };
                    self.eat_sym(";");
                    if key == "reqId" {
                        node.props.push(("rid".into(), fmt_str(&value)));
                    }
                }
                Tok::Word(w) if matches!(w.as_str(), "entry" | "do" | "exit") => {
                    self.pos += 1;
                    self.eat_word("action");
                    let text = self.name().unwrap_or_default();
                    self.eat_sym(";");
                    node.props.push((w.clone(), fmt_str(&text)));
                }
                Tok::Word(w) => {
                    self.pos += 1;
                    let key = match w.as_str() {
                        "attribute" => "values",
                        "part" => "parts",
                        "ref" => {
                            self.eat_word("part");
                            "references"
                        }
                        "port" => "ports",
                        "action" => "operations",
                        "constraint" => "constraints",
                        "flow" => "flows",
                        "in" => "parameters",
                        _ => {
                            self.skip_to_end();
                            continue;
                        }
                    };
                    let line = self.member_text();
                    push_list(&mut node.lists, key, line);
                }
                _ => self.pos += 1,
            }
        }
    }

    /// A member declaration back to compartment text: `wheels : Wheel[4]`
    /// -> `wheels : Wheel [4]`, `{ doc /* note */ }` -> ` { note }`.
    fn member_text(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.peek().cloned() {
                None => break,
                Some(Tok::Sym(";")) => {
                    self.pos += 1;
                    break;
                }
                Some(Tok::Sym("{")) => {
                    self.pos += 1;
                    let mut note = None;
                    while let Some(t) = self.bump() {
                        match t {
                            Tok::Block(b) => note = Some(b),
                            Tok::Sym("}") => break,
                            _ => {}
                        }
                    }
                    if let Some(n) = note {
                        let _ = write!(s, " {{ {n} }}");
                    }
                    break;
                }
                Some(Tok::Sym(sym)) => {
                    self.pos += 1;
                    match sym {
                        ":" | "=" => {
                            let _ = write!(s, " {sym} ");
                        }
                        "[" => s.push_str(" ["),
                        "," => s.push_str(", "),
                        other => s.push_str(other),
                    }
                }
                Some(Tok::Word(w) | Tok::Name(w)) => {
                    self.pos += 1;
                    s.push_str(&w);
                }
                Some(Tok::Str(v)) => {
                    self.pos += 1;
                    let _ = write!(s, "\"{v}\"");
                }
                Some(_) => self.pos += 1,
            }
        }
        s.trim().to_string()
    }
}

fn push_list(lists: &mut Vec<(String, Vec<String>)>, key: &str, item: String) {
    match lists.iter_mut().find(|(k, _)| k == key) {
        Some((_, l)) => l.push(item),
        None => lists.push((key.to_string(), vec![item])),
    }
}

/// A stereotype value, bare when it is a plain word.
fn stereo(s: &str) -> String {
    if s.chars().all(|c| c.is_alphanumeric() || c == '_') { s.to_string() } else { fmt_str(s) }
}

fn sym_static(s: &str) -> &'static str {
    match s {
        "{" => "{",
        "}" => "}",
        ";" => ";",
        ":" => ":",
        ":>" => ":>",
        ":>>" => ":>>",
        "," => ",",
        "." => ".",
        "=" => "=",
        "#" => "#",
        _ => "?",
    }
}

impl Model {
    fn finish(mut self) -> Imported {
        // `#actor` / `#lifeline` mean the core shapes outside SysML diagrams.
        if self.uses.as_deref().is_some_and(|u| !u.split(',').any(|p| p.trim() == "sysml")) {
            for n in &mut self.nodes {
                if let Some(core) = n.stencil.strip_prefix("sysml.").filter(|s| matches!(*s, "actor" | "lifeline")) {
                    n.stencil = core.to_string();
                }
            }
        }
        let mut s = String::new();
        let title = self.title.clone().unwrap_or_else(|| "Imported".into());
        let kind = self.diagram_props.clone().unwrap_or_else(|| {
            let has = |st: &str| self.nodes.iter().any(|n| n.stencil == st);
            let k = if has("sysml.requirement") {
                "req"
            } else if has("sysml.state") {
                "stm"
            } else if has("sysml.action") {
                "act"
            } else if has("sysml.usecase") {
                "uc"
            } else if has("sysml.lifeline") {
                "sd"
            } else if self.nodes.iter().any(|n| n.stencil == "sysml.part") {
                "ibd"
            } else {
                "bdd"
            };
            format!("kind: {k}")
        });
        let _ = writeln!(s, "diagram {} {{ {kind} }}", fmt_str(&title));
        let sysml = self.nodes.iter().any(|n| n.stencil.starts_with("sysml."));
        match &self.uses {
            Some(u) => {
                let _ = writeln!(s, "use {u}");
            }
            None if sysml => s.push_str("use sysml\n"),
            None => {}
        }
        s.push('\n');
        for n in &self.nodes {
            let mut props: Vec<String> = Vec::new();
            for (k, v) in &n.props {
                if k == "@raw" {
                    props.push(v.clone());
                } else {
                    props.push(format!("{k}: {v}"));
                }
            }
            for (k, items) in &n.lists {
                let list: Vec<String> = items.iter().map(|i| fmt_str(i)).collect();
                props.push(format!("{k}: [{}]", list.join(", ")));
            }
            let label = n.label.as_ref().map(|l| format!(" {}", fmt_str(l))).unwrap_or_default();
            let props = if props.is_empty() {
                String::new()
            } else if props.iter().any(|p| p.contains('[')) {
                format!(" {{\n  {}\n}}", props.join("\n  "))
            } else {
                format!(" {{ {} }}", props.join(", "))
            };
            let _ = writeln!(s, "{}: {}{label}{props}", n.id, n.stencil);
        }
        if !self.groups.is_empty() {
            s.push('\n');
        }
        for g in &self.groups {
            let label = g.label.as_ref().map(|l| format!(" {}", fmt_str(l))).unwrap_or_default();
            let props = if g.props.is_empty() { String::new() } else { format!(" {{ {} }}", g.props) };
            let _ = writeln!(s, "group {}{label} {{ {} }}{props}", g.id, g.members.join(" "));
        }
        if !self.edges.is_empty() {
            s.push('\n');
        }
        for e in &self.edges {
            let label = e.label.as_ref().map(|l| format!(" {}", fmt_str(l))).unwrap_or_default();
            let props: Vec<String> = e.props.iter().map(|(k, v)| if k == "@raw" { v.clone() } else { format!("{k}: {v}") }).collect();
            let props = if props.is_empty() { String::new() } else { format!(" {{ {} }}", props.join(", ")) };
            let _ = writeln!(s, "{} {} {}{label}{props}", e.from, e.arrow, e.to);
        }
        if !self.layout.is_empty() {
            s.push_str("\nlayout {\n");
            for l in &self.layout {
                let _ = writeln!(s, "  {l}");
            }
            s.push_str("}\n");
        }
        Imported { source: s, warnings: self.warnings }
    }
}
