//! Other diagram formats -> `.gph` source: mermaid, draw.io, SysML v2 text
//! and Visio `.vsdx`.
//!
//! Importers fill a small intermediate [`Out`] and print it, so every
//! format produces the same tidy layout: header, styles, nodes, groups,
//! edges, then geometry.

mod drawio;
mod mermaid;
mod sysml2;
mod vsdx;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use graphing_dsl::{fmt_num, fmt_str};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("unsupported diagram type `{0}`")]
    Unsupported(String),
    #[error("{0}")]
    Parse(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub source: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mermaid,
    DrawIo,
    /// SysML v2 textual notation.
    Sysml2,
    /// Visio `.vsdx` (binary; see [`import_file_as`]).
    Visio,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Mermaid, Format::DrawIo, Format::Sysml2, Format::Visio];

    /// How menus and errors name it.
    pub fn title(self) -> &'static str {
        match self {
            Format::Mermaid => "Mermaid",
            Format::DrawIo => "draw.io",
            Format::Sysml2 => "SysML v2",
            Format::Visio => "Visio",
        }
    }

    /// Its id in actions and settings (`mermaid`, `drawio`, `sysml`, `visio`).
    pub fn id(self) -> &'static str {
        match self {
            Format::Mermaid => "mermaid",
            Format::DrawIo => "drawio",
            Format::Sysml2 => "sysml",
            Format::Visio => "visio",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Format::ALL.into_iter().find(|f| f.id() == id)
    }
}

/// Guess the format from extension, then content.
pub fn detect(path: &Path, content: &str) -> Option<Format> {
    match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("mmd" | "mermaid") => return Some(Format::Mermaid),
        Some("drawio" | "dio") => return Some(Format::DrawIo),
        Some("sysml" | "kerml") => return Some(Format::Sysml2),
        _ => {}
    }
    let t = content.trim_start();
    if t.starts_with('<') && (t.contains("<mxfile") || t.contains("<mxGraphModel")) {
        return Some(Format::DrawIo);
    }
    let first = t.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("%%") && !l.starts_with("```") && !l.starts_with("//"))?;
    if first.starts_with("package ") || first.starts_with("part def ") {
        return Some(Format::Sysml2);
    }
    let kw = first.split_whitespace().next()?;
    matches!(kw, "graph" | "flowchart" | "stateDiagram" | "stateDiagram-v2").then_some(Format::Mermaid)
}

pub fn import(format: Format, content: &str) -> Result<Imported, ImportError> {
    match format {
        Format::Mermaid => from_mermaid(content),
        Format::DrawIo => from_drawio(content),
        Format::Sysml2 => from_sysml2(content),
        Format::Visio => Err(ImportError::Parse("Visio files are binary; use import_file_as".into())),
    }
}

/// SysML v2 text (the subset graphing exports, and common hand-written
/// forms of it).
pub fn from_sysml2(src: &str) -> Result<Imported, ImportError> {
    sysml2::parse(src)
}

/// Import a file of any supported format: `.vsdx` by extension or its zip
/// header, the text formats by [`detect`].
pub fn import_file(path: &Path) -> Result<Imported, ImportError> {
    import_file_as(path, None)
}

/// Import `path` as `format`, or as whatever it looks like with `None`.
pub fn import_file_as(path: &Path, format: Option<Format>) -> Result<Imported, ImportError> {
    let bytes = std::fs::read(path).map_err(|e| ImportError::Parse(e.to_string()))?;
    let zipped = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("vsdx")) || bytes.starts_with(b"PK\x03\x04");
    let format = match format {
        Some(f) => f,
        None if zipped => Format::Visio,
        None => detect(path, &String::from_utf8_lossy(&bytes)).ok_or_else(|| ImportError::Parse("not a mermaid, draw.io, SysML v2 or Visio file".into()))?,
    };
    match format {
        Format::Visio => from_vsdx(&bytes),
        text => import(text, &String::from_utf8_lossy(&bytes)),
    }
}

