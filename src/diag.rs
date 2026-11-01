//! Source spans and diagnostics.

use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub file: u32,
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn to(self, other: Span) -> Span {
        Span { file: self.file, lo: self.lo.min(other.lo), hi: self.hi.max(other.hi) }
    }
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub span: Span,
    pub msg: String,
    pub notes: Vec<(Span, String)>,
}

impl Diag {
    pub fn new(span: Span, msg: impl Into<String>) -> Diag {
        Diag { span, msg: msg.into(), notes: Vec::new() }
    }
    pub fn note(mut self, span: Span, msg: impl Into<String>) -> Diag {
        self.notes.push((span, msg.into()));
        self
    }
}

pub type Res<T> = Result<T, Diag>;

pub struct SourceFile {
    pub path: String,
    pub text: String,
}

#[derive(Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, path: String, text: String) -> u32 {
        self.files.push(SourceFile { path, text });
        (self.files.len() - 1) as u32
    }

    pub fn line_col(&self, file: u32, pos: u32) -> (usize, usize) {
        let text = &self.files[file as usize].text;
        let pos = (pos as usize).min(text.len());
        let before = &text[..pos];
        let line = before.matches('\n').count() + 1;
        let col = pos - before.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
        (line, col)
    }

    fn snippet(&self, out: &mut String, span: Span, label: &str) {
        let f = &self.files[span.file as usize];
        let (line, col) = self.line_col(span.file, span.lo);
        let _ = writeln!(out, "  --> {}:{}:{}", f.path, line, col);
        if let Some(text) = f.text.lines().nth(line - 1) {
            let width = if span.hi > span.lo {
                ((span.hi - span.lo) as usize).min(text.len().saturating_sub(col - 1)).max(1)
            } else {
                1
            };
            let _ = writeln!(out, "{:>5} | {}", line, text);
            let _ = writeln!(out, "      | {}{} {}", " ".repeat(col - 1), "^".repeat(width), label);
        }
    }

    pub fn render(&self, d: &Diag) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "error: {}", d.msg);
        if (d.span.file as usize) < self.files.len() {
            self.snippet(&mut out, d.span, "");
        }
        for (sp, n) in &d.notes {
            let _ = writeln!(out, "note: {}", n);
            if (sp.file as usize) < self.files.len() {
                self.snippet(&mut out, *sp, "");
            }
        }
        out
    }
}
