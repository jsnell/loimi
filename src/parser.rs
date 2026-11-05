//! Recursive-descent parser.

use crate::ast::*;
use crate::diag::{Diag, Res, Span};
use crate::lexer::{Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// Struct literals are not allowed in `if`/`while`/`match` heads.
    no_struct: bool,
    /// Inside a vm handler: control words are recognized.
    pub in_vm: bool,
    /// Parsing the type after `as`: `<` may be a comparison.
    cast_type: bool,
    /// Variables of `inline for` statements expanded inside vm bodies,
    /// checked against the vm's state names when the vm is complete.
    pub vm_loop_vars: Vec<Ident>,
    /// `#[allow(lint)]` attributes: (lint, attribute span, region). The
    /// region runs from the attribute to the end of what it is attached to
    /// (an item, a vm item, a match arm), or is the whole file for
    /// `#![allow(..)]`. Warnings inside a region are not reported.
    pub allows: Vec<(String, Span, Span)>,
    /// Allows whose construct is still being parsed: (lint, attribute span).
    open_allows: Vec<(String, Span)>,
}

thread_local! {
    /// The names `--cfg NAME` enabled for this build: `#[cfg(NAME)]`
    /// keeps what it is attached to, `#[cfg(not(NAME))]` drops it.
    static CFG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Enable the `#[cfg(..)]` names of the build (`--cfg NAME`).
pub fn set_cfg(names: &[String]) {
    CFG.with(|c| *c.borrow_mut() = names.to_vec());
}

fn cfg_enabled(name: &str) -> bool {
    CFG.with(|c| c.borrow().iter().any(|n| n == name))
}

/// Every attribute and where it applies, for messages. `allow` is
/// accepted everywhere.
const ATTRS: &[(&str, &str)] = &[
    ("allow", "any item"),
    ("inline", "functions"),
    ("cold", "functions"),
    ("unsafe", "functions"),
    ("raw", "functions (a raw-pointer operation: `unsafe { }` in a `#![safe]` module)"),
    ("safe", "modules (`#![safe]`: raw-pointer operations need `unsafe { }`)"),
    ("lifetimes", "modules (`#![lifetimes(off)]`: pointers into containers are not checked for use after the container may move or free them; checked by default)"),
    ("may_alias", "functions (`#[may_alias(p)]`: argument `p` may point into memory the function moves or frees)"),
    ("shrinks", "functions (`#[shrinks(v)]`: the function may remove elements of the container `v` points to, ending pointers into it)"),
    ("thread_local", "module-level `var`s"),
    ("require_verified", "`vm` declarations"),
    ("dispatch", "`vm` declarations"),
    ("profile", "`vm` declarations (`#[profile(\"file\")]`: handlers laid out by a dispatch profile)"),
    ("forward", "`super` declarations in a vm"),
    ("accumulate", "`op` declarations in a vm with an `accumulator`"),
    ("catch_panic", "`export \"C\"` functions"),
    ("c_name", "structs and enums named in a C header"),
    ("c_const", "`export \"C\"` functions (pointer parameters `const` in C)"),
    ("test", "functions without parameters (run by `loimi test`)"),
    ("track_caller", "functions (`test.caller()` is the call's location)"),
    ("scan_raw_words", "`value` types (words below 2^48 read as variants without references)"),
];
/// Attributes on functions (also extern, trait and vm functions).
pub const FN_ATTRS: &[&str] = &["inline", "cold", "unsafe", "raw", "catch_panic", "c_const", "test", "track_caller", "may_alias", "shrinks"];
/// Attributes on `vm` declarations.
pub const VM_ATTRS: &[&str] = &["require_verified", "dispatch", "profile"];

/// Levenshtein distance, for "did you mean" suggestions.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The closest of `known` to `name`, if close enough to be a typo.
pub fn suggest<'a>(name: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (name.chars().count() / 3).max(1);
    known
        .into_iter()
        .map(|k| (edit_distance(name, k), k))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k)
}

/// Check the attributes of a construct (`what`, for messages: "a vm",
/// "a match arm") against the ones it accepts besides `allow`. An unknown
/// attribute is an error, never ignored: a misspelled
/// `#[require_verified]` would silently drop a check.
pub fn check_attrs(attrs: &Attrs, what: &str, known: &[&str]) -> Res<()> {
    for (n, args, sp) in &attrs.list {
        if n == "uses" {
            continue; // effects, recorded as attributes by `parse_fn`
        }
        if n == "allow" || known.contains(&n.as_str()) {
            let flag = matches!(n.as_str(), "cold" | "unsafe" | "raw" | "safe" | "thread_local" | "require_verified" | "forward" | "accumulate" | "test" | "track_caller" | "scan_raw_words" | "names");
            if flag && !args.is_empty() {
                return Err(Diag::new(*sp, format!("`#[{}]` takes no arguments", n)));
            }
            if n == "inline" && !(args.is_empty() || (args.len() == 1 && (args[0] == "always" || args[0] == "never"))) {
                return Err(Diag::new(*sp, "`#[inline]` takes no argument, `always` or `never`"));
            }
            // `#![lifetimes(off)]` opts a module out of the pointer-stability
            // check (on by default); a bare `#![lifetimes]` is the default.
            if n == "lifetimes" && !(args.is_empty() || (args.len() == 1 && args[0] == "off")) {
                return Err(Diag::new(*sp, "`#![lifetimes]` takes no argument or `off`: `#![lifetimes(off)]` turns the pointer-stability check off for the module"));
            }
            if n == "catch_panic" && args.len() > 1 {
                return Err(Diag::new(*sp, "`#[catch_panic]` takes at most one value: what the function returns after a panic"));
            }
            if n == "trace" && (args.len() > 1 || (args.is_empty() && what.contains("object"))) {
                return Err(Diag::new(*sp, "`#[trace(f)]` names one function of the same module: `fn f(h: *gc.Heap, obj: Gc<Obj>)`"));
            }
            if n == "c_name" && args.len() != 1 {
                return Err(Diag::new(*sp, "`#[c_name(\"name\")]` takes the C name"));
            }
            if n == "c_const" && args.is_empty() {
                return Err(Diag::new(*sp, "`#[c_const(p, ..)]` takes the pointer parameters that are `const` in C"));
            }
            if n == "profile" && args.len() != 1 {
                return Err(Diag::new(*sp, "`#[profile(\"file\")]` takes one file: `--vm-stats` output or a list of op names, hottest first"));
            }
            if n == "dispatch" && args.len() != 1 {
                return Err(Diag::new(*sp, "`#[dispatch(..)]` takes one dispatch mode: tailcall, goto or switch"));
            }
            continue;
        }
        let here: Vec<&str> = std::iter::once("allow").chain(known.iter().copied()).collect();
        let list = here.iter().map(|a| format!("`{}`", a)).collect::<Vec<_>>().join(", ");
        let msg = if let Some((_, place)) = ATTRS.iter().find(|(a, _)| a == n) {
            format!("attribute `{}` does not apply to {}: it is for {} (here: {})", n, what, place, list)
        } else if let Some(s) = suggest(n, here.iter().copied()) {
            format!("unknown attribute `{}` on {}; did you mean `{}`?", n, what, s)
        } else if let Some(s) = suggest(n, ATTRS.iter().map(|(a, _)| *a)) {
            let place = ATTRS.iter().find(|(a, _)| *a == s).unwrap().1;
            format!("unknown attribute `{}` on {}; did you mean `{}` (for {})? (here: {})", n, what, s, place, list)
        } else {
            format!("unknown attribute `{}` on {} (here: {})", n, what, list)
        };
        return Err(Diag::new(*sp, msg));
    }
    Ok(())
}