/// Visio `.vsdx` (binary: a zip of XML).
pub fn from_vsdx(bytes: &[u8]) -> Result<Imported, ImportError> {
    vsdx::parse(bytes).map(Out::finish)
}

pub fn from_mermaid(src: &str) -> Result<Imported, ImportError> {
    mermaid::parse(src).map(Out::finish)
}

pub fn from_drawio(src: &str) -> Result<Imported, ImportError> {
    drawio::parse(src).map(Out::finish)
}

// ---- intermediate form ----

#[derive(Debug, Clone, Default)]
pub(crate) struct ONode {
    pub id: String,
    pub stencil: Option<&'static str>,
    pub label: Option<String>,
    pub classes: Vec<String>,
    pub props: Vec<(String, String)>,
    pub geom: Option<(f64, f64, f64, f64)>,
}

#[derive(Debug, Clone)]
pub(crate) struct OEdge {
    pub from: String,
    pub to: String,
    pub arrow: &'static str,
    pub label: Option<String>,
    pub props: Vec<(String, String)>,
    pub via: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OGroup {
    pub id: String,
    pub label: Option<String>,
    pub members: Vec<String>,
    pub props: Vec<(String, String)>,
    pub geom: Option<(f64, f64, f64, f64)>,
}

/// Props hold printed values (already `#hex`, numbers or idents).
#[derive(Debug, Clone, Default)]
pub(crate) struct Out {
    pub title: Option<String>,
    pub styles: Vec<(String, Vec<(String, String)>)>,
    pub nodes: Vec<ONode>,
    pub groups: Vec<OGroup>,
    pub edges: Vec<OEdge>,
    pub warnings: Vec<String>,
    index: HashMap<String, usize>,
}

const KEYWORDS: &[&str] = &["diagram", "use", "style", "group", "layout", "via"];

/// Make any string a valid graphing identifier.
pub(crate) fn ident(raw: &str) -> String {
    let mut s: String = raw.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    s = s.trim_matches('-').to_string();
    if s.is_empty() || !s.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        s.insert(0, '_');
    }
    if KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

impl Out {
    /// Node by id, created on first mention.
    pub fn node(&mut self, id: &str) -> &mut ONode {
        let i = match self.index.get(id) {
            Some(&i) => i,
            None => {
                self.nodes.push(ONode { id: id.to_string(), ..Default::default() });
                self.index.insert(id.to_string(), self.nodes.len() - 1);
                self.nodes.len() - 1
            }
        };
        &mut self.nodes[i]
    }

