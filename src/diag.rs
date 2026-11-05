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
    /// For warnings: the lint's name, for `#[allow(name)]`. Printed as
    /// `warning[name]: ..`.
    pub lint: Option<&'static str>,
}

/// The warnings the compiler knows, for `#[allow(..)]`.
pub const LINTS: &[&str] = &["match_fallout", "super_overlap", "hot_gc", "unreachable_arm", "value_layout", "handler_frame", "next_pc_then"];

/// Lints that no longer exist, still accepted by `#[allow(..)]` (which
/// then has no effect) so that code naming them keeps building:
/// `state_root` warned about traced vm state that `sync` did not store,
/// before the compiler rooted such state itself (language.md section 8).
pub const RETIRED: &[&str] = &["state_root"];

/// Lints that are off unless turned on with `-W name`.
pub const OPT_IN: &[&str] = &["hot_gc", "handler_frame"];

impl Diag {
    pub fn new(span: Span, msg: impl Into<String>) -> Diag {
        Diag { span, msg: msg.into(), notes: Vec::new(), lint: None }
    }
    pub fn warning(lint: &'static str, span: Span, msg: impl Into<String>) -> Diag {
        Diag { span, msg: msg.into(), notes: Vec::new(), lint: Some(lint) }
    }
    pub fn note(mut self, span: Span, msg: impl Into<String>) -> Diag {
        self.notes.push((span, msg.into()));
        self
    }
}

pub type Res<T> = Result<T, Diag>;

/// The errors of a build: one from a pass that stops at its first, or
/// all those a pass that goes on past errors found.
#[derive(Debug)]
pub struct Diags(pub Vec<Diag>);

impl From<Diag> for Diags {
    fn from(d: Diag) -> Diags {
        Diags(vec![d])
    }
}

/// The default of `--max-errors`.
pub const MAX_ERRORS: usize = 20;

pub struct SourceFile {
    pub path: String,
    pub text: String,
    /// For generated source (a vm's expansion): the vm's name, and where
    /// the user's text spliced into it came from.
    pub generated: Option<Generated>,
}

/// Generated source: `origins` maps the pieces of it that are user text,
/// copied verbatim, back to the user's source, so that diagnostics (and
/// panic locations) in them point there.
pub struct Generated {
    pub vm: String,
    pub origins: Vec<Origin>,
}

/// `lo..hi` of a generated file is a verbatim copy of the user's text at
/// `src..`; `whole` is the user's whole construct the piece is part of
/// (a `verify` clause split around a rewritten call is several pieces),
/// and `ctx` names the generated code it went into.
pub struct Origin {
    pub lo: u32,
    pub hi: u32,
    pub src: Span,
    pub whole: Span,
    pub ctx: String,
}

