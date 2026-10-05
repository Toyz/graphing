//! Syntax tree with byte spans for every piece an edit may touch.

use crate::lexer::Span;
use graphing_model::{Arrow, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct File {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    /// First token start to last token end.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Diagram { title: Option<Spanned<String>>, props: Option<PropBlock> },
    /// `use c4, sysml as s`: packs, each with an optional short name.
    Use { packs: Vec<(Spanned<String>, Option<Spanned<String>>)> },
    Style { name: Spanned<String>, props: PropBlock },
    Node(NodeDecl),
    Edge(EdgeDecl),
    Group(GroupDecl),
    Layout(LayoutBlock),
    Animate(AnimateBlock),
    /// Kept verbatim; reported as a warning.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct NodeDecl {
    pub id: Spanned<String>,
    pub stencil: Option<Spanned<String>>,
    pub label: Option<Spanned<String>>,
    pub classes: Vec<Spanned<String>>,
    pub props: Option<PropBlock>,
}

#[derive(Debug, Clone)]
pub struct EdgeDecl {
    pub id: Option<Spanned<String>>,
    /// Endpoints in order, as written (`node` or `node.port`); `arrows[i]`
    /// joins `chain[i]` and `chain[i + 1]`.
    pub chain: Vec<Spanned<String>>,
    pub arrows: Vec<Arrow>,
    pub arrow_spans: Vec<Span>,
    pub label: Option<Spanned<String>>,
    pub classes: Vec<Spanned<String>>,
    pub props: Option<PropBlock>,
}

#[derive(Debug, Clone)]
pub struct GroupDecl {
    pub id: Spanned<String>,
    pub label: Option<Spanned<String>>,
    pub members: Vec<Spanned<String>>,
    /// Span of the `{ ... }` member list, braces included.
    pub body: Span,
    pub props: Option<PropBlock>,
}

#[derive(Debug, Clone)]
pub struct PropBlock {
    /// Braces included.
    pub span: Span,
    pub entries: Vec<PropEntry>,
}

#[derive(Debug, Clone)]
pub struct PropEntry {
    pub key: Spanned<String>,
    pub value: Spanned<Value>,
}

#[derive(Debug, Clone)]
pub struct LayoutBlock {
    pub open: Span,
    pub close: Span,
    pub entries: Vec<LayoutEntry>,
}

#[derive(Debug, Clone)]
pub enum LayoutTarget {
    /// Node or group id, or a named edge.
    Id(String),
    /// `a -> b` style reference to an unnamed edge.
    Edge(String, String),
}

#[derive(Debug, Clone)]
pub struct LayoutEntry {
    pub target: LayoutTarget,
    /// The target as written: `a`, `a -> b` or `"a->b#2"`.
    pub target_span: Span,
    /// Whole entry, first token to last.
    pub span: Span,
    /// `x y [WxH]` tokens, if present.
    pub geom: Option<Span>,
    pub pos: Option<(f64, f64)>,
    pub size: Option<(f64, f64)>,
    /// `via x y, x y` including the keyword.
    pub via_span: Option<Span>,
    pub via: Vec<(f64, f64)>,
}

/// `animate { step "Title" 2s { show a, b  flow a -> b } }`
#[derive(Debug, Clone)]
pub struct AnimateBlock {
    pub open: Span,
    pub close: Span,
    pub steps: Vec<StepDecl>,
}

#[derive(Debug, Clone)]
pub struct StepDecl {
    /// `step` through its closing brace.
    pub span: Span,
    pub title: Option<Spanned<String>>,
    pub seconds: Option<f64>,
    pub actions: Vec<ActionDecl>,
    /// `ease snappy`
    pub ease: Option<Spanned<String>>,
    /// `move a 300 120`
    pub moves: Vec<MoveDecl>,
}

#[derive(Debug, Clone)]
pub struct MoveDecl {
    pub target: Spanned<String>,
    pub to: (f64, f64),
    /// `move` through the last number.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ActionDecl {
    pub verb: Spanned<String>,
    /// Ids, or `a->b` for an edge written `a -> b`.
    pub targets: Vec<Spanned<String>>,
}
