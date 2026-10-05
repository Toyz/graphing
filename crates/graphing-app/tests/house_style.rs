//! Chrome is built from graphing-ui's tokens and kit. A raw `px(..)` or
//! `rgb(..)` in a view is how one panel drifts from the next, so they are
//! only allowed in the design system crate and the canvas painter (which
//! works in diagram units and paints user-chosen colors).

use std::path::Path;

const EXEMPT: &[&str] = &["paint.rs", "tests.rs"];

#[test]
fn views_use_tokens() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for entry in walk(&src) {
        let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if EXEMPT.contains(&name) {
            continue;
        }
        let text = std::fs::read_to_string(&entry).unwrap();
        // Test modules sit at the end of a file and may use raw geometry.
        let code_part = text.split("#[cfg(test)]").next().unwrap_or("");
        for (i, line) in code_part.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let raw_px = code.match_indices("px(").any(|(at, _)| {
                let before = code[..at].chars().last();
                let after = code[at + 3..].chars().next();
                before.is_none_or(|c| !c.is_alphanumeric() && c != '_') && after.is_some_and(|c| c.is_ascii_digit())
            });
            if raw_px || code.contains("rgb(0x") || code.contains("rgba(0x") {
                offenders.push(format!("{}:{}: {}", entry.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(offenders.is_empty(), "use graphing_ui tokens instead of raw values:\n{}", offenders.join("\n"));
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out
}
