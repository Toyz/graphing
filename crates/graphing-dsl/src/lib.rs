//! The `.gph` text format.
//!
//! [`Document`] owns the source text and the model lowered from it. Edits come
//! in as [`Op`]s and are turned into minimal text splices, so comments,
//! ordering and formatting the user wrote are never rewritten.

pub mod ast;
mod edit;
pub mod lang;
pub mod lexer;
mod lower;
mod parser;
pub mod print;

pub use lower::Index;
pub use print::{fmt_num, fmt_str, fmt_value};

use graphing_model::{Diagram, Op};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub span: Range<usize>,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone)]
pub struct Document {
    src: String,
    file: ast::File,
    diagram: Diagram,
    index: Index,
    diags: Vec<Diag>,
}

impl Document {
    pub fn parse(src: impl Into<String>) -> Self {
        let mut doc = Self {
            src: src.into(),
            file: ast::File::default(),
            diagram: Diagram::default(),
            index: Index::default(),
            diags: Vec::new(),
        };
        doc.reparse();
        doc
    }

    pub fn source(&self) -> &str {
        &self.src
    }

    pub fn diagram(&self) -> &Diagram {
        &self.diagram
    }

    pub fn diags(&self) -> &[Diag] {
        &self.diags
    }

    pub fn ast(&self) -> &ast::File {
        &self.file
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Replace the whole text (the user typed in the source view).
    pub fn set_source(&mut self, src: impl Into<String>) {
        self.src = src.into();
        self.reparse();
    }

    /// Apply an edit to both the model and the text. Returns the inverse op,
    /// or `None` if the op does not apply to the current model.
    pub fn apply(&mut self, op: &Op) -> Option<Op> {
        let mut probe = self.diagram.clone();
        let inverse = probe.apply(op)?;
        self.patch(op);
        Some(inverse)
    }

    fn reparse(&mut self) {
        let (file, mut diags) = parser::parse(&self.src);
        let (diagram, index) = lower::lower(&file, &mut diags);
        self.file = file;
        self.diagram = diagram;
        self.index = index;
        self.diags = diags;
    }
}

#[cfg(test)]
mod tests;
