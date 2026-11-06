//! Parsing of `vm` and `value` declarations.

use crate::ast::*;
use crate::diag::{Diag, Res, Span};
use crate::lexer::Tok;
use crate::parser::Parser;
use crate::vmast::*;

/// The name of the generated breakpoint op for words of inline cache
/// layout `cache` (None: no cache), see `desugar_break`.
pub fn break_op_name(cache: Option<&str>) -> String {
    match cache {
        None => "BREAK".to_string(),
        Some(c) => format!("BREAK__{c}"),
    }
}

/// `on break { .. }`: one op per inline cache layout, plus one without a
/// cache, all running the block. `Vm.set_break` writes the one with the
/// patched instruction's (or its first part's) cache over its opcode, so
/// the breakpoint has the instruction's size: walks over the code
/// (`resume_at`, the disassembler, `deopt`, the step-over) stay on
/// instruction boundaries with breakpoints in place.
fn desugar_break(vm: &mut VmDecl) -> Res<()> {
    let Some(body) = vm.on_break.clone() else { return Ok(()) };
    let Some(fmt) = vm.formats.first() else { return Ok(()) };
    if let Some(o) = vm.ops.iter().find(|o| o.name.name == "BREAK" || o.name.name.starts_with("BREAK__")) {
        return Err(Diag::new(o.name.span, format!("`{}` is the name of a generated breakpoint op (this vm has `on break`)", o.name.name)));
    }
    if vm.ops.iter().any(|o| o.stack.is_some()) {
        return Err(Diag::new(body.span, "`on break` is not supported in stack vms yet"));
    }
    let span = body.span;
    let caches: Vec<Option<Ident>> = std::iter::once(None).chain(vm.caches.iter().map(|c| Some(c.name.clone()))).collect();
    for c in caches {
        vm.ops.push(OpDecl {
            name: Ident { name: break_op_name(c.as_ref().map(|c| c.name.as_str())), span },
            params: vec![],
            format: fmt.name.clone(),
            cache: c,
            body: body.clone(),
            family: None,
            then: None,
            stack: None,
            accumulate: false,
        });
    }
    Ok(())
}

/// `refuse CODE;`: after every `become X` of a family that some super
/// names (as a part, or a member of a part's family), call
/// `Vm.refuse(CODE)`, which selects value accumulator variants again and
/// fuses supers again over the whole code, as `Vm.optimize` does on
/// quickened code. A `become` of a family that no super names and that
/// has no accumulator variants to select cannot change what fuses, so it
/// stays a single store.
fn desugar_refuse(vm: &mut VmDecl) {
    let Some(code) = vm.refuse.clone() else { return };
    let fams: Vec<(String, String)> = vm.ops.iter().map(|o| (o.name.name.clone(), o.family.clone().unwrap_or_else(|| o.name.name.clone()))).collect();
    let fam = |n: &str| fams.iter().find(|(o, _)| o == n).map(|(_, f)| f.clone());
    let mut fused: Vec<String> = vm.supers.iter().flat_map(|s| s.parts.iter()).filter_map(|p| fam(&p.name)).collect();
    // `Vm.refuse` selects value accumulator variants too (`vmexpand`).
    if !vm.accs.is_empty() && vm.accs.iter().all(|a| a.wt) {
        fused.extend(vm.ops.iter().filter(|o| o.accumulate).filter_map(|o| fam(&o.name.name)));
    }
    // With `supers from`, the supers are known only at expansion: every
    // `become` re-fuses.
    let all = !vm.supers_from.is_empty();
    let wants = |n: &str| all || fam(n).is_some_and(|f| fused.contains(&f));
    let vname = vm.name.name.clone();
    let call = |sp: Span| Expr {
        kind: ExprKind::Call(
            Box::new(Expr {
                kind: ExprKind::Field(Box::new(Expr { kind: ExprKind::Ident(vname.clone()), span: sp }), Ident { name: "refuse".into(), span: sp }),
                span: sp,
            }),
            vec![code.clone()],
        ),
        span: sp,
    };
    fn is_call(s: &Stmt) -> bool {
        matches!(s, Stmt::Expr { expr: Expr { kind: ExprKind::Call(c, _), .. }, .. }
            if matches!(&c.kind, ExprKind::Field(_, f) if f.name == "refuse"))
    }
    // A call after each `become` statement of the block (and of its
    // `let .. else` blocks).
    fn ins(b: &mut Block, wants: &dyn Fn(&str) -> bool, call: &dyn Fn(Span) -> Expr) {
        let mut i = 0;
        while i < b.stmts.len() {
            if let Stmt::Let { els: Some(e), .. } = &mut b.stmts[i] {
                ins(e, wants, call);
            }
            let sp = match &b.stmts[i] {
                Stmt::Expr { expr: Expr { kind: ExprKind::Vm(VmExpr::Become(n)), span }, .. } if wants(&n.name) => Some(*span),
                _ => None,
            };
            if let Some(sp) = sp {
                if !b.stmts.get(i + 1).is_some_and(is_call) {
                    // The `become` may be the block's value (no `;`).
                    if let Stmt::Expr { semi, .. } = &mut b.stmts[i] {
                        *semi = true;
                    }
                    b.stmts.insert(i + 1, Stmt::Expr { expr: call(sp), semi: true });
                }
                i += 1;
            }
            i += 1;
        }
    }
    for o in &mut vm.ops {
        ins(&mut o.body, &wants, &call);
        crate::vmacc::walk_block_pub(&mut o.body, &mut |x: &mut Expr| {
            match &mut x.kind {
                ExprKind::Block(b)
                | ExprKind::Unsafe(b)
                | ExprKind::Loop(b)
                | ExprKind::While(_, b)
                | ExprKind::For(_, _, _, _, b)
                | ExprKind::If(_, b, _)
                | ExprKind::Vm(VmExpr::Cold(b)) => ins(b, &wants, &call),
                ExprKind::Match(_, arms) => {
                    for a in arms {
                        if matches!(&a.body.kind, ExprKind::Vm(VmExpr::Become(n)) if wants(&n.name)) {
                            let sp = a.body.span;
                            let b = a.body.clone();
                            a.body = Expr {
                                kind: ExprKind::Block(Block { stmts: vec![Stmt::Expr { expr: b, semi: true }, Stmt::Expr { expr: call(sp), semi: true }], span: sp }),
                                span: sp,
                            };
                        }
                    }
                }
                _ => {}
            }
            true
        });
    }
}

