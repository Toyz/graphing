use crate::ast::*;
use crate::lexer::{Span, Tok, Token, lex};
use crate::{Diag, Severity};
use graphing_model::{Arrow, Value};

/// Parse never fails; problems become diagnostics and the offending line is
/// kept as [`StmtKind::Unknown`].
pub fn parse(src: &str) -> (File, Vec<Diag>) {
    let mut p = Parser { toks: lex(src), pos: 0, diags: Vec::new() };
    let mut file = File::default();
    loop {
        p.skip_newlines();
        if p.peek().is_none() {
            break;
        }
        let start = p.pos;
        let kind = p.stmt();
        let kind = match kind {
            Some(k) if p.at_eol() => k,
            Some(_) | None => {
                let at = p.toks.get(p.pos).or(p.toks.last()).map(|t| t.span.clone()).unwrap_or(0..0);
                p.diag(at, "unexpected input, line kept as is");
                p.pos = start;
                p.skip_stmt();
                StmtKind::Unknown
            }
        };
        let span = p.toks[start].span.start..p.toks[p.pos - 1].span.end;
        file.stmts.push(Stmt { kind, span });
    }
    (file, p.diags)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diag>,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.pos + n).map(|t| &t.tok)
    }

    fn span(&self) -> Span {
        self.toks.get(self.pos).map(|t| t.span.clone()).unwrap_or(0..0)
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        self.pos += 1;
        t
    }

    fn at_eol(&self) -> bool {
        matches!(self.peek(), None | Some(Tok::Newline))
    }

    fn skip_newlines(&mut self) {
        while self.peek() == Some(&Tok::Newline) {
            self.pos += 1;
        }
    }

    /// Skip a broken statement from its first token: through its blocks when
    /// they close, so their lines do not read as statements of their own;
    /// only its first line when a block never closes (mid-typing), so the
    /// rest of the file still counts.
    fn skip_stmt(&mut self) {
        let start = self.pos;
        self.skip_line();
        if self.peek().is_none() && self.toks[start..].iter().filter(|t| t.tok == Tok::LBrace).count() > self.toks[start..].iter().filter(|t| t.tok == Tok::RBrace).count() {
            self.pos = start;
            while !self.at_eol() {
                self.pos += 1;
            }
        }
    }

    /// Whether a `{` comes next, on this line or after blank lines (a
    /// statement never starts with `{`, so one on its own line belongs to
    /// the statement above). Moves onto it.
    fn brace_next(&mut self) -> bool {
        let mut at = self.pos;
        while self.toks.get(at).is_some_and(|t| t.tok == Tok::Newline) {
            at += 1;
        }
        let found = self.toks.get(at).is_some_and(|t| t.tok == Tok::LBrace);
        if found {
            self.pos = at;
        }
        found
    }

    /// Whether the token `n` ahead, past line breaks, is a `{`.
    fn brace_at(&self, n: usize) -> bool {
        self.toks[self.pos + n..].iter().find(|t| t.tok != Tok::Newline).is_some_and(|t| t.tok == Tok::LBrace)
    }

    /// Skip to end of line, treating braces as nesting so a broken block is
    /// swallowed whole.
    fn skip_line(&mut self) {
        let mut depth = 0usize;
        while let Some(t) = self.peek() {
            match t {
                Tok::Newline if depth == 0 => break,
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn diag(&mut self, span: Span, msg: &str) {
        self.diags.push(Diag { span, message: msg.to_string(), severity: Severity::Warning });
    }

    fn ident(&mut self) -> Option<Spanned<String>> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let value = s.clone();
                let span = self.bump().span;
                Some(Spanned { value, span })
            }
            _ => None,
        }
    }

    /// `a.b.c`
    fn path(&mut self) -> Option<Spanned<String>> {
        let mut first = self.ident()?;
        // `a.b` only when tight; `a .b` is a stencil followed by a class.
        while self.peek() == Some(&Tok::Dot)
            && matches!(self.peek_at(1), Some(Tok::Ident(_)))
            && self.toks[self.pos].span.start == first.span.end
            && self.toks[self.pos + 1].span.start == first.span.end + 1
        {
            self.pos += 1;
            let next = self.ident()?;
            first.value.push('.');
            first.value.push_str(&next.value);
            first.span.end = next.span.end;
        }
        Some(first)
    }

    fn string(&mut self) -> Option<Spanned<String>> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                let value = s.clone();
                let span = self.bump().span;
                Some(Spanned { value, span })
            }
            _ => None,
        }
    }

    fn classes(&mut self) -> Vec<Spanned<String>> {
        let mut out = Vec::new();
        while self.peek() == Some(&Tok::Dot) && matches!(self.peek_at(1), Some(Tok::Ident(_))) {
            let dot = self.bump().span;
            let mut c = self.ident().expect("checked");
            c.span.start = dot.start;
            out.push(c);
        }
        out
    }

    fn arrow(&self) -> Option<Arrow> {
        Some(match self.peek()? {
            Tok::Fwd => Arrow::Forward,
            Tok::Back => Arrow::Back,
            Tok::Both => Arrow::Both,
            Tok::Line => Arrow::None,
            _ => return None,
        })
    }

    fn stmt(&mut self) -> Option<StmtKind> {
        let Tok::Ident(word) = self.peek()?.clone() else {
            return None;
        };
        let next = self.peek_at(1);
        let keyword = !matches!(next, Some(Tok::Colon | Tok::Fwd | Tok::Back | Tok::Both | Tok::Line));
        if keyword {
            match word.as_str() {
                "diagram" => {
                    self.pos += 1;
                    let title = self.string();
                    let props = if self.brace_next() { Some(self.props()?) } else { None };
                    return Some(StmtKind::Diagram { title, props });
                }
                "use" => {
                    self.pos += 1;
                    let mut packs = Vec::new();
                    loop {
                        let pack = self.path()?;
                        let alias = if matches!(self.peek(), Some(Tok::Ident(w)) if w == "as") {
                            self.pos += 1;
                            Some(self.ident()?)
                        } else {
                            None
                        };
                        packs.push((pack, alias));
                        if self.peek() != Some(&Tok::Comma) {
                            break;
                        }
                        self.pos += 1;
                    }
                    return Some(StmtKind::Use { packs });
                }
                "style" if matches!(next, Some(Tok::Ident(_))) => {
                    self.pos += 1;
                    let name = self.ident()?;
                    if !self.brace_next() {
                        return None;
                    }
                    let props = self.props()?;
                    return Some(StmtKind::Style { name, props });
                }
                "group" if matches!(next, Some(Tok::Ident(_))) => {
                    self.pos += 1;
                    return self.group().map(StmtKind::Group);
                }
                "layout" if self.brace_at(1) => {
                    self.pos += 1;
                    self.brace_next();
                    return self.layout().map(StmtKind::Layout);
                }
                "animate" if self.brace_at(1) => {
                    self.pos += 1;
                    self.brace_next();
                    return self.animate().map(StmtKind::Animate);
                }
                _ => {}
            }
        }
        // `a.port -> b` starts an edge at a port.
        if self.edge_ahead(0) {
            let from = self.endpoint()?;
            return self.edge(None, from).map(StmtKind::Edge);
        }
        let first = self.ident()?;
        if self.peek() == Some(&Tok::Colon) {
            self.pos += 1;
            // `id: a -> b` names an edge.
            if self.edge_ahead(0) {
                let from = self.endpoint()?;
                return self.edge(Some(first), from).map(StmtKind::Edge);
            }
            return Some(StmtKind::Node(self.node(first)));
        }
        // Lenient: `id` or `id "label"` declares a node.
        Some(StmtKind::Node(self.node(first)))
    }

    /// Tight `.ident` right after the token before `n` (no spaces around the dot).
    fn tight_port_at(&self, n: usize) -> bool {
        let (Some(prev), Some(dot), Some(name)) =
            (self.toks.get(self.pos + n), self.toks.get(self.pos + n + 1), self.toks.get(self.pos + n + 2))
        else {
            return false;
        };
        dot.tok == Tok::Dot && matches!(name.tok, Tok::Ident(_)) && dot.span.start == prev.span.end && name.span.start == dot.span.end
    }

    /// An endpoint (`a` or `a.port`) at offset `n` followed by an arrow.
    fn edge_ahead(&self, n: usize) -> bool {
        if !matches!(self.peek_at(n), Some(Tok::Ident(_))) {
            return false;
        }
        let after = if self.tight_port_at(n) { n + 3 } else { n + 1 };
        matches!(self.peek_at(after), Some(Tok::Fwd | Tok::Back | Tok::Both | Tok::Line))
    }

    /// `node` or `node.port`, kept as written.
    fn endpoint(&mut self) -> Option<Spanned<String>> {
        let tight = self.tight_port_at(0);
        let mut e = self.ident()?;
        if tight {
            self.pos += 1;
            let port = self.ident()?;
            e.value.push('.');
            e.value.push_str(&port.value);
            e.span.end = port.span.end;
        }
        Some(e)
    }

    fn node(&mut self, id: Spanned<String>) -> NodeDecl {
        let stencil = self.path();
        let label = self.string();
        let classes = self.classes();
        let props = if self.brace_next() { self.props() } else { None };
        NodeDecl { id, stencil, label, classes, props }
    }

    fn edge(&mut self, id: Option<Spanned<String>>, from: Spanned<String>) -> Option<EdgeDecl> {
        let mut chain = vec![from];
        let mut arrows = Vec::new();
        let mut arrow_spans = Vec::new();
        while let Some(a) = self.arrow() {
            arrow_spans.push(self.bump().span);
            arrows.push(a);
            chain.push(self.endpoint()?);
        }
        if let Some(id) = &id
            && chain.len() > 2
        {
            let span = id.span.clone();
            self.diag(span, "a named edge must have exactly one hop");
        }
        let label = self.string();
        let classes = self.classes();
        let props = if self.brace_next() { Some(self.props()?) } else { None };
        Some(EdgeDecl { id, chain, arrows, arrow_spans, label, classes, props })
    }

    fn group(&mut self) -> Option<GroupDecl> {
        let id = self.ident()?;
        let label = self.string();
        if !self.brace_next() {
            return None;
        }
        let open = self.bump().span;
        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek()? {
                Tok::RBrace => break,
                Tok::Comma => self.pos += 1,
                Tok::Ident(_) => members.push(self.ident()?),
                _ => return None,
            }
        }
        let close = self.bump().span;
        let props = if self.brace_next() { Some(self.props()?) } else { None };
        Some(GroupDecl { id, label, members, body: open.start..close.end, props })
    }

    /// `{ key: value, key: value }`, commas or newlines between entries.
    fn props(&mut self) -> Option<PropBlock> {
        if self.peek() != Some(&Tok::LBrace) {
            return None;
        }
        let open = self.bump().span;
        let mut entries = Vec::new();
        loop {
            match self.peek()? {
                Tok::Newline | Tok::Comma => self.pos += 1,
                Tok::RBrace => break,
                Tok::Ident(_) => {
                    let key = self.ident()?;
                    if self.peek() != Some(&Tok::Colon) {
                        return None;
                    }
                    self.pos += 1;
                    let value = self.value()?;
                    entries.push(PropEntry { key, value });
                }
                _ => return None,
            }
        }
        let close = self.bump().span;
        Some(PropBlock { span: open.start..close.end, entries })
    }

    /// A prop value: string, number, color, ident, or `[v, v, ...]`.
    fn value(&mut self) -> Option<Spanned<Value>> {
        let t = self.toks.get(self.pos)?.clone();
        self.pos += 1;
        let value = match t.tok {
            Tok::Str(s) => Value::Str(s),
            Tok::Num(n) => Value::Num(n),
            Tok::Color(c) => Value::Color(c),
            Tok::Ident(i) => {
                // Dotted idents like `sysml.flow` read as one value.
                let mut v = i;
                let mut end = t.span.end;
                while self.peek() == Some(&Tok::Dot) && self.toks[self.pos].span.start == end {
                    let Some(Tok::Ident(next)) = self.peek_at(1).cloned() else { break };
                    self.pos += 2;
                    v.push('.');
                    v.push_str(&next);
                    end = self.toks[self.pos - 1].span.end;
                }
                return Some(Spanned { value: Value::Ident(v), span: t.span.start..end });
            }
            Tok::LBracket => {
                let mut items = Vec::new();
                loop {
                    match self.peek()? {
                        Tok::Newline | Tok::Comma => self.pos += 1,
                        Tok::RBracket => break,
                        _ => items.push(self.value()?.value),
                    }
                }
                let close = self.bump().span;
                return Some(Spanned { value: Value::List(items), span: t.span.start..close.end });
            }
            _ => return None,
        };
        Some(Spanned { value, span: t.span })
    }

    fn num(&mut self) -> Option<f64> {
        match self.peek() {
            Some(Tok::Num(n)) => {
                let n = *n;
                self.pos += 1;
                Some(n)
            }
            _ => None,
        }
    }

    fn layout(&mut self) -> Option<LayoutBlock> {
        let open = self.bump().span;
        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek()? {
                Tok::RBrace => break,
                Tok::Ident(_) | Tok::Str(_) => {
                    let start = self.pos;
                    match self.layout_entry() {
                        Some(e) if self.at_eol() || self.peek() == Some(&Tok::RBrace) => entries.push(e),
                        _ => {
                            self.pos = start;
                            let at = self.span();
                            self.diag(at, "bad layout entry, ignored");
                            while !self.at_eol() && self.peek() != Some(&Tok::RBrace) {
                                self.pos += 1;
                            }
                        }
                    }
                }
                _ => {
                    let at = self.span();
                    self.diag(at, "bad layout entry, ignored");
                    while !self.at_eol() && self.peek() != Some(&Tok::RBrace) {
                        self.pos += 1;
                    }
                }
            }
        }
        let close = self.bump().span;
        Some(LayoutBlock { open, close, entries })
    }

    fn layout_entry(&mut self) -> Option<LayoutEntry> {
        // `"a->b#2" via ...` names a parallel edge by its key.
        if let Some(key) = self.string() {
            return self.layout_geometry(LayoutTarget::Id(key.value), key.span);
        }
        let first = self.endpoint()?;
        let node = |s: String| s.split('.').next().unwrap_or_default().to_string();
        let (target, end) = if self.arrow().is_some() {
            self.pos += 1;
            let to = self.endpoint()?;
            (LayoutTarget::Edge(node(first.value), node(to.value)), to.span.end)
        } else {
            (LayoutTarget::Id(first.value), first.span.end)
        };
        self.layout_geometry(target, first.span.start..end)
    }

    /// The numbers after a layout entry's target: `x y [WxH] [via x y, ...]`.
    fn layout_geometry(&mut self, target: LayoutTarget, target_span: Span) -> Option<LayoutEntry> {
        let start = target_span.start;
        let mut entry =
            LayoutEntry { target, target_span, span: start..0, geom: None, pos: None, size: None, via_span: None, via: Vec::new() };
        if let Some(Tok::Num(_)) = self.peek() {
            let gstart = self.span().start;
            let x = self.num()?;
            let y = self.num()?;
            entry.pos = Some((x, y));
            if let Some(Tok::Size(w, h)) = self.peek() {
                entry.size = Some((*w, *h));
                self.pos += 1;
            }
            entry.geom = Some(gstart..self.toks[self.pos - 1].span.end);
        }
        if matches!(self.peek(), Some(Tok::Ident(w)) if w == "via") {
            let vstart = self.bump().span.start;
            loop {
                let x = self.num()?;
                let y = self.num()?;
                entry.via.push((x, y));
                if self.peek() == Some(&Tok::Comma) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            entry.via_span = Some(vstart..self.toks[self.pos - 1].span.end);
        }
        entry.span.end = self.toks[self.pos - 1].span.end;
        Some(entry)
    }

    fn animate(&mut self) -> Option<AnimateBlock> {
        let open = self.bump().span;
        let mut steps = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek()? {
                Tok::RBrace => break,
                Tok::Ident(w) if w == "step" => {
                    let start = self.pos;
                    match self.step() {
                        Some(st) if self.at_eol() || self.peek() == Some(&Tok::RBrace) => steps.push(st),
                        _ => {
                            self.pos = start;
                            let at = self.span();
                            self.diag(at, "bad step, ignored");
                            self.skip_line();
                        }
                    }
                }
                _ => {
                    let at = self.span();
                    self.diag(at, "expected `step`, ignored");
                    while !self.at_eol() && self.peek() != Some(&Tok::RBrace) {
                        self.pos += 1;
                    }
                }
            }
        }
        let close = self.bump().span;
        Some(AnimateBlock { open, close, steps })
    }

    /// `step ["title"] [2s | 500ms] { action lines }`
    fn step(&mut self) -> Option<StepDecl> {
        let start = self.bump().span.start;
        let title = self.string();
        let mut seconds = None;
        if let Some(Tok::Num(n)) = self.peek() {
            let mut n = *n;
            let end = self.bump().span.end;
            if let Some(Tok::Ident(unit)) = self.peek()
                && self.span().start == end
            {
                n = match unit.as_str() {
                    "s" => n,
                    "ms" => n / 1000.0,
                    _ => return None,
                };
                self.pos += 1;
            }
            seconds = Some(n);
        }
        if !self.brace_next() {
            return None;
        }
        self.pos += 1;
        let (mut actions, mut ease, mut moves) = (Vec::new(), None, Vec::new());
        loop {
            self.skip_newlines();
            match self.peek()? {
                Tok::RBrace => break,
                Tok::Ident(w) if w == "ease" => {
                    self.pos += 1;
                    ease = Some(self.ident()?);
                }
                Tok::Ident(w) if w == "move" => {
                    let start = self.bump().span.start;
                    let target = self.endpoint()?;
                    let x = self.num()?;
                    let y = self.num()?;
                    moves.push(MoveDecl { target, to: (x, y), span: start..self.toks[self.pos - 1].span.end });
                }
                Tok::Ident(_) => actions.push(self.action()?),
                _ => return None,
            }
            if !self.at_eol() && self.peek() != Some(&Tok::RBrace) {
                return None;
            }
        }
        let end = self.bump().span.end;
        Some(StepDecl { span: start..end, title, seconds, actions, ease, moves })
    }

    /// `verb target, target` where a target is an id or `a -> b`.
    fn action(&mut self) -> Option<ActionDecl> {
        let verb = self.ident()?;
        let node = |s: &str| s.split('.').next().unwrap_or_default().to_string();
        let mut targets = Vec::new();
        while matches!(self.peek(), Some(Tok::Ident(_))) {
            let first = self.endpoint()?;
            let target = if self.arrow().is_some() {
                self.pos += 1;
                let to = self.endpoint()?;
                Spanned { value: format!("{}->{}", node(&first.value), node(&to.value)), span: first.span.start..to.span.end }
            } else {
                first
            };
            targets.push(target);
            if self.peek() == Some(&Tok::Comma) {
                self.pos += 1;
            }
        }
        Some(ActionDecl { verb, targets })
    }
}
