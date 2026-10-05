use std::ops::Range;

pub type Span = Range<usize>;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Str(String),
    Num(f64),
    /// `180x64`
    Size(f64, f64),
    /// `#3b5bdb`, kept with the hash.
    Color(String),
    Colon,
    Comma,
    Dot,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    /// `->`
    Fwd,
    /// `<-`
    Back,
    /// `<->`
    Both,
    /// `--`
    Line,
    Newline,
    /// Unknown character; parser reports it.
    Error(char),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Comments and spaces are skipped; their text stays in the source and is
/// never touched by edits because edits only splice token spans.
pub fn lex(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let tok = match c {
            b' ' | b'\t' | b'\r' => {
                i += 1;
                continue;
            }
            b'\n' => {
                i += 1;
                Tok::Newline
            }
            // Colors only appear as prop values, so `#` after `:` is a color
            // and anywhere else starts a comment.
            b'#' if out.last().is_some_and(|t: &Token| t.tok == Tok::Colon) && is_color(&b[i + 1..]) => {
                i += 1;
                while i < b.len() && b[i].is_ascii_hexdigit() {
                    i += 1;
                }
                Tok::Color(src[start..i].to_string())
            }
            b'#' | b'/' if c == b'#' || src[i..].starts_with("//") => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'"' => {
                i += 1;
                let mut s = String::new();
                while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        match b[i + 1] {
                            b'n' => s.push('\n'),
                            b't' => s.push('\t'),
                            other => s.push(other as char),
                        }
                        i += 2;
                    } else {
                        let ch = src[i..].chars().next().unwrap_or('?');
                        s.push(ch);
                        i += ch.len_utf8();
                    }
                }
                if i < b.len() && b[i] == b'"' {
                    i += 1;
                }
                Tok::Str(s)
            }
            b':' => {
                i += 1;
                Tok::Colon
            }
            b',' => {
                i += 1;
                Tok::Comma
            }
            b'.' => {
                i += 1;
                Tok::Dot
            }
            b'{' => {
                i += 1;
                Tok::LBrace
            }
            b'}' => {
                i += 1;
                Tok::RBrace
            }
            b'[' => {
                i += 1;
                Tok::LBracket
            }
            b']' => {
                i += 1;
                Tok::RBracket
            }
            b'<' if src[i..].starts_with("<->") => {
                i += 3;
                Tok::Both
            }
            b'<' if src[i..].starts_with("<-") => {
                i += 2;
                Tok::Back
            }
            b'-' if src[i..].starts_with("->") => {
                i += 2;
                Tok::Fwd
            }
            b'-' if src[i..].starts_with("--") => {
                i += 2;
                Tok::Line
            }
            b'-' | b'0'..=b'9' if c != b'-' || b.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let (n, end) = number(src, i);
                i = end;
                if i + 1 < b.len() && b[i] == b'x' && b[i + 1].is_ascii_digit() {
                    let (h, end) = number(src, i + 1);
                    i = end;
                    Tok::Size(n, h)
                } else {
                    Tok::Num(n)
                }
            }
            c if c == b'_' || c.is_ascii_alphabetic() || c >= 0x80 => {
                // A dot followed by a word char continues the ident, so `R1.1`,
                // `flow.process` and `uut.busPort` read as one token.
                while i < b.len()
                    && (b[i] == b'_'
                        || b[i] == b'-' && !src[i..].starts_with("->") && !src[i..].starts_with("--")
                        || b[i] == b'.' && b.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                        || b[i].is_ascii_alphanumeric()
                        || b[i] >= 0x80)
                {
                    i += 1;
                }
                Tok::Ident(src[start..i].to_string())
            }
            _ => {
                let ch = src[i..].chars().next().unwrap_or('?');
                i += ch.len_utf8();
                Tok::Error(ch)
            }
        };
        out.push(Token { tok, span: start..i });
    }
    out
}

/// `abc`, `aabbcc`, `aabbccdd` followed by a non-word char.
fn is_color(rest: &[u8]) -> bool {
    let n = rest.iter().take_while(|c| c.is_ascii_hexdigit()).count();
    matches!(n, 3 | 4 | 6 | 8) && rest.get(n).is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
}

fn number(src: &str, start: usize) -> (f64, usize) {
    let b = src.as_bytes();
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    (src[start..i].parse().unwrap_or(0.0), i)
}