    fn finish(self) -> Imported {
        let mut s = String::new();
        if let Some(t) = &self.title {
            let _ = writeln!(s, "diagram {}\n", fmt_str(t));
        }
        for (name, props) in &self.styles {
            let _ = writeln!(s, "style {name} {}", props_text(props));
        }
        if !self.styles.is_empty() {
            s.push('\n');
        }
        for n in &self.nodes {
            let mut line = n.id.clone();
            let mut tail = String::new();
            if let Some(l) = &n.label
                && l != &n.id
            {
                let _ = write!(tail, " {}", fmt_str(l));
            }
            for c in &n.classes {
                let _ = write!(tail, " .{c}");
            }
            if !n.props.is_empty() {
                let _ = write!(tail, " {}", props_text(&n.props));
            }
            match n.stencil {
                Some(st) => {
                    let _ = write!(line, ": {st}{tail}");
                }
                None if !tail.is_empty() => {
                    let _ = write!(line, ":{tail}");
                }
                None => {}
            }
            s.push_str(&line);
            s.push('\n');
        }
        if !self.groups.is_empty() {
            s.push('\n');
        }
        for g in &self.groups {
            let label = g.label.as_ref().map(|l| format!(" {}", fmt_str(l))).unwrap_or_default();
            let props = if g.props.is_empty() { String::new() } else { format!(" {}", props_text(&g.props)) };
            let _ = writeln!(s, "group {}{label} {{ {} }}{props}", g.id, g.members.join(" "));
        }
        if !self.edges.is_empty() {
            s.push('\n');
        }
        let mut seen: HashMap<(String, String), usize> = HashMap::new();
        let mut via_lines = Vec::new();
        for e in &self.edges {
            let label = e.label.as_ref().map(|l| format!(" {}", fmt_str(l))).unwrap_or_default();
            let props = if e.props.is_empty() { String::new() } else { format!(" {}", props_text(&e.props)) };
            let _ = writeln!(s, "{} {} {}{label}{props}", e.from, e.arrow, e.to);
            let count = seen.entry((e.from.clone(), e.to.clone())).or_insert(0);
            *count += 1;
            // Only the first unnamed edge between two nodes is addressable.
            if !e.via.is_empty() && *count == 1 {
                let pts: Vec<String> = e.via.iter().map(|(x, y)| format!("{} {}", fmt_num(*x), fmt_num(*y))).collect();
                via_lines.push(format!("  {} -> {} via {}", e.from, e.to, pts.join(", ")));
            }
        }
        let geoms: Vec<String> = self
            .groups
            .iter()
            .filter_map(|g| g.geom.map(|gm| (&g.id, gm)))
            .chain(self.nodes.iter().filter_map(|n| n.geom.map(|gm| (&n.id, gm))))
            .map(|(id, (x, y, w, h))| format!("  {id} {} {} {}x{}", fmt_num(x), fmt_num(y), fmt_num(w), fmt_num(h)))
            .collect();
        if !geoms.is_empty() || !via_lines.is_empty() {
            s.push_str("\nlayout {\n");
            for l in geoms.iter().chain(&via_lines) {
                s.push_str(l);
                s.push('\n');
            }
            s.push_str("}\n");
        }
        Imported { source: s, warnings: self.warnings }
    }
}

fn props_text(props: &[(String, String)]) -> String {
    let inner: Vec<String> = props.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    format!("{{ {} }}", inner.join(", "))
}

/// A color or plain value as graphing prints it: `#hex`, number, ident or
/// quoted string.
pub(crate) fn value(raw: &str) -> String {
    let v = raw.trim();
    if v.starts_with('#') && v.len() > 1 && v[1..].chars().all(|c| c.is_ascii_hexdigit()) {
        return v.to_lowercase();
    }
    let num = v.trim_end_matches("px");
    if num.parse::<f64>().is_ok() {
        return num.to_string();
    }
    if !v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') && v.starts_with(|c: char| c.is_alphabetic()) {
        return v.to_string();
    }
    fmt_str(v)
}

#[cfg(test)]
pub(crate) fn assert_clean(src: &str) -> graphing_model::Diagram {
    let doc = graphing_dsl::Document::parse(src);
    assert!(doc.diags().is_empty(), "diags {:?} in\n{src}", doc.diags());
    doc.diagram().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats() {
        assert_eq!(detect(Path::new("a.mmd"), ""), Some(Format::Mermaid));
        assert_eq!(detect(Path::new("a.txt"), "%% hi\ngraph TD\nA-->B"), Some(Format::Mermaid));
        assert_eq!(detect(Path::new("a.xml"), "<mxfile><diagram/></mxfile>"), Some(Format::DrawIo));
        assert_eq!(detect(Path::new("a.txt"), "hello"), None);
    }

    #[test]
    fn idents_are_valid() {
        assert_eq!(ident("hello world"), "hello_world");
        assert_eq!(ident("1st"), "_1st");
        assert_eq!(ident("group"), "group_");
        assert_eq!(value("#F9F"), "#f9f");
        assert_eq!(value("2px"), "2");
        assert_eq!(value("bold"), "bold");
        assert_eq!(value("a b"), "\"a b\"");
    }
}