fn is_upper(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

impl Parser {
    pub fn new(toks: Vec<Token>) -> Parser {
        Parser {
            toks,
            pos: 0,
            no_struct: false,
            in_vm: false,
            cast_type: false,
            vm_loop_vars: Vec::new(),
            allows: Vec::new(),
            open_allows: Vec::new(),
        }
    }

    /// A closure literal: `|a, b: T| expr`, `|| expr`, `|a| -> R { .. }`.
    fn closure_lit(&mut self) -> Res<(Vec<(Ident, Option<TypeExpr>)>, Option<TypeExpr>, Expr)> {
        let mut params = Vec::new();
        if self.bump().tok == Tok::Punct("|") {
            while !self.is("|") {
                let name = self.ident()?;
                let ty = if self.eat(":") { Some(self.parse_type()?) } else { None };
                params.push((name, ty));
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("|")?;
        }
        let saved = self.no_struct;
        self.no_struct = false;
        let (ret, body) = if self.eat("->") {
            let t = self.parse_type()?;
            let b = self.parse_block()?;
            let span = b.span;
            (Some(t), Expr { kind: ExprKind::Block(b), span })
        } else {
            // The body may be an assignment: `|i| sum += i`.
            let e = self.parse_expr()?;
            (None, self.parse_assign_tail(e)?)
        };
        self.no_struct = saved;
        Ok((params, ret, body))
    }

    // ── token helpers ────────────────────────────────────────────────

    pub fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    pub fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }
    pub fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    pub fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }
    pub fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    pub fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    pub fn is_at(&self, n: usize, p: &str) -> bool {
        matches!(self.peek_at(n), Tok::Punct(q) if *q == p)
    }
    pub fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == k)
    }
    pub fn is_kw_at(&self, n: usize, k: &str) -> bool {
        matches!(self.peek_at(n), Tok::Ident(s) if s == k)
    }
    pub fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    pub fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    pub fn err<T>(&self, msg: impl Into<String>) -> Res<T> {
        Err(Diag::new(self.span(), msg))
    }
    fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("`{}`", s),
            Tok::Int(i) => format!("`{}`", i),
            Tok::Float(f) => format!("`{}`", f),
            Tok::Str(_) => "string literal".into(),
            Tok::Char(_) => "character literal".into(),
            Tok::Punct(p) => format!("`{}`", p),
            Tok::Eof => "end of file".into(),
        }
    }
    pub fn expect(&mut self, p: &str) -> Res<Span> {
        if self.is(p) {
            Ok(self.bump().span)
        } else {
            self.err(format!("expected `{}`, found {}", p, self.describe()))
        }
    }
    /// Before closing a type-argument list: split a `>>`, `>>=` or `>=`
    /// token so that `Vec<Box<T>>` closes both lists.
    fn split_angle(&mut self) {
        let rest = match self.peek() {
            Tok::Punct(">>") => ">",
            Tok::Punct(">>=") => ">=",
            Tok::Punct(">=") => "=",
            _ => return,
        };
        let t = &mut self.toks[self.pos];
        let mut second = t.clone();
        t.tok = Tok::Punct(">");
        t.span.hi = t.span.lo + 1;
        second.tok = Tok::Punct(rest);
        second.span.lo += 1;
        second.space_before = false;
        self.toks.insert(self.pos + 1, second);
    }
    pub fn expect_kw(&mut self, k: &str) -> Res<Span> {
        if self.is_kw(k) {
            Ok(self.bump().span)
        } else {
            self.err(format!("expected `{}`, found {}", k, self.describe()))
        }
    }
    pub fn ident(&mut self) -> Res<Ident> {
        match self.peek().clone() {
            Tok::Ident(name) if !is_reserved(&name) => {
                let span = self.bump().span;
                Ok(Ident { name, span })
            }
            Tok::Ident(name) => self.err(format!(
                "expected identifier, found `{}`, which is a reserved word (rename it, e.g. `{}_`)",
                name, name
            )),
            _ => self.err(format!("expected identifier, found {}", self.describe())),
        }
    }
    pub fn int_lit(&mut self) -> Res<i128> {
        let neg = self.eat("-");
        match *self.peek() {
            Tok::Int(v) => {
                self.bump();
                Ok(if neg { -(v as i128) } else { v as i128 })
            }
            Tok::Char(c) if !neg => {
                self.bump();
                Ok(c as i128)
            }
            _ => self.err(format!("expected integer, found {}", self.describe())),
        }
    }
    fn string_lit(&mut self) -> Res<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                Ok(String::from_utf8_lossy(&s).into_owned())
            }
            _ => self.err("expected string literal"),
        }
    }

    // ── items ────────────────────────────────────────────────────────

    /// `inline for (a, b) in [(x1, y1), (x2, y2)] { tokens }`: expand the
    /// body once per element, substituting each variable's tokens (wrapped
    /// in parentheses when more than one token). `#a` turns a string or
    /// identifier value into an identifier. The expansion is spliced into
    /// the token stream in place of the construct; with `stmts` (in a
    /// block), each copy is followed by `;`. Tokens keep their source
    /// spans, so errors in the expansion point into the original source.
    /// Returns the loop variables.
    pub fn expand_inline_for(&mut self, stmts: bool) -> Res<Vec<Ident>> {
        let start = self.pos;
        let span = self.span();
        self.expect_kw("inline")?;
        self.expect_kw("for")?;
        let mut idents = Vec::new();
        if self.eat("(") {
            while !self.is(")") {
                idents.push(self.ident()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        } else {
            idents.push(self.ident()?);
        }
        let vars: Vec<String> = idents.iter().map(|i| i.name.clone()).collect();
        self.expect_kw("in")?;
        self.expect("[")?;
        // Elements: balanced token groups separated by top-level commas.
        let mut elements: Vec<Vec<Vec<Token>>> = Vec::new();
        while !self.is("]") {
            let group = self.balanced_until(&[",", "]"])?;
            let values = if vars.len() > 1 {
                if group.len() < 2 || group[0].tok != Tok::Punct("(") {
                    return Err(Diag::new(group.first().map(|t| t.span).unwrap_or(span), "expected a tuple of values"));
                }
                split_top_level(&group[1..group.len() - 1])
            } else {
                vec![group]
            };
            if values.len() != vars.len() {
                return Err(Diag::new(span, format!("expected {} value(s) per element", vars.len())));
            }
            elements.push(values);
            if !self.eat(",") {
                break;
            }
        }
        self.expect("]")?;
        self.expect("{")?;
        let mut body = self.balanced_until(&["}"])?;
        self.expect("}")?;
        if body.is_empty() {
            body = vec![];
        }
        // A nested `inline for` naming one of these variables would have its
        // own variable replaced by our values.
        for (i, t) in body.iter().enumerate() {
            if t.tok == Tok::Ident("inline".into()) && body.get(i + 1).is_some_and(|n| n.tok == Tok::Ident("for".into())) {
                for b in body[i + 2..].iter().take_while(|b| b.tok != Tok::Ident("in".into())) {
                    if let Tok::Ident(v) = &b.tok {
                        if vars.contains(v) {
                            return Err(Diag::new(
                                b.span,
                                format!("nested `inline for` reuses the variable `{}` of an enclosing `inline for`", v),
                            ));
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        for values in &elements {
            let mut i = 0;
            while i < body.len() {
                let t = &body[i];
                if t.tok == Tok::Punct("#") && body.get(i + 1).is_some_and(|n| n.tok == Tok::Punct("(")) {
                    i = splice_concat(&body, i, &vars, values, &mut out)?;
                    continue;
                }
                if t.tok == Tok::Punct("#") {
                    if let Some(Token { tok: Tok::Ident(v), span, .. }) = body.get(i + 1) {
                        if let Some(k) = vars.iter().position(|x| x == v) {
                            let name = match values[k].as_slice() {
                                [Token { tok: Tok::Str(s), .. }] => String::from_utf8_lossy(s).into_owned(),
                                [Token { tok: Tok::Ident(s), .. }] => s.clone(),
                                _ => return Err(Diag::new(*span, format!("`#{}` needs a string or identifier value", v))),
                            };
                            out.push(Token { tok: Tok::Ident(name), span: *span, space_before: true });
                            i += 2;
                            continue;
                        }
                    }
                }
                if let Tok::Ident(v) = &t.tok {
                    if let Some(k) = vars.iter().position(|x| x == v) {
                        let val = &values[k];
                        let wrap = val.len() > 1;
                        if wrap {
                            out.push(Token { tok: Tok::Punct("("), span: t.span, space_before: true });
                        }
                        for vt in val {
                            out.push(Token { tok: vt.tok.clone(), span: vt.span, space_before: vt.space_before });
                        }
                        if wrap {
                            out.push(Token { tok: Tok::Punct(")"), span: t.span, space_before: false });
                        }
                        i += 1;
                        continue;
                    }
                }
                out.push(t.clone());
                i += 1;
            }
            if stmts {
                // Each copy ends a statement: `inline for r in [a, b] { f(r) }`
                // is `f(a); f(b);`.
                let sp = out.last().map(|t: &Token| t.span).unwrap_or(span);
                out.push(Token { tok: Tok::Punct(";"), span: sp, space_before: false });
            }
        }
        let end = self.pos;
        self.toks.splice(start..end, out);
        self.pos = start;
        Ok(idents)
    }

    /// Tokens up to (not including) one of `stops` at nesting depth 0.
    fn balanced_until(&mut self, stops: &[&str]) -> Res<Vec<Token>> {
        let mut depth = 0i32;
        let mut out = Vec::new();
        loop {
            let t = self.peek().clone();
            if t == Tok::Eof {
                return self.err("unexpected end of file in `inline for`");
            }
            if depth == 0 {
                if let Tok::Punct(p) = t {
                    if stops.contains(&p) {
                        return Ok(out);
                    }
                }
            }
            // In an element list, `Pair<i64, i64>` is one element: type
            // arguments, recognized as for `f<T>(..)` (`<` right after a
            // name, balanced and type-shaped), keep their commas.
            if depth == 0
                && stops.contains(&",")
                && t == Tok::Punct("<")
                && !self.toks[self.pos].space_before
                && matches!(out.last(), Some(Token { tok: Tok::Ident(_), .. }))
            {
                if let Some(end) = self.angle_args_end() {
                    while self.pos < end {
                        out.push(self.bump());
                    }
                    continue;
                }
            }
            match t {
                Tok::Punct("(") | Tok::Punct("[") | Tok::Punct("{") => depth += 1,
                Tok::Punct(")") | Tok::Punct("]") | Tok::Punct("}") => depth -= 1,
                _ => {}
            }
            out.push(self.bump());
        }
    }

    pub fn parse_module(&mut self) -> Res<(Vec<Item>, Attrs)> {
        let mut items = Vec::new();
        let mut mod_attrs = Attrs::default();
        while *self.peek() != Tok::Eof {
            if self.is_kw("inline") && self.is_kw_at(1, "for") {
                self.expand_inline_for(false)?;
                continue;
            }
            if self.is("#") && self.is_at(1, "!") {
                self.bump();
                self.bump();
                let first = mod_attrs.list.len();
                self.parse_attr_body(&mut mod_attrs)?;
                check_attrs(&Attrs { list: mod_attrs.list[first..].to_vec(), data: vec![] }, "a module (`#![..]`)", &["safe", "lifetimes"])?;
                continue;
            }
            let keep = self.take_cfg()?;
            let mark = self.open_allows.len();
            let item = self.parse_item()?;
            self.close_allows(mark);
            if keep {
                items.push(item);
            }
        }
        let methods = items
            .iter()
            .filter_map(|it| match it {
                Item::Struct(sd) if sd.derive_trace.is_some() => Some(trace_method(sd, &items)),
                _ => None,
            })
            .collect::<Res<Vec<_>>>()?;
        items.extend(methods.into_iter().map(Item::Fn));
        if let Some((_, _, sp)) = mod_attrs.list.iter().find(|(n, a, _)| n == "lifetimes" && !a.is_empty()).filter(|_| mod_attrs.has("safe")) {
            return Err(Diag::new(*sp, "a `#![safe]` module is always checked for pointer stability: `#![lifetimes(off)]` does not apply (use `unsafe { }` at the site)"));
        }
        // `#![allow(..)]`: the whole file.
        let file = self.toks.last().map(|t| t.span.file).unwrap_or(0);
        for (n, sp) in self.open_allows.drain(..) {
            self.allows.push((n, sp, Span { file, lo: 0, hi: u32::MAX }));
        }
        Ok((items, mod_attrs))
    }

    /// The point to pass to `close_allows` after parsing a construct.
    pub fn allow_mark(&self) -> usize {
        self.open_allows.len()
    }

    /// The attached construct of the allows opened since `mark` ends here.
    pub fn close_allows(&mut self, mark: usize) {
        let end = self.prev_span();
        for (n, sp) in self.open_allows.drain(mark..) {
            self.allows.push((n, sp, sp.to(end)));
        }
    }

    fn parse_attr_body(&mut self, attrs: &mut Attrs) -> Res<()> {
        self.expect("[")?;
        loop {
            let start = self.span();
            // `#[unsafe]` marks a function that only `unsafe { }` may name.
            let name = if self.is_kw("unsafe") { self.bump(); "unsafe".to_string() } else { self.ident()?.name };
            let mut args = Vec::new();
            if name == "data" && self.eat("(") {
                // `#[data(key: T = e, key = e, e)]`: entries are parsed as
                // types and expressions (src/enumgen.rs checks them).
                while !self.is(")") {
                    let lo = self.span();
                    let keyed = matches!(self.peek(), Tok::Ident(_)) && (self.is_at(1, ":") || self.is_at(1, "="));
                    let key = if keyed { Some(self.ident()?) } else { None };
                    let ty = if key.is_some() && self.eat(":") { Some(self.parse_type()?) } else { None };
                    let value = if key.is_none() || self.eat("=") { Some(self.parse_expr()?) } else { None };
                    attrs.data.push(DataArg { key, ty, value, span: lo.to(self.prev_span()) });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
            } else if self.eat("(") {
                while !self.is(")") {
                    let t = self.bump();
                    match t.tok {
                        Tok::Ident(s) => args.push(s),
                        Tok::Int(i) => args.push(i.to_string()),
                        // `#[catch_panic(-1)]`
                        Tok::Punct("-") if matches!(self.peek(), Tok::Int(_)) => {
                            let Tok::Int(i) = self.bump().tok else { unreachable!() };
                            args.push(format!("-{}", i));
                        }
                        Tok::Str(s) => args.push(String::from_utf8_lossy(&s).into_owned()),
                        Tok::Punct(",") => {}
                        _ => return Err(Diag::new(t.span, "unexpected token in attribute")),
                    }
                }
                self.expect(")")?;
            } else if self.eat("=") {
                // `#![heap = std.gc.MarkSweep]`
                let mut v = self.ident()?.name;
                while self.eat(".") {
                    v.push('.');
                    v.push_str(&self.ident()?.name);
                }
                args.push(v);
            }
            let sp = start.to(self.prev_span());
            if name == "allow" {
                if args.is_empty() {
                    return Err(Diag::new(sp, "`allow` needs a lint name: `#[allow(match_fallout)]`"));
                }
                self.open_allows.extend(args.iter().map(|a| (a.clone(), sp)));
            }
            attrs.list.push((name, args, sp));
            if !self.eat(",") {
                break;
            }
        }
        self.expect("]")?;
        Ok(())
    }

    /// A leading `#[cfg(NAME)]` or `#[cfg(not(NAME))]` (several may
    /// follow each other; all must hold): consumed, and whether what follows
    /// is kept. The parser drops what is not, before name resolution, so a
    /// build without the name compiles as if the code were not there (the
    /// IR is the same as without it).
    pub fn take_cfg(&mut self) -> Res<bool> {
        let mut keep = true;
        while self.is("#") && self.is_at(1, "[") && self.is_kw_at(2, "cfg") && self.is_at(3, "(") {
            for _ in 0..4 {
                self.bump();
            }
            let neg = self.is_kw("not") && self.is_at(1, "(");
            if neg {
                self.bump();
                self.bump();
            }
            let name = self.ident()?;
            if neg {
                self.expect(")")?;
            }
            self.expect(")")?;
            self.expect("]")?;
            keep &= cfg_enabled(&name.name) != neg;
        }
        Ok(keep)
    }

    pub fn parse_attrs(&mut self) -> Res<Attrs> {
        let mut attrs = Attrs::default();
        while self.is("#") && self.is_at(1, "[") {
            self.bump();
            self.parse_attr_body(&mut attrs)?;
        }
        Ok(attrs)
    }

    pub fn parse_item(&mut self) -> Res<Item> {
        let attrs = self.parse_attrs()?;
        let is_pub = self.eat_kw("pub");
        let kw = match self.peek() {
            Tok::Ident(s) => s.clone(),
            _ => String::new(),
        };
        let (what, known): (String, &[&str]) = match kw.as_str() {
            "fn" | "extern" | "export" => ("a function".into(), FN_ATTRS),
            "var" => ("a module-level `var`".into(), &["thread_local"]),
            "vm" => ("a vm".into(), VM_ATTRS),
            "enum" => ("an `enum`".into(), &["c_name", "data", "names"]),
            "struct" => ("a `struct`".into(), &["c_name", "trace"]),
            "object" => ("an `object`".into(), &["trace"]),
            "value" => ("a `value` type".into(), &["scan_raw_words"]),
            "impl" | "import" => (format!("an `{}`", kw), &[]),
            "" => ("an item".into(), &[]),
            _ => (format!("a `{}`", kw), &[]),
        };
        check_attrs(&attrs, &what, known)?;
        if self.is_kw("fn") {
            return Ok(Item::Fn(self.parse_fn(attrs, Linkage::Internal, is_pub)?));
        }
        if self.is_kw("extern") || self.is_kw("export") {
            let linkage = if self.bump().tok == Tok::Ident("extern".into()) { Linkage::Extern } else { Linkage::Export };
            let abi = self.string_lit()?;
            if abi != "C" {
                return self.err("only the \"C\" ABI is supported");
            }
            // `export "C" const NAME: T = v;`: a constant in the C header.
            if linkage == Linkage::Export && self.eat_kw("const") {
                if !attrs.list.is_empty() {
                    return Err(Diag::new(attrs.list[0].2, "an exported constant takes no attributes"));
                }
                let name = self.ident()?;
                self.expect(":")?;
                let ty = self.parse_type()?;
                self.expect("=")?;
                let value = self.parse_expr()?;
                self.expect(";")?;
                return Ok(Item::Const(ConstDecl { name, ty, value, is_pub: true, export: true }));
            }
            return Ok(Item::Fn(self.parse_fn(attrs, linkage, is_pub)?));
        }
        if self.is_kw("struct") || self.is_kw("object") {
            let is_object = self.bump().tok == Tok::Ident("object".into());
            let name = self.ident()?;
            let (generics, bounds) = self.parse_generics()?;
            self.expect("{")?;
            let mut fields = Vec::new();
            let mut untraced = Vec::new();
            while !self.is("}") {
                let keep = self.take_cfg()?;
                let mark = self.allow_mark();
                let fattrs = self.parse_attrs()?;
                check_attrs(&fattrs, "a field", &["untraced"])?;
                let f = self.ident()?;
                self.expect(":")?;
                let t = if self.is_kw("verified") && self.is_at(1, "[") {
                    // `code: verified [u32] by Mini`
                    let start = self.bump().span;
                    let inner = self.parse_type()?;
                    let TypeKind::Trailing(elem) = inner.kind else {
                        return Err(Diag::new(inner.span, "`verified` marks a trailing array: `code: verified [u32] by Vm`"));
                    };
                    self.expect_kw("by")?;
                    let mut path = vec![self.ident()?];
                    while self.eat(".") {
                        path.push(self.ident()?);
                    }
                    TypeExpr { kind: TypeKind::VerifiedTrailing(elem, path), span: start.to(self.prev_span()) }
                } else {
                    self.parse_type()?
                };
                if keep {
                    if let Some((_, _, sp)) = fattrs.list.iter().find(|(n, _, _)| n == "untraced") {
                        if is_object || !attrs.list.iter().any(|(n, a, _)| n == "trace" && a.is_empty()) {
                            return Err(Diag::new(*sp, "`#[untraced]` is for fields of a `#[trace] struct`"));
                        }
                        untraced.push(f.clone());
                    }
                    fields.push((f, t));
                }
                self.close_allows(mark);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}")?;
            let c_name = attrs.get("c_name").map(|a| a[0].clone());
            let mut derive_trace = None;
            if let Some((_, a, sp)) = attrs.list.iter().find(|(n, _, _)| n == "trace") {
                if !is_object && !a.is_empty() {
                    return Err(Diag::new(*sp, "`#[trace]` on a struct takes no function: it generates `S.trace(s, heap)`"));
                }
                if is_object && a.is_empty() {
                    return Err(Diag::new(*sp, "`#[trace(f)]` on an object names the function that traces its trailing array"));
                }
                if !is_object && !generics.is_empty() {
                    return Err(Diag::new(*sp, "`#[trace]` is for structs without type parameters"));
                }
                if !is_object {
                    derive_trace = Some(*sp);
                }
            }
            let trace = attrs.list.iter().find(|(n, _, _)| n == "trace").filter(|_| is_object).map(|(_, a, sp)| Ident { name: a[0].clone(), span: *sp });
            return Ok(Item::Struct(StructDecl { name, generics, bounds, is_object, fields, is_pub, c_name, trace, derive_trace, untraced }));
        }
        if self.is_kw("trait") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            let name = self.ident()?;
            let (generics, bounds) = self.parse_generics()?;
            if let Some(b) = bounds.iter().flatten().next() {
                return Err(Diag::new(b.span, "a trait's type parameters take no bounds"));
            }
            self.expect("{")?;
            let mut methods = Vec::new();
            while !self.is("}") {
                let mark = self.allow_mark();
                let attrs = self.parse_attrs()?;
                check_attrs(&attrs, "a function", FN_ATTRS)?;
                let f = self.parse_fn(attrs, Linkage::Internal, false)?;
                self.close_allows(mark);
                if f.recv.is_some() {
                    return Err(Diag::new(
                        f.name.span,
                        "a trait method is written `fn name(a: Self, ..) -> R;` or with a default body, without a receiver type",
                    ));
                }
                methods.push(f);
            }
            self.expect("}")?;
            return Ok(Item::Trait(TraitDecl { name, generics, methods, is_pub }));
        }
        if self.is_kw("impl") && (matches!(self.peek_at(1), Tok::Ident(_)) || self.is_at(1, "<")) {
            let start = self.span();
            self.bump();
            let (generics, bounds) = self.parse_generics()?;
            let trait_ref = self.parse_trait_ref()?;
            self.expect_kw("for")?;
            let ty = self.parse_type()?;
            self.expect(";")?;
            if is_pub {
                return self.err("`impl` takes no `pub`: impls are visible everywhere");
            }
            return Ok(Item::Impl(ImplDecl { generics, bounds, trait_ref, ty, span: start.to(self.prev_span()) }));
        }
        if self.is_kw("type") && matches!(self.peek_at(1), Tok::Ident(_)) {
            // `type Name = T;`, `type Map<V> = HashMap<str, V>;`
            self.bump();
            let name = self.ident()?;
            let (generics, bounds) = self.parse_generics()?;
            if let Some(b) = bounds.iter().flatten().next() {
                return Err(Diag::new(
                    b.span,
                    "a type alias's parameters take no bounds; the aliased type's own bounds are checked where it is used",
                ));
            }
            self.expect("=")?;
            let ty = self.parse_type()?;
            self.expect(";")?;
            return Ok(Item::TypeAlias(TypeAliasDecl { name, generics, ty, is_pub }));
        }
        if self.eat_kw("enum") {
            let name = self.ident()?;
            let (generics, bounds) = self.parse_generics()?;
            let repr = if self.eat(":") { Some(self.parse_type()?) } else { None };
            self.expect("{")?;
            let mut variants = Vec::new();
            while !self.is("}") {
                let fattrs = self.parse_attrs()?;
                check_attrs(&fattrs, "an enum variant", &["name", "data"])?;
                let vname = self.ident()?;
                let mut fields = Vec::new();
                if self.eat("(") {
                    while !self.is(")") {
                        fields.push(self.parse_type()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                }
                let discr = if self.eat("=") { Some(self.int_lit()?) } else { None };
                variants.push(Variant { name: vname, fields, discr, attrs: fattrs });
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}")?;
            let c_name = attrs.get("c_name").map(|a| a[0].clone());
            return Ok(Item::Enum(EnumDecl { name, generics, bounds, repr, variants, is_pub, c_name, attrs }));
        }
        if self.eat_kw("var") {
            let name = self.ident()?;
            self.expect(":")?;
            let ty = self.parse_type()?;
            if self.is(";") {
                return self.err("a module-level `var` needs an initializer (a constant expression)");
            }
            self.expect("=")?;
            let value = self.parse_expr()?;
            self.expect(";")?;
            let thread_local = attrs.has("thread_local");
            return Ok(Item::Global(GlobalDecl { name, ty, value, is_pub, thread_local }));
        }
        if self.eat_kw("const") {
            let name = self.ident()?;
            self.expect(":")?;
            let ty = self.parse_type()?;
            self.expect("=")?;
            let value = self.parse_expr()?;
            self.expect(";")?;
            return Ok(Item::Const(ConstDecl { name, ty, value, is_pub, export: false }));
        }
        if self.is_kw("import") {
            let start = self.bump().span;
            let mut path = vec![self.ident()?];
            let mut names = None;
            while self.eat(".") {
                if self.eat("{") {
                    let mut ns = Vec::new();
                    while !self.is("}") {
                        ns.push(self.ident()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("}")?;
                    names = Some(ns);
                    break;
                }
                path.push(self.ident()?);
            }
            self.expect(";")?;
            return Ok(Item::Import(Import { path, names, span: start.to(self.prev_span()) }));
        }
        if self.is_kw("vm") {
            return Ok(Item::Vm(Box::new(self.parse_vm(attrs, is_pub)?)));
        }
        if self.is_kw("value") {
            let mut v = self.parse_value_decl(is_pub)?;
            v.scan_raw = attrs.list.iter().find(|a| a.0 == "scan_raw_words").map(|a| a.2);
            return Ok(Item::Value(v));
        }
        self.err(format!("expected an item, found {}", self.describe()))
    }

    pub fn parse_fn(&mut self, attrs: Attrs, linkage: Linkage, is_pub: bool) -> Res<FnDecl> {
        self.expect_kw("fn")?;
        let first = self.ident()?;
        let (mut generics, mut bounds) = self.parse_generics()?;
        // `fn Type.name<U>` / `fn Vec<T>.name<U>`: the method's own type
        // parameters follow the receiver's.
        let mut recv_generics = 0;
        let (recv, name) = if self.eat(".") {
            recv_generics = generics.len();
            let name = self.ident()?;
            let (g, b) = self.parse_generics()?;
            generics.extend(g);
            bounds.extend(b);
            (Some(first), name)
        } else {
            (None, first)
        };
        self.expect("(")?;
        let mut params = Vec::new();
        let mut variadic = false;
        while !self.is(")") {
            if self.eat("...") {
                variadic = true;
                break;
            }
            let pname = self.ident()?;
            self.expect(":")?;
            let ty = self.parse_type()?;
            params.push(Param { name: pname, ty });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let ret = if self.eat("->") { Some(self.parse_type()?) } else { None };
        // Effects: `uses gc, reenter` (parsed and recorded as attributes).
        let mut attrs = attrs;
        if self.eat_kw("uses") {
            loop {
                let e = self.ident()?;
                attrs.list.push(("uses".into(), vec![e.name], e.span));
                if !self.eat(",") {
                    break;
                }
            }
        }
        let body = if self.eat(";") { None } else { Some(self.parse_block()?) };
        Ok(FnDecl { name, generics, bounds, recv_generics, recv, params, variadic, ret, body, linkage, attrs, is_pub, link_name: None })
    }

    /// `<T, U: A + m.B + Hooks<T>>`: the parameters and the bounds of each.
    fn parse_generics(&mut self) -> Res<(Vec<Ident>, Vec<Bounds>)> {
        let mut gs = Vec::new();
        let mut bs = Vec::new();
        if self.eat("<") {
            while !self.is(">") {
                gs.push(self.ident()?);
                let mut b = Vec::new();
                if self.eat(":") {
                    loop {
                        b.push(self.parse_trait_ref()?);
                        if !self.eat("+") {
                            break;
                        }
                    }
                }
                bs.push(b);
                self.split_angle();
                if !self.eat(",") {
                    break;
                }
            }
            self.split_angle();
            self.expect(">")?;
        }
        Ok((gs, bs))
    }

    /// `Hash`, `hash.Hash`, `Hooks<K, E>`.
    fn parse_trait_ref(&mut self) -> Res<TraitRef> {
        let start = self.span();
        let mut path = vec![self.ident()?];
        while self.eat(".") {
            path.push(self.ident()?);
        }
        let mut args = Vec::new();
        if self.eat("<") {
            self.split_angle();
            while !self.is(">") {
                args.push(self.parse_type()?);
                self.split_angle();
                if !self.eat(",") {
                    break;
                }
            }
            self.split_angle();
            self.expect(">")?;
        }
        Ok(TraitRef { path, args, span: start.to(self.prev_span()) })
    }

    // ── types ────────────────────────────────────────────────────────

    /// At a `<`: if a balanced, type-shaped `<...>` follows, the index of
    /// the token after its closing `>`. Does not consume anything.
    fn angle_args_end(&self) -> Option<usize> {
        if !self.is("<") {
            return None;
        }
        let mut depth = 0i32;
        let mut i = self.pos;
        loop {
            let t = &self.toks.get(i)?.tok;
            match t {
                Tok::Punct("<") => depth += 1,
                Tok::Punct(">") => depth -= 1,
                Tok::Punct(">>") => depth -= 2,
                Tok::Punct(",") | Tok::Punct(".") | Tok::Punct("*") | Tok::Punct("?") | Tok::Punct("[")
                | Tok::Punct("]") | Tok::Punct(";") | Tok::Punct("(") | Tok::Punct(")") | Tok::Punct("->") => {}
                Tok::Int(_) => {}
                Tok::Ident(s) if !is_reserved(s) || matches!(s.as_str(), "fn" | "never") => {}
                _ => return None,
            }
            i += 1;
            if depth == 0 {
                return Some(i);
            }
            if depth < 0 {
                return None;
            }
        }
    }

    /// The type after `as`. A primitive type never takes type arguments,
    /// so `x as u64 < y` is a comparison. After another type name, `<`
    /// starts type arguments only if a balanced `<...>` of type tokens
    /// follows (`p as *Vec<T>`); otherwise it is a comparison. Write
    /// `(x as T) < y` to be explicit.
    fn parse_cast_type(&mut self) -> Res<TypeExpr> {
        let saved = self.cast_type;
        self.cast_type = true;
        let r = self.parse_type();
        self.cast_type = saved;
        r
    }

    pub fn parse_type(&mut self) -> Res<TypeExpr> {
        if self.in_vm && self.is_kw("cache") && matches!(self.peek_at(1), Tok::Ident(_)) {
            let start = self.bump().span;
            let name = self.ident()?;
            return Ok(TypeExpr { kind: TypeKind::Cache(name), span: start.to(self.prev_span()) });
        }
        let start = self.span();
        if self.is("??") {
            // `??T` is `?(?T)`: split the token.
            let t = &mut self.toks[self.pos];
            let mut second = t.clone();
            t.tok = Tok::Punct("?");
            t.span.hi = t.span.lo + 1;
            second.tok = Tok::Punct("?");
            second.span.lo += 1;
            second.space_before = false;
            self.toks.insert(self.pos + 1, second);
        }
        let kind = if self.eat("*") {
            let _ = self.eat_kw("mut") || self.eat_kw("const");
            TypeKind::Ptr(Box::new(self.parse_type()?))
        } else if self.eat("?") {
            TypeKind::Option(Box::new(self.parse_type()?))
        } else if self.eat("[") {
            if self.eat("]") {
                TypeKind::Slice(Box::new(self.parse_type()?))
            } else {
                let elem = self.parse_type()?;
                if self.eat("]") {
                    return Ok(TypeExpr { kind: TypeKind::Trailing(Box::new(elem)), span: start.to(self.prev_span()) });
                }
                self.expect(";")?;
                let n = self.parse_expr()?;
                self.expect("]")?;
                TypeKind::Array(Box::new(elem), Box::new(n))
            }
        } else if self.eat("(") {
            let mut ts = Vec::new();
            let mut comma = false;
            while !self.is(")") {
                ts.push(self.parse_type()?);
                comma = self.eat(",");
                if !comma {
                    break;
                }
            }
            self.expect(")")?;
            // `(T)` is `T` (for example `-> (fn() uses gc)`); `(T,)` is a 1-tuple.
            if ts.len() == 1 && !comma {
                let mut t = ts.pop().unwrap();
                t.span = start.to(self.prev_span());
                return Ok(t);
            }
            TypeKind::Tuple(ts)
        } else if self.is_kw("block") && self.is_kw_at(1, "fn") {
            self.bump();
            let t = self.parse_type()?;
            let TypeKind::Fn(ps, ret, eff) = t.kind else { unreachable!() };
            TypeKind::Block(ps, ret, eff)
        } else if self.is_kw("closure") && self.is_kw_at(1, "fn") {
            self.bump();
            let t = self.parse_type()?;
            let TypeKind::Fn(ps, ret, eff) = t.kind else { unreachable!() };
            TypeKind::Closure(ps, ret, eff)
        } else if self.eat_kw("fn") {
            self.expect("(")?;
            let mut ps = Vec::new();
            while !self.is(")") {
                ps.push(self.parse_type()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            let ret = if self.eat("->") {
                self.parse_type()?
            } else {
                TypeExpr { kind: TypeKind::Tuple(vec![]), span: self.prev_span() }
            };
            // `uses E` binds to the innermost function type: in
            // `fn f() -> fn() uses gc`, the effect belongs to the result
            // type. A function type takes one effect, since they are ordered.
            let eff = if self.eat_kw("uses") { Some(self.ident()?) } else { None };
            TypeKind::Fn(ps, Box::new(ret), eff)
        } else if self.eat_kw("never") {
            TypeKind::Never
        } else {
            let mut path = vec![self.ident()?];
            while self.is(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                self.bump();
                path.push(self.ident()?);
            }
            let mut args = Vec::new();
            let comparison = self.cast_type
                && self.is("<")
                && ((path.len() == 1 && crate::check::prim_type(&path[0].name).is_some()) || self.angle_args_end().is_none());
            let cast_type = std::mem::replace(&mut self.cast_type, false);
            if !comparison && self.eat("<") {
                self.split_angle();
                while !self.is(">") {
                    args.push(self.parse_type()?);
                    self.split_angle();
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(">")?;
            }
            self.cast_type = cast_type;
            TypeKind::Named(path, args)
        };
        Ok(TypeExpr { kind, span: start.to(self.prev_span()) })
    }

    // ── blocks and statements ────────────────────────────────────────

    pub fn parse_block(&mut self) -> Res<Block> {
        let start = self.expect("{")?;
        let saved = self.no_struct;
        self.no_struct = false;
        let mut stmts = Vec::new();
        while !self.is("}") {
            if *self.peek() == Tok::Eof {
                return self.err("unexpected end of file in block");
            }
            if self.eat(";") {
                continue;
            }
            if self.is_kw("inline") && self.is_kw_at(1, "for") {
                // Statements: the expansion becomes statements of this
                // block (no scope of its own).
                let vars = self.expand_inline_for(true)?;
                if self.in_vm {
                    self.vm_loop_vars.extend(vars);
                }
                continue;
            }
            let keep = self.take_cfg()?;
            let st = self.parse_stmt()?;
            if keep {
                stmts.push(st);
            }
        }
        let end = self.expect("}")?;
        self.no_struct = saved;
        Ok(Block { stmts, span: start.to(end) })
    }

    fn parse_stmt(&mut self) -> Res<Stmt> {
        let start = self.span();
        if self.is_kw("let") || self.is_kw("var") {
            let mutable = self.bump().tok == Tok::Ident("var".into());
            let pat = self.parse_pat()?;
            let ty = if self.eat(":") { Some(self.parse_type()?) } else { None };
            let init = if self.eat("=") { Some(self.parse_expr()?) } else { None };
            let els = if self.eat_kw("else") {
                if self.is_kw("cold") && self.is_at(1, "{") {
                    // `let P = e else cold { ... }`
                    let cs = self.bump().span;
                    let b = self.parse_block()?;
                    let span = cs.to(b.span);
                    Some(Block {
                        stmts: vec![Stmt::Expr {
                            expr: Expr { kind: ExprKind::Vm(VmExpr::Cold(b)), span },
                            semi: false,
                        }],
                        span,
                    })
                } else {
                    Some(self.parse_block()?)
                }
            } else {
                None
            };
            self.expect(";")?;
            return Ok(Stmt::Let { pat, mutable, ty, init, els, span: start.to(self.prev_span()) });
        }
        if self.eat_kw("defer") {
            let e = self.parse_expr_stmt_inner()?;
            self.eat(";");
            return Ok(Stmt::Defer(e));
        }
        let expr = self.parse_expr_stmt_inner()?;
        if self.eat(";") {
            return Ok(Stmt::Expr { expr, semi: true });
        }
        if self.is("}") {
            return Ok(Stmt::Expr { expr, semi: false });
        }
        if is_block_like(&expr) {
            return Ok(Stmt::Expr { expr, semi: true });
        }
        self.err(format!("expected `;` or `}}`, found {}{}", self.describe(), self.operator_hint()))
    }

    fn starts_block_like(&self) -> bool {
        self.is("{")
            || matches!(self.peek(), Tok::Ident(s) if matches!(s.as_str(), "if" | "match" | "while" | "loop" | "for"))
            || ((self.is_kw("unsafe") || self.is_kw("cold")) && self.is_at(1, "{"))
    }

    fn parse_expr_stmt_inner(&mut self) -> Res<Expr> {
        // Block-like expressions in statement position end at their `}`.
        if self.starts_block_like() {
            return self.parse_primary();
        }
        let lhs = self.parse_expr()?;
        self.parse_assign_tail(lhs)
    }

    /// After an expression: `= rhs` or `op= rhs` makes it an assignment.
    fn parse_assign_tail(&mut self, lhs: Expr) -> Res<Expr> {
        let start = lhs.span;
        let op = match self.peek() {
            Tok::Punct("=") => None,
            Tok::Punct(p) => match *p {
                "+=" => Some(BinOp::Add),
                "-=" => Some(BinOp::Sub),
                "*=" => Some(BinOp::Mul),
                "/=" => Some(BinOp::Div),
                "%=" => Some(BinOp::Rem),
                "&=" => Some(BinOp::And),
                "|=" => Some(BinOp::Or),
                "^=" => Some(BinOp::Xor),
                "<<=" => Some(BinOp::Shl),
                ">>=" => Some(BinOp::Shr),
                "+%=" => Some(BinOp::AddWrap),
                "-%=" => Some(BinOp::SubWrap),
                "*%=" => Some(BinOp::MulWrap),
                _ => return Ok(lhs),
            },
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.parse_expr()?;
        let span = start.to(rhs.span);
        let kind = match op {
            None => ExprKind::Assign(Box::new(lhs), Box::new(rhs)),
            Some(op) => ExprKind::CompoundAssign(op, Box::new(lhs), Box::new(rhs)),
        };
        Ok(Expr { kind, span })
    }

    // ── expressions ──────────────────────────────────────────────────

    pub fn parse_expr(&mut self) -> Res<Expr> {
        self.parse_bin(0)
    }

    fn parse_expr_no_struct(&mut self) -> Res<Expr> {
        let saved = self.no_struct;
        self.no_struct = true;
        let r = self.parse_expr();
        self.no_struct = saved;
        r
    }

    fn binop(&self) -> Option<(BinOp, u8)> {
        let t = self.peek();
        let op = match t {
            Tok::Ident(s) if s == "or" => (BinOp::LOr, 2),
            Tok::Ident(s) if s == "and" => (BinOp::LAnd, 3),
            Tok::Punct(p) => match *p {
                "||" => (BinOp::LOr, 2),
                "&&" => (BinOp::LAnd, 3),
                "==" => (BinOp::Eq, 4),
                "!=" => (BinOp::Ne, 4),
                "<" => (BinOp::Lt, 4),
                "<=" => (BinOp::Le, 4),
                ">" => (BinOp::Gt, 4),
                ">=" => (BinOp::Ge, 4),
                "|" => (BinOp::Or, 5),
                "^" => (BinOp::Xor, 6),
                "&" => (BinOp::And, 7),
                "<<" => (BinOp::Shl, 8),
                ">>" => (BinOp::Shr, 8),
                "+" => (BinOp::Add, 9),
                "-" => (BinOp::Sub, 9),
                "+%" => (BinOp::AddWrap, 9),
                "-%" => (BinOp::SubWrap, 9),
                "*" => (BinOp::Mul, 10),
                "/" => (BinOp::Div, 10),
                "%" => (BinOp::Rem, 10),
                "*%" => (BinOp::MulWrap, 10),
                _ => return None,
            },
            _ => return None,
        };
        Some(op)
    }

    fn parse_bin(&mut self, min: u8) -> Res<Expr> {
        let mut lhs = if min <= 1 { self.parse_coalesce()? } else { self.parse_cast()? };
        if min <= 1 {
            return Ok(lhs);
        }
        loop {
            // `e is P`, at the precedence of comparisons. `is` is not a
            // keyword: after an operand it can only be this.
            if self.is_kw("is") && min <= 4 {
                self.bump();
                let pat = self.parse_pat()?;
                let span = lhs.span.to(pat.span);
                lhs = Expr { kind: ExprKind::Is(Box::new(lhs), pat), span };
                continue;
            }
            let Some((op, prec)) = self.binop() else { break };
            if prec < min {
                break;
            }
            // `cond and let P = e` is a let-chain, handled by parse_conds.
            if op == BinOp::LAnd && self.is_kw_at(1, "let") {
                break;
            }
            self.bump();
            let rhs = self.parse_bin(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span };
        }
        Ok(lhs)
    }

    fn parse_coalesce(&mut self) -> Res<Expr> {
        let lhs = self.parse_bin(2)?;
        if self.eat("??") {
            let rhs = self.parse_coalesce()?;
            let span = lhs.span.to(rhs.span);
            return Ok(Expr { kind: ExprKind::Coalesce(Box::new(lhs), Box::new(rhs)), span });
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Res<Expr> {
        let start = self.span();
        let op = if self.is("-") {
            Some(UnOp::Neg)
        } else if self.is("!") {
            Some(UnOp::Not)
        } else if self.is("~") {
            Some(UnOp::BitNot)
        } else if self.is("*") {
            Some(UnOp::Deref)
        } else if self.is("&") {
            Some(UnOp::AddrOf)
        } else {
            None
        };
        if let Some(op) = op {
            self.bump();
            if op == UnOp::AddrOf {
                let _ = self.eat_kw("mut");
            }
            let e = self.parse_unary()?;
            // Fold negative literals so that `-128 as i8` and i64::MIN work.
            if op == UnOp::Neg {
                if let ExprKind::Float(f) = e.kind {
                    return Ok(Expr { kind: ExprKind::Float(-f), span: start.to(e.span) });
                }
            }
            let span = start.to(e.span);
            return Ok(Expr { kind: ExprKind::Unary(op, Box::new(e)), span });
        }
        self.parse_postfix()
    }

    /// Unary expression followed by `as` casts (casts bind looser than unary operators).
    fn parse_cast(&mut self) -> Res<Expr> {
        let mut e = self.parse_unary()?;
        while self.eat_kw("as") {
            // `e as! V`: checked unwrap of a variant.
            if self.is("!") {
                self.bump();
                let mut path = Vec::new();
                if self.is(".") {
                    path.push(Ident { name: String::new(), span: self.bump().span });
                }
                path.push(self.ident()?);
                while self.is(".") {
                    self.bump();
                    path.push(self.ident()?);
                }
                let span = e.span.to(self.prev_span());
                e = Expr { kind: ExprKind::AsVariant(Box::new(e), path), span };
                continue;
            }
            let t = self.parse_cast_type()?;
            let span = e.span.to(t.span);
            e = Expr { kind: ExprKind::Cast(Box::new(e), t), span };
        }
        Ok(e)
    }

    fn parse_postfix(&mut self) -> Res<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            if self.is("(") {
                self.bump();
                let args = self.parse_args(")")?;
                let span = e.span.to(self.prev_span());
                e = Expr { kind: ExprKind::Call(Box::new(e), args), span };
            } else if self.is(".") && !self.is_at(1, "{") {
                self.bump();
                match self.peek().clone() {
                    Tok::Int(i) => {
                        let sp = self.bump().span;
                        e = field(e, Ident { name: i.to_string(), span: sp });
                    }
                    Tok::Float(_) => {
                        // `t.0.1` lexes as `t` `.` `0.1`
                        let t = self.bump();
                        let src = format!("{}", match t.tok {
                            Tok::Float(f) => f,
                            _ => unreachable!(),
                        });
                        for part in src.split('.') {
                            e = field(e, Ident { name: part.to_string(), span: t.span });
                        }
                    }
                    _ => {
                        let name = self.ident()?;
                        // `module.Type { .. }`: a struct literal of an imported type.
                        if let ExprKind::Ident(m) = &e.kind {
                            if is_upper(&name.name) && self.is("{") && !self.no_struct {
                                let module = Ident { name: m.clone(), span: e.span };
                                let start = e.span;
                                e = self.parse_struct_lit(vec![module, name], start)?;
                                continue;
                            }
                        }
                        e = field(e, name);
                    }
                }
            } else if self.is("[") {
                self.bump();
                if self.eat("..") {
                    let hi = if self.is("]") { None } else { Some(Box::new(self.parse_expr()?)) };
                    self.expect("]")?;
                    let span = e.span.to(self.prev_span());
                    e = Expr { kind: ExprKind::SliceRange(Box::new(e), None, hi), span };
                    continue;
                }
                let idx = self.parse_expr()?;
                if self.eat("..") {
                    let hi = if self.is("]") { None } else { Some(Box::new(self.parse_expr()?)) };
                    self.expect("]")?;
                    let span = e.span.to(self.prev_span());
                    e = Expr { kind: ExprKind::SliceRange(Box::new(e), Some(Box::new(idx)), hi), span };
                    continue;
                }
                self.expect("]")?;
                let span = e.span.to(self.prev_span());
                e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(idx)), span };
            } else if self.is("<")
                && !self.toks[self.pos].space_before
                && matches!(e.kind, ExprKind::Ident(_) | ExprKind::Field(..))
                && self.angle_args_end().is_some_and(|i| {
                    matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Punct("(" | ")" | "," | ";" | "}" | "]")))
                })
            {
                // `f<T>(..)`, `mem.alloc<u8>(n)`, `v.map<f64>(g)`: explicit
                // type arguments of a generic call. Also `f<T>` as a value,
                // when followed by a token that cannot continue a
                // comparison (`)`, `,`, `;`, `}`, `]`).
                let saved = std::mem::replace(&mut self.cast_type, false);
                self.bump();
                let mut targs = Vec::new();
                self.split_angle();
                while !self.is(">") {
                    targs.push(self.parse_type()?);
                    self.split_angle();
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(">")?;
                self.cast_type = saved;
                let span = e.span.to(self.prev_span());
                e = Expr { kind: ExprKind::TypeArgs(Box::new(e), targs), span };
            } else if self.is("?") && !self.toks[self.pos].space_before {
                self.bump();
                let span = e.span.to(self.prev_span());
                e = Expr { kind: ExprKind::Try(Box::new(e)), span };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_args(&mut self, close: &str) -> Res<Vec<Expr>> {
        let saved = self.no_struct;
        self.no_struct = false;
        let mut args = Vec::new();
        while !self.is(close) {
            args.push(self.parse_expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(close)?;
        self.no_struct = saved;
        Ok(args)
    }

    fn parse_conds(&mut self) -> Res<Vec<Cond>> {
        let saved = self.no_struct;
        self.no_struct = true;
        let mut conds = Vec::new();
        loop {
            if self.eat_kw("let") {
                let pat = self.parse_pat()?;
                self.expect("=")?;
                // Stop before `and`: parse above the logical operators.
                let e = self.parse_bin(4)?;
                conds.push(Cond::Let(pat, e));
            } else {
                conds.push(Cond::Expr(self.parse_bin(2)?));
            }
            if self.is_kw("and") || self.is("&&") {
                self.bump();
                continue;
            }
            break;
        }
        self.no_struct = saved;
        Ok(conds)
    }

    fn parse_primary(&mut self) -> Res<Expr> {
        let start = self.span();
        let tok = self.peek().clone();
        let mk = |kind, p: &Parser| Ok(Expr { kind, span: start.to(p.prev_span()) });
        match tok {
            Tok::Int(v) => {
                self.bump();
                mk(ExprKind::Int(v), self)
            }
            Tok::Float(v) => {
                self.bump();
                mk(ExprKind::Float(v), self)
            }
            Tok::Str(s) => {
                self.bump();
                mk(ExprKind::Str(s), self)
            }
            Tok::Char(c) => {
                self.bump();
                mk(ExprKind::Char(c), self)
            }
            Tok::Punct("(") => {
                self.bump();
                if self.eat(")") {
                    return mk(ExprKind::Unit, self);
                }
                let saved = self.no_struct;
                self.no_struct = false;
                let first = self.parse_expr()?;
                if self.eat(")") {
                    self.no_struct = saved;
                    return Ok(Expr { kind: first.kind, span: start.to(self.prev_span()) });
                }
                let mut es = vec![first];
                while self.eat(",") {
                    if self.is(")") {
                        break;
                    }
                    es.push(self.parse_expr()?);
                }
                self.expect(")")?;
                self.no_struct = saved;
                mk(ExprKind::Tuple(es), self)
            }
            Tok::Punct("|") | Tok::Punct("||") => {
                let (params, ret, body) = self.closure_lit()?;
                mk(ExprKind::Closure(params, ret, Box::new(body), vec![]), self)
            }
            Tok::Punct("[") if self.is_kw_at(1, "var") => {
                // An escaping closure's state list: `[var n = e, var m] |..| body`.
                self.bump();
                let mut state = Vec::new();
                loop {
                    if !self.eat_kw("var") {
                        return self.err("expected `var` in a closure's state list (`[var n = 0, var m] || ..`)");
                    }
                    let name = self.ident()?;
                    let ty = if self.eat(":") { Some(self.parse_type()?) } else { None };
                    let init = if self.eat("=") { Some(self.parse_expr()?) } else { None };
                    state.push((name, ty, init));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("]")?;
                if !self.is("|") && !self.is("||") {
                    return self.err("expected a closure literal after its state list (`[var n = 0] || ..`)");
                }
                let (params, ret, body) = self.closure_lit()?;
                mk(ExprKind::Closure(params, ret, Box::new(body), state), self)
            }
            Tok::Punct("[") => {
                self.bump();
                if self.eat("]") {
                    return mk(ExprKind::ArrayLit(vec![]), self);
                }
                let first = self.parse_expr()?;
                if self.eat(";") {
                    let n = self.parse_expr()?;
                    self.expect("]")?;
                    return mk(ExprKind::ArrayRepeat(Box::new(first), Box::new(n)), self);
                }
                let mut es = vec![first];
                while self.eat(",") {
                    if self.is("]") {
                        break;
                    }
                    es.push(self.parse_expr()?);
                }
                self.expect("]")?;
                mk(ExprKind::ArrayLit(es), self)
            }
            Tok::Punct("{") => {
                let b = self.parse_block()?;
                mk(ExprKind::Block(b), self)
            }
            Tok::Punct(".") => {
                self.bump();
                let name = self.ident()?;
                mk(ExprKind::DotVariant(name.name), self)
            }
            Tok::Ident(ref s) => {
                let s = s.clone();
                match s.as_str() {
                    "true" | "false" => {
                        self.bump();
                        mk(ExprKind::Bool(s == "true"), self)
                    }
                    "if" => {
                        self.bump();
                        self.parse_if(start)
                    }
                    "match" => {
                        self.bump();
                        let scrut = self.parse_expr_no_struct()?;
                        self.expect("{")?;
                        let mut arms = Vec::new();
                        while !self.is("}") {
                            // `#[allow(lint)] pat => body`
                            let mark = self.open_allows.len();
                            let attrs = self.parse_attrs()?;
                            check_attrs(&attrs, "a match arm", &[])?;
                            let pat = self.parse_pat()?;
                            let guard = if self.eat_kw("if") { Some(self.parse_expr()?) } else { None };
                            self.expect("=>")?;
                            let body = self.parse_expr_stmt_inner()?;
                            let block_like = is_block_like(&body);
                            arms.push(Arm { pat, guard, body });
                            self.close_allows(mark);
                            if !self.eat(",") && !block_like && !self.is("}") {
                                return self.err(format!("expected `,` after match arm, found {}", self.describe()));
                            }
                        }
                        self.expect("}")?;
                        mk(ExprKind::Match(Box::new(scrut), arms), self)
                    }
                    "while" => {
                        self.bump();
                        let conds = self.parse_conds()?;
                        let body = self.parse_block()?;
                        mk(ExprKind::While(conds, body), self)
                    }
                    "loop" => {
                        self.bump();
                        let body = self.parse_block()?;
                        mk(ExprKind::Loop(body), self)
                    }
                    "for" => {
                        self.bump();
                        let var = self.ident()?;
                        // `for i: u8 in 0..10`: the type of the range.
                        let ty = if self.eat(":") { Some(self.parse_type()?) } else { None };
                        self.expect_kw("in")?;
                        let saved = self.no_struct;
                        self.no_struct = true;
                        let lo = self.parse_bin(2)?;
                        let hi = if self.eat("..") { Some(Box::new(self.parse_bin(2)?)) } else { None };
                        self.no_struct = saved;
                        let body = self.parse_block()?;
                        mk(ExprKind::For(var, ty, Box::new(lo), hi, body), self)
                    }
                    "unsafe" if self.is_at(1, "{") => {
                        self.bump();
                        let b = self.parse_block()?;
                        mk(ExprKind::Unsafe(b), self)
                    }
                    "cold" if self.is_at(1, "{") => {
                        self.bump();
                        let b = self.parse_block()?;
                        mk(ExprKind::Vm(VmExpr::Cold(b)), self)
                    }
                    "return" => {
                        self.bump();
                        let v = if self.ends_expr() { None } else { Some(Box::new(self.parse_expr()?)) };
                        mk(ExprKind::Return(v), self)
                    }
                    "break" => {
                        self.bump();
                        let v = if self.ends_expr() { None } else { Some(Box::new(self.parse_expr()?)) };
                        mk(ExprKind::Break(v), self)
                    }
                    "continue" => {
                        self.bump();
                        mk(ExprKind::Continue, self)
                    }
                    "size_of" if self.is_at(1, "<") => {
                        self.bump();
                        self.bump();
                        let t = self.parse_type()?;
                        self.split_angle();
                        self.expect(">")?;
                        self.expect("(")?;
                        self.expect(")")?;
                        mk(ExprKind::SizeOf(t), self)
                    }
                    _ if self.in_vm && self.vm_word(&s) => self.parse_vm_word(start),
                    _ => {
                        let id = self.ident()?;
                        if is_upper(&id.name) && self.is("{") && !self.no_struct {
                            return self.parse_struct_lit(vec![id], start);
                        }
                        mk(ExprKind::Ident(id.name), self)
                    }
                }
            }
            _ => self.err(format!("expected expression, found {}{}", self.describe(), self.operator_hint())),
        }
    }

    /// A hint for operators Loimi does not have, at an "expected
    /// expression" error.
    fn operator_hint(&self) -> &'static str {
        let tok = |back: usize| -> Option<&Token> { self.pos.checked_sub(back).map(|i| &self.toks[i]) };
        let punct = |t: Option<&Token>, p: &str| matches!(t, Some(Token { tok: Tok::Punct(q), .. }) if *q == p);
        let glued = |back: usize| tok(back).is_some_and(|t| !t.space_before);
        let cur = Some(&self.toks[self.pos]);
        if punct(cur, "+") && punct(tok(1), "+") && glued(0) {
            "; Loimi has no `++`: write `x += 1`"
        } else if punct(tok(1), "-") && punct(tok(2), "-") && glued(1) {
            "; Loimi has no `--`: write `x -= 1`"
        } else if punct(cur, "*") && punct(tok(1), "*") && glued(0) {
            "; Loimi has no `**` operator: multiply, or use a loop"
        } else if punct(cur, "=") && (punct(tok(1), "==") || punct(tok(1), "!=")) && glued(0) {
            "; use `==` or `!=` (there is no `===`)"
        } else if punct(cur, "+") {
            "; there is no unary `+`"
        } else if punct(cur, "?") {
            "; there is no `?:` operator: write `if c { a } else { b }`"
        } else {
            ""
        }
    }

    fn vm_word(&self, s: &str) -> bool {
        // `jump (t as *u32)` is the control word with a parenthesized
        // operand, not a call: in vm code these words are not function
        // names (call such a function through its module, `m.fail(..)`).
        // `next (` and `redo (` are reported by parse_vm_word.
        let next_is_field = self.is_at(1, ".") || self.is_at(1, "=");
        match s {
            "next" | "redo" => !next_is_field,
            "jump" | "exit" | "raise" | "fail" | "become" | "use_variant" => {
                !next_is_field && !matches!(self.peek_at(1), Tok::Punct(";") | Tok::Punct("}"))
            }
            _ => false,
        }
    }

    fn parse_vm_word(&mut self, start: Span) -> Res<Expr> {
        let w = match self.bump().tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        if matches!(w.as_str(), "next" | "redo") && self.is("(") {
            return self.err(format!(
                "`{}` is a control word and takes no operand (to call a function named `{}` from vm code, qualify it with its module)",
                w, w
            ));
        }
        if matches!(w.as_str(), "become" | "use_variant") && !matches!(self.peek(), Tok::Ident(_)) {
            let what = if w == "become" { "an op name" } else { "a variant name" };
            return self.err(format!("`{}` takes {}, found {}", w, what, self.describe()));
        }
        let kind = match w.as_str() {
            "next" => VmExpr::Next,
            "redo" => VmExpr::Redo,
            "jump" => VmExpr::Jump(Box::new(self.parse_expr()?)),
            "exit" => VmExpr::Exit(Box::new(self.parse_expr()?)),
            "raise" => VmExpr::Raise(Box::new(self.parse_expr()?)),
            "fail" => VmExpr::Fail(Box::new(self.parse_expr()?)),
            "become" => VmExpr::Become(self.ident()?),
            "use_variant" => VmExpr::UseVariant(self.ident()?),
            _ => unreachable!(),
        };
        Ok(Expr { kind: ExprKind::Vm(kind), span: start.to(self.prev_span()) })
    }

    fn ends_expr(&self) -> bool {
        matches!(self.peek(), Tok::Punct(";") | Tok::Punct("}") | Tok::Punct(")") | Tok::Punct(","))
    }

    fn parse_struct_lit(&mut self, path: Vec<Ident>, start: Span) -> Res<Expr> {
        self.expect("{")?;
        let mut fields = Vec::new();
        let mut rest = None;
        while !self.is("}") {
            if self.is("..") {
                let s = self.span();
                self.bump();
                let z = self.ident()?;
                if z.name != "zero" {
                    return Err(Diag::new(z.span, "expected `zero` after `..` (`..zero` sets the other fields to zero)"));
                }
                rest = Some(s.to(z.span));
                self.eat(",");
                break;
            }
            // `#[cfg(..)]` on a field initializer, to match a field
            // declared under the same condition.
            let keep = self.take_cfg()?;
            let f = self.ident()?;
            let v = if self.eat(":") {
                self.parse_expr()?
            } else {
                Expr { kind: ExprKind::Ident(f.name.clone()), span: f.span }
            };
            if keep {
                fields.push((f, v));
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")?;
        Ok(Expr { kind: ExprKind::StructLit(path, fields, rest), span: start.to(self.prev_span()) })
    }

    fn parse_if(&mut self, start: Span) -> Res<Expr> {
        let conds = self.parse_conds()?;
        let then = self.parse_block()?;
        let els = if self.eat_kw("else") {
            if self.is_kw("if") {
                let s = self.bump().span;
                Some(Box::new(self.parse_if(s)?))
            } else if self.is_kw("cold") && self.is_at(1, "{") {
                let s = self.bump().span;
                let b = self.parse_block()?;
                Some(Box::new(Expr { kind: ExprKind::Vm(VmExpr::Cold(b)), span: s.to(self.prev_span()) }))
            } else {
                let b = self.parse_block()?;
                let sp = b.span;
                Some(Box::new(Expr { kind: ExprKind::Block(b), span: sp }))
            }
        } else {
            None
        };
        Ok(Expr { kind: ExprKind::If(conds, then, els), span: start.to(self.prev_span()) })
    }

    // ── patterns ─────────────────────────────────────────────────────

    pub fn parse_pat(&mut self) -> Res<Pat> {
        let first = self.parse_pat1()?;
        if self.is("|") {
            let mut alts = vec![first];
            while self.eat("|") {
                alts.push(self.parse_pat1()?);
            }
            let span = alts[0].span.to(alts.last().unwrap().span);
            return Ok(Pat { kind: PatKind::Or(alts), span });
        }
        Ok(first)
    }

    fn parse_pat_list(&mut self) -> Res<Vec<Pat>> {
        let mut ps = Vec::new();
        while !self.is(")") {
            ps.push(self.parse_pat()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(ps)
    }

    fn parse_pat1(&mut self) -> Res<Pat> {
        let start = self.span();
        let mk = |kind, p: &Parser| Ok(Pat { kind, span: start.to(p.prev_span()) });
        match self.peek().clone() {
            Tok::Punct("(") => {
                self.bump();
                let ps = self.parse_pat_list()?;
                if ps.len() == 1 {
                    return Ok(ps.into_iter().next().unwrap());
                }
                mk(PatKind::Tuple(ps), self)
            }
            Tok::Punct(".") => {
                self.bump();
                let name = self.ident()?;
                let args = if self.eat("(") { Some(self.parse_pat_list()?) } else { None };
                let mut path = vec![Ident { name: String::new(), span: start }];
                path.push(name);
                mk(PatKind::Variant(path, args), self)
            }
            Tok::Int(_) | Tok::Punct("-") | Tok::Char(_) => {
                let is_char = matches!(self.peek(), Tok::Char(_));
                let lo = self.int_lit()?;
                if self.eat("..=") {
                    let hi = self.int_lit()?;
                    return mk(PatKind::Range(lo, hi), self);
                }
                if is_char {
                    return mk(PatKind::Char(lo as u8), self);
                }
                mk(PatKind::Int(lo), self)
            }
            Tok::Str(s) => {
                self.bump();
                mk(PatKind::Str(s), self)
            }
            Tok::Ident(s) if s == "_" => {
                self.bump();
                mk(PatKind::Wild, self)
            }
            Tok::Ident(s) if s == "true" || s == "false" => {
                self.bump();
                mk(PatKind::Bool(s == "true"), self)
            }
            Tok::Ident(s) if s == "var" => {
                self.bump();
                let id = self.ident()?;
                mk(PatKind::Ident(id.name, true), self)
            }
            Tok::Ident(_) => {
                let mut path = vec![self.ident()?];
                while self.is(".") {
                    self.bump();
                    path.push(self.ident()?);
                }
                if self.eat("(") {
                    let args = self.parse_pat_list()?;
                    return mk(PatKind::Variant(path, Some(args)), self);
                }
                if path.len() > 1 {
                    return mk(PatKind::Variant(path, None), self);
                }
                mk(PatKind::Ident(path.pop().unwrap().name, false), self)
            }
            _ => self.err(format!("expected pattern, found {}", self.describe())),
        }
    }
}

/// `#(a ++ "_X" ++ b)` at `body[i]` in an `inline for` body: one
/// identifier made of the parts, each a loop variable (with a string,
/// identifier or integer value), a string literal or an integer literal.
/// A part naming no variable of this loop is left for an inner `inline
/// for` (outside-in expansion): this loop's variables are replaced by
/// string literals and the construct stays. Returns the index after `)`.
fn splice_concat(body: &[Token], i: usize, vars: &[String], values: &[Vec<Token>], out: &mut Vec<Token>) -> Res<usize> {
    let span = body[i].span;
    let close = (i + 2..body.len()).find(|&k| body[k].tok == Tok::Punct(")"));
    let Some(close) = close else {
        return Err(Diag::new(span, "`#(` without `)`"));
    };
    let inner = &body[i + 2..close];
    // part (`++` part)*, with `++` as two `+` tokens.
    let mut parts: Vec<&Token> = Vec::new();
    let mut k = 0;
    loop {
        let Some(p) = inner.get(k) else {
            return Err(Diag::new(span, "`#(..)` needs one or more parts joined by `++`"));
        };
        parts.push(p);
        k += 1;
        if k == inner.len() {
            break;
        }
        if !(inner[k].tok == Tok::Punct("+") && inner.get(k + 1).is_some_and(|t| t.tok == Tok::Punct("+") && !t.space_before)) {
            return Err(Diag::new(inner[k].span, "expected `++` or `)` in `#(..)`"));
        }
        k += 2;
    }
    // Each part as text, or None when it is left for an inner loop.
    let mut texts: Vec<Option<String>> = Vec::new();
    for p in &parts {
        texts.push(match &p.tok {
            Tok::Str(s) => Some(String::from_utf8_lossy(s).into_owned()),
            Tok::Int(n) => Some(n.to_string()),
            Tok::Ident(v) => match vars.iter().position(|x| x == v) {
                Some(j) => Some(match values[j].as_slice() {
                    [Token { tok: Tok::Str(s), .. }] => String::from_utf8_lossy(s).into_owned(),
                    [Token { tok: Tok::Ident(s), .. }] => s.clone(),
                    [Token { tok: Tok::Int(n), .. }] => n.to_string(),
                    _ => return Err(Diag::new(p.span, format!("`#(..)` needs a string, identifier or integer value for `{}`", v))),
                }),
                None => None,
            },
            _ => return Err(Diag::new(p.span, "a part of `#(..)` is a loop variable, a string literal or an integer literal")),
        });
    }
    if texts.iter().all(|t| t.is_some()) {
        let name: String = texts.into_iter().map(|t| t.unwrap()).collect();
        let ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            return Err(Diag::new(span, format!("`#(..)` makes `{}`, which is not an identifier", name)));
        }
        out.push(Token { tok: Tok::Ident(name), span, space_before: true });
    } else {
        out.extend(body[i..i + 2].iter().cloned());
        let mut k = 0;
        for (p, t) in parts.iter().zip(texts) {
            if k > 0 {
                out.extend(inner[k - 2..k].iter().cloned());
            }
            out.push(match t {
                Some(t) if matches!(p.tok, Tok::Ident(_)) => Token { tok: Tok::Str(t.into_bytes()), span: p.span, space_before: p.space_before },
                _ => (*p).clone(),
            });
            k += 3;
        }
        out.push(body[close].clone());
    }
    Ok(close + 1)
}

fn split_top_level(toks: &[Token]) -> Vec<Vec<Token>> {
    let mut out = vec![vec![]];
    let mut depth = 0i32;
    for t in toks {
        match t.tok {
            Tok::Punct("(") | Tok::Punct("[") | Tok::Punct("{") => depth += 1,
            Tok::Punct(")") | Tok::Punct("]") | Tok::Punct("}") => depth -= 1,
            Tok::Punct(",") if depth == 0 => {
                out.push(vec![]);
                continue;
            }
            _ => {}
        }
        out.last_mut().unwrap().push(t.clone());
    }
    if out.last().is_some_and(|v| v.is_empty()) {
        out.pop();
    }
    out
}

fn field(e: Expr, name: Ident) -> Expr {
    let span = e.span.to(name.span);
    Expr { kind: ExprKind::Field(Box::new(e), name), span }
}

pub fn is_block_like(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::Block(_)
            | ExprKind::Unsafe(_)
            | ExprKind::If(..)
            | ExprKind::Match(..)
            | ExprKind::While(..)
            | ExprKind::Loop(_)
            | ExprKind::For(..)
            | ExprKind::Vm(VmExpr::Cold(_))
    )
}

pub fn is_reserved(s: &str) -> bool {
    matches!(
        s,
        "fn" | "let"
            | "var"
            | "if"
            | "else"
            | "while"
            | "loop"
            | "for"
            | "in"
            | "return"
            | "break"
            | "continue"
            | "struct"
            | "enum"
            | "const"
            | "import"
            | "extern"
            | "export"
            | "match"
            | "as"
            | "true"
            | "false"
            | "and"
            | "or"
            | "pub"
            | "defer"
            | "unsafe"
            | "never"
    )
}

/// `#[trace] struct S { .. }`: the method `fn S.trace(s: *S, h:
/// *gc.Heap)`, which marks the GC references in the fields, for a root
/// scanner to call. Its body is `__gc_trace_fields(h, s)`, which the
/// checker expands by the fields' types (`Checker::trace_fields`).
/// Emitted only if called (`#[__lazy]`).
fn trace_method(sd: &StructDecl, items: &[Item]) -> Res<FnDecl> {
    let sp = sd.derive_trace.unwrap();
    let id = |n: &str| Ident { name: n.into(), span: sp };
    // The heap's type, as this module names it.
    let heap = items.iter().find_map(|it| match it {
        Item::Import(Import { path, names, .. }) if path.len() == 2 && path[0].name == "std" && path[1].name == "gc" => match names {
            None => Some(vec![id(&path[1].name), id("Heap")]),
            Some(ns) => ns.iter().find(|n| n.name == "Heap").map(|_| vec![id("Heap")]),
        },
        _ => None,
    });
    let Some(heap) = heap else {
        return Err(Diag::new(sp, "`#[trace]` generates `S.trace(s, heap: *gc.Heap)`: add `import std.gc;`"));
    };
    let named = |path: Vec<Ident>| TypeExpr { kind: TypeKind::Named(path, vec![]), span: sp };
    let ptr = |t: TypeExpr| TypeExpr { kind: TypeKind::Ptr(Box::new(t)), span: sp };
    let ident = |n: &str| Expr { kind: ExprKind::Ident(n.into()), span: sp };
    let call = Expr { kind: ExprKind::Call(Box::new(ident("__gc_trace_fields")), vec![ident("h"), ident("s")]), span: sp };
    let mut attrs = Attrs::default();
    attrs.list.push((crate::vmexpand::LAZY_ATTR.into(), vec![], sp));
    Ok(FnDecl {
        name: id("trace"),
        generics: vec![],
        bounds: vec![],
        recv_generics: 0,
        recv: Some(sd.name.clone()),
        params: vec![Param { name: id("s"), ty: ptr(named(vec![sd.name.clone()])) }, Param { name: id("h"), ty: ptr(named(heap)) }],
        variadic: false,
        ret: None,
        body: Some(Block { stmts: vec![Stmt::Expr { expr: call, semi: false }], span: sp }),
        linkage: Linkage::Internal,
        attrs,
        is_pub: sd.is_pub,
        link_name: None,
    })
}
