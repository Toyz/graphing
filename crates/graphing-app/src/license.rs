//! SPDX license expressions: which license graphing takes a dependency
//! under, and whether that is one graphing refuses. Shared with `build.rs`
//! (included by path), so it uses nothing from the crate.

/// The license ids in an SPDX expression: `MIT OR Apache-2.0` → `MIT`, `Apache-2.0`.
pub fn license_ids(expr: &str) -> Vec<&str> {
    let mut ids: Vec<&str> = expr
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '/'))
        .filter(|w| !w.is_empty() && !matches!(*w, "OR" | "AND" | "WITH"))
        .collect();
    ids.dedup();
    ids
}

/// The license graphing takes a crate under. Where its license is a choice
/// (`MIT OR Apache-2.0`, `Apache-2.0 OR GPL-2.0-only`), the most permissive
/// option is taken, so nothing copyleft is picked while something else is on
/// offer; licenses joined by `AND` all apply. `/` is an old spelling of `OR`.
pub fn elected(expr: &str) -> String {
    let tokens = tokenize(expr);
    let mut parser = Parser { tokens: &tokens, at: 0 };
    let chosen = match parser.or() {
        Some(e) if parser.at == tokens.len() => e.choose().render(),
        _ => expr.trim().to_owned(),
    };
    if chosen.is_empty() { "Unknown".into() } else { chosen }
}

/// How restrictive a license is: lower is more permissive.
fn rank(id: &str) -> u8 {
    const PERMISSIVE: [&str; 12] =
        ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "Unlicense", "BSL-1.0", "CC0-1.0", "Unicode-3.0", "Unicode-DFS-2016"];
    if let Some(i) = PERMISSIVE.iter().position(|p| p.eq_ignore_ascii_case(id)) {
        return i as u8;
    }
    let id = id.to_ascii_uppercase();
    if id.starts_with("AGPL") {
        90
    } else if id.starts_with("GPL") {
        80
    } else if id.starts_with("LGPL") {
        70
    } else if ["MPL", "EPL", "CDDL"].iter().any(|p| id.starts_with(p)) {
        60
    } else {
        50
    }
}

/// An SPDX license expression.
enum Expr {
    Id(String),
    With(String, String),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

impl Expr {
    /// How restrictive it is: a choice as its most permissive option, licenses together as their most restrictive.
    fn rank(&self) -> u8 {
        match self {
            Expr::Id(id) | Expr::With(id, _) => rank(id),
            Expr::And(parts) => parts.iter().map(Expr::rank).max().unwrap_or(0),
            Expr::Or(parts) => parts.iter().map(Expr::rank).min().unwrap_or(0),
        }
    }

    /// Itself with every choice made.
    fn choose(self) -> Expr {
        match self {
            Expr::Or(parts) => parts.into_iter().map(Expr::choose).min_by_key(Expr::rank).unwrap_or(Expr::Id(String::new())),
            Expr::And(parts) => Expr::And(parts.into_iter().map(Expr::choose).collect()),
            e => e,
        }
    }

    fn render(&self) -> String {
        match self {
            Expr::Id(id) => id.clone(),
            Expr::With(id, exception) => format!("{id} WITH {exception}"),
            Expr::And(parts) => {
                parts.iter().map(|p| if matches!(p, Expr::Or(_)) { format!("({})", p.render()) } else { p.render() }).collect::<Vec<_>>().join(" AND ")
            }
            Expr::Or(parts) => parts.iter().map(Expr::render).collect::<Vec<_>>().join(" OR "),
        }
    }
}

/// Ids, operators and parentheses; `/` reads as `OR`.
fn tokenize(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in expr.chars() {
        if c.is_whitespace() || matches!(c, '(' | ')' | '/') {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            match c {
                '/' => out.push("OR".into()),
                '(' | ')' => out.push(c.to_string()),
                _ => {}
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// `or := and (OR and)*`, `and := atom (AND atom)*`, `atom := ( or ) | id [WITH id]`.
struct Parser<'a> {
    tokens: &'a [String],
    at: usize,
}

impl Parser<'_> {
    fn next_is(&self, word: &str) -> bool {
        self.tokens.get(self.at).is_some_and(|t| t.eq_ignore_ascii_case(word))
    }

    fn or(&mut self) -> Option<Expr> {
        let mut parts = vec![self.and()?];
        while self.next_is("OR") {
            self.at += 1;
            parts.push(self.and()?);
        }
        Some(if parts.len() == 1 { parts.remove(0) } else { Expr::Or(parts) })
    }

    fn and(&mut self) -> Option<Expr> {
        let mut parts = vec![self.atom()?];
        while self.next_is("AND") {
            self.at += 1;
            parts.push(self.atom()?);
        }
        Some(if parts.len() == 1 { parts.remove(0) } else { Expr::And(parts) })
    }

    fn atom(&mut self) -> Option<Expr> {
        let token = self.tokens.get(self.at)?.clone();
        self.at += 1;
        match token.as_str() {
            "(" => {
                let inner = self.or()?;
                if !self.next_is(")") {
                    return None;
                }
                self.at += 1;
                Some(inner)
            }
            ")" => None,
            _ if self.next_is("WITH") => {
                let exception = self.tokens.get(self.at + 1)?.clone();
                self.at += 2;
                Some(Expr::With(token, exception))
            }
            _ => Some(Expr::Id(token)),
        }
    }
}

/// Why a dependency's license is refused, if it is: graphing takes no
/// copyleft code. Where a crate offers a choice the permissive option is
/// taken (see [`elected`]); it is refused only when every option is
/// copyleft. MPL-2.0 is the one exception: its copyleft covers only the
/// crate's own files, which graphing uses unmodified. `build.rs` fails the
/// build on any refused dependency.
#[allow(dead_code)]
pub fn refused(expr: &str) -> Option<String> {
    let chosen = elected(expr);
    let bad: Vec<&str> = license_ids(&chosen).into_iter().filter(|id| rank(id) >= 60 && !id.eq_ignore_ascii_case("MPL-2.0")).collect();
    (!bad.is_empty()).then(|| format!("{chosen} is copyleft"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copyleft_is_refused_unless_there_is_a_choice() {
        assert_eq!(refused("MIT OR Apache-2.0"), None);
        assert_eq!(refused("Apache-2.0 OR GPL-2.0-only"), None);
        assert_eq!(refused("MPL-2.0"), None);
        assert!(refused("GPL-3.0-only").is_some());
        assert!(refused("LGPL-2.1-or-later").is_some());
        assert!(refused("MIT AND AGPL-3.0").is_some());
        assert!(refused("EPL-2.0").is_some());
        assert_eq!(refused("LGPL-2.1-or-later OR MPL-2.0"), None);
    }
}