impl Parser {
    pub fn parse_vm(&mut self, attrs: Attrs, is_pub: bool) -> Res<VmDecl> {
        self.expect_kw("vm")?;
        let name = self.ident()?;
        self.expect("{")?;
        let mut vm = VmDecl {
            name,
            attrs,
            context: None,
            state: vec![],
            operands: vec![],
            word: None,
            formats: vec![],
            caches: vec![],
            ops: vec![],
            supers: vec![],
            fns: vec![],
            entries: vec![],
            on_raise: None,
            on_break: None,
            variants: vec![],
            sync: None,
            reload: None,
            verify_ctx: vec![],
            verify_owner: None,
            checks: vec![],
            stack_entry: None,
            refuse: None,
            supers_from: vec![],
            accs: vec![],
            acc_list: vec![],
            acc_sel: vec![],
            rels: vec![],
            data_keys: vec![],
            data_vals: vec![],
            acc_ops: vec![],
            auto_cold: vec![],
            is_pub,
        };
        // `inline for` variables replace every matching token in the body,
        // so one named like a state variable would silently replace it.
        let mut loop_vars: Vec<Ident> = Vec::new();
        self.vm_polls.clear();
        self.vm_hot.clear();
        while !self.eat("}") {
            if self.is_kw("inline") && self.is_kw_at(1, "for") {
                loop_vars.extend(self.expand_inline_for(false)?);
                continue;
            }
            // `#[cfg(NAME)]` on a vm item: parsed, then dropped when NAME
            // is not enabled (the vm is as if the item were not written).
            let keep = self.take_cfg()?;
            let saved = if keep { None } else { Some((vm.clone(), self.vm_loop_vars.len(), self.vm_hot.len())) };
            let mark = self.allow_mark();
            let item_attrs = self.parse_attrs()?;
            let kw = match self.peek() {
                Tok::Ident(s) => s.clone(),
                _ => return self.err("expected a vm item (state, operand, encoding, op, ...)"),
            };
            let (what, known): (String, &[&str]) = match kw.as_str() {
                "fn" => ("a vm function".into(), crate::parser::FN_ATTRS),
                "super" | "supers" => (format!("a `{}`", kw), &["forward"]),
                "on" => ("`on raise` or `on break`".into(), &[]),
                "op" => ("an `op`".into(), &["accumulate", "poll"]),
                "family" => ("a `family`".into(), &["poll"]),
                "variant" => ("a `variant`".into(), &["unfused"]),
                "operand" | "entry" => (format!("an `{}`", kw), &[]),
                _ => (format!("a `{}`", kw), &[]),
            };
            crate::parser::check_attrs(&item_attrs, &what, known)?;
            match kw.as_str() {
                "context" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect(":")?;
                    let ty = self.parse_type()?;
                    self.expect(";")?;
                    vm.context = Some(Param { name: n, ty });
                }
                "state" => {
                    self.bump();
                    self.expect("{")?;
                    while !self.is("}") {
                        let n = self.ident()?;
                        self.expect(":")?;
                        let ty = self.parse_type()?;
                        vm.state.push(Param { name: n, ty });
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("}")?;
                }
                "operand" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect("=")?;
                    let body = self.parse_expr()?;
                    let mut verify = None;
                    let mut display = None;
                    let mut konst = None;
                    loop {
                        if verify.is_none() && self.eat_kw("verify") {
                            verify = Some(self.parse_expr()?);
                        } else if konst.is_none() && self.eat_kw("const") {
                            konst = Some(self.parse_expr()?);
                        } else if display.is_none() && self.eat_kw("display") {
                            let t = self.bump();
                            let Tok::Str(s) = t.tok else {
                                return Err(Diag::new(t.span, "expected a string after `display`, like \"r{}\""));
                            };
                            let s = String::from_utf8_lossy(&s).into_owned();
                            if s.matches("{}").count() != 1 {
                                return Err(Diag::new(t.span, "a `display` string needs exactly one `{}`"));
                            }
                            display = Some(s);
                        } else {
                            break;
                        }
                    }
                    self.expect(";")?;
                    vm.operands.push(OperandDecl { name: n, body, verify, konst, display });
                }
                "encoding" => {
                    self.bump();
                    let kind = self.ident()?;
                    if kind.name != "fixed" {
                        return Err(crate::diag::Diag::new(kind.span, "only `encoding fixed(T)` is supported so far"));
                    }
                    self.expect("(")?;
                    vm.word = Some(self.parse_type()?);
                    self.expect(")")?;
                    self.expect("{")?;
                    while !self.eat("}") {
                        self.expect_kw("format")?;
                        let fname = self.ident()?;
                        self.expect("{")?;
                        let mut fields = Vec::new();
                        while !self.is("}") {
                            let f = self.ident()?;
                            self.expect(":")?;
                            let (bits, signed) = match self.bump().tok {
                                Tok::Int(n) => (n as u32, false),
                                Tok::Ident(s) if s.starts_with('s') && s[1..].parse::<u32>().is_ok() => {
                                    (s[1..].parse().unwrap(), true)
                                }
                                _ => return self.err("expected a bit width like `8` or `s16`"),
                            };
                            fields.push(FormatField { name: f, bits, signed });
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("}")?;
                        vm.formats.push(FormatDecl { name: fname, fields });
                    }
                }
                "cache" => {
                    self.bump();
                    let n = self.ident()?;
                    if self.eat("=") {
                        // `cache X = S;`: the fields of struct `S`
                        // (vmexpand::cache_structs fills them in).
                        let t = self.parse_type()?;
                        self.expect(";")?;
                        vm.caches.push(CacheDecl { name: n, fields: vec![], from: Some(t) });
                    } else {
                        self.expect("{")?;
                        let mut fields = Vec::new();
                        while !self.is("}") {
                            let f = self.ident()?;
                            self.expect(":")?;
                            let weak = self.eat_kw("weak");
                            let t = self.parse_type()?;
                            fields.push((f, t, weak));
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("}")?;
                        vm.caches.push(CacheDecl { name: n, fields, from: None });
                    }
                }
                "poll" => {
                    // `poll gc { .. }`: statements that `#[poll(gc)]` on
                    // an op puts at its start, and `poll gc;` anywhere in
                    // the vm's code.
                    self.bump();
                    let n = self.ident()?;
                    if let Some((p, _)) = self.vm_polls.iter().find(|(p, _)| p.name == n.name) {
                        return Err(Diag::new(n.span, format!("poll `{}` is declared twice", n.name)).note(p.span, "first declared here"));
                    }
                    let body = self.vm_block()?;
                    self.vm_polls.push((n, keep.then_some(body)));
                }
                "op" => {
                    self.bump();
                    let (name, params, format, cache, then, stack) = self.parse_op_head()?;
                    let mut body = self.vm_block()?;
                    self.add_polls(&item_attrs, &mut body)?;
                    vm.ops.push(OpDecl { name, params, format, cache, body, family: None, then, stack, accumulate: item_attrs.has("accumulate") });
                }
                "family" => {
                    self.bump();
                    let (name, params, format, cache, then, stack) = self.parse_op_head()?;
                    self.expect("{")?;
                    while !self.eat("}") {
                        let member = self.ident()?;
                        let mut body = self.vm_block()?;
                        self.add_polls(&item_attrs, &mut body)?;
                        let mname = if member.name == "generic" {
                            name.clone()
                        } else {
                            member
                        };
                        vm.ops.push(OpDecl {
                            name: mname,
                            params: params.clone(),
                            format: format.clone(),
                            cache: cache.clone(),
                            body,
                            family: Some(name.name.clone()),
                            then: then.clone(),
                            stack: stack.clone(),
                            accumulate: false,
                        });
                    }
                }
                "super" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect("=")?;
                    let mut parts = vec![self.ident()?];
                    self.expect(";")?;
                    while matches!(self.peek(), Tok::Ident(s) if s.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()))
                        && self.is_at(1, ";")
                    {
                        parts.push(self.ident()?);
                        self.expect(";")?;
                    }
                    vm.supers.push(SuperDecl { name: n, parts, forward: item_attrs.has("forward") });
                }
                "supers" if matches!(self.peek_at(1), Tok::Ident(s) if s == "from") => {
                    // `supers from "file";`: the first list of supers a
                    // `--vm-stats` report suggests (`vmexpand::supers_from`;
                    // the path is relative to this file).
                    self.bump();
                    self.bump();
                    let sp = self.span();
                    let path = self.string_lit()?;
                    self.expect(";")?;
                    vm.supers_from.push((path, sp, item_attrs.has("forward")));
                }
                "refuse" => {
                    // `refuse fp.cl.proto.code;`: `become` fuses supers
                    // again in the code it rewrites (`desugar_refuse`).
                    self.bump();
                    if vm.refuse.is_some() {
                        return self.err("a vm has at most one `refuse`");
                    }
                    vm.refuse = Some(self.parse_expr()?);
                    self.expect(";")?;
                }
                "supers" => {
                    // `supers { A, B; NAME = C, D, E; }`: one super per
                    // `;`, each with the group's attributes (and its own),
                    // named by its parts joined with `_` unless named.
                    self.bump();
                    self.expect("{")?;
                    while !self.eat("}") {
                        let mark = self.allow_mark();
                        let attrs = self.parse_attrs()?;
                        crate::parser::check_attrs(&attrs, "a super of a `supers` group", &["forward"])?;
                        let name = if matches!(self.peek(), Tok::Ident(_)) && self.is_at(1, "=") {
                            let n = self.ident()?;
                            self.bump();
                            Some(n)
                        } else {
                            None
                        };
                        let mut parts = vec![self.ident()?];
                        while self.eat(",") {
                            parts.push(self.ident()?);
                        }
                        if parts.len() < 2 {
                            return Err(Diag::new(parts[0].span, "a super has at least two parts: `A, B;`"));
                        }
                        self.expect(";")?;
                        let name = name.unwrap_or_else(|| Ident {
                            name: parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join("_"),
                            span: parts[0].span.to(parts[parts.len() - 1].span),
                        });
                        vm.supers.push(SuperDecl { name, parts, forward: item_attrs.has("forward") || attrs.has("forward") });
                        self.close_allows(mark);
                    }
                }
                "rel" => {
                    // `rel kform(I32_ADD) = I32_ADDK;`: `Vm.kform(op)`.
                    // `_` for either op declares nothing (an `inline for`
                    // row without the relation).
                    self.bump();
                    let name = self.ident()?;
                    self.expect("(")?;
                    let from = self.ident()?;
                    self.expect(")")?;
                    self.expect("=")?;
                    let to = self.ident()?;
                    self.expect(";")?;
                    if from.name != "_" && to.name != "_" {
                        vm.rels.push(RelDecl { name, from, to });
                    }
                }
                "data" => {
                    // `data comm: bool = false;` declares a key, and
                    // `data comm(I32_ADD) = true;` gives an op its value.
                    self.bump();
                    let name = self.ident()?;
                    if self.eat(":") {
                        let ty = self.parse_type()?;
                        self.expect("=")?;
                        let default = self.parse_expr()?;
                        self.expect(";")?;
                        vm.data_keys.push(DataKeyDecl { name, ty, default });
                    } else {
                        self.expect("(")?;
                        let op = self.ident()?;
                        self.expect(")")?;
                        self.expect("=")?;
                        let value = self.parse_expr()?;
                        self.expect(";")?;
                        vm.data_vals.push(DataValDecl { key: name, op, value });
                    }
                }
                "fn" => {
                    self.in_vm = true;
                    let f = self.parse_fn(item_attrs, Linkage::Internal, false);
                    self.in_vm = false;
                    vm.fns.push(f?);
                }
                "entry" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect("(")?;
                    let mut params = Vec::new();
                    while !self.is(")") {
                        let pn = self.ident()?;
                        self.expect(":")?;
                        let ty = self.parse_type()?;
                        params.push(Param { name: pn, ty });
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    self.expect("->")?;
                    let ret = self.parse_type()?;
                    let mut effects = Attrs::default();
                    if self.eat_kw("uses") {
                        loop {
                            let e = self.ident()?;
                            effects.list.push(("uses".into(), vec![e.name], e.span));
                            if !self.eat(",") {
                                break;
                            }
                        }
                    }
                    let body = self.vm_block()?;
                    vm.entries.push(EntryDecl { name: n, params, ret, body, effects });
                }
                "on" if self.is_kw_at(1, "break") => {
                    // `on break { .. }`: the handler of the generated
                    // breakpoint ops (see `desugar_break`).
                    self.bump();
                    self.bump();
                    if vm.on_break.is_some() {
                        return self.err("a vm has at most one `on break` block");
                    }
                    vm.on_break = Some(self.vm_block()?);
                }
                "on" => {
                    self.bump();
                    self.expect_kw("raise")?;
                    self.expect("(")?;
                    let pn = self.ident()?;
                    self.expect(":")?;
                    let ty = self.parse_type()?;
                    self.expect(")")?;
                    let body = self.vm_block()?;
                    vm.on_raise = Some(RaiseDecl { param: Param { name: pn, ty }, body });
                }
                "verify_context" => {
                    // `verify_context proto: *Proto, lim: *Limits;`: the
                    // verifier's parameters after the code, in order (also
                    // across several declarations).
                    self.bump();
                    loop {
                        // `owner proto: Gc<Proto>`: the object that holds
                        // the code (`code: verified [W] by Vm`).
                        let owner = if self.is_kw("owner") && matches!(self.peek_at(1), Tok::Ident(_)) {
                            Some(self.bump().span)
                        } else {
                            None
                        };
                        let n = self.ident()?;
                        self.expect(":")?;
                        let ty = self.parse_type()?;
                        if vm.verify_ctx.iter().any(|p: &Param| p.name.name == n.name) {
                            return Err(Diag::new(n.span, format!("`{}` is declared twice in `verify_context`", n.name)));
                        }
                        if let Some(sp) = owner {
                            if vm.verify_owner.is_some() {
                                return Err(Diag::new(sp, "a vm has at most one `owner` verify context"));
                            }
                            let is_gc = matches!(&ty.kind, TypeKind::Named(p, a) if p.len() == 1 && p[0].name == "Gc" && a.len() == 1);
                            if !is_gc {
                                return Err(Diag::new(ty.span, "the `owner` verify context is the object holding the code: `owner p: Gc<Proto>`"));
                            }
                            vm.verify_owner = Some(vm.verify_ctx.len());
                        }
                        vm.verify_ctx.push(Param { name: n, ty });
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(";")?;
                }
                "verify" => {
                    // `verify handlers { .. }`: a `bool` block the verifier
                    // evaluates after checking the code, with the verify
                    // context, `code` and `is_insn_boundary(p)` in scope.
                    self.bump();
                    let n = self.ident()?;
                    if vm.checks.iter().any(|(c, _)| c.name == n.name) {
                        return Err(Diag::new(n.span, format!("`verify {}` is declared twice", n.name)));
                    }
                    let b = self.parse_block()?;
                    vm.checks.push((n, b));
                }
                "stack_entry" => {
                    // `stack_entry word.nin;`: the operand stack depth that
                    // code starts with, from the verify context.
                    self.bump();
                    if vm.stack_entry.is_some() {
                        return self.err("a vm has at most one `stack_entry`");
                    }
                    vm.stack_entry = Some(self.parse_expr()?);
                    self.expect(";")?;
                }
                "accumulator" => {
                    // `accumulator acc: u64 for ri, rq;`
                    self.bump();
                    let n = self.ident()?;
                    self.expect(":")?;
                    let ty = self.parse_type()?;
                    self.expect_kw("for")?;
                    let mut kinds = vec![self.ident()?];
                    while self.eat(",") {
                        kinds.push(self.ident()?);
                    }
                    self.expect(";")?;
                    vm.accs.push(AccDecl { name: n, ty, kinds, wt: false });
                }
                "accumulate" => {
                    // `accumulate ADD, SUB;`: as `#[accumulate]` on each.
                    // Op selectors: `A..B`, `I32_*K`, `with KIND`.
                    self.bump();
                    vm.acc_sel.push(self.parse_selector()?);
                    while self.eat(",") {
                        vm.acc_sel.push(self.parse_selector()?);
                    }
                    self.expect(";")?;
                }
                "sync" => {
                    self.bump();
                    vm.sync = Some(self.vm_block()?);
                }
                "reload" => {
                    self.bump();
                    if vm.reload.is_some() {
                        return self.err("a vm has at most one `reload` block");
                    }
                    vm.reload = Some(self.vm_block()?);
                }
                "variant" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect_kw("wraps")?;
                    let wraps = if self.eat_kw("all") {
                        None
                    } else {
                        self.expect("[")?;
                        let mut list = Vec::new();
                        while !self.is("]") {
                            list.push(self.parse_selector()?);
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("]")?;
                        Some(list)
                    };
                    self.expect("{")?;
                    self.expect_kw("before")?;
                    let before = self.vm_block()?;
                    self.expect("}")?;
                    vm.variants.push(VariantDecl { name: n, wraps: None, wraps_sel: wraps, before, unfused: item_attrs.has("unfused") });
                }
                _ => return self.err(format!("unknown vm item `{}`", kw)),
            }
            self.close_allows(mark);
            if let Some((v, n, h)) = saved {
                vm = v;
                self.vm_loop_vars.truncate(n);
                self.vm_hot.truncate(h);
            }
        }
        loop_vars.append(&mut self.vm_loop_vars);
        resolve_selectors(&mut vm)?;
        vm.auto_cold = crate::autocold::run(&mut vm, &self.vm_hot)?;
        desugar_break(&mut vm)?;
        crate::vmacc::desugar(&mut vm)?;
        desugar_refuse(&mut vm);
        for v in &loop_vars {
            if vm.state.iter().chain(vm.context.iter()).any(|p| p.name.name == v.name) {
                return Err(crate::diag::Diag::new(
                    v.span,
                    format!("`inline for` variable `{}` has the name of a vm state variable, which it would replace in the body", v.name),
                ));
            }
        }
        Ok(vm)
    }

