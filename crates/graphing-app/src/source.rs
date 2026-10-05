//! Source panel language support: highlighting, live diagnostics and
//! completions for `.gph` text. The token logic is in `graphing_dsl::lang`;
//! this file adapts it to the editor and fills in names from the registry
//! and the open document.

use std::ops::Range;
use std::rc::Rc;

use anyhow::Result;
use gpui_kit::base::input::{
    CompletionProvider, Diagnostic, DiagnosticSeverity, EditorState, FoldRange, HighlightStyleResolver,
    InputEdit, InputHighlighter, Rope, RopeExt,
};
use gpui_kit::{App, Context, FontStyle, FontWeight, HighlightStyle, SharedString, Task, Window};
use graphing_dsl::lang::{self, Owner, Role, Want};
use graphing_dsl::{Document, Severity};
use graphing_model::Diagram;
use graphing_scene::stencils::registry;
use graphing_ui::UiExt;
use graphing_ui::tokens::Syntax;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionResponse,
    CompletionTextEdit, TextEdit,
};

use crate::ops;

pub const LANGUAGE: &str = "gph";

/// Make `editor` a `.gph` editor: highlighter, folds and completions.
pub fn install(editor: &mut EditorState, cx: &mut Context<EditorState>) {
    editor.set_highlighter_factory(
        Rc::new(|name: &str| (name == LANGUAGE).then(|| Box::new(Highlighter::default()) as Box<dyn InputHighlighter>)),
        cx,
    );
    editor.set_highlighter(LANGUAGE, cx);
    editor.lsp_mut().completion_provider = Some(Rc::new(Completions));
}

/// Re-highlight after a theme change.
pub fn refresh(editor: &mut EditorState, cx: &mut Context<EditorState>) {
    editor.set_highlighter(LANGUAGE, cx);
}

/// Replace the editor's diagnostics with the document's.
pub fn set_diagnostics(editor: &mut EditorState, doc: &Document, cx: &mut Context<EditorState>) {
    let text = editor.text().clone();
    let len = text.len();
    let items: Vec<Diagnostic> = doc
        .diags()
        .iter()
        .map(|d| {
            let start = d.span.start.min(len);
            // Zero-width spans (end of file) still need something to underline.
            let end = d.span.end.min(len).max((start + 1).min(len));
            let severity = match d.severity {
                Severity::Error => DiagnosticSeverity::Error,
                Severity::Warning => DiagnosticSeverity::Warning,
            };
            Diagnostic::new(text.offset_to_position(start)..text.offset_to_position(end), d.message.clone())
                .with_severity(severity)
                .with_source("graphing")
        })
        .collect();
    if let Some(set) = editor.diagnostics_mut() {
        set.reset(&text);
        set.extend(items);
    }
    cx.notify();
}

#[derive(Default)]
struct Highlighter {
    runs: Vec<(Range<usize>, HighlightStyle)>,
    folds: Vec<FoldRange>,
}

/// The style for a role, from the design system's syntax palette.
fn style(role: &Role, k: &Syntax) -> HighlightStyle {
    let color = match role {
        Role::Comment => k.comment,
        Role::Keyword => k.keyword,
        Role::Def => k.def,
        Role::Ref => k.reference,
        Role::Port(_) => k.port,
        Role::Stencil | Role::Pack => k.stencil,
        Role::Class => k.class,
        Role::Key(_) => k.key,
        Role::Value(..) => k.value,
        Role::Str => k.string,
        Role::Num => k.number,
        Role::Color => k.color,
        Role::Arrow => k.arrow,
        Role::Punct => k.punct,
        Role::Error => k.error,
    };
    HighlightStyle {
        color: Some(color),
        font_weight: matches!(role, Role::Keyword | Role::Def).then_some(FontWeight::SEMIBOLD),
        font_style: matches!(role, Role::Comment).then_some(FontStyle::Italic),
        ..Default::default()
    }
}

