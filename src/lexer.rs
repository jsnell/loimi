//! Tokenizer.

use crate::diag::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(u128),
    Float(f64),
    Str(Vec<u8>),
    Char(u8),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// True if whitespace or a comment precedes this token (used to
    /// distinguish `a.b` from `a .b` in a few places).
    pub space_before: bool,
}

// Longest first, so that greedy matching works.
const PUNCTS: &[&str] = &[
    "<<=", ">>=", "...", "..=", "+%=", "-%=", "*%=", "+%", "-%", "*%", "::", "->", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>", "..",
    "??", "(", ")", "{", "}", "[", "]", ",", ";", ":", ".", "=", "<", ">", "+", "-", "*", "/",
    "%", "&", "|", "^", "!", "~", "?", "#", "@",
];

pub fn lex(file: u32, src: &str) -> Result<Vec<Token>, Diag> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    let mut space = true;
    let sp = |lo: usize, hi: usize| Span { file, lo: lo as u32, hi: hi as u32 };
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            space = true;
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            space = true;
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let start = i;
            let mut depth = 0;
            loop {
                if i + 1 >= b.len() {
                    return Err(Diag::new(sp(start, start + 2), "unterminated block comment"));
                }
                if b[i] == b'/' && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            space = true;
            continue;
        }
        let start = i;
        let tok = if let Some(hashes) = raw_string_start(b, i) {
            // r"..." or r#"..."#: no escapes; ends at `"` and as many `#`.
            i += 2 + hashes;
            let body = i;
            loop {
                if i >= b.len() {
                    return Err(Diag::new(sp(start, body), "unterminated raw string literal"));
                }
                if b[i] == b'"' && b.len() - i > hashes && b[i + 1..i + 1 + hashes].iter().all(|&h| h == b'#') {
                    break;
                }
                i += 1;
            }
            let s = b[body..i].to_vec();
            i += 1 + hashes;
            Tok::Str(s)
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            Tok::Ident(src[start..i].to_string())
        } else if c.is_ascii_digit() {
            lex_number(b, &mut i, file)?
        } else if c == b'"' {
            i += 1;
            let mut s = Vec::new();
            loop {
                if i >= b.len() {
                    return Err(Diag::new(sp(start, start + 1), "unterminated string literal"));
                }
                match b[i] {
                    b'"' => {
                        i += 1;
                        break;
                    }
                    b'\\' => {
                        i += 1;
                        s.push(lex_escape(b, &mut i, file)?);
                    }
                    ch => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            Tok::Str(s)
        } else if c == b'\'' {
            i += 1;
            let ch = if i < b.len() && b[i] == b'\\' {
                i += 1;
                lex_escape(b, &mut i, file)?
            } else if i < b.len() {
                i += 1;
                b[i - 1]
            } else {
                return Err(Diag::new(sp(start, i), "unterminated character literal"));
            };
            if i >= b.len() || b[i] != b'\'' {
                return Err(Diag::new(sp(start, i), "unterminated character literal"));
            }
            i += 1;
            Tok::Char(ch)
        } else {
            let rest = &src[i..];
            match PUNCTS.iter().find(|p| rest.starts_with(**p)) {
                Some(p) => {
                    i += p.len();
                    Tok::Punct(p)
                }
                None => {
                    return Err(Diag::new(sp(i, i + 1), format!("unexpected character `{}`", c as char)))
                }
            }
        };
        out.push(Token { tok, span: sp(start, i), space_before: space });
        space = false;
    }
    out.push(Token { tok: Tok::Eof, span: sp(b.len(), b.len()), space_before: true });
    Ok(out)
}

/// The number of `#`s if a raw string literal (`r"`, `r#"`, `r##"`, ..)
/// starts at `i`.
fn raw_string_start(b: &[u8], i: usize) -> Option<usize> {
    if b[i] != b'r' || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() && b[j] == b'#' {
        j += 1;
    }
    (j < b.len() && b[j] == b'"').then_some(j - i - 1)
}

fn lex_escape(b: &[u8], i: &mut usize, file: u32) -> Result<u8, Diag> {
    let at = *i;
    let c = *b.get(at).ok_or_else(|| Diag::new(Span { file, lo: at as u32, hi: at as u32 }, "bad escape"))?;
    *i += 1;
    Ok(match c {
        b'n' => b'\n',
        b't' => b'\t',
        b'r' => b'\r',
        b'0' => 0,
        b'\\' => b'\\',
        b'\'' => b'\'',
        b'"' => b'"',
        b'x' => {
            let h = std::str::from_utf8(b.get(at + 1..at + 3).unwrap_or(b"")).unwrap_or("");
            let v = u8::from_str_radix(h, 16)
                .map_err(|_| Diag::new(Span { file, lo: at as u32, hi: at as u32 + 3 }, "bad \\x escape"))?;
            *i += 2;
            v
        }
        _ => return Err(Diag::new(Span { file, lo: at as u32 - 1, hi: at as u32 + 1 }, "unknown escape")),
    })
}

fn lex_number(b: &[u8], i: &mut usize, file: u32) -> Result<Tok, Diag> {
    let start = *i;
    let err = |lo: usize, hi: usize, m: &str| Diag::new(Span { file, lo: lo as u32, hi: hi as u32 }, m.to_string());
    if b[*i] == b'0' && *i + 1 < b.len() && matches!(b[*i + 1], b'x' | b'b' | b'o') {
        let radix = match b[*i + 1] {
            b'x' => 16,
            b'b' => 2,
            _ => 8,
        };
        *i += 2;
        let ds = *i;
        while *i < b.len() && (b[*i].is_ascii_hexdigit() || b[*i] == b'_') {
            *i += 1;
        }
        let digits: String = std::str::from_utf8(&b[ds..*i]).unwrap().chars().filter(|c| *c != '_').collect();
        return u128::from_str_radix(&digits, radix).map(Tok::Int).map_err(|_| err(start, *i, "bad integer literal"));
    }
    while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'_') {
        *i += 1;
    }
    let mut is_float = false;
    // `1..2` is a range, not a float; `x.0.1` is tuple access (handled by the parser).
    if *i + 1 < b.len() && b[*i] == b'.' && b[*i + 1].is_ascii_digit() {
        is_float = true;
        *i += 1;
        while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'_') {
            *i += 1;
        }
    }
    if *i < b.len() && (b[*i] == b'e' || b[*i] == b'E') {
        let save = *i;
        *i += 1;
        if *i < b.len() && (b[*i] == b'+' || b[*i] == b'-') {
            *i += 1;
        }
        if *i < b.len() && b[*i].is_ascii_digit() {
            is_float = true;
            while *i < b.len() && b[*i].is_ascii_digit() {
                *i += 1;
            }
        } else {
            *i = save;
        }
    }
    let text: String = std::str::from_utf8(&b[start..*i]).unwrap().chars().filter(|c| *c != '_').collect();
    if is_float {
        text.parse::<f64>().map(Tok::Float).map_err(|_| err(start, *i, "bad float literal"))
    } else {
        text.parse::<u128>().map(Tok::Int).map_err(|_| err(start, *i, "integer literal too large"))
    }
}