    /// `#[poll(a, b)]` on an op or family: the polls' statements, in
    /// that order, before the body's.
    fn add_polls(&self, attrs: &Attrs, body: &mut Block) -> Res<()> {
        let mut pre = Vec::new();
        for (n, args, sp) in &attrs.list {
            if n != "poll" {
                continue;
            }
            if args.is_empty() {
                return Err(Diag::new(*sp, "`#[poll(..)]` names the polls to run at the start of the op: `#[poll(gc)]`"));
            }
            for a in args {
                pre.extend(self.poll_body(&Ident { name: a.clone(), span: *sp })?);
            }
        }
        if !pre.is_empty() {
            pre.append(&mut body.stmts);
            body.stmts = pre;
        }
        Ok(())
    }

    /// An op selector: `NAME`, `A..B`, a glob (`BR_I32_*`, `*_TK`; no
    /// spaces inside) or `with KIND`.
    fn parse_selector(&mut self) -> Res<Selector> {
        if self.is_kw("with") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            return Ok(Selector::With(self.ident()?));
        }
        let glob = self.is("*") || (matches!(self.peek(), Tok::Ident(_)) && self.is_at(1, "*") && self.adjacent(1));
        if glob {
            let lo = self.span();
            let mut pat = String::new();
            loop {
                match self.peek().clone() {
                    Tok::Ident(s) => pat.push_str(&s),
                    Tok::Punct("*") => pat.push('*'),
                    _ => break,
                }
                self.bump();
                if !self.adjacent(0) {
                    break;
                }
            }
            return Ok(Selector::Glob(pat, lo.to(self.prev_span())));
        }
        let a = self.ident()?;
        if self.eat("..") {
            return Ok(Selector::Range(a, self.ident()?));
        }
        Ok(Selector::Name(a))
    }

    fn vm_block(&mut self) -> Res<Block> {
        self.in_vm = true;
        let b = self.parse_block();
        self.in_vm = false;
        b
    }

    #[allow(clippy::type_complexity)]
    fn parse_op_head(&mut self) -> Res<(Ident, Vec<OpParam>, Ident, Option<Ident>, Option<Ident>, Option<StackEffect>)> {
        let name = self.ident()?;
        self.expect("(")?;
        let mut params = Vec::new();
        while !self.is(")") {
            let n = self.ident()?;
            self.expect(":")?;
            let k = self.ident()?;
            params.push(OpParam { name: n, kind: k });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        self.expect(":")?;
        let format = self.ident()?;
        let cache = if self.eat("+") { Some(self.ident()?) } else { None };
        let then = if self.eat_kw("then") { Some(self.ident()?) } else { None };
        let stack = if self.is_kw("stack") && self.is_at(1, "(") { Some(self.parse_stack_effect()?) } else { None };
        Ok((name, params, format, cache, then, stack))
    }

    /// `stack(a b -- c)`, `stack(xs[n] -- arr)`, `stack(.. --)`. Commas
    /// between items are optional.
    fn parse_stack_effect(&mut self) -> Res<StackEffect> {
        let lo = self.bump().span;
        self.expect("(")?;
        let (mut rest, mut after) = (false, false);
        let (mut ins, mut outs) = (Vec::new(), Vec::new());
        while !self.is(")") {
            if self.is("-") && self.is_at(1, "-") {
                if after {
                    return self.err("a stack effect has one `--`");
                }
                self.bump();
                self.bump();
                after = true;
                continue;
            }
            if self.eat(",") {
                continue;
            }
            if self.is("..") {
                if after || rest || !ins.is_empty() {
                    return self.err("`..` (every value on the stack) can only come first among the inputs");
                }
                self.bump();
                rest = true;
                continue;
            }
            let name = self.ident()?;
            let count = if self.eat("[") {
                let e = self.parse_expr()?;
                self.expect("]")?;
                Some(e)
            } else {
                None
            };
            if after { &mut outs } else { &mut ins }.push(StackItem { name, count });
        }
        let hi = self.bump().span;
        if !after {
            return Err(Diag::new(lo.to(hi), "a stack effect needs `--` between the inputs and the outputs: `stack(a b -- c)`"));
        }
        Ok(StackEffect { rest, ins, outs })
    }

    pub fn parse_value_decl(&mut self, is_pub: bool) -> Res<ValueDecl> {
        self.expect_kw("value")?;
        let name = self.ident()?;
        self.expect_kw("repr")?;
        let repr = self.ident()?;
        self.expect("{")?;
        let mut variants = Vec::new();
        while !self.is("}") {
            let vn = self.ident()?;
            let payload = if self.eat("(") {
                let t = self.parse_type()?;
                self.expect(")")?;
                Some(t)
            } else {
                None
            };
            let unboxed = self.eat_kw("unboxed");
            let block = if self.is_kw("block") {
                let sp = self.bump().span;
                let t = self.bump();
                let Tok::Int(n) = t.tok else {
                    return Err(Diag::new(t.span, "`block` takes the block width in bits: `Int(i32) block 48`"));
                };
                Some((n.min(1 << 16) as u32, sp.to(t.span)))
            } else {
                None
            };
            variants.push(ValueVariant { name: vn, payload, unboxed, block });
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")?;
        Ok(ValueDecl { name, repr, variants, is_pub, scan_raw: None })
    }
}

/// Does op name `n` match glob `pat` (`*`: any run of characters)?
fn glob_match(pat: &str, n: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == n;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if n.len() < first.len() + last.len() || !n.starts_with(first) || !n.ends_with(last) {
        return false;
    }
    let mut rest = &n[first.len()..n.len() - last.len()];
    for p in &parts[1..parts.len() - 1] {
        match rest.find(p) {
            Some(i) => rest = &rest[i + p.len()..],
            None => return false,
        }
    }
    true
}

/// The ops (indices into `vm.ops`, in declaration order) that selectors
/// name, each once; supers named by name are left to the caller. Ranges,
/// globs and `with` leave out family members (name those). A selector
/// that names no op is an error.
fn select_ops(vm: &VmDecl, sels: &[Selector], what: &str) -> Res<Vec<usize>> {
    let pos = |id: &Ident| -> Res<usize> {
        vm.ops.iter().position(|o| o.name.name == id.name).ok_or_else(|| Diag::new(id.span, format!("unknown op `{}`", id.name)))
    };
    let plain = |k: usize| vm.ops[k].family.is_none();
    let mut out: Vec<usize> = Vec::new();
    for s in sels {
        let (found, span): (Vec<usize>, crate::diag::Span) = match s {
            Selector::Name(id) if vm.supers.iter().any(|x| x.name.name == id.name) => continue,
            Selector::Name(id) => (vec![pos(id)?], id.span),
            Selector::Range(a, b) => {
                let (i, j) = (pos(a)?, pos(b)?);
                if i > j {
                    return Err(Diag::new(
                        a.span.to(b.span),
                        format!("`{}` is declared before `{}`: `A..B` names the ops declared from A to B", b.name, a.name),
                    ));
                }
                ((i..=j).filter(|&k| plain(k)).collect(), a.span.to(b.span))
            }
            Selector::Glob(p, sp) => ((0..vm.ops.len()).filter(|&k| plain(k) && glob_match(p, &vm.ops[k].name.name)).collect(), *sp),
            Selector::With(kind) => {
                if !vm.operands.iter().any(|o| o.name.name == kind.name) {
                    return Err(Diag::new(kind.span, format!("unknown operand kind `{}`", kind.name)));
                }
                let has = |k: usize| vm.ops[k].params.iter().any(|p| p.kind.name == kind.name);
                ((0..vm.ops.len()).filter(|&k| plain(k) && has(k)).collect(), kind.span)
            }
        };
        if found.is_empty() {
            return Err(Diag::new(span, format!("this selector names no op of the vm (in `{what}`)")));
        }
        for i in found {
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// `accumulate` and `variant .. wraps [..]` selectors into op names. A
/// variant that wraps an op also wraps every super with it as a part, so
/// that a super declared later is not silently left out.
/// `LOIMI_DUMP_VM_EXPANSION=1` prints the lists.
fn resolve_selectors(vm: &mut VmDecl) -> Res<()> {
    let dump = std::env::var_os("LOIMI_DUMP_VM_EXPANSION").is_some();
    if !vm.acc_sel.is_empty() {
        let sels = std::mem::take(&mut vm.acc_sel);
        for s in &sels {
            if let Selector::Name(id) = s {
                if vm.supers.iter().any(|x| x.name.name == id.name) {
                    return Err(Diag::new(id.span, format!("`{}` is a super: `accumulate` takes plain ops", id.name)));
                }
            }
        }
        let ops = select_ops(vm, &sels, "accumulate")?;
        for &i in &ops {
            vm.acc_list.push(vm.ops[i].name.clone());
        }
        if dump {
            let list: Vec<&str> = ops.iter().map(|&i| vm.ops[i].name.name.as_str()).collect();
            eprintln!("// vm {}: accumulate {}", vm.name.name, list.join(", "));
        }
    }
    for vi in 0..vm.variants.len() {
        let Some(sels) = vm.variants[vi].wraps_sel.take() else { continue };
        let ops = select_ops(vm, &sels, "wraps")?;
        let mut names: Vec<Ident> = ops.iter().map(|&i| vm.ops[i].name.clone()).collect();
        // Supers named in the list, and those with a wrapped part.
        for sup in &vm.supers {
            let named = sels.iter().any(|s| matches!(s, Selector::Name(id) if id.name == sup.name.name));
            let has_part = sup.parts.iter().any(|p| ops.iter().any(|&i| vm.ops[i].name.name == p.name));
            if named || has_part {
                names.push(sup.name.clone());
            }
        }
        if dump {
            let list: Vec<&str> = names.iter().map(|n| n.name.as_str()).collect();
            eprintln!("// vm {}: variant {} wraps {}", vm.name.name, vm.variants[vi].name.name, list.join(", "));
        }
        vm.variants[vi].wraps = Some(names);
    }
    Ok(())
}