impl InputHighlighter for Highlighter {
    fn language(&self) -> SharedString {
        LANGUAGE.into()
    }

    fn update(&mut self, _: Option<InputEdit>, text: &Rope, _folding: bool, _: &mut Window, cx: &mut Context<EditorState>) {
        let src = text.to_string();
        let k = Syntax::for_mode(cx.ui().dark);
        self.runs = lang::roles(&src).into_iter().map(|(s, r)| (s, style(&r, &k))).collect();
        self.folds = lang::folds(&src).into_iter().map(|(start_line, end_line)| FoldRange { start_line, end_line }).collect();
    }

    fn styles(&self, range: &Range<usize>, _: &dyn HighlightStyleResolver) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut out = Vec::new();
        let mut at = range.start;
        let first = self.runs.partition_point(|(s, _)| s.end <= range.start);
        for (span, style) in &self.runs[first..] {
            if span.start >= range.end {
                break;
            }
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            if start > at {
                out.push((at..start, HighlightStyle::default()));
            }
            out.push((start..end, *style));
            at = end;
        }
        if at < range.end {
            out.push((at..range.end, HighlightStyle::default()));
        }
        out
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        self.folds.clone()
    }
}

struct Completions;

impl CompletionProvider for Completions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let src = text.to_string();
        let items = lang::complete_at(&src, offset)
            .map(|c| {
                let range = text.offset_to_position(c.replace.start)..text.offset_to_position(c.replace.end);
                let doc = Document::parse(src.as_str());
                candidates(&c.want, doc.diagram())
                    .into_iter()
                    .filter(|cand| cand.label != c.prefix && matches(&cand.label, &c.prefix))
                    .take(80)
                    .map(|cand| cand.into_item(lsp_types::Range { start: range.start, end: range.end }))
                    .collect()
            })
            .unwrap_or_default();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _: &mut App) -> bool {
        new_text.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | ' ' | '>' | '{' | ','))
            && !new_text.is_empty()
    }
}

/// Case-insensitive prefix match on the whole name or any dotted part, so
/// `part` finds `sysml.part`.
fn matches(label: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let p = prefix.to_lowercase();
    let l = label.to_lowercase();
    l.starts_with(&p) || l.split(['.', '-', '_']).any(|part| part.starts_with(&p))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub label: String,
    pub detail: Option<String>,
    pub doc: Option<String>,
    pub kind: CompletionItemKind,
}

impl Candidate {
    fn new(label: impl Into<String>, kind: CompletionItemKind) -> Self {
        Self { label: label.into(), detail: None, doc: None, kind }
    }

    fn detail(mut self, d: impl Into<String>) -> Self {
        let d = d.into();
        if !d.is_empty() {
            self.detail = Some(d);
        }
        self
    }

    fn doc(mut self, d: impl Into<String>) -> Self {
        let d = d.into();
        if !d.is_empty() {
            self.doc = Some(d);
        }
        self
    }

    fn into_item(self, range: lsp_types::Range) -> CompletionItem {
        CompletionItem {
            label: self.label.clone(),
            kind: Some(self.kind),
            label_details: self.detail.clone().map(|d| CompletionItemLabelDetails { detail: None, description: Some(d) }),
            detail: self.detail,
            documentation: self.doc.map(lsp_types::Documentation::String),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: self.label })),
            ..Default::default()
        }
    }
}

const DIAGRAM_KEYS: &[(&str, &str)] =
    &[
        ("kind", "Diagram kind"),
        ("context", "Frame context"),
        ("view", "View name"),
        ("look", "technical or plain"),
        ("routing", "straight or orthogonal (elbow) lines"),
        ("flow", "right, down or radial, for new nodes"),
    ];
