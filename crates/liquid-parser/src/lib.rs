//! Rust port of `@shopify/liquid-html-parser`: `toLiquidHtmlAST` and `toLiquidAST` with their
//! default options (strict parsing; the TS tolerant/lax paths are unreachable from them).
//!
//! Positions in the AST are UTF-8 byte offsets. Error `loc` columns are UTF-16 based, like TS.

pub use liquid_ast as ast;
use liquid_ast::Document;

mod html;
mod lines;
mod liquid_doc;
mod markup;
mod parser;
mod tags;
mod tokenizer;

pub use markup::{tokenize_markup, MarkupParser, MarkupToken, MarkupTokenType};
pub use tags::{BUILTIN_TAGS, LOOP_NAMED_TAGS};
pub use tokenizer::{tokenize, Token, TokenType};

/// Keys of TS `LiquidLiteralValues`.
pub const LIQUID_LITERAL_KEYWORDS: &[&str] = &["nil", "null", "true", "false", "blank", "empty"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineColumn {
    /// 1-based.
    pub line: usize,
    /// 1-based, counted in UTF-16 code units (what TS reports).
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loc {
    pub start: LineColumn,
    pub end: LineColumn,
}

/// Mirrors `LiquidHTMLASTParsingError` (`name == "LiquidHTMLParsingError"`, `loc` set; TS's
/// `unclosed` is always null on these code paths). A few malformed inputs make TS throw a plain
/// `Error` instead (`name == "Error"`, no `loc`), e.g. `{% constructor %}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub name: &'static str,
    pub message: String,
    /// Byte offsets passed to the TS constructor (before its clamping).
    pub start: usize,
    pub end: usize,
    pub loc: Option<Loc>,
}

impl ParseError {
    pub(crate) fn liquid(message: String, start: usize, end: usize) -> Self {
        ParseError {
            name: "LiquidHTMLParsingError",
            message,
            start,
            end,
            loc: None,
        }
    }

    pub(crate) fn plain(message: String) -> Self {
        ParseError {
            name: "Error",
            message,
            start: 0,
            end: 0,
            loc: None,
        }
    }

    fn with_loc(mut self, source: &str) -> Self {
        if self.name == "LiquidHTMLParsingError" {
            self.loc = Some(Loc {
                start: line_column(source, self.start),
                end: line_column(source, self.end),
            });
        }
        self
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.message)
    }
}

impl std::error::Error for ParseError {}

/// The `line-column` package on a UTF-16 index clamped to `[0, len - 1]`; `1:1` for "".
fn line_column(source: &str, byte: usize) -> LineColumn {
    let byte = byte.min(source.len());
    let len16 = source.encode_utf16().count();
    if len16 == 0 {
        return LineColumn { line: 1, column: 1 };
    }
    let target = source[..byte].encode_utf16().count().min(len16 - 1);
    let (mut line, mut line_start, mut i) = (1, 0, 0);
    for c in source.chars() {
        if i >= target {
            break;
        }
        i += c.len_utf16();
        if c == '\n' {
            line += 1;
            line_start = i;
        }
    }
    LineColumn {
        line,
        column: target - line_start + 1,
    }
}

pub fn to_liquid_html_ast(source: &str) -> Result<Document<'_>, ParseError> {
    parser::parse_document(source, true, false).map_err(|e| e.with_loc(source))
}

pub fn to_liquid_ast(source: &str) -> Result<Document<'_>, ParseError> {
    parser::parse_document(source, false, true).map_err(|e| e.with_loc(source))
}

/// JS `\s` / `String.prototype.trim` whitespace.
pub(crate) fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

pub(crate) fn js_trim(s: &str) -> &str {
    js_trim_end(js_trim_start(s))
}

fn is_ascii_js_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

// Byte loops for the ASCII whitespace; the char-based trim only runs on a non-ASCII edge.
pub(crate) fn js_trim_start(s: &str) -> &str {
    let s = &s[s
        .bytes()
        .position(|c| !is_ascii_js_space(c))
        .unwrap_or(s.len())..];
    if s.as_bytes().first().is_some_and(|c| *c >= 0x80) {
        s.trim_start_matches(is_js_space)
    } else {
        s
    }
}

pub(crate) fn js_trim_end(s: &str) -> &str {
    let s = &s[..s
        .bytes()
        .rposition(|c| !is_ascii_js_space(c))
        .map_or(0, |i| i + 1)];
    if s.as_bytes().last().is_some_and(|c| *c >= 0x80) {
        s.trim_end_matches(is_js_space)
    } else {
        s
    }
}

/// `s.search(/\s/)`
pub(crate) fn js_find_space(s: &str) -> Option<usize> {
    let i = s.bytes().position(|c| is_ascii_js_space(c) || c >= 0x80)?;
    if is_ascii_js_space(s.as_bytes()[i]) {
        Some(i)
    } else {
        s[i..].find(is_js_space).map(|j| i + j)
    }
}

pub(crate) fn starts_with_js_space(s: &str) -> bool {
    s.chars().next().is_some_and(is_js_space)
}

/// `s.indexOf(pat)` for an ASCII `pat`: memchr on its first byte, which beats the generic
/// substring searcher on these short patterns.
pub(crate) fn find_ascii(s: &str, pat: &str) -> Option<usize> {
    let (first, b) = (pat.as_bytes()[0] as char, s.as_bytes());
    let mut i = 0;
    while let Some(j) = s[i..].find(first) {
        if b[i + j..].starts_with(pat.as_bytes()) {
            return Some(i + j);
        }
        i += j + 1;
    }
    None
}

/// `/\{[{%]/.test(s)`
pub(crate) fn has_liquid_open(s: &str) -> bool {
    find_ascii(s, "{{").is_some() || find_ascii(s, "{%").is_some()
}

/// The first word of a tag body: `trimmed.slice(0, trimmed.search(/\s/))` after `trimStart`.
pub(crate) fn first_word(s: &str) -> &str {
    let t = js_trim_start(s);
    match js_find_space(t) {
        Some(i) => &t[..i],
        None => t,
    }
}

/// JS `toLowerCase() === lower` where `lower` is already lowercase.
pub(crate) fn js_lower_eq(s: &str, lower: &str) -> bool {
    if s.is_ascii() {
        s.eq_ignore_ascii_case(lower)
    } else {
        s.to_lowercase() == lower
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_have_ts_locations() {
        let e = to_liquid_html_ast("a\n<div>{% if x %}</div>").unwrap_err();
        assert_eq!(e.name, "LiquidHTMLParsingError");
        assert_eq!(
            e.message,
            "Attempting to end parsing before LiquidTag 'if' was closed"
        );
        assert_eq!(e.loc.unwrap().start, LineColumn { line: 2, column: 6 });
        let e = to_liquid_html_ast("é\n{{ x").unwrap_err();
        assert_eq!(
            e.message,
            "Expected LiquidVariableOutputClose but got EndOfInput"
        );
        // EndOfInput sits past the end: clamped to the last UTF-16 unit.
        assert_eq!(e.loc.unwrap().start, LineColumn { line: 2, column: 4 });
        assert_eq!(line_column("", 3), LineColumn { line: 1, column: 1 });
    }

    #[test]
    fn js_whitespace_helpers() {
        assert_eq!(js_trim("\u{a0} \tx\u{2028}y \u{3000}\n"), "x\u{2028}y");
        assert_eq!(js_trim(" \u{feff} "), "");
        assert_eq!(js_find_space("é\u{2009}x y"), Some(2));
        assert_eq!(js_find_space("éx"), None);
    }
}
