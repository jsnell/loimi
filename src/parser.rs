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
}

fn is_upper(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

impl Parser {
    pub fn new(toks: Vec<Token>) -> Parser {
        Parser { toks, pos: 0, no_struct: false, in_vm: false }
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

    pub fn parse_module(&mut self) -> Res<(Vec<Item>, Attrs)> {
        let mut items = Vec::new();
        let mut mod_attrs = Attrs::default();
        while *self.peek() != Tok::Eof {
            if self.is("#") && self.is_at(1, "!") {
                self.bump();
                self.bump();
                self.parse_attr_body(&mut mod_attrs)?;
                continue;
            }
            items.push(self.parse_item()?);
        }
        Ok((items, mod_attrs))
    }

    fn parse_attr_body(&mut self, attrs: &mut Attrs) -> Res<()> {
        self.expect("[")?;
        loop {
            let start = self.span();
            let name = self.ident()?.name;
            let mut args = Vec::new();
            if self.eat("(") {
                while !self.is(")") {
                    let t = self.bump();
                    match t.tok {
                        Tok::Ident(s) => args.push(s),
                        Tok::Int(i) => args.push(i.to_string()),
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
            attrs.list.push((name, args, start.to(self.prev_span())));
            if !self.eat(",") {
                break;
            }
        }
        self.expect("]")?;
        Ok(())
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
        if self.is_kw("fn") {
            return Ok(Item::Fn(self.parse_fn(attrs, Linkage::Internal, is_pub)?));
        }
        if self.is_kw("extern") || self.is_kw("export") {
            let linkage = if self.bump().tok == Tok::Ident("extern".into()) { Linkage::Extern } else { Linkage::Export };
            let abi = self.string_lit()?;
            if abi != "C" {
                return self.err("only the \"C\" ABI is supported");
            }
            return Ok(Item::Fn(self.parse_fn(attrs, linkage, is_pub)?));
        }
        if self.eat_kw("struct") {
            let name = self.ident()?;
            self.expect("{")?;
            let mut fields = Vec::new();
            while !self.is("}") {
                let f = self.ident()?;
                self.expect(":")?;
                let t = self.parse_type()?;
                fields.push((f, t));
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}")?;
            return Ok(Item::Struct(StructDecl { name, fields, is_pub }));
        }
        if self.eat_kw("enum") {
            let name = self.ident()?;
            let repr = if self.eat(":") { Some(self.parse_type()?) } else { None };
            self.expect("{")?;
            let mut variants = Vec::new();
            while !self.is("}") {
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
                variants.push(Variant { name: vname, fields, discr });
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}")?;
            return Ok(Item::Enum(EnumDecl { name, repr, variants, is_pub }));
        }
        if self.eat_kw("const") {
            let name = self.ident()?;
            self.expect(":")?;
            let ty = self.parse_type()?;
            self.expect("=")?;
            let value = self.parse_expr()?;
            self.expect(";")?;
            return Ok(Item::Const(ConstDecl { name, ty, value, is_pub }));
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
            return Ok(Item::Value(self.parse_value_decl(is_pub)?));
        }
        self.err(format!("expected an item, found {}", self.describe()))
    }

    pub fn parse_fn(&mut self, attrs: Attrs, linkage: Linkage, is_pub: bool) -> Res<FnDecl> {
        self.expect_kw("fn")?;
        let first = self.ident()?;
        let (recv, name) = if self.eat(".") { (Some(first), self.ident()?) } else { (None, first) };
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
        Ok(FnDecl { name, recv, params, variadic, ret, body, linkage, attrs, is_pub })
    }

    // ── types ────────────────────────────────────────────────────────

    pub fn parse_type(&mut self) -> Res<TypeExpr> {
        let start = self.span();
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
                self.expect(";")?;
                let n = self.parse_expr()?;
                self.expect("]")?;
                TypeKind::Array(Box::new(elem), Box::new(n))
            }
        } else if self.eat("(") {
            let mut ts = Vec::new();
            while !self.is(")") {
                ts.push(self.parse_type()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            TypeKind::Tuple(ts)
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
            TypeKind::Fn(ps, Box::new(ret))
        } else if self.eat_kw("never") {
            TypeKind::Never
        } else {
            let mut path = vec![self.ident()?];
            while self.is(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                self.bump();
                path.push(self.ident()?);
            }
            let mut args = Vec::new();
            if self.eat("<") {
                while !self.is(">") {
                    args.push(self.parse_type()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(">")?;
            }
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
            stmts.push(self.parse_stmt()?);
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
        self.err(format!("expected `;` or `}}`, found {}", self.describe()))
    }

    fn parse_expr_stmt_inner(&mut self) -> Res<Expr> {
        let lhs = self.parse_expr()?;
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
        let mut lhs = if min <= 1 { self.parse_coalesce()? } else { self.parse_unary()? };
        if min <= 1 {
            return Ok(lhs);
        }
        loop {
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
        let mut e = self.parse_postfix()?;
        while self.eat_kw("as") {
            let t = self.parse_type()?;
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
                            let pat = self.parse_pat()?;
                            let guard = if self.eat_kw("if") { Some(self.parse_expr()?) } else { None };
                            self.expect("=>")?;
                            let body = self.parse_expr_stmt_inner()?;
                            let block_like = is_block_like(&body);
                            arms.push(Arm { pat, guard, body });
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
                        self.expect_kw("in")?;
                        let saved = self.no_struct;
                        self.no_struct = true;
                        let lo = self.parse_bin(2)?;
                        self.expect("..")?;
                        let hi = self.parse_bin(2)?;
                        self.no_struct = saved;
                        let body = self.parse_block()?;
                        mk(ExprKind::For(var, Box::new(lo), Box::new(hi), body), self)
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
            _ => self.err(format!("expected expression, found {}", self.describe())),
        }
    }

    fn vm_word(&self, s: &str) -> bool {
        let next_is_call_or_field = self.is_at(1, "(") || self.is_at(1, ".") || self.is_at(1, "=");
        match s {
            "next" | "redo" => !next_is_call_or_field,
            "jump" | "exit" | "raise" | "fail" | "become" => {
                !next_is_call_or_field && !matches!(self.peek_at(1), Tok::Punct(";") | Tok::Punct("}"))
            }
            _ => false,
        }
    }

    fn parse_vm_word(&mut self, start: Span) -> Res<Expr> {
        let w = match self.bump().tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        let kind = match w.as_str() {
            "next" => VmExpr::Next,
            "redo" => VmExpr::Redo,
            "jump" => VmExpr::Jump(Box::new(self.parse_expr()?)),
            "exit" => VmExpr::Exit(Box::new(self.parse_expr()?)),
            "raise" => VmExpr::Raise(Box::new(self.parse_expr()?)),
            "fail" => VmExpr::Fail(Box::new(self.parse_expr()?)),
            "become" => VmExpr::Become(self.ident()?),
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
        while !self.is("}") {
            let f = self.ident()?;
            let v = if self.eat(":") {
                self.parse_expr()?
            } else {
                Expr { kind: ExprKind::Ident(f.name.clone()), span: f.span }
            };
            fields.push((f, v));
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")?;
        Ok(Expr { kind: ExprKind::StructLit(path, fields), span: start.to(self.prev_span()) })
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