#[derive(Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, path: String, text: String) -> u32 {
        self.files.push(SourceFile { path, text, generated: None });
        (self.files.len() - 1) as u32
    }

    pub fn add_generated(&mut self, path: String, text: String, generated: Generated) -> u32 {
        self.files.push(SourceFile { path, text, generated: Some(generated) });
        (self.files.len() - 1) as u32
    }

    /// Where a span of generated source came from: the user's span and
    /// the context it was generated into, if the span lies in user text
    /// spliced into the generated source. A span that also covers a little
    /// generated text around one user construct (`((COUNT) as i64)`, a
    /// rewritten `is_insn_boundary(..)` call) maps to the user's part.
    pub fn origin(&self, span: Span) -> Option<(Span, &str)> {
        let g = self.files.get(span.file as usize)?.generated.as_ref()?;
        let hit: Vec<&Origin> = g
            .origins
            .iter()
            .filter(|o| if span.lo == span.hi { o.lo <= span.lo && span.lo <= o.hi } else { o.lo < span.hi && span.lo < o.hi })
            .collect();
        let (first, last) = (hit.first()?, hit.last()?);
        if hit.iter().any(|o| o.whole != first.whole) {
            return None;
        }
        if hit.len() > 1 || span.lo < first.lo || span.hi > last.hi {
            // The generated text the span covers besides the user's.
            let text = &self.files[span.file as usize].text;
            let mut glue = String::new();
            let mut p = span.lo;
            for o in &hit {
                if p < o.lo {
                    glue.push_str(&text[p as usize..o.lo as usize]);
                }
                p = p.max(o.hi);
            }
            if p < span.hi {
                glue.push_str(&text[p as usize..span.hi as usize]);
            }
            if glue.len() > 60 || glue.contains('\n') {
                return None;
            }
        }
        let at = |o: &Origin, p: u32| o.src.lo + (p.clamp(o.lo, o.hi) - o.lo);
        let s = Span { file: first.src.file, lo: at(first, span.lo), hi: at(last, span.hi) };
        Some((s, &first.ctx))
    }

    /// `span`, or where it came from if it is user text in generated source.
    pub fn user_span(&self, span: Span) -> Span {
        self.origin(span).map(|o| o.0).unwrap_or(span)
    }

    /// The source text `span` covers.
    pub fn text(&self, span: Span) -> &str {
        &self.files[span.file as usize].text[span.lo as usize..span.hi as usize]
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

    /// A build's errors: in source order (where they point in the user's
    /// source), each place once, at most `max` of them (0: all), then
    /// how many there were if more than one.
    pub fn render_all(&self, ds: &[Diag], max: usize) -> String {
        let key = |d: &Diag| {
            let s = self.user_span(d.span);
            (self.files.get(s.file as usize).map_or("", |f| f.path.as_str()), s.lo, s.hi)
        };
        let mut v: Vec<&Diag> = ds.iter().collect();
        v.sort_by_key(|d| key(d));
        v.dedup_by(|a, b| key(a) == key(b));
        let mut out = String::new();
        let shown = if max == 0 { v.len() } else { v.len().min(max) };
        for d in &v[..shown] {
            out.push_str(&self.render(d));
        }
        if v.len() > shown {
            let _ = writeln!(out, "error: {} errors; the first {} are shown (--max-errors N shows more, 0 all)", v.len(), shown);
        } else if v.len() > 1 {
            let _ = writeln!(out, "error: {} errors", v.len());
        }
        out
    }

    pub fn render(&self, d: &Diag) -> String {
        self.render_as("error", d)
    }

    pub fn render_as(&self, severity: &str, d: &Diag) -> String {
        let mut out = String::new();
        match d.lint {
            Some(l) => {
                let _ = writeln!(out, "{}[{}]: {}", severity, l, d.msg);
            }
            None => {
                let _ = writeln!(out, "{}: {}", severity, d.msg);
            }
        }
        // In generated source (a vm's expansion), point at the user's
        // text the error is in and say what it was generated into; an
        // error in the generated code itself is the compiler's.
        let mut span = d.span;
        let mut first_note = None;
        if let Some(g) = self.files.get(span.file as usize).and_then(|f| f.generated.as_ref()) {
            match self.origin(span) {
                Some((s, ctx)) => {
                    span = s;
                    first_note = Some(format!("in {ctx}"));
                }
                None => {
                    first_note = Some(format!(
                        "internal: generated code: the compiler generated this for vm `{}` and it should compile (a compiler bug); LOIMI_DUMP_VM_EXPANSION=1 prints the generated code",
                        g.vm
                    ))
                }
            }
        }
        if (span.file as usize) < self.files.len() {
            self.snippet(&mut out, span, "");
        }
        if let Some(n) = first_note {
            let _ = writeln!(out, "note: {}", n);
        }
        for (sp, n) in &d.notes {
            let _ = writeln!(out, "note: {}", n);
            if (sp.file as usize) < self.files.len() {
                self.snippet(&mut out, self.user_span(*sp), "");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A user file, and an expansion that copies `x + 1` from it.
    fn map() -> SourceMap {
        let mut sm = SourceMap::default();
        sm.add("a.lm".into(), "vm V {\n    operand r = it verify x + 1;\n}\n".into());
        let text = "fn f() {\n  let __w = 0; if !(x + 1) { }\n}\n";
        let lo = text.find("x + 1").unwrap() as u32;
        let src = Span { file: 0, lo: 33, hi: 38 };
        let o = Origin { lo, hi: lo + 5, src, whole: src, ctx: "the generated verifier".into() };
        sm.add_generated("<vm V expansion>".into(), text.into(), Generated { vm: "V".into(), origins: vec![o] });
        sm
    }

    #[test]
    fn user_text_in_expansion() {
        let sm = map();
        let lo = sm.files[1].text.find("x + 1").unwrap() as u32;
        let out = sm.render(&Diag::new(Span { file: 1, lo, hi: lo + 1 }, "unknown name `x`"));
        assert!(out.contains("--> a.lm:2:27\n"), "{out}");
        assert!(out.contains("note: in the generated verifier\n"), "{out}");
        // `(x + 1)`, with generated parentheses: the user's part.
        assert_eq!(sm.user_span(Span { file: 1, lo: lo - 1, hi: lo + 6 }), Span { file: 0, lo: 33, hi: 38 });
        // Generated text spanning lines is not the user's.
        assert_eq!(sm.user_span(Span { file: 1, lo: 0, hi: lo + 6 }), Span { file: 1, lo: 0, hi: lo + 6 });
    }

    #[test]
    fn generated_glue() {
        let sm = map();
        let out = sm.render(&Diag::new(Span { file: 1, lo: 11, hi: 14 }, "oops"));
        assert!(out.contains("--> <vm V expansion>:2:3"), "{out}");
        assert!(out.contains("note: internal: generated code"), "{out}");
        assert!(out.contains("LOIMI_DUMP_VM_EXPANSION=1"), "{out}");
    }
}
