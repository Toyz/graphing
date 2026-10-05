//! Editor support for `.gph` text: token roles for highlighting, the
//! completion context at a cursor, and foldable blocks.
//!
//! This works on tokens, not the AST, so it stays useful while the text is
//! half typed and does not parse.

use crate::lexer::{Span, Tok, Token, lex};

pub const KEYWORDS: &[&str] = &["diagram", "use", "style", "group", "layout", "animate"];

/// What owns a `{ ... }` prop block.
#[derive(Debug, Clone, PartialEq)]
pub enum Owner {
    Diagram,
    /// A node, with its stencil as written (if any).
    Node(Option<String>),
    Edge,
    Group,
    Style,
}

/// The role of one span of source text.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    Comment,
    Keyword,
    /// The id a statement declares (`api` in `api: rect`).
    Def,
    /// A node or group referred to (edge endpoints, layout, group members).
    Ref,
    /// The port part of `node.port`; carries the node id.
    Port(String),
    Stencil,
    Pack,
    /// `.class` after a node or edge, or the name in `style name { }`.
    Class,
    Key(Owner),
    /// A bare value; carries the owner and key it belongs to.
    Value(Owner, String),
    Str,
    Num,
    Color,
    Arrow,
    Punct,
    Error,
}

impl Role {
    /// The highlight style name, from the shared syntax theme.
    pub fn style(&self) -> &'static str {
        match self {
            Role::Comment => "comment",
            Role::Keyword => "keyword",
            Role::Def => "function",
            Role::Ref => "variable",
            Role::Port(_) => "property",
            Role::Stencil | Role::Pack => "type",
            Role::Class => "attribute",
            Role::Key(_) => "property",
            Role::Value(..) => "constant",
            Role::Str => "string",
            Role::Num => "number",
            Role::Color => "string.special",
            Role::Arrow => "operator",
            Role::Punct => "punctuation",
            Role::Error => "hint",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Frame {
    Props { owner: Owner, key: Option<String> },
    Layout,
    Members,
    List,
}

#[derive(Debug, Clone, PartialEq)]
enum St {
    Start,
    /// After a keyword, waiting for its argument.
    AfterKw,
    /// After `id:`.
    AfterDef,
    /// Inside a node statement, after its stencil.
    Node,
    Edge,
    Done,
}

/// Every span with a role, ordered and non-overlapping. Whitespace and
/// newlines are left out.
pub fn roles(src: &str) -> Vec<(Span, Role)> {
    let toks = lex(src);
    let mut out = Vec::with_capacity(toks.len());
    let mut stack: Vec<Frame> = Vec::new();
    let mut kw: Option<String> = None;
    let mut st = St::Start;
    let mut stencil: Option<String> = None;
    let mut braces = 0;
    let mut last_end = 0;

    for (i, t) in toks.iter().enumerate() {
        comments(src, last_end..t.span.start, &mut out);
        last_end = t.span.end;
        let next = toks.get(i + 1).map(|t| &t.tok);
        let prev = i.checked_sub(1).map(|p| &toks[p].tok);
        let role = match &t.tok {
            Tok::Newline => {
                if stack.is_empty() {
                    st = St::Start;
                    kw = None;
                    stencil = None;
                    braces = 0;
                }
                continue;
            }
            Tok::LBrace => {
                let frame = match (kw.as_deref(), &st) {
                    (Some("layout"), _) => Frame::Layout,
                    (Some("group"), _) if braces == 0 => Frame::Members,
                    (Some("group"), _) => Frame::Props { owner: Owner::Group, key: None },
                    (Some("diagram"), _) => Frame::Props { owner: Owner::Diagram, key: None },
                    (Some("style"), _) => Frame::Props { owner: Owner::Style, key: None },
                    (_, St::Edge) => Frame::Props { owner: Owner::Edge, key: None },
                    _ => Frame::Props { owner: Owner::Node(stencil.clone()), key: None },
                };
                if stack.is_empty() {
                    braces += 1;
                }
                stack.push(frame);
                Role::Punct
            }
            Tok::RBrace => {
                if stack.last().is_some_and(|f| *f == Frame::List) {
                    stack.pop();
                }
                stack.pop();
                Role::Punct
            }
            Tok::LBracket => {
                stack.push(Frame::List);
                Role::Punct
            }
            Tok::RBracket => {
                if stack.last() == Some(&Frame::List) {
                    stack.pop();
                }
                Role::Punct
            }
            Tok::Str(_) => Role::Str,
            Tok::Num(_) | Tok::Size(..) => Role::Num,
            Tok::Color(_) => Role::Color,
            Tok::Fwd | Tok::Back | Tok::Both | Tok::Line => {
                if stack.is_empty() {
                    st = St::Edge;
                }
                Role::Arrow
            }
            Tok::Colon | Tok::Comma | Tok::Dot => Role::Punct,
            Tok::Error(_) => Role::Error,
            Tok::Ident(w) => {
                let role = ident_role(w, prev, next, &mut stack, &mut st, &mut kw, &mut stencil);
                if matches!(role, Role::Ref) {
                    push_ref(w, t.span.clone(), &mut out);
                    continue;
                }
                role
            }
        };
        out.push((t.span.clone(), role));
    }
    comments(src, last_end..src.len(), &mut out);
    out
}

fn is_arrow(t: Option<&Tok>) -> bool {
    matches!(t, Some(Tok::Fwd | Tok::Back | Tok::Both | Tok::Line))
}

fn ident_role(
    w: &str,
    prev: Option<&Tok>,
    next: Option<&Tok>,
    stack: &mut [Frame],
    st: &mut St,
    kw: &mut Option<String>,
    stencil: &mut Option<String>,
) -> Role {
    let in_list = stack.last() == Some(&Frame::List);
    match stack.iter_mut().rev().find(|f| **f != Frame::List) {
        Some(Frame::Props { owner, key }) => {
            if !in_list && matches!(prev, Some(Tok::LBrace | Tok::Comma | Tok::Newline) | None) {
                *key = Some(w.to_string());
                Role::Key(owner.clone())
            } else {
                Role::Value(owner.clone(), key.clone().unwrap_or_default())
            }
        }
        Some(Frame::Layout) => match prev {
            Some(Tok::LBrace | Tok::Newline) | None => Role::Ref,
            _ => Role::Value(Owner::Diagram, String::new()),
        },
        Some(Frame::Members) => Role::Ref,
        Some(Frame::List) => unreachable!("skipped above"),
        None => {
            if prev == Some(&Tok::Dot) && matches!(st, St::Node | St::Edge | St::Done) {
                return Role::Class;
            }
            match st {
                St::Start => {
                    if KEYWORDS.contains(&w) && !matches!(next, Some(Tok::Colon)) && !is_arrow(next) {
                        *kw = Some(w.to_string());
                        *st = St::AfterKw;
                        Role::Keyword
                    } else if next == Some(&Tok::Colon) {
                        *st = St::AfterDef;
                        Role::Def
                    } else if is_arrow(next) {
                        *st = St::Edge;
                        Role::Ref
                    } else {
                        *st = St::Node;
                        Role::Def
                    }
                }
                St::AfterKw => match kw.as_deref() {
                    // `use c4 as arch`: `as` reads as a keyword, `arch` as a name.
                    Some("use") if w == "as" => Role::Keyword,
                    Some("use") if matches!(prev, Some(Tok::Ident(p)) if p == "as") => Role::Def,
                    Some("use") => Role::Pack,
                    Some("style") => {
                        *st = St::Done;
                        Role::Class
                    }
                    Some("group") => {
                        *st = St::Done;
                        Role::Def
                    }
                    _ => Role::Value(Owner::Diagram, String::new()),
                },
                St::AfterDef => {
                    if is_arrow(next) {
                        *st = St::Edge;
                        Role::Ref
                    } else {
                        *st = St::Node;
                        *stencil = Some(w.to_string());
                        Role::Stencil
                    }
                }
                St::Edge => Role::Ref,
                St::Node | St::Done => Role::Value(Owner::Diagram, String::new()),
            }
        }
    }
}

/// `node.port` splits into the node, the dot and the port.
fn push_ref(w: &str, span: Span, out: &mut Vec<(Span, Role)>) {
    match w.split_once('.') {
        Some((node, _)) => {
            let dot = span.start + node.len();
            out.push((span.start..dot, Role::Ref));
            out.push((dot..dot + 1, Role::Punct));
            out.push((dot + 1..span.end, Role::Port(node.to_string())));
        }
        None => out.push((span, Role::Ref)),
    }
}

/// Comments sit in the gaps between tokens.
fn comments(src: &str, gap: Span, out: &mut Vec<(Span, Role)>) {
    let Some(text) = src.get(gap.clone()) else { return };
    let at = text.find('#').or_else(|| text.find("//"));
    if let Some(at) = at {
        let start = gap.start + at;
        let end = src[start..gap.end].find('\n').map_or(gap.end, |n| start + n);
        out.push((start..end, Role::Comment));
    }
}

/// What the cursor wants completed.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub want: Want,
    /// The text the chosen item replaces.
    pub replace: Span,
    /// What is typed so far (the replaced text).
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Want {
    /// Start of a statement: keywords, or ids to start an edge from.
    Statement,
    /// A node or group id.
    Ref,
    Port(String),
    Stencil,
    Pack,
    Class,
    Key(Owner),
    Value(Owner, String),
}

/// The completion context at `offset`, or `None` inside strings, comments
/// and places nothing can go.
pub fn complete_at(src: &str, offset: usize) -> Option<Completion> {
    let offset = offset.min(src.len());
    if !src.is_char_boundary(offset) {
        return None;
    }
    let b = src.as_bytes();
    let mut start = offset;
    while start > 0 {
        let c = b[start - 1];
        if c == b'_' || c == b'-' || c == b'.' || c.is_ascii_alphanumeric() || c >= 0x80 {
            start -= 1;
        } else {
            break;
        }
    }
    while start < offset && b[start] == b'.' {
        start += 1;
    }
    let word = &src[start..offset];
    if word.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return None;
    }
    // Lex the text up to the word plus a placeholder ident, then ask what
    // role that ident would have.
    const HOLE: &str = "zq";
    let mut probe = String::with_capacity(start + word.len() + HOLE.len());
    probe.push_str(&src[..start]);
    probe.push_str(word);
    if word.is_empty() || word.ends_with('.') {
        probe.push_str(HOLE);
    }
    let toks = lex(&probe);
    let last = toks.last()?;
    if !matches!(last.tok, Tok::Ident(_)) || last.span.start != start {
        return None;
    }
    // A class after ` .`: the word itself has no dot.
    let roles = roles(&probe);
    let (span, role) = roles.into_iter().rev().find(|(s, _)| s.end == probe.len())?;
    let cut = |s: Span| s.start..s.end.min(offset);
    let (want, replace) = match role {
        Role::Keyword => (Want::Statement, start..offset),
        Role::Def if def_at_start(&toks) => (Want::Statement, start..offset),
        Role::Def => return None,
        Role::Ref => (Want::Ref, start..offset),
        Role::Port(node) => (Want::Port(node), cut(span)),
        Role::Stencil => (Want::Stencil, start..offset),
        Role::Pack => (Want::Pack, start..offset),
        Role::Class => (Want::Class, start..offset),
        Role::Key(owner) => (Want::Key(owner), start..offset),
        Role::Value(owner, key) => (Want::Value(owner, key), start..offset),
        _ => return None,
    };
    let prefix = src[replace.clone()].to_string();
    Some(Completion { want, replace, prefix })
}

/// The last token begins a line outside any block.
fn def_at_start(toks: &[Token]) -> bool {
    let mut depth = 0i32;
    for t in &toks[..toks.len() - 1] {
        match t.tok {
            Tok::LBrace | Tok::LBracket => depth += 1,
            Tok::RBrace | Tok::RBracket => depth -= 1,
            _ => {}
        }
    }
    depth <= 0 && toks.len().checked_sub(2).is_none_or(|p| toks[p].tok == Tok::Newline)
}

/// Brace blocks that span lines, as zero-based `(first, last)` line pairs.
pub fn folds(src: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut line = 0;
    for t in lex(src) {
        match t.tok {
            Tok::Newline => line += 1,
            Tok::LBrace => open.push(line),
            Tok::RBrace => {
                if let Some(start) = open.pop()
                    && line > start
                {
                    out.push((start, line));
                }
            }
            _ => {}
        }
    }
    out.sort();
    out
}