const EDGE_KEYS: &[(&str, &str)] = &[
    ("kind", "Edge kind"),
    ("line", "solid, dashed or dotted"),
    ("route", "straight or orthogonal for this line"),
    ("head", "End at the target"),
    ("tail", "End at the source"),
    ("stroke", "Line color"),
    ("color", "Label color"),
    ("width", "Line width"),
];
const STYLE_KEYS: &[(&str, &str)] =
    &[("fill", "Fill color"), ("stroke", "Border color"), ("color", "Text color"), ("line", "solid, dashed or dotted"), ("width", "Border width")];
const ENDS: &[&str] = &graphing_scene::End::NAMES;

/// Everything that fits `want`, unfiltered.
pub(crate) fn candidates(want: &Want, d: &Diagram) -> Vec<Candidate> {
    use CompletionItemKind as K;
    let reg = registry();
    let ids = || {
        let nodes = d.nodes.iter().map(|n| {
            Candidate::new(&n.id, K::VARIABLE)
                .detail(n.stencil.clone().unwrap_or_default())
                .doc(n.label.clone().unwrap_or_default())
        });
        let groups = d.groups.iter().map(|g| Candidate::new(&g.id, K::MODULE).detail("group").doc(g.label.clone().unwrap_or_default()));
        nodes.chain(groups).collect::<Vec<_>>()
    };
    let keys = |list: &[(&str, &str)]| list.iter().map(|(k, doc)| Candidate::new(*k, K::PROPERTY).doc(*doc)).collect::<Vec<_>>();
    match want {
        Want::Statement => {
            let mut out: Vec<Candidate> = lang::KEYWORDS.iter().map(|k| Candidate::new(*k, K::KEYWORD)).collect();
            out.extend(ids());
            out
        }
        Want::Ref => ids(),
        Want::Port(node) => {
            ops::ports_of(d, node).into_iter().map(|(name, ty, _)| Candidate::new(name, K::FIELD).detail(ty.unwrap_or_default())).collect()
        }
        Want::Stencil => reg
            .stencils()
            .filter(|s| !s.hidden)
            .map(|s| Candidate::new(&s.id, K::CLASS).detail(&s.category).doc(&s.title))
            .collect(),
        Want::Pack => reg.packs.iter().map(|p| Candidate::new(&p.id, K::MODULE).detail(&p.name)).collect(),
        Want::Class => d.styles.keys().map(|s| Candidate::new(s, K::ENUM_MEMBER).detail("style")).collect(),
        Want::Key(owner) => match owner {
            Owner::Diagram => keys(DIAGRAM_KEYS),
            Owner::Edge => keys(EDGE_KEYS),
            Owner::Style => keys(STYLE_KEYS),
            Owner::Group => {
                let mut out = vec![
                    Candidate::new("kind", K::PROPERTY).doc("Group preset (VPC, subnet, SysML block...)"),
                    Candidate::new("look", K::PROPERTY).doc("Group design"),
                    Candidate::new("stereotype", K::PROPERTY).doc("SysML stereotype"),
                ];
                out.extend(keys(STYLE_KEYS));
                out
            }
            Owner::Node(stencil) => {
                let mut out: Vec<Candidate> = graphing_scene::notation::props_for(stencil.as_deref())
                    .into_iter()
                    .map(|p| Candidate::new(p.key, K::PROPERTY).doc(p.label))
                    .collect();
                for c in keys(STYLE_KEYS) {
                    if !out.iter().any(|o| o.label == c.label) {
                        out.push(c);
                    }
                }
                out
            }
        },
        Want::Value(owner, key) => match (owner, key.as_str()) {
            (Owner::Edge, "kind") => reg
                .edge_kinds
                .iter()
                .map(|k| Candidate::new(&k.name, K::ENUM_MEMBER).detail(&k.group).doc(&k.description))
                .collect(),
            (Owner::Diagram, "kind") => reg
                .diagram_kinds
                .iter()
                .map(|k| Candidate::new(&k.id, K::ENUM_MEMBER).detail(&k.name).doc(&k.description))
                .collect(),
            (Owner::Diagram, "context") => {
                let mut seen: Vec<&str> = reg.diagram_kinds.iter().map(|k| k.context.as_str()).filter(|c| !c.is_empty()).collect();
                seen.sort();
                seen.dedup();
                seen.into_iter().map(|c| Candidate::new(c, K::ENUM_MEMBER)).collect()
            }
            (Owner::Diagram, "look") => ["technical", "plain"].iter().map(|v| Candidate::new(*v, K::ENUM_MEMBER)).collect(),
            (Owner::Group, "kind") => reg.group_kinds.iter().map(|k| Candidate::new(&k.name, K::ENUM_MEMBER).detail(&k.category).doc(&k.title)).collect(),
            (Owner::Group, "look") => graphing_scene::GroupLook::ALL.iter().map(|l| Candidate::new(l.name(), K::ENUM_MEMBER).detail(l.title()).doc(l.description())).collect(),
            (_, "line") => ["solid", "dashed", "dotted"].iter().map(|v| Candidate::new(*v, K::ENUM_MEMBER)).collect(),
            (Owner::Diagram, "routing") | (Owner::Edge, "route") => ["straight", "orthogonal"].iter().map(|v| Candidate::new(*v, K::ENUM_MEMBER)).collect(),
            (Owner::Diagram, "flow") => ["right", "down", "radial"].iter().map(|v| Candidate::new(*v, K::ENUM_MEMBER)).collect(),
            (_, "head" | "tail") => ENDS.iter().map(|v| Candidate::new(*v, K::ENUM_MEMBER)).collect(),
            _ => Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::workspace::Workspace;
    use gpui_kit::{AppContext, TestAppContext, VisualTestContext};

    #[gpui_kit::test]
    fn opened_file_shows_diagnostics_and_highlights(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("graphing-source-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bad.gph");
        std::fs::write(&file, "a: rect \"A\"\nlayout { bogus }\n").unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            graphing_ui::install(true, None, cx);
        });
        let mut ws = None;
        let window = cx.add_window(|window, cx| {
            let w = cx.new(|cx| Workspace::new(vec![file.clone()], Settings::default(), Vec::new(), window, cx));
            ws = Some(w.clone());
            gpui_kit::base::Root::new(w, window, cx)
        });
        let ws = ws.unwrap();
        let cx = VisualTestContext::from_window(*window, cx).into_mut();
        cx.run_until_parked();
        let editor = ws.read_with(cx, |w, _| w.editor_at(0));
        let n = editor.read_with(cx, |e, _| e.diagnostics().map_or(0, |d| d.len()));
        assert_eq!(n, 1, "the bad layout entry is flagged");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn labels(want: Want, src: &str) -> Vec<String> {
        let doc = Document::parse(src);
        candidates(&want, doc.diagram()).into_iter().map(|c| c.label).collect()
    }

    #[test]
    fn candidates_come_from_registry_and_document() {
        let src = "use sysml\nuut: sysml.part \"U\" { ports: [\"busPort : SpW\"] }\ngroup g \"G\" { uut }\nstyle hot { fill: #f00 }\n";
        assert!(labels(Want::Stencil, src).iter().any(|l| l == "sysml.part"));
        assert_eq!(labels(Want::Ref, src), vec!["uut", "g"]);
        assert_eq!(labels(Want::Port("uut".into()), src), vec!["busPort"]);
        assert_eq!(labels(Want::Class, src), vec!["hot"]);
        assert!(labels(Want::Value(Owner::Edge, "kind".into()), src).iter().any(|l| l == "flow"));
        assert!(labels(Want::Value(Owner::Diagram, "kind".into()), src).iter().any(|l| l == "ibd"));
        assert!(labels(Want::Statement, src).iter().any(|l| l == "layout"));
        assert!(matches("sysml.part", "part"));
        assert!(!matches("sysml.part", "xyz"));
    }
}
